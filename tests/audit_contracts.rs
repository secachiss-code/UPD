use cm::common::contract_fixtures::{self as fixtures, ChildGuard, EnvGuard, TempDirGuard};
use cm::helper::{self, Event, PromptKind, Request};
use std::io::{BufRead, Write};
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::{Duration, Instant};

const FIXTURE_TIMEOUT: Duration = Duration::from_secs(8);

fn wait_for_socket(child: &mut ChildGuard, socket: &Path) {
    let deadline = Instant::now() + Duration::from_secs(4);
    while Instant::now() < deadline {
        if let Some(status) = child.try_wait().expect("helper child status") {
            panic!("local helper exited before binding its fixture socket: {status}");
        }
        if socket.exists() {
            return;
        }
        thread::sleep(Duration::from_millis(10));
    }
    panic!("local helper did not bind its fixture socket before timeout");
}

fn spawn_helper(dir: &TempDirGuard, socket: &Path, fake_cm: Option<&Path>) -> ChildGuard {
    let mut command = Command::new(env!("CARGO_BIN_EXE_cm"));
    command
        .arg("helper")
        .env("CM_STATE_DIR", dir.path())
        .env("CM_HELPER_SOCK", socket)
        .env("CM_HELPER_ALLOW", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(
            std::fs::File::create(dir.path().join("helper.stdout")).unwrap(),
        ))
        .stderr(Stdio::from(
            std::fs::File::create(dir.path().join("helper.stderr")).unwrap(),
        ));
    if let Some(path) = fake_cm {
        command.env("CM_HELPER_EXE", path);
    }
    // Resource baselines count the helper's fds. Descriptors that the test process itself
    // inherited without CLOEXEC (IDE terminal, agent, jobserver) must not reach the fixture.
    // SAFETY: the closure only calls async-signal-safe setsid and ioctl(TIOCSCTTY) and allocates nothing.
    unsafe {
        use std::os::unix::process::CommandExt;
        command.pre_exec(|| {
            // stdio is already in place; ENOSYS on pre-5.9 kernels leaves the old behaviour.
            libc::syscall(libc::SYS_close_range, 3u32, u32::MAX, 0u32);
            Ok(())
        });
    }
    let mut child = ChildGuard::spawn(&mut command).expect("spawn local helper fixture");
    wait_for_socket(&mut child, socket);
    child
}

fn wait_for_finished(
    operation_id: &helper::OperationId,
    timeout: Duration,
) -> cm::summary::OpStatus {
    let deadline = Instant::now() + timeout;
    loop {
        let status: cm::summary::OpStatus =
            helper::call_as(&Request::Status).expect("helper status reply");
        if status.operation_id.as_ref() == Some(operation_id) && !status.running {
            return status;
        }
        assert!(
            Instant::now() < deadline,
            "operation {operation_id:?} did not finish: {status:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

fn process_is_running(pid: i32) -> bool {
    std::fs::read_to_string(format!("/proc/{pid}/stat"))
        .ok()
        .and_then(|stat| {
            stat.rsplit_once(')')
                .map(|(_, rest)| rest.trim_start().to_string())
        })
        .and_then(|rest| rest.split_whitespace().next().map(str::to_owned))
        .is_some_and(|state| state != "Z" && state != "X")
}

struct OperationCancelGuard(helper::OperationId);

impl Drop for OperationCancelGuard {
    fn drop(&mut self) {
        let _ = helper::call(&Request::Cancel {
            operation_id: self.0.clone(),
        });
    }
}

struct ProcessGuard(Option<i32>);

impl ProcessGuard {
    fn kill(&mut self) {
        if let Some(pid) = self.0.take() {
            // SAFETY: sending a signal accesses no memory; pid is the fixture child spawned by this test.
            unsafe { libc::kill(pid, libc::SIGKILL) };
        }
    }
}

impl Drop for ProcessGuard {
    fn drop(&mut self) {
        self.kill();
    }
}

fn with_mock_reply_and_events(
    socket: &Path,
    reply_and_events: &'static [u8],
) -> thread::JoinHandle<std::io::Result<()>> {
    let listener = UnixListener::bind(socket).expect("bind mock helper socket");
    listener
        .set_nonblocking(true)
        .expect("nonblocking mock listener");
    thread::spawn(move || {
        let deadline = Instant::now() + Duration::from_secs(3);
        for response in [
            &b"{\"protocol_version\":2,\"ok\":true}\n"[..],
            reply_and_events,
        ] {
            let (mut stream, _) = loop {
                match listener.accept() {
                    Ok(pair) => break pair,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(5));
                    }
                    Err(error) => return Err(error),
                }
            };
            stream.set_read_timeout(Some(Duration::from_secs(2)))?;
            let mut request = String::new();
            std::io::BufReader::new(&stream).read_line(&mut request)?;
            stream.write_all(response)?;
        }
        Ok(())
    })
}

/// The reply, event frames, and replay boundary arrive in one write and retain their order.
#[test]
fn helper_framing_preserves_events_coalesced_with_reply() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-framing").unwrap();
    let socket = dir.path().join("mock.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let server = with_mock_reply_and_events(
        &socket,
        b"{\"protocol_version\":2,\"ok\":true}\n{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":{\"ev\":\"reset\",\"command\":\"check\",\"started\":1}}\n{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":{\"ev\":\"exit\",\"code\":0}}\n{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":{\"ev\":\"replay_complete\"}}\n",
    );
    let frames: Vec<helper::OperationEvent> = helper::attach_events()
        .unwrap()
        .collect::<Result<_, _>>()
        .unwrap();
    server
        .join()
        .expect("mock helper thread joins")
        .expect("mock helper transport succeeds");
    assert_eq!(
        frames
            .iter()
            .map(|frame| frame.event.clone())
            .collect::<Vec<_>>(),
        vec![
            Event::Reset {
                command: "check".into(),
                started: 1
            },
            Event::Exit { code: 0 },
            Event::ReplayComplete,
        ]
    );
    assert!(
        frames
            .iter()
            .all(|frame| frame.operation_id == helper::OperationId("fixture-op".into()))
    );
}

#[test]
fn helper_framing_reports_malformed_event_json() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-malformed").unwrap();
    let socket = dir.path().join("mock.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let server = with_mock_reply_and_events(
        &socket,
        b"{\"protocol_version\":2,\"ok\":true}\n{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":\n",
    );
    let mut events = helper::attach_events().unwrap();
    let error = events.next().unwrap().unwrap_err();
    server
        .join()
        .expect("mock helper thread joins")
        .expect("mock helper transport succeeds");
    assert!(error.contains("invalid event frame"), "{error}");
    assert!(
        events.next().is_none(),
        "a malformed frame ends this event stream"
    );
}

#[test]
fn helper_client_rejects_old_server_before_mutation() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-old-server").unwrap();
    let socket = dir.path().join("mock.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let listener = UnixListener::bind(&socket).unwrap();
    let server = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        let mut request = String::new();
        std::io::BufReader::new(&stream)
            .read_line(&mut request)
            .unwrap();
        let request: serde_json::Value = serde_json::from_str(&request).unwrap();
        assert_eq!(
            request["op"], "hello",
            "the preflight has no mutation request"
        );
        stream.write_all(b"{\"ok\":true}\n").unwrap();
    });

    let error = helper::call(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap_err();
    server.join().unwrap();
    assert!(error.contains("helper protocol mismatch"), "{error}");
    assert!(error.contains("helper 0"), "{error}");
}

#[test]
fn new_helper_rejects_legacy_mutation_without_protocol_version() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-legacy-client").unwrap();
    let socket = dir.path().join("helper.sock");
    let marker = dir.path().join("should-not-run");
    let fake_cm = dir.path().join("fake-cm");
    std::fs::write(&fake_cm, format!("#!/bin/sh\ntouch {}\n", marker.display())).unwrap();
    std::fs::set_permissions(&fake_cm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake_cm));

    let mut stream = UnixStream::connect(&socket).unwrap();
    stream.set_read_timeout(Some(FIXTURE_TIMEOUT)).unwrap();
    stream
        .write_all(b"{\"lang\":\"en\",\"op\":\"start\",\"args\":[\"check\"]}\n")
        .unwrap();
    let mut response = String::new();
    std::io::BufReader::new(&stream)
        .read_line(&mut response)
        .unwrap();
    let reply: helper::Reply = serde_json::from_str(&response).unwrap();
    assert!(!reply.ok);
    assert!(
        reply.error.contains("helper protocol mismatch"),
        "{}",
        reply.error
    );
    assert!(
        !marker.exists(),
        "legacy mutation must be rejected before the fake child starts"
    );

    let mut legacy_input = UnixStream::connect(&socket).unwrap();
    legacy_input
        .set_read_timeout(Some(FIXTURE_TIMEOUT))
        .unwrap();
    legacy_input
        .write_all(b"{\"lang\":\"en\",\"op\":\"input\",\"data\":\"y\"}\n")
        .unwrap();
    let mut response = String::new();
    std::io::BufReader::new(&legacy_input)
        .read_line(&mut response)
        .unwrap();
    let reply: helper::Reply = serde_json::from_str(&response).unwrap();
    assert!(
        reply.error.contains("helper protocol mismatch"),
        "{}",
        reply.error
    );

    let status: cm::summary::OpStatus = helper::call_as(&Request::Status).unwrap();
    assert!(status.operation_id.is_none());
    child.terminate().unwrap();
}

/// Long output is truncated with an explicit marker.
#[test]
fn audit_term_bounds_long_lines() {
    let mut term = helper::Term::default();
    let bytes = vec![b'x'; 2 * 1024 * 1024];
    term.feed(&bytes);
    assert!(term.partial.len() <= 16 * 1024);
    let lines = term.feed(b"\n");
    assert!(lines[0].len() <= 16 * 1024);
    assert!(lines[0].ends_with("…"));
}

/// Invalid bytes do not strand the remaining output.
#[test]
fn audit_utf8_preserves_valid_tail_after_invalid_byte() {
    let mut term = helper::Term::default();
    let lines = term.feed(b"\xffhello\n");
    assert_eq!(lines, ["�hello"]);
    assert!(term.partial.is_empty());
}

fn proc_counts(pid: u32) -> (usize, usize) {
    // Directory iteration may count both retiring and newly created tasks.
    // The kernel status counter is an instantaneous thread count.
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let tasks = status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let fds = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .count();
    (tasks, fds)
}

/// Disconnect is detected without waiting for another operation event.
#[test]
fn audit_closed_attach_releases_helper_thread_and_fd() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-attach").unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, None);
    // Socket creation precedes backend detection, which briefly opens os-release.
    // This fixture owns only stdin/stdout/stderr and the listener when idle.
    let ready = wait_resource_baseline(child.id(), &(1, 4));
    let before = (ready.threads, ready.fds);

    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let mut errors = 0;
        for _ in 0..100 {
            match helper::attach_events() {
                Ok(events) => drop(events),
                Err(_) => errors += 1,
            }
        }
        let _ = tx.send(errors);
    });
    let errors = match rx.recv_timeout(FIXTURE_TIMEOUT) {
        Ok(errors) => errors,
        Err(error) => {
            let _ = child.terminate();
            let _ = worker.join();
            panic!("attach fixture exceeded its deadline: {error}");
        }
    };
    worker.join().expect("attach fixture worker joins");
    assert_eq!(errors, 0);

    let deadline = Instant::now() + Duration::from_secs(2);
    let mut after = proc_counts(child.id());
    while after != before && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
        after = proc_counts(child.id());
    }
    assert!(
        after == before,
        "attach workers and sockets must be released: before={before:?}, after={after:?}"
    );
    child
        .terminate()
        .expect("helper process is killed and reaped");
}

/// A complete safe helper fixture: only a local fake executable runs, and every wait is bounded.
#[test]
fn helper_fixture_runs_fake_child_and_reaps_helper() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-e2e").unwrap();
    let fake_cm = dir.path().join("fake-cm");
    std::fs::write(
        &fake_cm,
        "#!/bin/sh\necho \"[1/2] Start $*\"\nprintf 'Go on? [Y/n] '\nread answer\necho \"answer=$answer\"\necho '[2/2] Done'\nexit 3\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_cm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake_cm));

    let (tx, rx) = mpsc::sync_channel(1);
    let worker = thread::spawn(move || {
        let result =
            (|| -> Result<(i32, Vec<Event>, helper::StartReply, cm::summary::OpStatus), String> {
                // Subscribe before starting so this contract fixture does not depend on packet coalescing.
                let events = helper::attach_events()?;
                if helper::call(&Request::Start {
                    args: vec!["install".into()],
                })
                .is_ok()
                {
                    return Err("disallowed command was accepted".into());
                }
                let started = helper::call_as::<helper::StartReply>(&Request::Start {
                    args: vec!["check".into()],
                })?;
                let mut seen = Vec::new();
                let mut code = None;
                for item in events {
                    let frame = item?;
                    if frame.operation_id != started.operation_id {
                        return Err("event belongs to another operation".into());
                    }
                    let event = frame.event;
                    if let Event::Prompt { kind, .. } = &event {
                        if *kind != (PromptKind::YesNo { default_yes: true }) {
                            return Err(format!("unexpected prompt kind: {kind:?}"));
                        }
                        let prompt_id = frame
                            .prompt_id
                            .ok_or_else(|| "prompt event has no prompt id".to_string())?;
                        helper::call(&Request::Input {
                            operation_id: started.operation_id.clone(),
                            prompt_id,
                            data: "y".into(),
                        })?;
                    }
                    if let Event::Exit { code: exit } = &event {
                        code = Some(*exit);
                        break;
                    }
                    seen.push(event);
                }
                let code = code.ok_or_else(|| "fake operation ended without Exit".to_string())?;
                let status: cm::summary::OpStatus = helper::call_as(&Request::Status)?;
                Ok((code, seen, started, status))
            })();
        let _ = tx.send(result);
    });

    let outcome = match rx.recv_timeout(FIXTURE_TIMEOUT) {
        Ok(outcome) => outcome,
        Err(error) => {
            let _ = child.terminate();
            let _ = worker.join();
            panic!("fake helper operation exceeded its deadline: {error}");
        }
    };
    worker.join().expect("fake helper client worker joins");
    let (code, events, started, status) = match outcome {
        Ok(value) => value,
        Err(error) => {
            let _ = child.terminate();
            panic!("fake helper operation failed: {error}");
        }
    };
    assert_eq!(code, 3);
    assert!(
        events.contains(&Event::Stage {
            n: 2,
            m: 2,
            title: "Done".into()
        }),
        "{events:?}"
    );
    assert!(
        events
            .iter()
            .any(|event| matches!(event, Event::Line { text } if text.ends_with("answer=y"))),
        "{events:?}"
    );
    assert_eq!(
        (status.running, status.last_exit, status.command.as_str()),
        (false, Some(3), "check")
    );
    assert_eq!(status.operation_id, Some(started.operation_id));
    child
        .terminate()
        .expect("local helper process is killed and reaped");
}

#[test]
fn helper_runner_launch_failures_finish_and_clean_up() {
    let _isolation = fixtures::isolation_lock();
    for (case, injection) in [
        ("child", "CM_HELPER_FAIL_CHILD"),
        ("clone", "CM_HELPER_FAIL_CLONE"),
        ("reader", "CM_HELPER_FAIL_WORKER"),
        ("runner", "CM_HELPER_FAIL_WORKER"),
        ("inline", "CM_HELPER_FAIL_WORKER"),
    ] {
        let dir = TempDirGuard::new(&format!("cm-audit-launch-{case}")).unwrap();
        let fake_cm = dir.path().join("fake-cm");
        std::fs::write(
            &fake_cm,
            "#!/bin/sh\nbase=${0%/*}\n( sleep 0.25; : > \"$base/late-marker\" ) &\nwait\n",
        )
        .unwrap();
        std::fs::set_permissions(&fake_cm, std::fs::Permissions::from_mode(0o755)).unwrap();
        let socket = dir.path().join("helper.sock");
        let mut env = EnvGuard::new();
        env.remove("CM_HELPER_FAIL_CHILD");
        env.remove("CM_HELPER_FAIL_CLONE");
        env.remove("CM_HELPER_FAIL_WORKER");
        match injection {
            "CM_HELPER_FAIL_CHILD" => env.set(injection, "1"),
            "CM_HELPER_FAIL_CLONE" => env.set(injection, "1"),
            _ => env.set(injection, case),
        }
        env.set("CM_HELPER_SOCK", &socket);
        let mut child = spawn_helper(&dir, &socket, Some(&fake_cm));

        let start_result = if case == "inline" {
            helper::call(&Request::VpnAdd {
                url: "https://fixture.invalid/subscription".into(),
                name: "fixture".into(),
            })
        } else {
            helper::call_as::<helper::StartReply>(&Request::Start {
                args: vec!["check".into()],
            })
            .map(|_| serde_json::Value::Null)
        };
        assert!(
            start_result.is_err(),
            "{case} failure injection should reject the launch"
        );
        let initial_status: cm::summary::OpStatus =
            helper::call_as(&Request::Status).expect("status after launch failure");
        let operation_id = initial_status
            .operation_id
            .expect("failed launch keeps its operation id");
        let status = wait_for_finished(&operation_id, Duration::from_secs(2));
        assert!(
            !status.running,
            "{case} launch failure must leave no Running operation: {status:?}"
        );
        assert_eq!(status.last_exit, Some(127), "{case}: {status:?}");
        if case != "child" && case != "inline" {
            thread::sleep(Duration::from_millis(350));
            assert!(
                !dir.path().join("late-marker").exists(),
                "{case} failure left the child process group alive"
            );
        }
        child.terminate().expect("failed launch helper reaps");
    }
}

#[test]
fn helper_runner_drains_descendant_and_switches_without_stale_output() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-runner-descendant").unwrap();
    let fake_cm = dir.path().join("fake-cm");
    std::fs::write(
        &fake_cm,
        "#!/bin/sh\nbase=${0%/*}\nif [ ! -e \"$base/started\" ]; then\n  : > \"$base/started\"\n  setsid sh -c 'echo $$ > \"$1/descendant.pid\"; while :; do printf \"OLD\\n\"; done' sh \"$base\" &\n  while [ ! -s \"$base/descendant.pid\" ]; do sleep 0.01; done\n  exit 0\nfi\nprintf 'NEW\\n'\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_cm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake_cm));

    let first = helper::call_as::<helper::StartReply>(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    let descendant_path = dir.path().join("descendant.pid");
    let deadline = Instant::now() + Duration::from_secs(2);
    while !descendant_path.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    let descendant_pid: i32 = std::fs::read_to_string(&descendant_path)
        .expect("descendant pid file")
        .trim()
        .parse()
        .unwrap();
    let mut descendant = ProcessGuard(Some(descendant_pid));

    let first_status = wait_for_finished(&first.operation_id, Duration::from_secs(2));
    assert!(
        !first_status.running,
        "leader exit with a PTY-holding descendant must drain in bounded time"
    );
    assert_eq!(first_status.last_exit, Some(0));
    assert!(
        process_is_running(descendant_pid),
        "the fixture descendant must still be writing after its leader exits"
    );

    let second = helper::call_as::<helper::StartReply>(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    let second_status = wait_for_finished(&second.operation_id, Duration::from_secs(2));
    assert_eq!(second_status.last_exit, Some(0));
    let events: Vec<_> = helper::attach_events()
        .unwrap()
        .take_while(|frame| {
            frame
                .as_ref()
                .is_ok_and(|frame| frame.event != Event::ReplayComplete)
        })
        .collect::<Result<_, _>>()
        .unwrap();
    assert!(!events.is_empty());
    assert!(
        events
            .iter()
            .all(|frame| frame.operation_id == second.operation_id),
        "stale operation events leaked: {events:?}"
    );
    assert!(
        events
            .iter()
            .any(|frame| matches!(&frame.event, Event::Line { text } if text == "NEW")),
        "{events:?}"
    );
    assert!(
        !events
            .iter()
            .any(|frame| matches!(&frame.event, Event::Line { text } if text.contains("OLD"))),
        "{events:?}"
    );

    // The helper clears its process-group id as soon as it reaps the leader. Reap
    // this deliberately orphaned fixture process by its captured pid.
    descendant.kill();
    child
        .terminate()
        .expect("helper and operation workers are reaped");
}

#[test]
fn helper_runner_cancel_stops_continuous_output() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-runner-cancel").unwrap();
    let fake_cm = dir.path().join("fake-cm");
    std::fs::write(
        &fake_cm,
        "#!/bin/sh\ntrap '' INT TERM\nwhile :; do printf 'OLD\\n'; done\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake_cm, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake_cm));

    let started = helper::call_as::<helper::StartReply>(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    let _cancel_on_drop = OperationCancelGuard(started.operation_id.clone());
    drop(helper::attach_events().unwrap());
    thread::sleep(Duration::from_millis(50));
    let still_running: cm::summary::OpStatus = helper::call_as(&Request::Status).unwrap();
    assert!(
        still_running.running,
        "closing the event stream must not cancel the operation"
    );
    helper::call(&Request::Cancel {
        operation_id: started.operation_id.clone(),
    })
    .unwrap();
    let status = wait_for_finished(&started.operation_id, Duration::from_secs(2));
    assert!(!status.running);
    assert!(
        matches!(status.last_exit, Some(130 | 143 | 137)),
        "unexpected cancellation exit: {status:?}"
    );
    child
        .terminate()
        .expect("continuous-output helper and reader are reaped");
}

#[test]
fn helper_idle_exit_after_last_subscription() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-idle").unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    env.set("CM_HELPER_IDLE_MS", "100");
    let mut child = spawn_helper(&dir, &socket, None);
    let events = helper::attach_events().unwrap();
    thread::sleep(Duration::from_millis(1200));
    assert!(child.try_wait().unwrap().is_none());
    drop(events);
    let deadline = Instant::now() + Duration::from_secs(3);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            Instant::now() < deadline,
            "helper must exit after its last client disconnects"
        );
        thread::sleep(Duration::from_millis(20));
    }
}

#[test]
fn helper_secret_answer_is_not_echoed_or_replayed() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-secret").unwrap();
    let fake = dir.path().join("fake-cm");
    std::fs::write(&fake, "#!/bin/sh\nprintf '[sudo] password for fixture: '\nread answer\nprintf '\\naccepted\\n'\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake));
    let events = helper::attach_events().unwrap();
    let started: helper::StartReply = helper::call_as(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    let _cancel = OperationCancelGuard(started.operation_id.clone());
    let secret = "fixture-secret-987654";
    let (tx, rx) = mpsc::sync_channel(1);
    let worker_id = started.operation_id.clone();
    let worker = thread::spawn(move || {
        let result = (|| -> Result<Vec<String>, String> {
            let mut frames = Vec::new();
            for frame in events {
                let frame = frame?;
                frames.push(serde_json::to_string(&frame).unwrap());
                if matches!(
                    frame.event,
                    Event::Prompt {
                        kind: PromptKind::Secret,
                        ..
                    }
                ) {
                    helper::call(&Request::Input {
                        operation_id: worker_id.clone(),
                        prompt_id: frame.prompt_id.unwrap(),
                        data: secret.into(),
                    })?;
                }
                if matches!(frame.event, Event::Exit { .. }) {
                    return Ok(frames);
                }
            }
            Err("missing exit".into())
        })();
        let _ = tx.send(result);
    });
    let result = rx.recv_timeout(FIXTURE_TIMEOUT);
    if result.is_err() {
        child.terminate().unwrap();
    }
    worker.join().unwrap();
    let frames = result.unwrap().unwrap();
    assert!(frames.iter().all(|frame| !frame.contains(secret)));
    let replay: Vec<_> = helper::attach_events()
        .unwrap()
        .map(|frame| frame.unwrap())
        .take_while(|frame| !matches!(frame.event, Event::ReplayComplete))
        .collect();
    assert!(!serde_json::to_string(&replay).unwrap().contains(secret));
    for name in ["helper.stdout", "helper.stderr"] {
        assert!(
            !std::fs::read_to_string(dir.path().join(name))
                .unwrap()
                .contains(secret)
        );
    }
    child.terminate().unwrap();
}

fn raw_helper_call(socket: &Path, req: Request) -> helper::Reply {
    let mut stream = UnixStream::connect(socket).unwrap();
    stream
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    stream
        .set_write_timeout(Some(Duration::from_secs(1)))
        .unwrap();
    serde_json::to_writer(
        &mut stream,
        &helper::Envelope {
            protocol_version: helper::PROTOCOL_VERSION,
            lang: "en".into(),
            req,
        },
    )
    .unwrap();
    stream.write_all(b"\n").unwrap();
    let mut reader = std::io::BufReader::new(stream);
    let mut frame = String::new();
    reader.read_line(&mut frame).unwrap();
    serde_json::from_str(&frame).unwrap()
}

fn config_fixture_env(dir: &TempDirGuard, env: &mut EnvGuard) {
    env.set("CM_STATE_DIR", dir.path());
    env.set("CM_CONF", dir.path().join("cm.conf"));
    env.set("CM_VPN_ETC", dir.path().join("vpn-etc"));
    env.set("CM_VPN_HOME", dir.path().join("vpn-home"));
}

#[test]
fn config_save_merges_stale_fields_and_mirror_changes() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-config-merge").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let mut initial = cm::common::Config::defaults(vec![]);
    initial.save().unwrap();
    let mut first = cm::common::Config::load(vec![]).unwrap();
    let mut second = cm::common::Config::load(vec![]).unwrap();
    first.timeout = 30;
    first.mirrors.push("https://a.example/".into());
    first.save().unwrap();
    second.keep = 7;
    second.mirrors.push("https://b.example/".into());
    second.save().unwrap();
    let saved = cm::common::Config::load(vec![]).unwrap();
    assert_eq!((saved.timeout, saved.keep), (30, 7));
    assert_eq!(saved.mirrors, ["https://a.example/", "https://b.example/"]);
    assert_eq!(second.revision(), saved.revision());
}

#[test]
fn two_helper_processes_preserve_distinct_config_fields() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-config-processes").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let socket_one = dir.path().join("one.sock");
    let socket_two = dir.path().join("two.sock");
    let mut one = spawn_helper(&dir, &socket_one, None);
    let mut two = spawn_helper(&dir, &socket_two, None);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let barrier_two = barrier.clone();
    let first = thread::spawn(move || {
        barrier.wait();
        raw_helper_call(
            &socket_one,
            Request::ConfigSet {
                key: "keep".into(),
                value: "7".into(),
            },
        )
    });
    let second = thread::spawn(move || {
        barrier_two.wait();
        raw_helper_call(
            &socket_two,
            Request::ConfigSet {
                key: "timeout".into(),
                value: "30".into(),
            },
        )
    });
    assert!(first.join().unwrap().ok);
    assert!(second.join().unwrap().ok);
    let saved = cm::common::Config::load(vec![]).unwrap();
    assert_eq!((saved.keep, saved.timeout), (7, 30));
    one.terminate().unwrap();
    two.terminate().unwrap();
}

#[test]
fn config_rejects_dns_collision_and_reports_pending_then_apply_failure() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-config-status").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let socket = dir.path().join("helper.sock");
    let mut child = spawn_helper(&dir, &socket, None);
    let pending = raw_helper_call(
        &socket,
        Request::ConfigSet {
            key: "vpn_port".into(),
            value: "7899".into(),
        },
    );
    assert!(pending.ok);
    let pending: helper::SettingsReply = serde_json::from_value(pending.data).unwrap();
    assert!(matches!(
        pending.runtime,
        helper::SettingsRuntime::Pending { .. }
    ));
    assert_eq!(pending.config.vpn_port, 7899);
    let before = std::fs::read(dir.path().join("cm.conf")).unwrap();
    let invalid = raw_helper_call(
        &socket,
        Request::ConfigSet {
            key: "vpn_port".into(),
            value: "1053".into(),
        },
    );
    assert!(!invalid.ok);
    assert_eq!(std::fs::read(dir.path().join("cm.conf")).unwrap(), before);
    let etc = dir.path().join("vpn-etc");
    let home = dir.path().join("vpn-home");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::create_dir_all(home.join("profiles")).unwrap();
    std::fs::write(etc.join("subs.json"), r#"{"active":"p1","list":[{"id":"p1","name":"fixture","url":"https://example.com/p","kind":"clash"}]}"#).unwrap();
    std::fs::write(home.join("profiles/p1.yaml"), "proxies:\n  - {name: n1, type: ss, server: 1.2.3.4, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Proxy, type: select, proxies: [n1]}\nrules:\n  - MATCH,Proxy\n").unwrap();
    std::fs::create_dir(home.join("config.yaml")).unwrap();
    let result = raw_helper_call(
        &socket,
        Request::ConfigSet {
            key: "vpn_port".into(),
            value: "7900".into(),
        },
    );
    assert!(result.ok, "{}", result.error);
    let result: helper::SettingsReply = serde_json::from_value(result.data).unwrap();
    assert!(matches!(
        result.runtime,
        helper::SettingsRuntime::SavedButNotApplied { .. }
    ));
    assert!(result.runtime_error().unwrap().contains("saved"));
    let disk = cm::common::Config::load(vec![]).unwrap();
    assert_eq!(disk.vpn_port, 7900);
    assert_eq!(disk.revision(), result.revision);
    child.terminate().unwrap();
}

#[test]
fn waiting_config_file_lock_does_not_block_status_or_cancel() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-config-lock").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let socket = dir.path().join("helper.sock");
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, None);
    let file_lock = cm::common::vpn_config_lock(true).unwrap();
    let worker_socket = socket.clone();
    let worker = thread::spawn(move || {
        raw_helper_call(
            &worker_socket,
            Request::ConfigSet {
                key: "keep".into(),
                value: "7".into(),
            },
        )
    });
    let deadline = Instant::now() + Duration::from_secs(2);
    let status = loop {
        let status: cm::summary::OpStatus = helper::call_as(&Request::Status).unwrap();
        if status.running {
            break status;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    let started = Instant::now();
    let canceled = raw_helper_call(
        &socket,
        Request::Cancel {
            operation_id: status.operation_id.unwrap(),
        },
    );
    assert_eq!(
        canceled.control_error,
        Some(helper::ControlErrorCode::Unsupported)
    );
    assert!(started.elapsed() < Duration::from_millis(500));
    drop(file_lock);
    assert!(worker.join().unwrap().ok);
    child.terminate().unwrap();
}

#[test]
fn concurrent_cli_and_helper_write_complete_latest_vpn_yaml() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-cli-helper-yaml").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let etc = dir.path().join("vpn-etc");
    let home = dir.path().join("vpn-home");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::create_dir_all(home.join("profiles")).unwrap();
    std::fs::write(etc.join("subs.json"), r#"{"active":"p1","list":[{"id":"p1","name":"fixture","url":"https://example.com/p","kind":"clash"}]}"#).unwrap();
    std::fs::write(home.join("profiles/p1.yaml"), "proxies:\n  - {name: n1, type: ss, server: 1.2.3.4, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Proxy, type: select, proxies: [n1]}\nrules:\n  - MATCH,Proxy\n").unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    std::fs::write(
        bin.join("systemctl"),
        "#!/bin/sh\nprintf 'inactive\\n'\nexit 0\n",
    )
    .unwrap();
    std::fs::set_permissions(
        bin.join("systemctl"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(bin.join("gsettings"), "#!/bin/sh\nexit 0\n").unwrap();
    std::fs::set_permissions(
        bin.join("gsettings"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    env.set(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    let socket = dir.path().join("helper.sock");
    let mut helper = spawn_helper(&dir, &socket, None);
    let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
    let other_barrier = barrier.clone();
    let writer = thread::spawn(move || {
        barrier.wait();
        raw_helper_call(
            &socket,
            Request::ConfigSet {
                key: "vpn_port".into(),
                value: "7901".into(),
            },
        )
    });
    other_barrier.wait();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_cm"));
    cli.args(["vpn", "proxy"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut cli = ChildGuard::spawn(&mut cli).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = cli.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    assert!(writer.join().unwrap().ok);
    let disk = cm::common::Config::load(vec![]).unwrap();
    assert_eq!(disk.vpn_port, 7901);
    assert!(!disk.vpn_tun);
    let yaml: serde_yaml::Value =
        serde_yaml::from_slice(&std::fs::read(home.join("config.yaml")).unwrap()).unwrap();
    assert_eq!(yaml["mixed-port"].as_u64(), Some(7901));
    assert_eq!(yaml["tun"]["enable"].as_bool(), Some(false));
    helper.terminate().unwrap();
}

#[test]
fn explicit_user_context_keeps_proxy_uids_and_environment_separate() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-user-context").unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let log = dir.path().join("users.log");
    std::fs::write(
        bin.join("runuser"),
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$CM_PROXY_TEST_LOG\"\nexit 0\n",
    )
    .unwrap();
    std::fs::write(bin.join("gsettings"), "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$XDG_RUNTIME_DIR\" \"$DBUS_SESSION_BUS_ADDRESS\" \"$*\" >> \"$CM_PROXY_TEST_LOG\"\nexit 0\n").unwrap();
    std::fs::write(
        bin.join("systemctl"),
        "#!/bin/sh\nprintf 'inactive\\n'\nexit 0\n",
    )
    .unwrap();
    for name in ["runuser", "gsettings", "systemctl"] {
        std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    let mut env = EnvGuard::new();
    env.set("CM_PROXY_TEST_LOG", &log);
    env.set(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    env.set("XDG_RUNTIME_DIR", "/fixture/ambient-other-user");
    env.set("DBUS_SESSION_BUS_ADDRESS", "fixture:ambient");
    let before: Vec<_> = [
        "SUDO_USER",
        "SUDO_UID",
        "XDG_RUNTIME_DIR",
        "DBUS_SESSION_BUS_ADDRESS",
    ]
    .map(|key| (key, std::env::var_os(key)))
    .into();
    let first = cm::common::UserContext::from_uid(cm::common::sys::euid()).unwrap();
    let second = cm::common::UserContext::from_uid(65534).unwrap();
    let config = cm::common::Config::defaults(vec![]);
    for user in [&first, &second] {
        assert!(
            matches!(cm::vpn::sysproxy(&config, Some(user)).unwrap(), cm::vpn::SysproxyStatus::Applied { uid, .. } if uid == user.uid)
        );
    }
    let recorded = std::fs::read_to_string(&log).unwrap();
    assert!(recorded.contains(&format!("/run/user/{}", first.uid)));
    assert!(recorded.contains("/run/user/65534"));
    assert!(!recorded.contains("ambient-other-user"));
    assert!(!recorded.contains("fixture:ambient"));
    for (key, value) in before {
        assert_eq!(std::env::var_os(key), value);
    }
    let before_log = std::fs::read(&log).unwrap();
    assert_eq!(
        cm::vpn::sysproxy(&config, None).unwrap(),
        cm::vpn::SysproxyStatus::BackgroundSkipped
    );
    assert_eq!(std::fs::read(&log).unwrap(), before_log);
}

#[test]
fn cli_and_inline_configset_use_same_peer_proxy_context_and_report_errors() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-peer-proxy").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let etc = dir.path().join("vpn-etc");
    let home = dir.path().join("vpn-home");
    std::fs::create_dir_all(&etc).unwrap();
    std::fs::create_dir_all(home.join("profiles")).unwrap();
    std::fs::write(etc.join("subs.json"), r#"{"active":"p1","list":[{"id":"p1","name":"fixture","url":"https://example.com/p","kind":"clash"}]}"#).unwrap();
    std::fs::write(home.join("profiles/p1.yaml"), "proxies:\n  - {name: n1, type: ss, server: 1.2.3.4, port: 443, cipher: aes-128-gcm, password: x}\nproxy-groups:\n  - {name: Proxy, type: select, proxies: [n1]}\nrules:\n  - MATCH,Proxy\n").unwrap();
    let bin = dir.path().join("bin");
    std::fs::create_dir(&bin).unwrap();
    let log = dir.path().join("proxy.log");
    env.set("CM_PROXY_TEST_LOG", &log);
    std::fs::write(bin.join("gsettings"), "#!/bin/sh\nprintf '%s|%s|%s\\n' \"$XDG_RUNTIME_DIR\" \"$DBUS_SESSION_BUS_ADDRESS\" \"$*\" >> \"$CM_PROXY_TEST_LOG\"\nif [ -f \"$CM_PROXY_TEST_LOG.fail\" ]; then echo fixture-dconf-failure >&2; exit 1; fi\nexit 0\n").unwrap();
    std::fs::write(
        bin.join("systemctl"),
        "#!/bin/sh\nprintf 'inactive\\n'\nexit 0\n",
    )
    .unwrap();
    for name in ["gsettings", "systemctl"] {
        std::fs::set_permissions(bin.join(name), std::fs::Permissions::from_mode(0o755)).unwrap();
    }
    env.set(
        "PATH",
        format!("{}:{}", bin.display(), std::env::var("PATH").unwrap()),
    );
    env.set("SUDO_USER", "ambient-wrong-user");
    env.set("SUDO_UID", "65534");
    let socket = dir.path().join("helper.sock");
    let mut helper = spawn_helper(&dir, &socket, None);
    let result = raw_helper_call(
        &socket,
        Request::ConfigSet {
            key: "vpn_port".into(),
            value: "7902".into(),
        },
    );
    assert!(result.ok);
    let inline = std::fs::read_to_string(&log).unwrap();
    std::fs::write(&log, "").unwrap();
    let mut cli = Command::new(env!("CARGO_BIN_EXE_cm"));
    cli.args(["vpn", "proxy"])
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    let mut child = ChildGuard::spawn(&mut cli).unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        if let Some(status) = child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    }
    let cli = std::fs::read_to_string(&log).unwrap();
    let uid = cm::common::sys::euid();
    let expected = format!("/run/user/{uid}|unix:path=/run/user/{uid}/bus");
    assert!(inline.lines().all(|line| line.starts_with(&expected)));
    assert!(cli.lines().all(|line| line.starts_with(&expected)));
    assert!(!inline.is_empty() && !cli.is_empty());
    std::fs::write(dir.path().join("proxy.log.fail"), "fail").unwrap();
    let result = raw_helper_call(
        &socket,
        Request::ConfigSet {
            key: "vpn_port".into(),
            value: "7903".into(),
        },
    );
    assert!(result.ok);
    let result: helper::SettingsReply = serde_json::from_value(result.data).unwrap();
    let error = result.runtime_error().unwrap();
    assert!(error.contains("user proxy failed"));
    assert!(error.contains("fixture-dconf-failure"));
    assert_eq!(result.config.vpn_port, 7903);
    assert!(home.join("config.yaml").is_file());
    assert_eq!(std::env::var("SUDO_USER").unwrap(), "ambient-wrong-user");
    assert_eq!(std::env::var("SUDO_UID").unwrap(), "65534");
    helper.terminate().unwrap();
}

#[test]
fn helper_subprocess_receives_peer_identity_without_ambient_daemon_user() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-subprocess-context").unwrap();
    let fake = dir.path().join("fake-cm");
    std::fs::write(&fake, "#!/bin/sh\nprintf '%s|%s|%s|%s\\n' \"$SUDO_UID\" \"$SUDO_USER\" \"$XDG_RUNTIME_DIR\" \"$DBUS_SESSION_BUS_ADDRESS\"\nexit 0\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = EnvGuard::new();
    env.set("SUDO_USER", "ambient-other");
    env.set("SUDO_UID", "65534");
    env.set("XDG_RUNTIME_DIR", "/fixture/ambient");
    env.set("DBUS_SESSION_BUS_ADDRESS", "fixture:ambient");
    let socket = dir.path().join("helper.sock");
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake));
    let started: helper::StartReply = helper::call_as(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    assert_eq!(
        wait_for_finished(&started.operation_id, Duration::from_secs(2)).last_exit,
        Some(0)
    );
    let replay: Vec<_> = helper::attach_events()
        .unwrap()
        .map(|frame| frame.unwrap())
        .take_while(|frame| !matches!(frame.event, Event::ReplayComplete))
        .collect();
    let user = cm::common::UserContext::from_uid(cm::common::sys::euid()).unwrap();
    let expected = format!(
        "{}|{}|/run/user/{}|unix:path=/run/user/{}/bus",
        user.uid, user.name, user.uid, user.uid
    );
    assert!(
        replay
            .iter()
            .any(|frame| matches!(&frame.event, Event::Line { text } if text == &expected))
    );
    assert!(!serde_json::to_string(&replay).unwrap().contains("ambient"));
    assert_eq!(std::env::var("SUDO_USER").unwrap(), "ambient-other");
    child.terminate().unwrap();
}

#[derive(Debug)]
struct Resources {
    threads: usize,
    fds: usize,
    children: Vec<u32>,
    rss_kib: usize,
    peak_kib: usize,
}

fn resources(pid: u32) -> Resources {
    let (threads, fds) = proc_counts(pid);
    let mut children = Vec::new();
    for task in std::fs::read_dir(format!("/proc/{pid}/task")).unwrap() {
        let path = task.unwrap().path().join("children");
        if let Ok(text) = std::fs::read_to_string(path) {
            children.extend(text.split_whitespace().map(|n| n.parse::<u32>().unwrap()));
        }
    }
    children.sort_unstable();
    children.dedup();
    let status = std::fs::read_to_string(format!("/proc/{pid}/status")).unwrap();
    let value = |key: &str| {
        status
            .lines()
            .find_map(|line| line.strip_prefix(key))
            .unwrap()
            .split_whitespace()
            .next()
            .unwrap()
            .parse()
            .unwrap()
    };
    Resources {
        threads,
        fds,
        children,
        rss_kib: value("VmRSS:"),
        peak_kib: value("VmHWM:"),
    }
}

fn wait_resource_baseline(pid: u32, baseline: &(usize, usize)) -> Resources {
    let deadline = Instant::now() + Duration::from_secs(4);
    loop {
        let current = resources(pid);
        if (current.threads, current.fds) == *baseline && current.children.is_empty() {
            return current;
        }
        assert!(
            Instant::now() < deadline,
            "resources did not settle: {current:?}; baseline={baseline:?}"
        );
        thread::sleep(Duration::from_millis(10));
    }
}

/// Process-level resource gate, with allocator warm-up and an absolute bound
/// on each wait. The helper only runs a fake executable inside a temporary dir.
#[test]
fn helper_overload_has_resource_plateau_and_returns_fds_threads_children() {
    use std::io::Read;
    use std::os::fd::AsRawFd;
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-resource-gate").unwrap();
    let socket = dir.path().join("helper.sock");
    let fake = dir.path().join("fake-cm");
    let line = "x".repeat(1024);
    std::fs::write(&fake, format!("#!/bin/sh\ntrap '' INT TERM\nwhile [ ! -e \"${{0%/*}}/go\" ]; do sleep 0.01; done\ni=0\nwhile [ $i -lt 8000 ]; do printf '%s\\n' '{line}'; i=$((i+1)); done\n: > \"${{0%/*}}/ready\"\nwhile :; do sleep 0.01; done\n")).unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake));
    drop(helper::attach_events().unwrap());
    let initial = proc_counts(child.id());
    // A connection may still be finishing; obtain a stable empty baseline.
    let deadline = Instant::now() + Duration::from_secs(3);
    let baseline = loop {
        let r = resources(child.id());
        if r.threads == 1 && r.children.is_empty() {
            break (r.threads, r.fds);
        }
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(10));
    };
    assert!(baseline.1 <= initial.1);
    let mut warm_peak = 0;
    let mut measured = Vec::new();
    for round in 0..7 {
        // Burst of incomplete requests is bounded before authorization.
        let mut partial = Vec::new();
        for _ in 0..32 {
            let mut connection = UnixStream::connect(&socket).unwrap();
            let _ = connection.write_all(b"{\"protocol_version\":2");
            partial.push(connection);
            let r = resources(child.id());
            assert!(r.threads <= baseline.0 + 8, "pre-auth thread quota: {r:?}");
            assert!(r.fds <= baseline.1 + 32, "pre-auth FD quota: {r:?}");
        }
        // Keep accepted partials open until the server's absolute request deadline.
        let deadline = Instant::now() + Duration::from_secs(4);
        for connection in &mut partial {
            connection
                .set_read_timeout(Some(deadline.saturating_duration_since(Instant::now())))
                .unwrap();
            let mut bytes = [0; 4096];
            loop {
                match connection.read(&mut bytes) {
                    Ok(0) => break,
                    Ok(_) => {}
                    Err(error) if error.kind() == std::io::ErrorKind::ConnectionReset => break,
                    Err(error) => panic!("partial request missed deadline: {error}"),
                }
                assert!(Instant::now() < deadline);
            }
        }
        drop(partial);
        wait_resource_baseline(child.id(), &baseline);
        let _ = std::fs::remove_file(dir.path().join("ready"));
        let _ = std::fs::remove_file(dir.path().join("go"));
        let started: helper::StartReply = helper::call_as(&Request::Start {
            args: vec!["check".into()],
        })
        .unwrap();
        let _cancel = OperationCancelGuard(started.operation_id.clone());
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut attached = 0;
        while attached < 100 {
            if let Ok(events) = helper::attach_events() {
                drop(events);
                attached += 1;
            }
            assert!(Instant::now() < deadline, "100 attach/drop missed deadline");
        }
        let mut slow = UnixStream::connect(&socket).unwrap();
        let buffer: libc::c_int = 4096;
        assert_eq!(
            // SAFETY: the option value pointer and length describe a live c_int.
            unsafe {
                libc::setsockopt(
                    slow.as_raw_fd(),
                    libc::SOL_SOCKET,
                    libc::SO_RCVBUF,
                    &buffer as *const _ as *const libc::c_void,
                    std::mem::size_of_val(&buffer) as libc::socklen_t,
                )
            },
            0
        );
        serde_json::to_writer(
            &mut slow,
            &helper::Envelope {
                protocol_version: helper::PROTOCOL_VERSION,
                lang: "en".into(),
                req: Request::Attach,
            },
        )
        .unwrap();
        slow.write_all(b"\n").unwrap();
        std::fs::write(dir.path().join("go"), b"go").unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.path().join("ready").exists() {
            let r = resources(child.id());
            assert!(
                r.threads <= baseline.0 + 10 && r.fds <= baseline.1 + 64,
                "running resource quota: {r:?}"
            );
            assert!(Instant::now() < deadline, "fake burst stalled");
            thread::sleep(Duration::from_millis(10));
        }
        assert!(
            !raw_helper_call(
                &socket,
                Request::Start {
                    args: vec!["check".into()]
                }
            )
            .ok
        );
        assert!(
            raw_helper_call(
                &socket,
                Request::Cancel {
                    operation_id: started.operation_id.clone()
                }
            )
            .ok
        );
        // Repeat cancel may accelerate termination, or see the terminal state.
        let again = raw_helper_call(
            &socket,
            Request::Cancel {
                operation_id: started.operation_id.clone(),
            },
        );
        assert!(again.ok || again.control_error == Some(helper::ControlErrorCode::StaleOperation));
        wait_for_finished(&started.operation_id, Duration::from_secs(3));
        drop(slow);
        wait_resource_baseline(child.id(), &baseline);
        drop(helper::attach_events().unwrap());
        let r = wait_resource_baseline(child.id(), &baseline);
        assert!(r.rss_kib < 64 * 1024, "absolute RSS budget: {r:?}");
        if round < 3 {
            warm_peak = r.peak_kib;
        } else {
            measured.push(r.peak_kib);
        }
    }
    assert!(
        measured.iter().all(|peak| *peak <= warm_peak + 16 * 1024),
        "allocator did not plateau: warm={warm_peak} KiB, later={measured:?}"
    );
    eprintln!(
        "resource gate: baseline={baseline:?}, warmed peak={warm_peak} KiB, measured peak={measured:?} KiB"
    );
    child.terminate().unwrap();
}

#[test]
fn concurrent_cli_language_and_helper_field_preserve_separate_state_files() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-cli-helper-fields").unwrap();
    let mut env = EnvGuard::new();
    config_fixture_env(&dir, &mut env);
    let untouched = dir.path().join("unrelated-state.json");
    std::fs::write(&untouched, b"{\"sentinel\":17}\n").unwrap();
    let socket = dir.path().join("helper.sock");
    let mut helper = spawn_helper(&dir, &socket, None);
    for round in 0..20 {
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(2));
        let other = barrier.clone();
        let socket = socket.clone();
        let writer = thread::spawn(move || {
            other.wait();
            raw_helper_call(
                &socket,
                Request::ConfigSet {
                    key: "keep".into(),
                    value: (round % 10 + 1).to_string(),
                },
            )
        });
        let code = if round % 2 == 0 { "ar" } else { "en" };
        let mut cli = Command::new(env!("CARGO_BIN_EXE_cm"));
        cli.args(["lang", code])
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        barrier.wait();
        let mut cli = ChildGuard::spawn(&mut cli).unwrap();
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = cli.try_wait().unwrap() {
                assert!(status.success());
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(5));
        }
        assert!(writer.join().unwrap().ok);
        let saved = cm::common::Config::load(vec![]).unwrap();
        assert_eq!(saved.lang, code);
        assert_eq!(saved.keep, round % 10 + 1);
        assert_eq!(std::fs::read(&untouched).unwrap(), b"{\"sentinel\":17}\n");
    }
    helper.terminate().unwrap();
}

#[test]
fn helper_restart_does_not_fabricate_success_for_untracked_operation() {
    let _isolation = fixtures::isolation_lock();
    let dir = TempDirGuard::new("cm-audit-helper-restart").unwrap();
    let fake = dir.path().join("fake-cm");
    std::fs::write(
        &fake,
        "#!/bin/sh\ntrap '' HUP\necho $$ > \"${0%/*}/operation.pid\"\nprintf 'ready\\n'\nexec /bin/sleep 60\n",
    )
    .unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o755)).unwrap();
    let socket = dir.path().join("helper.sock");
    let mut env = EnvGuard::new();
    env.set("CM_HELPER_SOCK", &socket);
    let mut child = spawn_helper(&dir, &socket, Some(&fake));
    let start: helper::StartReply = helper::call_as(&Request::Start {
        args: vec!["check".into()],
    })
    .unwrap();
    let deadline = Instant::now() + Duration::from_secs(2);
    while !dir.path().join("operation.pid").exists() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(5));
    }
    let pid = std::fs::read_to_string(dir.path().join("operation.pid"))
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    let mut operation = ProcessGuard(Some(pid));
    let events = helper::attach_events().unwrap();
    let (tx, rx) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut exited = false;
        for event in events {
            match event {
                Ok(frame) if matches!(frame.event, Event::Exit { .. }) => exited = true,
                Err(_) => break,
                _ => {}
            }
        }
        tx.send(exited).unwrap();
    });
    child.terminate().unwrap();
    assert!(
        !rx.recv_timeout(Duration::from_secs(2)).unwrap(),
        "helper crash invented an Exit result"
    );
    reader.join().unwrap();
    assert!(
        process_is_running(pid),
        "fixture should demonstrate operation outside helper"
    );
    // A killed listener leaves a socket inode; existence alone is not readiness.
    std::fs::remove_file(&socket).unwrap();
    let mut restarted = spawn_helper(&dir, &socket, Some(&fake));
    let status: cm::summary::OpStatus = helper::call_as(&Request::Status).unwrap();
    assert!(!status.running && status.operation_id.is_none() && status.last_exit.is_none());
    let cancel = raw_helper_call(
        &socket,
        Request::Cancel {
            operation_id: start.operation_id,
        },
    );
    assert_eq!(
        cancel.control_error,
        Some(helper::ControlErrorCode::StaleOperation)
    );
    operation.kill();
    restarted.terminate().unwrap();
}
