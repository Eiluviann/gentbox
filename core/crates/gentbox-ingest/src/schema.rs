//! Validation against `docs/schema/message-v1.json`, embedded at build time:
//! the whole schema for `POST /v1/messages`, one `$defs` entry for every
//! other body.

use std::sync::LazyLock;

use jsonschema::Validator;
use serde_json::{Value, json};

use crate::error::ApiError;

/// The schema file.
pub const MESSAGE_SCHEMA: &str = include_str!("../../../../docs/schema/message-v1.json");

/// Which shape a body must have.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BodyShape {
    /// The message itself (the schema root).
    Message,
    /// A `$defs` entry.
    Definition(Definition),
}

/// The `$defs` entries request bodies use.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Definition {
    /// `DELETE /v1/messages/{id}`, the conversation and group rescinds.
    RescindRequest,
    /// `POST /v1/topics`.
    TopicCreate,
    /// `PATCH /v1/topics/{id}`.
    TopicUpdate,
    /// The topic and thread `close` routes.
    CloseRequest,
    /// `POST /v1/threads`.
    ThreadCreate,
    /// `PATCH /v1/threads/{id}`.
    ThreadUpdate,
    /// `PUT /v1/threads/{id}/topic`.
    ThreadMove,
    /// `PUT /v1/messages/{id}/thread`.
    MessageThreadMove,
    /// `PUT /v1/conversations/{agent}/{id}`.
    Conversation,
}

impl Definition {
    /// Every definition, in the order the validators are compiled.
    pub const ALL: [Self; 9] = [
        Self::RescindRequest,
        Self::TopicCreate,
        Self::TopicUpdate,
        Self::CloseRequest,
        Self::ThreadCreate,
        Self::ThreadUpdate,
        Self::ThreadMove,
        Self::MessageThreadMove,
        Self::Conversation,
    ];

    /// The entry's name under `$defs`.
    pub fn name(self) -> &'static str {
        match self {
            Self::RescindRequest => "rescindRequest",
            Self::TopicCreate => "topicCreate",
            Self::TopicUpdate => "topicUpdate",
            Self::CloseRequest => "closeRequest",
            Self::ThreadCreate => "threadCreate",
            Self::ThreadUpdate => "threadUpdate",
            Self::ThreadMove => "threadMove",
            Self::MessageThreadMove => "messageThreadMove",
            Self::Conversation => "conversation",
        }
    }
}

struct Schemas {
    message: Validator,
    /// One validator per [`Definition::ALL`] entry, in that order.
    definitions: Vec<Validator>,
}

static SCHEMAS: LazyLock<Schemas> = LazyLock::new(compile);

/// Validates a parsed body; the first failure becomes 422 `schema_invalid`
/// with its JSON pointer.
///
/// # Errors
///
/// [`ApiError::schema_invalid`] for a body that does not match.
pub fn validate(shape: BodyShape, body: &Value) -> Result<(), ApiError> {
    let validator = match shape {
        BodyShape::Message => &SCHEMAS.message,
        BodyShape::Definition(definition) => Definition::ALL
            .iter()
            .position(|candidate| *candidate == definition)
            .and_then(|index| SCHEMAS.definitions.get(index))
            .unwrap_or_else(|| unreachable!("a validator is compiled for every definition")),
    };
    match validator.iter_errors(body).next() {
        None => Ok(()),
        Some(error) => Err(ApiError::schema_invalid(
            error.instance_path().to_string(),
            error.to_string(),
        )),
    }
}

fn compile() -> Schemas {
    // The schema is part of the binary and a test compiles it, so a failure
    // here is a build defect.
    #[allow(clippy::expect_used)]
    let root: Value = serde_json::from_str(MESSAGE_SCHEMA).expect("the embedded schema is JSON");
    let message = build(&root);
    let definitions = Definition::ALL
        .iter()
        .map(|definition| {
            let wrapper = json!({
                "$schema": root["$schema"],
                "$ref": format!("#/$defs/{}", definition.name()),
                "$defs": root["$defs"],
            });
            build(&wrapper)
        })
        .collect();
    Schemas {
        message,
        definitions,
    }
}

fn build(schema: &Value) -> Validator {
    #[allow(clippy::expect_used)]
    jsonschema::options()
        .should_validate_formats(true)
        .offline()
        .build(schema)
        .expect("the embedded schema compiles")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_definition_compiles_and_exists() {
        let root: Value = serde_json::from_str(MESSAGE_SCHEMA).unwrap_or_default();
        for definition in Definition::ALL {
            let name = definition.name();
            assert!(root["$defs"].get(name).is_some(), "{name}");
        }
        assert_eq!(SCHEMAS.definitions.len(), Definition::ALL.len());
    }

    #[test]
    fn a_failure_names_its_pointer() {
        let error = validate(
            BodyShape::Message,
            &json!({ "version": 1, "source": { "agent": "a" }, "conversation": { "id": "c" }, "title": "" }),
        )
        .err();
        assert_eq!(
            error.and_then(|error| error.body().pointer.clone()),
            Some("/title".to_owned())
        );
    }
}
