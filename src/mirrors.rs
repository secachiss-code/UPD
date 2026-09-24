//! Замер и подбор зеркал, реакция на смену сети, проверка и предзагрузка обновлений.

use crate::backend::Backend;
use crate::common::*;
use crate::extras;
use std::io::Read;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

const PROBE_BYTES: u64 = 3 << 20; // 3 МБ: на маленьком файле не видно, как прокси/провайдер душит скорость
const PROBE_MIN: u64 = 256 << 10; // меньше этого за таймаут — зеркало нерабочее

pub fn agent(timeout: u64) -> ureq::Agent {
    ureq::AgentBuilder::new()
        .timeout_connect(Duration::from_secs(5))
        .timeout(Duration::from_secs(timeout))
        .user_agent("upd (mirror probe)")
        .build()
}

fn classify(msg: &str) -> String {
    if std::env::var_os("UPD_DEBUG").is_some() {
        eprintln!("[debug] {msg}");
    }
    let m = msg.to_lowercase();
    if m.contains("dns") || m.contains("resolve") || m.contains("lookup") {
        "нет DNS"
    } else if m.contains("timed out") || m.contains("timeout") || m.contains("deadline") {
        "таймаут"
    } else if m.contains("reset") || m.contains("eof") || m.contains("end of file") || m.contains("broken pipe") || m.contains("aborted") {
        // соединение оборвали посередине — чаще всего прокси/VPN под нагрузкой, повтор обычно помогает
        "обрыв"
    } else if m.contains("certificate") || m.contains("tls") || m.contains("handshake") {
        "ошибка TLS"
    } else {
        "нет связи"
    }
    .into()
}

/// Замер скорости: первые 3 МБ. Медленное, но живое зеркало (успело отдать ≥256 КБ) — считаем по факту.
fn probe_speed(url: &str, timeout: u64) -> (bool, f64, String) {
    let start = Instant::now();
    let resp = match agent(timeout).get(url).set("Range", &format!("bytes=0-{}", PROBE_BYTES - 1)).call() {
        Ok(r) => r,
        Err(ureq::Error::Status(code, _)) => return (false, 0.0, format!("HTTP {code}")),
        Err(e) => return (false, 0.0, classify(&e.to_string())),
    };
    let mut rd = resp.into_reader().take(PROBE_BYTES);
    let mut buf = vec![0u8; 64 << 10];
    let mut n: u64 = 0;
    loop {
        match rd.read(&mut buf) {
            Ok(0) => break,
            Ok(k) => n += k as u64,
            Err(e) => {
                if n >= PROBE_MIN && matches!(e.kind(), std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) {
                    break;
                }
                return (false, 0.0, classify(&e.to_string()));
            }
        }
    }
    let el = start.elapsed().as_secs_f64();
    if n == 0 || el <= 0.0 {
        return (false, 0.0, "пусто".into());
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
}

struct Done {
    idx: usize,
    ok: bool,
    speed: f64,
    err: String,
    fresh_body: Option<String>,
}

/// Параллельный замер: скорость + (для рабочих) дата синхронизации.
fn measure(jobs: Vec<Job>, parallel: usize, timeout: u64, on_done: &mut dyn FnMut(&Done)) -> Vec<Done> {
    let n = jobs.len();
    let queue = Arc::new(Mutex::new(jobs));
    let (tx, rx) = mpsc::channel();
    for _ in 0..parallel.max(1).min(n.max(1)) {
        let (q, tx) = (queue.clone(), tx.clone());
        std::thread::spawn(move || loop {
            let Some(j) = q.lock().unwrap().pop() else { break };
            let (ok, speed, err) = probe_speed(&j.probe, timeout);
            let fresh_body = if ok { j.fresh.as_deref().and_then(fetch_head) } else { None };
            let _ = tx.send(Done { idx: j.idx, ok, speed, err, fresh_body });
        });
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

pub fn candidates(b: &dyn Backend, c: &Config, extra: Option<&[String]>) -> Vec<Cand> {
    let mut r: Vec<Cand> = vec![];
    let mut add = |list: &[String], src: &'static str| {
        for u in list {
            if !r.iter().any(|x| x.url.trim_end_matches('/') == u.trim_end_matches('/')) {
                r.push(Cand { url: u.clone(), src });
            }
        }
    };
    add(&c.mirrors, "конфиг");
    add(&b.pinned(), "закреп");
    match extra {
        Some(e) => add(e, "поиск"),
        None => {
            let l = b.list_mirrors();
            add(&l[..l.len().min(c.extra_from_list)], "список");
        }
    }
    r
}

/// Замер кандидатов и закрепление лучших. Возвращает число рабочих.
pub fn check_mirrors(b: &dyn Backend, c: &Config, log: Log, extra: Option<&[String]>, fallback: Option<&[String]>) -> usize {
    let net = fingerprint();
    let cands = candidates(b, c, extra);
    let parallel = if net.vpn { c.parallel_vpn } else { c.parallel };
    log(&format!(
        "замеряю зеркала: {} шт. (по {} МБ, таймаут {} с, параллельно {}{})",
        cands.len(),
        PROBE_BYTES >> 20,
        c.timeout,
        parallel,
        if net.vpn { " — через VPN" } else { "" }
    ));
    let mk = |i: usize, cd: &Cand| Job { idx: i, probe: b.probe_url(&cd.url), fresh: b.fresh_url(&cd.url) };
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
    let done = measure(jobs, parallel, c.timeout, &mut |d| {
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
        .filter(|(_, p)| {
            !p.ok && (p.err == "обрыв" || (p.src != "поиск" && p.src != "список") || mem0.stats.get(&p.url).map(|s| s.ok > 0).unwrap_or(false))
        })
        .map(|(i, _)| mk(i, &cands[i]))
        .collect();
    if !retry.is_empty() && res.iter().any(|p| p.ok) {
        log(&format!("  повторный замер упавших: {}", retry.len()));
        for d in measure(retry, 1, c.timeout, &mut |d| {
            if d.ok {
                log(&format!("  ✓ со второй попытки: {}", cands[d.idx].url));
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
                    p.err = format!("отстаёт {lag:.0} ч");
                    log(&format!("  ⌛ {} отстаёт на {lag:.0} ч — не беру", host_of(&p.url)));
                }
            }
        }
    }

    let any_ok = res.iter().any(|p| p.ok);
    let mut st = load_mirror_state();
    st.checked = now();
    st.hints.clear();
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
        st.fingerprint = net.id.clone();
        st.label = net.label.clone();
        let cutoff = now() - 30 * 86400;
        st.networks.retain(|_, v| v.checked > cutoff);
        // Подсказка про прокси: вручную заданное зеркало не отвечает через VPN, а другие работают.
        if net.vpn {
            for p in res.iter().filter(|p| !p.ok && p.src == "конфиг" && !p.err.starts_with("отстаёт")) {
                st.hints.push(format!(
                    "{} не отвечает через VPN. Если это зеркало в твоей стране — добавь {} в правила DIRECT своего VPN-клиента (FlClash и т.п.)",
                    host_of(&p.url),
                    host_of(&p.url)
                ));
            }
        }
    } else {
        log("ни одно зеркало не ответило — оставляю прежние");
    }
    st.results = res;
    let _ = save_json("mirrors.json", &st);
    for h in &st.hints {
        log(&format!("  💡 {h}"));
    }
    apply_mirrors(b, c, &st, fallback, log);
    st.best.len() * any_ok as usize
}

pub fn load_mirror_state() -> MirrorState {
    load_json("mirrors.json")
}

pub fn desired_block(c: &Config, st: &MirrorState) -> Vec<String> {
    let src = if st.best.is_empty() { &c.mirrors } else { &st.best };
    src.iter().take(c.keep).cloned().collect()
}

pub fn apply_mirrors(b: &dyn Backend, c: &Config, st: &MirrorState, fallback: Option<&[String]>, log: Log) -> bool {
    if !b.mirrors_managed() {
        return false;
    }
    let block = desired_block(c, st);
    if block.is_empty() {
        log("нечего закреплять: нет рабочих зеркал");
        return false;
    }
    match b.apply_mirrors(&block, fallback) {
        Err(e) => {
            log(&format!("не удалось записать зеркала: {e}"));
            false
        }
        Ok(true) => {
            log(&format!("закреплено зеркал: {}, первое: {}", block.len(), block[0]));
            true
        }
        Ok(false) => {
            log("зеркала уже на месте");
            false
        }
    }
}

/// Полный подбор: ищем зеркала рядом с текущим местом и замеряем их.
pub fn rescan(b: &dyn Backend, c: &Config, log: Log) -> usize {
    if !b.mirrors_managed() {
        log(&b.mirror_note());
        return 0;
    }
    log("ищу зеркала рядом с текущим местоположением...");
    match b.discover(c.rescan_count, log) {
        Ok((cands, fallback)) if cands.len() >= 3 => check_mirrors(b, c, log, Some(&cands), fallback.as_deref()),
        Ok(_) => {
            log("автопоиск нашёл слишком мало зеркал — замеряю известные");
            check_mirrors(b, c, log, None, None)
        }
        Err(e) => {
            log(&format!("автопоиск не удался ({e}) — замеряю известные"));
            check_mirrors(b, c, log, None, None)
        }
    }
}

/// Реакция на смену сети. true — сеть сменилась.
pub fn handle_network(b: &dyn Backend, c: &Config, log: Log, force: bool) -> bool {
    if !b.mirrors_managed() {
        return false;
    }
    let net = fingerprint();
    if !net.online {
        log("сети нет — жду");
        return false;
    }
    let st = load_mirror_state();
    if st.fingerprint == net.id && !force {
        log(&format!("сеть та же ({})", net.label));
        return false;
    }
    log(&format!("сеть: {} (было: {})", net.label, if st.label.is_empty() { "—" } else { &st.label }));
    let fresh = st
        .networks
        .get(&net.id)
        .filter(|m| !m.best.is_empty() && now() - m.checked < c.network_memory_days * 86400)
        .cloned();
    match fresh {
        Some(m) if !force => {
            log("сеть знакомая — сразу ставлю её зеркала, потом перепроверяю");
            let mut tmp = st.clone();
            tmp.best = m.best;
            apply_mirrors(b, c, &tmp, None, log);
            check_mirrors(b, c, log, None, None);
        }
        _ => {
            log("новая сеть — полный подбор зеркал");
            rescan(b, c, log);
        }
    }
    let mut st = load_mirror_state();
    if st.fingerprint == net.id && !st.best.is_empty() {
        st.event = format!("Сеть сменилась ({}). Лучшее зеркало: {}", net.label, host_of(&st.best[0]));
        st.event_time = now();
        let _ = save_json("mirrors.json", &st);
    }
    true
}

// ======================= обновления =======================

pub fn pkg_names(list: &[String]) -> Vec<String> {
    list.iter().filter_map(|l| l.split_whitespace().next().map(String::from)).collect()
}

/// Почему фоновую загрузку сейчас делать не стоит (ноутбук на батарее, лимитная сеть).
pub fn background_block(c: &Config) -> Option<String> {
    let net = fingerprint();
    if !c.prefetch_on_metered && metered(&net.dev) {
        return Some("лимитная сеть — фоновая загрузка отложена".into());
    }
    if !c.prefetch_on_battery && on_battery() {
        return Some("работа от батареи — фоновая загрузка отложена".into());
    }
    None
}

/// Проверка: базы, список пакетов, Flatpak, прошивки, новости, размер загрузки и место на диске.
pub fn gather(b: &dyn Backend, c: &Config, log: Log, quiet: bool) -> UpdState {
    let mut st = UpdState { checked: now(), ..Default::default() };
    let mut res = b.refresh(quiet);
    if let Err(e) = &res {
        log(&format!("не удалось обновить базы пакетов: {e}"));
        if b.mirrors_managed() {
            rescan(b, c, log);
            res = b.refresh(quiet);
        }
    }
    if res.is_err() {
        st.error = "не удалось обновить базы пакетов".into();
        let _ = save_json("updates.json", &st);
        return st;
    }
    match b.updates() {
        Ok(l) => st.list = l,
        Err(e) => st.error = e,
    }
    if c.flatpak {
        st.flatpak = extras::flatpak_updates();
    }
    if c.firmware && extras::has_fwupd() {
        extras::firmware_refresh();
        st.firmware = extras::firmware_updates();
    }
    if c.news && b.arch_news() {
        match extras::arch_news(b.last_upgrade()) {
            Ok(n) => st.news = n,
            Err(e) => log(&format!("новости Arch не загрузились: {e}")),
        }
    }
    if !st.list.is_empty() {
        st.download_size = b.download_size(&pkg_names(&st.list)).unwrap_or(0);
        let dir = b.cache_dirs().first().copied().unwrap_or("/");
        let free = free_space(dir);
        let need = st.download_size + c.min_free_gb * (1 << 30);
        if free > 0 && free < need {
            st.error = format!("мало места: нужно {} (с запасом {} ГБ), свободно {}", fmt_bytes(need), c.min_free_gb, fmt_bytes(free));
        }
    }
    log(&format!(
        "обновлений: пакетов {}{}{}{}",
        st.list.len(),
        if st.download_size > 0 { format!(" ({})", fmt_bytes(st.download_size)) } else { String::new() },
        if st.flatpak.is_empty() { String::new() } else { format!(", Flatpak {}", st.flatpak.len()) },
        if st.firmware.is_empty() { String::new() } else { format!(", прошивок {}", st.firmware.len()) },
    ));
    if !st.news.is_empty() {
        log(&format!("новостей Arch с прошлого обновления: {}", st.news.len()));
    }
    if !st.error.is_empty() {
        log(&st.error);
    }
    let _ = save_json("updates.json", &st);
    st
}

/// Предзагрузка с повторами; на второй неудаче — переподбор зеркал.
pub fn download(b: &dyn Backend, c: &Config, st: &mut UpdState, log: Log, quiet: bool) {
    if st.error.starts_with("мало места") {
        log("загрузку пропускаю: мало места");
        return;
    }
    if !st.list.is_empty() {
        let names = pkg_names(&st.list);
        for attempt in 1..=c.retries {
            log(&format!("скачиваю заранее (попытка {attempt} из {})...", c.retries));
            match b.prefetch(&names, quiet) {
                Ok(()) => {
                    st.downloaded = true;
                    log("пакеты скачаны");
                    break;
                }
                Err(e) => log(&format!("загрузка оборвалась: {e}")),
            }
            if attempt == 2 && b.mirrors_managed() {
                log("похоже, зеркала плохие — подбираю заново");
                rescan(b, c, log);
            }
        }
        if !st.downloaded {
            st.error = "не удалось скачать обновления".into();
        }
    }
    if !st.flatpak.is_empty() {
        log("Flatpak: скачиваю заранее...");
        if let Err(e) = extras::flatpak_prefetch(quiet) {
            log(&format!("Flatpak: {e}"));
        }
    }
    let _ = save_json("updates.json", st);
}
