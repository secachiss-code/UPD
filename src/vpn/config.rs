fn rules_path() -> String {
    format!("{}/rules.txt", etc())
}

/// Шаблон rules.txt на языке интерфейса (комментарии и имя группы в примере); сами правила от языка не зависят.
pub fn rules_template() -> String {
    format!(
        "{}\n{}\n{}\n# DOMAIN-SUFFIX,mirror.yandex.ru,DIRECT\n# DOMAIN-KEYWORD,torrent,DIRECT\n# GEOSITE,youtube,{}\n# IP-CIDR,10.8.0.0/16,DIRECT,no-resolve\n",
        t!("# cm VPN: свои правила — идут первыми, раньше правил подписки."),
        t!("# Формат mihomo: ТИП,значение,куда. Куда: DIRECT (напрямую), REJECT (блок) или имя группы/сервера."),
        t!("# Примеры:"),
        label(AUTO_GROUP)
    )
}

/// Нетронутый шаблон прежних версий: такой файл можно заменить шаблоном на текущем языке.
const RULES_TEMPLATE_OLD: &str = "# cm VPN: свои правила — идут первыми, раньше правил подписки.\n\
# Формат mihomo: ТИП,значение,куда. Куда: DIRECT (напрямую), REJECT (блок) или имя группы/сервера.\n\
# Примеры:\n\
# DOMAIN-SUFFIX,mirror.yandex.ru,DIRECT\n\
# DOMAIN-KEYWORD,torrent,DIRECT\n\
# GEOSITE,youtube,⚡ Авто\n\
# IP-CIDR,10.8.0.0/16,DIRECT,no-resolve\n";

/// Свои правила. Файла нет — правил нет: сборка конфига (в том числе в ExecStartPre, где /etc только для чтения)
/// ничего не пишет; шаблон создаёт редактор правил.
pub fn user_rules() -> Result<Vec<String>, String> {
    let p = rules_path();
    let text = match fs::read_to_string(&p) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(format!("{p}: {e}")),
    };
    // группу автовыбора в правилах можно назвать на любом языке интерфейса (и прежним «⚡ Авто»):
    // в конфиг mihomo идёт её постоянное имя, иначе mihomo отверг бы правило
    Ok(text
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(|l| l.split(',').map(|f| if is_auto_group(f.trim()) { AUTO_GROUP } else { f }).collect::<Vec<_>>().join(","))
        .collect())
}

pub fn edit_rules() -> Result<(), String> {
    let p = rules_path();
    // нет файла или в нём нетронутый шаблон прежней версии — пишем шаблон на текущем языке
    let untouched = fs::read_to_string(&p).map(|t| t == RULES_TEMPLATE_OLD).unwrap_or(false);
    if !Path::new(&p).exists() || untouched {
        private_dir(&etc())?;
        atomic_write(Path::new(&p), rules_template().as_bytes(), 0o600).map_err(|e| format!("{p}: {e}"))?;
    }
    user_rules()?;
    let editor = std::env::var("EDITOR").ok().filter(|e| !e.is_empty()).unwrap_or_else(|| ["nano", "micro", "vim", "vi"].into_iter().find(|e| have(e)).unwrap_or("vi").to_string());
    run(false, &[], &editor, &[&rules_path()])
}

fn k(s: &str) -> Value {
    Value::String(s.into())
}

fn seq(items: &[&str]) -> Value {
    Value::Sequence(items.iter().map(|s| k(s)).collect())
}

fn yaml_map(pairs: Vec<(&str, Value)>) -> Value {
    let mut m = Mapping::new();
    for (key, v) in pairs {
        m.insert(k(key), v);
    }
    Value::Mapping(m)
}

/// Главная группа: та, куда ведёт последнее правило MATCH; иначе первая группа-селектор.
fn main_group(groups: &[Value], rules: &[Value]) -> Option<String> {
    let names: Vec<String> = groups.iter().filter_map(|g| g.get("name")?.as_str().map(String::from)).collect();
    let by_match = rules.iter().rev().filter_map(Value::as_str).find_map(|r| r.strip_prefix("MATCH,").map(|t| t.split(',').next().unwrap_or("").trim().to_string()));
    if let Some(t) = by_match.filter(|t| names.contains(t)) {
        return Some(t);
    }
    groups.iter().find(|g| g.get("type").and_then(Value::as_str) == Some("select")).and_then(|g| g.get("name")?.as_str().map(String::from))
}

// ======================= разбор профиля с бюджетом =======================

/// Пределы разбора профиля. Сырое тело уже ограничено 32 МБ, но алиасы YAML разворачиваются при разборе:
/// узлы и байты строк считаются вместе с развёрнутыми алиасами, до построения дерева сверх бюджета.
const YAML_MAX_NODES: usize = 1_000_000;
const YAML_MAX_BYTES: usize = 64 << 20;
const YAML_MAX_DEPTH: usize = 64;
const YAML_MAX_ALIASES: usize = 10_000;

struct YamlBudget {
    nodes: std::cell::Cell<usize>,
    bytes: std::cell::Cell<usize>,
}

impl YamlBudget {
    fn take<E: serde::de::Error>(&self, bytes: usize) -> Result<(), E> {
        let nodes = self.nodes.get() + 1;
        let total = self.bytes.get().saturating_add(bytes);
        if nodes > YAML_MAX_NODES || total > YAML_MAX_BYTES {
            return Err(E::custom(t!("профиль больше допустимого после развёртывания алиасов YAML")));
        }
        self.nodes.set(nodes);
        self.bytes.set(total);
        Ok(())
    }
}

#[derive(Clone, Copy)]
struct Budgeted<'a> {
    budget: &'a YamlBudget,
    depth: usize,
}

impl<'a> Budgeted<'a> {
    fn child<E: serde::de::Error>(self) -> Result<Self, E> {
        if self.depth >= YAML_MAX_DEPTH {
            return Err(E::custom(t!("профиль YAML слишком глубокий")));
        }
        Ok(Budgeted { budget: self.budget, depth: self.depth + 1 })
    }
}

impl<'de> serde::de::DeserializeSeed<'de> for Budgeted<'_> {
    type Value = Value;
    fn deserialize<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
}

impl<'de> serde::de::Visitor<'de> for Budgeted<'_> {
    type Value = Value;
    fn expecting(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        f.write_str("YAML value")
    }
    fn visit_bool<E: serde::de::Error>(self, v: bool) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Bool(v))
    }
    fn visit_i64<E: serde::de::Error>(self, v: i64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_u64<E: serde::de::Error>(self, v: u64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_f64<E: serde::de::Error>(self, v: f64) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Number(v.into()))
    }
    fn visit_str<E: serde::de::Error>(self, v: &str) -> Result<Value, E> {
        self.budget.take(v.len())?;
        Ok(Value::String(v.to_string()))
    }
    fn visit_string<E: serde::de::Error>(self, v: String) -> Result<Value, E> {
        self.budget.take(v.len())?;
        Ok(Value::String(v))
    }
    fn visit_unit<E: serde::de::Error>(self) -> Result<Value, E> {
        self.budget.take(0)?;
        Ok(Value::Null)
    }
    fn visit_none<E: serde::de::Error>(self) -> Result<Value, E> {
        self.visit_unit()
    }
    fn visit_some<D: serde::Deserializer<'de>>(self, d: D) -> Result<Value, D::Error> {
        d.deserialize_any(self)
    }
    fn visit_seq<A: serde::de::SeqAccess<'de>>(self, mut seq: A) -> Result<Value, A::Error> {
        self.budget.take(0)?;
        let child = self.child()?;
        let mut out = vec![];
        while let Some(v) = seq.next_element_seed(child)? {
            out.push(v);
        }
        Ok(Value::Sequence(out))
    }
    fn visit_map<A: serde::de::MapAccess<'de>>(self, mut map: A) -> Result<Value, A::Error> {
        self.budget.take(0)?;
        let child = self.child()?;
        let mut out = Mapping::new();
        while let Some(key) = map.next_key_seed(child)? {
            let value = map.next_value_seed(child)?;
            out.insert(key, value);
        }
        Ok(Value::Mapping(out))
    }
    fn visit_enum<A: serde::de::EnumAccess<'de>>(self, data: A) -> Result<Value, A::Error> {
        use serde::de::VariantAccess;
        self.budget.take(0)?;
        let (tag, variant): (String, _) = data.variant()?;
        let value = variant.newtype_variant_seed(self.child()?)?;
        Ok(Value::Tagged(Box::new(serde_yaml::value::TaggedValue { tag: serde_yaml::value::Tag::new(tag), value })))
    }
}

/// Сколько алиасов `*имя` в тексте YAML (оценка сверху: считает и похожие места внутри строк в одинарных кавычках).
fn yaml_alias_count(body: &str) -> usize {
    let mut n = 0;
    for line in body.lines() {
        let b = line.as_bytes();
        let mut quote = 0u8;
        for i in 0..b.len() {
            let c = b[i];
            if quote != 0 {
                if c == quote {
                    quote = 0;
                }
                continue;
            }
            match c {
                b'"' => quote = b'"',
                b'#' if i == 0 || b[i - 1] == b' ' => break,
                b'*' if (i == 0 || matches!(b[i - 1], b' ' | b'\t' | b'[' | b'{' | b',' | b':' | b'-'))
                    && b.get(i + 1).map(|x| x.is_ascii_alphanumeric() || *x == b'_').unwrap_or(false) =>
                {
                    n += 1
                }
                _ => {}
            }
        }
    }
    n
}

/// Разбор профиля подписки с пределами узлов, байт, глубины и числа алиасов; ключи слияния `<<` раскрываются.
fn parse_profile(body: &str) -> Result<Value, String> {
    if yaml_alias_count(body) > YAML_MAX_ALIASES {
        return Err(t!("в профиле слишком много алиасов YAML").into());
    }
    let budget = YamlBudget { nodes: std::cell::Cell::new(0), bytes: std::cell::Cell::new(0) };
    let seed = Budgeted { budget: &budget, depth: 0 };
    let mut v = serde::de::DeserializeSeed::deserialize(seed, serde_yaml::Deserializer::from_str(body)).map_err(|e| e.to_string())?;
    v.apply_merge().map_err(|e| e.to_string())?;
    Ok(v)
}

/// Что берётся из профиля подписки: узлы, группы, правила и сетевые provider-ы. Остальное (listeners, dns, hosts,
/// sniffer, authentication, skip-auth-prefixes, external-controller-*, iptables, ebpf…) задаёт cm или не задаёт никто.
const PROFILE_KEYS: [&str; 6] = ["proxies", "proxy-groups", "rules", "sub-rules", "proxy-providers", "rule-providers"];

fn check_subscription_placeholder(m: &Mapping) -> Result<(), String> {
    let nodes = m.get("proxies").and_then(Value::as_sequence);
    if !m.get("proxy-providers").and_then(Value::as_mapping).is_some_and(|p| !p.is_empty())
        && nodes.is_some_and(|nodes| !nodes.is_empty() && nodes.iter().all(|node| {
        let name = node.get("name").and_then(Value::as_str).unwrap_or("").to_lowercase();
        ["приложение не поддерживается", "unsupported client", "client not supported"].iter().any(|message| name.contains(message))
    })) {
    return Err(t!("сервер подписки вернул заглушку: приложение не поддерживается; проверьте User-Agent подписки").into());
}
    Ok(())
}
/// Подкаталоги каталога VPN, куда provider-у можно писать кэш или откуда читать файл.
const PROVIDER_DIRS: [&str; 6] = ["profiles", "providers", "proxies", "rules", "ruleset", "rule-sets"];

/// Путь provider-а: относительный, без «..», внутри одного из PROVIDER_DIRS.
fn provider_path_ok(p: &str) -> bool {
    use std::path::Component;
    let path = Path::new(p);
    let mut parts = path.components().filter(|c| !matches!(c, Component::CurDir));
    let Some(Component::Normal(first)) = parts.next() else { return false };
    !path.is_absolute()
        && PROVIDER_DIRS.iter().any(|d| first == std::ffi::OsStr::new(d))
        && parts.clone().count() > 0
        && parts.all(|c| matches!(c, Component::Normal(_)))
}

fn relative_provider_path(path: &str) -> Option<PathBuf> {
    use std::path::Component;
    if path.chars().any(char::is_control) || path.ends_with('/') { return None; }
    let parts: Vec<_> = Path::new(path).components().filter(|p| !matches!(p, Component::CurDir)).collect();
    if parts.is_empty() || !parts.iter().all(|p| matches!(p, Component::Normal(_))) { return None; }
    Some(parts.into_iter().collect())
}

/// Каждый HTTP-provider получает свой кэш, независимо от исходного имени каталога.
fn provider_cache_path(path: &str, key: &str, identity: &str) -> Option<String> {
    let relative = relative_provider_path(path)?;
    let identity = sha1_smol::Sha1::from(identity).digest().to_string();
    Some(format!("./providers/{key}/{identity}/{}", relative.display()))
}

type ProviderRedirects = std::collections::BTreeMap<PathBuf, Option<String>>;
fn check_providers(m: &mut Mapping, key: &str, redirects: &mut ProviderRedirects) -> Result<(), String> {
    let Some(list) = m.get_mut(key) else { return Ok(()) };
    let list = list.as_mapping_mut().ok_or_else(|| t!("{0} в профиле — не список", key))?;
    for (name, p) in list {
        let name = name.as_str().unwrap_or("?");
        let kind = p.get("type").and_then(Value::as_str).unwrap_or("").to_owned();
        if !matches!(kind.as_str(), "http" | "file" | "inline") {
            return Err(t!("provider «{0}»: тип «{1}» не поддерживается", name, kind));
        }
        let format = p.get("format").and_then(Value::as_str).filter(|s| !s.is_empty()).unwrap_or("yaml");
        let behavior = p.get("behavior").and_then(Value::as_str).unwrap_or("");
        if key == "rule-providers" && (!matches!(format, "yaml" | "text" | "mrs")
            || !matches!(behavior, "domain" | "ipcidr" | "classical")
            || (format == "mrs" && behavior == "classical" && kind != "inline")) {
            return Err(t!("provider «{0}»: неверный формат или behavior", name));
        }
        if kind == "http" {
            let url = p.get("url").and_then(Value::as_str).unwrap_or("");
            if !Url::parse(url).ok().is_some_and(|u| matches!(u.scheme(), "http" | "https") && u.host_str().is_some()) {
                return Err(t!("provider «{0}»: неверный URL источника", name));
            }
            let path = match p.get("path") {
                Some(Value::String(path)) if !path.is_empty() => path.as_str(),
                Some(Value::String(_)) | None => if format == "mrs" { "provider.mrs" } else { "provider.yaml" },
                Some(_) => return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, "")),
            };
            let headers = p.get("header").map(|h| serde_yaml::to_string(h).unwrap_or_default()).unwrap_or_default();
            let identity = format!("{url}\0{name}\0{format}\0{behavior}\0{headers}");
            let cache = provider_cache_path(path, key, &identity)
                .ok_or_else(|| t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path))?;
            if let Some(original) = p.get("path").and_then(Value::as_str).and_then(relative_provider_path) {
                redirects.entry(original).and_modify(|old| { if old.as_ref() != Some(&cache) { *old = None; } }).or_insert(Some(cache.clone()));
            }
            p.as_mapping_mut().unwrap().insert(k("path"), k(&cache));
        }
        // Inline-provider использует payload; path ядру не нужен.
        if kind == "inline" { p.as_mapping_mut().unwrap().remove("path"); }
    }
    Ok(())
}

fn check_file_providers(m: &mut Mapping, key: &str, redirects: &ProviderRedirects) -> Result<(), String> {
    let Some(list) = m.get_mut(key).and_then(Value::as_mapping_mut) else { return Ok(()) };
    for (name, p) in list {
        if p.get("type").and_then(Value::as_str) != Some("file") { continue; }
        let name = name.as_str().unwrap_or("?");
        let path = p.get("path").ok_or_else(|| t!("provider «{0}»: у файлового provider нет пути", name))?.as_str().unwrap_or("");
        let relative = relative_provider_path(path).ok_or_else(|| t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path))?;
        if let Some(cache) = redirects.get(&relative) {
            let cache = cache.as_ref().ok_or_else(|| t!("provider «{0}»: неоднозначный общий путь кэша", name))?;
            *p.get_mut("path").unwrap() = k(cache);
            continue;
        }
        if !provider_path_ok(path) { return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path)); }
        let absolute = Path::new(&home()).join(&relative);
        if !absolute.is_file() { return Err(t!("provider «{0}»: локальный файл не найден: {1}", name, path)); }
        let real = fs::canonicalize(&absolute).map_err(|e| e.to_string())?;
        let root = fs::canonicalize(home()).map_err(|e| e.to_string())?;
        if !real.starts_with(root) { return Err(t!("provider «{0}»: путь «{1}» вне каталога профиля", name, path)); }
    }
    Ok(())
}

fn check_provider_references(m: &Mapping) -> Result<(), String> {
    let rules = m.get("rule-providers").and_then(Value::as_mapping);
    let proxies = m.get("proxy-providers").and_then(Value::as_mapping);
    let check_rules = |lines: &Value| -> Result<(), String> {
        for line in lines.as_sequence().into_iter().flatten().filter_map(Value::as_str) {
            let mut fields = line.split(',').map(str::trim);
            if fields.next() == Some("RULE-SET")
                && let Some(name) = fields.next()
                && !rules.is_some_and(|r| r.contains_key(name)) {
                return Err(t!("правило ссылается на отсутствующий provider «{0}»", name));
            }
        }
        Ok(())
    };
    if let Some(rules) = m.get("rules") { check_rules(rules)?; }
    for (_, rules) in m.get("sub-rules").and_then(Value::as_mapping).into_iter().flatten() { check_rules(rules)?; }
    for group in m.get("proxy-groups").and_then(Value::as_sequence).into_iter().flatten() {
        for provider in group.get("use").and_then(Value::as_sequence).into_iter().flatten().filter_map(Value::as_str) {
            if !proxies.is_some_and(|p| p.contains_key(provider)) {
                return Err(t!("группа «{0}» ссылается на отсутствующий provider «{1}»", group.get("name").and_then(Value::as_str).unwrap_or("?"), provider));
            }
        }
    }
    Ok(())
}

/// Разрешённая часть профиля подписки.
fn profile_part(src: &Mapping) -> Result<Mapping, String> {
    check_subscription_placeholder(src)?;
    let mut m = Mapping::new();
    for key in PROFILE_KEYS {
        if let Some(v) = src.get(key) {
            m.insert(k(key), v.clone());
        }
    }
    let mut redirects = ProviderRedirects::new();
    check_providers(&mut m, "proxy-providers", &mut redirects)?;
    check_providers(&mut m, "rule-providers", &mut redirects)?;
    check_file_providers(&mut m, "proxy-providers", &redirects)?;
    check_file_providers(&mut m, "rule-providers", &redirects)?;
    check_provider_references(&m)?;
    Ok(m)
}

/// Порт прокси годится для VPN: диапазон как в TUI и не порт своего DNS.
pub fn check_port(c: &Config) -> Result<(), String> {
    if !(1024..=65535).contains(&c.vpn_port) {
        return Err(t!("порт прокси {0} вне диапазона 1024–65535 (vpn_port в {1})", c.vpn_port, conf_path()));
    }
    if c.vpn_dns && c.vpn_port == DNS_PORT {
        return Err(t!("порт прокси {0} занят своим DNS VPN — выбери другой vpn_port", c.vpn_port));
    }
    Ok(())
}

/// Собирает итоговый конфиг: профиль подписки + настройки cm (порты, TUN, DNS, геофайлы, правила, авто-выбор).
pub fn build_config(c: &Config) -> Result<String, String> {
    build_config_for(c, &load_subs()?)
}

/// Профиль активной legacy-подписки: тело clash или provider для списка ссылок.
fn legacy_profile(subs: &Subs) -> Result<Mapping, String> {
    let sub = subs.list.iter().find(|s| s.id == subs.active).ok_or(t!("нет подписки: добавь её (cm → VPN → Подписки → n)"))?;
    let body = fs::read_to_string(profile_path(sub, &sub.kind)).map_err(|_| t!("профиль подписки не скачан — обнови подписку").to_string())?;

    Ok(if sub.kind == "clash" {
        match parse_profile(&body) {
            Ok(Value::Mapping(m)) => profile_part(&m)?,
            Ok(_) => return Err(t!("профиль подписки повреждён — обнови подписку").into()),
            Err(e) => return Err(t!("профиль подписки не принят: {0}", e)),
        }
    } else {
        let mut m = Mapping::new();
        m.insert(
            k("proxy-providers"),
            yaml_map(vec![(
                "sub",
                yaml_map(vec![
                    ("type", k("file")),
                    ("path", k(&format!("./profiles/{}.txt", sub.id))),
                    ("health-check", yaml_map(vec![("enable", Value::Bool(true)), ("url", k(TEST_URL)), ("interval", Value::from(300))])),
                ]),
            )]),
        );
        m.insert(k("proxy-groups"), Value::Sequence(vec![yaml_map(vec![("name", k("Proxy")), ("type", k("select")), ("use", seq(&["sub"]))])]));
        m.insert(k("rules"), seq(&["MATCH,Proxy"]));
        m
    })
}

fn build_config_for(c: &Config, subs: &Subs) -> Result<String, String> {
    check_port(c)?;
    // источник Store (V.04) заменяет тело подписки целиком: узлы, группы и правила
    let mut m = match store_profile(c, subs)? {
        Some(m) => m,
        None => legacy_profile(subs)?,
    };

    // --- то, что задаёт cm поверх подписки ---
    m.insert(k("mixed-port"), Value::from(c.vpn_port));
    m.insert(k("allow-lan"), Value::Bool(c.vpn_allow_lan));
    m.insert(k("bind-address"), k(if c.vpn_allow_lan { "*" } else { "127.0.0.1" }));
    // API — только Unix-сокет в каталоге 0700 (UMask службы 0077); TCP-контроллера нет
    m.insert(k("external-controller-unix"), k(&api_socket().to_string_lossy()));
    m.insert(k("mode"), k(c.vpn_mode_name()));
    m.insert(k("log-level"), k("warning"));
    m.insert(k("ipv6"), Value::Bool(c.vpn_ipv6));
    m.insert(k("unified-delay"), Value::Bool(true));
    m.insert(k("tcp-concurrent"), Value::Bool(true));
    m.insert(k("find-process-mode"), k("off")); // экономит CPU; правила по процессам не нужны
    m.insert(k("profile"), yaml_map(vec![("store-selected", Value::Bool(true)), ("store-fake-ip", Value::Bool(true))]));
    // геофайлы — тот же источник, что у FlClash
    m.insert(k("geodata-mode"), Value::Bool(false));
    // геофайлы обновляет только cm (geo_update с проверкой формата), не само ядро
    m.insert(k("geo-auto-update"), Value::Bool(false));
    m.insert(
        k("geox-url"),
        yaml_map(vec![
            ("geoip", k(&format!("{GEO_BASE}/geoip.dat"))),
            ("geosite", k(&format!("{GEO_BASE}/geosite.dat"))),
            ("mmdb", k(&format!("{GEO_BASE}/geoip.metadb"))),
            ("asn", k(&format!("{GEO_BASE}/GeoLite2-ASN.mmdb"))),
        ]),
    );
    m.insert(
        k("tun"),
        yaml_map(vec![
            ("enable", Value::Bool(c.vpn_tun)),
            ("device", k(TUN_DEV)),
            ("stack", k("mixed")),
            ("auto-route", Value::Bool(true)),
            ("auto-redirect", Value::Bool(true)),
            ("auto-detect-interface", Value::Bool(true)),
            ("strict-route", Value::Bool(false)),
            ("dns-hijack", seq(&["any:53", "tcp://any:53"])),
        ]),
    );
    if c.vpn_dns {
        m.insert(
            k("dns"),
            yaml_map(vec![
                ("enable", Value::Bool(true)),
                ("listen", k("127.0.0.1:1053")),
                ("ipv6", Value::Bool(c.vpn_ipv6)),
                ("enhanced-mode", k("fake-ip")),
                ("fake-ip-range", k("198.18.0.1/16")),
                (
                    "fake-ip-filter",
                    seq(&["*.lan", "*.local", "+.home.arpa", "localhost.*", "+.msftconnecttest.com", "+.msftncsi.com", "time.*.com", "ntp.*.com", "+.ntp.org", "+.stun.*.*", "+.stun.*.*.*", "geosite:private"]),
                ),
                ("default-nameserver", seq(&["system", "77.88.8.8", "1.1.1.1"])),
                // адреса самих VPN-серверов резолвим напрямую — до подъёма туннеля DoH может быть недоступен
                ("proxy-server-nameserver", seq(&["system", "77.88.8.8"])),
                ("nameserver", seq(&["https://1.1.1.1/dns-query", "https://dns.google/dns-query"])),
                ("direct-nameserver", seq(&["system"])),
                ("respect-rules", Value::Bool(true)),
            ]),
        );
    }

    // --- группы: «⚡ Авто» первой в главной группе ---
    let rules_now: Vec<Value> = m.get("rules").and_then(Value::as_sequence).cloned().unwrap_or_default();
    let mut groups: Vec<Value> = m.get("proxy-groups").and_then(Value::as_sequence).cloned().unwrap_or_default();
    let main = main_group(&groups, &rules_now);
    groups.retain(|g| !matches!(g.get("name").and_then(Value::as_str), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
    if c.vpn_auto_select {
        // Apply to all nodes, including subscription providers. Country labels
        // come from server names; this does not alter manual selection.
        let metadata_filter = "(?i)(трафик|traffic|осталось|remain|истека|expire|срок|сайт|website|官网|剩余|到期|流量)";
        let exclude_filter = if c.vpn_auto_allow_ru { metadata_filter.to_string() } else {
            format!("{metadata_filter}|🇷🇺|(?i)(россия|россий|russia|russland|russie|俄罗斯|俄羅斯|روسيا)|(?i)(^|[^a-zа-яё])(ru|rus|рф|москва|moscow|moskva|санкт[ -]?петербург|saint[ -]?petersburg|st[ .-]?petersburg|novosibirsk)([^a-zа-яё]|$)")
        };
        groups.insert(
            0,
            yaml_map(vec![
                ("name", k(AUTO_GROUP)),
                ("type", k("url-test")),
                ("include-all", Value::Bool(true)),
                // служебные «серверы» с остатком трафика и сроком — не серверы
                ("exclude-filter", k(&exclude_filter)),
                ("url", k(TEST_URL)),
                ("interval", Value::from(300)),
                ("tolerance", Value::from(50)),
                ("timeout", Value::from(3000)),
                ("lazy", Value::Bool(false)),
            ]),
        );
        if let Some(main) = &main {
            for g in groups.iter_mut() {
                if g.get("name").and_then(Value::as_str) == Some(main.as_str())
                    && let Value::Mapping(gm) = g {
                    let mut list: Vec<Value> = gm.get("proxies").and_then(Value::as_sequence).cloned().unwrap_or_default();
                    list.retain(|x| !matches!(x.as_str(), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
                    list.insert(0, k(AUTO_GROUP));
                    gm.insert(k("proxies"), Value::Sequence(list));
                }
            }
        }
    } else if let Some(main) = &main {
        for g in groups.iter_mut() {
            if let (Some(n), Value::Mapping(gm)) = (g.get("name").and_then(Value::as_str).map(String::from), &mut *g)
                && n == *main
                && let Some(Value::Sequence(list)) = gm.get_mut("proxies") {
                list.retain(|x| !matches!(x.as_str(), Some(AUTO_GROUP | AUTO_GROUP_OLD)));
            }
        }
    }
    m.insert(k("proxy-groups"), Value::Sequence(groups));

    // --- правила: свои → локальная сеть → Россия → правила подписки ---
    let mut rules: Vec<Value> = user_rules()?.into_iter().map(Value::String).collect();
    if c.vpn_direct_lan {
        for r in ["GEOSITE,private,DIRECT", "GEOIP,private,DIRECT,no-resolve"] {
            rules.push(k(r));
        }
    }
    if c.vpn_direct_ru {
        for r in ["GEOSITE,category-ru,DIRECT", "DOMAIN-SUFFIX,ru,DIRECT", "DOMAIN-SUFFIX,xn--p1ai,DIRECT", "GEOIP,RU,DIRECT"] {
            rules.push(k(r));
        }
    }
    rules.extend(rules_now);
    if !rules.iter().any(|r| r.as_str().map(|s| s.starts_with("MATCH,")).unwrap_or(false)) {
        rules.push(k(&format!("MATCH,{}", main.unwrap_or_else(|| if c.vpn_auto_select { AUTO_GROUP.into() } else { "DIRECT".into() }))));
    }
    m.insert(k("rules"), Value::Sequence(rules));

    serde_yaml::to_string(&Value::Mapping(m)).map_err(|e| e.to_string())
}

pub fn write_config(c: &Config) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    write_config_locked(&latest)
}

fn write_config_locked(c: &Config) -> Result<bool, String> {
    let y = build_config(c)?;
    private_dir(&home())?;
    let p = config_path();
    if fs::read_to_string(&p).map(|o| o == y).unwrap_or(false) {
        return Ok(false);
    }
    write_private(&p, y.as_bytes())?;
    Ok(true)
}
