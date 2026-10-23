//! The title guard (ADR 0010): each heuristic positive and negative, every
//! rewrite branch, and the guarantee that a rewrite always passes.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use gentbox_core::message::MessageType;
use gentbox_core::response::WarningCode;
use gentbox_core::title_guard::{
    GuardContext, GuardFinding, GuardMode, GuardRejection, MessageTexts, code_box,
    cut_at_word_boundary, fallback_title, find_raw_input, first_sentence, guard_conversation_title,
    guard_message_texts, inline_group_title, is_plain_words, title_from_key,
};

use GuardFinding::{
    CodeMark, EnvironmentAssignment, Flag, Json, LeadingCommand, Path, ShellOperator, Url,
};

/// `(text, expected)`: `None` reads as plain words, `Some` is flagged for that reason.
const TABLE: &[(&str, Option<GuardFinding>)] = &[
    // Plain words, including near misses of every heuristic.
    ("Allow Bash to run the ingest tests?", None),
    ("Ingest auth accepts rotated tokens; 12 tests pass", None),
    ("Cargo tests pass", None),
    ("Go ahead and merge", None),
    ("The well-known, hyphenated-word case", None),
    ("A re-run of the long-running job finished", None),
    ("Check and/or update the changelog", None),
    ("Updated lib.rs and main.rs", None),
    ("Bumped to version 1.4.0 and then 2.0", None),
    ("Cost went from 3 to -5 percent", None),
    ("Wait for it - then continue", None),
    ("Pushed the branch — review needed", None),
    ("Use TCP/IP for the link", None),
    ("Fixed the NAME clash in docs", None),
    ("Tests: 12 passed, 0 failed", None),
    ("Allow editing lib.rs?", None),
    ("Mention @alice in the review", None),
    ("Ordering is [WIP] only inside the sentence", None),
    ("Renamed the crate from core to domain", None),
    ("Emoji ok ✅ and ünïcode wörds", None),
    // Code marks.
    ("Run `cargo test`", Some(CodeMark)),
    ("Here is a fence ```bash", Some(CodeMark)),
    // Shell operators.
    ("build | tee out", Some(ShellOperator)),
    ("lint && test", Some(ShellOperator)),
    ("a || b", Some(ShellOperator)),
    ("echo $(date)", Some(ShellOperator)),
    ("write out > file", Some(ShellOperator)),
    ("append out >> file", Some(ShellOperator)),
    ("read in < file", Some(ShellOperator)),
    ("noisy 2>/dev/null", Some(ShellOperator)),
    // Flags.
    ("Allow running with -p", Some(Flag)),
    ("Allow Bash to run tests --nocapture", Some(Flag)),
    ("Allow (--help) output", Some(Flag)),
    // Paths.
    ("Allow reading /etc/hosts", Some(Path)),
    ("Allow reading ~/notes", Some(Path)),
    ("Open ./scripts today", Some(Path)),
    ("Open ../up today", Some(Path)),
    ("Install @gentbox/cli now", Some(Path)),
    ("Edit src/core/lib now", Some(Path)),
    ("Edit src/lib.rs now", Some(Path)),
    // Environment assignments.
    ("Set RUST_LOG=debug first", Some(EnvironmentAssignment)),
    ("Run with A=1", Some(EnvironmentAssignment)),
    // URLs.
    ("Open https://example.com/login", Some(Url)),
    ("Clone ssh://host/repo", Some(Url)),
    // JSON.
    ("{\"a\": 1}", Some(Json)),
    ("[1, 2, 3]", Some(Json)),
    ("The payload was \"id\": 3 here", Some(Json)),
    // Leading commands.
    ("npm login", Some(LeadingCommand)),
    ("git push origin main", Some(LeadingCommand)),
    ("cargo test everything", Some(LeadingCommand)),
    ("curl example", Some(LeadingCommand)),
    ("go build the thing", Some(LeadingCommand)),
    ("sudo rm stuff", Some(LeadingCommand)),
    ("pytest tests", Some(LeadingCommand)),
];

#[test]
fn heuristics_table_positives_and_negatives() {
    assert!(TABLE.len() >= 40, "the plan asks for at least 40 strings");
    for &(text, expected) in TABLE {
        assert_eq!(find_raw_input(text), expected, "{text:?}");
        assert_eq!(is_plain_words(text), expected.is_none(), "{text:?}");
    }
}

#[test]
fn every_command_name_of_the_list_is_flagged_only_with_arguments() {
    for name in [
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
    ] {
        assert_eq!(
            find_raw_input(&format!("{name} something")),
            Some(LeadingCommand),
            "{name}"
        );
        assert_eq!(find_raw_input(name), None, "{name} alone is a word");
        assert_eq!(
            find_raw_input(&format!("{} something", capitalize(name))),
            None,
            "capitalised {name} is prose"
        );
    }
}

fn capitalize(word: &str) -> String {
    let mut characters = word.chars();
    characters
        .next()
        .map(|first| first.to_uppercase().chain(characters).collect())
        .unwrap_or_default()
}

#[test]
fn a_redaction_marker_does_not_look_like_json() {
    assert!(is_plain_words("[redacted]"));
    assert!(is_plain_words("Token [redacted] was removed"));
}

fn context(message_type: MessageType) -> GuardContext {
    GuardContext {
        message_type,
        agent_name: "Claude Code".to_owned(),
        project: Some("gentbox".to_owned()),
    }
}

fn texts(title: &str, summary: Option<&str>, body: Option<&str>) -> MessageTexts {
    MessageTexts {
        title: title.to_owned(),
        summary: summary.map(str::to_owned),
        body: body.map(str::to_owned),
    }
}

#[test]
fn clean_texts_pass_through_untouched() {
    let guarded = guard_message_texts(
        texts(
            "Allow Bash to run the ingest tests?",
            Some("gentbox: Run the ingest tests"),
            Some("```bash\ncargo test\n```"),
        ),
        &context(MessageType::DecisionRequired),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert_eq!(guarded.title, "Allow Bash to run the ingest tests?");
    assert_eq!(
        guarded.summary.as_deref(),
        Some("gentbox: Run the ingest tests")
    );
    assert_eq!(guarded.body.as_deref(), Some("```bash\ncargo test\n```"));
    assert!(guarded.warnings.is_empty());
}

#[test]
fn a_flagged_title_becomes_the_summarys_first_sentence_and_the_original_goes_on_top_of_the_body() {
    let guarded = guard_message_texts(
        texts(
            "Allow Bash: cargo test -p gentbox-ingest -- --nocapture?",
            Some("Run the ingest tests with output. It takes a minute."),
            Some("Working directory: gentbox"),
        ),
        &context(MessageType::DecisionRequired),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert_eq!(guarded.title, "Run the ingest tests with output");
    assert_eq!(
        guarded.summary.as_deref(),
        Some("Run the ingest tests with output. It takes a minute.")
    );
    let body = guarded.body.unwrap();
    assert!(body.starts_with(
        "**Original title**\n\n```\nAllow Bash: cargo test -p gentbox-ingest -- --nocapture?\n```"
    ));
    assert!(body.ends_with("Working directory: gentbox"));
    assert_eq!(guarded.warnings.len(), 1);
    assert_eq!(guarded.warnings[0].code, WarningCode::TitleLooksLikeCommand);
    assert_eq!(guarded.warnings[0].field.as_deref(), Some("title"));
    assert!(
        guarded.warnings[0]
            .message
            .contains("Run the ingest tests with output")
    );
}

#[test]
fn a_long_summary_sentence_is_cut_at_a_word_boundary() {
    let long = "word ".repeat(60);
    let guarded = guard_message_texts(
        texts("npm run build now", Some(long.trim()), None),
        &context(MessageType::TaskCompleted),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert!(guarded.title.chars().count() <= 120);
    assert!(guarded.title.ends_with("word…"), "{}", guarded.title);
    assert!(is_plain_words(&guarded.title));
}

#[test]
fn without_a_usable_summary_the_fallback_phrase_per_type_is_used() {
    let table = [
        (
            MessageType::DecisionRequired,
            "Claude Code needs your decision in gentbox",
        ),
        (MessageType::ActionRequired, "Action needed in gentbox"),
        (
            MessageType::TaskCompleted,
            "Claude Code finished a task in gentbox",
        ),
        (MessageType::Generic, "Update from Claude Code"),
    ];
    for (message_type, expected) in table {
        for summary in [None, Some("cargo test --all"), Some("   ")] {
            let guarded = guard_message_texts(
                texts("git push origin main", summary, None),
                &context(message_type),
                GuardMode::Rewrite,
            )
            .unwrap();
            assert_eq!(guarded.title, expected, "{message_type:?} {summary:?}");
        }
    }
}

#[test]
fn the_fallback_leaves_out_a_project_or_agent_name_that_fails_the_guard() {
    let mut odd = context(MessageType::DecisionRequired);
    odd.project = Some("A=B".to_owned());
    assert_eq!(fallback_title(&odd), "Claude Code needs your decision");
    odd.project = None;
    assert_eq!(fallback_title(&odd), "Claude Code needs your decision");
    odd.agent_name = "`rm -rf`".to_owned();
    assert_eq!(fallback_title(&odd), "The agent needs your decision");
}

#[test]
fn a_flagged_summary_moves_to_the_body_and_is_dropped() {
    let guarded = guard_message_texts(
        texts(
            "Ingest tests pass",
            Some("see /tmp/out.log for the run"),
            Some("details"),
        ),
        &context(MessageType::TaskCompleted),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert_eq!(guarded.title, "Ingest tests pass");
    assert_eq!(guarded.summary, None);
    assert_eq!(
        guarded.body.as_deref(),
        Some("**Original summary**\n\n```\nsee /tmp/out.log for the run\n```\n\ndetails")
    );
    assert_eq!(guarded.warnings.len(), 1);
    assert_eq!(
        guarded.warnings[0].code,
        WarningCode::SummaryLooksLikeCommand
    );
}

#[test]
fn both_flagged_title_and_summary_use_the_fallback_and_keep_both_originals() {
    let guarded = guard_message_texts(
        texts("git push", Some("git push --force"), None),
        &context(MessageType::ActionRequired),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert_eq!(guarded.title, "Action needed in gentbox");
    assert_eq!(guarded.summary, None);
    let body = guarded.body.unwrap();
    assert!(body.find("Original title").unwrap() < body.find("Original summary").unwrap());
    let codes: Vec<_> = guarded
        .warnings
        .iter()
        .map(|warning| warning.code)
        .collect();
    assert_eq!(
        codes,
        vec![
            WarningCode::TitleLooksLikeCommand,
            WarningCode::SummaryLooksLikeCommand
        ]
    );
}

#[test]
fn the_code_box_fence_outgrows_backticks_in_the_original() {
    assert_eq!(code_box("plain"), "```\nplain\n```");
    assert_eq!(code_box("a ``` b"), "````\na ``` b\n````");
    let guarded = guard_message_texts(
        texts("run ```x``` now", None, None),
        &context(MessageType::Generic),
        GuardMode::Rewrite,
    )
    .unwrap();
    assert!(
        guarded
            .body
            .unwrap()
            .contains("````\nrun ```x``` now\n````")
    );
}

#[test]
fn strict_mode_rejects_instead_of_rewriting() {
    let title = guard_message_texts(
        texts("npm login", Some("fine"), None),
        &context(MessageType::ActionRequired),
        GuardMode::Reject,
    );
    assert_eq!(
        title,
        Err(GuardRejection::TitleLooksLikeCommand(LeadingCommand))
    );
    let summary = guard_message_texts(
        texts("Fine", Some("open https://x.example/a"), None),
        &context(MessageType::ActionRequired),
        GuardMode::Reject,
    );
    assert_eq!(summary, Err(GuardRejection::SummaryLooksLikeCommand(Url)));
    assert!(
        guard_message_texts(
            texts("Fine", Some("Fine too"), None),
            &context(MessageType::Generic),
            GuardMode::Reject
        )
        .is_ok()
    );
    assert!(
        GuardRejection::TitleLooksLikeCommand(Path)
            .to_string()
            .starts_with("title_looks_like_command")
    );
}

#[test]
fn a_flagged_conversation_title_is_ignored() {
    let (title, warning) = guard_conversation_title(Some("cargo test -p ingest"));
    assert_eq!(title, None);
    assert_eq!(warning.unwrap().code, WarningCode::ConversationTitleIgnored);
    let (title, warning) = guard_conversation_title(Some("Fix flaky ingest test"));
    assert_eq!(title.as_deref(), Some("Fix flaky ingest test"));
    assert!(warning.is_none());
    assert_eq!(guard_conversation_title(Some("   ")), (None, None));
    assert_eq!(guard_conversation_title(None), (None, None));
}

#[test]
fn keys_become_words() {
    for (key, expected) in [
        ("phase-3-ingest", "Phase 3 ingest"),
        ("release-1.4", "Release 1.4"),
        ("project:gentbox", "Project gentbox"),
        ("repo:owner/name", "Repo owner name"),
        ("snake_case_key", "Snake case key"),
        ("v1.4", "V1.4"),
        ("a.b", "A b"),
        ("---", "Untitled"),
    ] {
        assert_eq!(title_from_key(key), expected, "{key}");
        assert!(is_plain_words(&title_from_key(key)));
    }
}

#[test]
fn inline_group_titles_prefer_a_clean_title_then_the_key() {
    assert_eq!(inline_group_title("k", Some("Release 1.4")), "Release 1.4");
    assert_eq!(
        inline_group_title("phase-3", Some("git push origin")),
        "Phase 3"
    );
    assert_eq!(inline_group_title("phase-3", Some("  ")), "Phase 3");
    assert_eq!(inline_group_title("phase-3", None), "Phase 3");
}

#[test]
fn sentences_and_cuts() {
    for (text, expected) in [
        ("One sentence. Two sentences.", "One sentence"),
        ("Is it done? Yes.", "Is it done?"),
        ("Done! Next.", "Done!"),
        (
            "Updated lib.rs and 1.4.0 today. More.",
            "Updated lib.rs and 1.4.0 today",
        ),
        ("No terminator", "No terminator"),
        ("  Padded.  ", "Padded"),
    ] {
        assert_eq!(first_sentence(text), expected);
    }
    assert_eq!(cut_at_word_boundary("short", 10), "short");
    assert_eq!(cut_at_word_boundary("aaaa bbbb cccc", 10), "aaaa bbbb…");
    assert_eq!(cut_at_word_boundary("abcdefghijklmnop", 8), "abcdefg…");
}

/// Hostile titles and summaries of every flavour: whatever goes in, a
/// rewritten title and summary pass the guard.
#[test]
fn no_rewritten_title_or_summary_ever_fails_the_guard() {
    let mut inputs: Vec<&str> = TABLE.iter().map(|&(text, _)| text).collect();
    inputs.extend([
        "",
        "   ",
        "-",
        "--",
        "/",
        "~",
        "{",
        "[",
        "`",
        "$(rm -rf /)",
        "KEY=value",
        "a/b/c",
        "cargo",
        "git push. Then more words follow.",
        "[redacted]",
        "sk-[redacted] and token=[redacted]",
        "\"quoted\": json",
        "ünïcode — dashes — everywhere",
    ]);
    let contexts = MessageType::ALL.map(context);
    for message_type_context in &contexts {
        for &title in &inputs {
            for &summary in &inputs {
                let title = if title.trim().is_empty() {
                    "x run now ./a"
                } else {
                    title
                };
                let Ok(guarded) = guard_message_texts(
                    texts(title, Some(summary), Some("body")),
                    message_type_context,
                    GuardMode::Rewrite,
                ) else {
                    panic!("rewrite mode never rejects");
                };
                assert!(
                    is_plain_words(&guarded.title),
                    "title {:?} from {title:?} / {summary:?}",
                    guarded.title
                );
                assert!(guarded.title.chars().count() <= 120);
                assert!(!guarded.title.trim().is_empty());
                if let Some(kept) = &guarded.summary {
                    assert!(is_plain_words(kept), "summary {kept:?}");
                }
            }
        }
    }
}
