//! The message model: what a sender posts ([`MessageInput`], schema
//! `message-v1.json`) and what the hub stores once the message has been
//! classified, grouped and guarded ([`Message`]).

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};
use time::OffsetDateTime;

use crate::classify::FieldSources;
use crate::ids::{ConversationKey, MessageId, ThreadId, TopicId};
use crate::lifecycle::Lifecycle;
use crate::rescind::RescindRequest;
use crate::topic::{ThreadRef, TopicRef};

/// What a message asks of you (ADR 0007).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageType {
    /// The agent needs your answer: approve or deny, pick a choice, reply.
    DecisionRequired,
    /// You must do something yourself outside the conversation, then say Done.
    ActionRequired,
    /// Work finished; `scope: goal` marks the agent's overall objective.
    TaskCompleted,
    /// Everything else, kept for reference.
    Generic,
}

impl MessageType {
    /// Every type, in schema order.
    pub const ALL: [Self; 4] = [
        Self::DecisionRequired,
        Self::ActionRequired,
        Self::TaskCompleted,
        Self::Generic,
    ];

    /// The snake_case name used on the wire and in rules.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::DecisionRequired => "decision_required",
            Self::ActionRequired => "action_required",
            Self::TaskCompleted => "task_completed",
            Self::Generic => "generic",
        }
    }

    /// Whether the agent can wait for an answer to this type of message.
    pub fn is_answerable(self) -> bool {
        matches!(self, Self::DecisionRequired | Self::ActionRequired)
    }

    /// Whether the type may be urgent: `generic` never is.
    pub fn can_be_urgent(self) -> bool {
        self != Self::Generic
    }

    /// Whether the type may carry a blocking level above `non_blocking`.
    pub fn can_block(self) -> bool {
        self.is_answerable()
    }
}

/// What the agent does meanwhile (ADR 0007). Ordered by how loud the level is:
/// `NonBlocking < Deferrable < Blocking`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Blocking {
    /// The agent keeps working and planned for the answer to come later.
    NonBlocking,
    /// The agent paused; you may Defer so it continues with work needing no input.
    Deferrable,
    /// Work cannot continue until you decide.
    Blocking,
}

impl Blocking {
    /// Every level, from quietest to loudest.
    pub const ALL: [Self; 3] = [Self::NonBlocking, Self::Deferrable, Self::Blocking];

    /// The snake_case name used on the wire and in rules.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::NonBlocking => "non_blocking",
            Self::Deferrable => "deferrable",
            Self::Blocking => "blocking",
        }
    }

    /// Whether the agent is paused on this message (`deferrable` or `blocking`).
    pub fn is_waiting(self) -> bool {
        self != Self::NonBlocking
    }
}

/// For `task_completed`: one finished task or the agent's overall goal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MessageScope {
    /// One finished task (the default).
    Task,
    /// The agent reached its overall objective.
    Goal,
}

/// Who sent a message.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Source {
    /// Stable lowercase agent id (`claude-code`).
    pub agent: String,
    /// Human-readable agent name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub display_name: Option<String>,
    /// Agent version.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Machine name when the sender runs elsewhere than the hub.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub host: Option<String>,
}

/// The agent session a message belongs to.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conversation {
    /// The agent's session id.
    pub id: String,
    /// Descriptive conversation title (passes the title guard or is ignored).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// Working directory; its last component is the project name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cwd: Option<String>,
    /// How the `open` action brings the conversation forward.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub focus: Option<FocusTarget>,
}

impl Conversation {
    /// The project name: the last component of the working directory.
    pub fn project_name(&self) -> Option<&str> {
        let cwd = self.cwd.as_deref()?;
        cwd.trim_end_matches('/')
            .rsplit('/')
            .find(|component| !component.is_empty())
    }
}

/// How the `open` action brings a conversation forward: URL, then terminal,
/// then the app's bundle id.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct FocusTarget {
    /// Deep link opening the conversation (allowlisted schemes only).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub url: Option<String>,
    /// macOS bundle id of the app hosting the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub bundle_id: Option<String>,
    /// The terminal tab hosting the session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal: Option<TerminalTarget>,
}

/// The terminal a session runs in.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TerminalTarget {
    /// The tty device, `/dev/ttys004`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tty: Option<String>,
    /// The tmux pane, `%3`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tmux_pane: Option<String>,
    /// The agent's process id.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pid: Option<u32>,
}

/// What a button does.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionKind {
    /// Answer: allow.
    Approve,
    /// Answer: refuse.
    Deny,
    /// Answer: one of the sender's choices, returning its `value`.
    Choice,
    /// Answer: free text.
    Reply,
    /// Answer: you did what an `action_required` message asked.
    Done,
    /// Answer that does not close: keep working on what needs no input.
    Defer,
    /// Non-answer: acknowledge and bring the conversation forward.
    Open,
    /// Non-answer: a local reminder after `value` minutes.
    Snooze,
}

impl ActionKind {
    /// Approve, deny, choice, reply and done close the message.
    pub fn is_answer(self) -> bool {
        matches!(
            self,
            Self::Approve | Self::Deny | Self::Choice | Self::Reply | Self::Done
        )
    }

    /// Whether the waiting sender hears about this action: the answers and
    /// defer. `open` and `snooze` never reach it.
    pub fn reaches_sender(self) -> bool {
        self.is_answer() || self == Self::Defer
    }
}

/// A button.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Action {
    /// Unique within the message; returned to the sender as `actionId`.
    pub id: String,
    /// Button label.
    pub label: String,
    /// What the button does.
    pub kind: ActionKind,
    /// For `choice` the value returned; for `snooze` the minutes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value: Option<String>,
    /// For `reply`: the text field's placeholder.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub placeholder: Option<String>,
    /// Shown in the destructive style and never a one-tap button.
    #[serde(default, skip_serializing_if = "is_false")]
    pub destructive: bool,
}

/// A link on an action step.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct StepLink {
    /// `https` or an allowlisted scheme; opened only on tap.
    pub url: String,
    /// Link label.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub label: Option<String>,
}

/// One step of an action plan (ADR 0011).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ActionStep {
    /// One plain instruction that passes the title guard.
    pub text: String,
    /// Markdown: commands in code blocks, paths, explanations.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    /// A link opened on tap.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub link: Option<StepLink>,
}

/// A flat metadata value.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum MetadataValue {
    /// A string.
    Text(String),
    /// A number.
    Number(serde_json::Number),
    /// A boolean.
    Flag(bool),
}

impl MetadataValue {
    /// The value as text, the form rules compare against.
    pub fn to_comparable_text(&self) -> String {
        match self {
            Self::Text(text) => text.clone(),
            Self::Number(number) => number.to_string(),
            Self::Flag(flag) => flag.to_string(),
        }
    }
}

/// Flat sender facts for rules and display.
pub type Metadata = BTreeMap<String, MetadataValue>;

/// A message as the sender posts it (`POST /v1/messages`).
///
/// Mirrors `message-v1.json`; the JSON Schema is validated by the ingest
/// layer, the rules it cannot express by [`crate::validate::validate_message`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct MessageInput {
    /// Schema major version; must be [`crate::MESSAGE_SCHEMA_VERSION`].
    pub version: u32,
    /// Sender-chosen retry key, unique per `source.agent`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Who sent it.
    pub source: Source,
    /// The agent session it belongs to.
    pub conversation: Conversation,
    /// The topic it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub topic: Option<TopicRef>,
    /// The thread it names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub thread: Option<ThreadRef>,
    /// Declared type; rules fill it when missing.
    #[serde(default, rename = "type", skip_serializing_if = "Option::is_none")]
    pub message_type: Option<MessageType>,
    /// Declared urgency; rules fill it when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub urgent: Option<bool>,
    /// Declared blocking level; rules fill it when missing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub blocking: Option<Blocking>,
    /// The sender's own dotted event name, for rules.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// A descriptive summary in plain words (ADR 0010).
    pub title: String,
    /// What is needed or what was done, in plain words.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Markdown for the detail view; the place for commands and paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The action plan of an `action_required` message.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub steps: Option<Vec<ActionStep>>,
    /// Buttons, in display order.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub actions: Option<Vec<Action>>,
    /// For `task_completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MessageScope>,
    /// Whether the sender intends to long-poll for the answer.
    #[serde(default, skip_serializing_if = "is_false")]
    pub awaits_response: bool,
    /// The slot this message occupies in its conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_key: Option<String>,
    /// Withdraws other open messages of the conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rescinds: Option<RescindRequest>,
    /// Even the title must not leave the Mac.
    #[serde(default, skip_serializing_if = "is_false")]
    pub sensitive: bool,
    /// When the sender produced the event.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub created_at: Option<OffsetDateTime>,
    /// After this instant the message expires.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub expires_at: Option<OffsetDateTime>,
    /// Flat sender facts for rules and display.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub metadata: Option<Metadata>,
}

/// A stored message: classified, grouped, guarded and tracked through its
/// [`Lifecycle`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Message {
    /// Hub-assigned id.
    pub id: MessageId,
    /// Sender-chosen retry key.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub idempotency_key: Option<String>,
    /// Who sent it.
    pub source: Source,
    /// The agent session it belongs to.
    pub conversation: Conversation,
    /// The topic it is grouped under (denormalised from its thread).
    pub topic_id: TopicId,
    /// The thread it is in.
    pub thread_id: ThreadId,
    /// Final type.
    #[serde(rename = "type")]
    pub message_type: MessageType,
    /// Final urgency.
    pub urgent: bool,
    /// Final blocking level; Defer turns `deferrable` into `non_blocking`.
    pub blocking: Blocking,
    /// Where each of the three fields came from.
    pub field_sources: FieldSources,
    /// A `mute` rule matched: the policy routes it like a message in a muted
    /// topic. Type, urgency and blocking level are unchanged.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub muted_by_rule: Option<String>,
    /// The sender's event name.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub event: Option<String>,
    /// Descriptive title, as guarded.
    pub title: String,
    /// Descriptive summary, as guarded.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub summary: Option<String>,
    /// Markdown body, secret-scanned.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub body: Option<String>,
    /// The action plan; never empty for `action_required`.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps: Vec<ActionStep>,
    /// Indices of ticked steps, ascending; progress, never an answer.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub steps_completed: Vec<usize>,
    /// Buttons, including the ones the hub added (Done, Defer).
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub actions: Vec<Action>,
    /// For `task_completed`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scope: Option<MessageScope>,
    /// Whether the sender declared it would long-poll.
    pub awaits_response: bool,
    /// The slot this message occupies in its conversation.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correlation_key: Option<String>,
    /// Even the title must not leave the Mac.
    pub sensitive: bool,
    /// When the sender produced the event.
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        with = "time::serde::rfc3339::option"
    )]
    pub created_at: Option<OffsetDateTime>,
    /// When the hub received it; messages are ordered by this.
    #[serde(with = "time::serde::rfc3339")]
    pub received_at: OffsetDateTime,
    /// When the message expires.
    #[serde(with = "time::serde::rfc3339")]
    pub expires_at: OffsetDateTime,
    /// Flat sender facts.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub metadata: Metadata,
    /// Lifecycle state and its timestamps.
    pub lifecycle: Lifecycle,
    /// How many identical or folded messages this one absorbed.
    pub repeat_count: u32,
}

impl Message {
    /// The conversation this message belongs to.
    pub fn conversation_key(&self) -> ConversationKey {
        ConversationKey::new(&self.source.agent, &self.conversation.id)
    }

    /// Whether the message is still open or acknowledged.
    pub fn is_live(&self) -> bool {
        self.lifecycle.state.is_live()
    }
}

fn is_false(value: &bool) -> bool {
    !*value
}
