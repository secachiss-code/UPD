//! Declarative builder for nested option maps (`ws-opts`, `reality-opts`, TLS fields…).
//!
//! A [`OptsSpec`] lists every accepted key with its kind and bounds, plus cross-field rules.
//! Unknown keys, case-insensitive duplicates, wrong types, out-of-bound values and broken
//! rules are refused with fixed codes that never echo a key or a value. Accepted keys are
//! copied into a fresh map, so nothing unchecked reaches `full_definition`; the map is a
//! `BTreeMap`, so the digest does not depend on input key order.

use super::ParserError;
use super::common::has_disallowed_text;
use serde_json::{Map, Number, Value};
use std::collections::BTreeSet;

/// Kind and bounds of one option value.
#[derive(Clone, Copy, Debug)]
pub(crate) enum OptKind {
    /// UTF-8 text without control or bidi characters, length in bytes.
    Str {
        min: usize,
        max: usize,
    },
    /// Non-negative integer within `[min, max]`.
    Uint {
        min: u64,
        max: u64,
    },
    Bool,
    /// One value from a closed set.
    Enum(&'static [&'static str]),
    /// List of bounded strings; `min_items` may be 0.
    StrList {
        min_items: usize,
        max_items: usize,
        max_item: usize,
    },
    /// String map with bounded keys/values; keys compared case-insensitively for duplicates.
    #[allow(dead_code)] // first real use: `ws-opts.headers` in I03.T04.f
    StrMap {
        max_entries: usize,
        max_key: usize,
        max_value: usize,
    },
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OptField {
    pub key: &'static str,
    pub kind: OptKind,
    pub required: bool,
}

/// Cross-field rule checked after every field is valid on its own.
#[derive(Clone, Copy, Debug)]
pub(crate) enum OptRule {
    /// At most one of the two keys may be present.
    Exclusive(&'static str, &'static str),
    /// Either both keys are present or neither.
    Paired(&'static str, &'static str),
    /// The first key may appear only when the second is present.
    #[allow(dead_code)] // first real use: TLS/REALITY specs in I03.T04.c/.d
    Requires(&'static str, &'static str),
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct OptsSpec {
    pub fields: &'static [OptField],
    pub rules: &'static [OptRule],
}

pub(crate) const fn required(key: &'static str, kind: OptKind) -> OptField {
    OptField {
        key,
        kind,
        required: true,
    }
}

pub(crate) const fn optional(key: &'static str, kind: OptKind) -> OptField {
    OptField {
        key,
        kind,
        required: false,
    }
}

impl OptsSpec {
    /// Validate `object` against the spec for the node at `node_index`.
    pub(crate) fn build(
        &self,
        object: &Map<String, Value>,
        node_index: usize,
    ) -> Result<Map<String, Value>, ParserError> {
        let invalid = ParserError::InvalidNode { node_index };
        if has_case_insensitive_duplicate(object.keys()) {
            return Err(ParserError::DuplicateKey);
        }
        for key in object.keys() {
            if !self.fields.iter().any(|field| field.key == key) {
                return Err(ParserError::UnsupportedNodeField { node_index });
            }
        }
        let mut out = Map::new();
        for field in self.fields {
            match object.get(field.key) {
                None if field.required => return Err(invalid),
                None => {}
                Some(value) => {
                    let checked = check_value(field.kind, value).ok_or(invalid)?;
                    out.insert(field.key.to_owned(), checked);
                }
            }
        }
        for rule in self.rules {
            let has = |key: &str| out.contains_key(key);
            let broken = match *rule {
                OptRule::Exclusive(a, b) => has(a) && has(b),
                OptRule::Paired(a, b) => has(a) != has(b),
                OptRule::Requires(a, b) => has(a) && !has(b),
            };
            if broken {
                return Err(invalid);
            }
        }
        Ok(out)
    }
}

fn check_value(kind: OptKind, value: &Value) -> Option<Value> {
    match (kind, value) {
        (OptKind::Str { min, max }, Value::String(text)) => {
            bounded_text(text, min, max).then(|| value.clone())
        }
        (OptKind::Uint { min, max }, Value::Number(number)) => {
            let n = number.as_u64()?;
            (min..=max)
                .contains(&n)
                .then(|| Value::Number(Number::from(n)))
        }
        (OptKind::Bool, Value::Bool(_)) => Some(value.clone()),
        (OptKind::Enum(allowed), Value::String(text)) => {
            allowed.contains(&text.as_str()).then(|| value.clone())
        }
        (
            OptKind::StrList {
                min_items,
                max_items,
                max_item,
            },
            Value::Array(items),
        ) => {
            if items.len() < min_items || items.len() > max_items {
                return None;
            }
            let ok = items.iter().all(|item| {
                item.as_str()
                    .is_some_and(|text| bounded_text(text, 1, max_item))
            });
            ok.then(|| value.clone())
        }
        (
            OptKind::StrMap {
                max_entries,
                max_key,
                max_value,
            },
            Value::Object(map),
        ) => {
            if map.len() > max_entries || has_case_insensitive_duplicate(map.keys()) {
                return None;
            }
            let ok = map.iter().all(|(key, item)| {
                bounded_text(key, 1, max_key)
                    && item
                        .as_str()
                        .is_some_and(|text| bounded_text(text, 0, max_value))
            });
            ok.then(|| value.clone())
        }
        _ => None,
    }
}

fn bounded_text(text: &str, min: usize, max: usize) -> bool {
    (min..=max).contains(&text.len()) && !has_disallowed_text(text)
}

fn has_case_insensitive_duplicate<'a>(keys: impl Iterator<Item = &'a String>) -> bool {
    let mut seen = BTreeSet::new();
    keys.into_iter()
        .any(|key| !seen.insert(key.to_ascii_lowercase()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    const SPEC: OptsSpec = OptsSpec {
        fields: &[
            required("path", OptKind::Str { min: 1, max: 8 }),
            optional("n", OptKind::Uint { min: 1, max: 10 }),
            optional(
                "headers",
                OptKind::StrMap {
                    max_entries: 2,
                    max_key: 8,
                    max_value: 8,
                },
            ),
            optional("a", OptKind::Bool),
            optional("b", OptKind::Bool),
        ],
        rules: &[OptRule::Exclusive("a", "b"), OptRule::Requires("n", "path")],
    };

    fn build(value: Value) -> Result<Map<String, Value>, ParserError> {
        SPEC.build(value.as_object().unwrap(), 0)
    }

    #[test]
    fn accepts_only_declared_keys_within_bounds() {
        assert!(build(json!({"path": "/x", "n": 3, "headers": {"Host": "h"}})).is_ok());
        assert!(matches!(
            build(json!({"path": "/x", "zz": 1})),
            Err(ParserError::UnsupportedNodeField { .. })
        ));
        assert!(build(json!({"path": "/x", "n": 11})).is_err());
        assert!(build(json!({"path": "/x", "n": -1})).is_err());
        assert!(build(json!({"path": "/x", "n": 1.5})).is_err());
        assert!(build(json!({"path": ""})).is_err());
        assert!(build(json!({"path": "/x\r\n"})).is_err());
        assert!(build(json!({"path": "/x", "headers": {"Host": "a", "host": "b"}})).is_err());
        assert!(build(json!({"path": "/x", "a": true, "b": false})).is_err());
        assert!(build(json!({"n": 1})).is_err());
    }

    #[test]
    fn output_is_independent_of_input_key_order() {
        let one = build(json!({"path": "/x", "n": 2})).unwrap();
        let two = build(json!({"n": 2, "path": "/x"})).unwrap();
        assert_eq!(
            serde_json::to_vec(&one).unwrap(),
            serde_json::to_vec(&two).unwrap()
        );
    }
}
