//! I03.T04.u: manual own server. Fixtures: grok-review/fixtures/I03.T04.u.

use cm::profiles::{SourceKind, Store};
use cm::sources::manual::{
    ManualError, SecretKind, SecretSource, manual_node, manual_source_input, parse_add_server_args,
    read_secret, read_secret_file,
};
use cm::sources::{create_source, update_source};
use std::os::unix::fs::PermissionsExt;
use std::sync::atomic::{AtomicU64, Ordering};

const PIN: &str = "1.19.32";
const ARGV_MARKER: &str = "c0ffee00-0004-4000-8000-000000000004";
const STDIN_MARKER: &str = "c0ffee00-0005-4000-8000-000000000005";
const FILE_MARKER: &str = "c0ffee00-0006-4000-8000-000000000006";
static NEXT: AtomicU64 = AtomicU64::new(1);

fn argv(text: &str) -> Vec<String> {
    text.split_whitespace().map(str::to_owned).collect()
}

fn temp(label: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("cm-i03-manual-{label}-{}-{}", std::process::id(), NEXT.fetch_add(1, Ordering::Relaxed)))
}

#[test]
fn secret_in_argv_is_refused_before_anything_else() {
    // grok argv-secret.txt, without the leading "cm source add-server".
    let args = argv(&format!("--protocol vless --server edge.example.invalid --port 443 --uuid {ARGV_MARKER}"));
    let error = parse_add_server_args(&args).unwrap_err();
    assert!(matches!(error, ManualError::SecretInArgv));
    assert!(!format!("{error} {error:?}").contains(ARGV_MARKER));
    for inline in [
        format!("--password={ARGV_MARKER}"),
        format!("vless://{ARGV_MARKER}@edge.example.invalid:443#x"),
    ] {
        let args = vec!["--secret-stdin".to_owned(), inline];
        assert!(matches!(parse_add_server_args(&args).unwrap_err(), ManualError::SecretInArgv));
    }
    // This test process itself never carried a marker in its command line.
    let cmdline = std::fs::read("/proc/self/cmdline").unwrap();
    assert!(!String::from_utf8_lossy(&cmdline).contains("c0ffee00"));
}

#[test]
fn stdin_and_file_secrets_are_accepted() {
    let args = parse_add_server_args(&argv("--protocol vless --server edge.example.invalid --port 443 --secret-stdin")).unwrap();
    assert_eq!(args.secret_source, SecretSource::Stdin);
    let secret = read_secret(format!("{STDIN_MARKER}\n").as_bytes()).unwrap();
    let node = manual_node(&args, &secret).unwrap();
    let input = manual_source_input(PIN, node, 1_000).unwrap();
    assert!(!format!("{args:?}").contains(STDIN_MARKER));
    let _ = input;

    let path = temp("file");
    std::fs::write(&path, format!("{FILE_MARKER}\n")).unwrap();
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600)).unwrap();
    let args = parse_add_server_args(&[
        "--protocol".into(), "vless".into(), "--server".into(), "edge.example.invalid".into(),
        "--port".into(), "443".into(), "--secret-file".into(), path.display().to_string(),
    ])
    .unwrap();
    let SecretSource::File(file) = &args.secret_source else { panic!("file source") };
    let secret = read_secret_file(file).unwrap();
    assert_eq!(secret, FILE_MARKER);
    manual_source_input(PIN, manual_node(&args, &secret).unwrap(), 1_000).unwrap();
    // Decision 2026-10-06: a secret file accessible to group/others is refused, without echo.
    for mode in [0o644, 0o640, 0o604, 0o660] {
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(mode)).unwrap();
        let error = read_secret_file(&path).unwrap_err();
        assert!(matches!(error, ManualError::InsecureCredentialFile), "{mode:o}");
        assert!(!format!("{error} {error:?}").contains(FILE_MARKER));
    }
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o400)).unwrap();
    assert_eq!(read_secret_file(&path).unwrap(), FILE_MARKER);
    let _ = std::fs::remove_file(path);

    // Control characters and empty input are refused without echo.
    assert!(matches!(read_secret(&b"\n"[..]).unwrap_err(), ManualError::InvalidSecret));
    assert!(matches!(read_secret(&b"a\x07b"[..]).unwrap_err(), ManualError::InvalidSecret));
}

#[test]
fn share_uri_from_stdin_builds_the_same_validated_node() {
    let args = parse_add_server_args(&argv("--uri-stdin --name mine")).unwrap();
    assert_eq!(args.secret_kind, SecretKind::ShareUri);
    let uri = format!("vless://{STDIN_MARKER}@edge.example.invalid:443?security=none#ignored");
    let node = manual_node(&args, &uri).unwrap();
    assert_eq!(node["name"], "mine");
    manual_source_input(PIN, node, 1_000).unwrap();
}

/// Edits publish a new generation of the same Source. Node IDs are issued once per
/// generation (I02 model), so the edited node has a new ID and the old one is not current.
#[test]
fn edits_keep_the_source_and_advance_the_generation() {
    let root = temp("store");
    let store = Store::initialize(&root).unwrap();
    let build = |protocol: &str, server: &str, at: i64| {
        let args = parse_add_server_args(&argv(&format!(
            "--name mine --protocol {protocol} --server {server} --port 443 --secret-stdin"
        )))
        .unwrap();
        manual_source_input(PIN, manual_node(&args, STDIN_MARKER).unwrap(), at).unwrap()
    };
    let revision = store.read_snapshot().unwrap().revision;
    let first = create_source(&store, revision, build("vless", "edge.example.invalid", 1_000)).unwrap();
    let graph = store.read_snapshot().unwrap();
    assert_eq!(graph.sources[&first.source_id].kind, SourceKind::ManualServer);

    // edit-server.txt
    let second = update_source(&store, &first.source_id, first.graph_revision, first.source_generation,
        build("vless", "other.example.invalid", 2_000), 2_000).unwrap();
    assert_eq!(second.source_id, first.source_id);
    assert_eq!(second.source_generation, first.source_generation + 1);
    assert_ne!(second.node_ids, first.node_ids);

    // edit-protocol.txt
    let third = update_source(&store, &first.source_id, second.graph_revision, second.source_generation,
        build("vmess", "other.example.invalid", 3_000), 3_000).unwrap();
    let graph = store.read_snapshot().unwrap();
    let current = &graph.sources[&first.source_id].current_node_ids;
    assert_eq!(current, &third.node_ids);
    assert!(!current.contains(&second.node_ids[0]));
    let state = std::fs::read_to_string(root.join("state.json")).unwrap();
    assert!(!state.contains(STDIN_MARKER), "secret must stay in the private blob");
    let _ = std::fs::remove_dir_all(root);
}
