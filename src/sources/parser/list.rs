//! Body classification (`I03.T04.t`) and URI / base64 subscription lists (`I03.T04.s`).
//!
//! Lists follow I03-DECISIONS D3: every line is validated by the same node validator as
//! native sources; a line that cannot be imported is skipped and reported by class and line
//! number only, never by text. Zero importable lines refuse the whole source.

use super::native::{MAX_NATIVE_NODES, ParsedSource, ParserError, node_definition, parse_native};
use super::uri::{decode_b64_loose, parse_share_uri};
use crate::profiles::{
    ImportOmissions, MAX_SKIPPED_LINES_PER_CLASS, SkippedLineClass, SkippedLines, TlsVerification,
};
use crate::sources::artifact::MAX_RAW_SOURCE_BYTES;
use crate::sources::capabilities::{ImportFormat, PINNED_CORE_VERSION};
use std::collections::{BTreeMap, BTreeSet};

/// Two base64 layers are seen in the wild (a base64 list served base64-encoded again);
/// a third is refused rather than unwrapped without bound.
const MAX_BASE64_LAYERS: usize = 2;

/// What a response body looks like, before any node is parsed.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum BodyKind {
    Empty,
    Html,
    MihomoJson,
    MihomoYaml,
    UriList,
    Base64,
}

/// Classify a body by shape only. One leading UTF-8 BOM is ignored.
pub fn detect_body(bytes: &[u8]) -> BodyKind {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let trimmed = bytes.trim_ascii();
    if trimmed.is_empty() {
        return BodyKind::Empty;
    }
    let head = &trimmed[..trimmed.len().min(1024)];
    let head_lower = head.to_ascii_lowercase();
    if trimmed[0] == b'<'
        || contains(&head_lower, b"<!doctype html")
        || contains(&head_lower, b"<html")
    {
        return BodyKind::Html;
    }
    if matches!(trimmed[0], b'{' | b'[') {
        return BodyKind::MihomoJson;
    }
    let first_line = trimmed
        .split(|byte| *byte == b'\n')
        .next()
        .unwrap_or(trimmed);
    if looks_like_uri(first_line.trim_ascii()) {
        return BodyKind::UriList;
    }
    if trimmed.iter().all(|byte| {
        byte.is_ascii_alphanumeric()
            || matches!(byte, b'+' | b'/' | b'-' | b'_' | b'=' | b'\r' | b'\n')
    }) {
        return BodyKind::Base64;
    }
    BodyKind::MihomoYaml
}

fn contains(haystack: &[u8], needle: &[u8]) -> bool {
    haystack
        .windows(needle.len())
        .any(|window| window == needle)
}

fn looks_like_uri(line: &[u8]) -> bool {
    let Some(position) = line.windows(3).position(|window| window == b"://") else {
        return false;
    };
    position > 0
        && line[..position]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'-' | b'.'))
}

/// Parse any supported subscription body, detecting its format.
///
/// Empty bodies and HTML are refused (`InvalidSyntax` / `UnusableBody`); base64 is unwrapped
/// at most [`MAX_BASE64_LAYERS`] times. The digest of the parsed source is always the digest
/// of `bytes` as received.
pub fn parse_source(core_version: &str, bytes: &[u8]) -> Result<ParsedSource, ParserError> {
    check_core_and_size(core_version, bytes)?;
    parse_layer(core_version, bytes, bytes, 0)
}

fn parse_layer(
    core_version: &str,
    original: &[u8],
    layer: &[u8],
    base64_layers: usize,
) -> Result<ParsedSource, ParserError> {
    match detect_body(layer) {
        BodyKind::Empty => Err(ParserError::InvalidSyntax),
        BodyKind::Html => Err(ParserError::UnusableBody),
        BodyKind::MihomoJson | BodyKind::MihomoYaml if base64_layers > 0 => {
            // A native config is never served base64-encoded by the providers we support.
            Err(ParserError::UnusableBody)
        }
        BodyKind::MihomoJson => parse_native(core_version, ImportFormat::MihomoJson, original),
        BodyKind::MihomoYaml => parse_native(core_version, ImportFormat::MihomoYaml, original),
        BodyKind::UriList => {
            let format = if base64_layers == 0 {
                ImportFormat::UriList
            } else {
                ImportFormat::Base64UriList
            };
            parse_lines(core_version, format, original, layer)
        }
        BodyKind::Base64 => {
            if base64_layers >= MAX_BASE64_LAYERS {
                return Err(ParserError::UnsupportedEncoding);
            }
            let decoded = decode_body_base64(layer)?;
            parse_layer(core_version, original, &decoded, base64_layers + 1)
        }
    }
}

/// Parse a body declared as a URI list or a base64 URI list.
pub fn parse_uri_list(
    core_version: &str,
    format: ImportFormat,
    bytes: &[u8],
) -> Result<ParsedSource, ParserError> {
    check_core_and_size(core_version, bytes)?;
    match format {
        ImportFormat::UriList => parse_lines(core_version, format, bytes, bytes),
        ImportFormat::Base64UriList => {
            let decoded = decode_body_base64(bytes)?;
            match detect_body(&decoded) {
                BodyKind::UriList => parse_lines(core_version, format, bytes, &decoded),
                BodyKind::Html => Err(ParserError::UnusableBody),
                BodyKind::Empty => Err(ParserError::InvalidSyntax),
                _ => Err(ParserError::InvalidSyntax),
            }
        }
        _ => Err(ParserError::UnsupportedFormat),
    }
}

fn check_core_and_size(core_version: &str, bytes: &[u8]) -> Result<(), ParserError> {
    if core_version.strip_prefix('v').unwrap_or(core_version) != PINNED_CORE_VERSION {
        return Err(ParserError::UnsupportedCoreVersion);
    }
    if bytes.len() > MAX_RAW_SOURCE_BYTES {
        return Err(ParserError::BodyTooLarge);
    }
    Ok(())
}

/// Whole-body base64: line breaks between chunks allowed, either alphabet, padding optional.
fn decode_body_base64(bytes: &[u8]) -> Result<Vec<u8>, ParserError> {
    let bytes = bytes.strip_prefix(b"\xef\xbb\xbf").unwrap_or(bytes);
    let compact: String = std::str::from_utf8(bytes)
        .map_err(|_| ParserError::InvalidUtf8)?
        .chars()
        .filter(|ch| !ch.is_ascii_whitespace())
        .collect();
    let decoded = decode_b64_loose(&compact).ok_or(ParserError::InvalidSyntax)?;
    if decoded.len() > MAX_RAW_SOURCE_BYTES {
        return Err(ParserError::BodyTooLarge);
    }
    Ok(decoded)
}

fn parse_lines(
    core_version: &str,
    format: ImportFormat,
    original: &[u8],
    text: &[u8],
) -> Result<ParsedSource, ParserError> {
    let text = std::str::from_utf8(text).map_err(|_| ParserError::InvalidUtf8)?;
    let text = text.strip_prefix('\u{feff}').unwrap_or(text);
    let mut definitions = Vec::new();
    let mut names = BTreeSet::new();
    let mut skipped: BTreeMap<SkippedLineClass, Vec<u32>> = BTreeMap::new();
    let mut disabled = 0u32;
    for (index, line) in text.lines().enumerate() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let line_number = u32::try_from(index + 1).map_err(|_| ParserError::TooManyEntries)?;
        let outcome = parse_share_uri(line).and_then(|node| {
            node_definition(core_version, definitions.len(), node).and_then(|(name, parsed)| {
                if names.contains(&name) {
                    Err(ParserError::DuplicateNodeName {
                        node_index: definitions.len(),
                    })
                } else {
                    Ok((name, parsed))
                }
            })
        });
        match outcome {
            Ok((name, parsed)) => {
                if definitions.len() >= MAX_NATIVE_NODES {
                    return Err(ParserError::TooManyNodes);
                }
                if parsed.tls_verification() == TlsVerification::Disabled {
                    disabled = disabled.saturating_add(1);
                }
                names.insert(name);
                definitions.push(parsed);
            }
            Err(error) => {
                let lines = skipped.entry(skip_class(error)).or_default();
                if lines.len() >= MAX_SKIPPED_LINES_PER_CLASS {
                    return Err(ParserError::TooManyEntries);
                }
                lines.push(line_number);
            }
        }
    }
    if definitions.is_empty() {
        return Err(ParserError::EmptyProxies);
    }
    let omissions = ImportOmissions {
        section_names: Vec::new(),
        skipped_lines: skipped
            .into_iter()
            .map(|(class, line_numbers)| SkippedLines {
                class,
                line_numbers,
            })
            .collect(),
        tls_verification_disabled_count: disabled,
    };
    Ok(ParsedSource::from_parts(
        format,
        definitions,
        omissions,
        original,
    ))
}

/// D3 classes. Only the class and the line number leave this function.
fn skip_class(error: ParserError) -> SkippedLineClass {
    match error {
        ParserError::UnsupportedProtocol { .. } => SkippedLineClass::UnsupportedScheme,
        ParserError::UnsupportedNodeFeature { .. }
        | ParserError::UnsupportedNodeField { .. }
        | ParserError::UnsupportedTransport { .. }
        | ParserError::RestrictedNodeOption { .. } => SkippedLineClass::UnsupportedFeature,
        _ => SkippedLineClass::Malformed,
    }
}
