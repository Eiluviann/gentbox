//! Every `docs/schema/examples/*.json` deserialises into its core type and
//! serialises back to the same JSON.
#![allow(clippy::unwrap_used, clippy::expect_used)]
mod support;

use std::collections::BTreeSet;
use std::fs;
use std::path::PathBuf;

use gentbox_core::classify::{BuiltInRules, RuleSet};
use gentbox_core::message::MessageInput;
use gentbox_core::prepare::{IngestSettings, prepare_message};
use gentbox_core::rescind::RescindRequest;
use gentbox_core::response::{ActionResponse, IngestResponse};
use gentbox_core::topic::{CloseRequest, ThreadCreate, ThreadRecord, TopicCreate, TopicRecord};
use gentbox_core::validate::validate_message;
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::Value;

fn examples_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../docs/schema/examples")
}

fn read(name: &str) -> Value {
    let path = examples_dir().join(name);
    let text =
        fs::read_to_string(&path).unwrap_or_else(|error| panic!("{}: {error}", path.display()));
    serde_json::from_str(&text).unwrap()
}

fn round_trips<T: DeserializeOwned + Serialize>(name: &str) -> T {
    let json = read(name);
    let value: T =
        serde_json::from_value(json.clone()).unwrap_or_else(|error| panic!("{name}: {error}"));
    assert_eq!(
        serde_json::to_value(&value).unwrap(),
        json,
        "{name} must round-trip"
    );
    value
}

const MESSAGES: &[&str] = &[
    "action-deferrable.json",
    "decision-blocking.json",
    "decision-nonblocking.json",
    "generic.json",
    "message-inline-topic.json",
    "message-thread-session-a.json",
    "message-thread-session-b.json",
    "task-completed.json",
    "undeclared-idle.json",
];
const ACTION_RESPONSES: &[&str] = &[
    "answered-response.json",
    "deferred-response.json",
    "rescinded-response.json",
];
const THREAD_RECORDS: &[&str] = &["thread-record.json", "thread-record-implicit.json"];
const TOPIC_RECORDS: &[&str] = &["topic-record.json", "topic-record-general.json"];

#[test]
fn every_example_file_is_covered_by_a_type() {
    let known: BTreeSet<&str> = MESSAGES
        .iter()
        .chain(ACTION_RESPONSES)
        .chain(THREAD_RECORDS)
        .chain(TOPIC_RECORDS)
        .chain(&[
            "close-request.json",
            "ingest-response.json",
            "rescind-request.json",
            "thread-conversation-takeover.json",
            "topic-create.json",
        ])
        .copied()
        .collect();
    let on_disk: BTreeSet<String> = fs::read_dir(examples_dir())
        .unwrap()
        .map(|entry| entry.unwrap().file_name().to_string_lossy().into_owned())
        .filter(|name| name.ends_with(".json"))
        .collect();
    let on_disk: BTreeSet<&str> = on_disk.iter().map(String::as_str).collect();
    assert_eq!(on_disk, known, "a new example needs a type in this test");
    assert_eq!(known.len(), 21);
}

#[test]
fn message_examples_deserialize_round_trip_validate_and_prepare() {
    for name in MESSAGES {
        let message: MessageInput = round_trips(name);
        validate_message(&message).unwrap_or_else(|error| panic!("{name}: {error}"));
        let prepared = prepare_message(
            message,
            &RuleSet::empty(),
            BuiltInRules::load(),
            IngestSettings::default(),
        )
        .unwrap_or_else(|error| panic!("{name}: {error}"));
        assert!(prepared.message.message_type.is_some(), "{name}");
    }
}

#[test]
fn message_examples_classify_as_their_names_say() {
    use gentbox_core::message::{Blocking, MessageType};
    let table = [
        (
            "decision-blocking.json",
            MessageType::DecisionRequired,
            false,
            Blocking::Blocking,
        ),
        (
            "decision-nonblocking.json",
            MessageType::DecisionRequired,
            false,
            Blocking::NonBlocking,
        ),
        (
            "action-deferrable.json",
            MessageType::ActionRequired,
            true,
            Blocking::Deferrable,
        ),
        (
            "task-completed.json",
            MessageType::TaskCompleted,
            false,
            Blocking::NonBlocking,
        ),
        (
            "generic.json",
            MessageType::Generic,
            false,
            Blocking::NonBlocking,
        ),
        (
            "message-inline-topic.json",
            MessageType::DecisionRequired,
            false,
            Blocking::Deferrable,
        ),
        (
            "undeclared-idle.json",
            MessageType::Generic,
            false,
            Blocking::NonBlocking,
        ),
    ];
    for (name, message_type, urgent, blocking) in table {
        let message: MessageInput = serde_json::from_value(read(name)).unwrap();
        let prepared = prepare_message(
            message,
            &RuleSet::empty(),
            BuiltInRules::load(),
            IngestSettings::default(),
        )
        .unwrap();
        assert_eq!(prepared.message.message_type, Some(message_type), "{name}");
        assert_eq!(prepared.message.urgent, Some(urgent), "{name}");
        assert_eq!(prepared.message.blocking, Some(blocking), "{name}");
    }
}

#[test]
fn the_examples_pass_the_guard_without_warnings() {
    for name in MESSAGES {
        let message: MessageInput = serde_json::from_value(read(name)).unwrap();
        let prepared = prepare_message(
            message,
            &RuleSet::empty(),
            BuiltInRules::load(),
            IngestSettings::default(),
        )
        .unwrap();
        assert!(
            prepared.warnings.is_empty(),
            "{name}: {:?}",
            prepared.warnings
        );
    }
}

#[test]
fn response_and_record_examples_deserialize() {
    for name in ACTION_RESPONSES {
        let _: ActionResponse = round_trips(name);
    }
    for name in THREAD_RECORDS {
        let _: ThreadRecord = round_trips(name);
    }
    for name in TOPIC_RECORDS {
        let _: TopicRecord = round_trips(name);
    }
    let created: TopicCreate = round_trips("topic-create.json");
    assert_eq!(created.key, "release-1.4");
    let _: IngestResponse = round_trips("ingest-response.json");
    let _: RescindRequest = round_trips("rescind-request.json");
    let close: CloseRequest = round_trips("close-request.json");
    assert!(close.rescind_open_messages);
    let takeover: ThreadCreate = round_trips("thread-conversation-takeover.json");
    assert_eq!(takeover.kind.as_deref(), Some("conversation"));
}

#[test]
fn the_ingest_response_example_names_the_title_guard_warning() {
    let response: IngestResponse = serde_json::from_value(read("ingest-response.json")).unwrap();
    assert_eq!(response.warnings.len(), 1);
    assert_eq!(
        response.warnings[0].code,
        gentbox_core::response::WarningCode::TitleLooksLikeCommand
    );
    assert_eq!(response.field_sources.unwrap().urgent.label(), "default");
}

#[test]
fn unknown_fields_are_rejected_like_the_schema_does() {
    let mut json = read("generic.json");
    json["typo"] = Value::Bool(true);
    assert!(serde_json::from_value::<MessageInput>(json).is_err());
}
