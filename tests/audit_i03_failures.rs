//! Source-failure fixtures (I03.T05.b). A refused refresh or import leaves the Store unchanged.
use cm::profiles::Store;
use cm::sources::artifact::ArtifactError;
use cm::sources::pipeline::{
    FetchSourceError, create_accepted_source, negotiate_source, update_accepted_source_with_omissions,
};
use cm::sources::{
    ConfiguredEndpoint, FetchFailure, HttpResponse, NegotiationError, NegotiationPolicy, UserAgent,
};
use serde_json::json;
use std::os::unix::fs::DirBuilderExt;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT: AtomicU64 = AtomicU64::new(1);

struct Root(PathBuf);
impl Root {
    fn new() -> Self {
        for _ in 0..16 {
            let path = std::env::temp_dir().join(format!(
                "cm-i03-failures-{}-{}",
                std::process::id(),
                NEXT.fetch_add(1, Ordering::Relaxed)
            ));
            match std::fs::DirBuilder::new().mode(0o700).create(&path) {
                Ok(()) => return Self(path),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => panic!("fixture directory error: {:?}", error.kind()),
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

fn proxies(nodes: &[(&str, &str)]) -> Vec<u8> {
    let list: Vec<_> = nodes
        .iter()
        .map(|(name, password)| {
            json!({
                "name": name,
                "type": "ss",
                "server": "edge.synthetic.invalid",
                "port": 443,
                "cipher": "aes-128-gcm",
                "password": password
            })
        })
        .collect();
    serde_json::to_vec(&json!({"proxies": list})).unwrap()
}

fn endpoint() -> ConfiguredEndpoint {
    ConfiguredEndpoint::new("https://feed.synthetic.invalid/sub", false).unwrap()
}

fn agents() -> [UserAgent; 1] {
    [UserAgent::new("fixture-ua").unwrap()]
}

enum Reply {
    Body(Vec<u8>),
    Header(Vec<u8>, String),
    Fail(FetchFailure),
}

fn negotiate(reply: Reply) -> Result<cm::sources::Negotiated<cm::sources::ParsedSource>, FetchSourceError> {
    let agents = agents();
    negotiate_source(
        &endpoint(),
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        move |_| match &reply {
            Reply::Fail(failure) => Err(match failure {
                FetchFailure::Timeout => FetchFailure::Timeout,
                FetchFailure::Transport => FetchFailure::Transport,
                FetchFailure::Tls => FetchFailure::Tls,
                FetchFailure::Redirect => FetchFailure::Redirect,
            }),
            Reply::Body(body) => Ok(HttpResponse::new(200, body.clone())),
            Reply::Header(body, header) => {
                Ok(HttpResponse::new(200, body.clone()).with_subscription_userinfo(header))
            }
        },
    )
}

fn publish(store: &Store, nodes: &[(&str, &str)]) -> cm::sources::SourceImportReceipt {
    let body = proxies(nodes);
    let accepted = negotiate(Reply::Body(body)).unwrap();
    create_accepted_source(store, store.read_snapshot().unwrap().revision, accepted).unwrap()
}

fn state_bytes(store_path: &std::path::Path) -> Vec<u8> {
    std::fs::read(store_path.join("state.json")).unwrap()
}

#[test]
fn html_stub_does_not_replace_working_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(&store, &[("one", "pw-html-1")]);
    let before = state_bytes(&store_path);
    let error = negotiate(Reply::Body(b"<html><body>provider stub</body></html>".to_vec())).unwrap_err();
    assert!(matches!(
        error,
        FetchSourceError::Negotiation(NegotiationError::NoUsableBody)
    ));
    let refresh = negotiate(Reply::Body(b"<html><body>provider stub</body></html>".to_vec()));
    assert!(refresh.is_err());
    let _ = refresh.and_then(|accepted| {
        update_accepted_source_with_omissions(
            &store,
            &receipt.source_id,
            receipt.graph_revision,
            receipt.source_generation,
            accepted,
            None,
            true,
            2_000,
        )
    });
    assert_eq!(state_bytes(&store_path), before);
    assert!(!format!("{error}").contains("provider stub"));
}

#[test]
fn exhausted_or_expired_subscription_userinfo_does_not_replace_working_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(&store, &[("one", "pw-quota-1")]);
    let before = state_bytes(&store_path);
    let body = proxies(&[("one", "pw-quota-2")]);
    for header in [
        "upload=424242; download=1; total=424242",
        "upload=0; download=0; total=10; expire=1",
    ] {
        let error = negotiate(Reply::Header(body.clone(), header.into())).unwrap_err();
        assert!(
            matches!(error, FetchSourceError::Negotiation(NegotiationError::QuotaRejected)),
            "{header}: {error}"
        );
        let shown = format!("{error}");
        assert!(!shown.contains("424242"), "{shown}");
        assert!(!shown.contains(header), "{shown}");
        assert_eq!(state_bytes(&store_path), before);
        assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].generation, 1);
    }
}

#[test]
fn healthy_subscription_userinfo_still_refreshes() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(&store, &[("one", "pw-ok-1")]);
    let body = proxies(&[("one", "pw-ok-2")]);
    let accepted = negotiate(Reply::Header(
        body,
        "upload=1; download=1; total=100; expire=4000000000".into(),
    ))
    .unwrap();
    let updated = update_accepted_source_with_omissions(
        &store,
        &receipt.source_id,
        receipt.graph_revision,
        receipt.source_generation,
        accepted,
        None,
        true,
        2_000,
    )
    .unwrap();
    assert!(matches!(updated, cm::sources::pipeline::ImportPublishOutcome::Published(receipt) if receipt.source_generation == 2));
}

#[test]
fn timeout_does_not_replace_working_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(&store, &[("one", "pw-timeout")]);
    let before = state_bytes(&store_path);
    let error = negotiate(Reply::Fail(FetchFailure::Timeout)).unwrap_err();
    assert!(matches!(
        error,
        FetchSourceError::Negotiation(NegotiationError::Fetch(FetchFailure::Timeout))
    ));
    assert_eq!(state_bytes(&store_path), before);
    assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].generation, 1);
}

#[test]
fn disappeared_nodes_do_not_replace_working_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(
        &store,
        &[
            ("n1", "pw-n1"),
            ("n2", "pw-n2"),
            ("n3", "pw-n3"),
            ("n4", "pw-n4"),
        ],
    );
    let before = state_bytes(&store_path);
    let shrunk = negotiate(Reply::Body(proxies(&[("n1", "pw-n1")]))).unwrap();
    let error = update_accepted_source_with_omissions(
        &store,
        &receipt.source_id,
        receipt.graph_revision,
        receipt.source_generation,
        shrunk,
        None,
        true,
        2_000,
    )
    .unwrap_err();
    assert!(matches!(
        error,
        FetchSourceError::Artifact(ArtifactError::AutoUpdateBlocked)
    ));
    let empty = negotiate(Reply::Body(br#"{"proxies":[]}"#.to_vec())).unwrap_err();
    assert!(matches!(
        empty,
        FetchSourceError::Negotiation(NegotiationError::NoUsableBody) | FetchSourceError::Parser(_)
    ));
    assert_eq!(state_bytes(&store_path), before);
    assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].current_node_ids.len(), 4);
}

#[test]
fn failed_refresh_of_active_source_leaves_store_unchanged() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let active = publish(&store, &[("active", "pw-active")]);
    let other = publish(&store, &[("other", "pw-other")]);
    let before = store.read_snapshot().unwrap();
    let before_bytes = state_bytes(&store_path);
    let error = negotiate(Reply::Fail(FetchFailure::Timeout)).unwrap_err();
    assert!(matches!(
        error,
        FetchSourceError::Negotiation(NegotiationError::Fetch(FetchFailure::Timeout))
    ));
    assert_eq!(state_bytes(&store_path), before_bytes);
    let after = store.read_snapshot().unwrap();
    assert_eq!(after.sources[&active.source_id], before.sources[&active.source_id]);
    assert_eq!(after.sources[&other.source_id], before.sources[&other.source_id]);
    assert_eq!(after, before);
}

#[test]
fn corrupt_import_does_not_replace_working_source() {
    let root = Root::new();
    let store_path = root.0.join("store");
    let store = Store::initialize(&store_path).unwrap();
    let receipt = publish(&store, &[("one", "pw-corrupt")]);
    let before = state_bytes(&store_path);
    let error = negotiate(Reply::Body(b"not-a-subscription {{{".to_vec())).unwrap_err();
    assert!(matches!(error, FetchSourceError::Negotiation(NegotiationError::NoUsableBody) | FetchSourceError::Parser(_)));
    assert!(!format!("{error}").contains("not-a-subscription"));
    assert_eq!(state_bytes(&store_path), before);
    assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].generation, 1);
    assert_eq!(
        std::fs::read_dir(store_path.join("credentials")).unwrap().count(),
        1
    );
}

/// Coordinator review 2026-10-06: an overlong or unparseable header is unknown, not "exhausted".
#[test]
fn odd_subscription_userinfo_does_not_block_refresh() {
    for header in [
        format!("upload=1; download=1; total=10; expire=4102444800; note={}", "x".repeat(600)),
        "garbage without equals".to_owned(),
        "total=abc; upload=zz".to_owned(),
    ] {
        let root = Root::new();
        let store_path = root.0.join("store");
        let store = Store::initialize(&store_path).unwrap();
        let receipt = publish(&store, &[("n1", "pw-n1")]);
        let accepted = negotiate(Reply::Header(proxies(&[("n1", "pw-n2")]), header)).unwrap();
        update_accepted_source_with_omissions(
            &store,
            &receipt.source_id,
            receipt.graph_revision,
            receipt.source_generation,
            accepted,
            None,
            false,
            2_000,
        )
        .expect("refresh with an odd header publishes");
        assert_eq!(store.read_snapshot().unwrap().sources[&receipt.source_id].generation, 2);
    }
}
