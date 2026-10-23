//! The words surfaces use: title phrases, badges and the native subtitle.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::message::{Blocking, MessageScope, MessageType};
use gentbox_core::presentation::{
    SubtitleParts, blocking_badge_label, notification_subtitle, title_phrase,
};
use serde_json::json;
use support::stored;

#[test]
fn title_phrases_never_carry_urgency() {
    let rows = [
        (MessageType::DecisionRequired, None, "Your input is needed"),
        (MessageType::ActionRequired, None, "Action required"),
        (MessageType::TaskCompleted, None, "Task complete"),
        (
            MessageType::TaskCompleted,
            Some(MessageScope::Task),
            "Task complete",
        ),
        (
            MessageType::TaskCompleted,
            Some(MessageScope::Goal),
            "Goal complete",
        ),
        (MessageType::Generic, None, "Ingest tests pass"),
    ];
    for (message_type, scope, expected) in rows {
        assert_eq!(
            title_phrase(message_type, scope, "Ingest tests pass"),
            expected
        );
        assert!(!expected.to_uppercase().contains("URGENT"));
    }
}

#[test]
fn an_urgent_message_keeps_its_title_unchanged() {
    for urgent in [false, true] {
        let message = stored(
            MessageType::DecisionRequired,
            urgent,
            Blocking::Blocking,
            json!({ "title": "Allow Bash to run the tests?" }),
        );
        assert_eq!(message.title, "Allow Bash to run the tests?");
        assert!(!message.title.to_lowercase().starts_with("urgent"));
    }
}

#[test]
fn blocking_badges_are_worded() {
    assert_eq!(
        blocking_badge_label(Blocking::Blocking),
        Some("Waiting on you")
    );
    assert_eq!(
        blocking_badge_label(Blocking::Deferrable),
        Some("Paused · can defer")
    );
    assert_eq!(blocking_badge_label(Blocking::NonBlocking), None);
}

fn parts<'a>(
    urgent: bool,
    blocking: Blocking,
    topic: &'a str,
    general: bool,
    thread: &'a str,
) -> SubtitleParts<'a> {
    SubtitleParts {
        urgent,
        blocking,
        topic_title: topic,
        topic_is_general: general,
        thread_title: thread,
    }
}

#[test]
fn native_subtitles_lead_with_urgent_then_the_blocking_badge() {
    let rows = [
        (
            parts(true, Blocking::Blocking, "gentbox", false, "Phase 3"),
            Some("Urgent · Waiting on you · gentbox · Phase 3"),
        ),
        (
            parts(false, Blocking::Blocking, "gentbox", false, "Phase 3"),
            Some("Waiting on you · gentbox · Phase 3"),
        ),
        (
            parts(true, Blocking::NonBlocking, "gentbox", false, "Phase 3"),
            Some("Urgent · gentbox · Phase 3"),
        ),
        (
            parts(false, Blocking::Deferrable, "gentbox", false, "Phase 3"),
            Some("Paused · can defer · gentbox · Phase 3"),
        ),
        (
            parts(false, Blocking::NonBlocking, "gentbox", false, "Phase 3"),
            Some("gentbox · Phase 3"),
        ),
        // A thread named like its topic is not repeated.
        (
            parts(false, Blocking::NonBlocking, "gentbox", false, "gentbox"),
            Some("gentbox"),
        ),
        (
            parts(false, Blocking::NonBlocking, "Gentbox", false, "gentbox"),
            Some("Gentbox"),
        ),
        // General is left out.
        (
            parts(
                false,
                Blocking::NonBlocking,
                "General",
                true,
                "Fix flaky ingest test",
            ),
            Some("Fix flaky ingest test"),
        ),
        (
            parts(
                true,
                Blocking::Blocking,
                "General",
                true,
                "Fix flaky ingest test",
            ),
            Some("Urgent · Waiting on you · Fix flaky ingest test"),
        ),
        (
            parts(false, Blocking::NonBlocking, "General", true, ""),
            None,
        ),
        (
            parts(true, Blocking::NonBlocking, "General", true, ""),
            Some("Urgent"),
        ),
    ];
    for (parts, expected) in rows {
        assert_eq!(
            notification_subtitle(&parts).as_deref(),
            expected,
            "{parts:?}"
        );
    }
}
