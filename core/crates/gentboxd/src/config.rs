//! Where `gentboxd` keeps its state and reads its settings (architecture
//! §3), and the part of `config.toml` it understands.
//!
//! PR 5 brings the full configuration model; `gentboxd` reads only the
//! per-source strict modes and ignores every other table, so a file written
//! by a later Gentbox still starts.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

use gentbox_core::action_plan::ActionPlanMode;
use gentbox_core::prepare::IngestSettings;
use gentbox_core::title_guard::GuardMode;
use serde::Deserialize;
use thiserror::Error;

/// The folders the daemon uses.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Folders {
    /// `hub.db`, `hub.lock` and `API/`.
    pub state: PathBuf,
    /// `config.toml` and `rules.toml`.
    pub config: PathBuf,
}

/// What the environment says about where folders go.
#[derive(Debug, Clone, Default)]
pub struct Environment {
    /// `$HOME`.
    pub home: Option<PathBuf>,
    /// `$XDG_STATE_HOME`.
    pub xdg_state_home: Option<PathBuf>,
    /// `$XDG_CONFIG_HOME`.
    pub xdg_config_home: Option<PathBuf>,
}

/// A settings file that could not be used.
#[derive(Debug, Error)]
pub enum ConfigError {
    /// No home folder and no folder given.
    #[error("cannot find the home folder; pass --state-dir and --config-dir")]
    NoHome,
    /// `config.toml` could not be read.
    #[error("cannot read {path}: {source}")]
    Read {
        /// The file.
        path: PathBuf,
        /// What failed.
        #[source]
        source: std::io::Error,
    },
    /// `config.toml` is not valid.
    #[error("invalid {path}: {message}")]
    Invalid {
        /// The file.
        path: PathBuf,
        /// What TOML or serde said.
        message: String,
    },
}

/// The folders for this platform, unless given: on macOS one folder,
/// `~/Library/Application Support/Gentbox`; on Linux the state in
/// `$XDG_STATE_HOME/gentbox` (default `~/.local/state/gentbox`) and the two
/// editable files in `$XDG_CONFIG_HOME/gentbox` (default `~/.config/gentbox`).
///
/// # Errors
///
/// [`ConfigError::NoHome`] when a folder is needed and there is no home.
pub fn folders(
    state: Option<PathBuf>,
    config: Option<PathBuf>,
    environment: &Environment,
) -> Result<Folders, ConfigError> {
    let home = || environment.home.clone().ok_or(ConfigError::NoHome);
    let state = match state {
        Some(state) => state,
        None if cfg!(target_os = "macos") => home()?.join("Library/Application Support/Gentbox"),
        None => match &environment.xdg_state_home {
            Some(base) if base.is_absolute() => base.join("gentbox"),
            _ => home()?.join(".local/state/gentbox"),
        },
    };
    let config = match config {
        Some(config) => config,
        None if cfg!(target_os = "macos") => state.clone(),
        None => match &environment.xdg_config_home {
            Some(base) if base.is_absolute() => base.join("gentbox"),
            _ => home()?.join(".config/gentbox"),
        },
    };
    Ok(Folders { state, config })
}

/// The strict modes of `config.toml`:
///
/// ```toml
/// [sources.my-adapter]
/// titleGuard = "reject"   # or "rewrite", the default
/// actionPlan = "reject"   # or "rewrite", the default
/// ```
#[derive(Debug, Default, Deserialize)]
struct ConfigFile {
    #[serde(default)]
    sources: BTreeMap<String, SourceEntry>,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct SourceEntry {
    #[serde(default)]
    title_guard: Option<Mode>,
    #[serde(default)]
    action_plan: Option<Mode>,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "snake_case")]
enum Mode {
    Rewrite,
    Reject,
}

/// Reads the per-source strict modes; a missing file means none.
///
/// # Errors
///
/// [`ConfigError`] for an unreadable or invalid file.
pub fn read_sources(path: &Path) -> Result<BTreeMap<String, IngestSettings>, ConfigError> {
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
        Err(source) => {
            return Err(ConfigError::Read {
                path: path.to_path_buf(),
                source,
            });
        }
    };
    parse_sources(&text).map_err(|message| ConfigError::Invalid {
        path: path.to_path_buf(),
        message,
    })
}

fn parse_sources(text: &str) -> Result<BTreeMap<String, IngestSettings>, String> {
    let file: ConfigFile = toml::from_str(text).map_err(|error| error.to_string())?;
    Ok(file
        .sources
        .into_iter()
        .map(|(agent, entry)| {
            let settings = IngestSettings {
                title_guard: match entry.title_guard {
                    Some(Mode::Reject) => GuardMode::Reject,
                    _ => GuardMode::Rewrite,
                },
                action_plan: match entry.action_plan {
                    Some(Mode::Reject) => ActionPlanMode::Reject,
                    _ => ActionPlanMode::Rewrite,
                },
            };
            (agent, settings)
        })
        .collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strict_modes_are_read_per_source_and_other_tables_are_ignored() {
        let sources = parse_sources(
            "[policy]\nanything = 1\n\n[sources.my-adapter]\ntitleGuard = \"reject\"\n\n[sources.other]\nactionPlan = \"reject\"\ntitleGuard = \"rewrite\"\n",
        )
        .unwrap_or_default();
        assert_eq!(sources.len(), 2);
        assert_eq!(sources["my-adapter"].title_guard, GuardMode::Reject);
        assert_eq!(sources["my-adapter"].action_plan, ActionPlanMode::Rewrite);
        assert_eq!(sources["other"].action_plan, ActionPlanMode::Reject);
        assert_eq!(sources["other"].title_guard, GuardMode::Rewrite);
    }

    #[test]
    fn a_typo_in_a_source_entry_is_an_error() {
        assert!(parse_sources("[sources.a]\ntitleGard = \"reject\"\n").is_err());
        assert!(parse_sources("[sources.a]\ntitleGuard = \"strict\"\n").is_err());
    }

    #[test]
    fn the_linux_folders_follow_xdg() {
        if cfg!(target_os = "macos") {
            return;
        }
        let environment = Environment {
            home: Some(PathBuf::from("/home/me")),
            xdg_state_home: None,
            xdg_config_home: Some(PathBuf::from("/xdg/config")),
        };
        let found = folders(None, None, &environment).ok();
        assert_eq!(
            found,
            Some(Folders {
                state: PathBuf::from("/home/me/.local/state/gentbox"),
                config: PathBuf::from("/xdg/config/gentbox"),
            })
        );
    }

    #[test]
    fn the_macos_folder_holds_everything() {
        if !cfg!(target_os = "macos") {
            return;
        }
        let environment = Environment {
            home: Some(PathBuf::from("/Users/me")),
            ..Environment::default()
        };
        let found = folders(None, None, &environment).ok();
        let expected = PathBuf::from("/Users/me/Library/Application Support/Gentbox");
        assert_eq!(
            found,
            Some(Folders {
                state: expected.clone(),
                config: expected,
            })
        );
    }

    #[test]
    fn a_given_state_folder_wins_and_the_config_folder_follows_the_platform() {
        let environment = Environment {
            home: Some(PathBuf::from("/home/me")),
            ..Environment::default()
        };
        let found = folders(Some(PathBuf::from("/s")), None, &environment).ok();
        let config = if cfg!(target_os = "macos") {
            PathBuf::from("/s")
        } else {
            PathBuf::from("/home/me/.config/gentbox")
        };
        assert_eq!(
            found,
            Some(Folders {
                state: PathBuf::from("/s"),
                config,
            })
        );
        let both = folders(
            Some(PathBuf::from("/s")),
            Some(PathBuf::from("/c")),
            &Environment::default(),
        )
        .ok()
        .map(|folders| folders.config);
        assert_eq!(
            both,
            Some(PathBuf::from("/c")),
            "no home needed when both are given"
        );
    }
}
