//! K07–K13, K15–K17: names, command order, the nftables table, execution with rollback,
//! TUN config, audit of the observed state and the tunnel state machine.

mod pack_support;

use std::collections::{BTreeMap, BTreeSet};
use std::io::Write;
use std::process::{Command, Stdio};

use cm::common::contract_fixtures::TempDirGuard;
use cm::core::mihomo::config::{ConfigError, attach_tun, attach_unix_controller};
use cm::core::mihomo::validate_file;
use cm::net::dns::resolv_conf;
use cm::net::{
    Cmd, Drift, MAX_TUNNELS, NetError, Observed, ObservedRoute, ObservedRule, Program,
    RecordingExec, TunnelEvent, TunnelNet, TunnelState, audit, create, create_commands, destroy,
    destroy_commands, net_axis, next, parse_links, parse_routes, parse_rules, passes_traffic,
    render_table, repair_commands, replace_commands, tunnel_net,
};
use cm::profiles::{Ipv6Policy, VerificationValue};
use serde_json::{Value, json};

fn net(index: u8) -> TunnelNet {
    tunnel_net(index, 1000).unwrap()
}

fn line(cmd: &Cmd) -> String {
    let program = match cmd.program {
        Program::Ip => "ip",
        Program::Nft => "nft",
        Program::Sysctl => "sysctl",
    };
    format!("{program} {}", cmd.args.join(" "))
}

fn lines(commands: &[Cmd]) -> Vec<String> {
    commands.iter().map(line).collect()
}

#[test]
fn k07_names_and_addresses() {
    assert_eq!(
        net(0),
        TunnelNet {
            index: 0,
            netns: "cm-0".to_owned(),
            veth_host: "cmv0h".to_owned(),
            veth_ns: "cmv0n".to_owned(),
            host_addr: "10.213.0.1/30".to_owned(),
            ns_addr: "10.213.0.2/30".to_owned(),
            gateway: "10.213.0.1".to_owned(),
            tun: "cmtun0".to_owned(),
            tun_addr: "198.18.0.1/30".to_owned(),
            dns_addr: "198.18.0.2".to_owned(),
            table: 100,
            rule_priority: 1000,
            mtu: 1400,
            owner_uid: 1000,
        }
    );
    let last = net(63);
    assert_eq!(
        (
            last.veth_host.as_str(),
            last.host_addr.as_str(),
            last.tun.as_str(),
            last.table,
            last.rule_priority
        ),
        ("cmv63h", "10.213.63.1/30", "cmtun63", 163, 1063)
    );
    assert_eq!(tunnel_net(64, 1000), Err(NetError::BadIndex));
    assert_eq!(MAX_TUNNELS, 64);
    let mut names = BTreeSet::new();
    for index in 0..64 {
        let tunnel = net(index);
        for name in [&tunnel.veth_host, &tunnel.veth_ns, &tunnel.tun] {
            assert!(name.len() <= 15, "{name}");
        }
        for value in [
            tunnel.netns.clone(),
            tunnel.veth_host.clone(),
            tunnel.veth_ns.clone(),
            tunnel.host_addr.clone(),
            tunnel.ns_addr.clone(),
            tunnel.tun.clone(),
            tunnel.tun_addr.clone(),
            tunnel.dns_addr.clone(),
            format!("table {}", tunnel.table),
            format!("priority {}", tunnel.rule_priority),
        ] {
            assert!(names.insert(value.clone()), "{value}");
        }
    }
}

const CREATE: [&str; 17] = [
    "ip netns add cm-0",
    "ip link add cmv0h type veth peer name cmv0n",
    "ip link set cmv0n netns cm-0",
    "ip addr add 10.213.0.1/30 dev cmv0h",
    "ip link set cmv0h up",
    "ip -n cm-0 link set lo up",
    "ip -n cm-0 addr add 10.213.0.2/30 dev cmv0n",
    "ip -n cm-0 link set cmv0n up",
    "ip -n cm-0 route add default via 10.213.0.1",
    "ip netns exec cm-0 sysctl -qw net.ipv6.conf.all.disable_ipv6=1",
    "ip tuntap add dev cmtun0 mode tun user 1000",
    "ip addr add 198.18.0.1/30 dev cmtun0",
    "ip link set cmtun0 mtu 1400 up",
    "ip route add blackhole default metric 200 table 100",
    "ip route add default dev cmtun0 table 100",
    "ip rule add iif cmv0h lookup 100 priority 1000",
    "sysctl -qw net.ipv4.ip_forward=1",
];

const DESTROY: [&str; 5] = [
    "ip rule del iif cmv0h lookup 100 priority 1000",
    "ip route flush table 100",
    "ip link del cmtun0",
    "ip link del cmv0h",
    "ip netns del cm-0",
];

#[test]
fn k08_command_order() {
    let commands = create_commands(&net(0), Ipv6Policy::Block);
    assert_eq!(lines(&commands), CREATE);
    let pass = lines(&create_commands(&net(0), Ipv6Policy::Pass));
    assert_eq!(pass.len(), 16);
    assert!(!pass.iter().any(|item| item.contains("disable_ipv6")));
    let at = |needle: &str| CREATE.iter().position(|item| *item == needle).unwrap();
    assert!(at(CREATE[13]) < at(CREATE[14]) && at(CREATE[14]) < at(CREATE[15]));
    assert_eq!(lines(&destroy_commands(&net(0))), DESTROY);
    for index in [0, 63] {
        for cmd in create_commands(&net(index), Ipv6Policy::Block)
            .iter()
            .chain(destroy_commands(&net(index)).iter())
        {
            assert!(cmd.stdin.is_none());
            for arg in &cmd.args {
                assert!(!arg.contains([' ', ';', '|', '$']), "{arg}");
            }
        }
    }
}

const TABLE_0_3: &str = "table inet cm {
  chain forward {
    type filter hook forward priority filter; policy accept;
    iifname \"cmv0h\" oifname \"cmtun0\" counter accept
    iifname \"cmtun0\" oifname \"cmv0h\" counter accept
    iifname \"cmv3h\" oifname \"cmtun3\" counter accept
    iifname \"cmtun3\" oifname \"cmv3h\" counter accept
    iifname \"cmv*\" counter drop
    oifname \"cmv*\" counter drop
  }
  chain input {
    type filter hook input priority filter; policy accept;
    iifname \"cmv*\" counter drop
  }
}
";

#[test]
fn k09_table_text() {
    assert_eq!(render_table(&[net(0), net(3)]), TABLE_0_3);
    assert_eq!(render_table(&[net(3), net(0)]), TABLE_0_3);
    let empty = render_table(&[]);
    assert!(empty.contains("iifname \"cmv*\" counter drop\n"));
    assert!(empty.contains("oifname \"cmv*\" counter drop\n"));
    assert!(!empty.contains("accept\n"));
    assert!(TABLE_0_3.contains("policy accept;") && !TABLE_0_3.contains("fwd"));
    // Q09: the application cannot talk to the host itself; the only way out is the TUN.
    assert!(empty.ends_with(
        "  chain input {\n    type filter hook input priority filter; policy accept;\n    iifname \"cmv*\" counter drop\n  }\n}\n"
    ));
    let replace = replace_commands(&[net(0), net(3)]);
    assert_eq!(replace.len(), 1);
    assert_eq!(replace[0].program, Program::Nft);
    assert_eq!(replace[0].args, ["-f", "-"]);
    assert_eq!(
        replace[0].stdin.as_deref(),
        Some(format!("table inet cm\ndelete table inet cm\n{TABLE_0_3}").as_str())
    );
}

#[test]
fn k09_nft_accepts_the_table_twice() {
    if pack_support::rerun_in_netns("k09_nft_accepts_the_table_twice", "CM_PACK_K09_INNER") {
        return;
    }
    let text = replace_commands(&[net(0), net(3)])[0]
        .stdin
        .clone()
        .unwrap();
    for _ in 0..2 {
        let mut child = Command::new("/usr/bin/nft")
            .args(["-f", "-"])
            .stdin(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(text.as_bytes())
            .unwrap();
        assert!(child.wait().unwrap().success());
    }
    let listed = Command::new("/usr/bin/nft")
        .args(["list", "table", "inet", "cm"])
        .output()
        .unwrap();
    let listed = String::from_utf8(listed.stdout).unwrap();
    assert_eq!(listed.matches("bytes 0 accept").count(), 4, "{listed}");
    assert_eq!(listed.matches("drop").count(), 3, "{listed}");
}

#[test]
fn k10_firewall_first_and_rollback() {
    let mut exec = RecordingExec::default();
    create(&net(0), Ipv6Policy::Block, &[net(0)], &mut exec).unwrap();
    assert_eq!(exec.ran[0].program, Program::Nft);
    assert_eq!(lines(&exec.ran[1..]), CREATE);
    for fail_at in 1..=17 {
        let mut exec = RecordingExec {
            fail_at: Some(fail_at),
            ..RecordingExec::default()
        };
        assert_eq!(
            create(&net(0), Ipv6Policy::Block, &[net(0)], &mut exec),
            Err(NetError::CommandFailed)
        );
        let ran = lines(&exec.ran);
        assert_eq!(ran[1..=fail_at], CREATE[..fail_at]);
        assert_eq!(ran[fail_at + 1..], DESTROY, "fail_at {fail_at}");
    }
    let mut exec = RecordingExec {
        fail_at: Some(1),
        ..RecordingExec::default()
    };
    // A part that is already gone is not a failure: the network must stay removable.
    assert_eq!(destroy(&net(0), &[net(3)], &mut exec), Ok(()));
    assert_eq!(lines(&exec.ran[..5]), DESTROY);
    let last = exec.ran.last().unwrap();
    assert_eq!(last.program, Program::Nft);
    assert!(last.stdin.as_deref().unwrap().contains("cmv3h"));
    assert!(!last.stdin.as_deref().unwrap().contains("cmv0h"));
    let source = include_str!("../src/net/exec.rs");
    for program in [
        "\"/usr/bin/ip\"",
        "\"/usr/bin/nft\"",
        "\"/usr/bin/sysctl\"",
        ".env_clear()",
    ] {
        assert!(source.contains(program), "{program}");
    }
    assert!(!source.contains("Command::new(\""));
}

fn tun_document(index: u8) -> Value {
    let doc = json!({"mode":"direct","ipv6":false});
    serde_json::from_slice(&attach_tun(&doc, &net(index)).unwrap()).unwrap()
}

#[test]
fn k11_tun_config() {
    let config = tun_document(0);
    assert_eq!(
        config["tun"],
        json!({"enable":true,"device":"cmtun0","stack":"gvisor","auto-route":false,
            "auto-redirect":false,"auto-detect-interface":false,"mtu":1400,
            "inet4-address":["198.18.0.1/30"],"dns-hijack":["any:53","tcp://any:53"]})
    );
    assert!(config.get("listeners").is_none());
    assert_eq!(config["dns"]["enable"], true);
    assert_eq!(config["dns"]["enhanced-mode"], "redir-host");
    assert_eq!(config["dns"]["ipv6"], false);
    for (key, value) in [
        ("tun", json!({"enable":true})),
        ("dns", json!({"enable":false})),
        ("mixed-port", json!(7890)),
        ("external-controller", json!("0.0.0.0:9090")),
    ] {
        let mut doc = json!({"mode":"direct"});
        doc[key] = value;
        assert_eq!(
            attach_tun(&doc, &net(0)).err(),
            Some(ConfigError::Forbidden),
            "{key}"
        );
    }
    let geo = json!({"mode":"rule","rules":["GEOIP,CN,DIRECT","MATCH,DIRECT"]});
    assert_eq!(
        attach_tun(&geo, &net(0)).err(),
        Some(ConfigError::InvalidRule)
    );
    assert_eq!(
        resolv_conf(&net(0)),
        "nameserver 198.18.0.2\noptions edns0\n"
    );
}

#[test]
fn k11_pinned_mihomo_accepts_the_tun_config() {
    let Some(binary) = std::env::var_os("CM_TEST_MIHOMO") else {
        eprintln!("K11 SKIPPED: CM_TEST_MIHOMO not set");
        return;
    };
    let dir = TempDirGuard::new("cm-pack-k11").unwrap();
    let socket = dir.path().join("api.sock");
    let bytes = attach_unix_controller(&tun_document(0), &socket).unwrap();
    validate_file(std::path::Path::new(&binary), dir.path(), &bytes).unwrap();
}

#[test]
fn k12_k13_everything_comes_from_tunnel_net() {
    for index in 0..64 {
        let tunnel = net(index);
        let text = render_table(std::slice::from_ref(&tunnel));
        let accepts: Vec<&str> = text
            .lines()
            .filter(|line| line.ends_with("accept"))
            .collect();
        assert_eq!(
            accepts,
            [
                format!(
                    "    iifname \"{}\" oifname \"{}\" counter accept",
                    tunnel.veth_host, tunnel.tun
                ),
                format!(
                    "    iifname \"{}\" oifname \"{}\" counter accept",
                    tunnel.tun, tunnel.veth_host
                ),
            ]
        );
    }
    let config = tun_document(5);
    assert_eq!(config["tun"]["device"], "cmtun5");
    assert_eq!(config["tun"]["inet4-address"], json!(["198.18.5.1/30"]));
    assert_eq!(
        resolv_conf(&net(5)),
        "nameserver 198.18.5.2\noptions edns0\n"
    );
}

fn observed(rules: &str, routes: &str, links: &str, netns: &[&str]) -> Observed {
    let mut tables = BTreeMap::new();
    tables.insert(100, parse_routes(routes).unwrap());
    Observed {
        rules: parse_rules(rules).unwrap(),
        routes: tables,
        links: parse_links(links).unwrap(),
        netns: netns.iter().map(|name| (*name).to_owned()).collect(),
    }
}

const RULES: &str = include_str!("fixtures/pack/rules.json");
const ROUTES: &str = include_str!("fixtures/pack/routes-100.json");
const LINKS: &str = include_str!("fixtures/pack/links.json");

fn edit(json: &str, change: impl Fn(&mut Vec<Value>)) -> String {
    let mut items: Vec<Value> = serde_json::from_str(json).unwrap();
    change(&mut items);
    serde_json::to_string(&items).unwrap()
}

#[test]
fn k15_drift_is_named_and_only_safe_drift_is_repaired() {
    let desired = [net(0)];
    assert_eq!(
        audit(&desired, &observed(RULES, ROUTES, LINKS, &["cm-0"])),
        []
    );

    let no_blackhole = edit(ROUTES, |items| {
        items.retain(|item| item.get("type").is_none())
    });
    let drift = audit(&desired, &observed(RULES, &no_blackhole, LINKS, &["cm-0"]));
    assert_eq!(drift, [Drift::MissingBlackhole(0)]);
    assert_eq!(lines(&repair_commands(&drift[0], &desired)), [CREATE[13]]);

    let no_rule = edit(RULES, |items| {
        items.retain(|item| item.get("iif").is_none())
    });
    let drift = audit(&desired, &observed(&no_rule, ROUTES, LINKS, &["cm-0"]));
    assert_eq!(drift, [Drift::MissingRule(0)]);
    assert_eq!(lines(&repair_commands(&drift[0], &desired)), [CREATE[15]]);

    let no_tun = edit(LINKS, |items| {
        items.retain(|item| item["ifname"] != "cmtun0")
    });
    let drift = audit(&desired, &observed(RULES, ROUTES, &no_tun, &["cm-0"]));
    assert_eq!(drift, [Drift::MissingTunRoute(0), Drift::MissingTun(0)]);
    for item in &drift {
        assert!(repair_commands(item, &desired).is_empty());
    }
    assert_eq!(
        audit(&desired, &observed(RULES, ROUTES, LINKS, &[])),
        [Drift::MissingNetns(0)]
    );

    let extra_link = edit(LINKS, |items| items.push(json!({"ifname":"cmv7h"})));
    let drift = audit(&desired, &observed(RULES, ROUTES, &extra_link, &["cm-0"]));
    assert_eq!(drift, [Drift::OrphanVeth("cmv7h".to_owned())]);
    assert_eq!(
        lines(&repair_commands(&drift[0], &desired)),
        ["ip link del cmv7h"]
    );

    let orphan_rule = edit(RULES, |items| {
        items.push(json!({"priority":1042,"src":"all","iif":"cmv42h","table":"142"}))
    });
    let drift = audit(&desired, &observed(&orphan_rule, ROUTES, LINKS, &["cm-0"]));
    assert_eq!(drift, [Drift::OrphanRule(1042)]);
    assert_eq!(
        lines(&repair_commands(&drift[0], &desired)),
        ["ip rule del iif cmv42h priority 1042"]
    );
    // A foreign rule that happens to use the same priority is not ours to delete.
    for foreign in [
        json!({"priority":1042,"src":"all","table":"142"}),
        json!({"priority":1042,"src":"all","iif":"eth0","table":"142"}),
        json!({"priority":1042,"src":"all","iif":"cmv7h","table":"142"}),
    ] {
        let rules = edit(RULES, |items| items.push(foreign.clone()));
        assert_eq!(
            audit(&desired, &observed(&rules, ROUTES, LINKS, &["cm-0"])),
            []
        );
    }
    assert_eq!(parse_rules("not json").err(), Some(NetError::Drift));
    assert_eq!(parse_routes("{}").err(), Some(NetError::Drift));
    assert_eq!(parse_links("[{}]").err(), Some(NetError::Drift));
}

#[test]
fn k17_audit_looks_for_the_names_of_tunnel_net() {
    let tunnel = net(5);
    let empty = Observed {
        rules: Vec::new(),
        routes: BTreeMap::new(),
        links: Vec::new(),
        netns: Vec::new(),
    };
    assert_eq!(
        audit(std::slice::from_ref(&tunnel), &empty),
        [
            Drift::MissingBlackhole(5),
            Drift::MissingRule(5),
            Drift::MissingTunRoute(5),
            Drift::MissingVeth(5),
            Drift::MissingTun(5),
            Drift::MissingNetns(5)
        ]
    );
    let mut routes = BTreeMap::new();
    routes.insert(
        tunnel.table,
        vec![
            ObservedRoute {
                dst: "default".to_owned(),
                dev: None,
                kind: Some("blackhole".to_owned()),
                metric: Some(200),
            },
            ObservedRoute {
                dst: "default".to_owned(),
                dev: Some(tunnel.tun.clone()),
                kind: None,
                metric: None,
            },
        ],
    );
    let full = Observed {
        rules: vec![ObservedRule {
            priority: tunnel.rule_priority,
            iif: Some(tunnel.veth_host.clone()),
            table: tunnel.table.to_string(),
        }],
        routes,
        links: vec![tunnel.veth_host.clone(), tunnel.tun.clone()],
        netns: vec![tunnel.netns.clone()],
    };
    assert_eq!(audit(std::slice::from_ref(&tunnel), &full), []);
}

#[test]
fn k16_state_machine() {
    use TunnelEvent as E;
    use TunnelState as S;
    let states = [
        S::Stopped,
        S::Starting,
        S::Up,
        S::Degraded,
        S::Blocked,
        S::Failed,
    ];
    let events = [
        E::StartRequested,
        E::NetReady,
        E::CoreReady,
        E::RemoteLost,
        E::RemoteBack,
        E::CoreDown,
        E::DriftFound,
        E::StopRequested,
        E::Repaired,
    ];
    let table = [
        (S::Stopped, E::StartRequested, S::Starting),
        (S::Failed, E::StartRequested, S::Starting),
        (S::Starting, E::CoreReady, S::Up),
        (S::Blocked, E::CoreReady, S::Up),
        (S::Starting, E::CoreDown, S::Failed),
        (S::Starting, E::DriftFound, S::Blocked),
        (S::Up, E::CoreDown, S::Blocked),
        (S::Up, E::DriftFound, S::Blocked),
        (S::Degraded, E::CoreDown, S::Blocked),
        (S::Degraded, E::DriftFound, S::Blocked),
        (S::Up, E::RemoteLost, S::Degraded),
        (S::Degraded, E::RemoteBack, S::Up),
        (S::Blocked, E::Repaired, S::Starting),
    ];
    for state in states {
        for event in events {
            let expected = if event == E::StopRequested {
                S::Stopped
            } else {
                table
                    .iter()
                    .find(|(from, on, _)| *from == state && *on == event)
                    .map(|(_, _, to)| *to)
                    .unwrap_or(state)
            };
            let got = next(state, event);
            assert_eq!(got, expected, "{state:?} + {event:?}");
            if got == S::Up && state != S::Up {
                assert!(matches!(event, E::CoreReady | E::RemoteBack));
            }
            // Traffic never starts on an event that is not about the core or the remote.
            if passes_traffic(got) && !passes_traffic(state) {
                assert_eq!(event, E::CoreReady);
            }
        }
    }
    assert_eq!(
        states.map(passes_traffic),
        [false, false, true, true, false, false]
    );
    assert_eq!(
        states.map(net_axis),
        [
            VerificationValue::Unknown,
            VerificationValue::Unknown,
            VerificationValue::Verified,
            VerificationValue::Partial,
            VerificationValue::Blocked,
            VerificationValue::Error
        ]
    );
}
