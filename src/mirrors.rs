//! Замер и подбор зеркал, реакция на смену сети, проверка и предзагрузка обновлений.

use crate::backend::{Backend, ProbeKind};
use crate::common::*;
use crate::extras;
use std::io::Read;
use std::path::Path;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const PROBE_BYTES: u64 = 3 << 20; // 3 МБ: на маленьком файле не видно, как прокси/провайдер душит скорость
const PROBE_MIN: u64 = 256 << 10; // меньше этого к таймауту — зеркало нерабочее

pub fn agent(timeout: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout))
        .user_agent("cm (mirror probe)")
        .build()
}

fn classify(msg: &str) -> String {
    if std::env::var_os("CM_DEBUG").is_some() {
        eprintln!("[debug] {msg}");
    }
    let m = msg.to_lowercase();
    if m.contains("dns") || m.contains("resolve") || m.contains("lookup") {
        "no DNS"
    } else if m.contains("timed out") || m.contains("timeout") || m.contains("deadline") {
        "timeout"
    } else if m.contains("reset") || m.contains("eof") || m.contains("end of file") || m.contains("broken pipe") || m.contains("aborted") {
        // соединение оборвали посередине — чаще всего прокси/VPN под нагрузкой, повтор обычно помогает
        "dropped"
    } else if m.contains("certificate") || m.contains("tls") || m.contains("handshake") {
        "TLS error"
    } else {
        "no connection"
    }
    .into()
}

/// Проверить ответ на captive portal и формат контрольного файла.
fn validate_probe(kind: ProbeKind, body: &[u8]) -> Result<(), String> {
    if body.is_empty() {
        return Err(t!("пустой ответ").into());
    }
    let prefix = String::from_utf8_lossy(&body[..body.len().min(1024)]).to_ascii_lowercase();
    if ["<html", "<!doctype html", "<head", "<body"].iter().any(|needle| prefix.contains(needle)) {
        return Err(t!("вместо файла получен HTML").into());
    }
    match kind {
        ProbeKind::AptInRelease => {
            let text = String::from_utf8_lossy(body);
            if !text.starts_with("-----BEGIN PGP SIGNED MESSAGE-----")
                || !text.contains("\nSHA256:")
                || !text.contains("-----BEGIN PGP SIGNATURE-----")
            {
                return Err(t!("ответ не похож на APT InRelease").into());
            }
        }
        ProbeKind::PacmanDb => {
            let gzip = body.starts_with(&[0x1f, 0x8b]);
            let known_compressed = gzip
                || body.starts_with(&[0x28, 0xb5, 0x2f, 0xfd]) // zstd
                || body.starts_with(b"BZh")
                || body.starts_with(&[0xfd, b'7', b'z', b'X', b'Z', 0x00]) // xz
                || body.starts_with(&[0x04, 0x22, 0x4d, 0x18]) // lz4
                || body.starts_with(&[0x89, b'L', b'Z', b'O', 0x00, 0x0d, 0x0a, 0x1a, 0x0a]) // lzop
                || body.starts_with(b"LRZI"); // lrzip
            let tar = body.len() >= 512 && valid_tar_header(&body[..512]);
            if !known_compressed && !tar {
                return Err(t!("ответ не похож на базу pacman").into());
            }
            if gzip {
                let mut decoder = flate2::read::GzDecoder::new(body);
                let mut header = [0u8; 512];
                if decoder.read_exact(&mut header).is_err() || !valid_tar_header(&header) {
                    return Err(t!("повреждённая база pacman").into());
                }
            }
        }
        ProbeKind::Generic => {}
    }
    Ok(())
}

fn valid_tar_header(header: &[u8]) -> bool {
    if header.len() < 512 || !header[..100].iter().any(|b| *b != 0) {
        return false;
    }
    let checksum = String::from_utf8_lossy(&header[148..156]);
    let checksum = checksum.trim_matches(|c| c == '\0' || c == ' ');
    let Ok(expected) = u64::from_str_radix(checksum, 8) else { return false };
    let actual: u64 = header.iter().enumerate().map(|(i, b)| if (148..156).contains(&i) { 32 } else { *b as u64 }).sum();
    actual == expected
}

fn content_range_len(value: &str) -> Option<(u64, u64)> {
    let (unit, rest) = value.trim().split_once(' ')?;
    if unit != "bytes" {
        return None;
    }
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let start: u64 = start.parse().ok()?;
    let end: u64 = end.parse().ok()?;
    if total != "*" && total.parse::<u64>().ok().filter(|n| *n > end).is_none() {
        return None;
    }
    (end >= start).then_some((start, end - start + 1))
}

/// Замеряет первые 3 МБ, сверяя размер ответа и формат файла до ранжирования.
fn probe_speed(url: &str, timeout: u64, kind: ProbeKind) -> (bool, f64, String) {
    let start = Instant::now();
    let resp = match agent(timeout).get(url).set("Range", &format!("bytes=0-{}", PROBE_BYTES - 1)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return (false, 0.0, format!("HTTP {code}")),
        Err(e) => return (false, 0.0, classify(&e.to_string())),
    };
    let status = resp.status();
    if status != 200 && status != 206 {
        return (false, 0.0, format!("HTTP {status}"));
    }
    let content_length = resp.header("content-length").and_then(|v| v.parse::<u64>().ok());
    let content_range = resp.header("content-range");
    let expected = if status == 206 {
        let Some(value) = content_range else { return (false, 0.0, t!("неверный Content-Range").into()) };
        let Some((start, range_len)) = content_range_len(value) else { return (false, 0.0, t!("неверный Content-Range").into()) };
        if start != 0 || content_length.map(|n| n != range_len).unwrap_or(false) || range_len > PROBE_BYTES {
            return (false, 0.0, t!("неверный размер Range-ответа").into());
        }
        Some(range_len)
    } else {
        if content_range.is_some() {
            return (false, 0.0, t!("неожиданный Content-Range").into());
        }
        content_length
    };
    let mut rd = resp.into_reader().take(PROBE_BYTES);
    let mut buf = vec![0u8; 64 << 10];
    let mut body = Vec::with_capacity(PROBE_BYTES as usize);
    let mut n: u64 = 0;
    loop {
        match rd.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => {
                n += k as u64;
                body.extend_from_slice(&buf[..k]);
            }
            Err(e) => {
                if n >= PROBE_MIN
                    && !expected.map(|size| size > n).unwrap_or(false)
                    && matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock)
                {
                    break;
                }
                return (false, 0.0, classify(&e.to_string()));
            }
        }
    }
    if let Some(size) = expected {
        if size <= PROBE_BYTES && n != size {
            return (false, 0.0, t!("ответ обрезан относительно Content-Length").into());
        }
        if size > PROBE_BYTES && n != PROBE_BYTES {
            return (false, 0.0, t!("ответ обрезан относительно Content-Length").into());
        }
    }
    if let Err(e) = validate_probe(kind, &body) {
        return (false, 0.0, e);
    }
    let el = start.elapsed().as_secs_f64();
    if n == 0 || el <= 0.0 {
        return (false, 0.0, t!("пусто").into());
    }
    (true, n as f64 / el, String::new())
}

fn fetch_head(url: &str) -> Option<String> {
    let r = agent(8).get(url).set("Range", "bytes=0-4095").call().ok()?;
    let mut s = String::new();
    r.into_reader().take(4096).read_to_string(&mut s).ok()?;
    Some(s)
}

struct Job {
    idx: usize,
    probe: String,
    fresh: Option<String>,
    kind: ProbeKind,
}

struct Done {
    idx: usize,
    ok: bool,
    speed: f64,
    err: String,
    fresh_body: Option<String>,
}

fn run_job(j: &Job, timeout: u64) -> Done {
    let (ok, speed, err) = probe_speed(&j.probe, timeout, j.kind);
    let fresh_body = if ok { j.fresh.as_deref().and_then(fetch_head) } else { None };
    Done { idx: j.idx, ok, speed, err, fresh_body }
}

/// Параллельный замер: скорость + (для рабочих) дата синхронизации.
/// Потоков не больше MAX_PARALLEL; после `deadline` новые замеры не начинаются.
fn measure(jobs: Vec<Job>, parallel: usize, timeout: u64, deadline: Option<Instant>, on_done: &mut dyn FnMut(&Done)) -> Vec<Done> {
    let n = jobs.len();
    let queue = Arc::new(Mutex::new(jobs));
    let (tx, rx) = mpsc::channel();
    let mut workers = 0;
    for _ in 0..parallel.clamp(1, MAX_PARALLEL).min(n.max(1)) {
        let (q, tx) = (queue.clone(), tx.clone());
        let spawned = spawn_thread("cm-mirror", move || loop {
            if deadline.map(|d| Instant::now() >= d).unwrap_or(false) {
                break;
            }
            let Some(j) = q.lock().unwrap_or_else(|e| e.into_inner()).pop() else { break };
            let _ = tx.send(run_job(&j, timeout));
        });
        // ОС не дала поток — замеряем оставшимися; ни одного — в этом потоке
        if spawned.is_err() {
            break;
        }
        workers += 1;
    }
    if workers == 0 {
        loop {
            if deadline.map(|d| Instant::now() >= d).unwrap_or(false) {
                break;
            }
            let Some(j) = queue.lock().unwrap_or_else(|e| e.into_inner()).pop() else { break };
            let _ = tx.send(run_job(&j, timeout));
        }
    }
    drop(tx);
    let mut res = vec![];
    for d in rx {
        on_done(&d);
        res.push(d);
    }
    res
}

#[derive(Clone)]
pub struct Cand {
    pub url: String,
    pub src: &'static str,
}

/// Кандидаты без повторов: множество адресов с сохранением порядка (O(1) на проверку),
/// каждый источник ограничен MAX_MIRRORS адресов до дедупликации.
pub fn candidates(b: &dyn Backend, c: &Config, extra: Option<&[String]>) -> Vec<Cand> {
    let mut r: Vec<Cand> = vec![];
    let mut seen = std::collections::HashSet::new();
    let mut add = |list: &[String], src: &'static str| {
        for u in list.iter().take(MAX_MIRRORS) {
            if seen.insert(u.trim_end_matches('/').to_string()) {
                r.push(Cand { url: u.clone(), src });
            }
        }
    };
    add(&c.mirrors, "config");
    add(&b.pinned(), "pinned");
    match extra {
        Some(e) => add(e, "search"),
        None => {
            let l = b.list_mirrors();
            add(&l[..l.len().min(c.extra_from_list)], "list");
        }
    }
    r
}

/// Временный сбой: повтор может помочь. 404, ошибка формата, DNS и TLS повтором не лечатся.
fn transient_error(err: &str) -> bool {
    match err {
        "timeout" | "dropped" | "no connection" => true,
        _ => err
            .strip_prefix("HTTP ")
            .and_then(|c| c.parse::<u16>().ok())
            .map(|c| c == 429 || (500..600).contains(&c))
            .unwrap_or(false),
    }
}

fn mirror_needs_retry(src: &str, ok: bool, err: &str, had_prior_ok: bool) -> bool {
    !ok && transient_error(err) && (err == "dropped" || (src != "search" && src != "list") || had_prior_ok)
}

/// Повторный проход: не больше стольких адресов и потоков, и общий бюджет времени.
const RETRY_MAX: usize = 16;
const RETRY_POOL: usize = 4;

/// Второй проход запускается по списку повторов, в том числе когда первый проход не дал ни одного ответа.
fn schedule_mirror_retry(any_ok: bool, retry_count: usize) -> bool {
    let _ = any_ok;
    retry_count > 0
}

/// Замер кандидатов и закрепление лучших. Возвращает число рабочих или ошибку применения.
pub fn check_mirrors(b: &dyn Backend, c: &Config, log: Log, extra: Option<&[String]>, fallback: Option<&[String]>) -> Result<usize, String> {
    let net = fingerprint();
    let cands = candidates(b, c, extra);
    let parallel = if net.vpn { c.parallel_vpn } else { c.parallel };
    log(&t!("замеряю зеркала: {} шт. (по {} МБ, таймаут {} с, параллельно {}{})", cands.len(), PROBE_BYTES >> 20, c.timeout, parallel, if net.vpn { t!(" — через VPN") } else { "" }));
    let mk = |i: usize, cd: &Cand| Job { idx: i, probe: b.probe_url(&cd.url), fresh: b.fresh_url(&cd.url), kind: b.probe_kind() };
    let mut res: Vec<Probe> = cands.iter().map(|cd| Probe { url: cd.url.clone(), src: cd.src.into(), ..Default::default() }).collect();
    let mut fresh: Vec<Option<i64>> = vec![None; cands.len()];
    let apply = |d: &Done, res: &mut Vec<Probe>, fresh: &mut Vec<Option<i64>>| {
        let p = &mut res[d.idx];
        p.ok = d.ok;
        p.speed = d.speed;
        p.err = d.err.clone();
        fresh[d.idx] = d.fresh_body.as_deref().and_then(|s| b.parse_fresh(s));
    };
    let jobs: Vec<Job> = cands.iter().enumerate().map(|(i, cd)| mk(i, cd)).collect();
    let done = measure(jobs, parallel, c.timeout, None, &mut |d| {
        let mark = if d.ok { "✓" } else { "✗" };
        let sp = Probe { ok: d.ok, speed: d.speed, err: d.err.clone(), ..Default::default() };
        log(&format!("  {mark} {:<11} {}", fmt_speed(&sp), cands[d.idx].url));
    });
    for d in &done {
        apply(d, &mut res, &mut fresh);
    }

    // Повтор: один замер — лотерея. Повторяем быстрые отказы (обрыв), а также зеркала,
    // которые заданы вручную или уже работали в этой сети.
    let st0 = load_mirror_state();
    let mem0 = st0.networks.get(&net.id).cloned().unwrap_or_default();
    let retry: Vec<Job> = res
        .iter()
        .enumerate()
        .filter(|(_, p)| mirror_needs_retry(&p.src, p.ok, &p.err, mem0.stats.get(&p.url).map(|s| s.ok > 0).unwrap_or(false)))
        .map(|(i, _)| mk(i, &cands[i]))
        .take(RETRY_MAX)
        .collect();
    if schedule_mirror_retry(res.iter().any(|p| p.ok), retry.len()) {
        log(&t!("  повторный замер упавших: {}", retry.len()));
        // небольшой пул и общий бюджет: серия недоступных зеркал не складывается в сумму таймаутов
        let budget = Instant::now() + Duration::from_secs(c.timeout.saturating_mul(3).max(10));
        for d in measure(retry, parallel.min(RETRY_POOL), c.timeout, Some(budget), &mut |d| {
            if d.ok {
                log(&t!("  ✓ со второй попытки: {}", cands[d.idx].url));
            }
        }) {
            if d.ok {
                apply(&d, &mut res, &mut fresh);
            }
        }
    }

    // Свежесть: отстающее зеркало быстро отдаёт старую базу, а потом pacman получает 404.
    if let Some(newest) = fresh.iter().flatten().max().copied() {
        for (p, f) in res.iter_mut().zip(&fresh) {
            if let (true, Some(t)) = (p.ok, f) {
                let lag = (newest - t).max(0) as f64 / 3600.0;
                p.lag_h = Some((lag * 10.0).round() / 10.0);
                if lag > c.max_lag_h as f64 {
                    p.ok = false;
                    // идентификатор, а не текст: по нему ниже узнают отставшие зеркала; переводится при показе
                    p.err = format!("lag {lag:.0} h");
                    log(&t!("  ⌛ {} отстаёт на {1:.0} ч — не беру", host_of(&p.url), lag));
                }
            }
        }
    }

    let any_ok = res.iter().any(|p| p.ok);
    let mut st = load_mirror_state();
    st.checked = now();
    st.hints.clear();
    if st.pending_fingerprint != net.id {
        st.pending_apply = false;
        st.pending_fingerprint.clear();
        st.pending_label.clear();
        st.pending_fallback = None;
        st.apply_error.clear();
    }
    if any_ok {
        // Сглаживание: скорость по истории замеров в этой сети, чтобы один удачный всплеск не решал всё.
        let mem = st.networks.entry(net.id.clone()).or_default();
        for p in res.iter_mut() {
            let s = mem.stats.entry(p.url.clone()).or_default();
            if p.ok {
                s.ewma = if s.ok == 0 { p.speed } else { 0.5 * s.ewma + 0.5 * p.speed };
                s.ok += 1;
                s.streak_fail = 0;
            } else {
                s.ewma *= 0.5;
                s.fail += 1;
                s.streak_fail += 1;
            }
            s.last = now();
            p.score = s.ewma;
        }
        res.sort_by(|a, b| b.ok.cmp(&a.ok).then(b.score.total_cmp(&a.score)));
        let best: Vec<String> = res.iter().filter(|p| p.ok).map(|p| p.url.clone()).collect();
        mem.best = best.clone();
        mem.checked = st.checked;
        mem.label = net.label.clone();
        st.best = best;
        let cutoff = now() - 30 * 86400;
        st.networks.retain(|_, v| v.checked > cutoff);
        // Подсказка про прокси: вручную заданное зеркало не отвечает через VPN, а другие работают.
        if net.vpn {
            for p in res.iter().filter(|p| !p.ok && p.src == "config" && !p.err.starts_with("lag ")) {
                st.hints.push(t!("{} не отвечает через VPN. Если это зеркало в твоей стране — добавь {} в правила DIRECT своего VPN-клиента (FlClash и т.п.)", host_of(&p.url), host_of(&p.url)));
            }
        }
    } else {
        log(t!("ни одно зеркало не ответило — оставляю прежние"));
    }
    st.results = res;
    let apply_best = b.mirrors_managed() && !st.best.is_empty();
    if apply_best {
        st.pending_apply = true;
        st.pending_fingerprint = net.id.clone();
        st.pending_label = net.label.clone();
        st.pending_fallback = fallback.map(|items| items.to_vec());
        st.apply_error.clear();
    }
    let working = st.best.len() * any_ok as usize;
    let _ = save_json("mirrors.json", &st);
    for h in &st.hints {
        log(&format!("  💡 {h}"));
    }
    if apply_best {
        apply_mirrors(b, c, &st, fallback, log)?;
    }
    Ok(working)
}

pub fn load_mirror_state() -> MirrorState {
    load_json("mirrors.json")
}

pub fn desired_block(c: &Config, st: &MirrorState) -> Vec<String> {
    let src = if st.best.is_empty() { &c.mirrors } else { &st.best };
    src.iter().take(c.keep).cloned().collect()
}

pub fn apply_mirrors(b: &dyn Backend, c: &Config, st: &MirrorState, fallback: Option<&[String]>, log: Log) -> Result<bool, String> {
    if !b.mirrors_managed() {
        return Ok(false);
    }
    let block = desired_block(c, st);
    if block.is_empty() {
        log(t!("нечего закреплять: нет рабочих зеркал"));
        return Ok(false);
    }
    let net = fingerprint();
    let has_pending_target = st.pending_apply && !st.pending_fingerprint.is_empty();
    let target_fingerprint = if has_pending_target { st.pending_fingerprint.clone() } else { net.id.clone() };
    let previous_fallback = if has_pending_target { st.pending_fallback.clone() } else { None };
    let result = b.apply_mirrors(&block, fallback);
    let mut saved = load_mirror_state();
    saved.best = st.best.clone();
    match result {
        Err(e) => {
            log(&t!("не удалось записать зеркала: {0}", e));
            saved.pending_apply = true;
            saved.pending_fingerprint = target_fingerprint;
            saved.pending_label = if has_pending_target { st.pending_label.clone() } else { net.label.clone() };
            saved.pending_fallback = fallback.map(|items| items.to_vec()).or(previous_fallback);
            saved.apply_error = e.clone();
            let _ = save_json("mirrors.json", &saved);
            Err(e)
        }
        Ok(changed) => {
            if changed {
                log(&t!("закреплено зеркал: {}, первое: {}", block.len(), block[0]));
            } else {
                log(t!("зеркала уже на месте"));
            }
            saved.fingerprint = target_fingerprint;
            saved.label = if has_pending_target && !st.pending_label.is_empty() { st.pending_label.clone() } else { net.label };
            saved.pending_apply = false;
            saved.pending_fingerprint.clear();
            saved.pending_label.clear();
            saved.pending_fallback = None;
            saved.apply_error.clear();
            let _ = save_json("mirrors.json", &saved);
            Ok(changed)
        }
    }
}

/// Полный подбор: ищем зеркала рядом с текущим местом и замеряем их.
pub fn rescan(b: &dyn Backend, c: &Config, log: Log) -> Result<usize, String> {
    if !b.mirrors_managed() {
        log(&b.mirror_note());
        return Ok(0);
    }
    log(t!("ищу зеркала рядом с текущим местоположением..."));
    match b.discover(c.rescan_count, log) {
        Ok((cands, fallback)) if cands.len() >= 3 => check_mirrors(b, c, log, Some(&cands), fallback.as_deref()),
        Ok(_) => {
            log(t!("автопоиск нашёл слишком мало зеркал — замеряю известные"));
            check_mirrors(b, c, log, None, None)
        }
        Err(e) => {
            log(&t!("автопоиск не удался ({0}) — замеряю известные", e));
            check_mirrors(b, c, log, None, None)
        }
    }
}

/// Реакция на смену сети. Ok(true) означает, что изменение или повтор применения обработаны.
pub fn handle_network(b: &dyn Backend, c: &Config, log: Log, force: bool) -> Result<bool, String> {
    if !b.mirrors_managed() {
        return Ok(false);
    }
    let net = fingerprint();
    if !net.online {
        log(t!("сети нет — жду"));
        return Ok(false);
    }
    let st = load_mirror_state();
    if st.pending_apply && st.pending_fingerprint == net.id && !force {
        log(t!("предыдущее применение зеркал не завершилось — повторяю"));
        let fallback = st.pending_fallback.clone();
        apply_mirrors(b, c, &st, fallback.as_deref(), log)?;
        let mut applied = load_mirror_state();
        if applied.fingerprint == net.id && !applied.pending_apply && !applied.best.is_empty() {
            applied.event = t!("Сеть {}. Лучшее зеркало: {}", net.label, host_of(&applied.best[0]));
            applied.event_time = now();
            let _ = save_json("mirrors.json", &applied);
        }
        return Ok(true);
    }
    if st.fingerprint == net.id && !force {
        log(&t!("сеть та же ({})", net.label));
        return Ok(false);
    }
    log(&t!("сеть: {} (было: {})", net.label, if st.label.is_empty() { "—" } else { &st.label }));
    let fresh = st
        .networks
        .get(&net.id)
        .filter(|m| !m.best.is_empty() && !elapsed_at_least(m.checked, c.network_memory_days.clamp(0, 365) * 86400))
        .cloned();
    match fresh {
        Some(m) if !force => {
            log(t!("сеть знакомая — сразу ставлю её зеркала, потом перепроверяю"));
            let mut tmp = st.clone();
            tmp.best = m.best;
            tmp.label = net.label.clone();
            tmp.pending_apply = true;
            tmp.pending_fingerprint = net.id.clone();
            tmp.pending_label = net.label.clone();
            tmp.pending_fallback = None;
            tmp.apply_error.clear();
            let _ = save_json("mirrors.json", &tmp);
            let cached_result = apply_mirrors(b, c, &tmp, None, log);
            let checked_result = check_mirrors(b, c, log, None, None);
            if let Err(e) = checked_result {
                log(&t!("зеркала не применены: {0}", e));
            } else if let Err(e) = cached_result {
                log(&t!("кэшированные зеркала не применены: {0}", e));
            }
        }
        _ => {
            log(t!("новая сеть — полный подбор зеркал"));
            if let Err(e) = rescan(b, c, log) {
                log(&t!("зеркала не применены: {0}", e));
            }
        }
    }
    let mut st = load_mirror_state();
    if st.fingerprint == net.id && !st.pending_apply && !st.best.is_empty() {
        st.event = t!("Сеть сменилась ({}). Лучшее зеркало: {}", net.label, host_of(&st.best[0]));
        st.event_time = now();
        let _ = save_json("mirrors.json", &st);
    }
    if st.pending_apply {
        return Err(if st.apply_error.is_empty() { t!("не удалось применить зеркала").into() } else { st.apply_error });
    }
    Ok(true)
}

// ======================= обновления =======================

pub fn pkg_names(list: &[String]) -> Vec<String> {
    list.iter().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
}

/// Почему фоновую загрузку сейчас делать не стоит (ноутбук на батарее, лимитная сеть).
pub fn background_block(c: &Config) -> Option<String> {
    let net = fingerprint();
    if !c.prefetch_on_metered && metered(&net.dev) {
        return Some(t!("лимитная сеть — фоновая загрузка отложена").into());
    }
    if !c.prefetch_on_battery && on_battery() {
        return Some(t!("работа от батареи — фоновая загрузка отложена").into());
    }
    None
}

fn cache_space_for_target(target: &str) -> (String, String) {
    let mut existing = Path::new(target);
    while !existing.is_dir() {
        let Some(parent) = existing.parent() else { break };
        if parent == existing {
            break;
        }
        existing = parent;
    }
    (target.to_string(), existing.to_string_lossy().into_owned())
}

fn cache_space_path(b: &dyn Backend) -> (String, String) {
    for target in b.cache_dirs() {
        if Path::new(target).is_dir() {
            return (target.to_string(), target.to_string());
        }
    }
    cache_space_for_target(b.cache_dirs().first().copied().unwrap_or("/"))
}

/// Проверка: базы, список пакетов, Flatpak, прошивки, новости, размер загрузки и место на диске.
pub fn gather(b: &dyn Backend, c: &Config, log: Log, quiet: bool, invoking_user: Option<&str>) -> UpdState {
    let mut st = UpdState { checked: now(), ..Default::default() };
    let mut res = b.refresh(quiet);
    if let Err(e) = &res {
        log(&t!("не удалось обновить базы пакетов: {0}", e));
        if b.mirrors_managed() {
            let _ = rescan(b, c, log);
            res = b.refresh(quiet);
        }
    }
    if res.is_err() {
        st.error = t!("не удалось обновить базы пакетов").into();
        st.package_check_failure = Some(PackageCheckFailure::Refresh);
        let _ = save_json("updates.json", &st);
        return st;
    }
    match b.updates() {
        Ok(l) => st.list = l,
        Err(e) => {
            st.error = e;
            st.package_check_failure = Some(PackageCheckFailure::UpdateList);
        }
    }
    if c.flatpak {
        let check = extras::flatpak_updates(invoking_user);
        st.flatpak = check.updates;
        st.flatpak_error = check.error;
    }
    if c.firmware && extras::has_fwupd() {
        match extras::firmware_refresh() {
            Ok(()) => match extras::firmware_updates() {
                Ok(updates) => st.firmware = updates,
                Err(e) => st.firmware_error = e,
            },
            Err(e) => st.firmware_error = e,
        }
    }
    if c.news && b.arch_news() {
        match extras::arch_news(b.last_upgrade()) {
            Ok(n) => st.news = n,
            Err(e) => log(&t!("новости Arch не загрузились: {0}", e)),
        }
    }
    if !st.list.is_empty() {
        st.download_size = b.download_size(&pkg_names(&st.list));
        let (dir, space_dir) = cache_space_path(b);
        let need = required_free(st.download_size, c.min_free_gb);
        let location = if dir == space_dir {
            t!("кэш {0}", dir)
        } else {
            t!("кэш {0}, место проверено по {1}", dir, space_dir)
        };
        match free_space(&space_dir) {
            Ok(free) if free == 0 || free < need => {
                st.error = low_space_message(st.download_size, c.min_free_gb, &location, free);
            }
            Ok(_) => {}
            Err(e) => {
                let message = t!("не удалось проверить свободное место ({0}): {1}", space_dir, e);
                st.space_check_error = Some(message.clone());
                st.error = message;
            }
        }
    }
    let size_label = match st.download_size {
        Some(0) if !st.list.is_empty() => t!(" (уже скачаны)").into(),
        Some(size) => format!(" ({})", fmt_bytes(size)),
        None if !st.list.is_empty() => t!(" (полный объём неизвестен)").into(),
        None => String::new(),
    };
    log(&t!("обновлений: пакетов {}{}{}{}", st.list.len(), size_label, if st.flatpak.is_empty() { String::new() } else { format!(", Flatpak {}", st.flatpak.len()) }, if st.firmware.is_empty() { String::new() } else { t!(", прошивок {}", st.firmware.len()) }));
    if !st.news.is_empty() {
        log(&t!("новостей Arch с прошлого обновления: {}", st.news.len()));
    }
    if !st.error.is_empty() {
        log(&st.error);
    }
    if !st.flatpak_error.is_empty() {
        log(&t!("Flatpak: ошибка проверки: {}", st.flatpak_error));
    }
    if !st.firmware_error.is_empty() {
        log(&t!("Прошивки: ошибка проверки: {}", st.firmware_error));
    }
    let _ = save_json("updates.json", &st);
    st
}

fn required_free(download_size: Option<u64>, min_free_gb: u64) -> u64 {
    download_size.unwrap_or(0).saturating_add(min_free_gb.saturating_mul(1 << 30))
}

fn low_space_message(download_size: Option<u64>, min_free_gb: u64, location: &str, free: u64) -> String {
    let need = required_free(download_size, min_free_gb);
    match download_size {
        // «мало места» — отдельный ключ: по этому началу строки ошибку узнают в main.rs
        Some(_) => format!("{}{}", t!("мало места"), t!(" ({3}): нужно {} (с запасом {} ГБ), свободно {}", fmt_bytes(need), min_free_gb, fmt_bytes(free), location)),
        None => format!("{}{}", t!("мало места"), t!(" ({2}) для резерва {} ГБ при неизвестном объёме загрузки, свободно {}", min_free_gb, fmt_bytes(free), location)),
    }
}

/// Предзагрузка с повторами; на второй неудаче — переподбор зеркал.
pub fn download(b: &dyn Backend, c: &Config, st: &mut UpdState, log: Log, quiet: bool, allow_unknown_size: bool) {
    if let Some(error) = &st.space_check_error {
        log(&t!("загрузку пропускаю: {0}", error));
        return;
    }
    if st.error.starts_with(t!("мало места")) {
        log(t!("загрузку пропускаю: мало места"));
        return;
    }
    if !st.list.is_empty() && st.download_size.is_none() && !allow_unknown_size {
        let message = t!("полный объём загрузки неизвестен; предзагрузку пропускаю");
        log(message);
        st.skipped = message.into();
        let _ = save_json("updates.json", st);
        return;
    }
    if !st.list.is_empty() {
        let names = pkg_names(&st.list);
        for attempt in 1..=c.retries {
            log(&t!("скачиваю заранее (попытка {1} из {})...", c.retries, attempt));
            match b.prefetch(&names, quiet) {
                Ok(()) => {
                    st.downloaded = true;
                    log(t!("пакеты скачаны"));
                    break;
                }
                Err(e) => log(&t!("загрузка оборвалась: {0}", e)),
            }
            if attempt == 2 && b.mirrors_managed() {
                log(t!("похоже, зеркала плохие — подбираю заново"));
                let _ = rescan(b, c, log);
            }
        }
        if !st.downloaded {
            st.error = t!("не удалось скачать обновления").into();
        }
    }
    if !st.flatpak.is_empty() {
        log(t!("Flatpak: скачиваю заранее..."));
        let user = invoking_user();
        if let Err(e) = extras::flatpak_prefetch(quiet, user.as_deref(), &st.flatpak) {
            let message = t!("не удалось скачать обновления Flatpak: {0}", e);
            log(&message);
            if st.error.is_empty() {
                st.error = message;
            } else {
                st.error.push_str("; ");
                st.error.push_str(&message);
            }
        }
    }
    let _ = save_json("updates.json", st);
}

#[cfg(test)]
mod contract_tests {
    use super::*;

    /// B09: 20 000 различных адресов дедуплицируются быстро и с пределом на источник.
    #[test]
    fn b09_candidates_dedup_is_linear_and_bounded() {
        let mut c = Config::defaults(vec![]);
        c.mirrors = (0..20_000).map(|i| format!("https://m{i}.example/$repo/os/$arch")).collect();
        c.mirrors.push("https://m1.example/$repo/os/$arch/".into());
        let b = CacheDirsBackend { dirs: vec![] };
        let t0 = Instant::now();
        let r = candidates(&b, &c, Some(&c.mirrors.clone()));
        assert!(t0.elapsed() < Duration::from_secs(1), "{:?}", t0.elapsed());
        assert_eq!(r.len(), MAX_MIRRORS, "повтор из search не добавляется, у каждого источника предел");
    }

    /// B08: огромный parallel не создаёт больше MAX_PARALLEL потоков; бюджет останавливает очередь.
    #[test]
    fn b08_measure_caps_threads_and_honours_deadline() {
        let jobs: Vec<Job> = (0..200).map(|i| Job { idx: i, probe: "http://127.0.0.1:9/x".into(), fresh: None, kind: ProbeKind::Generic }).collect();
        let peak = std::sync::atomic::AtomicUsize::new(0);
        let tasks = || std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(0);
        let base = tasks();
        let past = Some(Instant::now());
        let done = measure(jobs, 10_000, 2, past, &mut |_| {
            peak.fetch_max(tasks(), std::sync::atomic::Ordering::Relaxed);
        });
        assert!(done.is_empty(), "после бюджета замеры не начинаются");
        let jobs: Vec<Job> = (0..40).map(|i| Job { idx: i, probe: "http://127.0.0.1:9/x".into(), fresh: None, kind: ProbeKind::Generic }).collect();
        let done = measure(jobs, 10_000, 2, None, &mut |_| {
            peak.fetch_max(tasks(), std::sync::atomic::Ordering::Relaxed);
        });
        assert_eq!(done.len(), 40);
        assert!(peak.load(std::sync::atomic::Ordering::Relaxed) <= base + MAX_PARALLEL + 4, "потоков: {}", peak.load(std::sync::atomic::Ordering::Relaxed));
    }
    use crate::backend::ProbeKind;
    use crate::common::{PackageCheckFailure, Probe};
    use std::fs;

    // --- NET-03 ---
    #[test]
    fn net03_rejects_html_captive_portal() {
        let body = b"<html><head><title>login</title></head><body>wifi</body></html>";
        assert!(validate_probe(ProbeKind::Generic, body).is_err());
    }

    #[test]
    fn net03_accepts_minimal_apt_inrelease() {
        let body = b"-----BEGIN PGP SIGNED MESSAGE-----\nHash: SHA256\n\nSuite: stable\nSHA256:\n\n-----BEGIN PGP SIGNATURE-----\n";
        assert!(validate_probe(ProbeKind::AptInRelease, body).is_ok());
    }

    #[test]
    fn net03_rejects_truncated_inrelease() {
        let body = b"-----BEGIN PGP SIGNED MESSAGE-----\n";
        assert!(validate_probe(ProbeKind::AptInRelease, body).is_err());
    }

    #[test]
    fn net03_content_range_len_parses() {
        assert_eq!(content_range_len("bytes 0-1023/2048"), Some((0, 1024)));
        assert!(content_range_len("bytes 0-1023/*").is_some());
        assert!(content_range_len("invalid").is_none());
    }

    // --- NET-02 ---
    #[test]
    fn net02_retries_pinned_after_all_fail() {
        let pinned = Probe { url: "https://mirror/pinned".into(), src: "pinned".into(), ok: false, err: "timeout".into(), ..Default::default() };
        assert!(mirror_needs_retry(&pinned.src, pinned.ok, &pinned.err, false));
        // постоянная ошибка не повторяется даже у закреплённого и заданного вручную зеркала
        for err in ["HTTP 404", "HTTP 403", "вместо файла получен HTML", "no DNS", "TLS error"] {
            assert!(!mirror_needs_retry("pinned", false, err, true), "{err}");
            assert!(!mirror_needs_retry("config", false, err, false), "{err}");
        }
        for err in ["HTTP 429", "HTTP 503", "dropped"] {
            assert!(mirror_needs_retry("config", false, err, false), "{err}");
        }
        let search = Probe { url: "https://mirror/new".into(), src: "search".into(), ok: false, err: "timeout".into(), ..Default::default() };
        assert!(!mirror_needs_retry(&search.src, search.ok, &search.err, false));
        let memory = Probe { url: "https://mirror/old".into(), src: "list".into(), ok: false, err: "timeout".into(), ..Default::default() };
        assert!(mirror_needs_retry(&memory.src, memory.ok, &memory.err, true));
        assert!(schedule_mirror_retry(false, 1), "полный отказ первого прохода всё равно запускает повтор");
        assert!(!schedule_mirror_retry(false, 0), "пустой список повторов не делает лишних запросов");
    }

    struct CacheDirsBackend {
        dirs: Vec<&'static str>,
    }

    #[test]
    fn space01b_uses_existing_secondary_cache_dir() {
        let base = std::env::temp_dir().join(format!("cm-cache-{}", std::process::id()));
        let missing = base.join("dnf");
        let existing = base.join("libdnf5");
        fs::create_dir_all(&existing).unwrap();
        let d1: &'static str = Box::leak(missing.to_string_lossy().into_owned().into_boxed_str());
        let d2: &'static str = Box::leak(existing.to_string_lossy().into_owned().into_boxed_str());
        let b = CacheDirsBackend { dirs: vec![d1, d2] };
        let (label, space_dir) = cache_space_path(&b);
        assert_eq!(space_dir, d2);
        assert_eq!(label, d2);
        let _ = fs::remove_dir_all(&base);
    }

    impl Backend for CacheDirsBackend {
        fn name(&self) -> String {
            "cache-test".into()
        }
        fn mirrors_managed(&self) -> bool {
            false
        }
        fn mirror_note(&self) -> String {
            String::new()
        }
        fn probe_url(&self, m: &str) -> String {
            m.into()
        }
        fn valid_mirror(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn default_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn pinned(&self) -> Vec<String> {
            vec![]
        }
        fn list_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn discover(&self, _: usize, _: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
            Ok((vec![], None))
        }
        fn apply_mirrors(&self, _: &[String], _: Option<&[String]>) -> Result<bool, String> {
            Ok(false)
        }
        fn remove_mirrors(&self) -> Result<(), String> {
            Ok(())
        }
        fn refresh(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn updates(&self) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn prefetch(&self, _: &[String], _: bool) -> Result<(), String> {
            Ok(())
        }
        fn upgrade(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn clean(&self) -> Result<(), String> {
            Ok(())
        }
        fn orphans(&self) -> Vec<String> {
            vec![]
        }
        fn pending_configs(&self) -> Vec<String> {
            vec![]
        }
        fn merge(&self) -> Result<(), String> {
            Ok(())
        }
        fn cache_dirs(&self) -> Vec<&'static str> {
            self.dirs.clone()
        }
        fn db_path(&self) -> &'static str {
            ""
        }
        fn history(&self, _: usize) -> Vec<String> {
            vec![]
        }
    }

    struct UpdatesFail;

    impl Backend for UpdatesFail {
        fn name(&self) -> String {
            "fail".into()
        }
        fn mirrors_managed(&self) -> bool {
            false
        }
        fn mirror_note(&self) -> String {
            String::new()
        }
        fn probe_url(&self, m: &str) -> String {
            m.into()
        }
        fn valid_mirror(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn default_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn pinned(&self) -> Vec<String> {
            vec![]
        }
        fn list_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn discover(&self, _: usize, _: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
            Ok((vec![], None))
        }
        fn apply_mirrors(&self, _: &[String], _: Option<&[String]>) -> Result<bool, String> {
            Ok(false)
        }
        fn remove_mirrors(&self) -> Result<(), String> {
            Ok(())
        }
        fn refresh(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn updates(&self) -> Result<Vec<String>, String> {
            Err("пакеты: тестовая ошибка".into())
        }
        fn prefetch(&self, _: &[String], _: bool) -> Result<(), String> {
            Ok(())
        }
        fn upgrade(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn clean(&self) -> Result<(), String> {
            Ok(())
        }
        fn orphans(&self) -> Vec<String> {
            vec![]
        }
        fn pending_configs(&self) -> Vec<String> {
            vec![]
        }
        fn merge(&self) -> Result<(), String> {
            Ok(())
        }
        fn cache_dirs(&self) -> Vec<&'static str> {
            vec![]
        }
        fn db_path(&self) -> &'static str {
            ""
        }
        fn history(&self, _: usize) -> Vec<String> {
            vec![]
        }
    }

    // --- CM-01 ---
    #[test]
    fn upd01_gather_marks_update_list_failure() {
        let st = gather(&UpdatesFail, &Config::defaults(vec![]), &|_| {}, true, None);
        assert!(matches!(st.package_check_failure, Some(PackageCheckFailure::UpdateList)));
        assert!(!st.error.is_empty());
    }

    #[test]
    fn space01c_statvfs_failure_surfaces_error() {
        let err = free_space("/nonexistent-cm-statvfs-path").unwrap_err();
        assert!(!err.to_string().is_empty());
    }

    #[test]
    fn net04_pending_fingerprint_on_apply_error() {
        let base = std::env::temp_dir().join(format!("cm-mirror-state-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let _iso = crate::common::contract_fixtures::isolation_lock();
        unsafe {
            std::env::set_var("CM_STATE_DIR", base.to_str().unwrap());
        }
        let st = MirrorState {
            best: vec!["https://mirror.example/$repo/os/$arch".into()],
            pending_apply: false,
            pending_fingerprint: String::new(),
            ..Default::default()
        };
        struct FailApply;
        impl Backend for FailApply {
            fn name(&self) -> String {
                "fail".into()
            }
            fn mirrors_managed(&self) -> bool {
                true
            }
            fn mirror_note(&self) -> String {
                String::new()
            }
            fn probe_url(&self, m: &str) -> String {
                m.into()
            }
            fn valid_mirror(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn default_mirrors(&self) -> Vec<String> {
                vec![]
            }
            fn pinned(&self) -> Vec<String> {
                vec![]
            }
            fn list_mirrors(&self) -> Vec<String> {
                vec![]
            }
            fn discover(&self, _: usize, _: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
                Err("no".into())
            }
            fn apply_mirrors(&self, _: &[String], _: Option<&[String]>) -> Result<bool, String> {
                Err("disk full".into())
            }
            fn remove_mirrors(&self) -> Result<(), String> {
                Ok(())
            }
            fn refresh(&self, _: bool) -> Result<(), String> {
                Ok(())
            }
            fn updates(&self) -> Result<Vec<String>, String> {
                Ok(vec![])
            }
            fn prefetch(&self, _: &[String], _: bool) -> Result<(), String> {
                Ok(())
            }
            fn upgrade(&self, _: bool) -> Result<(), String> {
                Ok(())
            }
            fn clean(&self) -> Result<(), String> {
                Ok(())
            }
            fn orphans(&self) -> Vec<String> {
                vec![]
            }
            fn pending_configs(&self) -> Vec<String> {
                vec![]
            }
            fn merge(&self) -> Result<(), String> {
                Ok(())
            }
            fn cache_dirs(&self) -> Vec<&'static str> {
                vec![]
            }
            fn db_path(&self) -> &'static str {
                ""
            }
            fn history(&self, _: usize) -> Vec<String> {
                vec![]
            }
        }
        let c = Config::defaults(vec![]);
        let _ = apply_mirrors(&FailApply, &c, &st, None, &|_| {});
        let saved: MirrorState = load_json("mirrors.json");
        assert!(saved.pending_apply);
        assert!(!saved.pending_fingerprint.is_empty());
        unsafe {
            std::env::remove_var("CM_STATE_DIR");
        }
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn space01a_unknown_size_message_is_distinct() {
        assert!(required_free(None, 5) >= 5 << 30);
        assert!(required_free(Some(0), 5) >= 5 << 30);
        let unknown = low_space_message(None, 5, "кэш /var", 1 << 30);
        let zero = low_space_message(Some(0), 5, "кэш /var", 1 << 30);
        assert!(unknown.contains("неизвестном объёме"));
        assert!(zero.contains("нужно"));
        assert_ne!(unknown, zero);
    }

    struct PrefetchSpy {
        called: std::sync::atomic::AtomicBool,
    }

    impl Backend for PrefetchSpy {
        fn name(&self) -> String {
            "prefetch".into()
        }
        fn mirrors_managed(&self) -> bool {
            false
        }
        fn mirror_note(&self) -> String {
            String::new()
        }
        fn probe_url(&self, m: &str) -> String {
            m.into()
        }
        fn valid_mirror(&self, _: &str) -> Result<(), String> {
            Ok(())
        }
        fn default_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn pinned(&self) -> Vec<String> {
            vec![]
        }
        fn list_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn discover(&self, _: usize, _: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
            Ok((vec![], None))
        }
        fn apply_mirrors(&self, _: &[String], _: Option<&[String]>) -> Result<bool, String> {
            Ok(false)
        }
        fn remove_mirrors(&self) -> Result<(), String> {
            Ok(())
        }
        fn refresh(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn updates(&self) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn prefetch(&self, _: &[String], _: bool) -> Result<(), String> {
            self.called.store(true, std::sync::atomic::Ordering::SeqCst);
            Ok(())
        }
        fn upgrade(&self, _: bool) -> Result<(), String> {
            Ok(())
        }
        fn clean(&self) -> Result<(), String> {
            Ok(())
        }
        fn orphans(&self) -> Vec<String> {
            vec![]
        }
        fn pending_configs(&self) -> Vec<String> {
            vec![]
        }
        fn merge(&self) -> Result<(), String> {
            Ok(())
        }
        fn cache_dirs(&self) -> Vec<&'static str> {
            vec![]
        }
        fn db_path(&self) -> &'static str {
            ""
        }
        fn history(&self, _: usize) -> Vec<String> {
            vec![]
        }
    }

    #[test]
    fn space01a_unknown_size_skips_prefetch() {
        let base = std::env::temp_dir().join(format!("cm-space-{}", std::process::id()));
        fs::create_dir_all(&base).unwrap();
        let _iso = crate::common::contract_fixtures::isolation_lock();
        unsafe {
            std::env::set_var("CM_STATE_DIR", base.to_str().unwrap());
        }
        let spy = PrefetchSpy { called: std::sync::atomic::AtomicBool::new(false) };
        let mut st = UpdState { list: vec!["nano 1 -> 2".into()], download_size: None, ..Default::default() };
        download(&spy, &Config::defaults(vec![]), &mut st, &|_| {}, true, false);
        assert!(!spy.called.load(std::sync::atomic::Ordering::SeqCst));
        assert!(st.skipped.contains("неизвестен"), "{}", st.skipped);
        st.download_size = Some(0);
        st.skipped.clear();
        download(&spy, &Config::defaults(vec![]), &mut st, &|_| {}, true, false);
        assert!(spy.called.load(std::sync::atomic::Ordering::SeqCst));
        unsafe {
            std::env::remove_var("CM_STATE_DIR");
        }
        let _ = fs::remove_dir_all(&base);
    }
}
