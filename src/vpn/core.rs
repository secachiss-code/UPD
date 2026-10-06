// ======================= API mihomo =======================

/// Владелец сокета и процесса mihomo: служба работает от root; в тестах — текущий пользователь.
fn service_uid() -> u32 {
    if test_mode() { crate::common::sys::euid() } else { 0 }
}

/// Сокет API — наш: сокет и его каталог принадлежат службе, каталог закрыт для группы и остальных.
/// Права самого сокета mihomo ставит 0666, поэтому доступ ограничивает каталог: без права поиска
/// в нём к сокету не подключиться, и подменить сокет может только владелец каталога.
fn check_api_socket(path: &Path) -> Result<(), String> {
    use std::os::unix::fs::{FileTypeExt, MetadataExt};
    let bad = || t!("{0}: сокет API mihomo чужой или доступен не только службе", path.display());
    let md = fs::symlink_metadata(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let dir = path.parent().map(fs::symlink_metadata).ok_or_else(bad)?.map_err(|e| format!("{}: {e}", path.display()))?;
    let uid = service_uid();
    if !md.file_type().is_socket() || md.uid() != uid || !dir.is_dir() || dir.uid() != uid || dir.mode() & 0o077 != 0 {
        return Err(bad());
    }
    Ok(())
}

/// uid процесса на другом конце Unix-сокета (SO_PEERCRED).
fn peer_uid(s: &std::os::unix::net::UnixStream) -> Option<u32> {
    use std::os::fd::AsRawFd;
    let mut cred = libc::ucred { pid: 0, uid: 0, gid: 0 };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: cred and len describe a live ucred-sized buffer.
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0).then_some(cred.uid)
}

/// Предел ответа API: списки прокси и соединений бывают большими, остальное — короткий JSON.
fn api_limit(path: &str) -> u64 {
    if path.starts_with("/proxies") || path.starts_with("/connections") || path.starts_with("/group") { 16 << 20 } else { 1 << 20 }
}

/// Ответ HTTP/1.1: код и тело (Content-Length, chunked или до закрытия соединения).
fn parse_http_response(raw: &[u8]) -> Result<(u16, Vec<u8>), String> {
    let bad = || t!("неверный HTTP ответ").to_string();
    let head_end = raw.windows(4).position(|w| w == b"\r\n\r\n").ok_or_else(bad)?;
    let head = std::str::from_utf8(&raw[..head_end]).map_err(|_| bad())?;
    let mut lines = head.split("\r\n");
    let status: u16 = lines.next().and_then(|l| l.split_whitespace().nth(1)).and_then(|c| c.parse().ok()).ok_or_else(bad)?;
    let (mut length, mut chunked) = (None, false);
    for l in lines {
        let Some((k, v)) = l.split_once(':') else { continue };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => length = Some(v.trim().parse::<usize>().map_err(|_| bad())?),
            "transfer-encoding" => chunked = v.to_ascii_lowercase().contains("chunked"),
            _ => {}
        }
    }
    let body = &raw[head_end + 4..];
    if chunked {
        let mut out = vec![];
        let mut pos = 0;
        loop {
            let line_end = body[pos..].windows(2).position(|w| w == b"\r\n").ok_or_else(bad)? + pos;
            let size_str = std::str::from_utf8(&body[pos..line_end]).map_err(|_| bad())?;
            let size = usize::from_str_radix(size_str.split(';').next().unwrap_or("").trim(), 16).map_err(|_| bad())?;
            pos = line_end + 2;
            if size == 0 {
                return Ok((status, out));
            }
            let end = pos.checked_add(size).filter(|e| *e <= body.len()).ok_or_else(bad)?;
            out.extend_from_slice(&body[pos..end]);
            pos = end + 2;
        }
    }
    match length {
        Some(n) if n <= body.len() => Ok((status, body[..n].to_vec())),
        Some(_) => Err(t!("ответ обрезан относительно Content-Length").into()),
        None => Ok((status, body.to_vec())),
    }
}

/// Запрос к API mihomo через Unix-сокет. Сокет и процесс на нём проверяются до отправки запроса.
pub fn api(method: &str, path: &str, body: Option<serde_json::Value>) -> Result<serde_json::Value, String> {
    api_with_timeout(method, path, body, Duration::from_secs(12))
}

fn api_with_timeout(method: &str, path: &str, body: Option<serde_json::Value>, request_timeout: Duration) -> Result<serde_json::Value, String> {
    use std::io::Write;
    let sock = api_socket();
    check_api_socket(&sock)?;
    let mut s = std::os::unix::net::UnixStream::connect(&sock).map_err(|e| format!("{}: {e}", sock.display()))?;
    if peer_uid(&s) != Some(service_uid()) {
        return Err(t!("{0}: на сокете API не процесс службы", sock.display()));
    }
    let timeout = Some(request_timeout);
    let _ = s.set_read_timeout(timeout);
    let _ = s.set_write_timeout(timeout);
    let body = body.map(|b| b.to_string()).unwrap_or_default();
    let mut req = format!("{method} {path} HTTP/1.1\r\nHost: mihomo\r\nConnection: close\r\nContent-Length: {}\r\n", body.len());
    if !body.is_empty() {
        req += "Content-Type: application/json\r\n";
    }
    req += "\r\n";
    req += &body;
    s.write_all(req.as_bytes()).map_err(|e| e.to_string())?;
    let max = api_limit(path);
    let mut raw = vec![];
    (&mut s).take(max.saturating_add(1)).read_to_end(&mut raw).map_err(|e| e.to_string())?;
    if raw.len() as u64 > max {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let (status, body) = parse_http_response(&raw)?;
    let text = String::from_utf8_lossy(&body);
    match status {
        204 => Ok(serde_json::Value::Null),
        200..=299 if text.trim().is_empty() => Ok(serde_json::Value::Null),
        200..=299 => serde_json::from_str(&text).map_err(|e| e.to_string()),
        code => {
            let msg = serde_json::from_str::<serde_json::Value>(&text).ok().and_then(|v| v["message"].as_str().map(String::from));
            Err(msg.unwrap_or_else(|| format!("HTTP {code}")))
        }
    }
}

pub fn running() -> bool {
    api("GET", "/version", None).is_ok()
}

pub fn reload() -> Result<(), String> {
    api("PUT", "/configs?force=true", Some(serde_json::json!({ "path": config_path() }))).map(|_| ())
}

pub fn set_mode(mode: &str) -> Result<(), String> {
    api("PATCH", "/configs", Some(serde_json::json!({ "mode": mode }))).map(|_| ())
}

pub fn select(group: &str, name: &str) -> Result<(), String> {
    api("PUT", &format!("/proxies/{}", pct_encode(group)), Some(serde_json::json!({ "name": name })))?;
    // старые соединения шли через прежний сервер — закрываем, чтобы переключение было сразу
    let _ = api("DELETE", "/connections", None);
    Ok(())
}

pub fn group_delay(group: &str) -> Result<std::collections::BTreeMap<String, u64>, String> {
    let v = api("GET", &format!("/group/{}/delay?url={}&timeout=5000", pct_encode(group), pct_encode(TEST_URL)), None)?;
    Ok(v.as_object().map(|o| o.iter().filter_map(|(k, v)| Some((k.clone(), v.as_u64()?))).collect()).unwrap_or_default())
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Group {
    pub name: String,
    pub kind: String,
    pub now: String,
    pub all: Vec<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default, PartialEq)]
pub struct Snapshot {
    pub running: bool,
    pub mode: String,
    pub tun: bool,
    pub version: String,
    pub groups: Vec<Group>,
    /// последняя известная задержка каждого сервера, мс (0 — не отвечает)
    pub delay: std::collections::BTreeMap<String, u64>,
    pub down: u64,
    pub up: u64,
    pub conns: usize,
    /// почему ядро не ответило (пусто — ответило или сокета API нет)
    pub error: String,
}

impl Snapshot {
    /// Состояние API отдельно от последнего замера выбранного сервера.
    pub fn health_line(&self) -> String {
        if !self.running { return t!("API VPN не готов").into(); }
        let chain = self.chain();
        let Some(selected) = chain.last() else { return t!("API готов · сервер не выбран").into(); };
        match self.delay.get(selected) {
            Some(0) => t!("API готов · последний замер сервера завершился ошибкой").into(),
            Some(delay) => t!("API готов · последний замер сервера: {0} мс", delay),
            None => t!("API готов · доступность сервера ещё не проверена").into(),
        }
    }

    /// Цепочка от главной группы до реального сервера: Proxy → ⚡ Auto → 🇩🇪 DE-1
    pub fn chain(&self) -> Vec<String> {
        let mut out = vec![];
        let mut cur = self.groups.iter().find(|g| g.kind == "Selector").or(self.groups.first()).map(|g| g.name.clone());
        while let Some(n) = cur {
            if out.contains(&n) || out.len() > 6 {
                break;
            }
            out.push(n.clone());
            cur = self.groups.iter().find(|g| g.name == n).map(|g| g.now.clone()).filter(|x| !x.is_empty());
        }
        out
    }
}

pub fn snapshot() -> Snapshot {
    let mut s = Snapshot::default();
    let v = match api("GET", "/version", None) {
        Ok(v) => v,
        Err(e) => {
            if api_socket().exists() {
                s.error = e;
            }
            return s;
        }
    };
    s.running = true;
    s.version = v["version"].as_str().unwrap_or("").to_string();
    if let Ok(cfg) = api("GET", "/configs", None) {
        s.mode = cfg["mode"].as_str().unwrap_or("").to_string();
        s.tun = cfg["tun"]["enable"].as_bool().unwrap_or(false);
    }
    if let Ok(p) = api("GET", "/proxies", None)
        && let Some(obj) = p["proxies"].as_object() {
        for (name, x) in obj {
            if let Some(d) = x["history"].as_array().and_then(|h| h.last()).and_then(|h| h["delay"].as_u64()) {
                s.delay.insert(name.clone(), d);
            }
            let kind = x["type"].as_str().unwrap_or("");
            if ["Selector", "URLTest", "Fallback", "LoadBalance"].contains(&kind) && name != "GLOBAL" {
                s.groups.push(Group {
                    name: name.clone(),
                    kind: kind.into(),
                    now: x["now"].as_str().unwrap_or("").into(),
                    all: x["all"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default(),
                });
            }
        }
        // порядок групп как в конфиге: GLOBAL.all перечисляет их по порядку
        if let Some(order) = obj.get("GLOBAL").and_then(|g| g["all"].as_array()) {
            let pos = |n: &str| order.iter().position(|v| v.as_str() == Some(n)).unwrap_or(usize::MAX);
            s.groups.sort_by_key(|g| pos(&g.name));
        }
    }
    if let Ok(cn) = api("GET", "/connections", None) {
        s.down = cn["downloadTotal"].as_u64().unwrap_or(0);
        s.up = cn["uploadTotal"].as_u64().unwrap_or(0);
        s.conns = cn["connections"].as_array().map(|a| a.len()).unwrap_or(0);
    }
    s
}

// ======================= служба =======================

pub fn service_active() -> bool {
    unit_state(SERVICE) == "active"
}

/// После серии неудачных запусков systemd блокирует службу на 10 минут (start-limit-hit) — снимаем блок.
fn reset_failed() {
    let _ = out("systemctl", &["reset-failed", SERVICE]);
}

pub fn start(c: &Config) -> Result<(), String> {
    let result = (|| {
        if let Some(w) = conflict(c) { return Err(w); }
        reset_failed();
        run(true, &[], "systemctl", &["start", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))?;
        wait_ready(Duration::from_secs(10))?;
        let _ = autostart(c.vpn_autostart);
        Ok(())
    })();
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("start", error) }
    result
}

/// API readiness does not assert that a remote proxy server is reachable.
fn wait_ready(timeout: Duration) -> Result<(), String> {
    wait_ready_with(timeout, || api_with_timeout("GET", "/version", None, Duration::from_millis(500)).map(|_| ()))
}
fn wait_ready_with(timeout: Duration, mut probe: impl FnMut() -> Result<(), String>) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        match probe() {
            Ok(()) => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => return Err(format!("VPN service started, but API is not ready: {error}")),
            Err(_) => std::thread::sleep(Duration::from_millis(100).min(deadline.saturating_duration_since(std::time::Instant::now()))),
        }
    }
}

pub fn stop() -> Result<(), String> {
    run(true, &[], "systemctl", &["stop", SERVICE])
}

pub fn restart(c: &Config) -> Result<(), String> {
    let result = (|| {
        if let Some(w) = conflict(c) { return Err(w); }
        reset_failed();
        run(true, &[], "systemctl", &["restart", SERVICE]).map_err(|e| format!("{e}\n{}", journal_tail()))?;
        wait_ready(Duration::from_secs(10))
    })();
    match &result { Ok(()) => resolve_failure(), Err(error) => record_failure("restart", error) }
    result
}

pub fn core_restart_failure(error: &str) -> String {
    t!("Ядро VPN обновлено на диске, но перезапуск не удался: {0}", error)
}

fn note_core_update(state: &mut VpnState, changed: bool, active: bool, restart: Result<(), String>, flclash_tag: &str) -> Option<String> {
    match restart {
        Err(error) => {
            state.event = core_restart_failure(&error);
            state.event_time = now();
            Some(state.event.clone())
        }
        Ok(()) => {
            state.flclash_applied = flclash_tag.to_string();
            if changed {
                state.event = if active {
                    t!("Ядро VPN обновлено до {} (FlClash {})", state.core_version, flclash_tag)
                } else {
                    t!("Ядро VPN обновлено на диске до {} (FlClash {}); служба VPN не запущена", state.core_version, flclash_tag)
                };
                state.event_time = now();
            }
            None
        }
    }
}

/// Последние строки журнала службы — чтобы причину сбоя было видно сразу, без journalctl.
pub fn journal_tail() -> String {
    let (s, _) = out("journalctl", &["-u", SERVICE, "-n", "8", "--no-pager", "-o", "cat"]);
    s.lines().map(|l| format!("  {l}")).collect::<Vec<_>>().join("\n")
}

pub fn autostart(on: bool) -> Result<(), String> {
    run(true, &[], "systemctl", &[if on { "enable" } else { "disable" }, SERVICE])
}

/// Ядра VPN, чей TUN мешает нашему (два авто-маршрута не уживутся). Имя здесь не даёт обойти проверку:
/// создать TUN может только привилегированный процесс; mesh-сети вроде tailscale ядрами VPN не считаются.
fn cpu_level() -> &'static str {
    let info = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let flags: Vec<&str> = info.lines().find(|l| l.starts_with("flags")).map(|l| l.split_whitespace().collect()).unwrap_or_default();
    let has = |f: &str| flags.contains(&f);
    if has("avx2") && has("bmi2") && has("fma") && has("movbe") {
        "v3"
    } else if has("sse4_2") && has("popcnt") && has("ssse3") {
        "v2"
    } else {
        "v1"
    }
}

pub fn core_version() -> Option<String> {
    let (s, code) = out(&core_bin(), &["-v"]);
    if code != 0 {
        return None;
    }
    s.split_whitespace().find(|w| w.starts_with('v') && w[1..].starts_with(|c: char| c.is_ascii_digit())).map(String::from)
}

fn gh_latest(repo: &str, port: u16) -> Result<serde_json::Value, String> {
    let r = get(&format!("https://api.github.com/repos/{repo}/releases/latest"), 20, port)?;
    serde_json::from_str(&read_text(r, 4 << 20)?).map_err(|e| e.to_string())
}

/// Предел скачанного архива ядра и отдельно — распакованного бинарника (mihomo — десятки мегабайт).
const CORE_GZ_MAX: u64 = 64 << 20;
const CORE_BIN_MAX: u64 = 128 << 20;

/// Распаковка gzip не больше `max` байт: лишний байт сверх предела — ошибка до записи на диск.
fn gunzip_limited(gz: &[u8], max: u64) -> Result<Vec<u8>, String> {
    let mut bin = vec![];
    flate2::read::GzDecoder::new(gz).take(max.saturating_add(1)).read_to_end(&mut bin).map_err(|e| t!("распаковка: {0}", e))?;
    if bin.len() as u64 > max {
        return Err(t!("распакованное ядро больше {0} — установка отменена, прежнее ядро оставлено", fmt_bytes(max)));
    }
    Ok(bin)
}

/// Сигнал от FlClash: номер его последнего релиза; и актуальный релиз mihomo.
pub fn core_check(c: &Config, log: Log) -> Result<VpnState, String> {
    let fl = gh_latest("chen08209/FlClash", c.vpn_port)?;
    let mh = gh_latest("MetaCubeX/mihomo", c.vpn_port)?;
    let mut st = load_state();
    st.flclash_tag = fl["tag_name"].as_str().unwrap_or("").into();
    st.core_latest = mh["tag_name"].as_str().unwrap_or("").into();
    st.core_version = core_version().unwrap_or_default();
    st.checked = now();
    let _ = save_json("vpn.json", &st);
    log(&t!("FlClash: {} · mihomo: установлен {}, последний {}", st.flclash_tag, if st.core_version.is_empty() { "—" } else { &st.core_version }, st.core_latest));
    Ok(st)
}

/// Проверка SHA-256 digest релиза mihomo до распаковки и запуска кандидата.
fn verify_core_gz_digest(raw_digest: &str, gz: &[u8]) -> Result<(), String> {
    let digest = raw_digest.strip_prefix("sha256:").ok_or(t!("неверный формат SHA-256 digest; установка отменена"))?;
    if digest.len() != 64 || !digest.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(t!("неверный формат SHA-256 digest; установка отменена").into());
    }
    let digest = digest.to_ascii_lowercase();
    use sha2::Digest;
    let got: String = sha2::Sha256::digest(gz).iter().map(|b| format!("{b:02x}")).collect();
    if got != digest {
        return Err(t!("контрольная сумма не совпала (ожидалась {0}, получена {1})", digest, got));
    }
    Ok(())
}

/// Скачать и поставить mihomo (сборка под процессор, проверка SHA-256). true — ядро сменилось.
pub fn core_install(c: &Config, log: Log, force: bool) -> Result<bool, String> {
    let _vpn_files = vpn_config_lock(true)?;
    let rel = gh_latest("MetaCubeX/mihomo", c.vpn_port)?;
    let tag = rel["tag_name"].as_str().ok_or(t!("нет tag_name в релизе mihomo"))?.to_string();
    let cur = core_version();
    if !force && cur.as_deref() == Some(tag.as_str()) {
        log(&t!("ядро mihomo {0} уже актуально", tag));
        return Ok(false);
    }
    let name = if cfg!(target_arch = "aarch64") { format!("mihomo-linux-arm64-{tag}.gz") } else { format!("mihomo-linux-amd64-{}-{tag}.gz", cpu_level()) };
    let asset = rel["assets"].as_array().and_then(|a| a.iter().find(|x| x["name"] == name.as_str())).ok_or(t!("в релизе нет {0}", name))?;
    let raw_digest = asset["digest"].as_str().ok_or(t!("в релизе нет SHA-256 digest; установка отменена"))?;
    let url = asset["browser_download_url"].as_str().ok_or(t!("нет ссылки на файл"))?;
    log(&t!("скачиваю {0}...", name));
    let gz = read_progress(get(url, 600, c.vpn_port)?, CORE_GZ_MAX, log)?;
    verify_core_gz_digest(raw_digest, &gz)?;
    log(t!("контрольная сумма SHA-256 совпала"));
    let bin = gunzip_limited(&gz, CORE_BIN_MAX)?;
    private_dir(&format!("{}/bin", home()))?;
    let tmp = format!("{}.new", core_bin());
    atomic_write(Path::new(&tmp), &bin, 0o755).map_err(|e| e.to_string())?;
    let (ver, code) = out(&tmp, &["-v"]);
    if code != 0 {
        let _ = fs::remove_file(&tmp);
        return Err(t!("новое ядро не запускается — оставляю прежнее").into());
    }
    fs::rename(&tmp, core_bin()).map_err(|e| e.to_string())?;
    log(&t!("ядро: {}", ver.lines().next().unwrap_or("").trim()));
    let mut st = load_state();
    st.core_version = tag.clone();
    st.core_latest = tag;
    let _ = save_json("vpn.json", &st);
    Ok(true)
}

// ======================= геофайлы =======================

pub fn geo_files() -> Vec<(String, i64, u64)> {
    GEO.iter()
        .map(|(f, _)| {
            let m = fs::metadata(format!("{}/{f}", home())).ok();
            let t = m.as_ref().and_then(|m| m.modified().ok()).and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
            (f.to_string(), t, m.map(|m| m.len()).unwrap_or(0))
        })
        .collect()
}

struct ProtoField<'a> {
    number: u32,
    wire: u8,
    bytes: &'a [u8],
    integer: Option<u64>,
}

fn proto_varint(data: &[u8], pos: &mut usize) -> Option<u64> {
    let mut value = 0u64;
    for shift in 0..10 {
        let byte = *data.get(*pos)?;
        *pos += 1;
        if shift == 9 && byte > 1 {
            return None;
        }
        value |= u64::from(byte & 0x7f) << (shift * 7);
        if byte & 0x80 == 0 {
            return Some(value);
        }
    }
    None
}

fn next_proto_field<'a>(data: &'a [u8], pos: &mut usize) -> Result<Option<ProtoField<'a>>, String> {
    if *pos == data.len() {
        return Ok(None);
    }
    let key = proto_varint(data, pos).ok_or(t!("неверная структура protobuf"))?;
    let number = key >> 3;
    let wire = (key & 7) as u8;
    if number == 0 || number >= (1 << 29) {
        return Err(t!("неверная структура protobuf").into());
    }
    let (bytes, integer) = match wire {
        0 => (&[][..], Some(proto_varint(data, pos).ok_or(t!("неверная структура protobuf"))?)),
        1 => {
            let end = pos.checked_add(8).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        2 => {
            let len: usize = proto_varint(data, pos).and_then(|n| n.try_into().ok()).ok_or(t!("неверная длина protobuf"))?;
            let end = pos.checked_add(len).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        5 => {
            let end = pos.checked_add(4).filter(|end| *end <= data.len()).ok_or(t!("обрезанная структура protobuf"))?;
            let bytes = &data[*pos..end];
            *pos = end;
            (bytes, None)
        }
        _ => return Err(t!("неподдерживаемая структура protobuf").into()),
    };
    Ok(Some(ProtoField { number: number as u32, wire, bytes, integer }))
}

fn validate_geo_domain(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut value = false;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        if field.number == 2 {
            if field.wire != 2 || field.bytes.is_empty() || std::str::from_utf8(field.bytes).is_err() {
                return Err(t!("неверный домен в GeoSite").into());
            }
            value = true;
        }
    }
    if value { Ok(()) } else { Err(t!("пустой домен в GeoSite").into()) }
}

fn validate_geo_cidr(data: &[u8]) -> Result<(), String> {
    let mut pos = 0;
    let mut ip = None;
    let mut prefix = None;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        match field.number {
            1 => {
                if field.wire != 2 || !matches!(field.bytes.len(), 4 | 16) {
                    return Err(t!("неверный адрес в GeoIP").into());
                }
                ip = Some(field.bytes.len());
            }
            2 => {
                if field.wire != 0 {
                    return Err(t!("неверная маска в GeoIP").into());
                }
                prefix = field.integer;
            }
            _ => {}
        }
    }
    let Some(ip_len) = ip else { return Err(t!("в GeoIP нет адреса").into()) };
    if prefix.map(|n| n > (ip_len * 8) as u64).unwrap_or(false) {
        return Err(t!("неверная маска в GeoIP").into());
    }
    Ok(())
}

fn validate_geo_dat(data: &[u8], site: bool) -> Result<(), String> {
    let mut pos = 0;
    let mut entries = 0;
    while let Some(field) = next_proto_field(data, &mut pos)? {
        if field.number != 1 {
            continue;
        }
        if field.wire != 2 {
            return Err(t!("неверная запись GeoIP/GeoSite").into());
        }
        let mut entry_pos = 0;
        let mut country = false;
        let mut values = 0;
        while let Some(entry) = next_proto_field(field.bytes, &mut entry_pos)? {
            match entry.number {
                1 => {
                    if entry.wire != 2 || entry.bytes.is_empty() || std::str::from_utf8(entry.bytes).is_err() {
                        return Err(t!("неверный код GeoIP/GeoSite").into());
                    }
                    country = true;
                }
                2 => {
                    if entry.wire != 2 {
                        return Err(t!("неверная запись GeoIP/GeoSite").into());
                    }
                    if site {
                        validate_geo_domain(entry.bytes)?;
                    } else {
                        validate_geo_cidr(entry.bytes)?;
                    }
                    values += 1;
                }
                _ => {}
            }
        }
        if !country || values == 0 {
            return Err(t!("неполная запись GeoIP/GeoSite").into());
        }
        entries += 1;
    }
    if entries == 0 {
        return Err(t!("в GeoIP/GeoSite нет записей").into());
    }
    Ok(())
}

fn mmdb_size(data: &[u8], pos: &mut usize, code: u8) -> Result<usize, String> {
    let (extra, base) = match code {
        0..=28 => return Ok(code as usize),
        29 => (1, 29usize),
        30 => (2, 285usize),
        31 => (3, 65_821usize),
        _ => return Err(t!("неверный размер поля MaxMind DB").into()),
    };
    let end = pos.checked_add(extra).filter(|end| *end <= data.len()).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
    let mut value = 0usize;
    for byte in &data[*pos..end] {
        value = (value << 8) | *byte as usize;
    }
    *pos = end;
    base.checked_add(value).ok_or_else(|| t!("неверный размер поля MaxMind DB").into())
}

fn mmdb_skip_value(data: &[u8], pos: &mut usize, depth: usize) -> Result<u8, String> {
    if depth > 32 {
        return Err(t!("слишком глубокие метаданные MaxMind DB").into());
    }
    let control = *data.get(*pos).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
    *pos += 1;
    let mut kind = control >> 5;
    let size_code = control & 0x1f;
    if kind == 0 {
        kind = data.get(*pos).copied().and_then(|n| n.checked_add(7)).ok_or(t!("обрезанные метаданные MaxMind DB"))?;
        *pos += 1;
        if !(8..=15).contains(&kind) {
            return Err(t!("неизвестный тип метаданных MaxMind DB").into());
        }
    }
    if kind == 1 {
        let width = ((size_code >> 3) + 1) as usize;
        let end = pos.checked_add(width).filter(|end| *end <= data.len()).ok_or(t!("обрезанный указатель MaxMind DB"))?;
        *pos = end;
        return Ok(kind);
    }
    let size = mmdb_size(data, pos, size_code)?;
    match kind {
        2 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанная строка MaxMind DB"))?;
            std::str::from_utf8(&data[*pos..end]).map_err(|_| t!("неверная строка метаданных MaxMind DB"))?;
            *pos = end;
        }
        3 | 15 => {
            let expected = if kind == 3 { 8 } else { 4 };
            if size != expected {
                return Err(t!("неверный размер числа MaxMind DB").into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанное число MaxMind DB"))?;
            *pos = end;
        }
        4 => {
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанные байты MaxMind DB"))?;
            *pos = end;
        }
        5 | 6 | 8 | 9 | 10 => {
            let max = match kind { 5 => 2, 6 | 8 => 4, 9 => 8, _ => 16 };
            if size > max {
                return Err(t!("неверный размер целого MaxMind DB").into());
            }
            let end = pos.checked_add(size).filter(|end| *end <= data.len()).ok_or(t!("обрезанное целое MaxMind DB"))?;
            *pos = end;
        }
        7 => {
            if size > data.len().saturating_sub(*pos) / 2 {
                return Err(t!("обрезанная карта метаданных MaxMind DB").into());
            }
            for _ in 0..size {
                if mmdb_skip_value(data, pos, depth + 1)? != 2 {
                    return Err(t!("ключ метаданных MaxMind DB не является строкой").into());
                }
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        11 => {
            if size > data.len().saturating_sub(*pos) {
                return Err(t!("обрезанный список метаданных MaxMind DB").into());
            }
            for _ in 0..size {
                mmdb_skip_value(data, pos, depth + 1)?;
            }
        }
        14 if size <= 1 => {}
        12..=14 => return Err(t!("неподдерживаемый тип метаданных MaxMind DB").into()),
        _ => return Err(t!("неизвестный тип метаданных MaxMind DB").into()),
    }
    Ok(kind)
}

fn validate_mmdb(data: &[u8]) -> Result<(), String> {
    const MARKER: &[u8] = b"\xab\xcd\xefMaxMind.com";
    let start = data.len().saturating_sub((128 << 10) + MARKER.len());
    let Some(relative) = data[start..].windows(MARKER.len()).rposition(|w| w == MARKER) else {
        return Err(t!("нет сигнатуры MaxMind DB").into());
    };
    let metadata = &data[start + relative + MARKER.len()..];
    if metadata.is_empty() || metadata[0] >> 5 != 7 || metadata[0] & 0x1f == 0 {
        return Err(t!("повреждённый раздел метаданных MaxMind DB").into());
    }
    let mut pos = 0;
    if mmdb_skip_value(metadata, &mut pos, 0)? != 7 || pos != metadata.len() {
        return Err(t!("повреждённый раздел метаданных MaxMind DB").into());
    }
    Ok(())
}

fn save_geo_result(path: &Path, remote: &str, fetched: Result<Vec<u8>, String>) -> Result<(), String> {
    let data = fetched?;
    validate_geo_file(remote, &data)?;
    atomic_write(path, &data, 0o644).map_err(|e| e.to_string())
}

fn validate_geo_file(remote: &str, data: &[u8]) -> Result<(), String> {
    if data.len() <= 1024 {
        return Err(t!("файл слишком мал").into());
    }
    match remote {
        "geoip.metadb" | "ASN.mmdb" => validate_mmdb(data),
        "GeoSite.dat" => validate_geo_dat(data, true),
        "GeoIP.dat" => validate_geo_dat(data, false),
        _ => Err(t!("неизвестный формат геофайла").into()),
    }
}

pub fn geo_update(c: &Config, log: Log, only_missing: bool) -> Result<(), String> {
    let _vpn_files = vpn_config_lock(true)?;
    private_dir(&home())?;
    let mut errs = vec![];
    for (file, remote) in GEO {
        let path = format!("{}/{file}", home());
        if only_missing && Path::new(&path).exists() {
            continue;
        }
        log(&t!("геофайл {0}...", remote));
        match get(&format!("{GEO_BASE}/{remote}"), 300, c.vpn_port)
            .and_then(|r| {
                if r.status() != 200 {
                    return Err(t!("HTTP {} вместо полного файла", r.status()));
                }
                read_limited(r, 100 << 20)
            })
            .and_then(|b| validate_geo_file(remote, &b).map(|()| b))
        {
            Ok(b) => {
                if let Err(e) = save_geo_result(Path::new(&path), remote, Ok(b)) {
                    errs.push(format!("{remote}: {e}"));
                }
            }
            Err(e) => errs.push(format!("{remote}: {e}")),
        }
    }
    if errs.is_empty() {
        let mut st = load_state();
        st.geo_updated = now();
        let _ = save_json("vpn.json", &st);
        if service_active() && !only_missing {
            let _ = reload();
        }
        Ok(())
    } else {
        Err(errs.join("; "))
    }
}

// ======================= подготовка к запуску и обслуживание =======================

/// Скачать недостающее: ядро и геофайлы. Работает и при запущенном FlClash — тогда качает через его прокси.
pub fn fetch_missing(c: &Config, log: Log) -> Result<(), String> {
    if core_version().is_none() {
        log(t!("ядра ещё нет — ставлю mihomo"));
        core_install(c, log, true)?;
    }
    if let Err(e) = geo_update(c, log, true) {
        log(&t!("геофайлы не скачались ({0}) — mihomo попробует сам", e));
    }
    Ok(())
}

/// Вызывается службой перед стартом ядра (ExecStartPre): только проверка конфликта, сборка конфига и `mihomo -t`.
/// Загрузки здесь нет — медленная сеть не должна упираться в TimeoutStartSec; недостающее качает `cm vpn start`.
pub fn prepare(c: &Config, log: Log) -> Result<(), String> {
    let result = prepare_inner(c, log);
    if let Err(error) = &result { record_failure("prepare", error); }
    result
}

fn prepare_inner(c: &Config, log: Log) -> Result<(), String> {
    check_port(c)?;
    if let Some(w) = conflict(c) {
        return Err(w);
    }
    if core_version().is_none() {
        return Err(t!("ядра mihomo нет — выполни: sudo cm vpn core update").into());
    }
    let _vpn_files = vpn_config_lock(true)?;
    let latest = if Path::new(&conf_path()).exists() { Config::load(c.mirrors.clone())? } else { c.clone() };
    let candidate = build_config(&latest)?;
    validate_candidate(&candidate)?;
    write_private(&config_path(), candidate.as_bytes())?;
    log(t!("конфиг проверен"));
    Ok(())
}

/// Фоновое обслуживание (из cm auto): подписки по сроку, ядро по сигналу FlClash.
pub fn maintain(c: &Config, log: Log) {
    let changed = {
        let _vpn_files = match vpn_config_lock(true) { Ok(lock) => lock, Err(error) => { log(&error); return; } };
        let _lock = match subscriptions_lock(true) {
            Ok(lock) => lock,
            Err(error) => { log(&error); return; }
        };
        let subs = match load_subs() { Ok(subs) => subs, Err(error) => { log(&error); return; } };
        if subs.list.is_empty() { return; }
        match update_subs_locked(c, log, false) {
            Ok(changed) => changed,
            Err(error) => { log(&t!("VPN: подписки не обновлены: {0}", error)); false }
        }
    };
    if changed && let Err(error) = apply(c, None, log) { log(&format!("VPN: {error}")); }
    let st = load_state();
    if !elapsed_at_least(st.checked, hours_secs(c.vpn_core_check_h)) && core_version().is_some() {
        return;
    }
    let Ok(st) = core_check(c, log) else { return };
    let signal = st.flclash_tag != st.flclash_applied;
    if signal || core_version().is_none() {
        log(&t!("новый релиз FlClash {} — обновляю ядро mihomo", st.flclash_tag));
        match core_install(c, log, false) {
            Ok(changed) => {
                let active = service_active();
                let restart_result = if changed && active { restart(c) } else { Ok(()) };
                let mut s2 = load_state();
                if let Some(event) = note_core_update(&mut s2, changed, active, restart_result, &st.flclash_tag) {
                    log(&event);
                }
                let _ = save_json("vpn.json", &s2);
            }
            Err(e) => log(&t!("ядро не обновилось: {0}", e)),
        }
    }
}

