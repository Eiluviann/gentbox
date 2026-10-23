//! Classification (architecture §5.3, ADR 0007): `type`, `urgent` and
//! `blocking` are resolved field by field, then normalized.
//!
//! 1. **Declared** in the message: wins.
//! 2. **User rules** from `rules.toml`, in file order: each matching rule
//!    sets only the fields still missing.
//! 3. **Built-in rules**, the same way.
//! 4. **Defaults:** `generic`, `false`, `non_blocking`; a message with `steps`
//!    and no declared or rule-set type defaults to `action_required`.
//! 5. **Normalize:** `task_completed` and `generic` become `non_blocking`,
//!    `generic` becomes not urgent; `scope` and `steps` that no longer belong
//!    to the final type are dropped. Every normalization is recorded.
//!
//! A user rule with `override = true` also replaces *declared* fields. Among
//! rules the first one that sets a field wins; `override` never beats an
//! earlier rule, only the sender.

use std::collections::BTreeMap;
use std::fmt;
use std::sync::OnceLock;

use globset::{Glob, GlobBuilder, GlobMatcher};
use regex::Regex;
use serde::de::{self, Deserializer};
use serde::{Deserialize, Serialize, Serializer};
use thiserror::Error;

use crate::message::{Blocking, MessageInput, MessageScope, MessageType};
use crate::response::{IngestWarning, WarningCode};
use crate::template::GroupTemplate;

const BUILT_IN_RULES_TOML: &str = include_str!("builtin_rules.toml");

/// Where one classified field came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FieldSource {
    /// The sender declared it.
    Declared,
    /// A rule from the user's `rules.toml`, by name.
    UserRule(String),
    /// A built-in rule, by name.
    BuiltInRule(String),
    /// The default.
    Default,
    /// Changed to make the three fields consistent.
    Normalized,
}

impl FieldSource {
    /// The label used in `fieldSources`: `declared`, `default`, `normalized`,
    /// `rule:<name>` or `builtin:<name>`.
    pub fn label(&self) -> String {
        match self {
            Self::Declared => "declared".to_owned(),
            Self::UserRule(name) => format!("rule:{name}"),
            Self::BuiltInRule(name) => format!("builtin:{name}"),
            Self::Default => "default".to_owned(),
            Self::Normalized => "normalized".to_owned(),
        }
    }

    fn from_label(label: &str) -> Option<Self> {
        match label {
            "declared" => Some(Self::Declared),
            "default" => Some(Self::Default),
            "normalized" => Some(Self::Normalized),
            other => other
                .strip_prefix("rule:")
                .map(|name| Self::UserRule(name.to_owned()))
                .or_else(|| {
                    other
                        .strip_prefix("builtin:")
                        .map(|name| Self::BuiltInRule(name.to_owned()))
                }),
        }
    }
}

impl Serialize for FieldSource {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.label())
    }
}

impl<'de> Deserialize<'de> for FieldSource {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let label = String::deserialize(deserializer)?;
        Self::from_label(&label)
            .ok_or_else(|| de::Error::custom(format!("unknown field source `{label}`")))
    }
}

/// Where each of the three fields came from (`fieldSources` in the ingest
/// response).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FieldSources {
    /// Source of `type`.
    #[serde(rename = "type")]
    pub message_type: FieldSource,
    /// Source of `urgent`.
    pub urgent: FieldSource,
    /// Source of `blocking`.
    pub blocking: FieldSource,
}

/// A resolved field value and where it came from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Resolved<T> {
    /// The value.
    pub value: T,
    /// Where it came from.
    pub source: FieldSource,
}

/// What a normalization changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NormalizedField {
    /// `urgent` became false.
    Urgent,
    /// `blocking` became `non_blocking`.
    Blocking,
    /// `scope` was dropped.
    Scope,
    /// `steps` were dropped.
    Steps,
}

/// One recorded normalization, for the delivery log and the sender.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Normalization {
    /// The field that changed.
    pub field: NormalizedField,
    /// Its value before.
    pub from: String,
    /// Its value after.
    pub to: String,
}

impl fmt::Display for Normalization {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = match self.field {
            NormalizedField::Urgent => "urgent",
            NormalizedField::Blocking => "blocking",
            NormalizedField::Scope => "scope",
            NormalizedField::Steps => "steps",
        };
        write!(
            formatter,
            "{name} changed from {} to {}",
            self.from, self.to
        )
    }
}

/// A topic or thread template a rule assigned, with the rule's name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleTemplate {
    /// The rule that assigned it.
    pub rule: String,
    /// The template.
    pub template: GroupTemplate,
}

/// The outcome of classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Classification {
    /// Final type.
    pub message_type: Resolved<MessageType>,
    /// Final urgency.
    pub urgent: Resolved<bool>,
    /// Final blocking level.
    pub blocking: Resolved<Blocking>,
    /// The rule that muted the message. A rule's mute never changes the type,
    /// urgency or blocking level; the policy routes the message like one in a
    /// muted topic (non-urgent to the inbox only, urgent still breaks through).
    pub muted_by_rule: Option<String>,
    /// `scope`, kept only on `task_completed`.
    pub scope: Option<MessageScope>,
    /// Whether the message's `steps` survive (only on `action_required`).
    pub keeps_steps: bool,
    /// The topic a rule assigned, for the sender's unnamed topic.
    pub rule_topic: Option<RuleTemplate>,
    /// The thread a rule assigned, for the sender's unnamed thread.
    pub rule_thread: Option<RuleTemplate>,
    /// Every change normalization made.
    pub normalizations: Vec<Normalization>,
}

impl Classification {
    /// The sources of the three fields.
    pub fn field_sources(&self) -> FieldSources {
        FieldSources {
            message_type: self.message_type.source.clone(),
            urgent: self.urgent.source.clone(),
            blocking: self.blocking.source.clone(),
        }
    }

    /// One `normalized` warning per recorded normalization.
    pub fn normalization_warnings(&self) -> Vec<IngestWarning> {
        self.normalizations
			.iter()
			.map(|normalization| {
				let field = match normalization.field {
					NormalizedField::Urgent => "urgent",
					NormalizedField::Blocking => "blocking",
					NormalizedField::Scope => "scope",
					NormalizedField::Steps => "steps",
				};
				IngestWarning::new(
					WarningCode::Normalized,
					field,
					format!("{normalization} so the message's type, urgency and blocking level are consistent."),
				)
			})
			.collect()
    }
}

/// What a rule matches on. All set keys must hold.
#[derive(Debug, Clone)]
pub struct RuleMatch {
    source: Option<String>,
    event: Option<GlobMatcher>,
    title: Option<Regex>,
    cwd: Option<GlobMatcher>,
    metadata: Vec<(String, String)>,
}

impl RuleMatch {
    fn matches(&self, message: &MessageInput) -> bool {
        self.source
            .as_deref()
            .is_none_or(|source| message.source.agent == source)
            && self.event.as_ref().is_none_or(|glob| {
                message
                    .event
                    .as_deref()
                    .is_some_and(|event| glob.is_match(event))
            })
            && self
                .title
                .as_ref()
                .is_none_or(|regex| regex.is_match(&message.title))
            && self.cwd.as_ref().is_none_or(|glob| {
                message
                    .conversation
                    .cwd
                    .as_deref()
                    .is_some_and(|cwd| glob.is_match(cwd))
            })
            && self.metadata.iter().all(|(key, expected)| {
                message
                    .metadata
                    .as_ref()
                    .and_then(|metadata| metadata.get(key))
                    .is_some_and(|value| &value.to_comparable_text() == expected)
            })
    }
}

/// One `[[rule]]`.
#[derive(Debug, Clone)]
pub struct Rule {
    /// The rule's name, shown as the source of the fields it sets.
    pub name: String,
    matcher: RuleMatch,
    /// `type` it sets.
    pub message_type: Option<MessageType>,
    /// `urgent` it sets.
    pub urgent: Option<bool>,
    /// `blocking` it sets.
    pub blocking: Option<Blocking>,
    /// `mute` it sets.
    pub mute: Option<bool>,
    /// The topic template it assigns.
    pub topic: Option<GroupTemplate>,
    /// The thread template it assigns.
    pub thread: Option<GroupTemplate>,
    /// Also replaces declared fields.
    pub override_declared: bool,
}

/// A parsed rules file.
#[derive(Debug, Clone, Default)]
pub struct RuleSet {
    rules: Vec<Rule>,
}

/// Why a rules file was not accepted. The hub keeps the last good rules and
/// shows this in Settings.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum RuleParseError {
    /// The file is not valid TOML or does not have the rules' shape.
    #[error("{message}")]
    Syntax {
        /// What TOML or serde said.
        message: String,
        /// 1-based line, when known.
        line: Option<usize>,
        /// 1-based column, when known.
        column: Option<usize>,
    },
    /// A rule is well-formed TOML but invalid: a bad regex or glob, an unknown
    /// placeholder.
    #[error("rule \"{rule}\": {message}")]
    InvalidRule {
        /// The rule's name.
        rule: String,
        /// What is wrong.
        message: String,
        /// 1-based line of the rule's `[[rule]]` header, when found.
        line: Option<usize>,
    },
}

impl RuleParseError {
    /// The line the error points at, when known.
    pub fn line(&self) -> Option<usize> {
        match self {
            Self::Syntax { line, .. } | Self::InvalidRule { line, .. } => *line,
        }
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRulesFile {
    #[serde(default)]
    rule: Vec<RawRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawRule {
    name: String,
    #[serde(default, rename = "match")]
    matcher: RawMatch,
    #[serde(default, rename = "type")]
    message_type: Option<MessageType>,
    #[serde(default)]
    urgent: Option<bool>,
    #[serde(default)]
    blocking: Option<Blocking>,
    #[serde(default)]
    mute: Option<bool>,
    #[serde(default)]
    topic: Option<GroupTemplate>,
    #[serde(default)]
    thread: Option<GroupTemplate>,
    #[serde(default, rename = "override")]
    override_declared: bool,
}

#[derive(Deserialize, Default)]
#[serde(deny_unknown_fields)]
struct RawMatch {
    #[serde(default)]
    source: Option<String>,
    #[serde(default)]
    event: Option<String>,
    #[serde(default)]
    title: Option<String>,
    #[serde(default)]
    cwd: Option<String>,
    #[serde(default)]
    metadata: BTreeMap<String, MatchScalar>,
}

#[derive(Deserialize)]
#[serde(untagged)]
enum MatchScalar {
    Text(String),
    Integer(i64),
    Flag(bool),
}

impl RuleSet {
    /// A rule set with no rules.
    pub fn empty() -> Self {
        Self::default()
    }

    /// Parses a rules file. A leading `~` in a `cwd` glob stays literal; use
    /// [`RuleSet::parse_with_home`] to expand it.
    pub fn parse(source: &str) -> Result<Self, RuleParseError> {
        Self::parse_inner(source, None)
    }

    /// Parses a rules file, expanding a leading `~` in `cwd` globs to
    /// `home_dir` (the core never reads the environment itself).
    pub fn parse_with_home(source: &str, home_dir: &str) -> Result<Self, RuleParseError> {
        Self::parse_inner(source, Some(home_dir))
    }

    /// How many rules the file has.
    pub fn len(&self) -> usize {
        self.rules.len()
    }

    /// Whether the file has no rules.
    pub fn is_empty(&self) -> bool {
        self.rules.is_empty()
    }

    /// The rules in file order.
    pub fn rules(&self) -> &[Rule] {
        &self.rules
    }

    fn parse_inner(source: &str, home_dir: Option<&str>) -> Result<Self, RuleParseError> {
        let raw: RawRulesFile =
            toml::from_str(source).map_err(|error| syntax_error(source, &error))?;
        let rules = raw
            .rule
            .into_iter()
            .enumerate()
            .map(|(index, raw_rule)| compile_rule(source, index, raw_rule, home_dir))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Self { rules })
    }
}

/// The Claude Code mapping of architecture §5.3, shipped with the app and
/// applied after the user's rules.
#[derive(Debug, Clone)]
pub struct BuiltInRules {
    rules: RuleSet,
}

impl BuiltInRules {
    /// The embedded rules.
    pub fn load() -> &'static Self {
        static BUILT_IN: OnceLock<BuiltInRules> = OnceLock::new();
        BUILT_IN.get_or_init(|| match RuleSet::parse(BUILT_IN_RULES_TOML) {
            Ok(rules) => Self { rules },
            // A broken embedded file is a build defect; a test parses it.
            Err(error) => panic!("the embedded built-in rules are invalid: {error}"),
        })
    }

    /// The rules in table order.
    pub fn rules(&self) -> &[Rule] {
        self.rules.rules()
    }
}

/// Classifies a message: declared fields, user rules, built-in rules,
/// defaults, normalization.
pub fn classify(
    message: &MessageInput,
    user_rules: &RuleSet,
    built_in_rules: &BuiltInRules,
) -> Classification {
    let mut slots = Slots::from_declared(message);
    for rule in user_rules.rules() {
        slots.apply(
            rule,
            message,
            FieldSource::UserRule(rule.name.clone()),
            true,
        );
    }
    for rule in built_in_rules.rules() {
        slots.apply(
            rule,
            message,
            FieldSource::BuiltInRule(rule.name.clone()),
            false,
        );
    }
    slots.finish(message)
}

struct Slots {
    message_type: Option<Resolved<MessageType>>,
    urgent: Option<Resolved<bool>>,
    blocking: Option<Resolved<Blocking>>,
    /// Set by a rule: later rules never replace it.
    type_from_rule: bool,
    urgent_from_rule: bool,
    blocking_from_rule: bool,
    muted_by_rule: Option<(String, bool)>,
    rule_topic: Option<RuleTemplate>,
    rule_thread: Option<RuleTemplate>,
}

impl Slots {
    fn from_declared(message: &MessageInput) -> Self {
        Self {
            message_type: message.message_type.map(declared_field),
            urgent: message.urgent.map(declared_field),
            blocking: message.blocking.map(declared_field),
            type_from_rule: false,
            urgent_from_rule: false,
            blocking_from_rule: false,
            muted_by_rule: None,
            rule_topic: None,
            rule_thread: None,
        }
    }

    fn apply(
        &mut self,
        rule: &Rule,
        message: &MessageInput,
        source: FieldSource,
        may_override: bool,
    ) {
        if !rule.matcher.matches(message) {
            return;
        }
        let overrides = may_override && rule.override_declared;
        if let Some(value) = rule.message_type {
            set_field(
                &mut self.message_type,
                &mut self.type_from_rule,
                value,
                &source,
                overrides,
            );
        }
        if let Some(value) = rule.urgent {
            set_field(
                &mut self.urgent,
                &mut self.urgent_from_rule,
                value,
                &source,
                overrides,
            );
        }
        if let Some(value) = rule.blocking {
            set_field(
                &mut self.blocking,
                &mut self.blocking_from_rule,
                value,
                &source,
                overrides,
            );
        }
        if self.muted_by_rule.is_none()
            && let Some(mute) = rule.mute
        {
            self.muted_by_rule = Some((rule.name.clone(), mute));
        }
        if self.rule_topic.is_none() {
            self.rule_topic = rule.topic.clone().map(|template| RuleTemplate {
                rule: rule.name.clone(),
                template,
            });
        }
        if self.rule_thread.is_none() {
            self.rule_thread = rule.thread.clone().map(|template| RuleTemplate {
                rule: rule.name.clone(),
                template,
            });
        }
    }

    fn finish(self, message: &MessageInput) -> Classification {
        let muted_by_rule = self
            .muted_by_rule
            .and_then(|(name, mute)| mute.then_some(name));
        let message_type = self.message_type.unwrap_or_else(|| Resolved {
            value: if message.steps.is_some() {
                MessageType::ActionRequired
            } else {
                MessageType::Generic
            },
            source: FieldSource::Default,
        });
        let mut urgent = self.urgent.unwrap_or(Resolved {
            value: false,
            source: FieldSource::Default,
        });
        let mut blocking = self.blocking.unwrap_or(Resolved {
            value: Blocking::NonBlocking,
            source: FieldSource::Default,
        });
        let mut normalizations = Vec::new();
        if !message_type.value.can_block() && blocking.value != Blocking::NonBlocking {
            normalizations.push(Normalization {
                field: NormalizedField::Blocking,
                from: blocking.value.as_str().to_owned(),
                to: Blocking::NonBlocking.as_str().to_owned(),
            });
            blocking = Resolved {
                value: Blocking::NonBlocking,
                source: FieldSource::Normalized,
            };
        }
        if !message_type.value.can_be_urgent() && urgent.value {
            normalizations.push(Normalization {
                field: NormalizedField::Urgent,
                from: "true".to_owned(),
                to: "false".to_owned(),
            });
            urgent = Resolved {
                value: false,
                source: FieldSource::Normalized,
            };
        }
        let scope = match message.scope {
            Some(scope) if message_type.value == MessageType::TaskCompleted => Some(scope),
            Some(_) => {
                normalizations.push(Normalization {
                    field: NormalizedField::Scope,
                    from: "set".to_owned(),
                    to: "dropped".to_owned(),
                });
                None
            }
            None => None,
        };
        let keeps_steps = message_type.value == MessageType::ActionRequired;
        if message.steps.is_some() && !keeps_steps {
            normalizations.push(Normalization {
                field: NormalizedField::Steps,
                from: "set".to_owned(),
                to: "dropped".to_owned(),
            });
        }
        Classification {
            message_type,
            urgent,
            blocking,
            muted_by_rule,
            scope,
            keeps_steps,
            rule_topic: self.rule_topic,
            rule_thread: self.rule_thread,
            normalizations,
        }
    }
}

fn declared_field<T>(value: T) -> Resolved<T> {
    Resolved {
        value,
        source: FieldSource::Declared,
    }
}

/// A rule sets a field that is missing, or that the sender declared when the
/// rule overrides; never one an earlier rule set.
fn set_field<T>(
    slot: &mut Option<Resolved<T>>,
    set_by_rule: &mut bool,
    value: T,
    source: &FieldSource,
    overrides: bool,
) {
    if *set_by_rule {
        return;
    }
    let is_declared = slot
        .as_ref()
        .is_some_and(|resolved| resolved.source == FieldSource::Declared);
    if slot.is_none() || (is_declared && overrides) {
        *slot = Some(Resolved {
            value,
            source: source.clone(),
        });
        *set_by_rule = true;
    }
}

fn compile_rule(
    source: &str,
    index: usize,
    raw: RawRule,
    home_dir: Option<&str>,
) -> Result<Rule, RuleParseError> {
    let invalid = |message: String| RuleParseError::InvalidRule {
        rule: raw.name.clone(),
        message,
        line: rule_header_line(source, index),
    };
    let matcher = RuleMatch {
        source: raw.matcher.source.clone(),
        event: raw
            .matcher
            .event
            .as_deref()
            .map(|pattern| compile_glob(pattern, None))
            .transpose()
            .map_err(|error| invalid(format!("match.event: {error}")))?,
        title: raw
            .matcher
            .title
            .as_deref()
            .map(Regex::new)
            .transpose()
            .map_err(|error| invalid(format!("match.title: {error}")))?,
        cwd: raw
            .matcher
            .cwd
            .as_deref()
            .map(|pattern| compile_glob(pattern, home_dir))
            .transpose()
            .map_err(|error| invalid(format!("match.cwd: {error}")))?,
        metadata: raw
            .matcher
            .metadata
            .iter()
            .map(|(key, value)| (key.clone(), value_text(value)))
            .collect(),
    };
    for (field, template) in [("topic", &raw.topic), ("thread", &raw.thread)] {
        if let Some(template) = template {
            template
                .check()
                .map_err(|error| invalid(format!("{field}: {error}")))?;
        }
    }
    Ok(Rule {
        name: raw.name.clone(),
        matcher,
        message_type: raw.message_type,
        urgent: raw.urgent,
        blocking: raw.blocking,
        mute: raw.mute,
        topic: raw.topic,
        thread: raw.thread,
        override_declared: raw.override_declared,
    })
}

fn value_text(value: &MatchScalar) -> String {
    match value {
        MatchScalar::Text(text) => text.clone(),
        MatchScalar::Integer(number) => number.to_string(),
        MatchScalar::Flag(flag) => flag.to_string(),
    }
}

fn compile_glob(pattern: &str, home_dir: Option<&str>) -> Result<GlobMatcher, globset::Error> {
    let expanded = match (home_dir, pattern.strip_prefix('~')) {
        (Some(home), Some(rest)) if rest.is_empty() || rest.starts_with('/') => {
            format!("{}{rest}", home.trim_end_matches('/'))
        }
        _ => pattern.to_owned(),
    };
    let glob: Glob = GlobBuilder::new(&expanded)
        .literal_separator(true)
        .build()?;
    Ok(glob.compile_matcher())
}

fn syntax_error(source: &str, error: &toml::de::Error) -> RuleParseError {
    let (line, column) = match error.span() {
        Some(span) => {
            let before = &source[..span.start.min(source.len())];
            let line = before.matches('\n').count() + 1;
            let column = before
                .rsplit('\n')
                .next()
                .map_or(0, |last| last.chars().count())
                + 1;
            (Some(line), Some(column))
        }
        None => (None, None),
    };
    RuleParseError::Syntax {
        message: error.message().to_owned(),
        line,
        column,
    }
}

/// A `[[rule]]` header line, with or without a trailing comment.
fn is_rule_header(line: &str) -> bool {
    line.trim()
        .strip_prefix("[[rule]]")
        .is_some_and(|rest| rest.trim().is_empty() || rest.trim_start().starts_with('#'))
}

fn rule_header_line(source: &str, index: usize) -> Option<usize> {
    source
        .lines()
        .enumerate()
        .filter(|(_, line)| is_rule_header(line))
        .nth(index)
        .map(|(line_index, _)| line_index + 1)
}
