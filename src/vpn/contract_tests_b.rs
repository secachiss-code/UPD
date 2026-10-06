    /// B22: подтверждение адресует id; изменение списка между подтверждением и удалением не задевает другую запись.
    #[test]
    fn b22_delete_and_use_by_id_survive_list_change() {
        let _g = EnvGuard::vpn_dirs();
        two_subs();
        // пользователь подтвердил «three» (строка 3); другой процесс удалил «one»
        delete_sub(&SubRef::Id("a1".into())).unwrap();
        assert_eq!(delete_sub(&SubRef::Id("c3".into())).unwrap(), "three");
        let left = load_subs().unwrap();
        assert_eq!(left.list.iter().map(|s| s.id.as_str()).collect::<Vec<_>>(), vec!["b2"]);
        // записи уже нет — ничего не удаляется и не активируется
        assert!(delete_sub(&SubRef::Id("c3".into())).is_err());
        assert!(use_sub(&SubRef::Id("a1".into()), &Config::defaults(vec![])).is_err());
        assert_eq!(load_subs().unwrap().list.len(), 1);
        assert_eq!(load_subs().unwrap().active, "b2");
        assert_eq!(load_state().subs[0].id, "b2", "id публикуется для TUI");
        assert_eq!(SubRef::parse("id:b2"), Some(SubRef::Id("b2".into())));
        assert_eq!(SubRef::parse("2"), Some(SubRef::Index(1)));
        assert_eq!(SubRef::parse("0"), None);
        assert_eq!(SubRef::parse("id:"), None);
    }

    fn clash_profile(extra: &str) -> String {
        format!("proxies:\n  - {{name: n1, type: ss, server: 1.2.3.4, port: 443, cipher: aes-128-gcm, password: x}}\nproxy-groups:\n  - {{name: Proxy, type: select, proxies: [n1]}}\nrules:\n  - MATCH,Proxy\n{extra}")
    }

    fn build_with(profile: &str, c: &Config) -> Result<Mapping, String> {
        write_subs(serde_json::json!({"active":"p1","list":[{"id":"p1","name":"p","url":"https://example.com/p","kind":"clash"}]}));
        fs::create_dir_all(format!("{}/profiles", home())).unwrap();
        fs::write(format!("{}/profiles/p1.yaml", home()), profile).unwrap();
        let y = build_config(c)?;
        match serde_yaml::from_str::<Value>(&y).unwrap() {
            Value::Mapping(m) => Ok(m),
            _ => panic!("не mapping"),
        }
    }

    /// B01: listeners, свой DNS, skip-auth-prefixes и прочие ключи подписки не попадают в конфиг.
    #[test]
    fn russian_auto_setting_preserves_manual_nodes_and_provider_filtering() {
        let _g = EnvGuard::vpn_dirs();
        let profile = clash_profile("proxy-providers:\n  remote: {type: http, url: 'https://example.com/nodes', path: ./providers/remote.yaml}\n");
        let mut c = Config::defaults(vec![]);
        let allowed = build_with(&profile, &c).unwrap();
        let filter = |m: &Mapping| m["proxy-groups"].as_sequence().unwrap().iter()
            .find(|g| g["name"].as_str() == Some(AUTO_GROUP)).unwrap()["exclude-filter"].as_str().unwrap().to_string();
        assert!(!filter(&allowed).contains("🇷🇺"));
        c.set("vpn_auto_allow_ru", "0").unwrap();
        assert!(c.set("vpn_auto_allow_ru", "2").is_err());
        let excluded = build_with(&profile, &c).unwrap();
        assert!(filter(&excluded).contains("🇷🇺"));
        assert_eq!(allowed["proxies"], excluded["proxies"]);
        assert_eq!(allowed["proxy-providers"], excluded["proxy-providers"]);
        let groups = excluded["proxy-groups"].as_sequence().unwrap();
        assert_eq!(groups[0]["include-all"], Value::Bool(true));
        let manual = groups.iter().find(|g| g["name"].as_str() == Some("Proxy")).unwrap();
        assert!(manual.get("exclude-filter").is_none());
        c.set("vpn_auto_allow_ru", "1").unwrap();
        assert_eq!(filter(&build_with(&profile, &c).unwrap()), filter(&allowed));
        let mut legacy = serde_json::to_value(&c).unwrap();
        legacy.as_object_mut().unwrap().remove("vpn_auto_allow_ru");
        assert!(serde_json::from_value::<Config>(legacy).unwrap().vpn_auto_allow_ru);
    }

    #[test]
    fn b01_hostile_profile_keys_are_dropped() {
        let _g = EnvGuard::vpn_dirs();
        let hostile = "listeners:\n  - {name: open, type: mixed, port: 7777, listen: 0.0.0.0}\ndns: {enable: true, listen: 0.0.0.0:53}\nskip-auth-prefixes: [0.0.0.0/0]\nexternal-controller: 0.0.0.0:9090\nexternal-controller-cors: {allow-origins: ['*']}\nhosts: {a: 1.1.1.1}\nsniffer: {enable: true}\niptables: {enable: true}\nebpf: {redirect-to-tun: [eth0]}\nauthentication: ['u:p']\nexternal-ui: /etc\n";
        let mut c = Config::defaults(vec![]);
        c.vpn_dns = false;
        let m = build_with(&clash_profile(hostile), &c).unwrap();
        for key in ["listeners", "dns", "skip-auth-prefixes", "external-controller", "external-controller-cors", "hosts", "sniffer", "iptables", "ebpf", "authentication", "external-ui"] {
            assert!(!m.contains_key(key), "{key} остался");
        }
        assert_eq!(m.get("bind-address").and_then(Value::as_str), Some("127.0.0.1"));
        assert_eq!(m.get("geo-auto-update").and_then(Value::as_bool), Some(false));
        assert!(m.get("proxies").and_then(Value::as_sequence).map(|s| s.len() == 1).unwrap_or(false));
        // B02: только Unix-сокет в каталоге службы
        assert!(m.get("external-controller-unix").and_then(Value::as_str).unwrap().starts_with(&home()));
        assert!(!m.contains_key("secret"), "секрет не нужен: API только через сокет в закрытом каталоге");
        assert!(!Path::new(&format!("{}/secret", etc())).exists(), "сборка конфига ничего не пишет в /etc");
        c.vpn_dns = true;
        c.vpn_allow_lan = true;
        let m = build_with(&clash_profile(hostile), &c).unwrap();
        assert_eq!(m.get("dns").and_then(|d| d.get("listen")).and_then(Value::as_str), Some("127.0.0.1:1053"), "DNS — свой, не из подписки");
        assert_eq!(m.get("bind-address").and_then(Value::as_str), Some("*"));
    }

    /// B01: provider с путём вне каталога профиля или файловый без пути отвергается.
    #[test]
    fn b01_provider_paths_are_confined() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for bad in ["/etc/shadow", "../secret", "bin/mihomo", "config.yaml", "providers/../../etc/x", "profiles", "rule-sets", "rule-sets/../../etc/x", "/rule-sets/x.mrs"] {
            let p = format!("proxy-providers:\n  x: {{type: file, path: '{bad}'}}\n");
            assert!(build_with(&clash_profile(&p), &c).is_err(), "{bad}");
        }
        assert!(build_with(&clash_profile("rule-providers:\n  r: {type: file}\n"), &c).is_err());
        assert!(build_with(&clash_profile("rule-providers:\n  r: {type: exec, path: ./rules/a}\n"), &c).is_err());
        let ok = "proxy-providers:\n  x: {type: http, url: 'https://e.com/p', path: ./providers/x.yaml}\nrule-providers:\n  r: {type: http, url: 'https://e.com/r', behavior: domain}\n";
        let m = build_with(&clash_profile(ok), &c).unwrap();
        assert!(m.contains_key("proxy-providers") && m.contains_key("rule-providers"));
    }

    #[test]
    fn rule_sets_provider_is_isolated_without_changing_format() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  telegram_domains: {type: http, url: 'https://example.com/telegram_domains.mrs', behavior: domain, format: mrs, path: ./rule-sets/telegram_domains.mrs}\n");
        let m = build_with(&profile, &c).unwrap();
        let provider = &m["rule-providers"]["telegram_domains"];
        assert!(provider["path"].as_str().unwrap().starts_with("./providers/rule-providers/"));
        assert!(provider["path"].as_str().unwrap().ends_with("/rule-sets/telegram_domains.mrs"));
        assert_eq!(provider["format"].as_str(), Some("mrs"));
    }

    #[test]
    fn custom_http_provider_directories_are_relocated_to_cache() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for path in ["./oisd/big.mrs", "./custom/nested/telegram.mrs", "cache.yaml", "bin/mihomo"] {
            let profile = clash_profile(&format!("rule-providers:\n  oisd_big: {{type: http, url: 'https://example.com/big.mrs', behavior: domain, format: mrs, path: '{path}'}}\n"));
            let m = build_with(&profile, &c).unwrap();
            let provider = &m["rule-providers"]["oisd_big"];
            let cache = provider["path"].as_str().unwrap();
            assert!(cache.starts_with("./providers/rule-providers/") && provider_path_ok(cache), "{cache}");
            assert!(cache.ends_with(path.trim_start_matches("./")), "{cache}");
            assert_eq!(provider["format"].as_str(), Some("mrs"));
            assert_eq!(provider["url"].as_str(), Some("https://example.com/big.mrs"));
            assert_eq!(build_with(&profile, &c).unwrap(), m, "кэш стабилен при повторной сборке");
        }
        assert_ne!(provider_cache_path("./oisd/big.mrs", "rule-providers", "https://a.example/big"), provider_cache_path("./oisd/big.mrs", "rule-providers", "https://b.example/big"));
        for path in ["/etc/shadow", "../secret", "oisd/../../config.yaml", "./", "", "oisd/bad\nfile"] {
            assert!(provider_cache_path(path, "rule-providers", "https://example.com/big").is_none(), "{path}");
        }
        for path in ["/etc/shadow", "../secret", "oisd/../../config.yaml"] {
            let profile = clash_profile(&format!("rule-providers:\n  r: {{type: http, url: 'https://example.com/big', path: '{path}'}}\n"));
            assert!(build_with(&profile, &c).is_err(), "{path}");
        }
    }

    #[test]
    fn http_cache_isolated_for_existing_paths_headers_and_names() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let path = |name: &str, url: &str, token: &str| {
            let profile = clash_profile(&format!("rule-providers:\n  {name}: {{type: http, url: '{url}', behavior: domain, path: ./rules/shared.yaml, header: {{Authorization: ['{token}']}}}}\n"));
            build_with(&profile, &c).unwrap()["rule-providers"][name]["path"].as_str().unwrap().to_string()
        };
        let original = path("a", "https://a.example/rules", "first");
        assert_ne!(original, path("a", "https://b.example/rules", "first"));
        assert_ne!(original, path("b", "https://a.example/rules", "first"));
        assert_ne!(original, path("a", "https://a.example/rules", "second"));
        assert!(!original.contains("first"));
    }

    #[test]
    fn file_provider_links_follow_cache_and_ambiguous_links_fail() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = "rule-providers:\n  remote: {type: http, url: 'https://example.com/a', behavior: domain, path: ./custom/a.yaml}\n  local: {type: file, behavior: domain, path: custom/a.yaml}\n";
        let m = build_with(&clash_profile(profile), &c).unwrap();
        assert_eq!(m["rule-providers"]["remote"]["path"], m["rule-providers"]["local"]["path"]);
        let ambiguous = format!("{profile}  other: {{type: http, url: 'https://other.example/a', behavior: domain, path: custom/a.yaml}}\n");
        assert!(build_with(&clash_profile(&ambiguous), &c).unwrap_err().contains("неоднозначный"));
    }

    #[test]
    fn local_provider_file_errors_and_symlinks_are_checked() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  local: {type: file, behavior: domain, path: ./rules/local.yaml}\n");
        assert!(build_with(&profile, &c).unwrap_err().contains("файл не найден"));
        fs::create_dir_all(format!("{}/rules", home())).unwrap();
        fs::write(format!("{}/rules/local.yaml", home()), "payload: [example.com]\n").unwrap();
        assert!(build_with(&profile, &c).is_ok());
        fs::remove_file(format!("{}/rules/local.yaml", home())).unwrap();
        std::os::unix::fs::symlink("/etc/hosts", format!("{}/rules/local.yaml", home())).unwrap();
        assert!(build_with(&profile, &c).unwrap_err().contains("вне каталога"));
    }

    #[test]
    fn provider_types_formats_and_references_are_checked() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        for (provider, message) in [
            ("{type: http, behavior: domain}", "URL"),
            ("{type: http, url: 'file:///etc/hosts', behavior: domain}", "URL"),
            ("{type: http, url: 'https://example.com/r', behavior: classical, format: mrs}", "behavior"),
            ("{type: http, url: 'https://example.com/r', behavior: domain, format: wrong}", "формат"),
        ] {
            let profile = clash_profile(&format!("rule-providers:\n  invalid: {provider}\n"));
            assert!(build_with(&profile, &c).unwrap_err().contains(message));
        }
        let inline = clash_profile("rule-providers:\n  inline: {type: inline, behavior: domain, path: /unused, payload: [example.com]}\n");
        assert!(!build_with(&inline, &c).unwrap()["rule-providers"]["inline"].as_mapping().unwrap().contains_key("path"));
        assert!(build_with(&clash_profile("sub-rules:\n  local: ['RULE-SET,missing,Proxy']\n"), &c).is_err());
        assert!(build_with(&clash_profile("proxy-groups:\n  - {name: Group, type: select, use: [missing]}\n"), &c).unwrap_err().contains("missing"));
    }

    #[test]
    fn doctor_reports_each_profile_without_changing_selection_or_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("echo 'core rejected fixture' >&2; exit 1");
        let previous = fs::read_to_string(config_path()).unwrap();
        let checks = doctor(&Config::defaults(vec![]), None).unwrap();
        assert_eq!(checks.len(), load_subs().unwrap().list.len());
        assert!(checks.iter().all(|c| c.error.as_deref().unwrap().contains("core rejected")), "{checks:?}");
        assert_eq!(load_subs().unwrap().active, "a1");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), previous);
        assert_eq!(doctor(&Config::defaults(vec![]), Some(&SubRef::Id("b2".into()))).unwrap().len(), 1);
        assert!(fs::read_dir(home()).unwrap().flatten().all(|e| !e.file_name().to_string_lossy().starts_with(".candidate-")));
    }

    #[test]
    fn prepare_validation_failure_keeps_previous_config() {
        let _g = EnvGuard::vpn_dirs();
        switch_fixture("if [ \"$1\" = '-v' ]; then echo 'mihomo v1.19.0'; exit 0; fi; echo 'bad rule provider' >&2; exit 1");
        let mut c = Config::defaults(vec![]);
        c.vpn_tun = false;
        c.vpn_port = 47_123;
        c.vpn_dns = false;
        let error = prepare(&c, &|_| {}).unwrap_err();
        assert!(error.contains("bad rule provider"), "{error}");
        assert_eq!(fs::read_to_string(config_path()).unwrap(), "previous configuration");
    }

    #[test]
    fn diagnostic_hides_provider_headers_and_age_secrets() {
        let profile = serde_yaml::from_str::<Value>("proxy-providers:\n  a: {type: http, age-secret-key: 'AGE-SECRET-123', header: {Authorization: ['Bearer abcsecret'], X-Token: ['headersecret']}}\n").unwrap();
        let mut text = "invalid AGE-SECRET-123 Bearer abcsecret headersecret".to_string();
        redact_credentials(&profile, &mut text);
        assert!(!text.contains("SECRET-123") && !text.contains("abcsecret") && !text.contains("headersecret"));
    }

    #[test]
    fn subscription_negotiation_skips_placeholder_and_html_then_accepts_mihomo() {
        let placeholder = "proxies: [{name: 'unsupported client', type: ss, server: 127.0.0.1, port: 1}]";
        let mut attempted = Vec::new();
        let (body, (metadata, kind, nodes)) = negotiate_subscription("Custom/1", |agent| {
            attempted.push(agent.to_string());
            Ok((match attempted.len() { 1 => placeholder.into(), 2 => "<html>subscription page</html>".into(), _ => clash_profile("") }, attempted.len()))
        }).unwrap();
        assert_eq!(attempted, ["Custom/1", UA, "ClashMeta/1.19.32"]);
        assert_eq!((metadata, kind.as_str(), nodes), (3, "clash", 1));
        assert!(!body.contains("unsupported client"));
    }

    #[test]
    fn subscription_negotiation_accepts_uri_base64_and_stops_on_auth_failure() {
        let mut calls = 0;
        let (_, (_, kind, nodes)) = negotiate_subscription("", |_| {
            calls += 1;
            Ok(("dmxlc3M6Ly91c2VyQGV4YW1wbGUuY29tOjQ0MyNOb2Rl".into(), ()))
        }).unwrap();
        assert_eq!((calls, kind.as_str(), nodes), (1, "uri", 1));
        calls = 0;
        let error = negotiate_subscription::<()>("", |_| { calls += 1; Err("HTTP 403".into()) }).unwrap_err();
        assert_eq!(error, "HTTP 403");
        assert_eq!(calls, 1);
        calls = 0;
        assert!(negotiate_subscription("", |_| { calls += 1; Ok(("<html>unsupported</html>".into(), ())) }).is_err());
        assert_eq!(calls, SUBSCRIPTION_AGENTS.len());
    }

    #[test]
    fn unsupported_client_placeholder_and_html_are_not_valid_subscriptions() {
        let _g = EnvGuard::vpn_dirs();
        let body = clash_profile("").replace("name: n1", "name: Приложение не поддерживается");
        assert!(classify_sub(&body).unwrap_err().contains("User-Agent"));
        assert!(build_with(&body, &Config::defaults(vec![])).unwrap_err().contains("заглушку"));
        assert!(classify_sub("<html><a href=\"https://example.com\">Error</a></html>").is_err());
        assert!(classify_sub("url: https://example.com/rules\ninvalid: [").is_err());
        assert_eq!(classify_sub("vless://example-user@example.com:443#Node\n").unwrap(), ("uri".into(), 1));
    }

    #[test]
    fn core_validation_keeps_real_error_before_generic_failure_line() {
        assert_eq!(core_validation_reason("", "level=error msg=\"proxy group: node not found\"\nconfiguration file /tmp/test.yaml test failed\n"), "level=error msg=\"proxy group: node not found\"");
        assert_eq!(core_validation_reason("bad file\n", "configuration file /tmp/test.yaml test failed\n"), "bad file");
    }

    #[test]
    fn http_provider_empty_path_uses_private_default_cache() {
        let _g = EnvGuard::vpn_dirs();
        let c = Config::defaults(vec![]);
        let profile = clash_profile("rule-providers:\n  remote: {type: http, url: 'https://example.com/r', behavior: domain, path: ''}\n");
        let m = build_with(&profile, &c).unwrap();
        assert!(m["rule-providers"]["remote"]["path"].as_str().unwrap().ends_with("/provider.yaml"));
        let wrong = profile.replace("path: ''", "path: 123");
        assert!(build_with(&wrong, &c).is_err());
    }

    #[test]
    fn per_subscription_user_agent_is_validated_and_persisted_privately() {
        let _g = EnvGuard::vpn_dirs();
        two_subs();
        let reference = SubRef::Id("b2".into());
        set_user_agent(&reference, "mihomo/1.19.0").unwrap();
        let subs = load_subs().unwrap();
        assert_eq!(subs.active, "a1");
        assert_eq!(subs.list[0].user_agent, "");
        assert_eq!(subs.list[1].user_agent, "mihomo/1.19.0");
        for agent in ["", "bad\nheader", "bad\rheader", "кириллица"] { assert!(set_user_agent(&reference, agent).is_err()); }
        assert!(set_user_agent(&reference, &"x".repeat(513)).is_err());
        assert_eq!(load_subs().unwrap().list[1].user_agent, "mihomo/1.19.0");
        assert!(!fs::read_to_string(format!("{}/vpn.json", state_dir())).unwrap().contains("user_agent"));
    }

    #[test]
    fn subscription_request_uses_selected_user_agent() {
        use std::io::{BufRead, BufReader};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let worker = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
            let mut headers = String::new();
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() { break; }
                headers.push_str(&line);
            }
            std::io::Write::write_all(&mut stream, b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok").unwrap();
            headers
        });
        let _response = get_with_redirects_user_agent(&format!("http://{address}/profile"), 3, 7890, 0, "mihomo/1.19.0").unwrap();
        assert!(worker.join().unwrap().to_ascii_lowercase().contains("user-agent: mihomo/1.19.0\r\n"));
    }

    /// B23: «миллиард смешков» укладывается в лимит тела, но отвергается бюджетом до сборки конфига.
    #[test]
    fn b23_alias_bomb_is_rejected_by_budget() {
        let mut bomb = String::from("a: &a [x, x, x, x, x, x, x, x, x, x]\n");
        for i in 1..9 {
            let prev = (b'a' + i - 1) as char;
            let cur = (b'a' + i) as char;
            bomb += &format!("{cur}: &{cur} [*{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}, *{prev}]\n");
        }
        assert!(bomb.len() < 4096);
        let t0 = std::time::Instant::now();
        assert!(parse_profile(&bomb).is_err());
        assert!(t0.elapsed() < std::time::Duration::from_secs(5));
        // размножение, которое проходит предел повторов serde_yaml, останавливает наш бюджет узлов
        let base = "base: &b [".to_string() + &vec!["1"; 20_000].join(",") + "]\nx: [" + &vec!["*b"; 60].join(",") + "]\n";
        assert!(parse_profile(&base).unwrap_err().contains("больше"), "узлы");
        let big = "s: &s '".to_string() + &"x".repeat(1 << 20) + "'\nx: [" + &vec!["*s"; 70].join(",") + "]\n";
        assert!(parse_profile(&big).unwrap_err().contains("больше"), "байты");
        assert!(classify_sub(&format!("proxies: [1]\n{bomb}")).map(|(k, _)| k != "clash").unwrap_or(true));
        let deep = "a: ".to_string() + &"[".repeat(200) + &"]".repeat(200);
        assert!(parse_profile(&deep).is_err());
        let many_aliases = "a: &a 1\nb: [".to_string() + &vec!["*a"; YAML_MAX_ALIASES + 1].join(", ") + "]\n";
        assert!(parse_profile(&many_aliases).unwrap_err().contains("алиасов"));
        // обычные якоря и слияние << работают
        let v = parse_profile("base: &b {type: select}\ng: {<<: *b, name: G}\n").unwrap();
        assert_eq!(v["g"]["type"].as_str(), Some("select"));
    }

    /// B13: порт прокси, совпадающий с DNS, не попадает в YAML; конфиг cm при этом читается.
    #[test]
    fn b13_port_conflict_is_reported_by_vpn_build() {
        let _g = EnvGuard::vpn_dirs();
        let mut c = Config::defaults(vec![]);
        c.vpn_port = DNS_PORT;
        assert!(build_with(&clash_profile(""), &c).unwrap_err().contains("DNS"));
        c.vpn_dns = false;
        assert!(build_with(&clash_profile(""), &c).is_ok());
        c.vpn_port = 80;
        assert!(check_port(&c).is_err());
        c.vpn_port = 9097;
        assert!(check_port(&c).is_ok(), "9097 больше не занят контроллером");
    }

    /// B15: счётчики — целые без потери точности и без переполнения.
    #[test]
    fn b15_userinfo_counters_are_exact_and_safe() {
        let i = parse_userinfo("upload=9007199254740993; download=1; total=18446744073709551615; expire=1700000000");
        assert_eq!(i.upload, 9_007_199_254_740_993, "2^53+1 не округляется");
        assert_eq!(i.total, u64::MAX);
        let i = parse_userinfo("upload=18446744073709551615; download=18446744073709551615; total=18446744073709551615");
        assert!(i.nearly_exhausted());
        assert_eq!(fmt_bytes_wide(i.used()), fmt_bytes(u64::MAX));
        let i = parse_userinfo("upload=-5; download=1.5; total=1e400; expire=-1");
        assert_eq!((i.upload, i.download, i.total, i.expire), (0, 0, 0, 0));
        assert_eq!(parse_userinfo("upload=1.0e3").upload, 1000);
        let small = SubInfo { upload: 50, download: 39, total: 100, expire: 0 };
        assert!(!small.nearly_exhausted());
        assert!(SubInfo { download: 40, ..small }.nearly_exhausted());
    }

    /// B03: распаковка останавливается на пределе, до записи бинарника.
    #[test]
    fn b03_gunzip_is_limited() {
        use std::io::Write;
        let mut enc = flate2::write::GzEncoder::new(vec![], flate2::Compression::best());
        enc.write_all(&vec![0u8; 4 << 20]).unwrap();
        let gz = enc.finish().unwrap();
        assert!(gz.len() < 64 << 10);
        assert!(gunzip_limited(&gz, 1 << 20).unwrap_err().contains("больше"));
        assert_eq!(gunzip_limited(&gz, 4 << 20).unwrap().len(), 4 << 20);
    }

    /// B25: огромный интервал из заголовка не переполняет срок проверки.
    #[test]
    fn b25_huge_interval_does_not_overflow() {
        assert_eq!(hours_secs(i64::MAX), MAX_HOURS * 3600);
        assert_eq!(hours_secs(-5), 3600);
        assert!(!elapsed_at_least(now(), hours_secs(i64::MAX)));
        assert!(elapsed_at_least(i64::MIN, 1), "без переполнения на вычитании");
        let _g = EnvGuard::vpn_dirs();
        write_subs(serde_json::json!({"active":"a","list":[{"id":"a","name":"n","url":"https://example.invalid/s","updated": now(),"interval_h": i64::MAX}]}));
        assert!(!update_subs(&Config::defaults(vec![]), &|_| {}, false).unwrap(), "срок не подошёл, запросов нет");
    }

    fn proc_fixture(pid: &str, comm: &str, ours: bool, sockets: &[&str], tuns: &[&str]) -> ProcInfo {
        ProcInfo { pid: pid.into(), comm: comm.into(), ours, sockets: sockets.iter().map(|s| s.to_string()).collect(), tuns: tuns.iter().map(|s| s.to_string()).collect() }
    }

    /// B24: имя процесса без TUN и порта не мешает; настоящий занятый порт или чужой TUN — мешает.
    #[test]
    fn b24_conflict_needs_real_port_or_tun() {
        let c = Config::defaults(vec![]);
        let header = "sl local rem st tx rx tr retr uid timeout inode\n";
        // 0x1ED9 = 7897 (vpn_port по умолчанию), 0x041D = 1053
        let tcp = format!("{header} 0: 0100007F:1ED9 00000000:0000 0A 0:0 0:0 0 0 0 555 1\n");
        let udp = format!("{header} 0: 0100007F:041D 00000000:0000 07 0:0 0:0 0 0 0 777 1\n");
        let fake = [proc_fixture("10", "FlClashCore", false, &["1"], &[])];
        assert!(find_conflict(&c, &fake, &[(header, false), (header, true)]).is_none(), "подменённое имя без порта и TUN");
        let busy = [proc_fixture("11", "nc", false, &["555"], &[])];
        let msg = find_conflict(&c, &busy, &[(&tcp, false)]).unwrap();
        assert!(msg.contains("7897") && msg.contains("nc"), "{msg}");
        let ours = [proc_fixture("12", "mihomo", true, &["555", "777"], &["cm-vpn"])];
        assert!(find_conflict(&c, &ours, &[(&tcp, false), (&udp, true)]).is_none(), "свой mihomo — не конфликт");
        let dns = [proc_fixture("13", "dnsmasq", false, &["777"], &[])];
        assert!(find_conflict(&c, &dns, &[(&udp, true)]).unwrap().contains("1053"));
        let flclash = [proc_fixture("14", "FlClashCore", false, &[], &["FlClash"])];
        assert!(find_conflict(&c, &flclash, &[]).unwrap().contains("FlClash"));
        let mesh = [proc_fixture("15", "tailscaled", false, &[], &["tailscale0"])];
        assert!(find_conflict(&c, &mesh, &[]).is_none());
        let mut proxy = c.clone();
        proxy.vpn_tun = false;
        assert!(find_conflict(&proxy, &flclash, &[]).is_none(), "в режиме прокси TUN не мешает");
    }

    /// B02: клиент API ходит только в свой Unix-сокет, проверяет права и разбирает ответ; TCP не используется.
    #[test]
    fn b02_api_over_unix_socket() {
        use std::io::Write;
        use std::os::unix::net::UnixListener;
        let _g = EnvGuard::vpn_dirs();
        fs::set_permissions(home(), fs::Permissions::from_mode(0o700)).unwrap();
        let listener = UnixListener::bind(api_socket()).unwrap();
        // как у mihomo: сам сокет открыт всем, защищает каталог
        fs::set_permissions(api_socket(), fs::Permissions::from_mode(0o666)).unwrap();
        let server = std::thread::spawn(move || {
            let mut seen = vec![];
            for body in ["{\"version\":\"v1.19.0\"}", "chunked"] {
                let (mut s, _) = listener.accept().unwrap();
                let mut req = vec![0u8; 4096];
                let n = s.read(&mut req).unwrap();
                seen.push(String::from_utf8_lossy(&req[..n]).into_owned());
                if body == "chunked" {
                    s.write_all(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n5\r\n{\"a\":\r\n2\r\n1}\r\n0\r\n\r\n").unwrap();
                } else {
                    s.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\n\r\n{body}", body.len()).as_bytes()).unwrap();
                }
            }
            seen
        });
        assert_eq!(api("GET", "/version", None).unwrap()["version"], "v1.19.0");
        assert_eq!(api("PUT", "/configs?force=true", Some(serde_json::json!({"path":"x"}))).unwrap()["a"], 1);
        let seen = server.join().unwrap();
        assert!(seen[0].starts_with("GET /version HTTP/1.1\r\n"));
        assert!(!seen.iter().any(|r| r.contains("Authorization")), "секрет не отправляется");
        assert!(seen[1].ends_with("{\"path\":\"x\"}"));
        // каталог сокета доступен группе — запрос не отправляется
        fs::set_permissions(home(), fs::Permissions::from_mode(0o750)).unwrap();
        assert!(api("GET", "/version", None).unwrap_err().contains("чужой"));
        fs::set_permissions(home(), fs::Permissions::from_mode(0o700)).unwrap();
        fs::remove_file(api_socket()).unwrap();
        fs::write(api_socket(), b"").unwrap();
        assert!(api("GET", "/version", None).is_err(), "обычный файл вместо сокета");
        assert!(parse_http_response(b"HTTP/1.1 200 OK\r\nContent-Length: 10\r\n\r\nabc").is_err());
    }

    /// B14: prepare не качает ядро, а коротко отказывает, когда его нет.
    #[test]
    fn b14_prepare_without_core_fails_fast() {
        let _g = EnvGuard::vpn_dirs();
        let mut c = Config::defaults(vec![]);
        c.vpn_port = 47_123;
        c.vpn_dns = false;
        c.vpn_tun = false;
        let t0 = std::time::Instant::now();
        let err = prepare(&c, &|_| {}).unwrap_err();
        assert!(err.contains("core update"), "{err}");
        assert!(t0.elapsed() < std::time::Duration::from_secs(2));
    }
