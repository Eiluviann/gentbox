//! Backstops for two rules of the crate: no clock, no async inside the store,
//! and no SQL outside it.
#![allow(clippy::unwrap_used, clippy::expect_used)]

use std::fs;
use std::path::{Path, PathBuf};

fn rust_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory).unwrap() {
        let path = entry.unwrap().path();
        if path.is_dir() {
            files.extend(rust_files(&path));
        } else if path.extension().is_some_and(|extension| extension == "rs") {
            files.push(path);
        }
    }
    files
}

fn crates_directory() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .unwrap()
        .to_owned()
}

#[test]
fn the_store_has_no_clock_threads_or_async() {
    let forbidden = [
        "now_utc",
        "SystemTime",
        "Instant::now",
        "thread::spawn",
        "thread::sleep",
        "async fn",
        "tokio",
        ".await ",
        ".await;",
        ".await)",
    ];
    let source = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    for file in rust_files(&source) {
        let text = fs::read_to_string(&file).unwrap();
        for line in text
            .lines()
            .filter(|line| !line.trim_start().starts_with("//"))
        {
            for word in forbidden {
                assert!(
                    !line.contains(word),
                    "{} uses `{word}`: {line}",
                    file.display()
                );
            }
        }
    }
}

#[test]
fn no_other_crate_touches_sqlite() {
    for entry in fs::read_dir(crates_directory()).unwrap() {
        let crate_directory = entry.unwrap().path();
        if crate_directory
            .file_name()
            .is_some_and(|name| name == "gentbox-store")
        {
            continue;
        }
        let manifest = fs::read_to_string(crate_directory.join("Cargo.toml")).unwrap();
        assert!(
            !manifest.contains("rusqlite") && !manifest.contains("sqlite"),
            "{} depends on SQLite",
            crate_directory.display()
        );
        for file in rust_files(&crate_directory) {
            let text = fs::read_to_string(&file).unwrap();
            for statement in ["CREATE TABLE", "INSERT INTO", "DELETE FROM", "rusqlite"] {
                assert!(
                    !text.contains(statement),
                    "{} contains `{statement}`",
                    file.display()
                );
            }
        }
    }
}
