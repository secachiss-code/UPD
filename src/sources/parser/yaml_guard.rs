//! Streaming syntax guard over `saphyr_parser::Parser::next_event`.
//!
//! The value tree is still built by `parse_yaml` and `StrictValueSeed`, which
//! reject a duplicate key without copying the key into the error. This guard
//! only rejects documents the importer must not load.

use saphyr_parser::{Event, Parser};

const MAX_VALUES: usize = 1_000_000;
const MAX_DEPTH: usize = 64;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum GuardError {
    Malformed,
    UnsupportedYaml,
    TooDeep,
    TooManyValues,
}

struct Frame {
    mapping: bool,
    expecting_key: bool,
}

pub(super) fn validate(bytes: &[u8]) -> Result<(), GuardError> {
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
        let mapping_start = matches!(event, Event::MappingStart(_, _));
        let mapping_end = matches!(event, Event::MappingEnd);
        match event {
            Event::StreamStart | Event::Nothing => {}
            Event::StreamEnd => {
                return if documents == 1 && frames.is_empty() {
                    Ok(())
                } else {
                    Err(GuardError::Malformed)
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
            Event::Scalar(value, _style, anchor, tag) => {
                if anchor != 0 || tag.is_some() {
                    return Err(GuardError::UnsupportedYaml);
                }
                if frames
                    .last()
                    .is_some_and(|frame| frame.mapping && frame.expecting_key)
                    && value.as_ref() == "<<"
                {
                    return Err(GuardError::UnsupportedYaml);
                }
                begin_value(&mut frames, &mut values)?;
                if frames.len() + 1 > MAX_DEPTH {
                    return Err(GuardError::TooDeep);
                }
            }
            Event::SequenceStart(anchor, tag) | Event::MappingStart(anchor, tag) => {
                if frames
                    .last()
                    .is_some_and(|frame| frame.mapping && frame.expecting_key)
                {
                    return Err(GuardError::Malformed);
                }
                if anchor != 0 || tag.is_some() {
                    return Err(GuardError::UnsupportedYaml);
                }
                begin_value(&mut frames, &mut values)?;
                if frames.len() + 1 > MAX_DEPTH {
                    return Err(GuardError::TooDeep);
                }
                frames.push(Frame {
                    mapping: mapping_start,
                    expecting_key: true,
                });
            }
            Event::SequenceEnd | Event::MappingEnd => {
                let frame = frames.pop().ok_or(GuardError::Malformed)?;
                if frame.mapping != mapping_end || (frame.mapping && !frame.expecting_key) {
                    return Err(GuardError::Malformed);
                }
            }
        }
    }
}

fn has_yaml_directive(text: &str) -> bool {
    text.split('\n').any(|line| {
        let line = line.trim_end_matches('\r').trim_start();
        line.starts_with("%YAML") || line.starts_with("%TAG")
    })
}

fn begin_value(frames: &mut [Frame], values: &mut usize) -> Result<(), GuardError> {
    *values += 1;
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
