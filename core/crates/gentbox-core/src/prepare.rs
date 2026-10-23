//! The pure part of the ingest pipeline (architecture §5.1), in order:
//! validate → classify → secret scan → title guard → action plan → hub
//! buttons. Topic and thread resolution, coalescing and persistence need hub
//! state and follow in the hub; [`PreparedMessage`] feeds them.

use thiserror::Error;
use time::OffsetDateTime;

use crate::action_plan::{ActionPlanMode, PlanError, prepare_action_plan};
use crate::actions::with_hub_actions;
use crate::classify::{BuiltInRules, Classification, RuleSet, RuleTemplate, classify};
use crate::ids::{MessageId, ThreadId, TopicId};
use crate::lifecycle::{Lifecycle, default_time_to_live};
use crate::message::{ActionStep, Message, MessageInput, MessageType};
use crate::resolve::GroupingRequest;
use crate::response::{IngestWarning, WarningCode};
use crate::secret_scan::{redact_optional, redact_secrets, redact_step};
use crate::template::{GroupSpec, TemplateVars};
use crate::title_guard::{
    GuardContext, GuardMode, GuardRejection, MessageTexts, guard_conversation_title,
    guard_message_texts,
};
use crate::validate::{ValidationError, validate_message};

/// Per-source strict modes (`titleGuard`, `actionPlan` in `config.toml`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct IngestSettings {
    /// What to do with a title that looks like raw input.
    pub title_guard: GuardMode,
    /// What to do with an `action_required` message without steps.
    pub action_plan: ActionPlanMode,
}

/// Why a message is refused.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum IngestRejection {
    /// Schema-level rules the JSON Schema cannot express (422).
    #[error(transparent)]
    Invalid(#[from] ValidationError),
    /// Strict title guard (422 `title_looks_like_command`).
    #[error(transparent)]
    TitleGuard(#[from] GuardRejection),
    /// Strict action plan (422 `steps_missing`).
    #[error(transparent)]
    ActionPlan(#[from] PlanError),
}

/// A message that passed the pure pipeline.
#[derive(Debug, Clone, PartialEq)]
pub struct PreparedMessage {
    /// The message with every field final: type, urgency, blocking and scope
    /// resolved, texts guarded and redacted, steps and buttons complete, the
    /// conversation title guarded.
    pub message: MessageInput,
    /// How the fields were classified.
    pub classification: Classification,
    /// The topic a rule assigned, with its placeholders filled in.
    pub rule_topic: Option<GroupSpec>,
    /// The thread a rule assigned, with its placeholders filled in.
    pub rule_thread: Option<GroupSpec>,
    /// What the hub changed, for the 201 response.
    pub warnings: Vec<IngestWarning>,
}

/// Runs the pure pipeline over a posted message.
pub fn prepare_message(
    input: MessageInput,
    user_rules: &RuleSet,
    built_in_rules: &BuiltInRules,
    settings: IngestSettings,
) -> Result<PreparedMessage, IngestRejection> {
    validate_message(&input)?;
    let classification = classify(&input, user_rules, built_in_rules);
    let mut warnings = classification.normalization_warnings();
    let message_type = classification.message_type.value;

    let mut message = input;
    redact_texts(&mut message, &mut warnings);

    let (conversation_title, conversation_warning) =
        guard_conversation_title(message.conversation.title.as_deref());
    message.conversation.title = conversation_title;
    warnings.extend(conversation_warning);

    let context = GuardContext {
        message_type,
        agent_name: message
            .source
            .display_name
            .clone()
            .unwrap_or_else(|| message.source.agent.clone()),
        project: message.conversation.project_name().map(str::to_owned),
    };
    let guarded = guard_message_texts(
        MessageTexts {
            title: message.title,
            summary: message.summary,
            body: message.body,
        },
        &context,
        settings.title_guard,
    )?;
    message.title = guarded.title;
    message.summary = guarded.summary;
    message.body = guarded.body;
    warnings.extend(guarded.warnings);

    let steps = prepare_steps(&mut message, &classification, settings, &mut warnings)?;
    message.steps = steps;
    message.message_type = Some(message_type);
    message.urgent = Some(classification.urgent.value);
    message.blocking = Some(classification.blocking.value);
    message.scope = classification.scope;
    message.actions = Some(with_hub_actions(
        message.actions.take(),
        message_type,
        classification.blocking.value,
    ));

    let vars = TemplateVars::for_message(
        &message.source,
        &message.conversation,
        message.conversation.title.as_deref(),
    );
    let rule_topic = render_rule_template(classification.rule_topic.as_ref(), &vars);
    let rule_thread = render_rule_template(classification.rule_thread.as_ref(), &vars);
    Ok(PreparedMessage {
        message,
        classification,
        rule_topic,
        rule_thread,
        warnings,
    })
}

impl PreparedMessage {
    /// The request the hub passes to [`crate::resolve::resolve_grouping`].
    pub fn grouping_request(&self) -> GroupingRequest<'_> {
        GroupingRequest {
            agent: &self.message.source.agent,
            conversation_id: &self.message.conversation.id,
            topic: self.message.topic.as_ref(),
            thread: self.message.thread.as_ref(),
            rule_topic: self.rule_topic.as_ref(),
            rule_thread: self.rule_thread.as_ref(),
            conversation_title: self.message.conversation.title.as_deref(),
            project: self.message.conversation.project_name(),
        }
    }

    /// Builds the stored message once the hub has assigned an id and resolved
    /// the grouping. `expires_at` defaults by type and blocking level.
    pub fn into_message(
        self,
        id: MessageId,
        received_at: OffsetDateTime,
        topic_id: TopicId,
        thread_id: ThreadId,
    ) -> Message {
        let Self {
            message: input,
            classification,
            ..
        } = self;
        let message_type = classification.message_type.value;
        let blocking = classification.blocking.value;
        Message {
            id,
            idempotency_key: input.idempotency_key,
            source: input.source,
            conversation: input.conversation,
            topic_id,
            thread_id,
            message_type,
            urgent: classification.urgent.value,
            blocking,
            field_sources: classification.field_sources(),
            muted_by_rule: classification.muted_by_rule,
            event: input.event,
            title: input.title,
            summary: input.summary,
            body: input.body,
            steps: input.steps.unwrap_or_default(),
            steps_completed: Vec::new(),
            actions: input.actions.unwrap_or_default(),
            scope: classification.scope,
            awaits_response: input.awaits_response,
            correlation_key: input.correlation_key,
            sensitive: input.sensitive,
            created_at: input.created_at,
            received_at,
            expires_at: input
                .expires_at
                .unwrap_or_else(|| received_at + default_time_to_live(message_type, blocking)),
            metadata: input.metadata.unwrap_or_default(),
            lifecycle: Lifecycle::born(message_type),
            repeat_count: 0,
        }
    }
}

fn redact_texts(message: &mut MessageInput, warnings: &mut Vec<IngestWarning>) {
    let title = redact_secrets(&message.title);
    note_redactions(warnings, "title", title.redactions);
    message.title = title.text;

    let (summary, count) = redact_optional(message.summary.as_deref());
    note_redactions(warnings, "summary", count);
    message.summary = summary;

    let (body, count) = redact_optional(message.body.as_deref());
    note_redactions(warnings, "body", count);
    message.body = body;

    if let Some(steps) = message.steps.take() {
        let redacted = steps
            .iter()
            .enumerate()
            .map(|(index, step)| {
                let (step, count) = redact_step(step);
                note_redactions(warnings, &format!("steps[{index}]"), count);
                step
            })
            .collect();
        message.steps = Some(redacted);
    }
}

fn note_redactions(warnings: &mut Vec<IngestWarning>, field: &str, count: usize) {
    if count > 0 {
        warnings.push(IngestWarning::new(
			WarningCode::SecretRedacted,
			field,
			format!("{count} secret(s) in {field} were replaced by [redacted]; never put secrets in a message."),
		));
    }
}

fn prepare_steps(
    message: &mut MessageInput,
    classification: &Classification,
    settings: IngestSettings,
    warnings: &mut Vec<IngestWarning>,
) -> Result<Option<Vec<ActionStep>>, PlanError> {
    if classification.message_type.value != MessageType::ActionRequired {
        return Ok(None);
    }
    let focus_url = message
        .conversation
        .focus
        .as_ref()
        .and_then(|focus| focus.url.as_deref());
    let plan = prepare_action_plan(message.steps.take(), focus_url, settings.action_plan)?;
    warnings.extend(plan.warnings);
    Ok(Some(plan.steps))
}

fn render_rule_template(template: Option<&RuleTemplate>, vars: &TemplateVars) -> Option<GroupSpec> {
    template.and_then(|assigned| assigned.template.render(vars))
}
