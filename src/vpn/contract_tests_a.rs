    #[test]
    fn auto_group_label_and_rules_follow_language() {
        crate::i18n::set(crate::i18n::Lang::Zh);
        assert_eq!(label(AUTO_GROUP), "⚡ 自动");
        assert_eq!(label("🇩🇪 DE-1"), "🇩🇪 DE-1");
        assert!(rules_template().contains("# 示例：") && rules_template().contains("GEOSITE,youtube,⚡ 自动"));
        crate::i18n::set(crate::i18n::Lang::Ru);
        assert_eq!(label(AUTO_GROUP), "⚡ Авто");
        // в правилах группа на любом языке и прежнее имя — постоянный идентификатор
        for n in ["⚡ Авто", "⚡ 自动", "⚡ تلقائي", "⚡ Auto"] {
            assert!(is_auto_group(n), "{n}");
        }
        assert!(!is_auto_group("Proxy"));
    }

    use super::*;
    use std::path::PathBuf;

    const SECRET: &str = "SUPERSECRET_SUB_TOKEN_XYZ";

    struct EnvGuard {
        _iso: std::sync::MutexGuard<'static, ()>,
        saved: Vec<(String, Option<String>)>,
        cleanup: PathBuf,
    }

    impl EnvGuard {
        fn vpn_dirs() -> Self {
            let _iso = crate::common::contract_fixtures::isolation_lock();
            let base = std::env::temp_dir().join(format!("cm-vpn-test-{}-{}", std::process::id(), now()));
            let etc = base.join("etc");
            let home = base.join("home");
            let state = base.join("state");
            fs::create_dir_all(&etc).unwrap();
            fs::create_dir_all(&home).unwrap();
            fs::create_dir_all(&state).unwrap();
            let mut g = Self::set_vars(_iso, [("CM_VPN_ETC", etc.to_str().unwrap()), ("CM_VPN_HOME", home.to_str().unwrap()), ("CM_STATE_DIR", state.to_str().unwrap())]);
            g.cleanup = base;
            g
        }

        fn set_vars(iso: std::sync::MutexGuard<'static, ()>, pairs: [(&str, &str); 3]) -> Self {
            let mut saved = Vec::new();
            for (k, v) in pairs {
                saved.push((k.to_string(), std::env::var(k).ok()));
                // SAFETY: test fixture; environment writes are serialized by contract_fixtures::isolation_lock.
                unsafe {
                    std::env::set_var(k, v);
                }
            }
            Self { _iso: iso, saved, cleanup: PathBuf::new() }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (k, v) in &self.saved {
                match v {
                    // SAFETY: test fixture; environment writes are serialized by contract_fixtures::isolation_lock.
                    Some(val) => unsafe {
                        std::env::set_var(k, val);
                    },
                    // SAFETY: test fixture; environment writes are serialized by contract_fixtures::isolation_lock.
                    None => unsafe {
                        std::env::remove_var(k);
                    },
                }
            }
            if !self.cleanup.as_os_str().is_empty() {
                let _ = fs::remove_dir_all(&self.cleanup);
            }
        }
    }

    fn assert_no_secret(text: &str) {
        assert!(!text.contains(SECRET), "утечка секрета в: {text}");
        assert!(!text.contains("user:pass@"), "утечка userinfo в: {text}");
    }

    // --- SEC-01 ---
    #[test]
    fn sec01_safe_ureq_error_hides_url() {
        let response = ureq::Response::new(401, SECRET, SECRET).unwrap();
        let err = ureq::Error::Status(401, response);
        let msg = safe_ureq_error(&err);
        assert_eq!(msg, "HTTP 401");
        assert_no_secret(&msg);
    }

    #[test]
    fn sec01_transport_error_hides_url() {
        let url = format!("broken/url/{SECRET}");
        let err = agent(3, None).get(&url).call().unwrap_err();
        let msg = safe_ureq_error(&err);
        assert!(msg.contains("неверный"), "msg={msg}");
        assert_no_secret(&msg);
    }

    #[test]
    fn sec01_scrub_stored_error_and_publish() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        let leaked = format!("напрямую: GET {SECRET} failed https://{SECRET}@host/");
        let raw = serde_json::json!({
            "active": "a1",
            "list": [{
                "id": "a1",
                "name": "t",
                "url": format!("https://example.com/{SECRET}"),
                "error": leaked
            }]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        let subs = load_subs().unwrap();
        assert_no_secret(&subs.list[0].error);
        assert!(subs.list[0].error.contains("скрыт") || !subs.list[0].error.contains("://"));
        publish_state(&subs);
        let st = load_state();
        assert_no_secret(&st.subs[0].error);
        let vpn_json = fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap();
        assert_no_secret(&vpn_json);
    }

    #[test]
    fn sec01_fetch_failure_never_leaks_subscription_url() {
        let _g = EnvGuard::vpn_dirs();
        let url = format!("http://user:pass@example.invalid/sub/{SECRET}?token={SECRET}");
        let logs = std::sync::Mutex::new(Vec::<String>::new());
        let log = |s: &str| logs.lock().unwrap().push(s.to_string());
        let subs_path = format!("{}/subs.json", etc());
        let raw = serde_json::json!({
            "active": "s1",
            "list": [{
                "id": "s1",
                "name": "test",
                "url": url,
                "updated": 0
            }]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        let c = Config::defaults(vec![]);
        update_subs(&c, &log, true).unwrap();
        let subs = load_subs().unwrap();
        assert_no_secret(&subs.list[0].error);
        for line in logs.lock().unwrap().iter() {
            assert_no_secret(line);
        }
        let vpn_json = fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap();
        assert_no_secret(&vpn_json);
    }

    // --- SEC-02 ---
    #[test]
    fn sec02_rejects_missing_or_bad_digest() {
        assert!(verify_core_gz_digest("sha256:abc", b"data").is_err());
        assert!(verify_core_gz_digest("md5:0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef", b"x").is_err());
        let empty = "sha256:0000000000000000000000000000000000000000000000000000000000000000";
        assert!(verify_core_gz_digest(empty, b"not-zero").is_err());
    }

    #[test]
    fn sec02_accepts_matching_digest() {
        use sha2::Digest;
        let payload = b"fake-gz-payload";
        let hash: String = sha2::Sha256::digest(payload).iter().map(|b| format!("{b:02x}")).collect();
        verify_core_gz_digest(&format!("sha256:{hash}"), payload).unwrap();
    }

    // --- PARSE-01 ---
    #[test]
    fn parse01_pct_decode_utf8_percent() {
        assert_eq!(pct_decode("%D0%9F%D1%80%D0%BE%D1%84%D0%B8%D0%BB%D1%8C"), "Профиль");
    }

    #[test]
    fn parse01_pct_decode_malformed_percent_does_not_panic() {
        let out = pct_decode("name%ZZ%GG%");
        assert!(out.contains('%'));
    }

    // --- GEO-01 ---
    #[test]
    fn geo01_rejects_truncated_geo_file() {
        assert!(validate_geo_file("GeoIP.dat", &[0u8; 2048]).is_err());
    }

    // --- SEC-03 ---
    #[test]
    fn sec03_rejects_http_subscription_url() {
        assert!(subscription_url("http://example.com/sub").is_err());
        assert!(subscription_url("https://example.com/sub").is_ok());
    }

    // --- SEC-05 ---
    #[test]
    fn sec05_strips_control_chars_from_profile_name() {
        assert_eq!(sanitize_profile_name("test\u{202e}name"), "testname");
    }

    // --- SEC-06 ---
    #[test]
    fn vpn_rules_io_failure_is_not_empty_rules() {
        let _g = EnvGuard::vpn_dirs();
        let path = rules_path();
        fs::create_dir(&path).unwrap();
        assert!(user_rules().unwrap_err().contains("rules.txt"));
        assert!(edit_rules().is_err());
    }

    #[test]
    fn vpn_rules_existing_file_is_read() {
        let _g = EnvGuard::vpn_dirs();
        fs::write(rules_path(), "# comment\nDOMAIN-SUFFIX,example.org,DIRECT\n").unwrap();
        assert_eq!(user_rules().unwrap(), vec!["DOMAIN-SUFFIX,example.org,DIRECT"]);
    }

    #[test]
    fn data02_missing_subs_is_ok_corrupt_is_error() {
        let _g = EnvGuard::vpn_dirs();
        assert!(load_subs().unwrap().list.is_empty());
        write_private(&format!("{}/subs.json", etc()), b"{not-json").unwrap();
        assert!(load_subs().is_err());
    }

    // --- VPN-02 ---
    #[test]
    fn vpn02_proc_ipv6_addr_parses_loopback_listener() {
        // ::1 в формате /proc/net/tcp6 (native endian по словам)
        let raw = "00000000000000000000000001000000";
        assert!(proc_ipv6_addr(raw).unwrap().is_loopback());
    }

    #[test]
    fn vpn02_listen_ports_keep_local_and_drop_foreign() {
        let header = "sl local rem st tx rx tr retr uid timeout inode";
        let tcp = format!(
            "{header}\n 0: 0100007F:1F90 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 1: 00000000:2382 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 2: 0100007F:1F92 08080808:0050 01 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n 3: 08080808:1F93 00000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 42 1 0 100 0 0 10 0\n"
        );
        let tcp6 = format!(
            "{header}\n 0: 00000000000000000000000001000000:1F91 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 43 1 0 100 0 0 10 0\n 1: 00000000000000000000000001000000:1F94 00000000000000000000000000000000:0000 0A 00000000:00000000 00:00000000 00000000 0 0 99 1 0 100 0 0 10 0\n"
        );
        let inodes = ["42".to_string(), "43".to_string()].into_iter().collect();
        let ports = flclash_listen_ports(&tcp, &tcp6, &inodes);
        assert!(ports.contains(&(0x1F90, false)), "{ports:?}");
        assert!(ports.contains(&(0x1F91, true)), "{ports:?}");
        assert!(!ports.iter().any(|(port, _)| *port == 0x2382 || *port == 0x1F92 || *port == 0x1F93 || *port == 0x1F94), "{ports:?}");
    }

    #[test]
    fn vpn03_restart_failure_is_recorded_and_success_applies_tag() {
        let mut failed = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        let event = note_core_update(&mut failed, true, true, Err("код 1".into()), "FlClash 9").unwrap();
        assert!(event.contains("перезапуск не удался"));
        assert!(event.contains("код 1"));
        assert!(failed.flclash_applied.is_empty());
        let mut ok = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        assert!(note_core_update(&mut ok, true, true, Ok(()), "FlClash 9").is_none());
        assert_eq!(ok.flclash_applied, "FlClash 9");
        assert!(ok.event.contains("обновлено до v1.2.3"));
        let mut inactive = VpnState { core_version: "v1.2.3".into(), ..Default::default() };
        assert!(note_core_update(&mut inactive, true, false, Ok(()), "FlClash 9").is_none());
        assert!(inactive.event.contains("не запущена"));
    }

    fn proto_varint(mut n: usize) -> Vec<u8> {
        let mut out = vec![];
        loop {
            let mut byte = (n & 0x7f) as u8;
            n >>= 7;
            if n != 0 {
                byte |= 0x80;
            }
            out.push(byte);
            if n == 0 {
                break;
            }
        }
        out
    }

    fn proto_bytes(field: u32, data: &[u8]) -> Vec<u8> {
        let mut out = proto_varint(((field as usize) << 3) | 2);
        out.extend(proto_varint(data.len()));
        out.extend_from_slice(data);
        out
    }

    fn sample_geoip() -> Vec<u8> {
        let mut cidr = proto_bytes(1, &[1, 2, 3, 0]);
        cidr.extend(proto_varint(2 << 3));
        cidr.push(24);
        let mut entry = proto_bytes(1, b"US");
        entry.extend(proto_bytes(2, &cidr));
        entry.extend(proto_bytes(15, &vec![b'x'; 1200]));
        proto_bytes(1, &entry)
    }

    #[test]
    fn geo01_truncated_download_keeps_previous_file() {
        let base = std::env::temp_dir().join(format!("cm-geo-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let path = base.join("GeoIP.dat");
        fs::write(&path, b"KEEP-PREVIOUS").unwrap();
        let valid = sample_geoip();
        assert!(valid.len() > 1024);
        assert!(validate_geo_file("GeoIP.dat", &valid).is_ok());
        let truncated = valid[..100].to_vec();
        assert!(save_geo_result(&path, "GeoIP.dat", Ok(truncated)).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"KEEP-PREVIOUS");
        assert!(save_geo_result(&path, "GeoIP.dat", Err("ответ превышает лимит".into())).is_err());
        assert_eq!(fs::read(&path).unwrap(), b"KEEP-PREVIOUS");
        save_geo_result(&path, "GeoIP.dat", Ok(valid.clone())).unwrap();
        assert_eq!(fs::read(&path).unwrap(), valid);
        let huge = format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\nx", (100 << 20) + 1);
        let response: ureq::Response = huge.parse().unwrap();
        let err = read_limited(response, 100 << 20).unwrap_err();
        assert!(err.contains("лимит"), "{err}");
        let short = "HTTP/1.1 200 OK\r\nContent-Length: 100\r\n\r\nshort".parse::<ureq::Response>().unwrap();
        let err = read_limited(short, 100 << 20).unwrap_err();
        assert!(err.contains("closed before all bytes"), "{err}");
        let _ = fs::remove_dir_all(&base);
    }

    // --- DATA-01 ---
    #[test]
    fn data01_subscriptions_lock_serializes() {
        let _g = EnvGuard::vpn_dirs();
        let l1 = subscriptions_lock(false).unwrap();
        assert!(subscriptions_lock(false).is_err());
        drop(l1);
        assert!(subscriptions_lock(false).is_ok());
    }

    #[test]
    fn vpn_config_lock_needs_only_writable_vpn_home() {
        let _g = EnvGuard::vpn_dirs();
        // ExecStartPre cannot write to the parent state directory. Make it
        // impossible to open anything there even when tests run as root.
        let state = state_dir();
        fs::remove_dir(&state).unwrap();
        fs::write(&state, b"unwritable parent fixture").unwrap();
        let first = vpn_config_lock(false).unwrap();
        assert!(vpn_config_lock(false).is_err());
        assert!(Path::new(&home()).join(".vpn-config.lock").is_file());
        drop(first);
        assert!(vpn_config_lock(false).is_ok());
        assert!(lock(false).is_err());
    }

    // --- VPN-01 ---
    #[test]
    fn vpn01_delete_last_subscription_clears_list() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        let raw = serde_json::json!({
            "active": "a1",
            "list": [{"id":"a1","name":"one","url":"https://example.com/sub","interval_h":24,"updated":0,"nodes":1,"kind":"yaml"}]
        });
        write_private(&subs_path, raw.to_string().as_bytes()).unwrap();
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::write(format!("{}/profiles/a1.yaml", home()), "proxies: []\n").unwrap();
        let name = delete_sub(&SubRef::Index(0)).unwrap();
        assert_eq!(name, "one");
        assert!(load_subs().unwrap().list.is_empty());
    }

    // --- DATA-03 ---
    #[test]
    fn data03_delete_sub_persists_json_before_profile_removal() {
        let _g = EnvGuard::vpn_dirs();
        let subs_path = format!("{}/subs.json", etc());
        write_private(
            &subs_path,
            br#"{"active":"x","list":[{"id":"x","name":"n","url":"https://example.com/s","interval_h":1,"updated":0,"nodes":0,"kind":"yaml"}]}"#,
        )
        .unwrap();
        delete_sub(&SubRef::Index(0)).unwrap();
        let on_disk = fs::read_to_string(&subs_path).unwrap();
        assert!(on_disk.contains("\"list\":[]") || on_disk.contains("\"list\": []"));
    }

    // ---------- 0.2.7 ----------

    fn write_subs(json: serde_json::Value) {
        write_private(&format!("{}/subs.json", etc()), json.to_string().as_bytes()).unwrap();
    }

    fn switch_fixture(core_script: &str) {
        two_subs();
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::create_dir_all(format!("{}/bin", home())).unwrap();
        for id in ["a1", "b2", "c3"] {
            fs::write(format!("{}/profiles/{id}.yaml", home()), clash_profile("")).unwrap();
        }
        fs::write(core_bin(), format!("#!/bin/sh\n{core_script}\n")).unwrap();
        fs::set_permissions(core_bin(), fs::Permissions::from_mode(0o700)).unwrap();
        write_private(&config_path(), b"previous configuration").unwrap();
    }
    #[test]
    fn subscription_validation_failure_preserves_choice_and_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("echo 'bad group'; exit 1");
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || panic!("invalid configuration must never reach running core"));
        assert!(result.unwrap_err().contains("bad group"));
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
        let failure = last_failure().unwrap();
        assert_eq!(failure.stage, "validate");
        assert_eq!(failure.subscription, "two");
        assert!(failure.resolved.is_none());
        assert!(fs::read_dir(home()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".candidate-")));
    }
    #[test]
    fn failed_reload_restores_disk_and_running_core() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let mut calls = 0;
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || {
            calls += 1;
            if calls == 1 {
                assert_ne!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
                assert_eq!(load_subs().unwrap().active, "a1", "selection is not committed before reload");
                Err("reload rejected".into())
            } else {
                assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
                Ok(())
            }
        });
        assert!(result.unwrap_err().contains("restored"));
        assert_eq!(calls, 2);
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(last_failure().unwrap().stage, "apply");
    }
    #[test]
    fn rollback_failure_is_explicit_and_success_resolves_saved_error() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let result = switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || Err("core rejected".into()));
        assert!(result.unwrap_err().contains("rollback failed"));
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(switch_subscription(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true, &mut || Ok(())).unwrap(), "two");
        assert_eq!(load_subs().unwrap().active, "b2");
        assert_eq!(load_state().subs.iter().find(|s| s.active).unwrap().id, "b2");
        assert!(last_failure().unwrap().resolved.is_some());
    }
    #[test]
    fn failed_commit_after_rename_restores_subscription_and_runtime() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let mut commits = 0;
        let mut reloads = 0;
        let result = switch_subscription_with(&SubRef::Id("b2".into()), &Config::defaults(vec![]), true,
            &mut || { reloads += 1; Ok(()) },
            &mut |subs| {
                commits += 1;
                save_subs(subs)?;
                if commits == 1 { Err("directory sync failed after rename".into()) } else { Ok(()) }
            });
        assert!(result.unwrap_err().contains("restored"));
        assert_eq!(commits, 2);
        assert_eq!(reloads, 2);
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
        assert_eq!(load_state().subs.iter().find(|s| s.active).unwrap().id, "a1");
        assert_eq!(last_failure().unwrap().stage, "save");
    }
    #[test]
    fn persisted_diagnostic_redacts_profile_credentials_and_subscription_urls() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("exit 0");
        let sub = load_subs().unwrap().list[1].clone();
        fs::write(profile_path(&sub, "clash"), clash_profile(&format!("payload: {{uuid: {SECRET}}}\n"))).unwrap();
        store_failure_locked("validate", Some(&sub), &format!("invalid uuid {SECRET}"));
        let failure = last_failure().unwrap();
        assert_no_secret(&failure.reason);
        store_failure_locked("validate", Some(&sub), &format!("GET {}?token={SECRET} failed", sub.url));
        assert_no_secret(&last_failure().unwrap().reason);
        store_failure_locked("validate", Some(&sub), &format!("GET {}?token=QUERY_ONLY_CREDENTIAL failed", sub.url));
        assert!(!last_failure().unwrap().reason.contains("QUERY_ONLY_CREDENTIAL"));
        use std::os::unix::fs::MetadataExt;
        assert_eq!(fs::metadata(failure_path()).unwrap().mode() & 0o777, 0o600);
    }
    #[test]
    fn readiness_retries_and_has_a_deadline() {
        let mut calls = 0;
        wait_ready_with(Duration::from_secs(1), || { calls += 1; if calls < 3 { Err("starting".into()) } else { Ok(()) } }).unwrap();
        assert_eq!(calls, 3);
        let before = std::time::Instant::now();
        let error = wait_ready_with(Duration::from_millis(150), || Err("not listening".into())).unwrap_err();
        assert!(error.contains("API is not ready"));
        assert!(before.elapsed() < Duration::from_secs(1));
    }
    #[test]
    fn api_readiness_does_not_claim_remote_server_connectivity() {
        let _isolation = crate::common::contract_fixtures::isolation_lock();
        crate::i18n::set(crate::i18n::Lang::Ru);
        let mut snapshot = Snapshot { running: true, groups: vec![Group { name: "Proxy".into(), kind: "Selector".into(), now: "n1".into(), ..Default::default() }], ..Default::default() };
        assert!(snapshot.health_line().contains("ещё не проверена"));
        snapshot.delay.insert("n1".into(), 0);
        assert!(snapshot.health_line().contains("ошибкой"));
        snapshot.delay.insert("n1".into(), 42);
        assert!(snapshot.health_line().contains("42 мс"));
        snapshot.running = false;
        assert!(snapshot.health_line().contains("не готов"));
    }

    fn two_subs() {
        write_subs(serde_json::json!({
            "active": "a1",
            "list": [
                {"id":"a1","name":"one","url":"https://example.com/1","kind":"clash"},
                {"id":"b2","name":"two","url":"https://example.com/2","kind":"clash"},
                {"id":"c3","name":"three","url":"https://example.com/3","kind":"clash"}
            ]
        }));
    }
