//! Rule templates for topics and threads (architecture §5.3): strings with
//! `{project}`, `{agent}` and `{conversation}` placeholders.

use serde::Deserialize;
use thiserror::Error;

use crate::message::{Conversation, Source};
use crate::title_guard::inline_group_title;

/// The placeholders a template may use.
pub const PLACEHOLDERS: [&str; 3] = ["project", "agent", "conversation"];
/// Longest key a topic or thread may have.
pub const MAX_GROUP_KEY_CHARS: usize = 96;
/// Longest kind hint a topic or thread may have.
pub const MAX_GROUP_KIND_CHARS: usize = 32;

/// A topic or thread a rule assigns, before its placeholders are filled in.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GroupTemplate {
    /// The stable key, e.g. `project:{project}`.
    pub key: String,
    /// The title, e.g. `{project}`; without it the key becomes the title.
    #[serde(default)]
    pub title: Option<String>,
    /// The kind hint, e.g. `project`.
    #[serde(default)]
    pub kind: Option<String>,
}

/// A topic or thread with its placeholders filled in and its title through
/// the guard.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupSpec {
    /// A valid stable key.
    pub key: String,
    /// A title that passes the title guard.
    pub title: String,
    /// The kind hint.
    pub kind: Option<String>,
}

/// The values placeholders stand for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemplateVars {
    /// `{project}`: the working directory's name.
    pub project: Option<String>,
    /// `{agent}`: the agent's display name, else its id.
    pub agent: String,
    /// `{conversation}`: the conversation title (after the guard).
    pub conversation: Option<String>,
}

impl TemplateVars {
    /// The variables of a message's conversation. `conversation_title` is the
    /// title after the guard (a flagged one is already gone).
    pub fn for_message(
        source: &Source,
        conversation: &Conversation,
        conversation_title: Option<&str>,
    ) -> Self {
        Self {
            project: conversation.project_name().map(str::to_owned),
            agent: source
                .display_name
                .clone()
                .unwrap_or_else(|| source.agent.clone()),
            conversation: conversation_title.map(str::to_owned),
        }
    }

    fn value(&self, name: &str) -> Option<&str> {
        match name {
            "project" => self.project.as_deref(),
            "agent" => Some(self.agent.as_str()),
            "conversation" => self.conversation.as_deref(),
            _ => None,
        }
    }
}

/// Why a rule template is unusable. Found when the rules file is parsed, so
/// the user sees it in Settings instead of a rule that silently assigns
/// nothing.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum TemplateError {
    /// A `{placeholder}` that is not `project`, `agent` or `conversation`.
    #[error("unknown placeholder `{{{0}}}`")]
    UnknownPlaceholder(String),
    /// A `{` without a closing `}`.
    #[error("a `{{` has no closing `}}`")]
    Unterminated,
    /// The key can never be a valid key, whatever the placeholders stand for.
    #[error(
        "the key `{0}` can never be valid: keys use lower-case letters, digits and . _ : / - and have at most 96 characters"
    )]
    InvalidKey(String),
    /// The kind can never be a valid kind, whatever the placeholders stand for.
    #[error(
        "the kind `{0}` can never be valid: kinds use lower-case letters, digits, _ and - and have at most 32 characters"
    )]
    InvalidKind(String),
}

/// Checks that every `{placeholder}` in a template string is known.
///
/// # Errors
/// Returns the first unknown placeholder, or `Unterminated` for a `{` without
/// a closing `}`.
pub fn check_placeholders(template: &str) -> Result<(), TemplateError> {
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else {
            return Err(TemplateError::Unterminated);
        };
        let name = &after[..end];
        if !PLACEHOLDERS.contains(&name) {
            return Err(TemplateError::UnknownPlaceholder(name.to_owned()));
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

impl GroupTemplate {
    /// Checks the template: every placeholder is known, and the key and kind
    /// are valid for any value the placeholders may take (a key such as
    /// `Project:{project}` can never match the key pattern).
    ///
    /// # Errors
    /// Returns what is wrong with the template.
    pub fn check(&self) -> Result<(), TemplateError> {
        check_placeholders(&self.key)?;
        self.title.as_deref().map_or(Ok(()), check_placeholders)?;
        self.kind.as_deref().map_or(Ok(()), check_placeholders)?;
        let sample = TemplateVars {
            project: Some("x".to_owned()),
            agent: "x".to_owned(),
            conversation: Some("x".to_owned()),
        };
        let sample_key = fill(&self.key, &sample, slugify);
        if !sample_key.as_deref().is_some_and(is_valid_group_key) {
            return Err(TemplateError::InvalidKey(self.key.clone()));
        }
        if let Some(kind) = &self.kind {
            let sample_kind = fill(kind, &sample, slugify);
            if !sample_kind.as_deref().is_some_and(is_valid_group_kind) {
                return Err(TemplateError::InvalidKind(kind.clone()));
            }
        }
        Ok(())
    }

    /// Fills the placeholders. Returns `None` when a placeholder has no value
    /// (a `{project}` for a conversation with no working directory) or the
    /// resulting key is not a valid key, so the rule assigns nothing.
    ///
    /// Values are lower-cased and reduced to key characters inside the key;
    /// the title keeps them as written and goes through the title guard, with
    /// the key in words as the fallback.
    pub fn render(&self, vars: &TemplateVars) -> Option<GroupSpec> {
        let key: String = fill(&self.key, vars, slugify)?
            .chars()
            .take(MAX_GROUP_KEY_CHARS)
            .collect();
        if !is_valid_group_key(&key) {
            return None;
        }
        let title = match self.title.as_deref() {
            Some(template) => fill(template, vars, keep_as_written),
            None => None,
        };
        let kind = match self.kind.as_deref() {
            Some(template) => {
                let kind: String = fill(template, vars, slugify)?
                    .chars()
                    .take(MAX_GROUP_KIND_CHARS)
                    .collect();
                if !is_valid_group_kind(&kind) {
                    return None;
                }
                Some(kind)
            }
            None => None,
        };
        Some(GroupSpec {
            title: inline_group_title(&key, title.as_deref()),
            key,
            kind,
        })
    }
}

/// Whether `key` matches `^[a-z0-9][a-z0-9._:/-]{0,95}$`, the topic and
/// thread key pattern of the schema.
pub fn is_valid_group_key(key: &str) -> bool {
    let mut characters = key.chars();
    let first_is_valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit());
    first_is_valid
        && key.chars().count() <= MAX_GROUP_KEY_CHARS
        && key.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '.' | '_' | ':' | '/' | '-')
        })
}

/// Whether `kind` matches `^[a-z0-9][a-z0-9_-]{0,31}$`, the kind pattern of
/// the schema.
pub fn is_valid_group_kind(kind: &str) -> bool {
    let mut characters = kind.chars();
    let first_is_valid = characters
        .next()
        .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit());
    first_is_valid
        && kind.chars().count() <= MAX_GROUP_KIND_CHARS
        && kind.chars().all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '_' | '-')
        })
}

fn fill(
    template: &str,
    vars: &TemplateVars,
    transform: fn(&str) -> Option<String>,
) -> Option<String> {
    let mut result = String::new();
    let mut rest = template;
    while let Some(start) = rest.find('{') {
        result.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        let end = after.find('}')?;
        let value = vars.value(&after[..end])?;
        result.push_str(&transform(value)?);
        rest = &after[end + 1..];
    }
    result.push_str(rest);
    Some(result)
}

fn keep_as_written(value: &str) -> Option<String> {
    Some(value.to_owned())
}

/// Reduces a value to key characters; `None` when nothing is left.
fn slugify(value: &str) -> Option<String> {
    let mapped: String = value
        .chars()
        .map(|character| {
            let lower = character.to_ascii_lowercase();
            if lower.is_ascii_lowercase() || lower.is_ascii_digit() || matches!(lower, '.' | '_') {
                lower
            } else {
                '-'
            }
        })
        .collect();
    let mut slug = String::new();
    for character in mapped.chars() {
        if character == '-' && slug.ends_with('-') {
            continue;
        }
        slug.push(character);
    }
    let slug = slug.trim_matches('-');
    (!slug.is_empty()).then(|| slug.to_owned())
}
