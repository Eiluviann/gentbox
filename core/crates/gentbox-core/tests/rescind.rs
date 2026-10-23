//! Rescind by id and by correlation keys, and its effects per channel and state.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::ids::{ConversationKey, MessageId};
use gentbox_core::item::{DeliveredSurfaces, OpenItem};
use gentbox_core::lifecycle::LifecycleState;
use gentbox_core::message::{Blocking, MessageType};
use gentbox_core::rescind::{
    ChannelEffect, RescindError, RescindReason, RescindRequest, rescind_by_correlation_keys,
    rescind_by_id, rescission_for,
};
use serde_json::json;
use support::{item_of, stored};

use ChannelEffect::{
    MarkWithdrawn, RemoveNativeNotification, RemovePopup, RemoveToast, RetractPhoneNotification,
    WakeWaiter,
};

fn popup_item(correlation_key: &str) -> OpenItem {
    item_of(
        &stored(
            MessageType::DecisionRequired,
            false,
            Blocking::Blocking,
            json!({ "awaitsResponse": true, "correlationKey": correlation_key }),
        ),
        false,
    )
}

#[test]
fn rescinding_by_id_works_for_each_reason() {
    for reason in RescindReason::ALL {
        let item = popup_item("k");
        let rescission = rescind_by_id(&item, reason, false).unwrap();
        assert_eq!(rescission.message_id, item.id);
        assert_eq!(rescission.reason, reason);
        // With no successor named, even a superseded row stays visible as "Withdrawn".
        assert!(rescission.effects.contains(&MarkWithdrawn(reason)));
    }
}

#[test]
fn reasons_are_worded_for_the_inbox() {
    assert_eq!(
        RescindReason::ResolvedElsewhere.in_words(),
        "Handled in the session"
    );
    assert_eq!(RescindReason::Obsolete.in_words(), "No longer needed");
    assert_eq!(
        RescindReason::Superseded.in_words(),
        "Replaced by a newer message"
    );
}

#[test]
fn an_open_popup_item_removes_popup_and_native_notification_and_keeps_the_inbox_row() {
    let rescission =
        rescind_by_id(&popup_item("k"), RescindReason::ResolvedElsewhere, false).unwrap();
    assert_eq!(
        rescission.effects,
        vec![
            RemovePopup,
            RemoveNativeNotification,
            MarkWithdrawn(RescindReason::ResolvedElsewhere)
        ]
    );
}

#[test]
fn a_toast_item_removes_the_toast_only() {
    let item = item_of(
        &stored(
            MessageType::TaskCompleted,
            false,
            Blocking::NonBlocking,
            json!({}),
        ),
        false,
    );
    let rescission = rescind_by_id(&item, RescindReason::Obsolete, false).unwrap();
    assert_eq!(
        rescission.effects,
        vec![RemoveToast, MarkWithdrawn(RescindReason::Obsolete)]
    );
}

#[test]
fn an_inbox_only_item_just_gets_the_withdrawn_mark() {
    let item = item_of(
        &stored(
            MessageType::Generic,
            false,
            Blocking::NonBlocking,
            json!({}),
        ),
        false,
    );
    let rescission = rescind_by_id(&item, RescindReason::Obsolete, false).unwrap();
    assert_eq!(
        rescission.effects,
        vec![MarkWithdrawn(RescindReason::Obsolete)]
    );
}

#[test]
fn an_acknowledged_item_has_no_popup_left_to_remove_but_its_native_notification_may_remain() {
    let mut item = popup_item("k");
    item.state = LifecycleState::Acknowledged;
    let rescission = rescind_by_id(&item, RescindReason::Obsolete, false).unwrap();
    assert_eq!(
        rescission.effects,
        vec![
            RemoveNativeNotification,
            MarkWithdrawn(RescindReason::Obsolete)
        ]
    );
}

#[test]
fn a_live_waiter_is_woken_with_rescinded_and_never_with_an_answer() {
    let mut item = popup_item("k");
    item.has_live_waiter = true;
    let rescission = rescind_by_id(&item, RescindReason::ResolvedElsewhere, false).unwrap();
    assert_eq!(
        rescission.effects.last(),
        Some(&WakeWaiter(RescindReason::ResolvedElsewhere))
    );
    // Without a waiter nobody is woken.
    let quiet = rescind_by_id(&popup_item("k"), RescindReason::ResolvedElsewhere, false).unwrap();
    assert!(
        !quiet
            .effects
            .iter()
            .any(|effect| matches!(effect, WakeWaiter(_)))
    );
}

#[test]
fn a_superseded_row_folds_into_its_successor() {
    let item = popup_item("k");
    let successor = MessageId::new("SUCCESSOR");
    let rescission = rescission_for(&item, RescindReason::Superseded, Some(&successor), false);
    assert!(
        rescission
            .effects
            .contains(&ChannelEffect::FoldIntoSuccessor(successor))
    );
    assert!(
        !rescission
            .effects
            .iter()
            .any(|effect| matches!(effect, MarkWithdrawn(_)))
    );
}

#[test]
fn phone_entries_appear_only_when_a_phone_channel_exists() {
    let item = popup_item("k");
    let without = rescind_by_id(&item, RescindReason::Obsolete, false).unwrap();
    assert!(!without.effects.contains(&RetractPhoneNotification));
    let with = rescind_by_id(&item, RescindReason::Obsolete, true).unwrap();
    assert!(with.effects.contains(&RetractPhoneNotification));
}

#[test]
fn a_closed_message_cannot_be_rescinded() {
    for state in [
        LifecycleState::Answered,
        LifecycleState::Dismissed,
        LifecycleState::Rescinded,
        LifecycleState::Expired,
    ] {
        let mut item = popup_item("k");
        item.state = state;
        assert_eq!(
            rescind_by_id(&item, RescindReason::Obsolete, false),
            Err(RescindError::AlreadyClosed { state })
        );
    }
}

#[test]
fn rescinding_by_correlation_keys_hits_only_the_listed_keys_of_the_conversation() {
    let conversation = ConversationKey::new("claude-code", "conversation-1");
    let a = popup_item("ask-relay-name");
    let b = popup_item("publish-plan");
    let c = popup_item("other");
    let mut other_conversation = popup_item("ask-relay-name");
    other_conversation.conversation = ConversationKey::new("claude-code", "conversation-2");
    other_conversation.id = MessageId::new("OTHER");
    let mut closed = popup_item("ask-relay-name");
    closed.state = LifecycleState::Answered;
    closed.id = MessageId::new("CLOSED");

    let request = RescindRequest {
        reason: RescindReason::Obsolete,
        correlation_keys: Some(vec!["ask-relay-name".to_owned(), "publish-plan".to_owned()]),
    };
    let items = [a.clone(), b.clone(), c, other_conversation, closed];
    let rescissions = rescind_by_correlation_keys(&conversation, &request, &items, false).unwrap();
    let ids: Vec<_> = rescissions.iter().map(|r| r.message_id.clone()).collect();
    assert_eq!(ids, vec![a.id, b.id]);
    assert!(
        rescissions
            .iter()
            .all(|r| r.reason == RescindReason::Obsolete)
    );
}

#[test]
fn rescinding_by_keys_needs_keys() {
    let conversation = ConversationKey::new("claude-code", "conversation-1");
    for correlation_keys in [None, Some(vec![])] {
        let request = RescindRequest {
            reason: RescindReason::Obsolete,
            correlation_keys,
        };
        assert_eq!(
            rescind_by_correlation_keys(&conversation, &request, &[popup_item("k")], false),
            Err(RescindError::MissingCorrelationKeys)
        );
    }
}

#[test]
fn rescinding_a_key_nobody_holds_is_a_no_op() {
    let conversation = ConversationKey::new("claude-code", "conversation-1");
    let request = RescindRequest {
        reason: RescindReason::Obsolete,
        correlation_keys: Some(vec!["missing".to_owned()]),
    };
    assert_eq!(
        rescind_by_correlation_keys(&conversation, &request, &[popup_item("k")], false),
        Ok(vec![])
    );
}

#[test]
fn delivered_surfaces_follow_the_plan() {
    let item = popup_item("k");
    assert_eq!(
        item.delivered,
        DeliveredSurfaces {
            popup: true,
            toast: false,
            native_notification: true
        }
    );
}

#[test]
fn rescind_requests_parse_like_the_schema_examples() {
    let request: RescindRequest = serde_json::from_value(
        json!({ "reason": "obsolete", "correlationKeys": ["ask-relay-name"] }),
    )
    .unwrap();
    assert_eq!(request.reason, RescindReason::Obsolete);
    assert!(
        serde_json::from_value::<RescindRequest>(json!({ "reason": "obsolete", "extra": 1 }))
            .is_err()
    );
    assert!(serde_json::from_value::<RescindRequest>(json!({ "reason": "because" })).is_err());
}
