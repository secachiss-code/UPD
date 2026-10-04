//! Closed restart settings. Synthetic fixtures, NOT_RUN until installation.
use cm::profiles::NodeProtocol;
use cm::profiles::{SourceFormat, Store};
use cm::sources::negotiation::PrivateFetchSettings;
use cm::sources::{
    ConfiguredEndpoint, GlobalDefaults, HttpResponse, ImportFormat, NegotiationPolicy,
    NodeDefinitionInput, SourceImportInput, Transport, UserAgent, create_source,
    negotiate_with_clock, read_node_definition, read_source_artifact, update_source,
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
                "cm-i03-settings-{}-{}",
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

fn input(endpoint: &str, time: i64) -> SourceImportInput {
    let endpoint = ConfiguredEndpoint::new(endpoint, false).unwrap();
    let agents = [UserAgent::new("private-actual-UA").unwrap()];
    let negotiated = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(time),
        |_| Ok(HttpResponse::new(200, b"trusted-raw-body".to_vec())),
        |_| Ok(()),
    )
    .unwrap();
    SourceImportInput::from_negotiated(
        ImportFormat::MihomoJson,
        &negotiated,
        vec![
            NodeDefinitionInput::from_trusted_classifier(
                "1.19.32",
                NodeProtocol::Vless,
                Transport::Tcp,
                json!({"name":"trusted-marker"}),
            )
            .unwrap(),
        ],
        GlobalDefaults::default(),
    )
    .unwrap()
}

#[test]
fn endpoint_only_change_is_private_and_keeps_generation_and_old_node_artifact() {
    let root = Root::new();
    let path = root.0.join("store");
    let store = Store::initialize(&path).unwrap();
    let first = create_source(
        &store,
        0,
        input("https://one.synthetic.invalid/feed?token=secret-one", 1000),
    )
    .unwrap();
    let before = store.read_snapshot().unwrap();
    let changed = update_source(
        &store,
        &first.source_id,
        first.graph_revision,
        1,
        input("https://two.synthetic.invalid/feed?token=secret-two", 2000),
        2000,
    )
    .unwrap();
    assert_eq!(changed.source_generation, 1);
    assert_eq!(changed.node_ids, first.node_ids);
    assert!(!changed.artifact_reused);
    let after = store.read_snapshot().unwrap();
    assert_eq!(before.nodes, after.nodes);
    assert_ne!(
        before.sources[&first.source_id].credential,
        after.sources[&first.source_id].credential
    );
    let text = serde_json::to_string(&after).unwrap();
    for secret in [
        "secret-one",
        "secret-two",
        "private-actual-UA",
        "synthetic.invalid",
    ] {
        assert!(!text.contains(secret));
    }
    drop(store);
    let reopened = Store::open(&path).unwrap();
    let artifact = read_source_artifact(&reopened, &first.source_id).unwrap();
    let settings = artifact.fetch_settings().unwrap();
    assert_eq!(
        settings.endpoint().unwrap().expose_url(),
        "https://two.synthetic.invalid/feed?token=secret-two"
    );
    assert!(!format!("{settings:?}").contains("secret-two"));
    let agents = settings.candidates().unwrap();
    let winner = UserAgent::new(artifact.actual_user_agent().unwrap()).unwrap();
    let cache = settings.cached_winner(&winner, 2000).unwrap();
    let accepted = negotiate_with_clock(
        &settings.endpoint().unwrap(),
        "1.19.32",
        &agents,
        &[],
        Some(&cache),
        &settings.policy(),
        || Duration::ZERO,
        || Ok(2001),
        |request| {
            assert_eq!(
                request.endpoint().expose_url(),
                "https://two.synthetic.invalid/feed?token=secret-two"
            );
            assert_eq!(request.user_agent().expose_value(), "private-actual-UA");
            Ok(HttpResponse::new(200, b"trusted-raw-body".to_vec()))
        },
        |_| Ok(()),
    )
    .unwrap();
    assert!(accepted.used_cached_winner());
    let old = read_node_definition(&reopened, &first.node_ids[0]).unwrap();
    assert_eq!(old.source_generation(), 1);
    assert_eq!(old.format(), SourceFormat::MihomoJson);
}

#[test]
fn settings_are_captured_before_cached_reordering_and_reject_invalid_private_records() {
    let endpoint =
        ConfiguredEndpoint::new("https://feed.synthetic.invalid/?token=private", false).unwrap();
    let agents = [
        UserAgent::new("first").unwrap(),
        UserAgent::new("second").unwrap(),
    ];
    let mut count = 0;
    let first = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(1000),
        |_| {
            count += 1;
            Ok(HttpResponse::new(200, vec![count]))
        },
        |bytes| {
            if bytes == [1] {
                Err(cm::sources::BodyRejection::RetryableUnusableBody)
            } else {
                Ok(())
            }
        },
    )
    .unwrap();
    let cache = first.make_cache_record();
    let next = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        Some(&cache),
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(1001),
        |_| Ok(HttpResponse::new(200, vec![2])),
        |_| Ok(()),
    )
    .unwrap();
    assert!(next.used_cached_winner());
    assert_eq!(next.fetch_settings().candidates().unwrap(), agents);
    let serialized = serde_json::to_value(next.fetch_settings()).unwrap();
    let roundtrip: PrivateFetchSettings = serde_json::from_value(serialized.clone()).unwrap();
    assert_eq!(roundtrip, *next.fetch_settings());
    let foreign = UserAgent::new("not-in-configured-set").unwrap();
    assert!(roundtrip.cached_winner(&foreign, 1001).is_err());
    assert!(roundtrip.cached_winner(&agents[1], -1).is_err());
    let mut corrupt = serialized;
    corrupt["candidates"] = json!(["second", "second"]);
    let invalid: PrivateFetchSettings = serde_json::from_value(corrupt.clone()).unwrap();
    assert!(invalid.cached_winner(&agents[1], 1001).is_err());
    corrupt["arbitrary-secret-field"] = json!("private");
    assert!(serde_json::from_value::<PrivateFetchSettings>(corrupt).is_err());
}
