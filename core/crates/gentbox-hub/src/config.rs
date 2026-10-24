//! What a hub starts with: where its state lives, the rules, the policy, the
//! per-source strict modes and whether anything can answer.

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::Arc;

use gentbox_core::classify::RuleSet;
use gentbox_core::policy::Policy;
use gentbox_core::prepare::IngestSettings;
use gentbox_store::RetentionPolicy;

use crate::clock::{Clock, SystemClock};

/// The database file's name inside the state folder.
pub const DATABASE_FILE_NAME: &str = "hub.db";

/// The hub's settings. Everything but the state folder can be changed while
/// it runs through [`crate::HubHandle`].
#[derive(Debug, Clone)]
pub struct HubConfig {
    /// The state folder: `hub.db` and `hub.lock` live here (architecture §3).
    pub state_dir: PathBuf,
    /// The user's rules (`rules.toml`).
    pub rules: RuleSet,
    /// The policy table (`config.toml`).
    pub policy: Policy,
    /// Per-source strict modes, keyed on `source.agent` (`titleGuard`,
    /// `actionPlan`); sources not listed get the defaults.
    pub sources: BTreeMap<String, IngestSettings>,
    /// How long closed records are kept.
    pub retention: RetentionPolicy,
    /// Whether a channel that can answer is attached: the Mac app's UI.
    /// `gentboxd` has none, so its senders get `shouldWait: false` (no channel
    /// can answer, architecture §5.1).
    pub has_answer_channel: bool,
    /// Where the hub reads the time.
    pub clock: Arc<dyn Clock>,
}

impl HubConfig {
    /// Defaults for a state folder: no user rules, the default policy and
    /// retention, an answer channel, the system clock.
    pub fn new(state_dir: impl Into<PathBuf>) -> Self {
        Self {
            state_dir: state_dir.into(),
            rules: RuleSet::empty(),
            policy: Policy::default(),
            sources: BTreeMap::new(),
            retention: RetentionPolicy::default(),
            has_answer_channel: true,
            clock: Arc::new(SystemClock),
        }
    }

    /// The database file.
    pub fn database_path(&self) -> PathBuf {
        self.state_dir.join(DATABASE_FILE_NAME)
    }
}

/// The strict modes for one source.
pub(crate) fn source_settings(
    sources: &BTreeMap<String, IngestSettings>,
    agent: &str,
) -> IngestSettings {
    sources.get(agent).copied().unwrap_or_default()
}
