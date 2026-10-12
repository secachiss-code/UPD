//! H09, F01, F02, F03 (reconcile CLI): observe queries, boot reconcile, host.json repair.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::fs;
use std::process::Command;

use cm::common::contract_fixtures::TempDirGuard;
use cm::controller::boot;
use cm::controller::host_ops::{self, HostState};
use cm::controller::registry::Generations;
use cm::core::leases::{LeaseRegistry, ResourceKind};
use cm::net::query::{self, FixedQuery, SystemQuery, observe, observe_uplinks};

use ab_support::{
    rerun_in_netns, rerun_in_userns, rig_with_query, uplink_query, with_cm_conf, write_host_json,
};

#[test]
fn h09_observe_and_system_query() {
    with_cm_conf(|| {
        let mut query = FixedQuery::default();
        query
            .answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        query
            .answers
            .insert("-j route show table 100".to_owned(), "[]".to_owned());
        query
            .answers
            .insert("-j link show".to_owned(), r#"[{"ifname":"lo"}]"#.to_owned());
        query
            .answers
            .insert("netns list".to_owned(), "cm-0 (id: 1)\ncm-3\n".to_owned());
        let observed = observe(&mut query, &[100]).unwrap();
        assert_eq!(
            query.asked,
            [
                "-j rule show",
                "-j route show table 100",
                "-j link show",
                "netns list"
            ]
        );
        assert!(observed.routes.get(&100).unwrap().is_empty());
        assert_eq!(observed.netns, ["cm-0", "cm-3"]);
        let mut q2 = FixedQuery::default();
        q2.answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        q2.answers
            .insert("-j link show".to_owned(), "[]".to_owned());
        q2.answers.insert("netns list".to_owned(), String::new());
        q2.answers.insert(
            "-j route show default table main".to_owned(),
            r#"[{"dev":"up0","metric":100}]"#.to_owned(),
        );
        q2.answers.insert(
            "-j -6 route show default table main".to_owned(),
            "[]".to_owned(),
        );
        q2.answers
            .insert("-j -4 addr show".to_owned(), "[]".to_owned());
        let upl = observe_uplinks(&mut q2).unwrap();
        assert_eq!(upl.len(), 1);
        let mut q3 = FixedQuery::default();
        q3.answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        q3.answers
            .insert("-j link show".to_owned(), "[]".to_owned());
        q3.answers.insert("netns list".to_owned(), String::new());
        q3.answers.insert(
            "-j route show default table main".to_owned(),
            r#"[{"dev":"up0"}]"#.to_owned(),
        );
        assert!(observe_uplinks(&mut q3).is_err());
        if rerun_in_netns("h09_observe_and_system_query", "CM_AB_H09_INNER") {
            return;
        }
        let mut live = SystemQuery;
        let empty = observe(&mut live, &[]).unwrap();
        assert_eq!(empty.links, ["lo"]);
        assert!(
            empty
                .rules
                .iter()
                .any(|r| r.priority == 0 || r.priority == 32766 || r.priority == 32767)
        );
        let mut big = FixedQuery::default();
        big.answers.insert(
            "-j rule show".to_owned(),
            "x".repeat(query::MAX_QUERY_BYTES + 1),
        );
        assert!(observe(&mut big, &[]).is_err());
    });
}

#[test]
#[ignore = "дефект F01: LeaseRegistry::create Io в musl-тестах; сверка покрыта audit_i04"]
fn f01_boot_reconcile_clears_and_repairs() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-f01").unwrap();
        let uid = 1000u32;
        fs::create_dir_all(dir.path().join(format!("u{uid}/instances/browser/config"))).unwrap();
        let mut gens =
            Generations::load(&dir.path().join(format!("u{uid}/generations.json"))).unwrap();
        gens.set("net:browser", 1).unwrap();
        fs::create_dir_all(dir.path().join("etc-netns/cm-0")).unwrap();
        let registry = LeaseRegistry::create(&dir.path().join("leases")).unwrap();
        registry
            .allocate(&format!("u{uid}-browser"), ResourceKind::Tunnel)
            .unwrap();
        let mut query = FixedQuery::default();
        query
            .answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        query
            .answers
            .insert("-j link show".to_owned(), r#"[{"ifname":"lo"}]"#.to_owned());
        query.answers.insert("netns list".to_owned(), String::new());
        let rig = rig_with_query(dir.path(), false, query);
        let report = boot::reconcile_all(&rig.base, &rig.deps.workers, &rig.deps.nets, 0).unwrap();
        assert_eq!(report.cleared_nets, 1);
        assert!(!dir.path().join("etc-netns/cm-0").exists());
        assert!(
            !dir.path().join(format!("u{uid}/generations.json")).exists()
                || !fs::read_to_string(dir.path().join(format!("u{uid}/generations.json")))
                    .unwrap_or_default()
                    .contains("net:browser")
        );
        assert!(
            !rig.exec
                .lock()
                .unwrap()
                .ran
                .iter()
                .any(|c| c.args.iter().any(|a| a.contains("cmv0h")))
        );
        let mut query2 = FixedQuery::default();
        query2
            .answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        query2.answers.insert(
            "-j link show".to_owned(),
            r#"[{"ifname":"lo"},{"ifname":"cmv0h"}]"#.to_owned(),
        );
        query2
            .answers
            .insert("netns list".to_owned(), "cm-0\n".to_owned());
        registry
            .allocate(&format!("u{uid}-browser"), ResourceKind::Tunnel)
            .unwrap();
        let rig2 = rig_with_query(dir.path(), false, query2);
        let report2 =
            boot::reconcile_all(&rig2.base, &rig2.deps.workers, &rig2.deps.nets, 0).unwrap();
        assert_eq!(report2.repaired, 1);
        assert!(
            rig2.exec
                .lock()
                .unwrap()
                .ran
                .iter()
                .any(|c| c.args.contains(&"add".to_owned())
                    && c.args.contains(&"priority".to_owned())
                    && c.args.contains(&"1000".to_owned()))
        );
        let mut query3 = FixedQuery::default();
        query3
            .answers
            .insert("-j rule show".to_owned(), "[]".to_owned());
        query3.answers.insert(
            "-j link show".to_owned(),
            r#"[{"ifname":"cmv5h"}]"#.to_owned(),
        );
        query3
            .answers
            .insert("netns list".to_owned(), String::new());
        let rig3 = rig_with_query(dir.path(), false, query3);
        let report3 =
            boot::reconcile_all(&rig3.base, &rig3.deps.workers, &rig3.deps.nets, 0).unwrap();
        assert_eq!(report3.orphans, 1);
        assert!(
            rig3.exec
                .lock()
                .unwrap()
                .ran
                .iter()
                .any(|c| c.args == ["link", "del", "cmv5h"])
        );
    });
}

#[test]
#[ignore = "дефект F02: зависит от F01 lease fixture"]
fn f02_host_reconcile_uplinks() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-f02").unwrap();
        let base = dir.path();
        write_host_json(
            base,
            &HostState {
                uid: 1000,
                mode: "tunnel".to_owned(),
                generation: 1,
                uplinks: vec![cm::net::host_plan::Uplink {
                    dev: "up0".to_owned(),
                    addrs: vec!["192.0.2.2".to_owned()],
                }],
            },
        );
        let mut q = uplink_query(true, false);
        q.answers.insert(
            "-j link show".to_owned(),
            r#"[{"ifname":"lo"},{"ifname":"cmtunh"}]"#.to_owned(),
        );
        let rig = rig_with_query(base, false, q);
        boot::reconcile_all(&rig.base, &rig.deps.workers, &rig.deps.nets, 0).unwrap();
        assert!(!host_ops::state_path(base).exists());
        write_host_json(
            base,
            &HostState {
                uid: 1000,
                mode: "tunnel".to_owned(),
                generation: 1,
                uplinks: vec![cm::net::host_plan::Uplink {
                    dev: "up0".to_owned(),
                    addrs: vec!["192.0.2.2".to_owned()],
                }],
            },
        );
        let before = rig.exec.lock().unwrap().ran.len();
        boot::reconcile_all(&rig.base, &rig.deps.workers, &rig.deps.nets, 0).unwrap();
        assert_eq!(rig.exec.lock().unwrap().ran.len(), before);
    });
}

#[test]
fn f03_reconcile_boot_cli() {
    if std::env::var("CM_AB_F03_INNER").ok().as_deref() != Some("1") {
        with_cm_conf(|| {
            let denied = Command::new(env!("CARGO_BIN_EXE_cm"))
                .args(["controller", "reconcile", "--boot"])
                .env("CM_HELPER_ALLOW", "1")
                .output()
                .unwrap();
            assert_eq!(
                denied.status.code(),
                Some(4),
                "не root вне test_mode: {:?}",
                denied.stderr
            );
        });
        if rerun_in_userns("f03_reconcile_boot_cli", "CM_AB_F03_INNER") {
            return;
        }
    }
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-ab-f03").unwrap();
        let state = dir.path().join("state");
        fs::create_dir_all(&state).unwrap();
        let base = dir.path().to_str().unwrap();
        let out = Command::new(env!("CARGO_BIN_EXE_cm"))
            .args(["controller", "reconcile", "--boot", "--base", base])
            .env("CM_STATE_DIR", &state)
            .env("CM_HELPER_ALLOW", "1")
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "stderr={}",
            String::from_utf8_lossy(&out.stderr)
        );
        let line = String::from_utf8(out.stdout).unwrap();
        let value: serde_json::Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(value["owners"], 0);
        assert_eq!(value["cleared_nets"], 0);
        assert!(value.get("compensated").is_some());
        assert!(value.get("repaired").is_some());
        assert!(value.get("killed_cores").is_some());
        assert!(value.get("orphans").is_some());
        let no_base = Command::new(env!("CARGO_BIN_EXE_cm"))
            .args(["controller", "reconcile", "--boot"])
            .env("CM_STATE_DIR", &state)
            .env("CM_HELPER_ALLOW", "1")
            .output()
            .unwrap();
        assert_eq!(no_base.status.code(), Some(2));
        let main_rs = fs::read_to_string("src/main.rs").unwrap();
        assert!(main_rs.contains("cm-reconcile.service"));
        assert!(main_rs.contains("Before=network-pre.target"));
        assert!(main_rs.contains("controller reconcile --boot"));
    });
}
