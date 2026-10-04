//! Production parser-to-publication binding. Synthetic fixtures, NOT_RUN.
use cm::profiles::{Store, StoreError};
use cm::sources::pipeline::{create_accepted_source, update_accepted_source};
use cm::sources::{
    ArtifactError, ConfiguredEndpoint, HttpResponse, ImportFormat, Negotiated, NegotiationPolicy,
    ParsedSource, UserAgent, negotiate_with_clock, parse_native, read_source_artifact,
};
use serde_json::json;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::Duration;

static NEXT: AtomicU64 = AtomicU64::new(1);
struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "cm-i03-pipeline-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(e) => panic!("fixture directory error: {:?}", e.kind()),
            }
        }
        panic!("fixture collision limit");
    }
}
impl Drop for Root {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn body(name: &str, password: &str) -> Vec<u8> {
    serde_json::to_vec(&json!({"log-level":"warning","proxies":[{
        "name":name,"type":"ss","server":"edge.synthetic.invalid","port":443,
        "cipher":"aes-128-gcm","password":password
    }]}))
    .unwrap()
}

fn accepted(response: &[u8], parsed_from: &[u8], time: i64) -> Negotiated<ParsedSource> {
    let endpoint = ConfiguredEndpoint::new(
        "https://feed.synthetic.invalid/?token=private-endpoint",
        false,
    )
    .unwrap();
    let agents = [UserAgent::new("private-UA").unwrap()];
    // Deliberate mismatch is permitted by the generic injected negotiation API;
    // the public publication pipeline must refuse it before changing the Store.
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, parsed_from).unwrap();
    let mut parsed = Some(parsed);
    negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(time),
        |_| Ok(HttpResponse::new(200, response.to_vec())),
        |_| Ok(parsed.take().unwrap()),
    )
    .unwrap()
}

#[test]
fn parsed_payload_for_different_response_cannot_be_published() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let before = std::fs::read(store_path.join("state.json")).unwrap();
    let first = body("one", "private-one");
    let second = body("two", "private-two");
    let refused = create_accepted_source(&store, 0, accepted(&first, &second, 1000));
    assert!(matches!(refused, Err(ArtifactError::InvalidInput)));
    assert_eq!(
        std::fs::read(store_path.join("state.json")).unwrap(),
        before
    );
    assert_eq!(
        std::fs::read_dir(store_path.join("credentials"))
            .unwrap()
            .count(),
        0
    );
}

#[test]
fn independent_parsed_sources_and_failed_refresh_preserve_other_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let a1 = body("a1", "private-a1");
    let b = body("b", "private-b");
    let a2 = body("a2", "private-a2");
    let first = create_accepted_source(&store, 0, accepted(&a1, &a1, 1000)).unwrap();
    let second =
        create_accepted_source(&store, first.graph_revision, accepted(&b, &b, 1000)).unwrap();
    let before = store.read_snapshot().unwrap();
    let before_bytes = std::fs::read(store_path.join("state.json")).unwrap();
    let stale = update_accepted_source(
        &store,
        &first.source_id,
        first.graph_revision,
        1,
        accepted(&a2, &a2, 2000),
        2000,
    );
    assert!(matches!(
        stale,
        Err(ArtifactError::Store(StoreError::Conflict { .. }))
    ));
    assert_eq!(
        std::fs::read(store_path.join("state.json")).unwrap(),
        before_bytes
    );
    assert_eq!(
        std::fs::read_dir(store_path.join("credentials"))
            .unwrap()
            .count(),
        2
    );
    let updated = update_accepted_source(
        &store,
        &first.source_id,
        second.graph_revision,
        1,
        accepted(&a2, &a2, 2000),
        2000,
    )
    .unwrap();
    assert_eq!(updated.source_generation, 2);
    let after = store.read_snapshot().unwrap();
    assert_eq!(
        after.sources[&second.source_id],
        before.sources[&second.source_id]
    );
    assert_eq!(
        after.nodes[&second.node_ids[0]],
        before.nodes[&second.node_ids[0]]
    );
    let artifact = read_source_artifact(&store, &first.source_id).unwrap();
    assert_eq!(artifact.raw_body(), a2);
    assert_eq!(artifact.actual_user_agent(), Some("private-UA"));
    assert_eq!(
        artifact
            .definition(&updated.node_ids[0])
            .unwrap()
            .full_definition()["password"],
        "private-a2"
    );
    assert!(!serde_json::to_string(&after).unwrap().contains("private-"));
}

#[test]
fn native_negotiation_retries_unusable_body_but_stops_on_unsupported_semantics() {
    use cm::sources::pipeline::{FetchSourceError, negotiate_native_source};
    let endpoint =
        ConfiguredEndpoint::new("https://feed.synthetic.invalid/?token=private", false).unwrap();
    let agents = [
        UserAgent::new("HTML-UA").unwrap(),
        UserAgent::new("working-UA").unwrap(),
    ];
    let bytes = body("one", "private-one");
    let mut count = 0;
    let result = negotiate_native_source(
        &endpoint,
        "1.19.32",
        ImportFormat::MihomoJson,
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        |request| {
            count += 1;
            Ok(HttpResponse::new(
                200,
                if request.user_agent().expose_value() == "HTML-UA" {
                    b"<html>private-login</html>".to_vec()
                } else {
                    bytes.clone()
                },
            ))
        },
    )
    .unwrap();
    assert_eq!(count, 2);
    assert_eq!(result.actual_user_agent().expose_value(), "working-UA");
    assert_eq!(
        result.parsed().source_body_sha256(),
        result.source_body_sha256()
    );
    let mut count = 0;
    let mut unsupported: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
    unsupported["proxies"][0]["plugin"] = json!("private-plugin");
    let unsupported = serde_json::to_vec(&unsupported).unwrap();
    let failed = negotiate_native_source(
        &endpoint,
        "1.19.32",
        ImportFormat::MihomoJson,
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        |_| {
            count += 1;
            Ok(HttpResponse::new(200, unsupported.clone()))
        },
    );
    assert!(matches!(
        failed,
        Err(FetchSourceError::Parser(
            cm::sources::ParserError::UnsupportedNodeFeature { node_index: 0 }
        ))
    ));
    assert_eq!(count, 1);
    let failed = negotiate_native_source(
        &endpoint,
        "1.19.32",
        ImportFormat::UriList,
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        |_| {
            count += 1;
            panic!("format must be checked before fetch")
        },
    );
    assert!(matches!(
        failed,
        Err(FetchSourceError::Parser(
            cm::sources::ParserError::UnsupportedFormat
        ))
    ));
    assert_eq!(count, 1);
}
