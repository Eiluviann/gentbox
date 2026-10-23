//! Action plans (ADR 0011): validation and normalisation of the `steps` of an
//! `action_required` message.

use thiserror::Error;

use crate::message::{ActionStep, StepLink};
use crate::response::{IngestWarning, WarningCode};
use crate::title_guard::{code_box, is_plain_words, starts_with_command_name};

/// The single step the hub adds when a plan is missing.
pub const FALLBACK_STEP_TEXT: &str = "Open the conversation and follow the agent's instructions";
/// A step whose text was a command and moved into its detail.
pub const RUN_COMMAND_STEP_TEXT: &str = "Run the command below";
/// A step whose text was raw input and moved into its detail.
pub const FOLLOW_DETAILS_STEP_TEXT: &str = "Follow the details below";
/// Most steps a plan may have.
pub const MAX_STEPS: usize = 12;
/// Longest step text.
pub const MAX_STEP_TEXT_CHARS: usize = 200;
/// Longest step detail.
pub const MAX_STEP_DETAIL_CHARS: usize = 2000;
/// Schemes a step link may use besides `https`.
pub const STEP_LINK_SCHEMES: [&str; 5] = ["https", "claude-cli", "claude", "vscode", "cursor"];
/// The scheme that opens System Settings, allowed on step links only.
pub const SYSTEM_SETTINGS_SCHEME: &str = "x-apple.systempreferences";

/// What to do with a missing plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum ActionPlanMode {
    /// Add the fallback step and warn (the default).
    #[default]
    Rewrite,
    /// Answer 422 `steps_missing` (`actionPlan = "reject"`).
    Reject,
}

/// A rejected plan.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum PlanError {
    /// A plan has 1 to 12 steps.
    #[error("a plan needs between 1 and {MAX_STEPS} steps, got {count}")]
    StepCount {
        /// How many steps were given.
        count: usize,
    },
    /// A step's text is empty or too long.
    #[error("step {index}: text must be 1 to {MAX_STEP_TEXT_CHARS} characters")]
    TextLength {
        /// Zero-based step index.
        index: usize,
    },
    /// A step's detail is too long.
    #[error("step {index}: detail must be at most {MAX_STEP_DETAIL_CHARS} characters")]
    DetailLength {
        /// Zero-based step index.
        index: usize,
    },
    /// A step's link uses a scheme that is not allowed.
    #[error("step {index}: link scheme is not allowed")]
    LinkScheme {
        /// Zero-based step index.
        index: usize,
    },
    /// Strict mode: an `action_required` message has no plan.
    #[error("steps_missing: an action_required message needs steps")]
    StepsMissing,
    /// A tick named a step that does not exist.
    #[error("step {index} does not exist")]
    UnknownStep {
        /// Zero-based step index.
        index: usize,
    },
}

/// A plan ready to store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PreparedPlan {
    /// The steps, never empty; every `text` passes the title guard.
    pub steps: Vec<ActionStep>,
    /// `steps_missing` and `step_rewritten` warnings.
    pub warnings: Vec<IngestWarning>,
}

/// Validates the shape of a plan: 1 to 12 steps, bounded texts, link schemes.
pub fn validate_steps(steps: &[ActionStep]) -> Result<(), PlanError> {
    if steps.is_empty() || steps.len() > MAX_STEPS {
        return Err(PlanError::StepCount { count: steps.len() });
    }
    for (index, step) in steps.iter().enumerate() {
        let text_length = step.text.chars().count();
        if step.text.trim().is_empty() || text_length > MAX_STEP_TEXT_CHARS {
            return Err(PlanError::TextLength { index });
        }
        if step
            .detail
            .as_ref()
            .is_some_and(|detail| detail.chars().count() > MAX_STEP_DETAIL_CHARS)
        {
            return Err(PlanError::DetailLength { index });
        }
        if step
            .link
            .as_ref()
            .is_some_and(|link| !is_allowed_link_scheme(&link.url))
        {
            return Err(PlanError::LinkScheme { index });
        }
    }
    Ok(())
}

/// Whether a step link's URL uses `https` or an allowlisted scheme.
pub fn is_allowed_link_scheme(url: &str) -> bool {
    let Some((scheme, _)) = url.split_once(':') else {
        return false;
    };
    STEP_LINK_SCHEMES.contains(&scheme) || scheme == SYSTEM_SETTINGS_SCHEME
}

/// Prepares the plan of an `action_required` message.
///
/// - a missing plan gets the fallback step, linked to the conversation's
///   focus URL when it has an allowed one, with a `steps_missing` warning;
///   strict mode rejects it instead;
/// - a step `text` that fails the title guard moves into its `detail` as a
///   code box and is replaced by "Run the command below" (when it starts
///   with a known command) or "Follow the details below", with a
///   `step_rewritten` warning.
pub fn prepare_action_plan(
    steps: Option<Vec<ActionStep>>,
    focus_url: Option<&str>,
    mode: ActionPlanMode,
) -> Result<PreparedPlan, PlanError> {
    let Some(steps) = steps else {
        return missing_plan(focus_url, mode);
    };
    validate_steps(&steps)?;
    let mut warnings = Vec::new();
    let steps = steps
        .into_iter()
        .enumerate()
        .map(|(index, step)| rewrite_step(index, step, &mut warnings))
        .collect();
    Ok(PreparedPlan { steps, warnings })
}

/// Ticks or unticks a step. Ticks are your progress, never an answer.
///
/// Returns the ascending, duplicate-free list of ticked indices.
pub fn set_step_ticked(
    completed: &[usize],
    index: usize,
    is_ticked: bool,
    step_count: usize,
) -> Result<Vec<usize>, PlanError> {
    if index >= step_count {
        return Err(PlanError::UnknownStep { index });
    }
    let mut next: Vec<usize> = completed
        .iter()
        .copied()
        .filter(|&existing| existing != index && existing < step_count)
        .collect();
    if is_ticked {
        next.push(index);
    }
    next.sort_unstable();
    Ok(next)
}

fn missing_plan(focus_url: Option<&str>, mode: ActionPlanMode) -> Result<PreparedPlan, PlanError> {
    if mode == ActionPlanMode::Reject {
        return Err(PlanError::StepsMissing);
    }
    let link = focus_url
        .filter(|url| is_allowed_link_scheme(url))
        .map(|url| StepLink {
            url: url.to_owned(),
            label: None,
        });
    Ok(PreparedPlan {
        steps: vec![ActionStep {
            text: FALLBACK_STEP_TEXT.to_owned(),
            detail: None,
            link,
        }],
        warnings: vec![IngestWarning::new(
            WarningCode::StepsMissing,
            "steps",
            "An action_required message needs steps that explain how to complete the action; a single step pointing at the conversation was added.",
        )],
    })
}

fn rewrite_step(index: usize, step: ActionStep, warnings: &mut Vec<IngestWarning>) -> ActionStep {
    if is_plain_words(&step.text) {
        return step;
    }
    let replacement = if starts_with_command_name(&step.text) {
        RUN_COMMAND_STEP_TEXT
    } else {
        FOLLOW_DETAILS_STEP_TEXT
    };
    let moved = code_box(&step.text);
    let detail = match step.detail {
        Some(existing) if !existing.trim().is_empty() => format!("{moved}\n\n{existing}"),
        _ => moved,
    };
    warnings.push(IngestWarning::new(
		WarningCode::StepRewritten,
		format!("steps[{index}].text"),
		"A step's text looked like a command, path, URL or raw input and was moved into the step's detail. Put commands in a step's detail and keep its text a plain instruction.",
	));
    ActionStep {
        text: replacement.to_owned(),
        detail: Some(detail),
        link: step.link,
    }
}
