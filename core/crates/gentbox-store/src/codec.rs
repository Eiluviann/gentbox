//! Column encodings: timestamps as nanoseconds since the epoch, enums as
//! their wire names, nested records as JSON text.

use serde::Serialize;
use serde::de::DeserializeOwned;
use time::OffsetDateTime;

use crate::error::StoreError;

pub(crate) fn to_nanos(instant: OffsetDateTime) -> Result<i64, StoreError> {
    i64::try_from(instant.unix_timestamp_nanos()).map_err(|_| StoreError::TimestampOutOfRange)
}

pub(crate) fn to_optional_nanos(
    instant: Option<OffsetDateTime>,
) -> Result<Option<i64>, StoreError> {
    instant.map(to_nanos).transpose()
}

pub(crate) fn from_nanos(column: &'static str, nanos: i64) -> Result<OffsetDateTime, StoreError> {
    OffsetDateTime::from_unix_timestamp_nanos(i128::from(nanos))
        .map_err(|error| StoreError::corrupt(column, error))
}

pub(crate) fn from_optional_nanos(
    column: &'static str,
    nanos: Option<i64>,
) -> Result<Option<OffsetDateTime>, StoreError> {
    nanos.map(|nanos| from_nanos(column, nanos)).transpose()
}

/// The wire name of a unit enum such as `MessageType` or `LifecycleState`.
///
/// # Panics
///
/// If `value` does not serialise as a plain string, which would mean a core
/// enum stopped being a unit enum.
pub(crate) fn enum_text<T: Serialize>(value: &T) -> String {
    match serde_json::to_value(value) {
        Ok(serde_json::Value::String(name)) => name,
        other => panic!("expected a unit enum serialised as a string, got {other:?}"),
    }
}

pub(crate) fn parse_enum<T: DeserializeOwned>(
    column: &'static str,
    text: &str,
) -> Result<T, StoreError> {
    serde_json::from_value(serde_json::Value::String(text.to_owned()))
        .map_err(|error| StoreError::corrupt(column, error))
}

pub(crate) fn parse_optional_enum<T: DeserializeOwned>(
    column: &'static str,
    text: Option<&str>,
) -> Result<Option<T>, StoreError> {
    text.map(|text| parse_enum(column, text)).transpose()
}

pub(crate) fn to_json<T: Serialize>(column: &'static str, value: &T) -> Result<String, StoreError> {
    serde_json::to_string(value).map_err(|error| StoreError::corrupt(column, error))
}

pub(crate) fn to_optional_json<T: Serialize>(
    column: &'static str,
    value: Option<&T>,
) -> Result<Option<String>, StoreError> {
    value.map(|value| to_json(column, value)).transpose()
}

pub(crate) fn from_json<T: DeserializeOwned>(
    column: &'static str,
    text: &str,
) -> Result<T, StoreError> {
    serde_json::from_str(text).map_err(|error| StoreError::corrupt(column, error))
}

pub(crate) fn from_optional_json<T: DeserializeOwned>(
    column: &'static str,
    text: Option<&str>,
) -> Result<Option<T>, StoreError> {
    text.map(|text| from_json(column, text)).transpose()
}
