//! The buttons of a message: the ones the sender declared plus the ones the
//! hub adds by itself (Done on `action_required`, Defer on `deferrable`), and
//! which of them are offered right now.

use crate::message::{Action, ActionKind, Blocking, Message, MessageType};

/// Id the hub gives the Done button it adds.
pub const DONE_ACTION_ID: &str = "done";
/// Id the hub gives the Defer button it adds.
pub const DEFER_ACTION_ID: &str = "defer";

/// The stored buttons of a message: the declared ones plus Done on every
/// `action_required` message and Defer on every `deferrable` one, each added
/// only when the sender did not declare that kind.
pub fn with_hub_actions(
    declared: Option<Vec<Action>>,
    message_type: MessageType,
    blocking: Blocking,
) -> Vec<Action> {
    let mut actions = declared.unwrap_or_default();
    if message_type == MessageType::ActionRequired {
        add_missing(&mut actions, ActionKind::Done, DONE_ACTION_ID, "Done");
    }
    if blocking == Blocking::Deferrable {
        add_missing(&mut actions, ActionKind::Defer, DEFER_ACTION_ID, "Defer");
    }
    actions
}

/// The buttons to show now.
///
/// - Answer buttons (approve, deny, choice, reply) and Defer need a live
///   waiter, so a button never pretends to do something.
/// - **Done is always offered on `action_required`**, with or without a live
///   waiter: with one it tells the agent at once, without one it closes the
///   item locally and records the answer for a later poll. There is no
///   second way to finish such an item.
/// - Defer is offered only while the message is `deferrable`: after Defer it
///   is `non_blocking` and the button is gone.
/// - `open` and `snooze` are always offered.
pub fn offered_actions(message: &Message, has_live_waiter: bool) -> Vec<&Action> {
    if !message.is_live() {
        return Vec::new();
    }
    message
        .actions
        .iter()
        .filter(|action| match action.kind {
            ActionKind::Open | ActionKind::Snooze => true,
            ActionKind::Done if message.message_type == MessageType::ActionRequired => true,
            ActionKind::Defer => has_live_waiter && message.blocking == Blocking::Deferrable,
            _ => has_live_waiter,
        })
        .collect()
}

fn add_missing(actions: &mut Vec<Action>, kind: ActionKind, preferred_id: &str, label: &str) {
    if actions.iter().any(|action| action.kind == kind) {
        return;
    }
    let id = unique_id(actions, preferred_id);
    actions.push(Action {
        id,
        label: label.to_owned(),
        kind,
        value: None,
        placeholder: None,
        destructive: false,
    });
}

fn unique_id(actions: &[Action], preferred_id: &str) -> String {
    let is_taken = |candidate: &str| actions.iter().any(|action| action.id == candidate);
    if !is_taken(preferred_id) {
        return preferred_id.to_owned();
    }
    (2..)
        .map(|suffix| format!("{preferred_id}-{suffix}"))
        .find(|candidate| !is_taken(candidate))
        .unwrap_or_else(|| preferred_id.to_owned())
}
