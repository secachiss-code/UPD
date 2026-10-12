//! H01–H05, H08, H10: host tunnel plan, uplinks, commands, firewall, transitions, preflight.

#[path = "ab_support/mod.rs"]
mod ab_support;

use std::collections::BTreeSet;
use std::io::Write;
use std::process::{Command, Stdio};

use cm::core::mihomo::config::attach_tun;
use cm::net::host::{self, HostBlock, HostStep, Integration, detect, preflight, transition};
use cm::net::host_commands::{host_create_commands, host_destroy_commands, uplink_commands};
use cm::net::host_firewall::{host_firewall_commands, render_host_table};
use cm::net::host_plan::{BOOTSTRAP_DNS, host_net};
use cm::net::uplink::{parse_addrs, parse_default_routes, uplinks};
use cm::net::{NetError, Program, TunnelNet, tunnel_net};
use cm::profiles::HostMode;
use serde_json::json;

use ab_support::{CM_CONF, cmd_line, cmd_lines, rerun_in_netns, with_cm_conf};

fn host() -> cm::net::host_plan::HostNet {
    host_net(1000)
}

#[test]
fn h01_host_net_and_tunnel_range() {
    with_cm_conf(|| {
        let h = host();
        assert_eq!(h.tun, "cmtunh");
        assert_eq!(h.tun_addr, "198.18.64.1/30");
        assert_eq!(h.dns_addr, "198.18.64.2");
        assert_eq!(h.sink, "cmsink");
        assert_eq!(h.table, 90);
        assert_eq!(h.sink_table, 91);
        assert_eq!(h.rule_uplink, 1900);
        assert_eq!(h.rule_uplink_addr, 1950);
        assert_eq!(h.rule_main, 1990);
        assert_eq!(h.rule_tun, 2000);
        assert_eq!(h.rule_sink, 2001);
        assert_eq!(h.mtu, 1400);
        assert_eq!(h.owner_uid, 1000);
        for index in 0..64u8 {
            let t = tunnel_net(index, 1000).unwrap();
            assert_ne!(t.table, 90);
            assert_ne!(t.table, 91);
            assert!(t.rule_priority < 1900);
            assert_ne!(t.tun_addr, "198.18.64.1/30");
        }
        assert_eq!(tunnel_net(64, 0), Err(NetError::BadIndex));
        assert_eq!(BOOTSTRAP_DNS, ["1.1.1.1", "8.8.8.8"]);
        let doc: serde_json::Value =
            serde_json::from_slice(&attach_tun(&json!({"mode":"direct"}), &h.as_tunnel()).unwrap())
                .unwrap();
        assert_eq!(
            doc["dns"]["default-nameserver"],
            json!(["1.1.1.1", "8.8.8.8"])
        );
    });
}

#[test]
fn h02_host_firewall_table_and_nft_check() {
    with_cm_conf(|| {
        let h = host();
        let expected = render_host_table(&h);
        assert_eq!(render_host_table(&host_net(0)), expected);
        let apply = host_firewall_commands(Some(&h));
        assert_eq!(apply.len(), 1);
        assert_eq!(apply[0].program, Program::Nft);
        assert_eq!(apply[0].args, ["-f", "-"]);
        let stdin = apply[0].stdin.as_deref().unwrap();
        assert!(stdin.starts_with("table ip cm_host\ndelete table ip cm_host\n"));
        assert!(stdin.contains("table ip cm_host {"));
        let del = host_firewall_commands(None);
        assert_eq!(
            del[0].stdin.as_deref(),
            Some("table ip cm_host\ndelete table ip cm_host\n")
        );
        if rerun_in_netns("h02_host_firewall_table_and_nft_check", "CM_AB_H02_INNER") {
            return;
        }
        for text in [stdin, del[0].stdin.as_deref().unwrap()] {
            let mut child = Command::new("nft")
                .args(["-c", "-f", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::piped())
                .spawn()
                .expect("nft");
            child
                .stdin
                .take()
                .unwrap()
                .write_all(text.as_bytes())
                .unwrap();
            let status = child.wait().unwrap();
            assert!(status.success(), "nft rejected host table");
        }
    });
}

#[test]
fn h03_host_as_tunnel_attach_tun() {
    with_cm_conf(|| {
        let h = host();
        let t = h.as_tunnel();
        assert_eq!(
            t,
            TunnelNet {
                index: 64,
                netns: String::new(),
                veth_host: String::new(),
                veth_ns: String::new(),
                host_addr: String::new(),
                ns_addr: String::new(),
                gateway: String::new(),
                tun: "cmtunh".to_owned(),
                tun_addr: "198.18.64.1/30".to_owned(),
                dns_addr: "198.18.64.2".to_owned(),
                table: 90,
                rule_priority: 2000,
                mtu: 1400,
                owner_uid: 1000,
            }
        );
        let doc: serde_json::Value =
            serde_json::from_slice(&attach_tun(&json!({"mode":"direct"}), &t).unwrap()).unwrap();
        assert_eq!(doc["tun"]["device"], "cmtunh");
        assert_eq!(doc["tun"]["inet4-address"], json!(["198.18.64.1/30"]));
        assert_eq!(doc["tun"]["auto-route"], false);
        for index in 0..64u8 {
            let doc: serde_json::Value = serde_json::from_slice(
                &attach_tun(&json!({"mode":"direct"}), &tunnel_net(index, 1000).unwrap()).unwrap(),
            )
            .unwrap();
            assert_eq!(doc["tun"]["auto-detect-interface"], true);
        }
    });
}

#[test]
fn h04_uplink_parsing() {
    with_cm_conf(|| {
        let routes = include_str!("fixtures/ab/routes-default.json");
        let addrs = include_str!("fixtures/ab/addrs.json");
        let v4 = parse_default_routes(routes).unwrap();
        let parsed = parse_addrs(addrs).unwrap();
        let list = uplinks(&v4, &[], &parsed);
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].dev, "up0");
        assert_eq!(list[0].addrs, ["192.0.2.2"]);
        assert_eq!(list[1].dev, "wlan0");
        assert_eq!(list[1].addrs, ["10.0.0.5"]);
        let v4 = parse_default_routes(
            r#"[{"dev":"cmtunh","metric":0},{"dev":"lo","metric":0},{"dev":"up0","metric":100}]"#,
        )
        .unwrap();
        let list = uplinks(&v4, &[], &parsed);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].dev, "up0");
        let v6 = parse_default_routes(r#"[{"dev":"up0","metric":1024}]"#).unwrap();
        let list = uplinks(&[], &v6, &parsed);
        assert_eq!(
            list,
            [cm::net::host_plan::Uplink {
                dev: "up0".to_owned(),
                addrs: vec![],
            }]
        );
        let dup =
            parse_default_routes(r#"[{"dev":"up0","metric":100},{"dev":"up0","metric":200}]"#)
                .unwrap();
        assert_eq!(uplinks(&dup, &[], &parsed).len(), 1);
        let many: Vec<_> = (0..10)
            .map(|i| format!(r#"{{"dev":"eth{i}","metric":{i}}}"#))
            .collect();
        let body = format!("[{}]", many.join(","));
        let v4 = parse_default_routes(&body).unwrap();
        assert_eq!(uplinks(&v4, &[], &parsed).len(), 8);
        let no_metric = parse_default_routes(r#"[{"dev":"up0"}]"#).unwrap();
        assert_eq!(no_metric[0].metric, 0);
        assert!(parse_default_routes("не json").is_err());
        let link_scope =
            parse_addrs(r#"[{"ifname":"up0","addr_info":[{"local":"fe80::1","scope":"link"}]}]"#)
                .unwrap();
        assert!(link_scope.get("up0").unwrap().is_empty());
    });
}

#[test]
fn h05_host_command_order() {
    with_cm_conf(|| {
        let h = host();
        let upl = [cm::net::host_plan::Uplink {
            dev: "up0".to_owned(),
            addrs: vec!["192.0.2.2".to_owned()],
        }];
        let create = cmd_lines(&host_create_commands(&h, &upl));
        assert_eq!(create.len(), 16);
        assert_eq!(create[0], "ip link add cmsink type dummy");
        assert_eq!(
            create.last().unwrap(),
            "ip rule add priority 2000 lookup 90"
        );
        let p2001 = create
            .iter()
            .position(|l| l.contains("priority 2001 lookup 91"))
            .unwrap();
        let p2000 = create
            .iter()
            .position(|l| l.contains("priority 2000 lookup 90"))
            .unwrap();
        assert!(p2001 < p2000);
        for needle in [
            "ip rule add priority 1900 oif up0 lookup main",
            "ip -6 rule add priority 1900 oif up0 lookup main",
            "ip rule add priority 1950 from 192.0.2.2 lookup main",
            "ip -6 rule add priority 2001 lookup 91",
            "ip tuntap add dev cmtunh mode tun user 1000",
        ] {
            assert!(create.iter().any(|l| l == needle), "{needle}");
        }
        assert!(
            !create
                .iter()
                .any(|l| l.contains("lookup 90") && l.starts_with("ip -6"))
        );
        let upl2 = [
            cm::net::host_plan::Uplink {
                dev: "up0".to_owned(),
                addrs: vec!["192.0.2.2".to_owned()],
            },
            cm::net::host_plan::Uplink {
                dev: "up1".to_owned(),
                addrs: vec!["10.0.0.1".to_owned(), "10.0.0.2".to_owned()],
            },
        ];
        let multi = cmd_lines(&host_create_commands(&h, &upl2));
        assert!(multi.iter().any(|l| l.contains("priority 1901 oif up1")));
        assert!(multi.iter().any(|l| l.contains("priority 1951 from")));
        assert!(multi.iter().any(|l| l.contains("priority 1952 from")));
        let destroy = cmd_lines(&host_destroy_commands(&h, &upl));
        assert_eq!(destroy[0], "ip rule del priority 2000 lookup 90");
        assert_eq!(destroy[destroy.len() - 2], "ip link del cmtunh");
        assert_eq!(destroy.last().unwrap(), "ip link del cmsink");
        let add_rules: BTreeSet<_> = create
            .iter()
            .filter(|l| l.contains(" rule add "))
            .cloned()
            .collect();
        for rule in add_rules {
            let del = rule.replace(" rule add ", " rule del ");
            assert!(destroy.iter().any(|l| l == &del), "{del}");
        }
        assert!(uplink_commands(&h, &upl, &upl).is_empty());
        let old = [cm::net::host_plan::Uplink {
            dev: "up0".to_owned(),
            addrs: vec![],
        }];
        let new = [cm::net::host_plan::Uplink {
            dev: "up1".to_owned(),
            addrs: vec![],
        }];
        let change = cmd_lines(&uplink_commands(&h, &old, &new));
        let dels: Vec<_> = change
            .iter()
            .take_while(|l| l.contains(" rule del "))
            .collect();
        let adds: Vec<_> = change
            .iter()
            .skip(dels.len())
            .filter(|l| l.contains(" rule add "))
            .collect();
        assert!(!dels.is_empty() && !adds.is_empty());
        for line in create.iter().chain(destroy.iter()) {
            assert!(!line.contains("blackhole"));
        }
    });
}

#[test]
fn h08_preflight_and_detect() {
    with_cm_conf(|| {
        let clean = Integration {
            network_manager: false,
            firewalld: false,
            resolved: false,
        };
        assert_eq!(
            preflight(
                HostMode::Tunnel,
                &["lo".to_owned(), "cm-vpn".to_owned()],
                &[],
                &clean
            ),
            Err(HostBlock::LegacyCm)
        );
        assert_eq!(
            preflight(
                HostMode::Tunnel,
                &["lo".to_owned()],
                &[cm::net::observe::ObservedRule {
                    priority: 100,
                    table: "2022".to_owned(),
                    iif: None,
                }],
                &clean
            ),
            Err(HostBlock::ForeignAutoRoute)
        );
        assert_eq!(
            preflight(
                HostMode::Tunnel,
                &["lo".to_owned()],
                &[],
                &Integration {
                    resolved: true,
                    ..clean
                }
            ),
            Err(HostBlock::Resolved)
        );
        assert!(
            preflight(
                HostMode::Proxy,
                &["cm-vpn".to_owned()],
                &[cm::net::observe::ObservedRule {
                    priority: 100,
                    table: "2022".to_owned(),
                    iif: None,
                }],
                &Integration {
                    resolved: true,
                    ..clean
                }
            )
            .is_ok()
        );
        assert!(
            preflight(
                HostMode::Tunnel,
                &["lo".to_owned()],
                &[],
                &Integration {
                    network_manager: true,
                    firewalld: true,
                    resolved: false,
                }
            )
            .is_ok()
        );
        let dir = cm::common::contract_fixtures::TempDirGuard::new("cm-ab-h08").unwrap();
        std::fs::create_dir_all(dir.path().join("NetworkManager")).unwrap();
        std::fs::create_dir_all(dir.path().join("systemd/resolve")).unwrap();
        std::fs::write(
            dir.path().join("systemd/resolve/stub-resolv.conf"),
            b"nameserver 127.0.0.53\n",
        )
        .unwrap();
        let found = detect(dir.path());
        assert!(found.network_manager && found.resolved && !found.firewalld);
    });
}

#[test]
fn h10_host_transitions() {
    with_cm_conf(|| {
        use HostMode::{Off, Proxy, Tunnel};
        use HostStep::{ApplyNet, RevertNet, StartWorker, StopWorker};
        assert_eq!(transition(Off, Proxy), vec![ApplyNet(Proxy), StartWorker]);
        assert_eq!(transition(Off, Tunnel), vec![ApplyNet(Tunnel), StartWorker]);
        assert_eq!(transition(Proxy, Off), vec![StopWorker, RevertNet]);
        assert_eq!(transition(Tunnel, Off), vec![StopWorker, RevertNet]);
        assert_eq!(
            transition(Proxy, Tunnel),
            vec![StopWorker, RevertNet, ApplyNet(Tunnel), StartWorker]
        );
        assert_eq!(
            transition(Tunnel, Proxy),
            vec![StopWorker, RevertNet, ApplyNet(Proxy), StartWorker]
        );
        for mode in [Off, Proxy, Tunnel] {
            assert!(transition(mode, mode).is_empty());
        }
        for (from, to) in [(Off, Proxy), (Proxy, Off), (Proxy, Tunnel)] {
            let steps = transition(from, to);
            if steps.contains(&StopWorker) && steps.contains(&RevertNet) {
                assert!(
                    steps.iter().position(|s| *s == StopWorker)
                        < steps.iter().position(|s| *s == RevertNet)
                );
            }
            if steps.contains(&ApplyNet(to)) && steps.contains(&StartWorker) {
                assert!(
                    steps.iter().position(|s| matches!(s, ApplyNet(_)))
                        < steps.iter().position(|s| *s == StartWorker)
                );
            }
        }
    });
}
