#[cfg(test)]
fn agent(timeout: u64, via_proxy: Option<(u16, bool)>) -> ureq::Agent {
    agent_with_redirects(timeout, via_proxy, 5)
}

fn agent_with_redirects(timeout: u64, via_proxy: Option<(u16, bool)>, redirects: u32) -> ureq::Agent {
    // timeout_read: если загрузка встала (сервер молчит 30 с), не ждать общий таймаут
    let mut b = ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(10))
        .timeout_read(Duration::from_secs(30))
        .timeout(Duration::from_secs(timeout))
        .redirects(redirects)
        .user_agent(UA);
    if let Some((p, ipv6)) = via_proxy {
        let host = if ipv6 { "[::1]" } else { "127.0.0.1" };
        if let Ok(px) = ureq::Proxy::new(format!("http://{host}:{p}")) {
            b = b.proxy(px);
        }
    }
    b.build()
}

fn safe_ureq_error(error: &ureq::Error) -> String {
    match error {
        ureq::Error::Status(code, _) => format!("HTTP {code}"),
        ureq::Error::Transport(transport) => match transport.kind() {
            ureq::ErrorKind::InvalidUrl => t!("неверный URL").into(),
            ureq::ErrorKind::UnknownScheme => t!("неподдерживаемая схема URL").into(),
            ureq::ErrorKind::Dns => t!("ошибка DNS").into(),
            ureq::ErrorKind::InsecureRequestHttpsOnly => t!("небезопасная HTTP схема").into(),
            ureq::ErrorKind::ConnectionFailed => t!("ошибка соединения").into(),
            ureq::ErrorKind::TooManyRedirects => t!("слишком много перенаправлений").into(),
            ureq::ErrorKind::BadStatus => t!("неверный HTTP ответ").into(),
            ureq::ErrorKind::BadHeader => t!("неверный HTTP заголовок").into(),
            ureq::ErrorKind::Io => t!("ошибка ввода-вывода").into(),
            ureq::ErrorKind::InvalidProxyUrl => t!("неверный адрес прокси").into(),
            ureq::ErrorKind::ProxyConnect => t!("ошибка соединения с прокси").into(),
            ureq::ErrorKind::ProxyUnauthorized => t!("ошибка авторизации прокси").into(),
            ureq::ErrorKind::HTTP => t!("ошибка HTTP").into(),
        },
    }
}

/// GET напрямую; если не вышло — через наш VPN (если работает), затем через прокси FlClash (если запущен).
fn get(url: &str, timeout: u64, port: u16) -> Result<ureq::Response, String> {
    get_with_redirects(url, timeout, port, 5)
}

fn get_with_redirects(url: &str, timeout: u64, port: u16, redirects: u32) -> Result<ureq::Response, String> {
    get_with_redirects_user_agent(url, timeout, port, redirects, UA)
}

fn get_with_redirects_user_agent(url: &str, timeout: u64, port: u16, redirects: u32, user_agent: &str) -> Result<ureq::Response, String> {
    let mut errs = match agent_with_redirects(timeout, None, redirects).get(url).set("User-Agent", user_agent).call() {
        Ok(r) => return Ok(r),
        Err(e) => vec![t!("напрямую: {}", safe_ureq_error(&e))],
    };
    let mut via: Vec<((u16, bool), &str)> = vec![];
    if running() {
        via.push(((port, false), t!("через VPN")));
    }
    via.extend(flclash_ports().into_iter().map(|p| (p, t!("через FlClash"))));
    for ((p, ipv6), what) in via {
        match agent_with_redirects(timeout, Some((p, ipv6)), redirects).get(url).set("User-Agent", user_agent).call() {
            Ok(r) => return Ok(r),
            Err(e) => errs.push(format!("{what} :{p}: {}", safe_ureq_error(&e))),
        }
    }
    Err(errs.join("; "))
}

fn subscription_url(raw: &str) -> Result<Url, String> {
    let url = Url::parse(raw).map_err(|_| t!("некорректный URL подписки").to_string())?;
    if url.scheme() != "https" {
        return Err(t!("URL подписки должен использовать HTTPS; HTTP не поддерживается").into());
    }
    if url.host_str().filter(|host| !host.is_empty()).is_none() {
        return Err(t!("у URL подписки отсутствует host").into());
    }
    Ok(url)
}

fn get_subscription(raw: &str, timeout: u64, port: u16, user_agent: &str) -> Result<ureq::Response, String> {
    const MAX_REDIRECTS: u32 = 5;
    let mut current = subscription_url(raw)?;
    for redirects in 0..=MAX_REDIRECTS {
        let response = get_with_redirects_user_agent(current.as_str(), timeout, port, 0, user_agent)?;
        if !matches!(response.status(), 301 | 302 | 303 | 307 | 308) {
            if (300..400).contains(&response.status()) {
                return Err(t!("HTTP {} вместо профиля подписки", response.status()));
            }
            return Ok(response);
        }
        if redirects == MAX_REDIRECTS {
            return Err(t!("слишком много перенаправлений подписки").into());
        }
        let location = response.header("location").ok_or(t!("в перенаправлении нет Location"))?;
        current = subscription_url(current.join(location).map_err(|_| t!("неверный адрес перенаправления подписки"))?.as_str())?;
    }
    unreachable!()
}

/// IPv6 адрес в формате /proc/net/tcp6: каждое 32-битное слово выведено в native endian.
fn proc_ipv6_addr(raw: &str) -> Option<std::net::Ipv6Addr> {
    if raw.len() != 32 {
        return None;
    }
    let mut octets = [0u8; 16];
    for (i, word) in raw.as_bytes().as_chunks::<8>().0.iter().enumerate() {
        let word = std::str::from_utf8(word).ok()?;
        let bytes = u32::from_str_radix(word, 16).ok()?.to_ne_bytes();
        octets[i * 4..i * 4 + 4].copy_from_slice(&bytes);
    }
    Some(std::net::Ipv6Addr::from(octets))
}

fn flclash_listen_ports(tcp: &str, tcp6: &str, inodes: &std::collections::HashSet<String>) -> Vec<(u16, bool)> {
    let mut ports = vec![];
    for line in tcp.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        // локальный адрес 127.0.0.1 или 0.0.0.0, состояние 0A — LISTEN
        let Some((addr, port)) = f.get(1).and_then(|a| a.split_once(':')) else { continue };
        if f.get(3) != Some(&"0A") || !["0100007F", "00000000"].contains(&addr) || !f.get(9).map(|i| inodes.contains(*i)).unwrap_or(false) {
            continue;
        }
        if let Ok(p) = u16::from_str_radix(port, 16)
            && p != 9090 && !ports.iter().any(|(seen, _)| *seen == p) {
            ports.push((p, false));
        }
    }
    for line in tcp6.lines().skip(1) {
        let f: Vec<&str> = line.split_whitespace().collect();
        let Some((addr, port)) = f.get(1).and_then(|a| a.split_once(':')) else { continue };
        let Some(addr) = proc_ipv6_addr(addr) else { continue };
        if f.get(3) != Some(&"0A")
            || !(addr.is_loopback() || addr.is_unspecified())
            || !f.get(9).map(|i| inodes.contains(*i)).unwrap_or(false)
        {
            continue;
        }
        if let Ok(p) = u16::from_str_radix(port, 16)
            && p != 9090 && !ports.iter().any(|(seen, _)| *seen == p) {
            ports.push((p, true));
        }
    }
    ports
}

/// Реальный uid процесса из /proc/PID/status.
fn proc_uid(pid_dir: &Path) -> Option<u32> {
    fs::read_to_string(pid_dir.join("status")).ok()?.lines().find_map(|l| l.strip_prefix("Uid:")?.split_whitespace().next()?.parse().ok())
}

/// Чьим прокси можно доверить адрес подписки: root и пользователь, запустивший cm.
fn trusted_uids() -> Vec<u32> {
    let mut uids = vec![0, crate::common::sys::uid()];
    for k in ["SUDO_UID", "PKEXEC_UID"] {
        if let Some(uid) = std::env::var(k).ok().and_then(|v| v.parse().ok()) {
            uids.push(uid);
        }
    }
    if let Some(user) = invoking_user()
        && let Ok(uid) = out("id", &["-u", &user]).0.trim().parse() {
        uids.push(uid);
    }
    uids
}

/// Локальные mixed-порты FlClashCore; bool указывает, что listener доступен по IPv6.
/// Имя процесса задаёт он сам, поэтому слушатель принимается только от процесса root или пользователя cm.
fn flclash_ports() -> Vec<(u16, bool)> {
    let mut inodes = std::collections::HashSet::new();
    let trusted = trusted_uids();
    for e in fs::read_dir("/proc").into_iter().flatten().flatten() {
        if fs::read_to_string(e.path().join("comm")).unwrap_or_default().trim() != "FlClashCore" {
            continue;
        }
        if !proc_uid(&e.path()).map(|uid| trusted.contains(&uid)).unwrap_or(false) {
            continue;
        }
        for fd in fs::read_dir(e.path().join("fd")).into_iter().flatten().flatten() {
            let l = fs::read_link(fd.path()).map(|l| l.to_string_lossy().into_owned()).unwrap_or_default();
            if let Some(i) = l.strip_prefix("socket:[").and_then(|x| x.strip_suffix(']')) {
                inodes.insert(i.to_string());
            }
        }
    }
    if inodes.is_empty() {
        return vec![];
    }
    let tcp = fs::read_to_string("/proc/net/tcp").unwrap_or_default();
    let tcp6 = fs::read_to_string("/proc/net/tcp6").unwrap_or_default();
    flclash_listen_ports(&tcp, &tcp6, &inodes)
}

/// Как read_limited, но пишет в лог прогресс каждые 10% (для больших файлов).
fn read_progress(r: ureq::Response, max: u64, log: Log) -> Result<Vec<u8>, String> {
    let total: u64 = r.header("content-length").and_then(|v| v.parse().ok()).unwrap_or(0);
    if total > max {
        return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
    }
    let mut rd = r.into_reader().take(max.saturating_add(1));
    let (mut b, mut buf, mut step) = (vec![], [0u8; 64 << 10], 1u64);
    loop {
        let n = rd.read(&mut buf).map_err(|e| t!("загрузка прервалась на {}: {1}", fmt_bytes(b.len() as u64), e))?;
        if n == 0 {
            return Ok(b);
        }
        if (b.len() + n) as u64 > max {
            return Err(t!("ответ превышает лимит {}", fmt_bytes(max)));
        }
        b.extend_from_slice(&buf[..n]);
        if total > 0 && b.len() as u64 * 10 >= total * step {
            log(&t!("  {}% ({} из {})", step * 10, fmt_bytes(b.len() as u64), fmt_bytes(total)));
            step += 1;
        }
    }
}

/// Счётчик из заголовка: десятичное целое. Целое в записи с плавающей точкой принимается, пока оно точное (< 2^53);
/// отрицательное, дробное и слишком большое значение — «неизвестно» (0), а не насыщение до u64::MAX.
fn userinfo_num(v: &str) -> Option<u64> {
    let v = v.trim();
    if let Ok(n) = v.parse::<u64>() {
        return Some(n);
    }
    let f: f64 = v.parse().ok()?;
    (f.is_finite() && f >= 0.0 && f.fract() == 0.0 && f < 9_007_199_254_740_992.0).then_some(f as u64)
}

fn parse_userinfo(h: &str) -> SubInfo {
    let mut i = SubInfo::default();
    for part in h.split(';') {
        let Some((k, v)) = part.trim().split_once('=') else { continue };
        let n = userinfo_num(v).unwrap_or(0);
        match k.trim() {
            "upload" => i.upload = n,
            "download" => i.download = n,
            "total" => i.total = n,
            "expire" => i.expire = i64::try_from(n).unwrap_or(0),
            _ => {}
        }
    }
    i
}

/// Only retry successful HTTP responses with unusable bodies. Transport/auth errors
/// stop immediately; never send the private URL to a conversion service.
fn negotiate_subscription<T>(preferred: &str, mut request: impl FnMut(&str) -> Result<(String, T), String>) -> Result<(String, (T, String, usize)), String> {
    let mut agents = Vec::new();
    if !preferred.is_empty() {
        validate_user_agent(preferred)?;
        agents.push(preferred);
    }
    for agent in SUBSCRIPTION_AGENTS {
        if !agents.contains(&agent) { agents.push(agent); }
    }
    let mut errors = Vec::new();
    for agent in agents {
        let (body, metadata) = request(agent)?;
        match classify_sub(&body) {
            Ok((kind, nodes)) => return Ok((body, (metadata, kind, nodes))),
            Err(error) => errors.push(format!("{agent}: {error}")),
        }
    }
    Err(errors.join("; "))
}

/// Скачать подписку, понять формат, сохранить профиль. Меняет sub на месте.
fn fetch_sub(sub: &mut Sub, c: &Config) -> Result<ProfileChange, String> {
    let previous_kind = sub.kind.clone();
    let (body, (accepted, kind, nodes)) = negotiate_subscription(&sub.user_agent, |user_agent| {
        let r = get_subscription(&sub.url, 15, c.vpn_port, user_agent)?;
        let mut candidate = sub.clone();
    if let Some(h) = r.header("subscription-userinfo") {
        candidate.info = Some(parse_userinfo(h));
    }
    if let Some(h) = r.header("profile-update-interval").and_then(|v| v.trim().parse::<i64>().ok()) {
        candidate.interval_h = h.clamp(1, MAX_HOURS);
    }
    candidate.name = sanitize_profile_name(&candidate.name);
    if candidate.name.is_empty() {
        const MAX_TITLE_HEADER: usize = 4096;
        const MAX_DISPOSITION_HEADER: usize = 8192;
        let title = r.header("profile-title").filter(|t| t.len() <= MAX_TITLE_HEADER).map(|t| match t.strip_prefix("base64:") {
            Some(b) => b64_decode(b).map(|v| String::from_utf8_lossy(&v).into_owned()).unwrap_or_default(),
            None => t.to_string(),
        }).map(|name| sanitize_profile_name(&name)).filter(|name| !name.is_empty());
        let file = r.header("content-disposition").filter(|d| d.len() <= MAX_DISPOSITION_HEADER).and_then(|d| {
            d.split(';').find_map(|p| {
                let p = p.trim();
                p.strip_prefix("filename*=UTF-8''").map(pct_decode).or_else(|| p.strip_prefix("filename=").map(|x| x.trim_matches('"').to_string()))
            })
        }).map(|name| sanitize_profile_name(&name)).filter(|name| !name.is_empty());
        candidate.name = title.or(file).unwrap_or_else(|| sanitize_profile_name(host_of(&mask_url(&candidate.url))));
    }
        let body = String::from_utf8_lossy(&read_limited(r, 32 << 20)?).into_owned();
        Ok((body, candidate))
    })?;
    private_dir(&format!("{}/profiles", home()))?;
    let target = PathBuf::from(profile_path(sub, &kind));
    let temp = stage_profile(&target, &body, &kind, nodes)?;
    let old_extension = match previous_kind.as_str() {
        "uri" => Some(PathBuf::from(profile_path(sub, "uri"))),
        "clash" => Some(PathBuf::from(profile_path(sub, "clash"))),
        _ => None,
    };
    let change = match install_profile(&temp, &target, old_extension) {
        Ok(change) => change,
        Err(e) => {
            let _ = remove_profile_file(&temp);
            return Err(e);
        }
    };
    *sub = accepted;
    sub.kind = kind;
    sub.nodes = nodes;
    sub.updated = now();
    sub.error.clear();
    Ok(change)
}

