//! The secret scan of ADR 0011: known token and key patterns are replaced by
//! `[redacted]` in every text field before anything is stored or sent.

use std::sync::OnceLock;

use regex::Regex;

use crate::message::ActionStep;

/// What a redacted secret is replaced by.
pub const REDACTION_MARKER: &str = "[redacted]";

/// One secret pattern: what to find and what to put back (`$1` keeps a
/// leading label such as `password=`).
struct SecretPattern {
    regex: Regex,
    replacement: &'static str,
}

const SECRET_PATTERNS: &[(&str, &str)] = &[
    // Private keys: the whole block when its end is present, else to the end.
    (
        r"(?s)-----BEGIN [A-Z ]*PRIVATE KEY-----.*?(?:-----END [A-Z ]*PRIVATE KEY-----|\z)",
        REDACTION_MARKER,
    ),
    // GitHub tokens.
    (r"\bgh[pos]_[A-Za-z0-9]{20,}", REDACTION_MARKER),
    (r"\bgithub_pat_[A-Za-z0-9_]{20,}", REDACTION_MARKER),
    // Anthropic and OpenAI-style keys.
    (r"\bsk-ant-[A-Za-z0-9_-]{8,}", REDACTION_MARKER),
    (r"\bsk-[A-Za-z0-9_-]{20,}", REDACTION_MARKER),
    // ntfy tokens.
    (r"\btk_[A-Za-z0-9]{16,}", REDACTION_MARKER),
    // AWS access key ids.
    (r"\bAKIA[0-9A-Z]{16}\b", REDACTION_MARKER),
    // Slack tokens.
    (r"\bxox[abprs]-[A-Za-z0-9-]{10,}", REDACTION_MARKER),
    // JWTs.
    (
        r"\beyJ[A-Za-z0-9_-]{5,}\.eyJ[A-Za-z0-9_-]{5,}\.[A-Za-z0-9_-]{5,}",
        REDACTION_MARKER,
    ),
    // Bearer tokens.
    (
        r"(?i)(\bBearer\s+)[A-Za-z0-9._~+/=-]{20,}",
        "${1}[redacted]",
    ),
    // password= / secret= / token= assignments, quoted or bare.
    (
        r#"(?i)(\b[A-Za-z0-9_]*(?:password|secret|token)=)(?:"[^"]*"|'[^']*'|[^\s"']+)"#,
        "${1}[redacted]",
    ),
];

fn patterns() -> &'static [SecretPattern] {
    static PATTERNS: OnceLock<Vec<SecretPattern>> = OnceLock::new();
    PATTERNS.get_or_init(|| {
        SECRET_PATTERNS
            .iter()
            .filter_map(|(pattern, replacement)| {
                // The patterns are constants covered by tests; a pattern that
                // failed to compile would only drop one redaction rule.
                Regex::new(pattern)
                    .ok()
                    .map(|regex| SecretPattern { regex, replacement })
            })
            .collect()
    })
}

/// The result of scanning one text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ScanResult {
    /// The text with every secret replaced.
    pub text: String,
    /// How many secrets were replaced.
    pub redactions: usize,
}

/// Replaces every known secret pattern in `text` by `[redacted]`.
pub fn redact_secrets(text: &str) -> ScanResult {
    let mut current = text.to_owned();
    let mut redactions = 0;
    for pattern in patterns() {
        // A match that already reads as its own replacement (`password=[redacted]`)
        // is not a new redaction, which keeps the scan idempotent.
        let replaced = pattern
            .regex
            .replace_all(&current, |captures: &regex::Captures<'_>| {
                let mut redacted = String::new();
                captures.expand(pattern.replacement, &mut redacted);
                if redacted != captures[0] {
                    redactions += 1;
                }
                redacted
            })
            .into_owned();
        current = replaced;
    }
    ScanResult {
        text: current,
        redactions,
    }
}

/// Scans an optional text, reporting whether anything was redacted.
pub fn redact_optional(text: Option<&str>) -> (Option<String>, usize) {
    match text {
        Some(text) => {
            let scan = redact_secrets(text);
            (Some(scan.text), scan.redactions)
        }
        None => (None, 0),
    }
}

/// Scans one step's `text` and `detail`.
pub fn redact_step(step: &ActionStep) -> (ActionStep, usize) {
    let text = redact_secrets(&step.text);
    let (detail, detail_redactions) = redact_optional(step.detail.as_deref());
    let redacted = ActionStep {
        text: text.text,
        detail,
        link: step.link.clone(),
    };
    (redacted, text.redactions + detail_redactions)
}
