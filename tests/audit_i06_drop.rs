//! C09: a worker or an application runs with the caller's uid, gid and groups and cannot
//! get root back. Runs as uid 0 of a user namespace with subordinate uids mapped.

mod pack_support;

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use cm::controller::drop::{RunAs, run_as_for, spawn_as};

fn user() -> RunAs {
    RunAs {
        uid: 1,
        gid: 1,
        groups: vec![1],
    }
}

fn output(program: &str, args: &[&str], env: &BTreeMap<String, String>) -> String {
    let args: Vec<String> = args.iter().map(|arg| (*arg).to_owned()).collect();
    let mut child = spawn_as(user(), Path::new(program), &args, env).unwrap();
    let mut text = String::new();
    child
        .stdout
        .take()
        .unwrap()
        .read_to_string(&mut text)
        .unwrap();
    assert!(child.wait().unwrap().success(), "{program} {args:?}");
    text
}

#[test]
fn c09_child_runs_as_the_caller_and_cannot_return() {
    if pack_support::rerun_in_userns(
        "c09_child_runs_as_the_caller_and_cannot_return",
        "CM_I06_C09_INNER",
    ) {
        return;
    }
    let empty = BTreeMap::new();
    assert_eq!(output("/usr/bin/id", &["-u"], &empty).trim(), "1");
    assert_eq!(output("/usr/bin/id", &["-g"], &empty).trim(), "1");
    assert_eq!(output("/usr/bin/id", &["-G"], &empty).trim(), "1");
    let status = output("/bin/sh", &["-c", "cat /proc/self/status"], &empty);
    for line in [
        "CapEff:\t0000000000000000",
        "CapPrm:\t0000000000000000",
        "CapBnd:\t0000000000000000",
        "NoNewPrivs:\t1",
        "Uid:\t1\t1\t1\t1",
        "Gid:\t1\t1\t1\t1",
    ] {
        assert!(
            status.lines().any(|candidate| candidate == line),
            "{line}\n{status}"
        );
    }
    // The parent's environment does not reach the child.
    // SAFETY: the inner run executes this one test on one thread.
    unsafe { std::env::set_var("SECRET_TOKEN", "x") };
    let env = BTreeMap::from([("A".to_owned(), "1".to_owned())]);
    assert_eq!(output("/usr/bin/env", &[], &env).trim(), "A=1");
    // uid 0 of the namespace asks for itself: the group list comes from the system, not the caller.
    let root = run_as_for(0, 0).unwrap();
    assert_eq!((root.uid, root.gid), (0, 0));
    assert!(root.groups.contains(&0));
}
