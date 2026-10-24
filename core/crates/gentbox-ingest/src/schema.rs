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
    /// A `$defs` entry, by name.
    Definition(&'static str),
}

/// The definitions request bodies use.
const DEFINITIONS: [&str; 9] = [
    "rescindRequest",
    "topicCreate",
    "topicUpdate",
    "closeRequest",
    "threadCreate",
    "threadUpdate",
    "threadMove",
    "messageThreadMove",
    "conversation",
];

struct Schemas {
    message: Validator,
    definitions: Vec<(&'static str, Validator)>,
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
        BodyShape::Definition(name) => SCHEMAS
            .definitions
            .iter()
            .find(|(definition, _)| *definition == name)
            .map(|(_, validator)| validator)
            .unwrap_or(&SCHEMAS.message),
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
    let definitions = DEFINITIONS
        .iter()
        .map(|name| {
            let wrapper = json!({
                "$schema": root["$schema"],
                "$ref": format!("#/$defs/{name}"),
                "$defs": root["$defs"],
            });
            (*name, build(&wrapper))
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
        for name in DEFINITIONS {
            assert!(root["$defs"].get(name).is_some(), "{name}");
        }
        assert_eq!(SCHEMAS.definitions.len(), DEFINITIONS.len());
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
