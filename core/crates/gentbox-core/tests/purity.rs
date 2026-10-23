//! The crate is pure: no clock, IO, async, randomness or `unsafe` anywhere in
//! its source (architecture §4: "no IO, no clock, no async").
// The tests read files; the crate itself must not (see clippy.toml).
#![allow(clippy::unwrap_used, clippy::expect_used, clippy::disallowed_methods)]

use std::fs;
use std::path::{Path, PathBuf};

/// Tokens that must not appear in non-comment source lines.
const FORBIDDEN: &[&str] = &[
    "SystemTime",
    "Instant::now",
    "::now(",
    "now_utc",
    "now_local",
    "std::fs",
    "std::net",
    "std::env",
    "std::io",
    "std::process",
    "std::thread",
    "std::sync::mpsc",
    "tokio",
    "async ",
    "unsafe",
    "rand::",
    "getrandom",
    "println!",
    "eprintln!",
    "dbg!",
];

fn source_files(directory: &Path, found: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            source_files(&path, found);
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            found.push(path);
        }
    }
}

/// `.await` as a keyword, not the prefix of a field such as `.awaits_response`.
fn has_await_keyword(line: &str) -> bool {
    line.match_indices(".await").any(|(index, matched)| {
        !line[index + matched.len()..]
            .chars()
            .next()
            .is_some_and(|next| next.is_alphanumeric() || next == '_')
    })
}

fn code_lines(source: &str) -> impl Iterator<Item = (usize, &str)> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| !line.trim_start().starts_with("//"))
        .map(|(index, line)| (index + 1, line))
}

#[test]
fn no_source_file_touches_the_clock_io_async_or_unsafe() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    source_files(&root, &mut files);
    assert!(files.len() >= 20, "found {} source files", files.len());
    let mut violations = Vec::new();
    for file in files {
        let source = fs::read_to_string(&file).unwrap();
        for (line_number, line) in code_lines(&source) {
            if has_await_keyword(line) {
                violations.push(format!("{}:{line_number}: `.await`", file.display()));
            }
            for token in FORBIDDEN {
                if line.contains(token) {
                    violations.push(format!("{}:{line_number}: `{token}`", file.display()));
                }
            }
        }
    }
    assert!(
        violations.is_empty(),
        "impure code:\n{}",
        violations.join("\n")
    );
}

#[test]
fn decide_and_classify_take_time_as_data() {
    // The only time types in the core are values the caller passes in.
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for module in [
        "policy.rs",
        "classify.rs",
        "coalesce.rs",
        "lifecycle.rs",
        "resolve.rs",
    ] {
        let source = fs::read_to_string(root.join(module)).unwrap();
        for (line_number, line) in code_lines(&source) {
            assert!(!line.contains("::now"), "{module}:{line_number}");
        }
    }
}

#[test]
fn the_workspace_forbids_unsafe_for_this_crate() {
    let manifest =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml")).unwrap();
    assert!(manifest.contains("[lints]\nworkspace = true"));
    let workspace =
        fs::read_to_string(Path::new(env!("CARGO_MANIFEST_DIR")).join("../../Cargo.toml")).unwrap();
    assert!(workspace.contains("unsafe_code = \"forbid\""));
}
