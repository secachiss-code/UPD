mod backend;
mod common;
mod extras;
mod mirrors;
mod tui;

use backend::Backend;
use common::*;
use mirrors::*;
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;

const VERSION: &str = env!("CARGO_PKG_VERSION");

const USAGE: &str = "upd — обновление Linux с автоподбором зеркал

  upd                     интерфейс (TUI)
  upd update              обновить всё: зеркала → проверка → загрузка → снапшот → установка → службы
  upd check [--no-download]   проверить обновления и скачать заранее
  upd list                что доступно (по последней проверке)
  upd status              состояние системы
  upd mirrors [status|check|rescan|apply|add URL|del URL]
  upd news [--all]        новости Arch с прошлого обновления (--all — последние)
  upd snapshots           снапшоты и как откатиться
  upd restart             перезапустить службы со старыми библиотеками
  upd clean               очистить кэш и ненужные пакеты
  upd merge               слить новые файлы настроек (.pacnew)
  upd install | uninstall установить в систему / удалить
  upd reconcile           сверить список обновлений с установленным (без сети; вызывается хуком pacman/apt)
  upd version

Служебные (их запускают таймеры): upd auto, upd net, upd notify
";

fn stdlog(s: &str) {
    println!("{s}");
}

fn main() {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pause = args.iter().any(|a| a == "--pause");
    let no_download = args.iter().any(|a| a == "--no-download");
    let all = args.iter().any(|a| a == "--all");
    args.retain(|a| !a.starts_with("--"));
    let cmd = args.first().cloned().unwrap_or_else(|| "tui".into());
    let pos: Vec<String> = args.iter().skip(1).cloned().collect();

    match cmd.as_str() {
        "-h" | "help" => return print!("{USAGE}"),
        "version" => return println!("upd {VERSION}"),
        _ => {}
    }
    let b = match backend::detect() {
        Ok(b) => b,
        Err(e) => {
            eprintln!("upd: {e}");
            std::process::exit(1)
        }
    };
    let user_cmd = matches!(cmd.as_str(), "status" | "list" | "notify" | "news") || (cmd == "mirrors" && pos.first().map(|s| s == "status").unwrap_or(true));
    if !user_cmd {
        become_root();
    }
    let c = Config::load(b.default_mirrors());
    if is_root() && !Path::new(&conf_path()).exists() {
        let _ = c.save();
    }
    let b = b.as_ref();
    let code = match cmd.as_str() {
        "tui" => tui::run(b),
        "update" => cmd_update(b, &c),
        "check" => with_lock(true, || {
            let mut st = gather(b, &c, &stdlog, false);
            if !no_download && st.error.is_empty() {
                download(b, &c, &mut st, &stdlog, false);
            }
            (!st.error.is_empty()) as i32
        }),
        "list" => cmd_list(),
        "status" => {
            print_status(b);
            0
        }
        "mirrors" => cmd_mirrors(b, &c, &pos),
        "snapshots" => {
            for l in extras::snap_list(20) {
                println!("{l}");
            }
            println!();
            for l in extras::rollback_hint() {
                println!("{l}");
            }
            0
        }
        "restart" => cmd_restart(),
        "reconcile" => cmd_reconcile(b),
        "news" => {
            if !b.arch_news() {
                println!("новости есть только для Arch и производных");
                return;
            }
            let since = if all { 0 } else { b.last_upgrade() };
            match extras::arch_news(since) {
                Ok(n) if n.is_empty() => println!("новых новостей с прошлого обновления ({}) нет", fmt_time(since)),
                Ok(n) => {
                    for x in n.iter().take(if all { 10 } else { usize::MAX }) {
                        println!("{}  {}\n      {}", fmt_time(x.date), x.title, x.link);
                    }
                }
                Err(e) => println!("не удалось загрузить: {e}"),
            }
            0
        }
        "clean" => err_code(b.clean()),
        "merge" => err_code(b.merge()),
        "auto" => with_lock(false, || cmd_auto(b, &c)),
        "net" => with_lock(false, || {
            if let Some(why) = background_block(&c).filter(|w| w.starts_with("лимитная")) {
                println!("{why}: замер зеркал пропущен");
                return 0;
            }
            handle_network(b, &c, &stdlog, false);
            0
        }),
        "notify" => {
            cmd_notify();
            0
        }
        "install" => cmd_install(b, &c),
        "uninstall" => cmd_uninstall(b),
        _ => {
            eprint!("неизвестная команда: {cmd}\n\n{USAGE}");
            2
        }
    };
    if pause {
        print!("\n\x1b[2mEnter — вернуться в меню\x1b[0m ");
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

fn err_code(r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("ошибка: {e}");
            1
        }
    }
}

/// Перезапуск через sudo/doas/run0/pkexec.
fn become_root() {
    if is_root() || test_mode() {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|_| "upd".into());
    for s in ["sudo", "doas", "run0", "pkexec"] {
        if have(s) {
            let err = std::process::Command::new(s).arg(&exe).args(std::env::args().skip(1)).exec();
            eprintln!("upd: {s}: {err}");
        }
    }
    eprintln!("upd: нужны права root (sudo/doas не найдены)");
    std::process::exit(1);
}

fn with_lock(block: bool, f: impl FnOnce() -> i32) -> i32 {
    let l = match lock(false) {
        Ok(l) => l,
        Err(_) if block => {
            println!("идёт фоновая задача upd, жду завершения...");
            match lock(true) {
                Ok(l) => l,
                Err(e) => {
                    println!("upd: {e}");
                    return 1;
                }
            }
        }
        Err(_) => {
            println!("upd: занято другой задачей upd");
            return 0;
        }
    };
    let r = f();
    drop(l);
    r
}

fn step(n: u32, title: &str) {
    println!("\n\x1b[1;36m[{n}/6] {title}\x1b[0m");
}

fn cmd_auto(b: &dyn Backend, c: &Config) -> i32 {
    let block = background_block(c);
    let metered = block.as_deref().map(|w| w.starts_with("лимитная")).unwrap_or(false);
    if metered {
        println!("{}: пропускаю всё", block.as_deref().unwrap_or(""));
        let mut st: UpdState = load_json("updates.json");
        st.skipped = block.unwrap_or_default();
        let _ = save_json("updates.json", &st);
        return 0;
    }
    let changed = handle_network(b, c, &stdlog, false);
    let ms = load_mirror_state();
    if !changed && b.mirrors_managed() && now() - ms.checked > c.mirror_max_age_h * 3600 {
        check_mirrors(b, c, &stdlog, None, None);
    }
    let mut st = gather(b, c, &stdlog, true);
    match (&block, c.prefetch && st.error.is_empty()) {
        (Some(why), true) => {
            println!("{why}");
            st.skipped = why.clone();
            let _ = save_json("updates.json", &st);
        }
        (None, true) => download(b, c, &mut st, &stdlog, true),
        _ => {}
    }
    0
}

fn cmd_update(b: &dyn Backend, c: &Config) -> i32 {
    with_lock(true, || {
        let user = invoking_user();

        step(1, "Зеркала");
        if b.mirrors_managed() {
            if !handle_network(b, c, &stdlog, false) {
                let st = load_mirror_state();
                if now() - st.checked > c.mirror_max_age_h * 3600 {
                    check_mirrors(b, c, &stdlog, None, None);
                } else {
                    apply_mirrors(b, c, &st, None, &stdlog);
                    println!("последний замер: {}", fmt_ago(st.checked));
                }
            }
        } else {
            println!("{}", b.mirror_note());
        }

        step(2, "Проверка");
        let mut st = gather(b, c, &stdlog, false);
        if st.error == "не удалось обновить базы пакетов" {
            println!("\x1b[31mБазы пакетов не обновились — проверь сеть и попробуй ещё раз.\x1b[0m");
            return 1;
        }
        let aur = match (&user, c.aur) {
            (Some(u), true) => extras::aur_updates(u),
            _ => vec![],
        };
        if !aur.is_empty() {
            println!("AUR: {} обновлений", aur.len());
        }
        if st.list.is_empty() && st.flatpak.is_empty() && aur.is_empty() && st.firmware.is_empty() {
            println!("\x1b[32mСистема уже обновлена.\x1b[0m");
            return 0;
        }
        if !st.news.is_empty() {
            println!("\n\x1b[1;33mНовости Arch с прошлого обновления — прочитай, там бывают ручные шаги:\x1b[0m");
            for n in &st.news {
                println!("  {}  {}\n      {}", fmt_time(n.date), n.title, n.link);
            }
            if !confirm("\nПрочитал(а), продолжить?", true) {
                return 1;
            }
        }
        if st.error.starts_with("мало места") && !confirm("Мало места на диске. Всё равно продолжить?", false) {
            return 1;
        }

        step(3, "Загрузка");
        download(b, c, &mut st, &stdlog, false);
        if !st.list.is_empty() && !st.downloaded && !confirm("Не всё скачалось. Всё равно запустить установку?", false) {
            return 1;
        }

        step(4, "Снапшот");
        let mut pre = None;
        if !c.snapshot {
            println!("отключено в настройках (snapshot = 0)");
        } else if b.auto_snapshots() {
            println!("снапшоты «до» и «после» сделает snap-pac автоматически");
        } else {
            match extras::snap_pre() {
                Ok(n) => {
                    println!("снапшот создан{}", n.as_deref().map(|x| format!(": #{x}")).unwrap_or_default());
                    pre = n;
                }
                Err(e) => {
                    println!("\x1b[33m{e}\x1b[0m");
                    if !confirm("Продолжить без снапшота?", true) {
                        return 1;
                    }
                }
            }
        }

        step(5, "Установка");
        let aur_by_installer = c.aur && b.upgrade_handles_aur();
        let mut result = if st.list.is_empty() && !aur_by_installer { Ok(()) } else { b.upgrade(aur_by_installer) };
        if result.is_ok() && !aur.is_empty() && !aur_by_installer {
            if let Some(u) = &user {
                println!("\n→ AUR");
                result = extras::aur_upgrade(u);
            }
        }
        if !st.flatpak.is_empty() {
            println!("\n→ Flatpak");
            if let Err(e) = extras::flatpak_upgrade() {
                println!("Flatpak: {e}");
            }
        }
        if let Some(p) = &pre {
            extras::snap_post(p);
        }
        match &result {
            Ok(()) => {
                let _ = save_json("updates.json", &UpdState { checked: now(), ..Default::default() });
                println!("\n\x1b[32m✓ Обновление завершено\x1b[0m");
            }
            Err(e) => println!("\n\x1b[31m✗ Обновление завершилось с ошибкой: {e}\x1b[0m"),
        }

        step(6, "После обновления");
        if !st.firmware.is_empty() {
            println!("Доступны прошивки:");
            for f in &st.firmware {
                println!("  {f}");
            }
            if confirm("Установить прошивки? (может понадобиться перезагрузка)", false) {
                let _ = extras::firmware_upgrade();
            }
        }
        let r = needs_restart();
        extras::print_restart(&r);
        if !r.services.is_empty() && confirm("Перезапустить эти службы сейчас?", true) {
            extras::restart_services(&r.services);
        }
        if reboot_needed() || !r.critical.is_empty() {
            println!("\x1b[33m⟳ Нужна перезагрузка\x1b[0m");
        }
        let p = b.pending_configs();
        if !p.is_empty() {
            println!("\x1b[33m⚙ Новых файлов настроек: {} — upd merge\x1b[0m", p.len());
        }
        if result.is_ok() && (pre.is_some() || b.auto_snapshots()) {
            println!("\x1b[2mЕсли что-то сломалось: upd snapshots — как откатиться\x1b[0m");
        }
        result.is_err() as i32
    })
}

fn cmd_list() -> i32 {
    let st: UpdState = load_json("updates.json");
    println!("проверено: {}, пакетов: {}", fmt_time(st.checked), st.list.len());
    for l in &st.list {
        println!("  {l}");
    }
    for (title, list) in [("Flatpak", &st.flatpak), ("Прошивки", &st.firmware)] {
        if !list.is_empty() {
            println!("{title}:");
            for l in list {
                println!("  {l}");
            }
        }
    }
    for n in &st.news {
        println!("новость {}: {} — {}", fmt_time(n.date), n.title, n.link);
    }
    0
}

/// Систему обновили в обход upd (garuda-update, pacman, pamac…): убрать из списка то, что уже установлено.
/// Без сети: pacman сравнивает с уже скачанной временной базой, apt — с локальными списками.
fn cmd_reconcile(b: &dyn Backend) -> i32 {
    let mut st: UpdState = load_json("updates.json");
    if st.checked == 0 {
        return 0;
    }
    let Ok(list) = b.updates() else { return 0 };
    if list != st.list {
        println!("upd: доступных обновлений было {}, стало {}", st.list.len(), list.len());
        if list.is_empty() {
            st.downloaded = false;
            st.download_size = 0;
        }
        st.list = list;
        st.news.clear(); // новости отбираются по дате последнего обновления — после него они прочитаны
        let _ = save_json("updates.json", &st);
    }
    0
}

fn cmd_restart() -> i32 {
    let r = needs_restart();
    extras::print_restart(&r);
    if !r.services.is_empty() && confirm("Перезапустить службы?", true) {
        extras::restart_services(&r.services);
    }
    0
}

fn cmd_mirrors(b: &dyn Backend, c: &Config, pos: &[String]) -> i32 {
    let sub = pos.first().map(String::as_str).unwrap_or("status");
    match sub {
        "status" => {
            let st = load_mirror_state();
            println!("сеть: {}\nпоследний замер: {}", fingerprint().label, fmt_time(st.checked));
            if !b.mirrors_managed() {
                println!("{}", b.mirror_note());
                return 0;
            }
            for m in b.pinned() {
                println!("  ● {m}");
            }
            for p in &st.results {
                let lag = p.lag_h.map(|l| format!(" · отставание {l} ч")).unwrap_or_default();
                println!("  {:<12} {:<7} {}{lag}", fmt_speed(p), p.src, p.url);
            }
            for h in &st.hints {
                println!("💡 {h}");
            }
            0
        }
        "check" => with_lock(true, || {
            check_mirrors(b, c, &stdlog, None, None);
            0
        }),
        "rescan" => with_lock(true, || {
            rescan(b, c, &stdlog);
            0
        }),
        // вызывается слежением за файлом: без сети и без блокировки, мгновенно
        "apply" => {
            apply_mirrors(b, c, &load_mirror_state(), None, &stdlog);
            0
        }
        "add" | "del" => {
            let Some(u) = pos.get(1) else {
                println!("укажи URL");
                return 2;
            };
            let mut c = c.clone();
            if sub == "add" {
                if let Err(e) = b.valid_mirror(u) {
                    println!("{e}");
                    return 2;
                }
                if !contains(&c.mirrors, u) {
                    c.mirrors.push(u.clone());
                }
            } else {
                c.mirrors.retain(|m| m.trim_end_matches('/') != u.trim_end_matches('/'));
            }
            err_code(c.save().map_err(|e| e.to_string()))
        }
        _ => {
            println!("mirrors: status|check|rescan|apply|add URL|del URL");
            2
        }
    }
}

// ---------- статус ----------

#[derive(Clone, Default)]
pub struct Status {
    pub name: String,
    pub upd: UpdState,
    pub mir: MirrorState,
    pub net_label: String,
    pub pinned: Vec<String>,
    pub last_tx: i64,
    pub reboot: bool,
    pub pending: Vec<String>,
    pub orphans: usize,
    pub cache: u64,
    pub failed: usize,
    pub auto_timer: String,
    pub net_timer: String,
    pub managed: bool,
    pub mirror_note: String,
    pub restart: Restart,
    pub battery: bool,
    pub metered: bool,
    pub free: u64,
    pub snapshots: String,
}

pub fn gather_status(b: &dyn Backend) -> Status {
    let net = fingerprint();
    let cache_dirs = b.cache_dirs();
    Status {
        name: b.name(),
        upd: load_json("updates.json"),
        mir: load_mirror_state(),
        net_label: net.label.clone(),
        pinned: b.pinned(),
        last_tx: b.last_upgrade(),
        reboot: reboot_needed(),
        pending: b.pending_configs(),
        orphans: b.orphans().len(),
        cache: dir_size(&cache_dirs),
        failed: failed_units(),
        auto_timer: unit_state("upd-auto.timer"),
        net_timer: unit_state("upd-net.timer"),
        managed: b.mirrors_managed(),
        mirror_note: b.mirror_note(),
        restart: needs_restart(),
        battery: on_battery(),
        metered: metered(&net.dev),
        free: free_space(cache_dirs.first().copied().unwrap_or("/")),
        snapshots: match (b.auto_snapshots(), extras::snap_tool()) {
            (true, _) => "snap-pac (автоматически)".into(),
            (_, extras::SnapTool::Snapper) => "snapper (делает upd)".into(),
            (_, extras::SnapTool::Timeshift) => "timeshift (делает upd)".into(),
            _ => "нет".into(),
        },
    }
}

fn print_status(b: &dyn Backend) {
    let s = gather_status(b);
    let u = &s.upd;
    let mut upd = format!("пакетов {}", u.list.len());
    if u.downloaded && !u.list.is_empty() {
        upd += " (скачаны)";
    }
    if !u.flatpak.is_empty() {
        upd += &format!(", Flatpak {}", u.flatpak.len());
    }
    if !u.firmware.is_empty() {
        upd += &format!(", прошивок {}", u.firmware.len());
    }
    if !u.error.is_empty() {
        upd += &format!(" — {}", u.error);
    }
    if !u.skipped.is_empty() {
        upd += &format!(" — {}", u.skipped);
    }
    println!("Система:      {}", s.name);
    println!("Обновления:   {upd} · проверка {}", fmt_ago(u.checked));
    if !u.news.is_empty() {
        println!("Новости Arch: {} непрочитанных — upd list", u.news.len());
    }
    println!("Последнее:    {}", fmt_time(s.last_tx));
    println!("Перезагрузка: {}", if s.reboot { "НУЖНА" } else { "не нужна" });
    let rs = &s.restart;
    if !rs.services.is_empty() || !rs.critical.is_empty() {
        println!("Перезапуск:   служб {} (upd restart), требуют перезагрузки {}", rs.services.len(), rs.critical.len());
    }
    println!("Сеть:         {}{}{}", s.net_label, if s.metered { " · лимитная" } else { "" }, if s.battery { " · от батареи" } else { "" });
    if s.managed {
        let first = s.pinned.first().map(|p| host_of(p).to_string()).unwrap_or_else(|| "—".into());
        println!("Зеркала:      закреплено {}, первое {} · замер {}", s.pinned.len(), first, fmt_ago(s.mir.checked));
        for h in &s.mir.hints {
            println!("              💡 {h}");
        }
    } else {
        println!("Зеркала:      {}", s.mirror_note);
    }
    println!("Снапшоты:     {}", s.snapshots);
    println!(
        "Обслуживание: новых настроек {} · сирот {} · кэш {} · свободно {} · упавших служб {}",
        s.pending.len(),
        s.orphans,
        fmt_bytes(s.cache),
        fmt_bytes(s.free),
        s.failed
    );
    println!("Автоматика:   обновления {}, слежение за сетью {}", or_dash(&s.auto_timer), or_dash(&s.net_timer));
}

pub fn or_dash(s: &str) -> &str {
    if s.is_empty() {
        "—"
    } else {
        s
    }
}

// ---------- уведомления (от имени пользователя) ----------

fn cmd_notify() {
    if !have("notify-send") {
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let cache = format!("{home}/.cache/upd-notified.json");
    let mut seen: BTreeMap<String, String> = fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut send = |key: &str, val: String, title: &str, body: &str| {
        if seen.get(key) == Some(&val) {
            return;
        }
        let _ = std::process::Command::new("notify-send").args(["-a", "upd", "-i", "system-software-update", title, body]).status();
        seen.insert(key.into(), val);
    };
    let u: UpdState = load_json("updates.json");
    let total = u.list.len() + u.flatpak.len();
    if total > 0 {
        let h = sha1_smol::Sha1::from((u.list.join("\n") + &u.flatpak.join("\n")).as_bytes()).digest().to_string();
        let body = if u.downloaded { "Уже скачаны. Запусти в терминале: upd" } else { "Запусти в терминале: upd" };
        send("updates", h, &format!("Доступно обновлений: {total}"), body);
    }
    if !u.news.is_empty() {
        let h = u.news.iter().map(|n| n.title.clone()).collect::<Vec<_>>().join("|");
        send("news", h, "Новости Arch перед обновлением", &u.news.iter().map(|n| n.title.clone()).collect::<Vec<_>>().join("\n"));
    }
    if !u.firmware.is_empty() {
        send("firmware", u.firmware.join("|"), "Доступны прошивки", &u.firmware.join("\n"));
    }
    if reboot_needed() {
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
        send("reboot", boot, "Нужна перезагрузка", "Обновилось ядро");
    }
    let m = load_mirror_state();
    if !m.event.is_empty() {
        send("mirrors", m.event_time.to_string(), "Зеркала подобраны заново", &m.event);
    }
    let _ = fs::create_dir_all(format!("{home}/.cache"));
    let _ = fs::write(&cache, serde_json::to_vec(&seen).unwrap_or_default());
}

// ---------- установка ----------

const BIN: &str = "/usr/local/bin/upd";
const NM_PATH: &str = "/etc/NetworkManager/dispatcher.d/90-upd";
const CRON_PATH: &str = "/etc/cron.d/upd";

fn system_units(b: &dyn Backend) -> Vec<(&'static str, String)> {
    let mut u = vec![
        (
            "upd-auto.service",
            "[Unit]\nDescription=upd: смена сети, замер зеркал, проверка и предзагрузка обновлений\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart=/usr/local/bin/upd auto\nNice=10\nIOSchedulingClass=idle\n".to_string(),
        ),
        (
            "upd-auto.timer",
            "[Unit]\nDescription=upd: периодическая проверка обновлений\n\n[Timer]\nOnBootSec=10min\nOnUnitActiveSec=6h\nRandomizedDelaySec=10min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            "upd-net.service",
            "[Unit]\nDescription=upd: подбор зеркал при смене сети\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart=/usr/local/bin/upd net\nNice=10\n".to_string(),
        ),
        (
            "upd-net.timer",
            "[Unit]\nDescription=upd: не сменилась ли сеть (дёшево, без запросов в сеть)\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec=15min\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
    ];
    if let (Some(w), true) = (b.watch_path(), b.mirrors_managed()) {
        u.push((
            "upd-mirrors.path",
            format!("[Unit]\nDescription=upd: вернуть закреплённые зеркала, если список перезаписали\n\n[Path]\nPathChanged={w}\nUnit=upd-mirrors-apply.service\n\n[Install]\nWantedBy=paths.target\n"),
        ));
        u.push((
            "upd-mirrors-apply.service",
            "[Unit]\nDescription=upd: вернуть закреплённые зеркала наверх\n\n[Service]\nType=oneshot\nExecStart=/usr/local/bin/upd mirrors apply\n".to_string(),
        ));
    }
    u
}

const USER_UNITS: &[(&str, &str)] = &[
    ("upd-notify.service", "[Unit]\nDescription=upd: уведомления об обновлениях\n\n[Service]\nType=oneshot\nExecStart=/usr/local/bin/upd notify\n"),
    ("upd-notify.timer", "[Unit]\nDescription=upd: уведомления об обновлениях\n\n[Timer]\nOnActiveSec=3min\nOnUnitActiveSec=30min\n\n[Install]\nWantedBy=timers.target\n"),
];

const PACMAN_HOOK: &str = "/etc/pacman.d/hooks/zz-upd.hook";
const PACMAN_HOOK_BODY: &str = "# upd: после любой транзакции сверить список доступных обновлений (без сети)\n[Trigger]\nOperation = Install\nOperation = Upgrade\nOperation = Remove\nType = Package\nTarget = *\n\n[Action]\nDescription = upd: сверяю список обновлений...\nWhen = PostTransaction\nExec = /usr/local/bin/upd reconcile\n";
const APT_HOOK: &str = "/etc/apt/apt.conf.d/99upd";
const APT_HOOK_BODY: &str = "// upd: после любой установки сверить список доступных обновлений (без сети)\nDPkg::Post-Invoke { \"/usr/local/bin/upd reconcile >/dev/null 2>&1 || true\"; };\n";
const GARUDA_CONF: &str = "/etc/garuda/garuda-update/config";
const GARUDA_MARK: &str = "# upd: зеркалами управляет upd — garuda-update их не перезаписывает";

/// garuda-update, запущенный напрямую, пересобирает зеркала через rate-mirrors — запрещаем, пока стоит upd.
fn garuda_skip_mirrors(on: bool) -> Option<String> {
    let text = fs::read_to_string(GARUDA_CONF).ok()?;
    let cleaned: Vec<&str> = text.lines().filter(|l| *l != GARUDA_MARK && !(l.trim() == "SKIP_MIRRORLIST=1" && text.contains(GARUDA_MARK))).collect();
    let mut body = cleaned.join("\n") + "\n";
    if on {
        body += &format!("{GARUDA_MARK}\nSKIP_MIRRORLIST=1\n");
    }
    (body != text).then(|| {
        let _ = atomic_write(Path::new(GARUDA_CONF), body.as_bytes(), 0o644);
        GARUDA_CONF.to_string()
    })
}

const NM_DISPATCHER: &str = "#!/bin/sh\n# upd: при смене сети проверить, не пора ли подобрать другие зеркала\ncase \"$2\" in\n  up|down|vpn-up|vpn-down|connectivity-change) systemctl start --no-block upd-net.service ;;\nesac\n";

fn cmd_install(b: &dyn Backend, c: &Config) -> i32 {
    let ok = |s: String| println!("\x1b[32m✓\x1b[0m {s}");
    let exe = std::env::current_exe().and_then(fs::canonicalize).unwrap_or_default();
    if exe != Path::new(BIN) {
        let _ = fs::create_dir_all("/usr/local/bin");
        let tmp = format!("{BIN}.new");
        if let Err(e) = fs::copy(&exe, &tmp).and_then(|_| fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o755))).and_then(|_| fs::rename(&tmp, BIN)) {
            eprintln!("не удалось скопировать бинарник: {e}");
            return 1;
        }
    }
    ok(format!("бинарник: {BIN}"));
    if !Path::new(&conf_path()).exists() {
        let _ = c.save();
    }
    ok(format!("настройки: {}", conf_path()));
    let _ = fs::create_dir_all(state_dir());

    if systemd() {
        let mut enable = vec![];
        for (name, body) in system_units(b) {
            if let Err(e) = fs::write(format!("/etc/systemd/system/{name}"), body) {
                eprintln!("{name}: {e}");
                return 1;
            }
            if name.ends_with(".timer") || name.ends_with(".path") {
                enable.push(name);
            }
        }
        let _ = fs::create_dir_all("/etc/systemd/user");
        for (name, body) in USER_UNITS {
            let _ = fs::write(format!("/etc/systemd/user/{name}"), body);
        }
        let _ = run(true, &[], "systemctl", &["daemon-reload"]);
        let mut args = vec!["enable", "--now"];
        args.extend(enable.iter().copied());
        if let Err(e) = run(false, &[], "systemctl", &args) {
            eprintln!("{e}");
            return 1;
        }
        let _ = run(true, &[], "systemctl", &["--global", "enable", "upd-notify.timer"]);
        ok(format!("службы: {}", enable.join(", ")));
        ok("уведомления: upd-notify.timer (для всех пользователей, после входа)".into());
        if Path::new("/etc/NetworkManager/dispatcher.d").is_dir() {
            let _ = fs::write(NM_PATH, NM_DISPATCHER).and_then(|_| fs::set_permissions(NM_PATH, std::os::unix::fs::PermissionsExt::from_mode(0o755)));
            ok(format!("реакция на смену сети: {NM_PATH}"));
        }
    } else if Path::new("/etc/cron.d").is_dir() {
        let _ = fs::write(CRON_PATH, "# upd\n*/15 * * * * root /usr/local/bin/upd net\n17 */6 * * * root /usr/local/bin/upd auto\n");
        ok(format!("systemd нет — задания в {CRON_PATH}"));
    } else {
        println!("⚠ нет ни systemd, ни cron: автоматика не установлена, запускай upd вручную");
    }

    if Path::new("/etc/pacman.d").is_dir() && have("pacman") {
        let _ = fs::create_dir_all("/etc/pacman.d/hooks");
        let _ = fs::write(PACMAN_HOOK, PACMAN_HOOK_BODY);
        ok(format!("хук pacman: {PACMAN_HOOK} (статус верный, даже если обновлялись в обход upd)"));
    } else if Path::new("/etc/apt/apt.conf.d").is_dir() {
        let _ = fs::write(APT_HOOK, APT_HOOK_BODY);
        ok(format!("хук apt: {APT_HOOK}"));
    }
    if b.mirrors_managed() && Path::new(GARUDA_CONF).exists() {
        garuda_skip_mirrors(true);
        ok(format!("garuda-update больше не перезаписывает зеркала ({GARUDA_CONF})"));
    }

    if b.mirrors_managed() {
        println!("\nПервый подбор зеркал для текущей сети:");
        with_lock(true, || {
            handle_network(b, c, &stdlog, true);
            0
        });
    } else {
        println!("\n{}", b.mirror_note());
    }
    if systemd() {
        // таймеры сработали во время установки и пропустили ход из-за блокировки — первая проверка сейчас
        let _ = run(true, &[], "systemctl", &["start", "--no-block", "upd-auto.service"]);
        println!("\nПервая проверка обновлений запущена в фоне: journalctl -u upd-auto -f");
    }
    // Garuda: в стандартных настройках fish/bash есть alias upd → garuda-update, он перехватывает команду
    let shadowed = ["/usr/share/garuda/garuda-fish-config/config.fish", "/usr/share/garuda/garuda-bash-config/bashrc"]
        .iter()
        .any(|f| fs::read_to_string(f).map(|t| t.lines().any(|l| l.trim_start().starts_with("alias upd"))).unwrap_or(false));
    if shadowed {
        println!("\n\x1b[33m⚠ В Garuda команда `upd` занята алиасом на garuda-update. Сними его в своих настройках:\x1b[0m");
        println!("   fish: добавь `functions -e upd` в ~/.config/fish/config.fish (ниже строки source …garuda-fish-config…)");
        println!("   bash: добавь `unalias upd 2>/dev/null` в ~/.bashrc (ниже строки source …garuda-bash-config…)");
        println!("   или запускай полным путём: {BIN}");
    }
    println!("\nГотово. Запуск интерфейса: upd");
    0
}

fn cmd_uninstall(b: &dyn Backend) -> i32 {
    if systemd() {
        let mut names: Vec<&str> = system_units(b).iter().map(|x| x.0).collect();
        names.extend(["upd-mirrors.path", "upd-mirrors-apply.service"]);
        let _ = run(true, &[], "systemctl", &["--global", "disable", "upd-notify.timer"]);
        let mut args = vec!["disable", "--now"];
        args.extend(names.iter().copied());
        let _ = run(true, &[], "systemctl", &args);
        for n in &names {
            let _ = fs::remove_file(format!("/etc/systemd/system/{n}"));
        }
        for (n, _) in USER_UNITS {
            let _ = fs::remove_file(format!("/etc/systemd/user/{n}"));
        }
        let _ = run(true, &[], "systemctl", &["daemon-reload"]);
    }
    let _ = fs::remove_file(NM_PATH);
    let _ = fs::remove_file(CRON_PATH);
    let _ = fs::remove_file(PACMAN_HOOK);
    let _ = fs::remove_file(APT_HOOK);
    garuda_skip_mirrors(false);
    if let Err(e) = b.remove_mirrors() {
        println!("не удалось вернуть зеркала: {e}");
    }
    let _ = fs::remove_file(BIN);
    println!("upd удалён. Оставлены: {} и {} (удали вручную, если не нужны).", conf_path(), state_dir());
    0
}
