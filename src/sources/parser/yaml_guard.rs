//! Streaming syntax/resource guard before serde_yaml constructs an event tree.
//! Uses the same locked libyaml implementation, without alias expansion or loading.

use std::mem::MaybeUninit;
use unsafe_libyaml as unsafe_libyaml_sys;

const MAX_VALUES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GuardError {
    Malformed,
    UnsupportedYaml,
    TooDeep,
    TooManyValues,
}

struct Parser(Box<unsafe_libyaml_sys::yaml_parser_t>);
impl Drop for Parser {
    fn drop(&mut self) {
        // SAFETY: initialized once, stable Box address and exactly one deletion.
        unsafe { unsafe_libyaml_sys::yaml_parser_delete(&mut *self.0) }
    }
}
struct Event(unsafe_libyaml_sys::yaml_event_t);
impl Drop for Event {
    fn drop(&mut self) {
        // SAFETY: only successfully parsed events become this unique owner.
        unsafe { unsafe_libyaml_sys::yaml_event_delete(&mut self.0) }
    }
}

struct Frame {
    mapping: bool,
    expecting_key: bool,
}

pub(super) fn validate(bytes: &[u8]) -> Result<(), GuardError> {
    if bytes.len() > crate::sources::artifact::MAX_RAW_SOURCE_BYTES
        || std::str::from_utf8(bytes).is_err()
    {
        return Err(GuardError::Malformed);
    }
    let mut allocation = Box::<unsafe_libyaml_sys::yaml_parser_t>::new_uninit();
    // SAFETY: writable, aligned allocation. Initialization fills the entire parser.
    if unsafe { unsafe_libyaml_sys::yaml_parser_initialize(allocation.as_mut_ptr()) }.fail {
        return Err(GuardError::Malformed);
    }
    // SAFETY: successful initialization above; never move the parser out of its Box.
    let mut parser = Parser(unsafe { allocation.assume_init() });
    // SAFETY: parser address remains stable; input lives until all events/parser drop.
    unsafe {
        unsafe_libyaml_sys::yaml_parser_set_encoding(
            &mut *parser.0,
            unsafe_libyaml_sys::YAML_UTF8_ENCODING,
        );
        unsafe_libyaml_sys::yaml_parser_set_input_string(
            &mut *parser.0,
            bytes.as_ptr(),
            bytes.len() as u64,
        );
    }
    let mut frames: Vec<Frame> = Vec::with_capacity(MAX_DEPTH);
    let mut values = 0usize;
    let mut documents = 0usize;
    loop {
        let mut raw = MaybeUninit::<unsafe_libyaml_sys::yaml_event_t>::uninit();
        // SAFETY: initialized parser, writable event allocation; parse-only API mode.
        if unsafe { unsafe_libyaml_sys::yaml_parser_parse(&mut *parser.0, raw.as_mut_ptr()) }.fail {
            return Err(GuardError::Malformed);
        }
        // SAFETY: parse success initializes the event, uniquely freed even on early return.
        let event = Event(unsafe { raw.assume_init() });
        match event.0.type_ {
            unsafe_libyaml_sys::YAML_STREAM_START_EVENT => {}
            unsafe_libyaml_sys::YAML_STREAM_END_EVENT => {
                return if documents == 1 && frames.is_empty() {
                    Ok(())
                } else {
                    Err(GuardError::Malformed)
                };
            }
            unsafe_libyaml_sys::YAML_DOCUMENT_START_EVENT => {
                documents += 1;
                // SAFETY: this union member corresponds to the checked event type.
                let start = unsafe { event.0.data.document_start };
                if documents != 1
                    || !start.version_directive.is_null()
                    || start.tag_directives.start != start.tag_directives.end
                {
                    return Err(GuardError::UnsupportedYaml);
                }
            }
            unsafe_libyaml_sys::YAML_DOCUMENT_END_EVENT => {
                if !frames.is_empty() {
                    return Err(GuardError::Malformed);
                }
            }
            unsafe_libyaml_sys::YAML_ALIAS_EVENT => return Err(GuardError::UnsupportedYaml),
            unsafe_libyaml_sys::YAML_SCALAR_EVENT => {
                // SAFETY: scalar event owns these valid pointers until Event drops.
                let scalar = unsafe { event.0.data.scalar };
                if !scalar.anchor.is_null() || !scalar.tag.is_null() {
                    return Err(GuardError::UnsupportedYaml);
                }
                if frames.last().is_some_and(|frame| frame.mapping && frame.expecting_key)
                    && scalar.length == 2
                    // SAFETY: a scalar of length 2 has two readable bytes.
                    && unsafe { std::slice::from_raw_parts(scalar.value, 2) } == b"<<"
                {
                    return Err(GuardError::UnsupportedYaml);
                }
                begin_value(&mut frames, &mut values)?;
                if frames.len() + 1 > MAX_DEPTH {
                    return Err(GuardError::TooDeep);
                }
            }
            unsafe_libyaml_sys::YAML_SEQUENCE_START_EVENT
            | unsafe_libyaml_sys::YAML_MAPPING_START_EVENT => {
                // Complex map keys are outside the string-keyed native schema.
                if frames
                    .last()
                    .is_some_and(|frame| frame.mapping && frame.expecting_key)
                {
                    return Err(GuardError::Malformed);
                }
                let mapping = event.0.type_ == unsafe_libyaml_sys::YAML_MAPPING_START_EVENT;
                // SAFETY: read only the union member selected by the event type.
                let has_annotation = unsafe {
                    if mapping {
                        let start = event.0.data.mapping_start;
                        !start.anchor.is_null() || !start.tag.is_null()
                    } else {
                        let start = event.0.data.sequence_start;
                        !start.anchor.is_null() || !start.tag.is_null()
                    }
                };
                if has_annotation {
                    return Err(GuardError::UnsupportedYaml);
                }
                begin_value(&mut frames, &mut values)?;
                if frames.len() + 1 > MAX_DEPTH {
                    return Err(GuardError::TooDeep);
                }
                frames.push(Frame {
                    mapping,
                    expecting_key: true,
                });
            }
            unsafe_libyaml_sys::YAML_SEQUENCE_END_EVENT
            | unsafe_libyaml_sys::YAML_MAPPING_END_EVENT => {
                let frame = frames.pop().ok_or(GuardError::Malformed)?;
                if frame.mapping != (event.0.type_ == unsafe_libyaml_sys::YAML_MAPPING_END_EVENT)
                    || (frame.mapping && !frame.expecting_key)
                {
                    return Err(GuardError::Malformed);
                }
            }
            _ => return Err(GuardError::Malformed),
        }
    }
}

fn begin_value(frames: &mut [Frame], values: &mut usize) -> Result<(), GuardError> {
    *values += 1; // bounded immediately, so usize cannot overflow.
    if *values > MAX_VALUES {
        return Err(GuardError::TooManyValues);
    }
    if let Some(frame) = frames.last_mut()
        && frame.mapping
    {
        frame.expecting_key = !frame.expecting_key;
    }
    Ok(())
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
