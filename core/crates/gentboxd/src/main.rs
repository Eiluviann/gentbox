//! `gentboxd`: the headless Gentbox hub (architecture §3). It takes the hub
//! lock, opens the store, serves the local API on the Unix socket and
//! loopback, reloads `rules.toml` when it changes, and stops cleanly on
//! SIGTERM or SIGINT, removing `API/port` and the socket.
//!
//! No UI is attached, so nothing can answer: senders get `shouldWait:
//! false`. No phone relay is registered (v1).
//!
//! ```text
//! gentboxd [--state-dir <path>] [--config-dir <path>]
//! ```
//!
//! Logging follows `GENTBOX_LOG` (default `info`); logs never carry message
//! bodies, tokens or tool input.

mod config;

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use gentbox_core::classify::RuleSet;
use gentbox_hub::{Hub, HubConfig, HubDelegate, HubEvent, HubHandle, StartError};
use gentbox_ingest::{IngestServer, Limits};

use crate::config::{Environment, Folders};

/// How often `rules.toml` is checked for changes.
const RULES_POLL: Duration = Duration::from_secs(2);

const USAGE: &str = "usage: gentboxd [--state-dir <path>] [--config-dir <path>]";

/// The command line.
#[derive(Debug, Default)]
struct Arguments {
    state_dir: Option<PathBuf>,
    config_dir: Option<PathBuf>,
}

fn main() -> ExitCode {
    let arguments = match parse_arguments(std::env::args().skip(1)) {
        Ok(Some(arguments)) => arguments,
        Ok(None) => return ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gentboxd: {message}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    init_logging();
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            eprintln!("gentboxd: cannot start the runtime: {error}");
            return ExitCode::FAILURE;
        }
    };
    match runtime.block_on(run(arguments)) {
        Ok(()) => ExitCode::SUCCESS,
        Err(message) => {
            eprintln!("gentboxd: {message}");
            tracing::error!("{message}");
            ExitCode::FAILURE
        }
    }
}

/// `Ok(None)` for `--help` and `--version`, which print and exit.
fn parse_arguments(
    mut arguments: impl Iterator<Item = String>,
) -> Result<Option<Arguments>, String> {
    let mut parsed = Arguments::default();
    while let Some(argument) = arguments.next() {
        let mut value = |name: &str| {
            arguments
                .next()
                .map(PathBuf::from)
                .ok_or_else(|| format!("{name} needs a path"))
        };
        match argument.as_str() {
            "--state-dir" => parsed.state_dir = Some(value("--state-dir")?),
            "--config-dir" => parsed.config_dir = Some(value("--config-dir")?),
            "--help" | "-h" => {
                println!("{USAGE}");
                return Ok(None);
            }
            "--version" | "-V" => {
                println!("gentboxd {}", env!("CARGO_PKG_VERSION"));
                return Ok(None);
            }
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    Ok(Some(parsed))
}

fn init_logging() {
    let filter = tracing_subscriber::EnvFilter::try_from_env("GENTBOX_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"));
    tracing_subscriber::fmt()
        .with_env_filter(filter)
        .with_writer(std::io::stderr)
        .init();
}

async fn run(arguments: Arguments) -> Result<(), String> {
    let environment = Environment {
        home: std::env::var_os("HOME").map(PathBuf::from),
        xdg_state_home: std::env::var_os("XDG_STATE_HOME").map(PathBuf::from),
        xdg_config_home: std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from),
    };
    let folders = config::folders(arguments.state_dir, arguments.config_dir, &environment)
        .map_err(|error| error.to_string())?;
    let rules_path = folders.config.join("rules.toml");
    let mut config = HubConfig::new(&folders.state);
    config.sources = config::read_sources(&folders.config.join("config.toml"))
        .map_err(|error| error.to_string())?;
    config.rules = load_rules(&rules_path, environment.home.as_deref()).unwrap_or_default();
    config.has_answer_channel = false;

    // The hub takes the lock before it opens the store or binds anything.
    let hub = Hub::start(config, Arc::new(LogDelegate), None).map_err(|error| match error {
        StartError::Lock(lock) => lock.to_string(),
        other => other.to_string(),
    })?;
    let ingest = match IngestServer::start(hub.handle(), &folders.state, Limits::default()).await {
        Ok(ingest) => ingest,
        Err(error) => {
            hub.shutdown().await;
            return Err(error.to_string());
        }
    };
    log_start(&folders, &ingest);

    let reloader = tokio::spawn(watch_rules(
        hub.handle(),
        rules_path,
        environment.home.clone(),
    ));
    let signal = wait_for_signal().await;
    tracing::info!(signal, "stopping");
    reloader.abort();
    ingest.shutdown().await;
    hub.shutdown().await;
    tracing::info!("stopped");
    Ok(())
}

fn log_start(folders: &Folders, ingest: &IngestServer) {
    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        state = %folders.state.display(),
        config = %folders.config.display(),
        port = ingest.port(),
        "gentboxd running"
    );
}

/// Reads `rules.toml`; a missing file is no rules, an invalid one is logged
/// and gives `None` (the caller keeps what it had).
fn load_rules(path: &Path, home: Option<&Path>) -> Option<RuleSet> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Some(RuleSet::empty());
        }
        Err(error) => {
            tracing::error!(path = %path.display(), %error, "cannot read the rules");
            return None;
        }
    };
    let parsed = match home.and_then(Path::to_str) {
        Some(home) => RuleSet::parse_with_home(&text, home),
        None => RuleSet::parse(&text),
    };
    match parsed {
        Ok(rules) => Some(rules),
        Err(error) => {
            tracing::error!(path = %path.display(), line = ?error.line(), %error, "invalid rules; keeping the last good ones");
            None
        }
    }
}

/// Reloads the rules whenever the file's modification time changes.
async fn watch_rules(hub: HubHandle, path: PathBuf, home: Option<PathBuf>) {
    let modified = |path: &Path| -> Option<SystemTime> {
        std::fs::metadata(path)
            .and_then(|meta| meta.modified())
            .ok()
    };
    let mut last_seen = modified(&path);
    loop {
        tokio::time::sleep(RULES_POLL).await;
        let now_seen = modified(&path);
        if now_seen == last_seen {
            continue;
        }
        last_seen = now_seen;
        if let Some(rules) = load_rules(&path, home.as_deref()) {
            tracing::info!(rules = rules.len(), "rules reloaded");
            if hub.set_rules(rules).await.is_err() {
                return;
            }
        }
    }
}

#[cfg(unix)]
async fn wait_for_signal() -> &'static str {
    use tokio::signal::unix::{SignalKind, signal};
    let terminate = signal(SignalKind::terminate());
    let interrupt = signal(SignalKind::interrupt());
    match (terminate, interrupt) {
        (Ok(mut terminate), Ok(mut interrupt)) => tokio::select! {
            _ = terminate.recv() => "SIGTERM",
            _ = interrupt.recv() => "SIGINT",
        },
        _ => {
            let _ = tokio::signal::ctrl_c().await;
            "interrupt"
        }
    }
}

#[cfg(not(unix))]
async fn wait_for_signal() -> &'static str {
    let _ = tokio::signal::ctrl_c().await;
    "interrupt"
}

/// With no UI, events go to the log: ids and kinds only, never content.
#[derive(Debug)]
struct LogDelegate;

impl HubDelegate for LogDelegate {
    fn on_event(&self, event: HubEvent) {
        match event {
            HubEvent::Delivered { message, plan, .. } => tracing::info!(
                id = %message.id,
                kind = message.message_type.as_str(),
                surface = ?plan.mac,
                "delivered"
            ),
            HubEvent::Repinged { message_id, .. } => tracing::info!(id = %message_id, "re-pinged"),
            HubEvent::Withdrawn(rescission) => tracing::info!(
                id = %rescission.message_id,
                reason = ?rescission.reason,
                "withdrawn"
            ),
            HubEvent::Closed {
                message_id, state, ..
            } => tracing::info!(id = %message_id, ?state, "closed"),
            other => tracing::debug!(event = event_name(&other), "hub event"),
        }
    }
}

fn event_name(event: &HubEvent) -> &'static str {
    match event {
        HubEvent::Delivered { .. } => "delivered",
        HubEvent::Repinged { .. } => "repinged",
        HubEvent::Repeated { .. } => "repeated",
        HubEvent::Updated { .. } => "updated",
        HubEvent::Closed { .. } => "closed",
        HubEvent::Withdrawn(_) => "withdrawn",
        HubEvent::WaiterChanged { .. } => "waiter_changed",
        HubEvent::TopicChanged(_) => "topic_changed",
        HubEvent::ThreadChanged(_) => "thread_changed",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(arguments: &[&str]) -> Result<Option<Arguments>, String> {
        parse_arguments(arguments.iter().map(|argument| (*argument).to_owned()))
    }

    #[test]
    fn arguments_are_parsed_and_unknown_ones_refused() {
        let parsed = parse(&["--state-dir", "/s", "--config-dir", "/c"])
            .ok()
            .flatten()
            .unwrap_or_default();
        assert_eq!(parsed.state_dir, Some(PathBuf::from("/s")));
        assert_eq!(parsed.config_dir, Some(PathBuf::from("/c")));
        assert!(parse(&["--state-dir"]).is_err());
        assert!(parse(&["--port", "1"]).is_err());
        assert!(matches!(parse(&["--version"]), Ok(None)));
    }

    #[test]
    fn a_missing_rules_file_is_no_rules_and_an_invalid_one_is_none() {
        let directory = tempfile::tempdir().ok();
        let Some(directory) = directory else { return };
        let path = directory.path().join("rules.toml");
        assert!(load_rules(&path, None).is_some_and(|rules| rules.is_empty()));
        std::fs::write(&path, "[[rule]]\nname = 1\n").ok();
        assert!(load_rules(&path, None).is_none());
        std::fs::write(
            &path,
            "[[rule]]\nname = \"Quiet\"\nmatch = { source = \"docs-bot\" }\ntype = \"generic\"\n",
        )
        .ok();
        assert_eq!(load_rules(&path, None).map(|rules| rules.len()), Some(1));
    }
}
