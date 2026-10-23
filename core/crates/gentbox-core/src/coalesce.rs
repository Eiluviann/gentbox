//! Coalescing, dedupe, supersede and the implicit rescinds (architecture §7),
//! as functions of the conversation's open items and their live-waiter flags.
//!
//! Everything is within one conversation (`source.agent` + `conversation.id`):
//! topics and threads never coalesce, so items of other conversations in the
//! slice are ignored. The hub applies the rules one message at a time.
//!
//! An incoming message has no long-poll yet, so its own `awaitsResponse`
//! stands in for "will have a live waiter": the sender polls right after the
//! 201.

use time::{Duration, OffsetDateTime};

use crate::ids::{ConversationKey, MessageId};
use crate::item::OpenItem;
use crate::message::{Message, MessageType};
use crate::rescind::{
    RescindReason, RescindRequest, Rescission, rescind_by_correlation_keys, rescission_for,
};

/// A message that repeats its predecessor within this window is dropped.
pub const DEDUPE_WINDOW: Duration = Duration::seconds(10);
/// The correlation key of Claude Code's idle reminder, which progress rescinds.
pub const IDLE_CORRELATION_KEY: &str = "idle";

/// Why a message was not stored.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DuplicateKind {
    /// Same `source.agent` + `idempotencyKey`: return the stored message and
    /// change nothing.
    Idempotent,
    /// Same fields, title and summary as the message it would supersede,
    /// within 10 s: only a repeat counter increases.
    Dedupe,
}

/// What the hub does with an incoming message.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum CoalesceOutcome {
    /// Not stored.
    Duplicate {
        /// The message that already exists.
        of: MessageId,
        /// Why it counts as a duplicate.
        kind: DuplicateKind,
    },
    /// Not stored: it only increases the repeat counter of a message with a
    /// live waiter, which it must never displace.
    Folded {
        /// The message that absorbs it.
        into: MessageId,
    },
    /// Stored, taking the slot of an open message in place (same popup, same
    /// native notification).
    Superseded {
        /// The message whose slot it takes.
        replaces: MessageId,
        /// Whether it pings: only when louder (urgent over non-urgent, or a
        /// higher blocking level).
        pings: bool,
        /// The superseded message first, then any implicit or explicit rescinds.
        rescissions: Vec<Rescission>,
    },
    /// Stored as a new item.
    New {
        /// Implicit (completion) and explicit (`rescinds`) rescinds.
        rescissions: Vec<Rescission>,
    },
}

/// What coalescing needs about the incoming message.
#[derive(Debug, Clone)]
pub struct CoalesceInput<'a> {
    /// The incoming message, classified and guarded; `received_at` is "now".
    pub incoming: &'a Message,
    /// The message's `rescinds` field, if any.
    pub rescinds: Option<&'a RescindRequest>,
    /// The stored message with the same `source.agent` and `idempotencyKey`.
    pub idempotent_match: Option<&'a MessageId>,
    /// The conversation's items (others are ignored).
    pub open_items: &'a [OpenItem],
    /// Whether a phone channel exists (v1: never).
    pub has_phone_channel: bool,
}

/// Applies rules 1 to 5 and 7 of architecture §7, in that order.
pub fn coalesce(input: &CoalesceInput<'_>) -> CoalesceOutcome {
    let incoming = input.incoming;
    if let Some(existing) = input.idempotent_match {
        return CoalesceOutcome::Duplicate {
            of: existing.clone(),
            kind: DuplicateKind::Idempotent,
        };
    }
    let conversation = incoming.conversation_key();
    let items: Vec<&OpenItem> = input
        .open_items
        .iter()
        .filter(|item| item.is_live() && item.conversation == conversation)
        .collect();
    let will_have_waiter = incoming.awaits_response;

    if let Some(old) = latest_with_correlation_key(&items, incoming.correlation_key.as_deref()) {
        if old.has_live_waiter && !will_have_waiter {
            return CoalesceOutcome::Folded {
                into: old.id.clone(),
            };
        }
        if is_repeat_of(incoming, old) {
            return CoalesceOutcome::Duplicate {
                of: old.id.clone(),
                kind: DuplicateKind::Dedupe,
            };
        }
        let mut rescissions = vec![rescission_for(
            old,
            RescindReason::Superseded,
            Some(&incoming.id),
            input.has_phone_channel,
        )];
        rescissions.extend(other_rescissions(input, &items, Some(&old.id)));
        return CoalesceOutcome::Superseded {
            replaces: old.id.clone(),
            pings: is_louder(incoming, old),
            rescissions,
        };
    }
    if incoming.blocking.is_waiting()
        && !will_have_waiter
        && let Some(waiting) = latest_waiting_with_live_waiter(&items)
    {
        return CoalesceOutcome::Folded {
            into: waiting.id.clone(),
        };
    }
    CoalesceOutcome::New {
        rescissions: other_rescissions(input, &items, None),
    }
}

/// Rule 6, progress: the conversation moved on, so its idle item and its
/// `blocking` and `deferrable` items without a live waiter were handled in
/// the session. They are rescinded as `resolved_elsewhere`.
pub fn progress_rescissions(
    conversation: &ConversationKey,
    items: &[OpenItem],
    has_phone_channel: bool,
) -> Vec<Rescission> {
    items
        .iter()
        .filter(|item| item.is_live() && &item.conversation == conversation)
        .filter(|item| {
            item.correlation_key.as_deref() == Some(IDLE_CORRELATION_KEY)
                || (item.is_waiting_on_you() && !item.has_live_waiter)
        })
        .map(|item| {
            rescission_for(
                item,
                RescindReason::ResolvedElsewhere,
                None,
                has_phone_channel,
            )
        })
        .collect()
}

fn other_rescissions(
    input: &CoalesceInput<'_>,
    items: &[&OpenItem],
    already_rescinded: Option<&MessageId>,
) -> Vec<Rescission> {
    let mut rescissions = Vec::new();
    if input.incoming.message_type == MessageType::TaskCompleted {
        rescissions.extend(completion_rescissions(
            items,
            already_rescinded,
            input.has_phone_channel,
        ));
    }
    if let Some(request) = input.rescinds {
        let owned: Vec<OpenItem> = items.iter().map(|&item| item.clone()).collect();
        let explicit = rescind_by_correlation_keys(
            &input.incoming.conversation_key(),
            request,
            &owned,
            input.has_phone_channel,
        )
        .unwrap_or_default();
        rescissions.extend(explicit);
    }
    let mut seen: Vec<&MessageId> = already_rescinded.into_iter().collect();
    let mut unique = Vec::new();
    for rescission in &rescissions {
        if !seen.contains(&&rescission.message_id) {
            seen.push(&rescission.message_id);
            unique.push(rescission.clone());
        }
    }
    unique
}

/// Rule 5: a completion rescinds, as `obsolete`, the `blocking` and
/// `deferrable` items without a live waiter. An item with a live waiter stays
/// open (the agent cannot have finished while it waits), and `non_blocking`
/// items survive the end of the turn.
fn completion_rescissions(
    items: &[&OpenItem],
    already_rescinded: Option<&MessageId>,
    has_phone_channel: bool,
) -> Vec<Rescission> {
    items
        .iter()
        .filter(|item| item.is_waiting_on_you() && !item.has_live_waiter)
        .filter(|item| Some(&item.id) != already_rescinded)
        .map(|item| rescission_for(item, RescindReason::Obsolete, None, has_phone_channel))
        .collect()
}

fn latest_with_correlation_key<'a>(
    items: &[&'a OpenItem],
    key: Option<&str>,
) -> Option<&'a OpenItem> {
    let key = key?;
    items
        .iter()
        .copied()
        .filter(|item| item.correlation_key.as_deref() == Some(key))
        .max_by_key(|item| item.received_at)
}

fn latest_waiting_with_live_waiter<'a>(items: &[&'a OpenItem]) -> Option<&'a OpenItem> {
    items
        .iter()
        .copied()
        .filter(|item| item.is_waiting_on_you() && item.has_live_waiter)
        .max_by_key(|item| item.received_at)
}

fn is_repeat_of(incoming: &Message, old: &OpenItem) -> bool {
    let elapsed = incoming.received_at - old.received_at;
    incoming.message_type == old.message_type
        && incoming.urgent == old.urgent
        && incoming.blocking == old.blocking
        && incoming.title == old.title
        && incoming.summary == old.summary
        && is_within_dedupe_window(elapsed)
}

fn is_within_dedupe_window(elapsed: Duration) -> bool {
    elapsed >= Duration::ZERO && elapsed < DEDUPE_WINDOW
}

fn is_louder(incoming: &Message, old: &OpenItem) -> bool {
    (incoming.urgent && !old.urgent) || incoming.blocking > old.blocking
}

/// The instant a message's dedupe window closes; useful for the hub's tests
/// and logs.
pub fn dedupe_window_end(received_at: OffsetDateTime) -> OffsetDateTime {
    received_at + DEDUPE_WINDOW
}
