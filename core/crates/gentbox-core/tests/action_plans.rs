//! Action plans and the secret scan (ADR 0011).
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use gentbox_core::action_plan::{
    ActionPlanMode, FALLBACK_STEP_TEXT, FOLLOW_DETAILS_STEP_TEXT, MAX_STEPS, PlanError,
    RUN_COMMAND_STEP_TEXT, is_allowed_link_scheme, prepare_action_plan, set_step_ticked,
    validate_steps,
};
use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::message::{ActionKind, ActionStep, MessageType};
use gentbox_core::prepare::{IngestRejection, IngestSettings, prepare_message};
use gentbox_core::response::WarningCode;
use gentbox_core::secret_scan::{REDACTION_MARKER, redact_secrets};
use gentbox_core::title_guard::{GuardMode, is_plain_words};
use serde_json::json;
use support::input;

fn step(text: &str) -> ActionStep {
    ActionStep {
        text: text.to_owned(),
        detail: None,
        link: None,
    }
}

fn plan(steps: serde_json::Value) -> Vec<ActionStep> {
    serde_json::from_value(steps).unwrap()
}

fn prepare(
    extra: serde_json::Value,
    settings: IngestSettings,
) -> Result<gentbox_core::prepare::PreparedMessage, IngestRejection> {
    prepare_message(
        input(extra),
        &RuleSet::empty(),
        BuiltInRules::load(),
        settings,
    )
}

// --- steps validation ---

#[test]
fn a_plan_has_one_to_twelve_steps() {
    for (count, valid) in [(0, false), (1, true), (12, true), (13, false)] {
        let steps: Vec<_> = (0..count)
            .map(|index| step(&format!("Do thing {index}")))
            .collect();
        assert_eq!(validate_steps(&steps).is_ok(), valid, "{count} steps");
        if !valid {
            assert_eq!(validate_steps(&steps), Err(PlanError::StepCount { count }));
        }
    }
    assert_eq!(MAX_STEPS, 12);
}

#[test]
fn step_text_and_detail_have_length_limits() {
    assert_eq!(
        validate_steps(&[step("")]),
        Err(PlanError::TextLength { index: 0 })
    );
    assert_eq!(
        validate_steps(&[step("   ")]),
        Err(PlanError::TextLength { index: 0 })
    );
    assert!(validate_steps(&[step(&"a".repeat(200))]).is_ok());
    assert_eq!(
        validate_steps(&[step("ok"), step(&"a".repeat(201))]),
        Err(PlanError::TextLength { index: 1 })
    );
    let mut long_detail = step("ok");
    long_detail.detail = Some("d".repeat(2000));
    assert!(validate_steps(&[long_detail.clone()]).is_ok());
    long_detail.detail = Some("d".repeat(2001));
    assert_eq!(
        validate_steps(&[long_detail]),
        Err(PlanError::DetailLength { index: 0 })
    );
}

#[test]
fn link_schemes_are_allowlisted() {
    for (url, allowed) in [
        ("https://www.npmjs.com/settings/profile", true),
        ("claude-cli://open?cwd=x", true),
        ("claude://code/new", true),
        ("vscode://file/x", true),
        ("cursor://file/x", true),
        (
            "x-apple.systempreferences:com.apple.preference.security",
            true,
        ),
        ("http://example.com", false),
        ("javascript:alert(1)", false),
        ("file:///etc/passwd", false),
        ("ftp://host", false),
        ("no-scheme", false),
        ("HTTPS://example.com", false),
    ] {
        assert_eq!(is_allowed_link_scheme(url), allowed, "{url}");
        let mut linked = step("Open it");
        linked.link = Some(gentbox_core::message::StepLink {
            url: url.to_owned(),
            label: None,
        });
        assert_eq!(validate_steps(&[linked]).is_ok(), allowed, "{url}");
    }
}

// --- the plan pipeline ---

#[test]
fn a_clean_plan_passes_through() {
    let steps = plan(json!([
        { "text": "Open a terminal in any folder" },
        { "text": "Sign in to npm", "detail": "```bash\nnpm login\n```" },
        { "text": "Check you are signed in", "link": { "url": "https://www.npmjs.com/settings/profile", "label": "npm profile" } }
    ]));
    let prepared = prepare_action_plan(Some(steps.clone()), None, ActionPlanMode::Rewrite).unwrap();
    assert_eq!(prepared.steps, steps);
    assert!(prepared.warnings.is_empty());
}

#[test]
fn a_missing_plan_gets_the_fallback_step_linked_to_the_focus_target() {
    let prepared = prepare_action_plan(
        None,
        Some("claude-cli://open?cwd=%2Fx"),
        ActionPlanMode::Rewrite,
    )
    .unwrap();
    assert_eq!(prepared.steps.len(), 1);
    assert_eq!(prepared.steps[0].text, FALLBACK_STEP_TEXT);
    assert_eq!(
        prepared.steps[0].link.as_ref().unwrap().url,
        "claude-cli://open?cwd=%2Fx"
    );
    assert_eq!(prepared.warnings.len(), 1);
    assert_eq!(prepared.warnings[0].code, WarningCode::StepsMissing);
    assert!(is_plain_words(&prepared.steps[0].text));

    let unlinked = prepare_action_plan(None, None, ActionPlanMode::Rewrite).unwrap();
    assert_eq!(unlinked.steps[0].link, None);
    let disallowed =
        prepare_action_plan(None, Some("ftp://host/x"), ActionPlanMode::Rewrite).unwrap();
    assert_eq!(disallowed.steps[0].link, None);
}

#[test]
fn strict_mode_rejects_a_missing_plan() {
    assert_eq!(
        prepare_action_plan(None, None, ActionPlanMode::Reject),
        Err(PlanError::StepsMissing)
    );
    assert!(prepare_action_plan(Some(vec![step("Do it")]), None, ActionPlanMode::Reject).is_ok());
}

#[test]
fn a_step_text_that_fails_the_guard_moves_into_its_detail() {
    let table = [
        ("npm login", RUN_COMMAND_STEP_TEXT),
        ("git push origin main", RUN_COMMAND_STEP_TEXT),
        ("Edit ~/.config/app.toml", FOLLOW_DETAILS_STEP_TEXT),
        ("Open https://example.com", FOLLOW_DETAILS_STEP_TEXT),
        ("Set KEY=value", FOLLOW_DETAILS_STEP_TEXT),
        ("Run `make`", FOLLOW_DETAILS_STEP_TEXT),
    ];
    for (text, replacement) in table {
        let prepared =
            prepare_action_plan(Some(vec![step(text)]), None, ActionPlanMode::Rewrite).unwrap();
        assert_eq!(prepared.steps[0].text, replacement, "{text}");
        assert_eq!(
            prepared.steps[0].detail.as_deref(),
            Some(format!("```\n{text}\n```").as_str()),
            "{text}"
        );
        assert_eq!(prepared.warnings.len(), 1);
        assert_eq!(prepared.warnings[0].code, WarningCode::StepRewritten);
        assert_eq!(prepared.warnings[0].field.as_deref(), Some("steps[0].text"));
        assert!(
            is_plain_words(&prepared.steps[0].text),
            "the replacement passes the guard"
        );
    }
}

#[test]
fn a_rewritten_step_keeps_its_detail_and_link() {
    let steps = plan(
        json!([{ "text": "npm login", "detail": "Use the publishing account.", "link": { "url": "https://www.npmjs.com/login" } }]),
    );
    let prepared = prepare_action_plan(Some(steps), None, ActionPlanMode::Rewrite).unwrap();
    let rewritten = &prepared.steps[0];
    assert_eq!(
        rewritten.detail.as_deref(),
        Some("```\nnpm login\n```\n\nUse the publishing account.")
    );
    assert!(rewritten.link.is_some());
}

#[test]
fn only_the_flagged_steps_are_rewritten_and_warned_about() {
    let steps = vec![
        step("Open the terminal"),
        step("npm login"),
        step("Return here"),
        step("/usr/bin/true"),
    ];
    let prepared = prepare_action_plan(Some(steps), None, ActionPlanMode::Rewrite).unwrap();
    let fields: Vec<_> = prepared
        .warnings
        .iter()
        .map(|w| w.field.clone().unwrap())
        .collect();
    assert_eq!(fields, vec!["steps[1].text", "steps[3].text"]);
    assert_eq!(prepared.steps[0].text, "Open the terminal");
    assert_eq!(prepared.steps[2].text, "Return here");
}

#[test]
fn an_invalid_plan_is_an_error_not_a_rewrite() {
    assert!(matches!(
        prepare_action_plan(Some(vec![]), None, ActionPlanMode::Rewrite),
        Err(PlanError::StepCount { count: 0 })
    ));
}

#[test]
fn ticks_are_progress_kept_sorted_and_bounded() {
    assert_eq!(set_step_ticked(&[], 1, true, 3), Ok(vec![1]));
    assert_eq!(set_step_ticked(&[1], 0, true, 3), Ok(vec![0, 1]));
    assert_eq!(
        set_step_ticked(&[0, 1], 1, true, 3),
        Ok(vec![0, 1]),
        "ticking twice is idempotent"
    );
    assert_eq!(set_step_ticked(&[0, 1], 0, false, 3), Ok(vec![1]));
    assert_eq!(set_step_ticked(&[], 0, false, 3), Ok(vec![]));
    assert_eq!(
        set_step_ticked(&[], 3, true, 3),
        Err(PlanError::UnknownStep { index: 3 })
    );
    assert_eq!(
        set_step_ticked(&[5, 0], 1, true, 3),
        Ok(vec![0, 1]),
        "out-of-range leftovers are dropped"
    );
}

// --- through the ingest pipeline ---

#[test]
fn action_required_without_steps_gets_the_fallback_step_done_and_a_warning() {
    let prepared = prepare(
        json!({ "type": "action_required", "title": "Sign in to npm" }),
        IngestSettings::default(),
    )
    .unwrap();
    let steps = prepared.message.steps.as_ref().unwrap();
    assert_eq!(steps.len(), 1);
    assert_eq!(steps[0].text, FALLBACK_STEP_TEXT);
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::StepsMissing)
    );
    let actions = prepared.message.actions.as_ref().unwrap();
    assert!(actions.iter().any(|action| action.kind == ActionKind::Done));
}

#[test]
fn the_fallback_step_links_the_conversations_focus_url() {
    let prepared = prepare(
        json!({ "type": "action_required", "conversation": { "id": "c", "focus": { "url": "claude-cli://open?cwd=x" } } }),
        IngestSettings::default(),
    )
    .unwrap();
    assert_eq!(
        prepared.message.steps.unwrap()[0]
            .link
            .as_ref()
            .unwrap()
            .url,
        "claude-cli://open?cwd=x"
    );
}

#[test]
fn strict_action_plan_mode_rejects_through_the_pipeline() {
    let settings = IngestSettings {
        action_plan: ActionPlanMode::Reject,
        ..IngestSettings::default()
    };
    let rejected = prepare(json!({ "type": "action_required" }), settings);
    assert_eq!(
        rejected.unwrap_err(),
        IngestRejection::ActionPlan(PlanError::StepsMissing)
    );
    assert!(
        prepare(
            json!({ "type": "action_required", "steps": [{ "text": "Do it" }] }),
            settings
        )
        .is_ok()
    );
}

#[test]
fn steps_without_a_declared_type_make_it_action_required() {
    let prepared = prepare(
        json!({ "steps": [{ "text": "Sign in" }] }),
        IngestSettings::default(),
    )
    .unwrap();
    assert_eq!(
        prepared.message.message_type,
        Some(MessageType::ActionRequired)
    );
    assert_eq!(prepared.message.steps.as_ref().unwrap().len(), 1);
}

#[test]
fn steps_on_a_declared_other_type_are_rejected() {
    let rejected = prepare(
        json!({ "type": "task_completed", "steps": [{ "text": "Sign in" }] }),
        IngestSettings::default(),
    );
    assert!(matches!(rejected, Err(IngestRejection::Invalid(_))));
}

#[test]
fn other_types_never_get_steps() {
    for message_type in ["decision_required", "task_completed", "generic"] {
        let prepared = prepare(json!({ "type": message_type }), IngestSettings::default()).unwrap();
        assert_eq!(prepared.message.steps, None, "{message_type}");
        assert!(
            prepared
                .warnings
                .iter()
                .all(|w| w.code != WarningCode::StepsMissing)
        );
    }
}

#[test]
fn step_text_goes_through_the_secret_scan_and_then_the_guard() {
    let prepared = prepare(
        json!({ "type": "action_required", "steps": [
            { "text": "Paste ghp_abcdefghijklmnopqrstuvwxyz0123456789 into the prompt" },
            { "text": "Set the key", "detail": "```\nANTHROPIC_API_KEY=sk-ant-api03-abcdefgh1234\n```" }
        ] }),
        IngestSettings::default(),
    )
    .unwrap();
    let steps = prepared.message.steps.unwrap();
    assert_eq!(
        steps[0].text,
        format!("Paste {REDACTION_MARKER} into the prompt")
    );
    assert!(
        is_plain_words(&steps[0].text),
        "a redacted step text still passes the guard"
    );
    assert!(
        steps[1]
            .detail
            .as_deref()
            .unwrap()
            .contains(REDACTION_MARKER)
    );
    assert!(!steps[1].detail.as_deref().unwrap().contains("abcdefgh1234"));
    let redacted_fields: Vec<_> = prepared
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::SecretRedacted)
        .map(|w| w.field.clone().unwrap())
        .collect();
    assert_eq!(redacted_fields, vec!["steps[0]", "steps[1]"]);
}

#[test]
fn redaction_that_lengthens_a_step_never_turns_a_valid_plan_into_a_422() {
    // 200 characters: valid. The scan turns `password=x` into `password=[redacted]`.
    let text = format!("{}password=x", "a ".repeat(95));
    assert_eq!(text.chars().count(), 200);
    let detail = format!("{}password=x", "d".repeat(1990));
    let prepared = prepare(
        json!({ "type": "action_required", "steps": [{ "text": text, "detail": detail }] }),
        IngestSettings::default(),
    )
    .unwrap();
    let step = &prepared.message.steps.unwrap()[0];
    assert!(step.text.ends_with("password=[redacted]"));
    assert!(
        step.text.chars().count() > 200,
        "redaction lengthened the text"
    );
    assert!(step.detail.as_deref().unwrap().chars().count() > 2000);
    assert!(
        prepared
            .warnings
            .iter()
            .any(|w| w.code == WarningCode::SecretRedacted)
    );
}

#[test]
fn lengths_are_still_checked_once_before_redaction() {
    let too_long = format!("{}password=x", "a ".repeat(96));
    let rejected = prepare(
        json!({ "type": "action_required", "steps": [{ "text": too_long }] }),
        IngestSettings::default(),
    );
    assert!(matches!(rejected, Err(IngestRejection::Invalid(_))));
}

#[test]
fn the_scan_covers_step_link_urls_and_labels() {
    let prepared = prepare(
        json!({ "type": "action_required", "steps": [{
            "text": "Open the dashboard",
            "link": { "url": "https://example.com/?token=abc123def456", "label": "Dashboard ghp_abcdefghijklmnopqrstuvwxyz0123456789" }
        }] }),
        IngestSettings::default(),
    )
    .unwrap();
    let link = prepared.message.steps.unwrap()[0].link.clone().unwrap();
    assert_eq!(link.url, "https://example.com/?token=[redacted]");
    assert_eq!(link.label.as_deref(), Some("Dashboard [redacted]"));
}

#[test]
fn the_scan_covers_the_conversation_title_and_topic_and_thread_titles() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let prepared = prepare(
        json!({
            "conversation": { "id": "c", "title": format!("Debugging {secret} access") },
            "topic": { "key": "k", "title": format!("Topic {secret}"), "summary": format!("Summary {secret}") },
            "thread": { "key": "t", "title": format!("Thread {secret}"), "summary": format!("About {secret}") },
        }),
        IngestSettings::default(),
    )
    .unwrap();
    let message = &prepared.message;
    assert_eq!(
        message.conversation.title.as_deref(),
        Some("Debugging [redacted] access")
    );
    let serialized = serde_json::to_string(message).unwrap();
    assert!(!serialized.contains("ghp_"), "{serialized}");
    let fields: Vec<_> = prepared
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::SecretRedacted)
        .map(|w| w.field.clone().unwrap())
        .collect();
    for field in [
        "conversation.title",
        "topic.title",
        "topic.summary",
        "thread.title",
        "thread.summary",
    ] {
        assert!(fields.iter().any(|f| f == field), "{field} in {fields:?}");
    }
}

#[test]
fn explicit_topic_and_thread_creates_are_scanned_too() {
    use gentbox_core::resolve::{
        GroupingSnapshot, ThreadCreateAction, ThreadCreateContext, TopicCreateOutcome,
        create_thread, create_topic,
    };
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let topic = serde_json::from_value(
        json!({ "key": "k", "title": format!("Topic {secret}"), "summary": secret }),
    )
    .unwrap();
    let TopicCreateOutcome::Created(created) =
        create_topic(&GroupingSnapshot::default(), &topic).unwrap()
    else {
        panic!("expected a create")
    };
    assert_eq!(created.title, "Topic [redacted]");
    assert_eq!(created.summary.as_deref(), Some("[redacted]"));
    assert_eq!(created.secrets_redacted, 2);

    let thread =
        serde_json::from_value(json!({ "key": "t", "title": format!("Thread {secret}") })).unwrap();
    let context = ThreadCreateContext {
        agent: "claude-code",
        rule_topic: None,
    };
    let outcome = create_thread(&GroupingSnapshot::default(), &thread, &context).unwrap();
    let ThreadCreateAction::Created { thread, .. } = outcome.action else {
        panic!("expected a create")
    };
    assert_eq!(thread.title, "Thread [redacted]");
    assert_eq!(thread.secrets_redacted, 1);
}

// --- the secret scan ---

/// `(label, text containing a secret, a fragment that must be gone)`.
const SECRET_POSITIVES: &[(&str, &str, &str)] = &[
    (
        "github ghp",
        "token ghp_abcdefghijklmnopqrstuvwxyz0123456789 ok",
        "ghp_abcdefghijkl",
    ),
    (
        "github gho",
        "gho_abcdefghijklmnopqrstuvwxyz0123456789",
        "gho_abcdefghijkl",
    ),
    (
        "github ghs",
        "ghs_abcdefghijklmnopqrstuvwxyz0123456789",
        "ghs_abcdefghijkl",
    ),
    (
        "github fine-grained",
        "github_pat_11ABCDEFG0abcdefghijklmnop_qrstuvwxyz",
        "github_pat_11ABC",
    ),
    (
        "anthropic",
        "key sk-ant-api03-AbCdEf123456_-xyz here",
        "sk-ant-api03",
    ),
    (
        "openai",
        "key sk-proj-abcdefghijklmnopqrstuvwxyz1234 here",
        "sk-proj-abcdef",
    ),
    (
        "ntfy",
        "auth tk_abcdefghijklmnopqrstuvwxyz012 now",
        "tk_abcdefghij",
    ),
    ("aws", "id AKIAIOSFODNN7EXAMPLE end", "AKIAIOSFODNN7"),
    ("slack bot", "xoxb-123456789012-abcdefghijkl", "xoxb-1234"),
    ("slack user", "xoxp-123456789012-abcdefghijkl", "xoxp-1234"),
    (
        "bearer",
        "Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789",
        "abcdefghijklmnopqrst",
    ),
    (
        "bearer lowercase",
        "bearer abcdefghijklmnopqrstuvwxyz0123456789",
        "abcdefghijklmnopqrst",
    ),
    (
        "password assignment",
        "password=hunter2hunter2",
        "hunter2hunter2",
    ),
    ("secret assignment", "client_secret=abc123xyz", "abc123xyz"),
    ("token assignment", "GITHUB_TOKEN=abc123xyz", "abc123xyz"),
    (
        "quoted assignment",
        "password=\"correct horse battery\" next",
        "correct horse",
    ),
    (
        "single-quoted assignment",
        "secret='top secret' next",
        "top secret",
    ),
    ("uppercase assignment", "PASSWORD=Hunter2", "Hunter2"),
];

/// Near misses that must survive untouched.
const SECRET_NEAR_MISSES: &[&str] = &[
    "ghp_short",
    "github_pat_short",
    "the task-force-leader-needs-approval-of-everything",
    "disk-usage-statistics-report-for-all-volumes",
    "sk-short",
    "sk-ant-x",
    "tk_short",
    "AKIA123",
    "AKIAIOSFODNN7EXAMPLE12345",
    "xoxb-short",
    "eyJhbGciOiJIUzI1NiJ9 without payload",
    "Bearer short",
    "the bearer of bad news",
    "the password was changed",
    "token count is high",
    "tokens=5",
    "secrets=ok",
    "set the password later",
    "-----BEGIN CERTIFICATE-----",
    "a plain sentence with nothing secret",
];

/// Secret-shaped fixtures that are assembled at run time, so the repository
/// holds no literal that a secret scanner would flag.
fn assembled_positives() -> Vec<(&'static str, String, &'static str)> {
    let dashes = "-----";
    let private_key = " PRIVATE KEY";
    let block = |kind: &str, body: &str| {
        format!(
            "{dashes}BEGIN {kind}{private_key}{dashes}\n{body}\n{dashes}END {kind}{private_key}{dashes}"
        )
    };
    let jwt_head = "ey".to_owned() + "JhbGciOiJIUzI1NiJ9";
    let jwt_body = "ey".to_owned() + "JzdWIiOiIxMjM0NTY3ODkwIn0";
    vec![
        (
            "private key block",
            format!("key:\n{}\nrest", block("RSA", "MIIEabc")),
            "MIIEabc",
        ),
        (
            "private key header only",
            format!("{dashes}BEGIN{private_key}{dashes}\nMIIEabc"),
            "MIIEabc",
        ),
        ("openssh key", block("OPENSSH", "b3BlbnNzaA"), "b3BlbnNzaA"),
        (
            "jwt",
            format!("jwt {jwt_head}.{jwt_body}.dBjftJeZ4CVPmB92K27uhbUJU1p end"),
            "JhbGciOi",
        ),
    ]
}

#[test]
fn assembled_secret_fixtures_are_redacted() {
    for (label, text, gone) in assembled_positives() {
        let scan = redact_secrets(&text);
        assert!(scan.redactions >= 1, "{label}");
        assert!(scan.text.contains(REDACTION_MARKER), "{label}");
        assert!(!scan.text.contains(gone), "{label}: {:?}", scan.text);
    }
}

#[test]
fn every_secret_pattern_is_redacted() {
    for &(label, text, gone) in SECRET_POSITIVES {
        let scan = redact_secrets(text);
        assert!(scan.redactions >= 1, "{label}: {text:?}");
        assert!(scan.text.contains(REDACTION_MARKER), "{label}");
        assert!(!scan.text.contains(gone), "{label}: {:?}", scan.text);
    }
}

#[test]
fn near_misses_are_left_alone() {
    for text in SECRET_NEAR_MISSES {
        let scan = redact_secrets(text);
        assert_eq!(scan.redactions, 0, "{text:?} -> {:?}", scan.text);
        assert_eq!(&scan.text, text);
    }
}

#[test]
fn assignments_keep_their_label() {
    assert_eq!(
        redact_secrets("password=hunter2hunter2").text,
        "password=[redacted]"
    );
    assert_eq!(
        redact_secrets("Authorization: Bearer abcdefghijklmnopqrstuvwxyz0123456789").text,
        "Authorization: Bearer [redacted]"
    );
}

#[test]
fn redaction_counts_every_secret_and_is_idempotent() {
    let text =
        "ghp_abcdefghijklmnopqrstuvwxyz0123456789 and AKIAIOSFODNN7EXAMPLE and password=x1y2z3";
    let once = redact_secrets(text);
    assert_eq!(once.redactions, 3);
    let twice = redact_secrets(&once.text);
    assert_eq!(twice.redactions, 0);
    assert_eq!(twice.text, once.text);
}

#[test]
fn every_text_field_is_scanned_before_it_is_stored() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let prepared = prepare(
        json!({
            "title": format!("Token {secret} found"),
            "summary": format!("Leaked {secret}"),
            "body": format!("```\n{secret}\n```"),
        }),
        IngestSettings::default(),
    )
    .unwrap();
    let message = prepared.message;
    for text in [
        Some(message.title.as_str()),
        message.summary.as_deref(),
        message.body.as_deref(),
    ] {
        assert!(!text.unwrap().contains("ghp_"));
    }
    let fields: Vec<_> = prepared
        .warnings
        .iter()
        .filter(|w| w.code == WarningCode::SecretRedacted)
        .map(|w| w.field.clone().unwrap())
        .collect();
    assert_eq!(fields, vec!["title", "summary", "body"]);
}

#[test]
fn a_fully_redacted_title_still_passes_the_guard_and_the_original_in_the_body_is_redacted_too() {
    let secret = "ghp_abcdefghijklmnopqrstuvwxyz0123456789";
    let prepared = prepare(
        json!({ "title": format!("git clone with {secret}"), "summary": "Cloning the private repository" }),
        IngestSettings::default(),
    )
    .unwrap();
    assert!(is_plain_words(&prepared.message.title));
    let body = prepared.message.body.unwrap();
    assert!(body.contains("Original title"));
    assert!(!body.contains("ghp_"));
    assert!(is_plain_words("[redacted]"));
    let only_secret = prepare(json!({ "title": secret }), IngestSettings::default()).unwrap();
    assert_eq!(only_secret.message.title, REDACTION_MARKER);
    assert!(is_plain_words(&only_secret.message.title));
}

#[test]
fn strict_title_guard_still_rejects_through_the_pipeline() {
    let settings = IngestSettings {
        title_guard: GuardMode::Reject,
        ..IngestSettings::default()
    };
    assert!(matches!(
        prepare(json!({ "title": "npm login" }), settings),
        Err(IngestRejection::TitleGuard(_))
    ));
}
