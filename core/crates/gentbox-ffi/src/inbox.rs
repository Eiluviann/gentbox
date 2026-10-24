//! The menu-bar inbox, grouped topic → thread → messages (architecture §5.4,
//! ADRs 0008 and 0009), built from one [`InboxSnapshot`].
//!
//! - **Order.** Urgent first, then what an agent is blocked on, then what it
//!   paused for, then newest. A
//!   thread sorts by its most pressing message and a topic by its most
//!   pressing thread, so the order never needs a counter to explain it.
//! - **Next up.** The most pressing message an agent waits on is lifted out
//!   of the groups into the popover's full card.
//! - **Closed.** Closed topics and threads leave the groups for the Closed
//!   section, with their open messages; reopening brings them back.
//! - **Mute** only changes delivery: muted groups stay listed and are named in
//!   the footer.

use std::cmp::Reverse;
use std::collections::HashMap;
use std::time::SystemTime;

use gentbox_core::topic::{GroupStatus, Thread, Topic};
use gentbox_hub::InboxSnapshot;

use crate::records::{Blocking, InboxMessage, LifecycleState, MessageType};

/// At most this many closed topics and threads, most recently closed first.
const CLOSED_LIMIT: usize = 8;

/// A topic or a thread.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, uniffi::Enum)]
pub enum GroupKind {
    /// A topic.
    Topic,
    /// A thread.
    Thread,
}

/// A thread header and its messages, most pressing first.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct InboxThread {
    /// The hub id.
    pub id: String,
    /// The title; a conversation's default thread carries the conversation's.
    pub title: String,
    /// A conversation's default thread, made by the hub.
    pub is_implicit: bool,
    /// Muted itself (a muted topic mutes it too; see [`InboxTopic::is_muted`]).
    pub is_muted: bool,
    /// The messages.
    pub messages: Vec<InboxMessage>,
}

/// A topic header and its threads.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct InboxTopic {
    /// The hub id.
    pub id: String,
    /// The title.
    pub title: String,
    /// An agent's implicit General topic.
    pub is_general: bool,
    /// Muted, and so are its threads.
    pub is_muted: bool,
    /// The agent's SF Symbol hint, checked by the app before use.
    pub symbol: Option<String>,
    /// The threads.
    pub threads: Vec<InboxThread>,
}

/// A row in the Closed section.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct ClosedGroup {
    /// Topic or thread.
    pub kind: GroupKind,
    /// The hub id.
    pub id: String,
    /// The title.
    pub title: String,
    /// For a thread, its topic's title.
    pub topic_title: Option<String>,
    /// When it closed.
    pub closed_at: SystemTime,
}

/// A muted open topic or thread, for the footer.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct MutedGroup {
    /// Topic or thread.
    pub kind: GroupKind,
    /// The hub id.
    pub id: String,
    /// The title.
    pub title: String,
}

/// A topic a thread can move to.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct TopicChoice {
    /// The hub id.
    pub id: String,
    /// The title.
    pub title: String,
}

/// Everything the menu-bar inbox draws.
#[derive(Debug, Clone, PartialEq, uniffi::Record)]
pub struct Inbox {
    /// The message an agent is blocked on, for the full card.
    pub next_up: Option<InboxMessage>,
    /// The other open messages, grouped.
    pub topics: Vec<InboxTopic>,
    /// Closed topics and threads, most recently closed first.
    pub closed: Vec<ClosedGroup>,
    /// Muted open topics and threads.
    pub muted: Vec<MutedGroup>,
    /// Open explicit topics, for "Move to topic".
    pub move_targets: Vec<TopicChoice>,
    /// How many messages agents wait on: the menu-bar count.
    pub waiting_count: u32,
    /// Some listed message is still unread (open, not acknowledged): the
    /// menu-bar dot.
    pub has_unread: bool,
}

/// Groups a snapshot for the menu-bar inbox.
pub(crate) fn build_inbox(snapshot: &InboxSnapshot) -> Inbox {
    let topics: HashMap<&str, &Topic> = snapshot
        .topics
        .iter()
        .map(|topic| (topic.id.as_str(), topic))
        .collect();
    let threads: HashMap<&str, &Thread> = snapshot
        .threads
        .iter()
        .map(|thread| (thread.id.as_str(), thread))
        .collect();
    let is_listed = |message: &InboxMessage| {
        let topic_is_open = topics
            .get(message.topic_id.as_str())
            .is_some_and(|topic| topic.status == GroupStatus::Open);
        let thread_is_open = threads
            .get(message.thread_id.as_str())
            .is_some_and(|thread| thread.status == GroupStatus::Open);
        topic_is_open && thread_is_open
    };
    let mut messages: Vec<InboxMessage> = snapshot
        .entries
        .iter()
        .map(InboxMessage::from_entry)
        .filter(is_listed)
        .collect();
    messages.sort_by_key(pressing_order);
    let waiting_count = u32::try_from(
        messages
            .iter()
            .filter(|message| message.is_waiting())
            .count(),
    )
    .unwrap_or(u32::MAX);
    let has_unread = messages
        .iter()
        .any(|message| message.state == LifecycleState::Open);
    let next_up = messages
        .iter()
        .position(is_blocked_on)
        .map(|index| messages.remove(index));
    Inbox {
        next_up,
        topics: group_by_topic_and_thread(messages, &topics, &threads),
        closed: closed_groups(snapshot, &topics),
        muted: muted_groups(snapshot, &topics),
        move_targets: move_targets(snapshot),
        waiting_count,
        has_unread,
    }
}

/// Keeps the order of `messages`: a topic appears where its first message
/// does, and a thread within its topic likewise.
fn group_by_topic_and_thread(
    messages: Vec<InboxMessage>,
    topics: &HashMap<&str, &Topic>,
    threads: &HashMap<&str, &Thread>,
) -> Vec<InboxTopic> {
    let mut grouped: Vec<InboxTopic> = Vec::new();
    for message in messages {
        let (Some(topic), Some(thread)) = (
            topics.get(message.topic_id.as_str()),
            threads.get(message.thread_id.as_str()),
        ) else {
            continue;
        };
        let topic_index = match grouped
            .iter()
            .position(|group| group.id == topic.id.as_str())
        {
            Some(index) => index,
            None => {
                grouped.push(inbox_topic(topic));
                grouped.len() - 1
            }
        };
        let Some(topic_group) = grouped.get_mut(topic_index) else {
            continue;
        };
        match topic_group
            .threads
            .iter_mut()
            .find(|group| group.id == thread.id.as_str())
        {
            Some(thread_group) => thread_group.messages.push(message),
            None => {
                let mut thread_group = inbox_thread(thread);
                thread_group.messages.push(message);
                topic_group.threads.push(thread_group);
            }
        }
    }
    grouped
}

fn closed_groups(snapshot: &InboxSnapshot, topics: &HashMap<&str, &Topic>) -> Vec<ClosedGroup> {
    let closed_topics = snapshot
        .topics
        .iter()
        .filter(|topic| topic.status == GroupStatus::Closed)
        .map(|topic| ClosedGroup {
            kind: GroupKind::Topic,
            id: topic.id.as_str().to_owned(),
            title: topic.title.clone(),
            topic_title: None,
            closed_at: SystemTime::from(topic.closed_at.unwrap_or(topic.updated_at)),
        });
    // A thread closed with its topic reopens with it: the topic's row stands
    // for both.
    let closed_threads = snapshot
        .threads
        .iter()
        .filter(|thread| thread.status == GroupStatus::Closed && !thread.closed_by_topic)
        .filter_map(|thread| {
            let topic = topics.get(thread.topic_id.as_str())?;
            (topic.status == GroupStatus::Open).then(|| ClosedGroup {
                kind: GroupKind::Thread,
                id: thread.id.as_str().to_owned(),
                title: thread.title.clone(),
                topic_title: Some(topic.title.clone()),
                closed_at: SystemTime::from(thread.closed_at.unwrap_or(thread.updated_at)),
            })
        });
    let mut closed: Vec<ClosedGroup> = closed_topics.chain(closed_threads).collect();
    closed.sort_by_key(|group| Reverse(group.closed_at));
    closed.truncate(CLOSED_LIMIT);
    closed
}

fn muted_groups(snapshot: &InboxSnapshot, topics: &HashMap<&str, &Topic>) -> Vec<MutedGroup> {
    let muted_topics = snapshot
        .topics
        .iter()
        .filter(|topic| topic.status == GroupStatus::Open && topic.muted)
        .map(|topic| MutedGroup {
            kind: GroupKind::Topic,
            id: topic.id.as_str().to_owned(),
            title: topic.title.clone(),
        });
    // A thread of a muted topic is covered by the topic's entry.
    let muted_threads = snapshot
        .threads
        .iter()
        .filter(|thread| thread.status == GroupStatus::Open && thread.muted)
        .filter(|thread| {
            topics
                .get(thread.topic_id.as_str())
                .is_some_and(|topic| topic.status == GroupStatus::Open && !topic.muted)
        })
        .map(|thread| MutedGroup {
            kind: GroupKind::Thread,
            id: thread.id.as_str().to_owned(),
            title: thread.title.clone(),
        });
    let mut muted: Vec<MutedGroup> = muted_topics.chain(muted_threads).collect();
    muted.sort_by(|left, right| left.title.cmp(&right.title));
    muted
}

fn move_targets(snapshot: &InboxSnapshot) -> Vec<TopicChoice> {
    let mut targets: Vec<TopicChoice> = snapshot
        .topics
        .iter()
        .filter(|topic| topic.status == GroupStatus::Open && !topic.implicit)
        .map(|topic| TopicChoice {
            id: topic.id.as_str().to_owned(),
            title: topic.title.clone(),
        })
        .collect();
    targets.sort_by(|left, right| left.title.cmp(&right.title));
    targets
}

/// Urgent first, then blocking, then deferrable, then newest.
fn pressing_order(message: &InboxMessage) -> (bool, u8, Reverse<SystemTime>) {
    let waits = match message.blocking {
        Blocking::Blocking => 0,
        Blocking::Deferrable => 1,
        Blocking::NonBlocking => 2,
    };
    (!message.is_urgent, waits, Reverse(message.received_at))
}

/// A question or an action an agent has stopped for.
fn is_blocked_on(message: &InboxMessage) -> bool {
    message.is_waiting()
        && matches!(
            message.message_type,
            MessageType::DecisionRequired | MessageType::ActionRequired
        )
}

fn inbox_topic(topic: &Topic) -> InboxTopic {
    InboxTopic {
        id: topic.id.as_str().to_owned(),
        title: topic.title.clone(),
        is_general: topic.implicit,
        is_muted: topic.muted,
        symbol: topic.style.as_ref().and_then(|style| style.symbol.clone()),
        threads: Vec::new(),
    }
}

fn inbox_thread(thread: &Thread) -> InboxThread {
    InboxThread {
        id: thread.id.as_str().to_owned(),
        title: thread.title.clone(),
        is_implicit: thread.implicit,
        is_muted: thread.muted,
        messages: Vec::new(),
    }
}
