//! I03.T04.s / .t: body classification, URI and base64 lists (D3), negotiate_source.
//! Fixtures: grok-review/fixtures/I03.T04.t (copied unchanged to tests/fixtures/i03_t04_t).

use cm::profiles::SkippedLineClass;
use cm::sources::pipeline::{FetchSourceError, negotiate_source};
use cm::sources::{
    BodyKind, ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, ParserError,
    UserAgent, detect_body, parse_source, parse_uri_list,
};
use std::cell::Cell;
use std::path::PathBuf;

const PIN: &str = "1.19.32";
const UUID: &str = "123e4567-e89b-12d3-a456-426614174000";

fn fixture(name: &str) -> Vec<u8> {
    std::fs::read(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/i03_t04_t").join(name)).unwrap()
}

/// Negotiate one body with two User-Agents; returns (result, number of fetches).
fn negotiate(body: Vec<u8>) -> (Result<cm::sources::Negotiated<cm::sources::ParsedSource>, FetchSourceError>, usize) {
    let endpoint = ConfiguredEndpoint::new("https://feed.example.invalid/sub?token=private", false).unwrap();
    let agents = [UserAgent::new("ua-one").unwrap(), UserAgent::new("ua-two").unwrap()];
    let calls = Cell::new(0usize);
    let result = negotiate_source(&endpoint, PIN, &agents, &[], None, &NegotiationPolicy::default(), |_| {
        calls.set(calls.get() + 1);
        Ok(HttpResponse::new(200, body.clone()))
    });
    (result, calls.get())
}

fn assert_clean(error: &FetchSourceError, markers: &[&str]) {
    let shown = format!("{error} {error:?}");
    for marker in markers {
        assert!(!shown.contains(marker), "{marker} leaked: {shown}");
    }
}

#[test]
fn grok_t_catalogue() {
    // HTML stub → retry with the next UA (two fetches), then exhausted.
    let (result, calls) = negotiate(fixture("html-stub.html"));
    assert_eq!(calls, 2);
    assert_clean(&result.unwrap_err(), &["CMFIXT-html-stub"]);
    // Empty → terminal InvalidSyntax, one fetch.
    let (result, calls) = negotiate(fixture("empty.txt"));
    assert_eq!(calls, 1);
    assert!(matches!(result.unwrap_err(), FetchSourceError::Parser(ParserError::InvalidSyntax)));
    // base64(HTML) → one decode, retry like HTML.
    let (_, calls) = negotiate(fixture("base64-html.txt"));
    assert_eq!(calls, 2);
    // Two base64 layers → one node `edge-b64`, no retry.
    let (result, calls) = negotiate(fixture("base64-of-base64.txt"));
    assert_eq!(calls, 1);
    let accepted = result.unwrap();
    assert_eq!(accepted.parsed().node_count(), 1);
    assert_eq!(accepted.parsed().format(), ImportFormat::Base64UriList);
    // Three layers → terminal, no retry.
    let (result, calls) = negotiate(fixture("base64-triple.txt"));
    assert_eq!(calls, 1);
    assert!(matches!(result.unwrap_err(), FetchSourceError::Parser(ParserError::UnsupportedEncoding)));
    // A URI list is imported as lines, not retried as broken YAML.
    let (result, calls) = negotiate(fixture("yaml-looking-uri-list.txt"));
    assert_eq!(calls, 1);
    assert_eq!(result.unwrap().parsed().node_count(), 2);
    // One leading BOM is stripped.
    let (result, calls) = negotiate(fixture("json-with-bom.json"));
    assert_eq!(calls, 1);
    let accepted = result.unwrap();
    assert_eq!(accepted.parsed().node_count(), 1);
    assert!(!format!("{:?}", accepted.parsed()).contains("CMFIXT-bom-pass"));
    // Oversized body → terminal BodyTooLarge.
    let over = vec![0x61u8; cm::sources::MAX_RAW_SOURCE_BYTES + 1];
    assert!(matches!(parse_source(PIN, &over).unwrap_err(), ParserError::BodyTooLarge));
    // Valid JSON with a host-control field → terminal, exactly one fetch.
    let (result, calls) = negotiate(fixture("unsupported-semantics.json"));
    assert_eq!(calls, 1);
    let error = result.unwrap_err();
    assert!(matches!(error, FetchSourceError::Parser(ParserError::RestrictedNodeOption { .. })));
    assert_clean(&error, &["CMFIXT-restricted", "eth0"]);
}

#[test]
fn detection_by_shape() {
    assert_eq!(detect_body(b"  \n"), BodyKind::Empty);
    assert_eq!(detect_body(b"<html><body>x</body></html>"), BodyKind::Html);
    assert_eq!(detect_body(b"proxies:\n  - {name: a}\n"), BodyKind::MihomoYaml);
    assert_eq!(detect_body(b"\xef\xbb\xbf{\"proxies\":[]}"), BodyKind::MihomoJson);
    assert_eq!(detect_body(b"trojan://p@h:443#n\n"), BodyKind::UriList);
    assert_eq!(detect_body(b"dHJvamFuOi8vcEBoOjQ0MyNu"), BodyKind::Base64);
}

/// D3: working lines import, the rest is reported by class and line number only.
#[test]
fn mixed_list_reports_skipped_lines_without_text() {
    let body = format!(
        "trojan://list-secret@edge.example.invalid:443#ok-one\n\
         \n\
         anytls://list-secret@edge.example.invalid:443#future\n\
         vless://{UUID}@edge.example.invalid:443?security=xtls#unsupported\n\
         vless://not a uri at all\n\
         ss://YWVzLTEyOC1nY206bGlzdC1zZWNyZXQ=@edge.example.invalid:8388#ok-two\n\
         trojan://list-secret@edge.example.invalid:443#ok-one\n"
    );
    let parsed = parse_uri_list(PIN, ImportFormat::UriList, body.as_bytes()).unwrap();
    assert_eq!(parsed.node_count(), 2);
    let omissions = parsed.omissions();
    let lines = |class| {
        omissions
            .skipped_lines
            .iter()
            .find(|report| report.class == class)
            .map(|report| report.line_numbers.clone())
            .unwrap_or_default()
    };
    assert_eq!(lines(SkippedLineClass::UnsupportedScheme), vec![3]);
    assert_eq!(lines(SkippedLineClass::UnsupportedFeature), vec![4]);
    // Unparseable line and the duplicate name are both malformed.
    assert_eq!(lines(SkippedLineClass::Malformed), vec![5, 7]);
    let shown = format!("{omissions:?} {parsed:?}");
    assert!(!shown.contains("list-secret") && !shown.contains("anytls") && !shown.contains("future"));
    assert!(omissions.requires_confirmation());
}

#[test]
fn list_without_importable_lines_is_refused() {
    let body = b"anytls://x@h:1#a\nnot-a-uri\n";
    assert!(matches!(parse_uri_list(PIN, ImportFormat::UriList, body).unwrap_err(), ParserError::EmptyProxies));
}

#[test]
fn base64_list_variants_decode_to_the_same_nodes() {
    // base64 of "trojan://p@edge.example.invalid:443#t\n" in std, url-safe and wrapped forms.
    let text = "trojan://p@edge.example.invalid:443#t\n";
    let std = b64(text.as_bytes(), false);
    let unpadded = std.trim_end_matches('=').to_owned();
    let wrapped = std.as_bytes().chunks(8).map(|c| std::str::from_utf8(c).unwrap()).collect::<Vec<_>>().join("\r\n");
    for body in [std.clone(), unpadded, wrapped, b64(text.as_bytes(), true)] {
        let parsed = parse_uri_list(PIN, ImportFormat::Base64UriList, body.as_bytes()).unwrap();
        assert_eq!(parsed.node_count(), 1);
    }
    assert!(parse_uri_list(PIN, ImportFormat::Base64UriList, b"@@not base64@@").is_err());
}

/// URI nodes go through the native validator: a REALITY link without fingerprint is skipped.
#[test]
fn uri_nodes_are_validated_like_native_nodes() {
    let pbk = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
    let body = format!(
        "vless://{UUID}@edge.example.invalid:443?security=reality&pbk={pbk}&sni=www.example.com&fp=chrome#good\n\
         vless://{UUID}@edge.example.invalid:443?security=reality&pbk={pbk}&sni=www.example.com#no-fp\n"
    );
    let parsed = parse_uri_list(PIN, ImportFormat::UriList, body.as_bytes()).unwrap();
    assert_eq!(parsed.node_count(), 1);
    assert_eq!(parsed.omissions().skipped_lines[0].line_numbers, vec![2]);
}

fn b64(bytes: &[u8], url_safe: bool) -> String {
    let table: &[u8; 64] = if url_safe {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789-_"
    } else {
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
    };
    let mut out = String::new();
    for chunk in bytes.chunks(3) {
        let n = chunk.iter().fold(0u32, |acc, b| (acc << 8) | u32::from(*b)) << (8 * (3 - chunk.len()));
        for i in 0..=chunk.len() {
            out.push(table[((n >> (18 - 6 * i)) & 63) as usize] as char);
        }
        if !url_safe {
            out.push_str(&"=".repeat(3 - chunk.len()));
        }
    }
    out
}
