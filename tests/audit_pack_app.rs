//! M01, M02, M04–M08, X05, X06: application description, launch plan, desktop entry,
//! `cm app`, shared tunnel reference counting and reassignment.

mod pack_support;

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

use cm::app::cli::stored_command;
use cm::app::{
    LaunchSpec, Reassign, ReassignReason, RefAction, SpecError, TunnelRefs, check, decide,
    desktop_entry, exec_quote, plan, rebuild,
};
use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::client::{ClientError, call, frame_of};
use cm::controller::drop::RunAs;
use cm::controller::protocol::Op;
use cm::profiles::{
    ApplicationAssignment, ApplicationDefinition, ApplicationEnvironmentVariable,
    EnvironmentPreset, Id, SCHEMA_VERSION, Session, SessionLifecycle, TunnelOwner,
};
use serde_json::{Value, json};

fn id(value: &str) -> Id {
    Id::new(value).unwrap()
}

fn definition(executable: &str, argv: &[&str], env: &[(&str, &str)]) -> ApplicationDefinition {
    ApplicationDefinition {
        schema_version: SCHEMA_VERSION,
        id: id("browser"),
        executable: executable.to_owned(),
        argv: argv.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd: None,
        data_profile_id: id("data-1"),
        environment: env
            .iter()
            .map(|(name, value)| ApplicationEnvironmentVariable {
                name: (*name).to_owned(),
                value: (*value).to_owned(),
            })
            .collect(),
        environment_preset_id: id("preset-1"),
        assignment: ApplicationAssignment::OwnTunnel {
            tunnel_instance_id: id("t1"),
        },
        autostart: false,
    }
}

#[test]
fn m01_description_cannot_replace_program_region_or_libraries() {
    let spec = check(&definition(
        "/usr/bin/firefox",
        &["--new-window"],
        &[("MOZ_ENABLE_WAYLAND", "1")],
    ))
    .unwrap();
    assert_eq!(
        spec,
        LaunchSpec {
            program: PathBuf::from("/usr/bin/firefox"),
            args: vec!["--new-window".to_owned()],
            cwd: None,
            env: BTreeMap::from([("MOZ_ENABLE_WAYLAND".to_owned(), "1".to_owned())]),
        }
    );
    let error = |definition: ApplicationDefinition| check(&definition).unwrap_err();
    assert_eq!(
        error(definition("firefox", &[], &[])),
        SpecError::NotAbsolute
    );
    assert_eq!(
        error(definition("/usr/../bin/sh", &[], &[])),
        SpecError::DotDot
    );
    assert_eq!(
        error(definition("/usr/bin/a\0b", &[], &[])),
        SpecError::HasNul
    );
    assert_eq!(
        error(definition("/bin/sh", &["a\0b"], &[])),
        SpecError::HasNul
    );
    assert_eq!(
        error(definition("/bin/sh", &["a"; 257], &[])),
        SpecError::TooManyArgs
    );
    assert!(check(&definition("/bin/sh", &["a"; 256], &[])).is_ok());
    let long = "a".repeat(8193);
    assert_eq!(
        error(definition("/bin/sh", &[&long], &[])),
        SpecError::ArgTooLong
    );
    for cwd in ["relative", "/a/../b"] {
        let mut bad = definition("/bin/sh", &[], &[]);
        bad.cwd = Some(cwd.to_owned());
        assert_eq!(error(bad), SpecError::BadCwd);
    }
    for name in ["1A", "A-B", ""] {
        assert_eq!(
            error(definition("/bin/sh", &[], &[(name, "1")])),
            SpecError::BadEnvName
        );
    }
    assert_eq!(
        error(definition("/bin/sh", &[], &[("A", "1"), ("A", "2")])),
        SpecError::DuplicateEnv
    );
    for name in [
        "LD_PRELOAD",
        "LD_LIBRARY_PATH",
        "TZ",
        "LANG",
        "LANGUAGE",
        "LC_ALL",
        "LC_TIME",
        "PATH",
        "HOME",
        "DBUS_SESSION_BUS_ADDRESS",
        "XDG_RUNTIME_DIR",
    ] {
        let failure = error(definition("/bin/sh", &[], &[(name, "secret-value-marker")]));
        assert_eq!(failure, SpecError::ForbiddenEnv(name.to_owned()));
        assert!(!failure.to_string().contains("secret-value-marker"));
    }
}

fn run_as() -> RunAs {
    RunAs {
        uid: 1000,
        gid: 1000,
        groups: vec![1000, 998],
    }
}

#[test]
fn m02_x06_plan_environment() {
    let spec = check(&definition("/usr/bin/firefox", &[], &[("A", "1")])).unwrap();
    let preset = EnvironmentPreset {
        schema_version: SCHEMA_VERSION,
        id: id("preset-1"),
        timezone: "Europe/Berlin".to_owned(),
        locale: "de_DE.UTF-8".to_owned(),
        languages: vec!["de-DE".to_owned()],
    };
    let session: BTreeMap<String, String> = [
        ("DISPLAY", ":0"),
        ("HOME", "/h"),
        ("SECRET_TOKEN", "x"),
        ("TZ", "Europe/Moscow"),
        ("LC_TIME", "ru_RU.UTF-8"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    let launch = plan(&spec, "cm-3", run_as(), &preset, &session);
    let expected: BTreeMap<String, String> = [
        ("A", "1"),
        ("DISPLAY", ":0"),
        ("HOME", "/h"),
        ("LANG", "de_DE.UTF-8"),
        ("PATH", "/usr/bin:/bin"),
        ("TZ", "Europe/Berlin"),
    ]
    .into_iter()
    .map(|(key, value)| (key.to_owned(), value.to_owned()))
    .collect();
    assert_eq!(launch.env, expected);
    assert_eq!(launch.netns, "cm-3");
    assert_eq!(launch.run_as, run_as());
    assert_eq!(launch.program, Path::new("/usr/bin/firefox"));
    let source = include_str!("../src/app/plan.rs");
    assert!(source.contains("allowlisted_env"));
    assert!(!source.contains("\"DISPLAY\"") && !source.contains("\"WAYLAND_DISPLAY\""));
}

#[test]
fn m03_stored_command_keeps_cwd_and_env_inside_the_application_network() {
    let plain = check(&definition("/usr/bin/firefox", &["-a"], &[])).unwrap();
    assert_eq!(
        stored_command(&plain),
        Some(("/usr/bin/firefox".to_owned(), vec!["-a".to_owned()]))
    );
    let mut with_cwd = definition("/usr/bin/firefox", &["-a"], &[("A", "1")]);
    with_cwd.cwd = Some("/w".to_owned());
    let spec = check(&with_cwd).unwrap();
    assert_eq!(
        stored_command(&spec),
        Some((
            "/usr/bin/env".to_owned(),
            vec![
                "--chdir=/w".to_owned(),
                "A=1".to_owned(),
                "/usr/bin/firefox".to_owned(),
                "-a".to_owned()
            ]
        ))
    );
}

const ENTRY: &str = "[Desktop Entry]\nType=Application\nName=Браузер\nExec=cm app run browser\nTerminal=false\nX-CM-Application=browser\n";

#[test]
fn m04_desktop_entry() {
    assert_eq!(desktop_entry("browser", "Браузер", None).unwrap(), ENTRY);
    let with_icon = desktop_entry("browser", "Браузер", Some("firefox")).unwrap();
    assert_eq!(
        with_icon.lines().collect::<Vec<_>>()[3..5],
        ["Exec=cm app run browser", "Icon=firefox"]
    );
    assert_eq!(with_icon.lines().nth(5), Some("Terminal=false"));
    assert_eq!(exec_quote("plain"), "plain");
    assert_eq!(exec_quote("a b"), "\"a b\"");
    assert_eq!(exec_quote("a$b"), "\"a\\$b\"");
    assert_eq!(exec_quote("50%"), "50%%");
    assert_eq!(exec_quote("a\"b"), "\"a\\\"b\"");
    assert!(desktop_entry("browser", "a\nb", None).is_err());
    assert!(
        desktop_entry("browser", "a\\b", None)
            .unwrap()
            .contains("Name=a\\\\b\n")
    );
    for bad in ["../x", "a b", ""] {
        assert_eq!(desktop_entry(bad, "N", None).err(), Some(SpecError::BadId));
    }
    // The entry can only run `cm app run <id>`: no other Exec line can be produced.
    for name in ["x\rExec=/bin/sh", "x\tb"] {
        assert!(desktop_entry("browser", name, None).is_err());
    }
}

fn cm(args: &[&str], env: &[(&str, &str)]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cm"));
    // The host config may pin a language; the test reads none.
    command
        .args(args)
        .env("CM_CONF", "/nonexistent/cm.conf")
        .env_remove("LANG")
        .env_remove("LANGUAGE")
        .env_remove("LC_MESSAGES")
        .env("LC_ALL", "C");
    for (key, value) in env {
        command.env(key, value);
    }
    command.output().unwrap()
}

#[test]
fn m05_command_prints_the_entry() {
    let output = cm(&["app", "desktop", "browser", "Браузер"], &[]);
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap(), ENTRY);
    assert_eq!(
        cm(&["app", "desktop", "../x", "N"], &[]).status.code(),
        Some(2)
    );
    assert_eq!(
        cm(&["app", "desktop", "browser"], &[]).status.code(),
        Some(2)
    );
}

/// A server that answers each connection with the next scripted line and records the frames.
fn fake_controller(
    dir: &Path,
    replies: Vec<String>,
) -> (PathBuf, std::thread::JoinHandle<Vec<Value>>) {
    let path = dir.join("controller.sock");
    let listener = UnixListener::bind(&path).unwrap();
    let thread = std::thread::spawn(move || {
        let mut frames = Vec::new();
        for answer in replies {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(stream.try_clone().unwrap())
                .read_line(&mut line)
                .unwrap();
            frames.push(serde_json::from_str(&line).unwrap());
            stream.write_all(answer.as_bytes()).unwrap();
        }
        frames
    });
    (path, thread)
}

fn status_reply(generation: u64) -> String {
    format!(
        "{}\n",
        json!({"v":1,"id":"x","ok":true,"code":"ok","data":
            {"type":"status","running":true,"generation":generation,"api":"api_ready","route":"route_ready","remote":"unknown"}})
    )
}

#[test]
fn m06_usage_refusal_and_unavailable_are_different_exit_codes() {
    let dir = TempDirGuard::new("cm-pack-m06").unwrap();
    for language in [
        "en_US.UTF-8",
        "de_DE.UTF-8",
        "it_IT.UTF-8",
        "zh_CN.UTF-8",
        "ar_SA.UTF-8",
    ] {
        let output = cm(&["app"], &[("LC_ALL", language), ("LANG", language)]);
        assert_eq!(output.status.code(), Some(2));
        let text = String::from_utf8(output.stdout).unwrap();
        assert!(text.contains("cm app run"), "{language}: {text}");
        assert!(
            !text
                .chars()
                .any(|ch| ('\u{0400}'..='\u{04FF}').contains(&ch)),
            "{language}: {text}"
        );
    }
    let good = dir.path().join("good.json");
    fs::write(
        &good,
        serde_json::to_vec(&definition(
            "/usr/bin/firefox",
            &["-a"],
            &[("TOKEN", "secret-value-marker")],
        ))
        .unwrap(),
    )
    .unwrap();
    let output = cm(&["app", "check", good.to_str().unwrap()], &[]);
    assert_eq!(output.status.code(), Some(0));
    let text = String::from_utf8(output.stdout).unwrap();
    assert!(
        text.contains("TOKEN") && !text.contains("secret-value-marker"),
        "{text}"
    );
    let bad = dir.path().join("bad.json");
    fs::write(
        &bad,
        serde_json::to_vec(&definition(
            "/usr/bin/firefox",
            &[],
            &[("LD_PRELOAD", "/x.so")],
        ))
        .unwrap(),
    )
    .unwrap();
    let output = cm(&["app", "check", bad.to_str().unwrap()], &[]);
    assert_eq!(output.status.code(), Some(2));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("[ForbiddenEnv]")
    );

    let launched = format!(
        "{}\n",
        json!({"v":1,"id":"x","ok":true,"code":"ok","data":{"type":"launched","pid":42}})
    );
    let (socket, server) = fake_controller(dir.path(), vec![status_reply(7), launched]);
    let env = [
        ("CM_CONTROLLER_SOCKET", socket.to_str().unwrap()),
        ("WAYLAND_DISPLAY", "wayland-7"),
        ("LD_PRELOAD", ""),
        ("SECRET_TOKEN", "secret-value-marker"),
    ];
    let output = cm(
        &["app", "run", "browser", "--", "/usr/bin/true", "-x"],
        &env,
    );
    assert_eq!(output.status.code(), Some(0));
    assert_eq!(String::from_utf8(output.stdout).unwrap().trim(), "42");
    let frames = server.join().unwrap();
    assert_eq!(
        frames[0]["op"],
        json!({"type":"worker_status","instance":"browser"})
    );
    assert_eq!(frames[1]["op"]["type"], "app_launch");
    assert_eq!(frames[1]["op"]["generation"], 7);
    assert_eq!(frames[1]["op"]["program"], "/usr/bin/true");
    assert_eq!(frames[1]["op"]["args"], json!(["-x"]));
    // Only session variables travel; the rest of the client's environment stays behind.
    assert_eq!(frames[1]["op"]["env"]["WAYLAND_DISPLAY"], "wayland-7");
    assert!(!frames[1].to_string().contains("secret-value-marker"));
    for frame in &frames {
        assert_eq!(frame["v"], 1);
        let id = frame["id"].as_str().unwrap();
        assert!(
            id.len() == 16 && id.bytes().all(|b| b.is_ascii_hexdigit()),
            "{id}"
        );
    }
    fs::remove_file(&socket).unwrap();

    let refusal = format!(
        "{}\n",
        json!({"v":1,"id":"x","ok":false,"code":"not_running","data":null})
    );
    let (socket, server) = fake_controller(dir.path(), vec![status_reply(7), refusal]);
    let env = [("CM_CONTROLLER_SOCKET", socket.to_str().unwrap())];
    let output = cm(&["app", "run", "browser", "--", "/usr/bin/true"], &env);
    assert_eq!(output.status.code(), Some(3));
    assert!(
        String::from_utf8(output.stderr)
            .unwrap()
            .contains("[not_running]")
    );
    server.join().unwrap();
    fs::remove_file(&socket).unwrap();

    // X05: a reply that is not one JSON line of at most 64 KiB is a client error.
    for garbage in ["not json\n".to_owned(), format!("{}\n", "x".repeat(70_000))] {
        let (socket, server) = fake_controller(dir.path(), vec![garbage]);
        let env = [("CM_CONTROLLER_SOCKET", socket.to_str().unwrap())];
        let output = cm(&["app", "run", "browser", "--", "/usr/bin/true"], &env);
        assert_eq!(output.status.code(), Some(4));
        server.join().unwrap();
        fs::remove_file(&socket).unwrap();
    }
    let missing = dir.path().join("none.sock");
    let env = [("CM_CONTROLLER_SOCKET", missing.to_str().unwrap())];
    assert_eq!(
        cm(&["app", "run", "browser", "--", "/usr/bin/true"], &env)
            .status
            .code(),
        Some(4)
    );
    // Usage errors never reach the socket.
    let listener = UnixListener::bind(&missing).unwrap();
    listener.set_nonblocking(true).unwrap();
    assert_eq!(
        cm(&["app", "run", "browser", "--", "true"], &env)
            .status
            .code(),
        Some(2)
    );
    assert_eq!(
        cm(&["app", "run", "browser", "extra"], &env).status.code(),
        Some(2)
    );
    assert!(listener.accept().is_err());
}

#[test]
fn x05_client_speaks_the_protocol_frame() {
    let dir = TempDirGuard::new("cm-pack-x05").unwrap();
    let bytes = frame_of(Op::Reconcile).unwrap();
    assert_eq!(bytes.iter().filter(|byte| **byte == b'\n').count(), 1);
    let frame: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(frame["v"], 1);
    assert_eq!(frame["op"], json!({"type":"reconcile"}));
    assert_eq!(frame["id"].as_str().unwrap().len(), 16);
    assert!(cm::controller::codec::decode(&bytes[..bytes.len() - 1]).is_ok());
    let ok = format!(
        "{}\n",
        json!({"v":1,"id":"x","ok":true,"code":"ok","data":null})
    );
    let (socket, server) = fake_controller(dir.path(), vec![ok]);
    let reply = call(&socket, Op::Reconcile).unwrap();
    assert!(reply.ok);
    assert_eq!(server.join().unwrap().len(), 1);
    assert_eq!(
        call(&dir.path().join("none.sock"), Op::Reconcile).err(),
        Some(ClientError::Unavailable)
    );
}

fn session(name: &str, tunnel: &str, lifecycle: SessionLifecycle, generation: u64) -> Session {
    Session {
        schema_version: SCHEMA_VERSION,
        id: id(name),
        lifecycle,
        application_id: id("browser"),
        data_profile_id: id("data-1"),
        environment_profile_id: id("env-1"),
        tunnel_instance_id: id(tunnel),
        tunnel_generation: generation,
        source_id: id("source-1"),
        source_generation: 1,
        node_id: id("node-1"),
        created_at_unix_ms: 1,
        ended_at_unix_ms: None,
    }
}

#[test]
fn m07_shared_tunnel_reference_counting() {
    let (t, s1, s2) = (id("t"), id("s1"), id("s2"));
    let group = TunnelOwner::Group { group_id: id("g") };
    let mut refs = TunnelRefs::default();
    assert_eq!(refs.acquire(&t, &s1), RefAction::StartTunnel(t.clone()));
    assert_eq!(refs.acquire(&t, &s2), RefAction::None);
    assert_eq!(refs.acquire(&t, &s1), RefAction::None);
    assert_eq!(refs.holders(&t), 2);
    assert_eq!(refs.release(&t, &id("s9"), &group), RefAction::None);
    assert_eq!(refs.holders(&t), 2);
    assert_eq!(refs.release(&t, &s1, &group), RefAction::None);
    assert_eq!(
        refs.release(&t, &s2, &group),
        RefAction::StopTunnel(t.clone())
    );
    assert_eq!(refs.holders(&t), 0);
    assert_eq!(refs.release(&t, &s2, &group), RefAction::None);
    assert_eq!(refs.holders(&t), 0);
    refs.acquire(&t, &s1);
    assert_eq!(refs.release(&t, &s1, &TunnelOwner::Host), RefAction::None);

    // Every interleaving that acquires before it releases starts and stops exactly once.
    let ops = [(true, &s1), (true, &s2), (false, &s1), (false, &s2)];
    let mut orders = 0;
    for a in 0..4 {
        for b in 0..4 {
            for c in 0..4 {
                for d in 0..4 {
                    let order = [a, b, c, d];
                    let mut seen = [false; 4];
                    order.iter().for_each(|index| seen[*index] = true);
                    let place =
                        |index: usize| order.iter().position(|item| *item == index).unwrap();
                    if seen != [true; 4] || place(0) > place(2) || place(1) > place(3) {
                        continue;
                    }
                    orders += 1;
                    let mut refs = TunnelRefs::default();
                    let mut actions = Vec::new();
                    for index in order {
                        let (acquire, session) = ops[index];
                        actions.push(if acquire {
                            refs.acquire(&t, session)
                        } else {
                            refs.release(&t, session, &group)
                        });
                    }
                    let starts = actions
                        .iter()
                        .filter(|a| matches!(a, RefAction::StartTunnel(_)))
                        .count();
                    let stops = actions
                        .iter()
                        .filter(|a| matches!(a, RefAction::StopTunnel(_)))
                        .count();
                    // Sessions that do not overlap give two start/stop pairs; overlapping give one.
                    assert_eq!(starts, stops, "{order:?}");
                    assert_eq!(actions.last(), Some(&RefAction::StopTunnel(t.clone())));
                    assert_eq!(refs.holders(&t), 0);
                }
            }
        }
    }
    assert_eq!(orders, 6);
    let rebuilt = rebuild(&[
        session("s1", "t", SessionLifecycle::Active, 1),
        session("s2", "t", SessionLifecycle::Active, 1),
        session("s3", "t", SessionLifecycle::Ended, 1),
    ]);
    assert_eq!(rebuilt.holders(&t), 2);
}

#[test]
fn m08_reassignment_never_redirects_a_live_session() {
    let own = |tunnel: &str| ApplicationAssignment::OwnTunnel {
        tunnel_instance_id: id(tunnel),
    };
    let group = ApplicationAssignment::Group { group_id: id("g") };
    let live = session("s1", "t1", SessionLifecycle::Active, 3);
    let ended = session("s1", "t1", SessionLifecycle::Ended, 3);
    assert_eq!(
        decide(None, &own("t1"), &own("t1"), 3, true),
        Reassign::NoChange
    );
    assert_eq!(
        decide(None, &own("t1"), &own("t2"), 3, true),
        Reassign::AppliesToNextLaunch
    );
    assert_eq!(
        decide(Some(&ended), &own("t1"), &own("t2"), 3, true),
        Reassign::AppliesToNextLaunch
    );
    let restart = |reason| Reassign::RestartRequired { reason };
    assert_eq!(
        decide(Some(&live), &own("t1"), &own("t2"), 3, true),
        restart(ReassignReason::TunnelChanged)
    );
    assert_eq!(
        decide(Some(&live), &own("t1"), &group, 3, true),
        restart(ReassignReason::TunnelChanged)
    );
    assert_eq!(
        decide(Some(&live), &own("t1"), &own("t1"), 4, true),
        restart(ReassignReason::GenerationChanged)
    );
    assert_eq!(
        decide(Some(&live), &own("t1"), &own("t1"), 3, true),
        Reassign::NoChange
    );
    for (session, wanted) in [
        (Some(&live), own("t1")),
        (None, own("t2")),
        (Some(&live), group.clone()),
    ] {
        assert_eq!(
            decide(session, &own("t1"), &wanted, 3, false),
            restart(ReassignReason::NodeGone)
        );
    }
}
