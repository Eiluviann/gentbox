//! Rescinding (ADR 0007, architecture §7.7): a sender withdraws its own
//! messages. Rescinding never answers; the sender token can rescind but not
//! approve.

use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::ids::{ConversationKey, MessageId};
use crate::item::OpenItem;
use crate::lifecycle::LifecycleState;

/// Why a message was withdrawn.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RescindReason {
    /// Answered or handled in the session or another channel.
    ResolvedElsewhere,
    /// No longer needed.
    Obsolete,
    /// Replaced by a newer message.
    Superseded,
}

impl RescindReason {
    /// Every reason.
    pub const ALL: [Self; 3] = [Self::ResolvedElsewhere, Self::Obsolete, Self::Superseded];

    /// The reason in words, for the inbox's "Withdrawn" row.
    pub fn in_words(self) -> &'static str {
        match self {
            Self::ResolvedElsewhere => "Handled in the session",
            Self::Obsolete => "No longer needed",
            Self::Superseded => "Replaced by a newer message",
        }
    }
}

/// Body of `DELETE /v1/messages/{id}` (reason only), of
/// `POST /v1/conversations/{agent}/{id}/rescind` and of a message's
/// `rescinds` field (reason and correlation keys).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RescindRequest {
    /// Why the messages are withdrawn.
    pub reason: RescindReason,
    /// The slots to withdraw, within the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_keys: Option<Vec<String>>,
}

/// What one channel must do when a message is withdrawn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelEffect {
    /// Close the persistent popup.
    RemovePopup,
    /// Close the toast.
    RemoveToast,
    /// Remove the Mac native notification (`removeDeliveredNotifications`).
    RemoveNativeNotification,
    /// Keep the inbox row, marked "Withdrawn" with the reason in words.
    MarkWithdrawn(RescindReason),
    /// Fold the inbox row into the message that replaced it.
    FoldIntoSuccessor(MessageId),
    /// Wake the waiting sender with `rescinded` and the reason.
    WakeWaiter(RescindReason),
    /// Retract the phone notification (only when a phone channel exists).
    RetractPhoneNotification,
}

/// One withdrawn message and everything that follows from it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rescission {
    /// The withdrawn message.
    pub message_id: MessageId,
    /// Why.
    pub reason: RescindReason,
    /// What each channel must do.
    pub effects: Vec<ChannelEffect>,
}

/// A rejected rescind.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RescindError {
    /// The message is already closed; there is nothing to withdraw.
    #[error("the message is already {state:?}")]
    AlreadyClosed {
        /// The terminal state it is in.
        state: LifecycleState,
    },
    /// Rescinding by correlation key needs at least one key.
    #[error("a rescind by correlation key needs at least one correlation key")]
    MissingCorrelationKeys,
}

/// Builds the effects of withdrawing one message.
///
/// - **open:** the popup or toast that shows it is removed, the native
///   notification is removed, the inbox row is kept as "Withdrawn".
/// - **acknowledged:** closing a popup acknowledges, so no popup or toast is
///   left to remove; the native notification may still sit in Notification
///   Centre and is removed; the inbox row is kept.
/// - **superseded:** the inbox row folds into `successor` instead of staying
///   as "Withdrawn".
/// - a waiter with a live long-poll is woken with `rescinded`.
/// - phone entries appear only when `has_phone_channel`.
pub fn rescission_for(
    item: &OpenItem,
    reason: RescindReason,
    successor: Option<&MessageId>,
    has_phone_channel: bool,
) -> Rescission {
    let mut effects = Vec::new();
    if item.state == LifecycleState::Open {
        if item.delivered.popup {
            effects.push(ChannelEffect::RemovePopup);
        }
        if item.delivered.toast {
            effects.push(ChannelEffect::RemoveToast);
        }
    }
    if item.delivered.native_notification {
        effects.push(ChannelEffect::RemoveNativeNotification);
    }
    match (reason, successor) {
        (RescindReason::Superseded, Some(successor)) => {
            effects.push(ChannelEffect::FoldIntoSuccessor(successor.clone()));
        }
        _ => effects.push(ChannelEffect::MarkWithdrawn(reason)),
    }
    if item.has_live_waiter {
        effects.push(ChannelEffect::WakeWaiter(reason));
    }
    if has_phone_channel {
        effects.push(ChannelEffect::RetractPhoneNotification);
    }
    Rescission {
        message_id: item.id.clone(),
        reason,
        effects,
    }
}

/// `DELETE /v1/messages/{id}`: withdraws one message.
pub fn rescind_by_id(
    item: &OpenItem,
    reason: RescindReason,
    has_phone_channel: bool,
) -> Result<Rescission, RescindError> {
    if !item.is_live() {
        return Err(RescindError::AlreadyClosed { state: item.state });
    }
    Ok(rescission_for(item, reason, None, has_phone_channel))
}

/// `POST /v1/conversations/{agent}/{id}/rescind` and the `rescinds` field:
/// withdraws the conversation's live messages whose correlation key is listed.
///
/// Items of other conversations and closed items are ignored. A request
/// without keys is an error rather than "everything", so a typo never
/// withdraws a whole conversation.
pub fn rescind_by_correlation_keys(
    conversation: &ConversationKey,
    request: &RescindRequest,
    items: &[OpenItem],
    has_phone_channel: bool,
) -> Result<Vec<Rescission>, RescindError> {
    let keys = match request.correlation_keys.as_deref() {
        Some(keys) if !keys.is_empty() => keys,
        _ => return Err(RescindError::MissingCorrelationKeys),
    };
    let rescissions = items
        .iter()
        .filter(|item| item.is_live() && &item.conversation == conversation)
        .filter(|item| {
            item.correlation_key
                .as_deref()
                .is_some_and(|key| keys.iter().any(|candidate| candidate == key))
        })
        .map(|item| rescission_for(item, request.reason, None, has_phone_channel))
        .collect();
    Ok(rescissions)
}
