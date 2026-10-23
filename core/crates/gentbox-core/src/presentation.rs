//! The words surfaces use for a message: the title phrase, the blocking
//! badge and the native notification subtitle (architecture §10). The same on
//! the Mac's native notifications and, later, the phone.

use crate::message::{Blocking, MessageScope, MessageType};

/// The notification title for a message. Urgency never changes it.
///
/// `generic` shows the message's own title.
pub fn title_phrase(
    message_type: MessageType,
    scope: Option<MessageScope>,
    message_title: &str,
) -> &str {
    match (message_type, scope) {
        (MessageType::DecisionRequired, _) => "Your input is needed",
        (MessageType::ActionRequired, _) => "Action required",
        (MessageType::TaskCompleted, Some(MessageScope::Goal)) => "Goal complete",
        (MessageType::TaskCompleted, _) => "Task complete",
        (MessageType::Generic, _) => message_title,
    }
}

/// The blocking badge label: "Waiting on you" or "Paused · can defer";
/// `non_blocking` has none.
pub fn blocking_badge_label(blocking: Blocking) -> Option<&'static str> {
    match blocking {
        Blocking::NonBlocking => None,
        Blocking::Deferrable => Some("Paused · can defer"),
        Blocking::Blocking => Some("Waiting on you"),
    }
}

/// What a native notification's subtitle is made of.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SubtitleParts<'a> {
    /// Whether the message is urgent.
    pub urgent: bool,
    /// The blocking level.
    pub blocking: Blocking,
    /// The topic title.
    pub topic_title: &'a str,
    /// Whether the topic is an agent's implicit General topic.
    pub topic_is_general: bool,
    /// The thread title.
    pub thread_title: &'a str,
}

/// The native notification subtitle: "Urgent · Waiting on you · topic ·
/// thread". Native notifications have no badge slot, so urgency and the
/// blocking badge lead the subtitle. A part that repeats the previous one (a
/// thread named like its topic) is left out, and "General" is left out.
pub fn notification_subtitle(parts: &SubtitleParts<'_>) -> Option<String> {
    let mut segments: Vec<&str> = Vec::new();
    if parts.urgent {
        segments.push("Urgent");
    }
    if let Some(label) = blocking_badge_label(parts.blocking) {
        segments.push(label);
    }
    if !parts.topic_is_general && !parts.topic_title.trim().is_empty() {
        segments.push(parts.topic_title.trim());
    }
    let thread_title = parts.thread_title.trim();
    let repeats_topic = thread_title.eq_ignore_ascii_case(parts.topic_title.trim());
    if !thread_title.is_empty() && !repeats_topic {
        segments.push(thread_title);
    }
    (!segments.is_empty()).then(|| segments.join(" · "))
}
