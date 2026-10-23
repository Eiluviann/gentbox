//! Policy engine table tests (architecture §6).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::time::Duration;

use gentbox_core::message::{Blocking, MessageType};
use gentbox_core::policy::{
    BlockingBadge, DeliveryPlan, MacSurface, NativeLevel, PhonePlan, PlanReason, Policy,
    PolicyCell, PolicyContext, PolicyKey, PresenceState, Reping, SortTier, decide,
};
use serde_json::json;
use support::{awaiting, calm_context, stored, valid_combinations};

fn plan_for(
    message_type: MessageType,
    urgent: bool,
    blocking: Blocking,
    context: &PolicyContext,
) -> DeliveryPlan {
    decide(
        &awaiting(message_type, urgent, blocking),
        context,
        &Policy::default(),
    )
}

#[test]
fn there_are_fifteen_valid_combinations() {
    assert_eq!(valid_combinations().len(), 15);
}

/// The base cells of architecture §6.2, calm context: type, blocking, mac, native, sound, re-ping.
#[test]
fn base_cells_follow_the_defaults_table() {
    let five = Some(Reping {
        after: Duration::from_secs(300),
        max: 3,
    });
    let ten = Some(Reping {
        after: Duration::from_secs(600),
        max: 2,
    });
    let rows = [
        (
            MessageType::DecisionRequired,
            Blocking::Blocking,
            MacSurface::PersistentPopup,
            true,
            true,
            five,
        ),
        (
            MessageType::ActionRequired,
            Blocking::Blocking,
            MacSurface::PersistentPopup,
            true,
            true,
            five,
        ),
        (
            MessageType::DecisionRequired,
            Blocking::Deferrable,
            MacSurface::PersistentPopup,
            true,
            true,
            ten,
        ),
        (
            MessageType::ActionRequired,
            Blocking::Deferrable,
            MacSurface::PersistentPopup,
            true,
            true,
            ten,
        ),
        (
            MessageType::DecisionRequired,
            Blocking::NonBlocking,
            MacSurface::Banner,
            true,
            true,
            None,
        ),
        (
            MessageType::ActionRequired,
            Blocking::NonBlocking,
            MacSurface::Banner,
            true,
            true,
            None,
        ),
        (
            MessageType::TaskCompleted,
            Blocking::NonBlocking,
            MacSurface::Toast,
            false,
            true,
            None,
        ),
        (
            MessageType::Generic,
            Blocking::NonBlocking,
            MacSurface::InboxOnly,
            false,
            false,
            None,
        ),
    ];
    for (message_type, blocking, mac, has_native, sound, reping) in rows {
        let plan = plan_for(message_type, false, blocking, &calm_context());
        assert_eq!(plan.mac, mac, "{message_type:?} {blocking:?}");
        assert_eq!(
            plan.native_notification != NativeLevel::None,
            has_native,
            "{message_type:?} {blocking:?} native"
        );
        assert_eq!(plan.sound, sound, "{message_type:?} {blocking:?} sound");
        assert_eq!(plan.reping, reping, "{message_type:?} {blocking:?} reping");
    }
}

#[test]
fn defer_is_offered_only_on_deferrable() {
    for (message_type, urgent, blocking) in valid_combinations() {
        let plan = plan_for(message_type, urgent, blocking, &calm_context());
        assert_eq!(
            plan.offer_defer,
            blocking == Blocking::Deferrable,
            "{message_type:?} urgent={urgent} {blocking:?}"
        );
    }
}

#[test]
fn defer_needs_a_waiter_to_be_offered() {
    let message = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Deferrable,
        json!({}),
    );
    let plan = decide(&message, &calm_context(), &Policy::default());
    assert!(!plan.should_wait);
    assert!(!plan.offer_defer);
}

#[test]
fn urgent_is_time_sensitive_and_badged_only_when_urgent() {
    for (message_type, urgent, blocking) in valid_combinations() {
        let plan = plan_for(message_type, urgent, blocking, &calm_context());
        assert_eq!(plan.urgent_badge, urgent);
        assert_eq!(
            plan.sort_tier,
            if urgent {
                SortTier::Urgent
            } else {
                SortTier::Normal
            }
        );
        match plan.native_notification {
            NativeLevel::TimeSensitive => assert!(urgent, "only urgent is Time Sensitive"),
            NativeLevel::Active => assert!(!urgent),
            NativeLevel::None => {}
        }
        if urgent && plan.native_notification != NativeLevel::None {
            assert_eq!(plan.native_notification, NativeLevel::TimeSensitive);
        }
    }
}

#[test]
fn urgent_sorts_above_normal() {
    assert!(SortTier::Urgent < SortTier::Normal);
}

#[test]
fn blocking_badge_follows_the_blocking_level_independent_of_urgency() {
    for (message_type, urgent, blocking) in valid_combinations() {
        let plan = plan_for(message_type, urgent, blocking, &calm_context());
        let expected = match blocking {
            Blocking::NonBlocking => BlockingBadge::None,
            Blocking::Deferrable => BlockingBadge::PausedCanDefer,
            Blocking::Blocking => BlockingBadge::WaitingOnYou,
        };
        assert_eq!(plan.blocking_badge, expected);
    }
}

/// What the policy does in a calm context, written out literally per cell of
/// architecture §6.2 (for a message that awaits a response).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Expected {
    mac: MacSurface,
    native: NativeLevel,
    sound: bool,
    reping: Option<Reping>,
    offer_defer: bool,
    should_wait: bool,
}

fn calm_table() -> Vec<((MessageType, bool, Blocking), Expected)> {
    use MacSurface::{Banner, InboxOnly, PersistentPopup, Toast};
    use NativeLevel::{Active, None as NoNative, TimeSensitive};
    let five = Some(Reping {
        after: Duration::from_secs(300),
        max: 3,
    });
    let ten = Some(Reping {
        after: Duration::from_secs(600),
        max: 2,
    });
    let row = |mac, native, sound, reping, offer_defer, should_wait| Expected {
        mac,
        native,
        sound,
        reping,
        offer_defer,
        should_wait,
    };
    let mut table = Vec::new();
    for message_type in [MessageType::DecisionRequired, MessageType::ActionRequired] {
        for (urgent, native) in [(false, Active), (true, TimeSensitive)] {
            table.push((
                (message_type, urgent, Blocking::Blocking),
                row(PersistentPopup, native, true, five, false, true),
            ));
            table.push((
                (message_type, urgent, Blocking::Deferrable),
                row(PersistentPopup, native, true, ten, true, true),
            ));
            table.push((
                (message_type, urgent, Blocking::NonBlocking),
                row(Banner, native, true, None, false, true),
            ));
        }
    }
    for urgent in [false, true] {
        table.push((
            (MessageType::TaskCompleted, urgent, Blocking::NonBlocking),
            row(Toast, NoNative, true, None, false, false),
        ));
    }
    table.push((
        (MessageType::Generic, false, Blocking::NonBlocking),
        row(InboxOnly, NoNative, false, None, false, false),
    ));
    table
}

/// The spec's modulation rules, applied to a literal calm cell (independent of `decide`).
fn modulate(calm: Expected, urgent: bool, focus: bool, quiet: bool, muted: bool) -> Expected {
    let mut expected = calm;
    if urgent {
        return expected;
    }
    if muted {
        return Expected {
            mac: MacSurface::InboxOnly,
            native: NativeLevel::None,
            sound: false,
            reping: None,
            offer_defer: false,
            should_wait: false,
        };
    }
    let inbox_only = |expected: &mut Expected| {
        expected.mac = MacSurface::InboxOnly;
        expected.native = NativeLevel::None;
    };
    if quiet {
        if matches!(expected.mac, MacSurface::Toast | MacSurface::Banner) {
            inbox_only(&mut expected);
        }
        expected.sound = false;
    }
    if focus {
        if expected.mac == MacSurface::Toast {
            inbox_only(&mut expected);
        }
        expected.sound = false;
    }
    expected
}

fn observed(plan: &DeliveryPlan) -> Expected {
    Expected {
        mac: plan.mac,
        native: plan.native_notification,
        sound: plan.sound,
        reping: plan.reping,
        offer_defer: plan.offer_defer,
        should_wait: plan.should_wait,
    }
}

#[test]
fn the_calm_table_has_all_fifteen_cells_and_decide_matches_it_literally() {
    let table = calm_table();
    assert_eq!(table.len(), 15);
    for ((message_type, urgent, blocking), expected) in table {
        let plan = plan_for(message_type, urgent, blocking, &calm_context());
        assert_eq!(
            observed(&plan),
            expected,
            "{message_type:?} urgent={urgent} {blocking:?}"
        );
    }
}

/// Every cell × presence × Focus × quiet hours, against the literal table.
#[test]
fn full_matrix_of_cells_presence_focus_and_quiet_hours() {
    let mut checked = 0;
    for ((message_type, urgent, blocking), calm) in calm_table() {
        for state in [
            PresenceState::Present,
            PresenceState::Away,
            PresenceState::Locked,
        ] {
            for focus in [false, true] {
                for quiet in [false, true] {
                    let mut context = calm_context();
                    context.presence.state = state;
                    context.focus_active = focus;
                    context.quiet_hours_active = quiet;
                    let plan = plan_for(message_type, urgent, blocking, &context);
                    let label = format!(
                        "{message_type:?} urgent={urgent} {blocking:?} {state:?} focus={focus} quiet={quiet}"
                    );
                    assert_eq!(
                        observed(&plan),
                        modulate(calm, urgent, focus, quiet, false),
                        "{label}"
                    );
                    // No phone channel exists: no phone part, whatever the presence.
                    assert_eq!(plan.phone, PhonePlan::None, "{label}");
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 180);
}

#[test]
fn quiet_hours_and_focus_break_through_only_when_urgent() {
    for (message_type, blocking) in [
        (MessageType::DecisionRequired, Blocking::Blocking),
        (MessageType::DecisionRequired, Blocking::NonBlocking),
        (MessageType::TaskCompleted, Blocking::NonBlocking),
    ] {
        let mut context = calm_context();
        context.quiet_hours_active = true;
        context.focus_active = true;
        let urgent = plan_for(message_type, true, blocking, &context);
        let calm = plan_for(message_type, true, blocking, &calm_context());
        assert_eq!(urgent.mac, calm.mac);
        assert!(urgent.sound);
        assert!(urgent.reasons.contains(&PlanReason::UrgentBreakthrough));
        let quiet = plan_for(message_type, false, blocking, &context);
        assert!(!quiet.sound);
        assert!(!quiet.reasons.contains(&PlanReason::UrgentBreakthrough));
        assert!(quiet.reasons.contains(&PlanReason::QuietHours));
        assert!(quiet.reasons.contains(&PlanReason::Focus));
    }
}

#[test]
fn muted_non_urgent_is_inbox_only_silent_and_never_waits() {
    for (message_type, urgent, blocking) in valid_combinations() {
        let mut context = calm_context();
        context.is_muted = true;
        let plan = plan_for(message_type, urgent, blocking, &context);
        let calm = plan_for(message_type, urgent, blocking, &calm_context());
        if urgent {
            assert_eq!(plan, calm, "urgent is unaffected by mute");
        } else {
            assert_eq!(plan.mac, MacSurface::InboxOnly);
            assert_eq!(plan.native_notification, NativeLevel::None);
            assert!(!plan.sound);
            assert_eq!(plan.reping, None);
            assert!(!plan.should_wait, "{message_type:?} {blocking:?}");
            assert!(!plan.offer_defer);
            assert_eq!(plan.reasons, vec![PlanReason::Muted]);
        }
    }
}

#[test]
fn not_muted_answerable_messages_wait() {
    for (message_type, urgent, blocking) in valid_combinations() {
        let plan = plan_for(message_type, urgent, blocking, &calm_context());
        assert_eq!(
            plan.should_wait,
            message_type.is_answerable(),
            "{message_type:?}"
        );
    }
}

#[test]
fn a_message_that_does_not_await_never_waits() {
    let message = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({}),
    );
    assert!(!decide(&message, &calm_context(), &Policy::default()).should_wait);
}

#[test]
fn completion_toast_is_suppressed_when_the_conversations_app_is_frontmost() {
    let message = stored(
        MessageType::TaskCompleted,
        false,
        Blocking::NonBlocking,
        json!({ "conversation": { "id": "c", "focus": { "bundleId": "com.mitchellh.ghostty" } } }),
    );
    let mut context = calm_context();
    context.presence.frontmost_bundle_id = Some("com.mitchellh.ghostty".to_owned());
    let plan = decide(&message, &context, &Policy::default());
    assert_eq!(plan.mac, MacSurface::InboxOnly);
    assert!(!plan.sound);
    assert!(plan.reasons.contains(&PlanReason::FrontmostSuppression));

    context.presence.frontmost_bundle_id = Some("com.apple.Safari".to_owned());
    assert_eq!(
        decide(&message, &context, &Policy::default()).mac,
        MacSurface::Toast
    );

    context.presence.frontmost_bundle_id = None;
    assert_eq!(
        decide(&message, &context, &Policy::default()).mac,
        MacSurface::Toast
    );
}

#[test]
fn frontmost_suppression_applies_only_while_present_and_only_to_toasts() {
    let completed = stored(
        MessageType::TaskCompleted,
        false,
        Blocking::NonBlocking,
        json!({ "conversation": { "id": "c", "focus": { "bundleId": "app" } } }),
    );
    let mut context = calm_context();
    context.presence.frontmost_bundle_id = Some("app".to_owned());
    for state in [PresenceState::Away, PresenceState::Locked] {
        context.presence.state = state;
        assert_eq!(
            decide(&completed, &context, &Policy::default()).mac,
            MacSurface::Toast
        );
    }
    let blocking = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({ "awaitsResponse": true, "conversation": { "id": "c", "focus": { "bundleId": "app" } } }),
    );
    context.presence.state = PresenceState::Present;
    assert_eq!(
        decide(&blocking, &context, &Policy::default()).mac,
        MacSurface::PersistentPopup,
        "a blocking prompt is never suppressed by the frontmost app"
    );
}

#[test]
fn without_a_phone_channel_every_plan_has_no_phone_part() {
    for (message_type, urgent, blocking) in valid_combinations() {
        assert_eq!(
            plan_for(message_type, urgent, blocking, &calm_context()).phone,
            PhonePlan::None
        );
    }
}

#[test]
fn the_policy_table_is_configurable_per_cell() {
    let key = PolicyKey {
        message_type: MessageType::DecisionRequired,
        urgent: false,
        blocking: Blocking::Blocking,
    };
    let policy = Policy::default().with_cell(
        key,
        PolicyCell {
            surface: MacSurface::Banner,
            native_notification: true,
            sound: false,
            reping: None,
            offer_defer: false,
        },
    );
    let message = awaiting(MessageType::DecisionRequired, false, Blocking::Blocking);
    let plan = decide(&message, &calm_context(), &policy);
    assert_eq!(plan.mac, MacSurface::Banner);
    assert!(!plan.sound);
    assert_eq!(plan.reping, None);
}

#[test]
fn kind_and_topic_hints_never_change_a_decision() {
    // A different thread, topic and event leave the plan unchanged.
    let first = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({ "awaitsResponse": true, "thread": { "key": "phase-1", "kind": "phase" } }),
    );
    let second = stored(
        MessageType::DecisionRequired,
        false,
        Blocking::Blocking,
        json!({ "awaitsResponse": true, "thread": { "key": "v2", "kind": "version" } }),
    );
    assert_eq!(
        decide(&first, &calm_context(), &Policy::default()),
        decide(&second, &calm_context(), &Policy::default())
    );
}

#[test]
fn a_rule_mute_acts_exactly_like_topic_or_thread_mute_for_every_combination() {
    let policy = Policy::default();
    let mut checked = 0;
    for ((message_type, urgent, blocking), calm) in calm_table() {
        for state in [
            PresenceState::Present,
            PresenceState::Away,
            PresenceState::Locked,
        ] {
            for focus in [false, true] {
                for quiet in [false, true] {
                    let mut context = calm_context();
                    context.presence.state = state;
                    context.focus_active = focus;
                    context.quiet_hours_active = quiet;
                    let label = format!(
                        "{message_type:?} urgent={urgent} {blocking:?} {state:?} focus={focus} quiet={quiet}"
                    );

                    let mut rule_muted = awaiting(message_type, urgent, blocking);
                    rule_muted.muted_by_rule = Some("Mute auth success".to_owned());
                    let by_rule = decide(&rule_muted, &context, &policy);
                    assert_eq!(
                        observed(&by_rule),
                        modulate(calm, urgent, focus, quiet, true),
                        "{label}"
                    );
                    if !urgent {
                        assert_eq!(by_rule.reasons, vec![PlanReason::Muted], "{label}");
                    }

                    let topic_muted = awaiting(message_type, urgent, blocking);
                    let mut muted_context = context.clone();
                    muted_context.is_muted = true;
                    assert_eq!(
                        by_rule,
                        decide(&topic_muted, &muted_context, &policy),
                        "{label}: the same plan as a muted topic"
                    );
                    checked += 1;
                }
            }
        }
    }
    assert_eq!(checked, 180);
}

#[test]
fn a_rule_muted_blocking_prompt_keeps_its_type_and_levels_but_does_not_hang_its_sender() {
    use gentbox_core::classify::{BuiltInRules, RuleSet};
    use gentbox_core::ids::{MessageId, ThreadId, TopicId};
    use gentbox_core::prepare::{IngestSettings, prepare_message};

    let rules = RuleSet::parse("[[rule]]\nname = \"Mute prompts\"\nmatch = { event = \"claude-code.permission_request\" }\nmute = true\n").unwrap();
    let prepared = prepare_message(
        support::input(
            json!({ "event": "claude-code.permission_request", "awaitsResponse": true }),
        ),
        &rules,
        BuiltInRules::load(),
        IngestSettings::default(),
    )
    .unwrap();
    let message = prepared.into_message(
        MessageId::new("M"),
        support::NOW,
        TopicId::new("T"),
        ThreadId::new("H"),
    );
    assert_eq!(message.message_type, MessageType::DecisionRequired);
    assert_eq!(message.blocking, Blocking::Blocking);
    assert_eq!(message.muted_by_rule.as_deref(), Some("Mute prompts"));
    let plan = decide(&message, &calm_context(), &Policy::default());
    assert_eq!(plan.mac, MacSurface::InboxOnly);
    assert!(!plan.should_wait);
    assert_eq!(
        plan.blocking_badge,
        BlockingBadge::WaitingOnYou,
        "it is still a blocking item in the inbox"
    );

    let mut urgent = message.clone();
    urgent.urgent = true;
    let plan = decide(&urgent, &calm_context(), &Policy::default());
    assert_eq!(plan.mac, MacSurface::PersistentPopup);
    assert!(plan.should_wait);
}
