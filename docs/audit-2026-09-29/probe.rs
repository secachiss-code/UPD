//! HISTORICAL BASELINE: targets commit 31d3b7c (2026-09-29), protocol v1.
//! Not a current acceptance gate; current assertions live in tests/audit_contracts.rs.
//! Read-only audit of public APIs; writes only beneath a unique temporary directory.
//! Build/run instructions and interpretation are in ../AUDIT-LUNA6-DAG.md.
use std::io::{BufRead, BufReader, Write};
use std::os::unix::net::UnixListener;
use std::sync::{Arc, Barrier};
use std::time::Duration;

fn main() {
    if std::env::args().nth(1).as_deref() == Some("helper-fixture") {
        std::process::exit(upd::helper::serve());
    }
    let dir = std::env::temp_dir().join(format!("upd-audit-{}", std::process::id()));
    std::fs::create_dir(&dir).unwrap();

    // A complete reply and two events are already waiting in the same socket read.
    let sock = dir.join("mock.sock");
    let listener = UnixListener::bind(&sock).unwrap();
    unsafe { std::env::set_var("UPD_HELPER_SOCK", &sock); }
    let server = std::thread::spawn(move || {
        let (mut s, _) = listener.accept().unwrap();
        let mut request = String::new();
        BufReader::new(&s).read_line(&mut request).unwrap();
        s.write_all(b"{\"ok\":true}\n{\"ev\":\"reset\",\"command\":\"check\",\"started\":1}\n{\"ev\":\"exit\",\"code\":0}\n").unwrap();
    });
    let events: Vec<_> = upd::helper::attach_events().unwrap().collect();
    server.join().unwrap();
    println!("framing: expected 2 events, received {}", events.len());

    let mut term = upd::helper::Term::default();
    term.feed(&vec![b'x'; 2 * 1024 * 1024]);
    println!("term: partial bytes after 2 MiB without newline = {}", term.partial.len());
    let mut term = upd::helper::Term::default();
    let lines = term.feed(b"\xffhello\n");
    println!("utf8: expected replacement + hello as a complete line, got {lines:?}, partial {:?}", term.partial);

    let barrier = Arc::new(Barrier::new(16));
    let mut workers = vec![];
    for n in 0..16 {
        let barrier = barrier.clone();
        let path = dir.join(format!("atomic-{n}"));
        workers.push(std::thread::spawn(move || {
            let data = vec![b'A' + n as u8; 64 * 1024];
            barrier.wait();
            let result = upd::common::atomic_write(&path, &data, 0o600);
            let matches = std::fs::read(&path).is_ok_and(|b| b == data);
            (result.is_err(), matches)
        }));
    }
    let results: Vec<_> = workers.into_iter().map(|h| h.join().unwrap()).collect();
    println!("atomic: {} errors, {} files with missing/wrong content out of 16", results.iter().filter(|x| x.0).count(), results.iter().filter(|x| !x.1).count());

    // No system operations: a separate helper fixture receives only Attach requests.
    let sock = dir.join("helper.sock");
    let mut child = std::process::Command::new(std::env::current_exe().unwrap())
        .arg("helper-fixture")
        .env("UPD_HELPER_SOCK", &sock)
        .env("UPD_STATE_DIR", &dir)
        .env("UPD_HELPER_ALLOW", "1")
        .spawn().unwrap();
    for _ in 0..100 {
        if sock.exists() { break; }
        std::thread::sleep(Duration::from_millis(20));
    }
    unsafe { std::env::set_var("UPD_HELPER_SOCK", &sock); }
    let counts = || {
        let tasks = std::fs::read_dir(format!("/proc/{}/task", child.id())).unwrap().count();
        let fds = std::fs::read_dir(format!("/proc/{}/fd", child.id())).unwrap().count();
        (tasks, fds)
    };
    let before = counts();
    let mut attach_errors = 0;
    for _ in 0..20 {
        match upd::helper::attach_events() {
            Ok(events) => drop(events),
            Err(_) => attach_errors += 1,
        }
    }
    std::thread::sleep(Duration::from_millis(500));
    println!("closed attaches: before {before:?}, after {:?} (threads, fds); attach errors {attach_errors}", counts());
    child.kill().unwrap();
    child.wait().unwrap();
    unsafe { std::env::remove_var("UPD_HELPER_SOCK"); }
    std::fs::remove_dir_all(dir).unwrap();
}
