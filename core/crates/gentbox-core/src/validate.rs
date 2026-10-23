//! Validation of the rules the JSON Schema cannot express, run after schema
//! validation and before classification.

use thiserror::Error;

use crate::MESSAGE_SCHEMA_VERSION;
use crate::action_plan::{PlanError, validate_steps};
use crate::message::{MessageInput, MessageType};

/// How a declared combination of fields is invalid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CombinationViolation {
    /// `task_completed` or `generic` with a blocking level above `non_blocking`.
    BlockingOnNonAnswerableType(MessageType),
    /// `generic` and urgent.
    UrgentGeneric,
    /// `scope` on a type other than `task_completed`.
    ScopeOnWrongType(MessageType),
    /// `steps` on a type other than `action_required`.
    StepsOnWrongType(MessageType),
}

/// A message the hub must refuse with 422.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ValidationError {
    /// Only schema version 1 is accepted.
    #[error("unsupported schema version {0}")]
    UnsupportedVersion(u32),
    /// Declared fields contradict each other.
    #[error("invalid_combination: {0:?}")]
    InvalidCombination(CombinationViolation),
    /// Two buttons share an id.
    #[error("action id `{0}` is used twice")]
    DuplicateActionId(String),
    /// The action plan is malformed.
    #[error(transparent)]
    InvalidSteps(#[from] PlanError),
    /// `rescinds` on a message names no correlation key.
    #[error("`rescinds` needs at least one correlation key")]
    RescindsWithoutCorrelationKeys,
    /// `expiresAt` is not after `createdAt`.
    #[error("`expiresAt` must be after `createdAt`")]
    ExpiresBeforeCreated,
}

impl ValidationError {
    /// The 422 `code` of the error.
    pub fn code(&self) -> &'static str {
        match self {
            Self::InvalidCombination(_) => "invalid_combination",
            _ => "schema_invalid",
        }
    }
}

/// Checks what the schema cannot say: the declared combination, unique
/// action ids, the action plan, `rescinds` and the expiry.
pub fn validate_message(message: &MessageInput) -> Result<(), ValidationError> {
    if message.version != MESSAGE_SCHEMA_VERSION {
        return Err(ValidationError::UnsupportedVersion(message.version));
    }
    check_declared_combination(message)?;
    check_action_ids(message)?;
    if let Some(steps) = &message.steps {
        validate_steps(steps)?;
    }
    if let Some(rescinds) = &message.rescinds {
        let has_keys = rescinds
            .correlation_keys
            .as_ref()
            .is_some_and(|keys| !keys.is_empty());
        if !has_keys {
            return Err(ValidationError::RescindsWithoutCorrelationKeys);
        }
    }
    if let (Some(created), Some(expires)) = (message.created_at, message.expires_at)
        && expires <= created
    {
        return Err(ValidationError::ExpiresBeforeCreated);
    }
    Ok(())
}

fn check_declared_combination(message: &MessageInput) -> Result<(), ValidationError> {
    let Some(message_type) = message.message_type else {
        return Ok(());
    };
    let violation =
        if !message_type.can_block() && message.blocking.is_some_and(|level| level.is_waiting()) {
            Some(CombinationViolation::BlockingOnNonAnswerableType(
                message_type,
            ))
        } else if !message_type.can_be_urgent() && message.urgent == Some(true) {
            Some(CombinationViolation::UrgentGeneric)
        } else if message.scope.is_some() && message_type != MessageType::TaskCompleted {
            Some(CombinationViolation::ScopeOnWrongType(message_type))
        } else if message.steps.is_some() && message_type != MessageType::ActionRequired {
            Some(CombinationViolation::StepsOnWrongType(message_type))
        } else {
            None
        };
    match violation {
        Some(violation) => Err(ValidationError::InvalidCombination(violation)),
        None => Ok(()),
    }
}

fn check_action_ids(message: &MessageInput) -> Result<(), ValidationError> {
    let Some(actions) = &message.actions else {
        return Ok(());
    };
    let mut seen: Vec<&str> = Vec::new();
    for action in actions {
        if seen.contains(&action.id.as_str()) {
            return Err(ValidationError::DuplicateActionId(action.id.clone()));
        }
        seen.push(&action.id);
    }
    Ok(())
}
