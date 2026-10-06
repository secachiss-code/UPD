//! V.02: `cm source` is read-only. URLs stay off argv; Store and legacy files stay put.

use cm::common::contract_fixtures::TempDirGuard;
use cm::profiles::Store;
use cm::sources::pipeline::create_accepted_source;
use cm::sources::{
    ConfiguredEndpoint, HttpResponse, ImportFormat, NegotiationPolicy, UserAgent,
    negotiate_with_clock, parse_native,
};
use std::collections::BTreeMap;
use std::fs;
use std::io::{Read, Write};
use std::net::TcpListener;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const URL_MARKER: &str = "url-marker-v02";
const BODY_MARKER: &str = "body-marker-v02";

fn fixture() -> (TempDirGuard, PathBuf) {
    let dir = TempDirGuard::new("cm-v02-source").unwrap();
    fs::write(dir.path().join("cm.conf"), "lang = ru\n").unwrap();
    fs::create_dir(dir.path().join("state")).unwrap();
    fs::create_dir(dir.path().join("vpn-etc")).unwrap();
    fs::create_dir(dir.path().join("vpn-home")).unwrap();
    fs::write(dir.path().join("state").join("sentinel"), b"state-sentinel").unwrap();
    let store = dir.path().join("store");
    (dir, store)
}

fn command(dir: &Path, args: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cm"));
    command
        .args(args)
        .env("CM_CONF", dir.join("cm.conf"))
        .env("CM_STATE_DIR", dir.join("state"))
        .env("CM_VPN_ETC", dir.join("vpn-etc"))
        .env("CM_VPN_HOME", dir.join("vpn-home"))
        .env("CM_PROFILE_STORE", dir.join("store"))
        .env_remove("UPD_CONF")
        .env_remove("UPD_STATE_DIR")
        .env_remove("UPD_VPN_ETC")
        .env_remove("UPD_VPN_HOME")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

fn output_of(dir: &Path, args: &[&str]) -> (i32, String, String) {
    let output = command(dir, args).output().unwrap();
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn tree(root: &Path) -> BTreeMap<String, (u128, Vec<u8>)> {
    let mut out = BTreeMap::new();
    if !root.exists() {
        return out;
    }
    fn walk(root: &Path, dir: &Path, out: &mut BTreeMap<String, (u128, Vec<u8>)>) {
        let meta = fs::symlink_metadata(dir).unwrap();
        let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
        let stamp = modified.duration_since(UNIX_EPOCH).unwrap().as_nanos();
        let rel = dir
            .strip_prefix(root)
            .unwrap_or(dir)
            .to_string_lossy()
            .into_owned();
        if meta.file_type().is_symlink() {
            out.insert(rel, (stamp, b"symlink".to_vec()));
            return;
        }
        if meta.is_dir() {
            out.insert(format!("{rel}/"), (stamp, Vec::new()));
            for entry in fs::read_dir(dir).unwrap() {
                walk(root, &entry.unwrap().path(), out);
            }
            return;
        }
        out.insert(rel, (stamp, fs::read(dir).unwrap()));
    }
    walk(root, root, &mut out);
    out
}

fn assert_clean(text: &str) {
    assert!(!text.contains(URL_MARKER), "{text}");
    assert!(!text.contains(BODY_MARKER), "{text}");
    assert!(!text.contains("://"), "{text}");
}

struct Server {
    url: String,
    hits: std::sync::Arc<std::sync::atomic::AtomicUsize>,
}

fn serve(body: &'static [u8]) -> Server {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = listener.local_addr().unwrap().port();
    let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counter = hits.clone();
    std::thread::spawn(move || {
        for _ in 0..8 {
            let Ok((mut socket, _)) = listener.accept() else {
                break;
            };
            let _ = socket.set_read_timeout(Some(Duration::from_secs(2)));
            let mut buf = [0u8; 2048];
            let _ = socket.read(&mut buf);
            counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            let header = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            let _ = socket.write_all(header.as_bytes());
            let _ = socket.write_all(body);
        }
    });
    Server {
        url: format!("http://user:{URL_MARKER}@127.0.0.1:{port}/sub"),
        hits,
    }
}

fn stdin_import(dir: &Path, url: &str) -> (i32, String, String, String) {
    let mut child = command(dir, &["source", "import", "--dry-run", "--url-stdin"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let cmdline = fs::read(format!("/proc/{}/cmdline", child.id())).unwrap_or_default();
    let cmdline = String::from_utf8_lossy(&cmdline).replace('\0', " ");
    let mut stdin = child.stdin.take().unwrap();
    writeln!(stdin, "{url}").unwrap();
    drop(stdin);
    let output = child.wait_with_output().unwrap();
    (
        output.status.code().unwrap_or(1),
        String::from_utf8_lossy(&output.stdout).into_owned(),
        String::from_utf8_lossy(&output.stderr).into_owned(),
        cmdline,
    )
}

#[test]
fn dry_run_prints_nodes_and_omissions_without_writing() {
    let body: &'static [u8] = br#"{"proxies":[
        {"name":"ss","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"body-marker-v02"},
        {"name":"vx","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","tls":true,"skip-cert-verify":true}
    ],"proxy-groups":[],"rules":[]}"#;
    let server = serve(body);
    let (dir, store) = fixture();
    fs::write(
        dir.path().join("vpn-etc").join("subs.json"),
        format!(r#"{{"active":"a","list":[{{"id":"a","name":"n","url":"https://{URL_MARKER}.example/sub"}}]}}"#),
    )
    .unwrap();
    Store::initialize(&store).unwrap();
    let before_store = tree(&store);
    let before_legacy = tree(&dir.path().join("vpn-etc"));
    let before_state = tree(&dir.path().join("state"));
    let (code, stdout, stderr, cmdline) = stdin_import(dir.path(), &server.url);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("формат: mihomo_json"), "{stdout}");
    assert!(stdout.contains("узлов: 2"), "{stdout}");
    assert!(stdout.contains("протокол shadowsocks: 1"), "{stdout}");
    assert!(stdout.contains("протокол vless: 1"), "{stdout}");
    assert!(stdout.contains("tls not_applicable: 1"), "{stdout}");
    assert!(stdout.contains("tls disabled: 1"), "{stdout}");
    assert!(stdout.contains("пропуск D1: proxy-groups"), "{stdout}");
    assert!(stdout.contains("пропуск D1: rules"), "{stdout}");
    assert!(
        stdout.contains("пропуск D2: tls_verification disabled 1"),
        "{stdout}"
    );
    assert!(stdout.contains("user-agent: mihomo/1.19.32"), "{stdout}");
    assert!(
        stdout.contains("запись: нет (Store и legacy не изменялись)"),
        "{stdout}"
    );
    assert!(stdout.contains("VPN не изменялся"), "{stdout}");
    assert_clean(&stdout);
    assert_clean(&stderr);
    assert!(!cmdline.contains(URL_MARKER), "{cmdline}");
    assert!(!cmdline.contains("http"), "{cmdline}");
    assert!(server.hits.load(std::sync::atomic::Ordering::Relaxed) >= 1);
    assert_eq!(tree(&store), before_store);
    assert_eq!(tree(&dir.path().join("vpn-etc")), before_legacy);
    assert_eq!(tree(&dir.path().join("state")), before_state);
}

#[test]
fn skipped_uri_lines_are_reported_by_class() {
    let body: &'static [u8] = b"trojan://body-marker-v02@edge.example.invalid:443#ok\nanytls://body-marker-v02@edge.example.invalid:443#no\n";
    let server = serve(body);
    let (dir, _) = fixture();
    let (code, stdout, stderr, _) = stdin_import(dir.path(), &server.url);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("формат: uri_list"), "{stdout}");
    assert!(stdout.contains("узлов: 1"), "{stdout}");
    assert!(stdout.contains("протокол trojan: 1"), "{stdout}");
    assert!(
        stdout.contains("пропуск D3: unsupported_scheme 2"),
        "{stdout}"
    );
    assert_clean(&stdout);
    assert_clean(&stderr);
}

#[test]
fn failures_and_argv_do_not_leak_or_connect() {
    let server = serve(b"<html>body-marker-v02</html>");
    let (dir, store) = fixture();
    Store::initialize(&store).unwrap();
    let before = tree(&store);

    let (code, stdout, stderr, _) = stdin_import(dir.path(), &server.url);
    assert_ne!(code, 0);
    assert!(stderr.contains("не удалось прочитать источник"), "{stderr}");
    assert_clean(&stdout);
    assert_clean(&stderr);

    let (code, stdout, stderr) = output_of(
        dir.path(),
        &[
            "source",
            "import",
            "--dry-run",
            &format!("http://127.0.0.1/{}", URL_MARKER),
        ],
    );
    assert_eq!(code, 2, "{stdout}{stderr}");
    assert!(stderr.contains("не через аргументы"), "{stderr}");
    assert_clean(&stdout);
    assert!(!stderr.contains(URL_MARKER), "{stderr}");

    let (code, stdout, stderr, _) =
        stdin_import(dir.path(), &format!("http://{URL_MARKER}.invalid/sub"));
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("127.0.0.1"), "{stderr}");
    assert_clean(&stdout);
    assert!(!stderr.contains(URL_MARKER), "{stderr}");

    let (code, stdout, stderr) = output_of(dir.path(), &["source", "import", "--url-stdin"]);
    assert_eq!(code, 2, "{stdout}{stderr}");
    assert!(stderr.contains("--dry-run"), "{stderr}");
    assert_eq!(tree(&store), before);
}

#[test]
fn url_file_mode_is_the_same_check_as_a_secret_file() {
    let (dir, _) = fixture();
    let loose = dir.path().join("url");
    fs::write(&loose, format!("http://127.0.0.1/{URL_MARKER}\n")).unwrap();
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o644)).unwrap();
    let (code, stdout, stderr) = output_of(
        dir.path(),
        &[
            "source",
            "import",
            "--dry-run",
            "--url-file",
            loose.to_str().unwrap(),
        ],
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("0600"), "{stderr}");
    assert_clean(&stdout);
    assert!(!stderr.contains(URL_MARKER), "{stderr}");

    let link = dir.path().join("url-link");
    fs::set_permissions(&loose, fs::Permissions::from_mode(0o600)).unwrap();
    std::os::unix::fs::symlink(&loose, &link).unwrap();
    let (code, stdout, stderr) = output_of(
        dir.path(),
        &[
            "source",
            "import",
            "--dry-run",
            "--url-file",
            link.to_str().unwrap(),
        ],
    );
    assert_ne!(code, 0, "{stdout}{stderr}");
    assert_clean(&stdout);
    assert!(!stderr.contains(URL_MARKER), "{stderr}");
}

#[test]
fn from_legacy_refuses_without_root_and_does_not_rewrite_subs() {
    let (dir, store) = fixture();
    let subs = dir.path().join("vpn-etc").join("subs.json");
    fs::write(
        &subs,
        format!(
            r#"{{"active":"a","list":[{{"id":"a","name":"https://{URL_MARKER}","url":"https://{URL_MARKER}.example/sub"}}]}}"#
        ),
    )
    .unwrap();
    Store::initialize(&store).unwrap();
    let before_legacy = tree(&dir.path().join("vpn-etc"));
    let before_store = tree(&store);
    let (code, stdout, stderr) = output_of(
        dir.path(),
        &["source", "import", "--dry-run", "--from-legacy", "1"],
    );
    assert_eq!(code, 1, "{stdout}{stderr}");
    assert!(stderr.contains("root"), "{stderr}");
    assert_clean(&stdout);
    assert!(!stderr.contains(URL_MARKER), "{stderr}");
    assert_eq!(tree(&dir.path().join("vpn-etc")), before_legacy);
    assert_eq!(tree(&store), before_store);
}

#[test]
fn list_show_and_doctor_read_the_store_without_changing_it() {
    let (dir, store) = fixture();
    let opened = Store::initialize(&store).unwrap();
    let bytes = format!(
        r#"{{"proxies":[{{"name":"one","type":"ss","server":"edge.example","port":443,"cipher":"aes-128-gcm","password":"{BODY_MARKER}"}}]}}"#
    )
    .into_bytes();
    let endpoint = ConfiguredEndpoint::new(
        format!("https://feed.example.invalid/?token={URL_MARKER}"),
        false,
    )
    .unwrap();
    let agents = [UserAgent::new("mihomo/1.19.32").unwrap()];
    let parsed = parse_native("1.19.32", ImportFormat::MihomoJson, &bytes).unwrap();
    let mut parsed = Some(parsed);
    let accepted = negotiate_with_clock(
        &endpoint,
        "1.19.32",
        &agents,
        &[],
        None,
        &NegotiationPolicy::default(),
        || Duration::ZERO,
        || Ok(1_700_000_000_000),
        |_| Ok(HttpResponse::new(200, bytes.clone())),
        |_| Ok(parsed.take().unwrap()),
    )
    .unwrap();
    let receipt = create_accepted_source(&opened, 0, accepted).unwrap();
    drop(opened);
    let before = tree(&store);
    let (code, stdout, stderr) = output_of(dir.path(), &["source", "list"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains(&receipt.source_id.to_string()), "{stdout}");
    assert!(stdout.contains("узлов 1"), "{stdout}");
    assert!(stdout.contains("mihomo_json"), "{stdout}");
    assert!(stdout.contains("VPN не изменялся"), "{stdout}");
    assert_clean(&stdout);
    assert_clean(&stderr);
    assert_eq!(tree(&store), before);

    let (code, stdout, stderr) =
        output_of(dir.path(), &["source", "show", receipt.source_id.as_str()]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("протокол shadowsocks: 1"), "{stdout}");
    assert!(stdout.contains("tls not_applicable: 1"), "{stdout}");
    assert!(stdout.contains("пропусков нет"), "{stdout}");
    assert!(stdout.contains("user-agent: только хеш"), "{stdout}");
    assert_clean(&stdout);
    assert_clean(&stderr);
    assert_eq!(tree(&store), before);

    let (code, stdout, stderr) = output_of(dir.path(), &["source", "doctor"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("VPN не изменялся"), "{stdout}");
    assert_clean(&stdout);
    assert_eq!(tree(&store), before);

    fs::write(dir.path().join("cm.conf"), "lang = en\n").unwrap();
    let (code, stdout, stderr) = output_of(dir.path(), &["source", "list"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("VPN was not changed"), "{stdout}");
    assert!(!stdout.contains("VPN не изменялся"), "{stdout}");
}

#[test]
fn missing_store_is_an_empty_list_and_unknown_commands_fail() {
    let (dir, store) = fixture();
    let (code, stdout, stderr) = output_of(dir.path(), &["source", "list"]);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("источников нет"), "{stdout}");
    assert!(!store.exists());
    let (code, stdout, stderr) = output_of(dir.path(), &["source", "nope"]);
    assert_eq!(code, 2, "{stdout}{stderr}");
    assert!(stderr.contains("неизвестная команда source"), "{stderr}");
    let (code, stdout, _) = output_of(dir.path(), &["source", "help"]);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("--url-stdin"), "{stdout}");
    assert!(stdout.contains("--from-legacy"), "{stdout}");
}
