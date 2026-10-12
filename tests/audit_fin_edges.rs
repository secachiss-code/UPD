//! L1 checks for the selected R.Z, C.O1, I.H1, and I.T1 edges.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::collections::BTreeMap;
use std::collections::VecDeque;
use std::fs;
use std::process::Command;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use cm::common::contract_fixtures::TempDirGuard;
use cm::common::sys::euid;
use cm::controller::core_ops::{AdapterFactory, Workers};
use cm::controller::dispatch::{Deps, OpSlots};
use cm::controller::drop::RunAs;
use cm::controller::host_ops::HostState;
use cm::controller::owner::owned;
use cm::controller::protocol::{Op, OpClass, ReplyData};
use cm::controller::registry::Generations;
use cm::controller::teardown::teardown_all;
use cm::controller::watch::Watch;
use cm::core::adapter::{
    CoreAdapter, CoreCapabilities, CoreConfig, CoreError, CoreReadiness, CoreStatistics,
};
use cm::core::{FakeAdapter, FakeOp, LeaseRegistry, ResourceKind};
use cm::net::host::{self, HostFacts, Integration, Verdict};
use cm::net::host_commands::host_destroy_commands;
use cm::net::host_firewall::{host_firewall_commands, render_host_table};
use cm::net::host_plan::{Uplink, host_net};
use cm::net::observe::{Observed, ObservedRoute, ObservedRule};
use cm::net::policy::{self, FailureMode, Lifetime, TunnelPolicy};
use cm::net::query::FixedQuery;
use cm::net::{RecordingExec, TunnelNet};
use cm::profiles::{FakeIpPolicy, Ipv6Policy};

use ab_support::{cmd_lines, rig_with_query, with_cm_conf, write_config, write_host_json};

const CORE_CONFIG: &str = r#"{"mode":"direct","ipv6":false}"#;

struct TestStateEnv(Option<std::ffi::OsString>);

impl TestStateEnv {
    fn new(base: &std::path::Path) -> Self {
        let old = std::env::var_os("CM_STATE_DIR");
        // SAFETY: tests that change process environment run under ab_support::with_cm_conf's lock.
        unsafe {
            std::env::set_var("CM_STATE_DIR", base.join("state"));
        }
        Self(old)
    }
}

impl Drop for TestStateEnv {
    fn drop(&mut self) {
        // SAFETY: paired with the serialized environment update in `new`.
        unsafe {
            if let Some(value) = self.0.take() {
                std::env::set_var("CM_STATE_DIR", value);
            } else {
                std::env::remove_var("CM_STATE_DIR");
            }
        }
    }
}

type Script = Arc<Mutex<VecDeque<(FakeOp, CoreError)>>>;

struct WatchFactory {
    made: AtomicUsize,
    script: Script,
    lease_dir: std::path::PathBuf,
}

struct WatchAdapter {
    inner: FakeAdapter,
    script: Script,
}

impl CoreAdapter for WatchAdapter {
    fn capabilities(&self) -> CoreCapabilities {
        self.inner.capabilities()
    }
    fn validate(&mut self, config: &CoreConfig) -> Result<(), CoreError> {
        self.inner.validate(config)
    }
    fn start(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        take_watch_failure(&self.script, FakeOp::Start)?;
        self.inner.start(config)
    }
    fn stop(&mut self) -> Result<(), CoreError> {
        take_watch_failure(&self.script, FakeOp::Stop)?;
        self.inner.stop()
    }
    fn reload(&mut self, config: &CoreConfig) -> Result<CoreReadiness, CoreError> {
        self.inner.reload(config)
    }
    fn health(&mut self) -> Result<CoreReadiness, CoreError> {
        take_watch_failure(&self.script, FakeOp::Health)?;
        self.inner.health()
    }
    fn statistics(&mut self) -> Result<CoreStatistics, CoreError> {
        self.inner.statistics()
    }
    fn alive(&mut self) -> bool {
        self.inner.alive()
    }
    fn set_tunnel_net(&mut self, tunnel: TunnelNet) {
        self.inner.set_tunnel_net(tunnel);
    }
    fn set_dns_runtime(&mut self, dns: cm::net::dns::DnsRuntime) {
        self.inner.set_dns_runtime(dns);
    }
}

fn take_watch_failure(script: &Script, op: FakeOp) -> Result<(), CoreError> {
    let mut script = script.lock().unwrap();
    if let Some(index) = script.iter().position(|(candidate, _)| *candidate == op) {
        return Err(script.remove(index).unwrap().1);
    }
    Ok(())
}

impl AdapterFactory for WatchFactory {
    fn make(
        &self,
        _owned: &cm::controller::owner::Owned,
        _run_as: &RunAs,
        _tunnel: Option<&TunnelNet>,
    ) -> Box<dyn CoreAdapter + Send> {
        self.made.fetch_add(1, Ordering::SeqCst);
        Box::new(WatchAdapter {
            inner: FakeAdapter::new(),
            script: Arc::clone(&self.script),
        })
    }
    fn lease_dir(&self) -> Option<&std::path::Path> {
        Some(&self.lease_dir)
    }
}

struct WatchRig {
    deps: Deps,
    factory: Arc<WatchFactory>,
    exec: Arc<Mutex<RecordingExec>>,
    query: Arc<Mutex<FixedQuery>>,
}

fn watch_rig(base: &std::path::Path) -> WatchRig {
    watch_rig_with_query(base, FixedQuery::default())
}

fn watch_rig_with_query(base: &std::path::Path, initial_query: FixedQuery) -> WatchRig {
    let lease_dir = base.join("leases");
    let factory = Arc::new(WatchFactory {
        made: AtomicUsize::new(0),
        script: Arc::new(Mutex::new(VecDeque::new())),
        lease_dir: lease_dir.clone(),
    });
    let exec = Arc::new(Mutex::new(RecordingExec::default()));
    let query = Arc::new(Mutex::new(initial_query));
    let net_query: Arc<Mutex<dyn cm::net::query::NetQuery + Send>> = query.clone();
    let nets = Arc::new(cm::controller::net_ops::NetCtx {
        exec: exec.clone(),
        query: net_query,
        lease_dir,
        etc_root: base.join("etc-netns"),
    });
    let workers = Arc::new(Workers::new(factory.clone()));
    workers.set_net(Arc::clone(&nets));
    let deps = Deps {
        base: base.to_owned(),
        authorizer: Arc::new(ab_support::ScriptedAuth {
            deny: false,
            actions: Mutex::new(Vec::new()),
        }),
        workers,
        slots: Arc::new(OpSlots::new()),
        now: || 1_000,
        nets,
    };
    WatchRig {
        deps,
        factory,
        exec,
        query,
    }
}

fn start_fake_worker(
    rig: &WatchRig,
    base: &std::path::Path,
    uid: u32,
    instance: &str,
    generation: u64,
    txn: &str,
) -> cm::controller::owner::Owned {
    let owner = owned(base, uid, instance).unwrap();
    write_config(base, uid, instance, generation, CORE_CONFIG);
    rig.deps
        .workers
        .start(
            &owner,
            &RunAs {
                uid,
                gid: uid,
                groups: vec![uid],
            },
            txn,
            "digest",
            generation,
            1,
        )
        .unwrap();
    owner
}

fn seed_tunnel(
    rig: &WatchRig,
    owner: &cm::controller::owner::Owned,
    generation: u64,
) -> cm::core::leases::Lease {
    fs::create_dir_all(&rig.deps.nets.lease_dir).unwrap();
    let registry = match LeaseRegistry::open(&rig.deps.nets.lease_dir) {
        Ok(registry) => registry,
        Err(_) => LeaseRegistry::create(&rig.deps.nets.lease_dir).unwrap(),
    };
    let lease = registry
        .allocate(
            &format!("u{}-{}", owner.uid, owner.instance.as_str()),
            ResourceKind::Tunnel,
        )
        .unwrap();
    let mut generations = Generations::load(&owner.generations_path()).unwrap();
    generations
        .set(&format!("net:{}", owner.instance.as_str()), generation)
        .unwrap();
    fs::create_dir_all(rig.deps.nets.etc_root.join(format!("cm-{}", lease.value))).unwrap();
    fs::write(
        rig.deps
            .nets
            .etc_root
            .join(format!("cm-{}/resolv.conf", lease.value)),
        "nameserver 198.18.0.2\n",
    )
    .unwrap();
    lease
}

fn set_refcount_policy(owner: &cm::controller::owner::Owned) {
    let path = cm::controller::owner::policy_path(owner);
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, br#"{"lifetime":"refcount"}"#).unwrap();
}

fn install_watch(rig: &WatchRig, base: &std::path::Path) -> Arc<Mutex<Watch>> {
    let watch = Arc::new(Mutex::new(Watch::with_proc_root(base.join("proc"))));
    rig.deps.workers.set_watch(Arc::clone(&watch));
    watch
}

fn facts() -> HostFacts {
    HostFacts {
        nft: true,
        links: Vec::new(),
        rules: Vec::new(),
        integration: Integration {
            network_manager: false,
            firewalld: false,
            resolved: false,
        },
        uplinks: 1,
        legacy_unit_active: false,
        controller_socket: true,
        host_mode: "off".to_owned(),
        rp_filter_all: Some(0),
        rp_filter_default: Some(2),
    }
}

fn ensure_instance(base: &std::path::Path, uid: u32, name: &str) {
    fs::create_dir_all(base.join(format!("u{uid}")).join("instances").join(name)).unwrap();
}

fn query_with(answers: &[(&str, &str)]) -> FixedQuery {
    FixedQuery {
        answers: answers
            .iter()
            .map(|(key, value)| ((*key).to_owned(), (*value).to_owned()))
            .collect(),
        asked: Vec::new(),
    }
}

fn empty_host_query() -> FixedQuery {
    query_with(&[
        ("-j link show", "[]"),
        ("-j rule show", "[]"),
        ("-j -6 rule show", "[]"),
        ("-j route show default table main", "[]"),
        ("-j -6 route show default table main", "[]"),
        ("-j -4 addr show", "[]"),
    ])
}

fn watch_host_query() -> FixedQuery {
    query_with(&[
        (
            "-j route show default table main",
            r#"[{"dev":"up0","metric":100}]"#,
        ),
        ("-j -6 route show default table main", "[]"),
        (
            "-j -4 addr show",
            r#"[{"ifname":"up0","addr_info":[{"local":"192.0.2.2","scope":"global"}]}]"#,
        ),
        (
            "-j rule show",
            r#"[{"priority":1900,"table":"main"},{"priority":1950,"table":"main"},{"priority":1990,"table":"main"},{"priority":2000,"table":"90"},{"priority":2001,"table":"91"}]"#,
        ),
        ("-j route show table 90", "[]"),
        (
            "-j route show table 91",
            r#"[{"dst":"default","dev":"cmsink"}]"#,
        ),
        ("-j link show", r#"[{"ifname":"cmsink"}]"#),
        ("netns list", ""),
        (
            "-j -6 rule",
            r#"[{"priority":1900,"table":"main"},{"priority":1990,"table":"main"},{"priority":2001,"table":"91"}]"#,
        ),
        (
            "-j -6 route show table 91",
            r#"[{"dst":"default","dev":"cmsink"}]"#,
        ),
        ("list table ip cm_host", "table ip cm_host"),
    ])
}

fn uplinks() -> Vec<Uplink> {
    vec![Uplink {
        dev: "up0".to_owned(),
        addrs: vec!["192.0.2.2".to_owned()],
    }]
}

fn host_routes6(host: &cm::net::host_plan::HostNet) -> Vec<ObservedRoute> {
    vec![ObservedRoute {
        dst: "default".to_owned(),
        dev: Some(host.sink.clone()),
        kind: None,
        metric: None,
    }]
}

fn host_observation(
    host: &cm::net::host_plan::HostNet,
    uplinks: &[Uplink],
) -> (Observed, Vec<ObservedRule>) {
    let mut rules = vec![
        ObservedRule {
            priority: 1900,
            table: "main".to_owned(),
            iif: None,
        },
        ObservedRule {
            priority: 1950,
            table: "main".to_owned(),
            iif: None,
        },
        ObservedRule {
            priority: 1990,
            table: "main".to_owned(),
            iif: None,
        },
        ObservedRule {
            priority: host.rule_tun,
            table: host.table.to_string(),
            iif: None,
        },
        ObservedRule {
            priority: host.rule_sink,
            table: host.sink_table.to_string(),
            iif: None,
        },
    ];
    rules.truncate(uplinks.len() + uplinks.iter().map(|item| item.addrs.len()).sum::<usize>() + 3);
    let routes = BTreeMap::from([
        (
            host.table,
            vec![ObservedRoute {
                dst: "default".to_owned(),
                dev: Some(host.tun.clone()),
                kind: None,
                metric: None,
            }],
        ),
        (
            host.sink_table,
            vec![ObservedRoute {
                dst: "default".to_owned(),
                dev: Some(host.sink.clone()),
                kind: None,
                metric: None,
            }],
        ),
    ]);
    let observed = Observed {
        rules: std::mem::take(&mut rules),
        routes,
        links: vec![host.tun.clone(), host.sink.clone()],
        netns: Vec::new(),
    };
    let rules6 = vec![
        ObservedRule {
            priority: 1900,
            table: "main".to_owned(),
            iif: None,
        },
        ObservedRule {
            priority: 1990,
            table: "main".to_owned(),
            iif: None,
        },
        ObservedRule {
            priority: host.rule_sink,
            table: host.sink_table.to_string(),
            iif: None,
        },
    ];
    (observed, rules6)
}

#[test]
fn rp01_policy_accepts_only_bounded_objects() {
    with_cm_conf(|| {
        // [RP01.1]
        for input in [b"[]".as_slice(), b"null", b"1", b"\"x\""] {
            assert_eq!(policy::parse(input), Err(cm::net::NetError::Unsupported));
        }
        // [RP01.2]
        assert_eq!(
            policy::parse(b"{}").unwrap(),
            TunnelPolicy {
                ipv6: Ipv6Policy::Block,
                fake_ip: FakeIpPolicy::Forbidden,
                lifetime: Lifetime::Pinned,
                failure: FailureMode::Block,
            }
        );
        // [RP01.3]
        let over_limit = vec![b' '; policy::MAX_POLICY_BYTES + 1];
        assert_eq!(
            policy::parse(&over_limit),
            Err(cm::net::NetError::Unsupported)
        );
        // [RP01.4]
        assert_eq!(
            policy::parse(br#"{"lifetime":"refcount","extra":1}"#),
            Err(cm::net::NetError::Unsupported)
        );
    });
}

#[test]
fn rw01_auto_transaction_keys_are_bounded_and_unique_inputs() {
    with_cm_conf(|| {
        let max = cm::controller::watch::auto_txn("revert", i64::MAX, u32::MAX);
        // [RW01.3]
        assert_eq!(max.len(), 42);
        assert!(
            max.bytes()
                .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'-')
        );
        assert_eq!(
            cm::controller::watch::auto_txn("stop", -5, 1),
            "auto-stop-0-1"
        );
    });
}

#[test]
fn rz01_ab_stand_waits_with_a_finite_hundred_ms_poll() {
    with_cm_conf(|| {
        let source = include_str!("ab_stand.py");
        let app = source
            .split("def app(")
            .nth(1)
            .unwrap()
            .split("def host_http(")
            .next()
            .unwrap();
        // [RZ01.3]
        assert!(
            app.contains("for _ in range(wait * 10):")
                && app.contains("time.sleep(0.1)")
                && app.contains("return \"NO OUTPUT\"")
        );
    });
}

#[test]
#[ignore = "дефект RZ01.1: AB linter оставляет 326 значений без проверки"]
fn rz01_ab_linter_requires_full_coverage() {
    with_cm_conf(|| {
        let output = Command::new("python3")
            .args([
                "docs/design/cm-network-manager/tools/test_lint.py",
                "--dags",
                "AB",
            ])
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        // [RZ01.1]
        assert!(
            output.status.success()
                && text.contains(
                    "значений 326, проверено 326, заблокировано 0, без проверки 0, замечаний 0"
                )
        );
    });
}

#[test]
#[ignore = "дефект RZ01.2: D01 и U02 остаются отключёнными"]
fn rz01_ab_audits_have_no_ignored_tests() {
    with_cm_conf(|| {
        let ignore_marker = ["#", "[ignore"].concat();
        let clean = fs::read_dir("tests")
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry.file_name().to_string_lossy().starts_with("audit_ab_")
                    && entry.file_name().to_string_lossy().ends_with(".rs")
            })
            .all(|entry| {
                !fs::read_to_string(entry.path())
                    .unwrap()
                    .contains(&ignore_marker)
            });
        let stand = fs::read_to_string("tests/ab_stand.py").unwrap();
        // [RZ01.2]
        assert!(clean && !stand.contains("defect("));
    });
}

#[test]
fn rz01_ab_blocked_values_are_bounded_l2_entries() {
    with_cm_conf(|| {
        let blocked: Vec<serde_json::Value> =
            serde_json::from_slice(include_bytes!("fixtures/ab/blocked.json")).unwrap();
        let dag: serde_json::Value = serde_json::from_slice(include_bytes!(
            "../docs/design/cm-network-manager/AB-DAG.json"
        ))
        .unwrap();
        let entries_valid = blocked.iter().all(|item| {
            let edge = item["edge"].as_str().unwrap_or_default();
            let n = item["n"].as_u64().unwrap_or_default() as usize;
            let specified = dag["edges"]
                .as_array()
                .unwrap()
                .iter()
                .find(|candidate| candidate["id"] == edge);
            let Some(specified) = specified else {
                return false;
            };
            specified["level"] == "L2"
                && n > 0
                && n <= specified["values"].as_array().unwrap().len()
                && item["reason"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
                && item["driver"]
                    .as_str()
                    .is_some_and(|value| !value.is_empty())
        });
        // [RZ01.5]
        assert!(blocked.len() <= 12 && entries_valid);
    });
}

#[test]
fn rw01_tick_stops_refcount_tunnels_repeatedly() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw01").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        let owner = start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw01-first-start");
        set_refcount_policy(&owner);
        let first_lease = seed_tunnel(&rig, &owner, 1);
        rig.deps.workers.note_launch(uid, "browser");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 0);
        rig.exec.lock().unwrap().ran.clear();
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 5_000);
        let first_commands = cmd_lines(&rig.exec.lock().unwrap().ran);
        let tunnel = cm::net::tunnel_net(first_lease.value.parse().unwrap(), uid).unwrap();
        let mut expected = cmd_lines(&cm::net::commands::destroy_commands(&tunnel));
        expected.extend(cmd_lines(&cm::net::replace_commands(&[])));
        // [RW01.1]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Stopped, 0))
        );
        assert_eq!(first_commands, expected);
        assert!(
            LeaseRegistry::open(&rig.deps.nets.lease_dir)
                .unwrap()
                .list()
                .unwrap()
                .is_empty()
        );

        let owner = start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw01-second-start");
        let _second_lease = seed_tunnel(&rig, &owner, 1);
        rig.deps.workers.note_launch(uid, "browser");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 6_000);
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 11_000);
        let states = cm::controller::journal::Journal::open(&owner.journal_path())
            .unwrap()
            .replay()
            .unwrap();
        let stop_ids: std::collections::BTreeSet<_> = states
            .iter()
            .filter(|state| {
                state.status == cm::controller::journal::TxnStatus::Committed
                    && state.txn.starts_with("auto-stop-")
            })
            .map(|state| state.txn.as_str())
            .collect();
        // [RW01.2]
        assert_eq!(stop_ids.len(), 2);
        assert!(stop_ids.iter().all(|id| id.starts_with("auto-stop-")));
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Stopped, 0))
        );
    });
}

#[test]
fn rw01_pinned_tunnel_survives_empty_holders() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw01-pinned").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw01-pinned-start");
        rig.deps.workers.note_launch(uid, "browser");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 0);
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 5_000);
        // [RW01.5]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Up, 0))
        );
        assert!(rig.exec.lock().unwrap().ran.is_empty());
    });
}

#[test]
fn rw01_failed_core_stop_keeps_network_and_blocks_instance() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw01-stop-fail").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        let owner = start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw01-fail-start");
        set_refcount_policy(&owner);
        seed_tunnel(&rig, &owner, 1);
        rig.deps.workers.note_launch(uid, "browser");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 0);
        rig.factory
            .script
            .lock()
            .unwrap()
            .push_back((FakeOp::Stop, CoreError::Failed));
        rig.exec.lock().unwrap().ran.clear();
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 5_000);
        // [RW01.4]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Blocked, 0))
        );
        assert_eq!(
            LeaseRegistry::open(&rig.deps.nets.lease_dir)
                .unwrap()
                .list()
                .unwrap()
                .len(),
            1
        );
        assert!(rig.exec.lock().unwrap().ran.is_empty());
    });
}

#[test]
#[ignore = "дефект RW02.1: run_as_for возвращает Failed на целевом musl до создания адаптера"]
fn rw02_tick_restarts_on_backoff_and_gives_up_after_five_attempts() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw02").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        let owner = start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw02-start");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, -2_000);
        {
            let mut script = rig.factory.script.lock().unwrap();
            script.push_back((FakeOp::Health, CoreError::ApiUnavailable));
            for _ in 0..5 {
                script.push_back((FakeOp::Start, CoreError::Failed));
            }
        }
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 0);
        let mut attempts = Vec::new();
        for at in [2_000, 4_000, 10_000, 40_000, 160_000] {
            watch
                .lock()
                .unwrap()
                .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, at);
            attempts.push(rig.factory.made.load(Ordering::SeqCst) - 1);
        }
        // [RW02.1]
        assert_eq!(attempts, [1, 2, 3, 4, 5]);
        // [RW02.2]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Failed, 5))
        );
        let made = rig.factory.made.load(Ordering::SeqCst);
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 162_000);
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 164_000);
        assert_eq!(rig.factory.made.load(Ordering::SeqCst), made);
        let mut generations = Generations::load(&owner.generations_path()).unwrap();
        generations.clear("browser").unwrap();
        write_config(dir.path(), uid, "browser", 1, CORE_CONFIG);
        // [RW02.5]
        assert!(
            rig.deps
                .workers
                .start(
                    &owner,
                    &RunAs {
                        uid,
                        gid: uid,
                        groups: vec![uid]
                    },
                    "rw02-after-failed",
                    "digest",
                    1,
                    164
                )
                .is_ok()
        );
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Starting, 0))
        );
    });
}

#[test]
fn rw02_tick_exposes_blocked_state_without_network_commands() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw02-blocked").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw02-blocked-start");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, -2_000);
        rig.factory
            .script
            .lock()
            .unwrap()
            .push_back((FakeOp::Health, CoreError::ApiUnavailable));
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, 0);
        // [RW02.3]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Blocked, 1))
        );
        assert!(
            rig.exec
                .lock()
                .unwrap()
                .ran
                .iter()
                .all(|command| command.program == cm::net::Program::Nft
                    || command.program == cm::net::Program::Ip)
        );
        assert!(rig.exec.lock().unwrap().ran.is_empty());
    });
}

#[test]
#[ignore = "дефект RW02.1: run_as_for возвращает Failed на целевом musl до создания адаптера"]
fn rw02_third_restart_recovers_before_failed_limit() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw02-third").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        start_fake_worker(&rig, dir.path(), uid, "browser", 1, "rw02-third-start");
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, -2_000);
        {
            let mut script = rig.factory.script.lock().unwrap();
            script.push_back((FakeOp::Health, CoreError::ApiUnavailable));
            script.push_back((FakeOp::Start, CoreError::Failed));
            script.push_back((FakeOp::Start, CoreError::Failed));
        }
        for at in [0, 2_000, 4_000, 10_000, 12_000] {
            watch
                .lock()
                .unwrap()
                .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, at);
        }
        // [RW02.4]
        assert_eq!(
            watch.lock().unwrap().state(uid, "browser"),
            Some((cm::net::state::TunnelState::Up, 3))
        );
    });
}

#[test]
fn co01_overview_keeps_worker_and_network_generations_separate() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-co01").unwrap();
        let uid = euid();
        ensure_instance(dir.path(), uid, "browser");
        let owner = owned(dir.path(), uid, "host").unwrap();
        let mut generations = Generations::load(&owner.generations_path()).unwrap();
        generations.set("browser", 2).unwrap();
        generations.set("net:browser", 1).unwrap();
        let rig = rig_with_query(dir.path(), false, empty_host_query());
        let overview = cm::controller::watch::overview(
            dir.path(),
            uid,
            &rig.deps.workers,
            &rig.deps.nets,
            "co01",
        )
        .unwrap();
        let Some(ReplyData::Overview { tunnels, .. }) = overview.data else {
            panic!("overview reply missing");
        };
        // [CO01.1]
        assert_eq!(
            (tunnels[0].worker_generation, tunnels[0].net_generation),
            (Some(2), Some(1))
        );
        generations.clear("browser").unwrap();
        let after_stop = cm::controller::watch::overview(
            dir.path(),
            uid,
            &rig.deps.workers,
            &rig.deps.nets,
            "co01-stop",
        )
        .unwrap();
        let Some(ReplyData::Overview { tunnels, .. }) = after_stop.data else {
            panic!("overview reply missing after stop");
        };
        // [CO01.2]
        assert_eq!(
            (tunnels[0].worker_generation, tunnels[0].net_generation),
            (None, Some(1))
        );
        generations.clear("net:browser").unwrap();
        let after_revert = cm::controller::watch::overview(
            dir.path(),
            uid,
            &rig.deps.workers,
            &rig.deps.nets,
            "co01-revert",
        )
        .unwrap();
        let Some(ReplyData::Overview { tunnels, .. }) = after_revert.data else {
            panic!("overview reply missing after revert");
        };
        // [CO01.3]
        assert!(
            dir.path()
                .join(format!("u{uid}/instances/browser"))
                .is_dir()
        );
        assert_eq!(
            (
                tunnels[0].worker_generation,
                tunnels[0].net_generation,
                tunnels[0].netns.as_deref()
            ),
            (None, None, None)
        );
    });
}

#[test]
fn co02_overview_is_owner_scoped_sorted_and_bounded() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-co02").unwrap();
        let uid = euid();
        ensure_instance(dir.path(), uid, "b");
        ensure_instance(dir.path(), uid, "a");
        ensure_instance(dir.path(), uid, "host");
        ensure_instance(dir.path(), uid.wrapping_add(1), "foreign");
        let owner = owned(dir.path(), uid, "host").unwrap();
        let mut generations = Generations::load(&owner.generations_path()).unwrap();
        generations.set("a", 1).unwrap();
        fs::create_dir_all(dir.path().join(format!("u{uid}/instances/Bad"))).unwrap();
        for index in 0..70 {
            ensure_instance(dir.path(), uid, &format!("row{index:02}"));
        }
        let rig = rig_with_query(dir.path(), false, empty_host_query());
        let overview = cm::controller::watch::overview(
            dir.path(),
            uid,
            &rig.deps.workers,
            &rig.deps.nets,
            "co02",
        )
        .unwrap();
        let Some(ReplyData::Overview { host, tunnels }) = overview.data else {
            panic!("overview reply missing");
        };
        // [CO02.1]
        assert_eq!(
            &tunnels[..2]
                .iter()
                .map(|row| row.instance.as_str())
                .collect::<Vec<_>>(),
            &["a", "b"]
        );
        // [CO02.2]
        assert!(tunnels.iter().all(|row| row.instance != "foreign"));
        assert_eq!(host.mode, "off");
        // [CO02.3]
        assert_eq!(tunnels.len(), 64);
        assert!(
            tunnels
                .iter()
                .all(|row| row.instance != "Bad" && row.instance != "host")
        );
        // [CO02.4]
        let row = tunnels.iter().find(|row| row.instance == "a").unwrap();
        assert_eq!(
            (
                row.state.as_str(),
                row.restarts,
                row.holders,
                row.worker_generation,
                row.net_generation,
                row.netns.as_deref()
            ),
            ("stopped", 0, 0, Some(1), None, None)
        );
    });
}

#[test]
#[ignore = "дефект CO02.5: sandbox запрещает bind AF_UNIX в peer fixture"]
fn co02_overview_authorizer_denial_is_returned() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-co02-denied").unwrap();
        let denied_rig = rig_with_query(dir.path(), true, empty_host_query());
        let peer = ab_support::child_peer(dir.path());
        let denied = ab_support::reply(&cm::controller::dispatch::handle(
            &ab_support::frame("co02-denied", serde_json::json!({"type":"overview"})),
            &peer.identity,
            &denied_rig.deps,
        ));
        // [CO02.5]
        assert_eq!(Op::Overview.class(), OpClass::Status);
        assert_eq!(denied["code"], "denied");
    });
}

#[test]
fn rw03_host_audit_reports_and_repairs_each_drift() {
    with_cm_conf(|| {
        let host = host_net(1000);
        let uplinks = uplinks();
        let (observed, rules6) = host_observation(&host, &uplinks);
        // [RW03.1]
        assert!(
            cm::net::host_audit::host_audit(
                &host,
                &uplinks,
                &observed,
                &rules6,
                &host_routes6(&host),
                true
            )
            .is_empty()
        );

        let mut no_v4 = host_observation(&host, &uplinks).0;
        no_v4.rules.retain(|rule| rule.priority != host.rule_sink);
        let drift = cm::net::host_audit::host_audit(
            &host,
            &uplinks,
            &no_v4,
            &rules6,
            &host_routes6(&host),
            true,
        );
        // [RW03.2]
        assert_eq!(
            drift,
            [cm::net::host_audit::HostDrift::MissingRule {
                v6: false,
                priority: 2001
            }]
        );
        assert_eq!(
            cmd_lines(&cm::net::host_audit::host_repair_commands(
                &host, &uplinks, &drift
            )),
            ["ip rule add priority 2001 lookup 91"]
        );

        let (_, mut no_v6) = host_observation(&host, &uplinks);
        no_v6.retain(|rule| rule.priority != host.rule_sink);
        let observed = host_observation(&host, &uplinks).0;
        let drift = cm::net::host_audit::host_audit(
            &host,
            &uplinks,
            &observed,
            &no_v6,
            &host_routes6(&host),
            true,
        );
        // [RW03.3]
        assert_eq!(
            drift,
            [cm::net::host_audit::HostDrift::MissingRule {
                v6: true,
                priority: 2001
            }]
        );
        assert_eq!(
            cmd_lines(&cm::net::host_audit::host_repair_commands(
                &host, &uplinks, &drift
            )),
            ["ip -6 rule add priority 2001 lookup 91"]
        );

        let (mut no_tun, rules6) = host_observation(&host, &uplinks);
        no_tun.links.retain(|link| link != "cmtunh");
        no_tun.routes.get_mut(&host.table).unwrap().clear();
        let drift = cm::net::host_audit::host_audit(
            &host,
            &uplinks,
            &no_tun,
            &rules6,
            &host_routes6(&host),
            true,
        );
        // [RW03.4]
        assert_eq!(
            drift,
            [
                cm::net::host_audit::HostDrift::MissingTun,
                cm::net::host_audit::HostDrift::MissingTunRoute
            ]
        );
        assert_eq!(
            cmd_lines(&cm::net::host_audit::host_repair_commands(
                &host, &uplinks, &drift
            )),
            [
                "ip tuntap add dev cmtunh mode tun user 1000",
                "ip addr add 198.18.64.1/30 dev cmtunh",
                "ip link set cmtunh mtu 1400 up",
                "ip route replace default dev cmtunh table 90"
            ]
        );

        let (mut no_sink, mut no_sink_v6) = host_observation(&host, &uplinks);
        no_sink.links.retain(|link| link != "cmsink");
        no_sink.routes.get_mut(&host.sink_table).unwrap().clear();
        no_sink.rules.retain(|rule| rule.priority != host.rule_tun);
        no_sink_v6.retain(|rule| rule.priority != host.rule_tun);
        let drift =
            cm::net::host_audit::host_audit(&host, &uplinks, &no_sink, &no_sink_v6, &[], true);
        let repairs = cmd_lines(&cm::net::host_audit::host_repair_commands(
            &host, &uplinks, &drift,
        ));
        let sink = repairs
            .iter()
            .position(|line| line == "ip link add cmsink type dummy")
            .unwrap();
        // [RW03.5]
        assert!(
            sink < repairs
                .iter()
                .position(|line| line == "ip rule add priority 2000 lookup 90")
                .unwrap()
        );

        let (observed, rules6) = host_observation(&host, &uplinks);
        let drift = cm::net::host_audit::host_audit(
            &host,
            &uplinks,
            &observed,
            &rules6,
            &host_routes6(&host),
            false,
        );
        let repair = cm::net::host_audit::host_repair_commands(&host, &uplinks, &drift);
        // [RW03.6]
        assert_eq!(repair.len(), 1);
        assert_eq!(
            repair[0].stdin.as_deref(),
            Some(
                format!(
                    "table ip cm_host\ndelete table ip cm_host\n{}",
                    render_host_table(&host)
                )
                .as_str()
            )
        );
    });
}

#[test]
fn rw03_watch_repairs_missing_host_tunnel_before_retrying_same_tick_time() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw03-repair").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let rig = watch_rig_with_query(dir.path(), watch_host_query());
        let watch = install_watch(&rig, dir.path());
        write_host_json(
            dir.path(),
            &HostState {
                uid,
                mode: "tunnel".to_owned(),
                generation: 1,
                uplinks: uplinks(),
            },
        );
        let owner = owned(dir.path(), uid, "host").unwrap();
        write_config(dir.path(), uid, "host", 1, CORE_CONFIG);
        let mut generations = Generations::load(&owner.generations_path()).unwrap();
        generations.set("host", 1).unwrap();
        let at = 20_000;
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, at);
        let repair_commands = cmd_lines(&rig.exec.lock().unwrap().ran);
        // [RW03.7]
        assert_eq!(
            repair_commands,
            [
                "ip tuntap add dev cmtunh mode tun user 1000",
                "ip addr add 198.18.64.1/30 dev cmtunh",
                "ip link set cmtunh mtu 1400 up",
                "ip route replace default dev cmtunh table 90"
            ]
        );
        rig.exec.lock().unwrap().ran.clear();
        watch
            .lock()
            .unwrap()
            .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, at);
        assert_eq!(
            watch.lock().unwrap().state(uid, "host"),
            Some((cm::net::state::TunnelState::Stopped, 1))
        );
    });
}

#[test]
fn rw03_watch_skips_host_observation_when_off_or_busy() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-rw03-skip").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let uid = euid();
        let proxy_rig = watch_rig(dir.path());
        let proxy_watch = install_watch(&proxy_rig, dir.path());
        write_host_json(
            dir.path(),
            &HostState {
                uid,
                mode: "proxy".to_owned(),
                generation: 1,
                uplinks: uplinks(),
            },
        );
        proxy_watch.lock().unwrap().tick(
            dir.path(),
            &proxy_rig.deps.workers,
            &proxy_rig.deps.nets,
            1_000,
        );
        let proxy_queries = proxy_rig.query.lock().unwrap().asked.clone();
        let proxy_no_host_observation = !proxy_queries
            .iter()
            .any(|query| query.contains("default table main"));
        assert!(proxy_rig.exec.lock().unwrap().ran.is_empty());

        fs::remove_file(cm::controller::host_ops::state_path(dir.path())).unwrap();
        let absent_rig = watch_rig(dir.path());
        let absent_watch = install_watch(&absent_rig, dir.path());
        absent_watch.lock().unwrap().tick(
            dir.path(),
            &absent_rig.deps.workers,
            &absent_rig.deps.nets,
            2_000,
        );
        let absent_queries = absent_rig.query.lock().unwrap().asked.clone();
        let absent_no_host_observation = !absent_queries
            .iter()
            .any(|query| query.contains("default table main"));
        // [RW03.8]
        assert!(proxy_no_host_observation && absent_no_host_observation);
        assert!(absent_rig.exec.lock().unwrap().ran.is_empty());

        write_host_json(
            dir.path(),
            &HostState {
                uid,
                mode: "tunnel".to_owned(),
                generation: 1,
                uplinks: uplinks(),
            },
        );
        let busy_rig = watch_rig(dir.path());
        let busy_watch = install_watch(&busy_rig, dir.path());
        assert!(busy_rig.deps.workers.try_begin(uid, "host"));
        busy_watch.lock().unwrap().tick(
            dir.path(),
            &busy_rig.deps.workers,
            &busy_rig.deps.nets,
            3_000,
        );
        let busy_queries = busy_rig.query.lock().unwrap().asked.clone();
        // [RW03.9]
        assert!(
            !busy_queries
                .iter()
                .any(|query| query.contains("default table main"))
        );
        assert!(busy_rig.exec.lock().unwrap().ran.is_empty());
    });
}

#[test]
fn ih01_host_check_facts_render_fixed_order_and_read_only() {
    with_cm_conf(|| {
        let clean = facts();
        let lines = host::check_lines(&clean);
        let rendered = host::render_text(&lines, &clean);
        // [IH01.1]
        assert_eq!(lines.len(), 10);
        assert!(lines.iter().all(|line| line.verdict == Verdict::Ok));
        assert_eq!(host::status_code(&lines), 0);
        let mut legacy = facts();
        legacy.links.push("cm-vpn".to_owned());
        let lines = host::check_lines(&legacy);
        // [IH01.2]
        assert_eq!(
            host::render_line(&lines[1], &legacy),
            "check=legacy_vpn result=block [legacy_vpn_active]"
        );
        assert_eq!(host::status_code(&lines), 1);
        let mut foreign = facts();
        foreign.rules.push(ObservedRule {
            priority: 100,
            table: "2022".to_owned(),
            iif: None,
        });
        let lines = host::check_lines(&foreign);
        // [IH01.3]
        assert_eq!(
            host::render_line(&lines[2], &foreign),
            "check=foreign_autoroute result=block [foreign_autoroute]"
        );
        let mut resolved = facts();
        resolved.integration.resolved = true;
        let lines = host::check_lines(&resolved);
        // [IH01.4]
        assert_eq!(
            host::render_line(&lines[3], &resolved),
            "check=resolved result=block [resolved_unsupported]"
        );
        let mut integrations = facts();
        integrations.integration.firewalld = true;
        integrations.integration.network_manager = true;
        let lines = host::check_lines(&integrations);
        // [IH01.5]
        assert_eq!(
            (lines[4].verdict, lines[5].verdict),
            (Verdict::Warn, Verdict::Warn)
        );
        assert_eq!(host::status_code(&lines), 0);
        let mut no_uplink = facts();
        no_uplink.uplinks = 0;
        let no_uplink_lines = host::check_lines(&no_uplink);
        let mut too_many = facts();
        too_many.uplinks = 9;
        let too_many_lines = host::check_lines(&too_many);
        // [IH01.6]
        assert_eq!(
            host::render_line(&no_uplink_lines[6], &no_uplink),
            "check=uplinks result=block [no_uplink]"
        );
        assert_eq!(
            host::render_line(&too_many_lines[6], &too_many),
            "check=uplinks result=block [too_many_uplinks]"
        );
        let mut stale = facts();
        stale.links.push("cmsink".to_owned());
        let stale_lines = host::check_lines(&stale);
        // [IH01.7]
        assert_eq!(
            host::render_line(&stale_lines[8], &stale),
            "check=stale_net result=block [stale_host_net]"
        );
        // [IH01.8]
        assert!(
            rendered
                .lines()
                .nth(9)
                .unwrap()
                .ends_with("all=0 default=2")
        );
        let dir = TempDirGuard::new("cm-fin-ih01").unwrap();
        let mut query = empty_host_query();
        let facts = host::collect_facts(
            &mut query,
            &dir.path().join("run"),
            &dir.path().join("proc"),
            &dir.path().join("missing.sock"),
            &dir.path().join("host.json"),
            true,
            false,
        );
        assert_eq!(facts.uplinks, 0);
        assert!(!query.asked.iter().any(|request| request.contains("add")
            || request.contains("del")
            || request.contains("set")
            || request.contains("replace")));
    });
}

#[test]
fn it01_teardown_removes_recorded_and_only_observed_host_state() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-it01-state").unwrap();
        let uid = euid();
        let rig = rig_with_query(dir.path(), false, empty_host_query());
        let uplinks = uplinks();
        write_host_json(
            dir.path(),
            &HostState {
                uid,
                mode: "tunnel".to_owned(),
                generation: 7,
                uplinks: uplinks.clone(),
            },
        );
        let owner = owned(dir.path(), uid, "host").unwrap();
        let mut generations = Generations::load(&owner.generations_path()).unwrap();
        generations.set("net:host", 7).unwrap();
        let report = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        let mut expected = cmd_lines(&host_firewall_commands(None));
        expected.extend(cmd_lines(&host_destroy_commands(&host_net(uid), &uplinks)));
        let actual = cmd_lines(&rig.exec.lock().unwrap().ran);
        // [IT01.1]
        assert_eq!(&actual[..expected.len()], &expected);
        assert!(report.host);
        assert!(!cm::controller::host_ops::state_path(dir.path()).exists());
        assert_eq!(
            Generations::load(&owner.generations_path())
                .unwrap()
                .current("net:host"),
            None
        );

        let dir = TempDirGuard::new("cm-fin-it01-observed").unwrap();
        let query = query_with(&[
            (
                "-j link show",
                r#"[{"ifname":"cmsink"},{"ifname":"cmtunh"}]"#,
            ),
            (
                "-j rule show",
                r#"[{"priority":1990,"table":"main"},{"priority":2000,"table":"90"},{"priority":2001,"table":"91"},{"priority":3000,"table":"main"}]"#,
            ),
            ("-j -6 rule show", r#"[{"priority":2001,"table":"91"}]"#),
            ("-j route show default table main", "[]"),
            ("-j -6 route show default table main", "[]"),
            ("-j -4 addr show", "[]"),
        ]);
        let rig = rig_with_query(dir.path(), false, query);
        let report = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        let commands = cmd_lines(&rig.exec.lock().unwrap().ran);
        // [IT01.2]
        assert_eq!(
            [1990, 2000, 2001].map(|priority| commands
                .iter()
                .filter(|line| *line == &format!("ip rule del priority {priority}"))
                .count()),
            [1, 1, 1]
        );
        assert!(report.host);

        let dir = TempDirGuard::new("cm-fin-it01-foreign").unwrap();
        let query = query_with(&[
            ("-j link show", r#"[{"ifname":"cmtunh"}]"#),
            (
                "-j rule show",
                r#"[{"priority":1990,"table":"200"},{"priority":3000,"table":"main"}]"#,
            ),
            ("-j -6 rule show", "[]"),
            ("-j route show default table main", "[]"),
            ("-j -6 route show default table main", "[]"),
            ("-j -4 addr show", "[]"),
        ]);
        let rig = rig_with_query(dir.path(), false, query);
        let report = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        let commands = cmd_lines(&rig.exec.lock().unwrap().ran);
        // [IT01.3]
        assert!(
            !commands
                .iter()
                .any(|line| line == "ip rule del priority 1990"
                    || line == "ip rule del priority 3000")
        );
        assert!(report.host);

        let dir = TempDirGuard::new("cm-fin-it01-leases").unwrap();
        let uid = euid();
        let rig = rig_with_query(dir.path(), false, empty_host_query());
        fs::create_dir_all(&rig.deps.nets.lease_dir).unwrap();
        let registry = LeaseRegistry::create(&rig.deps.nets.lease_dir).unwrap();
        let mut owners = Vec::new();
        for name in ["a", "b"] {
            ensure_instance(dir.path(), uid, name);
            let owner = owned(dir.path(), uid, name).unwrap();
            let mut generations = Generations::load(&owner.generations_path()).unwrap();
            generations.set(&format!("net:{name}"), 1).unwrap();
            owners.push(owner);
            let _lease = registry
                .allocate(&format!("u{uid}-{name}"), ResourceKind::Tunnel)
                .unwrap();
        }
        let leases = registry.list().unwrap();
        fs::create_dir_all(&rig.deps.nets.etc_root).unwrap();
        for lease in &leases {
            let netns = rig.deps.nets.etc_root.join(format!("cm-{}", lease.value));
            fs::create_dir_all(&netns).unwrap();
            fs::write(netns.join("resolv.conf"), "nameserver 198.18.0.2\n").unwrap();
        }
        let report = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        // [IT01.4]
        assert_eq!(report.nets, 2);
        assert!(registry.list().unwrap().is_empty());
        assert!(leases.iter().all(|lease| {
            !rig.deps
                .nets
                .etc_root
                .join(format!("cm-{}", lease.value))
                .exists()
        }));
        for owner in owners {
            assert_eq!(
                Generations::load(&owner.generations_path())
                    .unwrap()
                    .current(&format!("net:{}", owner.instance.as_str())),
                None
            );
        }

        let report = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        // [IT01.5]
        assert_eq!(report, Default::default());
    });
}

#[test]
fn it02_watch_is_quiet_for_two_passes_after_teardown() {
    with_cm_conf(|| {
        let dir = TempDirGuard::new("cm-fin-it02-watch").unwrap();
        let _state_env = TestStateEnv::new(dir.path());
        let rig = watch_rig(dir.path());
        let watch = install_watch(&rig, dir.path());
        let _ = teardown_all(dir.path(), &rig.deps.workers, &rig.deps.nets);
        rig.exec.lock().unwrap().ran.clear();
        for at in [1_000, 3_000] {
            watch
                .lock()
                .unwrap()
                .tick(dir.path(), &rig.deps.workers, &rig.deps.nets, at);
        }
        // [IT02.4]
        assert!(rig.exec.lock().unwrap().ran.is_empty());
    });
}
