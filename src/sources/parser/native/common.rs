//! Bounded strict deserialization and shared scalar validators.

use super::ParserError;
use super::opts::{OptKind, OptRule, OptsSpec, optional, required};
use serde::de::{self, DeserializeSeed, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};
use std::collections::BTreeMap;
use std::fmt;
use std::net::IpAddr;

use super::MAX_NATIVE_DEPTH;
use super::MAX_NATIVE_ENTRIES;

pub(crate) const MAX_NAME_BYTES: usize = 128;
pub(crate) const MAX_SECRET_BYTES: usize = 4096;
pub(crate) const MAX_HOST_BYTES: usize = 253;

#[derive(Default)]
pub(crate) struct ParseBudget {
    entries: usize,
    failure: Option<ParserError>,
}

impl ParseBudget {
    pub(crate) fn entry<E: de::Error>(&mut self) -> Result<(), E> {
        match self.entries.checked_add(1) {
            Some(value) if value <= MAX_NATIVE_ENTRIES => {
                self.entries = value;
                Ok(())
            }
            _ => {
                self.failure = Some(ParserError::TooManyEntries);
                Err(E::custom("bounded entry limit"))
            }
        }
    }

    pub(crate) fn fail<E: de::Error>(&mut self, error: ParserError) -> E {
        self.failure = Some(error);
        E::custom("bounded native parse rejected")
    }
}

enum StrictValue {
    Null,
    Bool(bool),
    Number(Number),
    String(String),
    Sequence(Vec<StrictValue>),
    Mapping(BTreeMap<String, StrictValue>),
}

impl StrictValue {
    fn into_json(self) -> Value {
        match self {
            Self::Null => Value::Null,
            Self::Bool(value) => Value::Bool(value),
            Self::Number(value) => Value::Number(value),
            Self::String(value) => Value::String(value),
            Self::Sequence(values) => {
                Value::Array(values.into_iter().map(Self::into_json).collect())
            }
            Self::Mapping(values) => Value::Object(
                values
                    .into_iter()
                    .map(|(key, value)| (key, value.into_json()))
                    .collect(),
            ),
        }
    }
}

struct StrictValueSeed<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> DeserializeSeed<'de> for StrictValueSeed<'_> {
    type Value = StrictValue;

    fn deserialize<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        if self.depth > MAX_NATIVE_DEPTH {
            return Err(self.budget.fail(ParserError::TooDeep));
        }
        deserializer.deserialize_any(StrictValueVisitor {
            budget: self.budget,
            depth: self.depth,
        })
    }
}

struct StrictValueVisitor<'a> {
    budget: &'a mut ParseBudget,
    depth: usize,
}

impl<'de> Visitor<'de> for StrictValueVisitor<'_> {
    type Value = StrictValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a bounded JSON-compatible YAML/JSON value")
    }

    fn visit_unit<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_none<E: de::Error>(self) -> Result<Self::Value, E> {
        Ok(StrictValue::Null)
    }

    fn visit_bool<E: de::Error>(self, value: bool) -> Result<Self::Value, E> {
        Ok(StrictValue::Bool(value))
    }

    fn visit_i64<E: de::Error>(self, value: i64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_u64<E: de::Error>(self, value: u64) -> Result<Self::Value, E> {
        Ok(StrictValue::Number(Number::from(value)))
    }

    fn visit_f64<E: de::Error>(self, value: f64) -> Result<Self::Value, E> {
        Number::from_f64(value)
            .map(StrictValue::Number)
            .ok_or_else(|| E::custom("invalid native number"))
    }

    fn visit_str<E: de::Error>(self, value: &str) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_borrowed_str<E: de::Error>(self, value: &'de str) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value.to_owned()))
    }

    fn visit_string<E: de::Error>(self, value: String) -> Result<Self::Value, E> {
        Ok(StrictValue::String(value))
    }

    fn visit_some<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        }
        .deserialize(deserializer)
    }

    fn visit_newtype_struct<D>(self, deserializer: D) -> Result<Self::Value, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        }
        .deserialize(deserializer)
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element_seed(StrictValueSeed {
            budget: self.budget,
            depth: self.depth + 1,
        })? {
            self.budget.entry::<A::Error>()?;
            values.push(value);
        }
        Ok(StrictValue::Sequence(values))
    }

    fn visit_map<A>(self, mut mapping: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = BTreeMap::new();
        while let Some(key) = mapping.next_key::<String>()? {
            self.budget.entry::<A::Error>()?;
            if values.contains_key(&key) {
                return Err(self.budget.fail(ParserError::DuplicateKey));
            }
            let value = mapping.next_value_seed(StrictValueSeed {
                budget: self.budget,
                depth: self.depth + 1,
            })?;
            values.insert(key, value);
        }
        Ok(StrictValue::Mapping(values))
    }
}

pub(crate) fn parse_json(bytes: &[u8]) -> Result<Value, ParserError> {
    let mut deserializer = serde_json::Deserializer::from_slice(bytes);
    let mut budget = ParseBudget::default();
    let parsed = StrictValueSeed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(&mut deserializer);
    match parsed {
        Ok(value) => {
            if deserializer.end().is_err() {
                return Err(ParserError::InvalidSyntax);
            }
            Ok(value.into_json())
        }
        Err(_) => Err(budget.failure.unwrap_or(ParserError::InvalidSyntax)),
    }
}

pub(crate) fn parse_yaml(text: &str) -> Result<Value, ParserError> {
    let mut documents = serde_yaml::Deserializer::from_str(text);
    let Some(document) = documents.next() else {
        return Err(ParserError::InvalidSyntax);
    };
    let mut budget = ParseBudget::default();
    let parsed = StrictValueSeed {
        budget: &mut budget,
        depth: 1,
    }
    .deserialize(document);
    let value = match parsed {
        Ok(value) => value,
        Err(_) => return Err(budget.failure.unwrap_or(ParserError::InvalidSyntax)),
    };
    if documents.next().is_some() {
        return Err(ParserError::MultipleDocuments);
    }
    Ok(value.into_json())
}

pub(crate) fn required_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
    max_bytes: usize,
) -> Result<&'a str, ParserError> {
    let value = object
        .get(field)
        .and_then(Value::as_str)
        .ok_or(ParserError::InvalidNode { node_index })?;
    if value.is_empty() || value.len() > max_bytes || has_control(value) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(value)
}

pub(crate) fn optional_string<'a>(
    object: &'a serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
    max_bytes: usize,
) -> Result<Option<&'a str>, ParserError> {
    match object.get(field) {
        None => Ok(None),
        Some(Value::String(value)) if value.len() <= max_bytes && !has_control(value) => {
            Ok(Some(value))
        }
        _ => Err(ParserError::InvalidNode { node_index }),
    }
}

pub(crate) fn required_host(
    object: &serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
) -> Result<(), ParserError> {
    let host = required_string(object, field, node_index, MAX_HOST_BYTES)?;
    if !is_host(host) {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

pub(crate) fn is_host(value: &str) -> bool {
    if value.len() > MAX_HOST_BYTES
        || value.contains("//")
        || value.bytes().any(|byte| {
            byte.is_ascii_control()
                || byte.is_ascii_whitespace()
                || matches!(byte, b'/' | b'?' | b'#' | b'@' | b'%')
        })
    {
        return false;
    }
    if value.parse::<IpAddr>().is_ok() {
        return true;
    }
    let host = value.strip_suffix('.').unwrap_or(value);
    !host.is_empty()
        && host.is_ascii()
        && host.split('.').all(|label| {
            !label.is_empty()
                && label.len() <= 63
                && label.as_bytes()[0].is_ascii_alphanumeric()
                && label.as_bytes()[label.len() - 1].is_ascii_alphanumeric()
                && label
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
        })
}

pub(crate) fn required_port(
    object: &serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    if object
        .get("port")
        .and_then(Value::as_u64)
        .is_some_and(|port| (1..=u16::MAX as u64).contains(&port))
    {
        Ok(())
    } else {
        Err(ParserError::InvalidNode { node_index })
    }
}

pub(crate) fn bool_value(
    object: &serde_json::Map<String, Value>,
    field: &str,
    node_index: usize,
) -> Result<bool, ParserError> {
    object
        .get(field)
        .and_then(Value::as_bool)
        .ok_or(ParserError::InvalidNode { node_index })
}

pub(crate) fn validate_uuid(
    object: &serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<(), ParserError> {
    let uuid = required_string(object, "uuid", node_index, 36)?;
    let bytes = uuid.as_bytes();
    if bytes.len() != 36
        || ![8, 13, 18, 23]
            .into_iter()
            .all(|index| bytes[index] == b'-')
        || !bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| [8, 13, 18, 23].contains(&index) || byte.is_ascii_hexdigit())
    {
        return Err(ParserError::InvalidNode { node_index });
    }
    Ok(())
}

pub(crate) fn has_control(value: &str) -> bool {
    value.chars().any(char::is_control)
}

pub(crate) fn has_disallowed_text(value: &str) -> bool {
    value
        .chars()
        .any(|ch| ch.is_control() || is_bidi_markup(ch))
}

fn is_bidi_markup(ch: char) -> bool {
    matches!(
        ch,
        '\u{200e}'
            | '\u{200f}'
            | '\u{202a}'
            | '\u{202b}'
            | '\u{202c}'
            | '\u{202d}'
            | '\u{202e}'
            | '\u{2066}'
            | '\u{2067}'
            | '\u{2068}'
            | '\u{2069}'
    )
}

const FIXTURE_OPT_PATH_MAX: usize = 256;

/// Synthetic `fixture-opts` schema from grok-review `I03.T04.b`: it exercises every
/// [`super::opts`] kind and rule; real option maps (`ws-opts`, TLS) declare their own spec.
const FIXTURE_OPTS: OptsSpec = OptsSpec {
    fields: &[
        required(
            "path",
            OptKind::Str {
                min: 1,
                max: FIXTURE_OPT_PATH_MAX,
            },
        ),
        required(
            "count",
            OptKind::Uint {
                min: 0,
                max: u64::MAX,
            },
        ),
        required("flag", OptKind::Bool),
        required(
            "items",
            OptKind::StrList {
                min_items: 0,
                max_items: 128,
                max_item: 32,
            },
        ),
        required("mode", OptKind::Enum(&["plain", "marked"])),
        optional(
            "left",
            OptKind::Str {
                min: 1,
                max: FIXTURE_OPT_PATH_MAX,
            },
        ),
        optional(
            "right",
            OptKind::Str {
                min: 1,
                max: FIXTURE_OPT_PATH_MAX,
            },
        ),
        optional("alpha", OptKind::Bool),
        optional("beta", OptKind::Bool),
        optional(
            "token",
            OptKind::Str {
                min: 1,
                max: FIXTURE_OPT_PATH_MAX,
            },
        ),
        optional(
            "token-name",
            OptKind::Str {
                min: 1,
                max: FIXTURE_OPT_PATH_MAX,
            },
        ),
    ],
    rules: &[
        OptRule::Exclusive("left", "right"),
        OptRule::Exclusive("alpha", "beta"),
        OptRule::Paired("token", "token-name"),
    ],
};

pub(crate) fn build_fixture_opts(
    object: serde_json::Map<String, Value>,
    node_index: usize,
) -> Result<Value, ParserError> {
    FIXTURE_OPTS.build(&object, node_index).map(Value::Object)
}

pub(crate) fn fixture_opts_definition_digest(value: &Value) -> String {
    use sha2::{Digest, Sha256};
    let bytes = serde_json::to_vec(value).expect("fixture opts fit in memory");
    Sha256::digest(bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect()
}

pub(crate) fn is_supported_ss_cipher(cipher: &str) -> bool {
    [
        "aes-128-gcm",
        "aes-192-gcm",
        "aes-256-gcm",
        "chacha20-ietf-poly1305",
        "xchacha20-ietf-poly1305",
    ]
    .contains(&cipher)
}

pub(crate) fn map_capability(
    error: crate::sources::capabilities::CapabilityError,
    node_index: usize,
) -> ParserError {
    use crate::sources::capabilities::CapabilityError;
    match error {
        CapabilityError::UnsupportedCoreVersion => ParserError::UnsupportedCoreVersion,
        CapabilityError::UnsupportedTransport => ParserError::UnsupportedTransport { node_index },
        CapabilityError::UnsupportedProtocol => ParserError::UnsupportedProtocol { node_index },
        _ => ParserError::UnsupportedNodeFeature { node_index },
    }
}
