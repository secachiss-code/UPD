//! Single-pass YAML loader over `saphyr_parser::Parser::next_event`.
//!
//! The same event stream that is checked is the one that becomes the value tree: there is no
//! second YAML parser that could read the text differently. Anchors, aliases, tags, merge
//! keys, directives, multiple documents and complex keys are refused; depth, value count and
//! duplicate keys are enforced while loading. Scalars follow the YAML 1.2 core schema; a
//! plain scalar that is not a JSON-representable number, bool or null stays a string.

use saphyr_parser::{Event, Parser, ScalarStyle};
use serde_json::{Map, Number, Value};

const MAX_VALUES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GuardError {
    Malformed,
    UnsupportedYaml,
    TooDeep,
    TooManyValues,
    DuplicateKey,
}

enum Frame {
    Sequence(Vec<Value>),
    /// Map under construction and the key waiting for its value.
    Mapping(Map<String, Value>, Option<String>),
}

impl Frame {
    fn expecting_key(&self) -> bool {
        matches!(self, Frame::Mapping(_, None))
    }
}

/// Check and load one YAML document into a JSON value.
pub(super) fn load(bytes: &[u8]) -> Result<Value, GuardError> {
    if bytes.len() > crate::sources::artifact::MAX_RAW_SOURCE_BYTES {
        return Err(GuardError::Malformed);
    }
    let Ok(text) = std::str::from_utf8(bytes) else {
        return Err(GuardError::Malformed);
    };
    if has_yaml_directive(text) {
        return Err(GuardError::UnsupportedYaml);
    }
    let mut parser = Parser::new_from_str(text);
    let mut frames: Vec<Frame> = Vec::with_capacity(MAX_DEPTH);
    let mut values = 0usize;
    let mut documents = 0usize;
    let mut root: Option<Value> = None;
    loop {
        let event = match parser.next_event() {
            Some(Ok((event, _span))) => event,
            Some(Err(error)) => {
                let info = error.info();
                if info.contains("unknown anchor") || info.contains("alias") {
                    return Err(GuardError::UnsupportedYaml);
                }
                return Err(GuardError::Malformed);
            }
            None => return Err(GuardError::Malformed),
        };
        match event {
            Event::StreamStart | Event::Nothing => {}
            Event::StreamEnd => {
                return match (documents, frames.is_empty(), root) {
                    (1, true, Some(value)) => Ok(value),
                    _ => Err(GuardError::Malformed),
                };
            }
            Event::DocumentStart(_) => {
                documents += 1;
                if documents != 1 {
                    return Err(GuardError::UnsupportedYaml);
                }
            }
            Event::DocumentEnd => {
                if !frames.is_empty() {
                    return Err(GuardError::Malformed);
                }
            }
            Event::Alias(_) => return Err(GuardError::UnsupportedYaml),
            Event::Scalar(text, style, anchor, tag) => {
                if anchor != 0 || tag.is_some() {
                    return Err(GuardError::UnsupportedYaml);
                }
                count(&mut values, frames.len())?;
                if let Some(frame @ Frame::Mapping(_, None)) = frames.last_mut() {
                    // A key: the raw text, never resolved to a number or bool.
                    if text.as_ref() == "<<" && style == ScalarStyle::Plain {
                        return Err(GuardError::UnsupportedYaml);
                    }
                    let Frame::Mapping(map, pending) = frame else {
                        unreachable!()
                    };
                    if map.contains_key(text.as_ref()) {
                        return Err(GuardError::DuplicateKey);
                    }
                    *pending = Some(text.into_owned());
                    continue;
                }
                let value = resolve_scalar(&text, style)?;
                attach(&mut frames, &mut root, value)?;
            }
            event @ (Event::SequenceStart(..) | Event::MappingStart(..)) => {
                let is_mapping = matches!(event, Event::MappingStart(..));
                let (Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag)) = event
                else {
                    unreachable!()
                };
                if frames.last().is_some_and(Frame::expecting_key) {
                    // Complex keys have no JSON representation.
                    return Err(GuardError::Malformed);
                }
                if anchor != 0 || tag.is_some() {
                    return Err(GuardError::UnsupportedYaml);
                }
                count(&mut values, frames.len())?;
                if frames.len() + 1 > MAX_DEPTH {
                    return Err(GuardError::TooDeep);
                }
                frames.push(if is_mapping {
                    Frame::Mapping(Map::new(), None)
                } else {
                    Frame::Sequence(Vec::new())
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let value = match (frames.pop(), matches!(event, Event::MappingEnd)) {
                    (Some(Frame::Sequence(items)), false) => Value::Array(items),
                    (Some(Frame::Mapping(map, None)), true) => Value::Object(map),
                    _ => return Err(GuardError::Malformed),
                };
                attach(&mut frames, &mut root, value)?;
            }
        }
    }
}

/// Check only (fixtures and callers that do not need the value).
#[cfg(test)]
pub(super) fn validate(bytes: &[u8]) -> Result<(), GuardError> {
    load(bytes).map(|_| ())
}

fn count(values: &mut usize, depth: usize) -> Result<(), GuardError> {
    *values += 1;
    if *values > MAX_VALUES {
        return Err(GuardError::TooManyValues);
    }
    if depth + 1 > MAX_DEPTH {
        return Err(GuardError::TooDeep);
    }
    Ok(())
}

fn attach(frames: &mut [Frame], root: &mut Option<Value>, value: Value) -> Result<(), GuardError> {
    match frames.last_mut() {
        None => {
            if root.replace(value).is_some() {
                return Err(GuardError::Malformed);
            }
        }
        Some(Frame::Sequence(items)) => items.push(value),
        Some(Frame::Mapping(map, pending)) => {
            let key = pending.take().ok_or(GuardError::Malformed)?;
            map.insert(key, value);
        }
    }
    Ok(())
}

/// YAML 1.2 core schema for plain scalars; quoted and block scalars are always strings.
fn resolve_scalar(text: &str, style: ScalarStyle) -> Result<Value, GuardError> {
    if style != ScalarStyle::Plain {
        return Ok(Value::String(text.to_owned()));
    }
    match text {
        "" | "~" | "null" | "Null" | "NULL" => return Ok(Value::Null),
        "true" | "True" | "TRUE" => return Ok(Value::Bool(true)),
        "false" | "False" | "FALSE" => return Ok(Value::Bool(false)),
        ".inf" | ".Inf" | ".INF" | "+.inf" | "+.Inf" | "+.INF" | "-.inf" | "-.Inf" | "-.INF"
        | ".nan" | ".NaN" | ".NAN" => {
            // Not representable in JSON; never silently turned into a string either.
            return Err(GuardError::UnsupportedYaml);
        }
        _ => {}
    }
    if let Some(number) = resolve_int(text) {
        return Ok(Value::Number(number));
    }
    if is_core_float(text)
        && let Some(number) = text.parse::<f64>().ok().and_then(Number::from_f64)
    {
        return Ok(Value::Number(number));
    }
    Ok(Value::String(text.to_owned()))
}

fn resolve_int(text: &str) -> Option<Number> {
    let (negative, digits) = match text.as_bytes().first()? {
        b'-' => (true, &text[1..]),
        b'+' => (false, &text[1..]),
        _ => (false, text),
    };
    let magnitude = if let Some(hex) = digits.strip_prefix("0x") {
        u64::from_str_radix(hex, 16).ok()?
    } else if let Some(octal) = digits.strip_prefix("0o") {
        u64::from_str_radix(octal, 8).ok()?
    } else {
        // A leading zero is not read as a number (no implicit octal, no lost zeros).
        if digits.is_empty()
            || !digits.bytes().all(|b| b.is_ascii_digit())
            || (digits.len() > 1 && digits.starts_with('0'))
        {
            return None;
        }
        digits.parse::<u64>().ok()?
    };
    if negative {
        let value = i64::try_from(magnitude).ok()?.checked_neg()?;
        Some(Number::from(value))
    } else {
        Some(Number::from(magnitude))
    }
}

/// `[-+]?(\.[0-9]+|[0-9]+(\.[0-9]*)?)([eE][-+]?[0-9]+)?` from the core schema.
fn is_core_float(text: &str) -> bool {
    let body = text.strip_prefix(['-', '+']).unwrap_or(text);
    let (mantissa, exponent) = match body.find(['e', 'E']) {
        Some(index) => (&body[..index], Some(&body[index + 1..])),
        None => (body, None),
    };
    let (int_part, frac_part) = match mantissa.split_once('.') {
        Some((int_part, frac_part)) => (int_part, Some(frac_part)),
        None => (mantissa, None),
    };
    let digits = |s: &str| s.bytes().all(|b| b.is_ascii_digit());
    let mantissa_ok = match frac_part {
        None => !int_part.is_empty() && digits(int_part),
        Some(frac) => digits(int_part) && digits(frac) && !(int_part.is_empty() && frac.is_empty()),
    };
    let exponent_ok = exponent.is_none_or(|exp| {
        let exp = exp.strip_prefix(['-', '+']).unwrap_or(exp);
        !exp.is_empty() && digits(exp)
    });
    // Must contain a dot or an exponent; plain integers were handled before.
    mantissa_ok && exponent_ok && (frac_part.is_some() || exponent.is_some())
}

fn has_yaml_directive(text: &str) -> bool {
    // Directives start at column 0; an indented `%YAML` inside a block scalar is content.
    text.split('\n')
        .any(|line| line.starts_with("%YAML") || line.starts_with("%TAG"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn syntax_annotations_refused_but_secret_text_is_data() {
        for bytes in [
            b"value: &secret plaintext".as_slice(),
            b"value: *secret".as_slice(),
            b"value: !private plaintext".as_slice(),
            b"value: !!str plaintext".as_slice(),
            b"value: {<<: {password: private}}".as_slice(),
            b"%YAML 1.1\n---\nvalue: private".as_slice(),
            b"---\nvalue: one\n---\nvalue: two".as_slice(),
        ] {
            assert_eq!(validate(bytes), Err(GuardError::UnsupportedYaml));
        }
        for bytes in [
            b"value: 'a &anchor *alias !tag <<'".as_slice(),
            b"value: \"a &anchor *alias !tag <<\"".as_slice(),
            b"value: ordinary &anchor *alias !tag <<".as_slice(),
            b"value: |\n  &anchor *alias !tag\n  ---\n  <<: text".as_slice(),
            b"value: 'it''s &anchor *alias !tag' # ignored syntax".as_slice(),
            b"value: <<".as_slice(),
        ] {
            assert_eq!(validate(bytes), Ok(()));
        }
    }

    #[test]
    fn streaming_depth_limit_before_tree_loading() {
        let valid = format!("{}'secret'{}", "[".repeat(63), "]".repeat(63));
        assert_eq!(validate(valid.as_bytes()), Ok(()));
        let invalid = format!("{}'secret'{}", "[".repeat(64), "]".repeat(64));
        assert_eq!(validate(invalid.as_bytes()), Err(GuardError::TooDeep));
    }

    #[test]
    fn streaming_entry_limit_before_tree_loading() {
        let bytes = format!("[{}null]", "null,".repeat(MAX_VALUES));
        assert_eq!(validate(bytes.as_bytes()), Err(GuardError::TooManyValues));
    }

    #[test]
    fn values_follow_the_core_schema_and_duplicates_are_refused() {
        let value = load(b"a: 443\nb: true\nc: ~\nd: '443'\ne: 0123\nf: 1.5\ng: -7\nh: 0x1F\ni: yes\nj: |\n  text\n").unwrap();
        assert_eq!(value["a"], 443);
        assert_eq!(value["b"], true);
        assert!(value["c"].is_null());
        assert_eq!(value["d"], "443");
        assert_eq!(value["e"], "0123");
        assert_eq!(value["f"], 1.5);
        assert_eq!(value["g"], -7);
        assert_eq!(value["h"], 31);
        assert_eq!(value["i"], "yes", "YAML 1.2: yes is a string");
        assert_eq!(value["j"], "text\n");
        assert_eq!(load(b"k: 1\nk: 2\n"), Err(GuardError::DuplicateKey));
        assert_eq!(load(b"x: .nan\n"), Err(GuardError::UnsupportedYaml));
        let nested = load(b"proxies:\n  - {name: n, port: 1}\n  - name: m\n    port: 2\n").unwrap();
        assert_eq!(nested["proxies"][1]["port"], 2);
    }

    #[test]
    fn malformed_and_complex_keys_fail_without_parser_text() {
        for bytes in [
            b"[unfinished".as_slice(),
            b"{[private]: value}".as_slice(),
            b"".as_slice(),
            &[0xff],
        ] {
            assert_eq!(validate(bytes), Err(GuardError::Malformed));
        }
    }
}
