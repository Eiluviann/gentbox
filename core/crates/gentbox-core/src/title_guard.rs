//! The title guard of ADR 0010: a pure, deterministic check that title-like
//! text reads as plain words, and the rewrite that moves raw input into the
//! body.
//!
//! The heuristics err towards flagging: a false positive costs a less
//! specific title, never a lost message.

use crate::message::MessageType;
use crate::response::{IngestWarning, WarningCode};
use crate::secret_scan::REDACTION_MARKER;
use thiserror::Error;

/// Longest title a rewrite may produce.
pub const MAX_REWRITTEN_TITLE_CHARS: usize = 120;

/// Command names that flag a text when it starts with one and has more words.
const COMMAND_NAMES: &[&str] = &[
    "git",
    "npm",
    "pnpm",
    "npx",
    "yarn",
    "bun",
    "deno",
    "node",
    "cargo",
    "rustc",
    "make",
    "docker",
    "kubectl",
    "python",
    "python3",
    "pip",
    "go",
    "swift",
    "xcodebuild",
    "gh",
    "curl",
    "wget",
    "ssh",
    "sudo",
    "rm",
    "mv",
    "cp",
    "brew",
    "tsc",
    "vitest",
    "pytest",
];

/// Shell operators that flag a text (those with spaces need them).
const SHELL_OPERATORS: &[&str] = &[" | ", "&&", "||", "$(", " > ", " >> ", " < ", "2>"];

/// Why a text looks like raw input.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum GuardFinding {
    /// A backtick or code fence.
    CodeMark,
    /// A shell operator.
    ShellOperator,
    /// A command-line flag (`-p`, `--nocapture`).
    Flag,
    /// A file path.
    Path,
    /// An environment assignment (`NAME=value`).
    EnvironmentAssignment,
    /// A URL (`://`).
    Url,
    /// JSON (starts with `{` or `[`, or contains `":`).
    Json,
    /// A leading command name followed by arguments.
    LeadingCommand,
}

/// Returns the first reason `text` looks like raw input, or `None` when it
/// reads as plain words.
pub fn find_raw_input(text: &str) -> Option<GuardFinding> {
    let neutral = text.replace(REDACTION_MARKER, "redacted");
    let trimmed = neutral.trim();
    if trimmed.contains('`') {
        return Some(GuardFinding::CodeMark);
    }
    if SHELL_OPERATORS
        .iter()
        .any(|operator| trimmed.contains(operator))
    {
        return Some(GuardFinding::ShellOperator);
    }
    if trimmed.contains("://") {
        return Some(GuardFinding::Url);
    }
    if trimmed.starts_with('{') || trimmed.starts_with('[') || trimmed.contains("\":") {
        return Some(GuardFinding::Json);
    }
    let words: Vec<&str> = trimmed.split_whitespace().collect();
    if let Some(finding) = words.iter().find_map(|word| find_in_word(word)) {
        return Some(finding);
    }
    if starts_with_command(&words) {
        return Some(GuardFinding::LeadingCommand);
    }
    None
}

/// Whether `text` reads as plain words.
pub fn is_plain_words(text: &str) -> bool {
    find_raw_input(text).is_none()
}

fn find_in_word(word: &str) -> Option<GuardFinding> {
    let core = trim_word_punctuation(word);
    if is_flag(core) {
        Some(GuardFinding::Flag)
    } else if is_path(core) {
        Some(GuardFinding::Path)
    } else if is_environment_assignment(core) {
        Some(GuardFinding::EnvironmentAssignment)
    } else {
        None
    }
}

fn trim_word_punctuation(word: &str) -> &str {
    word.trim_start_matches(['(', '"', '\'', '[', '{', '<'])
        .trim_end_matches(['.', ',', ';', ':', '!', '?', ')', '"', '\'', ']', '}', '>'])
}

fn is_flag(word: &str) -> bool {
    let after_dashes = word.strip_prefix("--").or_else(|| word.strip_prefix('-'));
    after_dashes
        .and_then(|rest| rest.chars().next())
        .is_some_and(char::is_alphabetic)
}

fn is_path(word: &str) -> bool {
    if word.starts_with('/')
        || word.starts_with('~')
        || word.starts_with("./")
        || word.starts_with("../")
        || (word.starts_with('@') && word.contains('/'))
    {
        return true;
    }
    let slash_count = word.matches('/').count();
    slash_count >= 2 || (slash_count == 1 && has_file_extension(word))
}

fn has_file_extension(word: &str) -> bool {
    let last_component = word.rsplit('/').next().unwrap_or(word);
    match last_component.rsplit_once('.') {
        Some((stem, extension)) => {
            !stem.is_empty()
                && (1..=5).contains(&extension.len())
                && extension
                    .chars()
                    .all(|character| character.is_ascii_alphanumeric())
        }
        None => false,
    }
}

fn is_environment_assignment(word: &str) -> bool {
    let Some((name, _value)) = word.split_once('=') else {
        return false;
    };
    let mut characters = name.chars();
    characters
        .next()
        .is_some_and(|first| first.is_ascii_uppercase())
        && characters.all(|character| {
            character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_'
        })
}

fn starts_with_command(words: &[&str]) -> bool {
    match words {
        [first, _, ..] => COMMAND_NAMES.contains(first),
        _ => false,
    }
}

/// What to do when the guard flags something.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum GuardMode {
    /// Rewrite and warn (the default: rejecting a blocking prompt strands the
    /// agent).
    #[default]
    Rewrite,
    /// Answer 422 `title_looks_like_command` instead (`titleGuard = "reject"`).
    Reject,
}

/// A strict-mode rejection.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Error)]
pub enum GuardRejection {
    /// The message title looks like raw input.
    #[error("title_looks_like_command: the title looks like a command, path, URL or raw input")]
    TitleLooksLikeCommand(GuardFinding),
    /// The message summary looks like raw input.
    #[error("title_looks_like_command: the summary looks like a command, path, URL or raw input")]
    SummaryLooksLikeCommand(GuardFinding),
}

/// The texts of a message that surfaces show as lines or boxes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MessageTexts {
    /// The message title.
    pub title: String,
    /// The message summary.
    pub summary: Option<String>,
    /// The body, markdown.
    pub body: Option<String>,
}

/// What the rewrite needs to build a fallback title.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardContext {
    /// The message's final type.
    pub message_type: MessageType,
    /// The agent's display name, else its id.
    pub agent_name: String,
    /// The conversation's project name, when known.
    pub project: Option<String>,
}

/// The texts after the guard, with what was changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuardedTexts {
    /// A title that passes the guard.
    pub title: String,
    /// A summary that passes the guard, or none.
    pub summary: Option<String>,
    /// The body, with flagged originals on top as code boxes.
    pub body: Option<String>,
    /// `title_looks_like_command` / `summary_looks_like_command`.
    pub warnings: Vec<IngestWarning>,
}

/// Runs the guard over a message's title and summary.
///
/// Rewrite mode: a flagged title moves to the top of the body as an
/// "Original title" code box and is replaced by the summary's first sentence
/// (when the summary passes, cut to 120 characters at a word boundary), else
/// by a fallback phrase; a flagged summary moves to the body the same way and
/// is dropped. Strict mode returns the rejection instead. A rewritten title
/// or summary always passes the guard.
pub fn guard_message_texts(
    texts: MessageTexts,
    context: &GuardContext,
    mode: GuardMode,
) -> Result<GuardedTexts, GuardRejection> {
    let title_finding = find_raw_input(&texts.title);
    let summary_finding = texts.summary.as_deref().and_then(find_raw_input);
    if mode == GuardMode::Reject {
        if let Some(finding) = title_finding {
            return Err(GuardRejection::TitleLooksLikeCommand(finding));
        }
        if let Some(finding) = summary_finding {
            return Err(GuardRejection::SummaryLooksLikeCommand(finding));
        }
    }
    let mut warnings = Vec::new();
    let mut prepended = Vec::new();

    let kept_summary = match (&texts.summary, summary_finding) {
        (Some(summary), Some(_)) => {
            prepended.push(("Original summary", summary.clone()));
            warnings.push(IngestWarning::new(
				WarningCode::SummaryLooksLikeCommand,
				"summary",
				"The summary looked like a command, path, URL or raw input and was moved into the body. Write the summary as plain words.",
			));
            None
        }
        (Some(summary), None) => non_blank(summary),
        (None, _) => None,
    };

    let title = if title_finding.is_some() {
        prepended.insert(0, ("Original title", texts.title.clone()));
        let replacement = replacement_title(kept_summary.as_deref(), context);
        warnings.insert(
			0,
			IngestWarning::new(
				WarningCode::TitleLooksLikeCommand,
				"title",
				format!(
					"The title looked like a command, path, URL or raw input and was moved into the body; it now reads \"{replacement}\". Write titles as plain-word summaries."
				),
			),
		);
        replacement
    } else {
        texts.title
    };

    let body = prepend_code_boxes(texts.body, &prepended);
    Ok(GuardedTexts {
        title,
        summary: kept_summary,
        body,
        warnings,
    })
}

/// A conversation title that looks like raw input is ignored: subtitles fall
/// back to the project name. Returns the title to keep and the warning.
pub fn guard_conversation_title(title: Option<&str>) -> (Option<String>, Option<IngestWarning>) {
    match title {
        Some(title) if find_raw_input(title).is_some() => (
            None,
            Some(IngestWarning::new(
                WarningCode::ConversationTitleIgnored,
                "conversation.title",
                "The conversation title looked like a command, path, URL or raw input and was ignored.",
            )),
        ),
        Some(title) => (non_blank(title), None),
        None => (None, None),
    }
}

/// Turns a topic or thread key into words: `phase-3-ingest` becomes
/// "Phase 3 ingest"; dots between digits stay (`release-1.4` becomes
/// "Release 1.4"). The result always passes the guard.
pub fn title_from_key(key: &str) -> String {
    let characters: Vec<char> = key.chars().collect();
    let spaced: String = characters
        .iter()
        .enumerate()
        .map(|(index, character)| match character {
            '-' | '_' | ':' | '/' => ' ',
            '.' if is_dot_between_digits(&characters, index) => '.',
            '.' => ' ',
            other => *other,
        })
        .collect();
    let words = spaced.split_whitespace().collect::<Vec<_>>().join(" ");
    let title = capitalize_first(&words);
    if title.is_empty() || !is_plain_words(&title) {
        "Untitled".to_owned()
    } else {
        title
    }
}

/// A title for an inline topic or thread, or one from a rule template: the
/// given title when it passes the guard, else the key in words.
pub fn inline_group_title(key: &str, title: Option<&str>) -> String {
    match title.map(str::trim) {
        Some(title) if !title.is_empty() && is_plain_words(title) => title.to_owned(),
        _ => title_from_key(key),
    }
}

/// The fallback phrase for a message whose title and summary cannot serve.
///
/// "Claude Code needs your decision in gentbox", "Action needed in gentbox",
/// "Claude Code finished a task in gentbox", "Update from Claude Code". The
/// agent and project names are used only when they pass the guard; the
/// phrase itself always passes.
pub fn fallback_title(context: &GuardContext) -> String {
    let agent = if is_plain_words(&context.agent_name) && !context.agent_name.trim().is_empty() {
        context.agent_name.trim()
    } else {
        "The agent"
    };
    let project = context
        .project
        .as_deref()
        .map(str::trim)
        .filter(|project| !project.is_empty() && is_plain_words(project));
    let in_project = project
        .map(|project| format!(" in {project}"))
        .unwrap_or_default();
    let phrase = match context.message_type {
        MessageType::DecisionRequired => format!("{agent} needs your decision{in_project}"),
        MessageType::ActionRequired => format!("Action needed{in_project}"),
        MessageType::TaskCompleted => format!("{agent} finished a task{in_project}"),
        MessageType::Generic => format!("Update from {agent}"),
    };
    if is_plain_words(&phrase) {
        phrase
    } else {
        constant_fallback(context.message_type).to_owned()
    }
}

/// The first sentence of a text: up to the first `.`, `?` or `!` that is
/// followed by whitespace or the end. A trailing period is dropped.
pub fn first_sentence(text: &str) -> &str {
    let trimmed = text.trim();
    let mut characters = trimmed.char_indices().peekable();
    while let Some((index, character)) = characters.next() {
        if matches!(character, '.' | '?' | '!') {
            let ends_sentence = match characters.peek() {
                Some((_, next)) => next.is_whitespace(),
                None => true,
            };
            if ends_sentence {
                let end = index + character.len_utf8();
                let sentence = &trimmed[..end];
                return if character == '.' {
                    &sentence[..index]
                } else {
                    sentence
                };
            }
        }
    }
    trimmed
}

/// Cuts `text` to at most `max_chars` characters at a word boundary,
/// appending an ellipsis when it was shortened.
pub fn cut_at_word_boundary(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_owned();
    }
    let budget = max_chars.saturating_sub(1);
    let cut: String = text.chars().take(budget).collect();
    let ends_at_word_boundary = text[cut.len()..].starts_with(char::is_whitespace);
    let at_boundary = match cut.rfind(char::is_whitespace) {
        Some(position) if !ends_at_word_boundary => cut[..position].trim_end().to_owned(),
        _ => cut.trim_end().to_owned(),
    };
    format!("{at_boundary}…")
}

fn replacement_title(summary: Option<&str>, context: &GuardContext) -> String {
    let from_summary = summary
        .map(first_sentence)
        .map(|sentence| cut_at_word_boundary(sentence, MAX_REWRITTEN_TITLE_CHARS))
        .filter(|candidate| !candidate.trim().is_empty() && is_plain_words(candidate));
    from_summary.unwrap_or_else(|| fallback_title(context))
}

fn constant_fallback(message_type: MessageType) -> &'static str {
    match message_type {
        MessageType::DecisionRequired => "Your decision is needed",
        MessageType::ActionRequired => "Action needed",
        MessageType::TaskCompleted => "A task finished",
        MessageType::Generic => "An update arrived",
    }
}

fn prepend_code_boxes(body: Option<String>, boxes: &[(&str, String)]) -> Option<String> {
    if boxes.is_empty() {
        return body;
    }
    let mut result = boxes
        .iter()
        .map(|(heading, text)| format!("**{heading}**\n\n{}", code_box(text)))
        .collect::<Vec<_>>()
        .join("\n\n");
    if let Some(body) = body.filter(|body| !body.trim().is_empty()) {
        result.push_str("\n\n");
        result.push_str(&body);
    }
    Some(result)
}

/// Fences `text` as a code box, longer than any backtick run inside it.
pub fn code_box(text: &str) -> String {
    let longest_run = text
        .split(|character| character != '`')
        .map(str::len)
        .max()
        .unwrap_or(0);
    let fence = "`".repeat((longest_run + 1).max(3));
    format!("{fence}\n{text}\n{fence}")
}

fn non_blank(text: &str) -> Option<String> {
    let trimmed = text.trim();
    (!trimmed.is_empty()).then(|| trimmed.to_owned())
}

fn is_dot_between_digits(characters: &[char], index: usize) -> bool {
    let before = index
        .checked_sub(1)
        .and_then(|position| characters.get(position));
    let after = characters.get(index + 1);
    matches!((before, after), (Some(left), Some(right)) if left.is_ascii_digit() && right.is_ascii_digit())
}

fn capitalize_first(text: &str) -> String {
    let mut characters = text.chars();
    match characters.next() {
        Some(first) => first.to_uppercase().chain(characters).collect(),
        None => String::new(),
    }
}

/// Whether `text` starts with a command name from the guard's list, with or
/// without arguments (`npm login`, `cargo`).
pub fn starts_with_command_name(text: &str) -> bool {
    text.split_whitespace()
        .next()
        .is_some_and(|first| COMMAND_NAMES.contains(&first))
}
