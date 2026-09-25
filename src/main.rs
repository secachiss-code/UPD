mod backend;
mod common;
mod extras;
mod mirrors;
mod tui;
mod vpn;

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
  upd vpn [...]           VPN (mihomo): upd vpn help
  upd install | uninstall установить в систему / удалить
  upd reconcile           сверить список обновлений с установленным (без сети; вызывается хуком pacman/apt)
  upd version

Служебные (их запускают таймеры): upd auto, upd net, upd notify
";

fn stdlog(s: &str) {
    println!("{s}");
}

fn main() {
    // `upd list | head` не должен падать с паникой на закрытом канале
    unsafe {
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
    }
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pause = args.iter().any(|a| a == "--pause");
    let no_download = args.iter().any(|a| a == "--no-download");
    let all = args.iter().any(|a| a == "--all");
    let pkg_mode = args.iter().any(|a| a == "--package");
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
    let user_cmd = matches!(cmd.as_str(), "status" | "list" | "notify" | "news" | "gen-files") || (cmd == "mirrors" && pos.first().map(|s| s == "status").unwrap_or(true));
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
        "vpn" => cmd_vpn(&c, &pos),
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
        "install" => cmd_install(b, &c, pkg_mode),
        "gen-files" => cmd_gen_files(b, &pos),
        "uninstall" => cmd_uninstall(b, pkg_mode),
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
    vpn::maintain(c, &stdlog);
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

// ---------- VPN ----------

const VPN_USAGE: &str = "upd vpn — VPN на ядре mihomo (как в FlClash)

  upd vpn                 состояние
  upd vpn add [URL]       добавить подписку (без URL — спросит; так адрес не попадёт в историю шелла)
  upd vpn subs            подписки
  upd vpn use N | del N   сделать активной / удалить подписку номер N
  upd vpn update          обновить подписки сейчас
  upd vpn start | stop | restart
  upd vpn tun | proxy     режим: вся система (TUN) / только прокси на vpn_port
  upd vpn rule | global | direct   маршрутизация: по правилам / всё через VPN / всё напрямую
  upd vpn servers         группы и серверы с задержками
  upd vpn core [check|update|reinstall]  ядро mihomo (обновляется по релизам FlClash)
  upd vpn geo             обновить геофайлы
  upd vpn rules           править свои правила
";

fn cmd_vpn(c: &Config, pos: &[String]) -> i32 {
    let sub = pos.first().map(String::as_str).unwrap_or("status");
    let arg = pos.get(1).map(String::as_str);
    let mut c = c.clone();
    let apply = |c: &Config| err_code(vpn::apply(c, &stdlog));
    match sub {
        "help" => {
            print!("{VPN_USAGE}");
            0
        }
        "status" => {
            let s = vpn::snapshot();
            println!("VPN: {}", vpn_line());
            if s.running {
                println!("ядро {} · {} · маршрутизация: {}", s.version, if s.tun { "TUN (вся система)" } else { "только прокси" }, s.mode);
                println!("цепочка: {}", s.chain().join(" → "));
                println!("трафик: ↓ {} ↑ {} · соединений {}", fmt_bytes(s.down), fmt_bytes(s.up), s.conns);
            }
            if let Some(w) = vpn::flclash_running() {
                println!("⚠ {w}");
            }
            0
        }
        "add" => {
            let url = match arg {
                Some(u) => u.to_string(),
                None => {
                    print!("адрес подписки: ");
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    let mut s = String::new();
                    let _ = std::io::stdin().read_line(&mut s);
                    s.trim().to_string()
                }
            };
            let name = pos.get(2).cloned().unwrap_or_default();
            if let Err(e) = vpn::add_sub(&url, &name, &c, &stdlog) {
                eprintln!("ошибка: {e}");
                return 1;
            }
            if vpn::service_active() {
                apply(&c)
            } else {
                println!("подписка добавлена. Запуск VPN: upd vpn start");
                0
            }
        }
        "subs" | "list" => {
            let s = vpn::load_subs();
            if s.list.is_empty() {
                println!("подписок нет — upd vpn add");
            }
            for (i, x) in s.list.iter().enumerate() {
                let mark = if x.id == s.active { "●" } else { " " };
                let info = x.info.as_ref().map(sub_info).unwrap_or_default();
                let err = if x.error.is_empty() { String::new() } else { format!(" ⚠ {}", x.error) };
                println!("{} {}. {} ({}) · серверов {} · обновлена {}{info}{err}", mark, i + 1, x.name, vpn::mask_url(&x.url), x.nodes, fmt_ago(x.updated));
            }
            0
        }
        "use" | "del" => {
            let Some(n) = arg.and_then(|a| a.parse::<usize>().ok()).filter(|n| *n > 0) else {
                println!("укажи номер подписки (upd vpn subs)");
                return 2;
            };
            let r = if sub == "use" { vpn::use_sub(n - 1) } else { vpn::delete_sub(n - 1) };
            match r {
                Ok(name) => {
                    println!("{}: «{name}»", if sub == "use" { "активна" } else { "удалена" });
                    if vpn::service_active() && !vpn::load_subs().list.is_empty() {
                        return apply(&c);
                    }
                    0
                }
                Err(e) => err_code(Err(e)),
            }
        }
        "update" => {
            vpn::update_subs(&c, &stdlog, true);
            apply(&c)
        }
        "start" => {
            if let Err(e) = vpn::write_config(&c) {
                return err_code(Err(e));
            }
            let r = vpn::start(&c);
            if r.is_ok() {
                std::thread::sleep(std::time::Duration::from_secs(2));
                vpn::sysproxy(&c);
                println!("VPN: {}", vpn_line());
            }
            err_code(r)
        }
        "stop" => {
            let r = vpn::stop();
            vpn::sysproxy(&c);
            err_code(r)
        }
        "restart" => err_code(vpn::restart()),
        "tun" | "proxy" => {
            c.vpn_tun = sub == "tun";
            let _ = c.save();
            apply(&c)
        }
        "rule" | "global" | "direct" => {
            c.vpn_mode = ["rule", "global", "direct"].iter().position(|m| *m == sub).unwrap_or(0) as u8;
            let _ = c.save();
            let _ = vpn::write_config(&c);
            if vpn::running() {
                return err_code(vpn::set_mode(sub));
            }
            0
        }
        "servers" => {
            let s = vpn::snapshot();
            if !s.running {
                println!("VPN не запущен");
                return 1;
            }
            for g in &s.groups {
                println!("[{}] {} → {}", g.kind, g.name, g.now);
                for p in &g.all {
                    let d = s.delay.get(p).map(|d| if *d == 0 { "✗".to_string() } else { format!("{d} мс") }).unwrap_or_default();
                    println!("   {} {p:<40} {d}", if *p == g.now { "●" } else { " " });
                }
            }
            0
        }
        "core" => match arg.unwrap_or("check") {
            a @ ("update" | "reinstall") => {
                let r = vpn::core_install(&c, &stdlog, a == "reinstall");
                if matches!(r, Ok(true)) && vpn::service_active() {
                    let _ = vpn::restart();
                }
                err_code(r.map(|_| ()))
            }
            _ => err_code(vpn::core_check(&c, &stdlog).map(|_| ())),
        },
        "geo" => err_code(vpn::geo_update(&c, &stdlog, false)),
        "rules" => {
            let r = vpn::edit_rules();
            if r.is_ok() {
                return apply(&c);
            }
            err_code(r)
        }
        "prepare" => err_code(vpn::prepare(&c, &stdlog)),
        _ => {
            print!("{VPN_USAGE}");
            2
        }
    }
}

pub fn sub_info(i: &vpn::SubInfo) -> String {
    let mut s = String::new();
    if i.total > 0 {
        s += &format!(" · трафик {} из {}", fmt_bytes(i.upload + i.download), fmt_bytes(i.total));
    }
    if i.expire > 0 {
        s += &format!(" · до {}", fmt_time(i.expire).split(' ').next().unwrap_or(""));
    }
    s
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
    pub vpn: String,
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
        vpn: vpn_line(),
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
    println!("VPN:          {}", vpn_line());
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

/// Короткая строка о VPN без обращения к API (для статуса от имени пользователя).
pub fn vpn_line() -> String {
    let st = vpn::load_state();
    let state = match unit_state(vpn::SERVICE).as_str() {
        "active" => "работает",
        "failed" => "ОШИБКА (journalctl -u upd-vpn)",
        "" => "не установлен",
        _ => "выключен",
    };
    let sub = st.subs.iter().find(|s| s.active).map(|s| format!(" · «{}»", s.name)).unwrap_or_else(|| " · нет подписки".into());
    let core = if st.core_version.is_empty() { String::new() } else { format!(" · mihomo {}", st.core_version) };
    format!("{state}{sub}{core}")
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
    let v = vpn::load_state();
    if !v.event.is_empty() {
        send("vpn-core", v.event_time.to_string(), "VPN", &v.event);
    }
    for sp in v.subs.iter().filter(|s| s.active) {
        if let Some(i) = &sp.info {
            if i.expire > 0 && i.expire - now() < 3 * 86400 {
                send("vpn-expire", i.expire.to_string(), "Подписка VPN скоро закончится", &format!("«{}» — до {}", sp.name, fmt_time(i.expire)));
            }
            if i.total > 0 && (i.upload + i.download) * 10 >= i.total * 9 {
                send("vpn-traffic", i.total.to_string(), "Трафик VPN почти исчерпан", &format!("«{}»: {} из {}", sp.name, fmt_bytes(i.upload + i.download), fmt_bytes(i.total)));
            }
        }
    }
    let m = load_mirror_state();
    if !m.event.is_empty() {
        send("mirrors", m.event_time.to_string(), "Зеркала подобраны заново", &m.event);
    }
    let _ = fs::create_dir_all(format!("{home}/.cache"));
    let _ = fs::write(&cache, serde_json::to_vec(&seen).unwrap_or_default());
}

// ---------- установка ----------

const LOCAL_BIN: &str = "/usr/local/bin/upd";
const PKG_BIN: &str = "/usr/bin/upd";
const CRON_PATH: &str = "/etc/cron.d/upd";
const GARUDA_CONF: &str = "/etc/garuda/garuda-update/config";
const GARUDA_MARK: &str = "# upd: зеркалами управляет upd — garuda-update их не перезаписывает";

/// Куда кладутся системные файлы: ручная установка (upd install) — в /etc; пакет — в /usr/lib и /usr/share.
struct Layout {
    bin: &'static str,
    units: &'static str,
    user_units: &'static str,
    pacman_hook: &'static str,
    apt_hook: &'static str,
    nm: &'static str,
}

const MANUAL: Layout = Layout {
    bin: LOCAL_BIN,
    units: "/etc/systemd/system",
    user_units: "/etc/systemd/user",
    pacman_hook: "/etc/pacman.d/hooks/zz-upd.hook",
    apt_hook: "/etc/apt/apt.conf.d/99upd",
    nm: "/etc/NetworkManager/dispatcher.d/90-upd",
};

const PACKAGE: Layout = Layout {
    bin: PKG_BIN,
    units: "/usr/lib/systemd/system",
    user_units: "/usr/lib/systemd/user",
    pacman_hook: "/usr/share/libalpm/hooks/zz-upd.hook",
    apt_hook: "/etc/apt/apt.conf.d/99upd",
    nm: "/usr/lib/NetworkManager/dispatcher.d/90-upd",
};

fn system_units(bin: &str, watch: Option<&str>) -> Vec<(&'static str, String)> {
    let mut u = vec![
        (
            "upd-auto.service",
            format!("[Unit]\nDescription=upd: смена сети, зеркала, VPN, проверка и предзагрузка обновлений\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} auto\nNice=10\nIOSchedulingClass=idle\n"),
        ),
        (
            "upd-auto.timer",
            "[Unit]\nDescription=upd: периодическая проверка обновлений\n\n[Timer]\nOnBootSec=10min\nOnUnitActiveSec=6h\nRandomizedDelaySec=10min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            "upd-net.service",
            format!("[Unit]\nDescription=upd: подбор зеркал при смене сети\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} net\nNice=10\n"),
        ),
        (
            "upd-net.timer",
            "[Unit]\nDescription=upd: не сменилась ли сеть (дёшево, без запросов в сеть)\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec=15min\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            vpn::SERVICE,
            format!(
                "[Unit]\nDescription=upd: VPN (ядро mihomo)\nWants=network-online.target\nAfter=network-online.target\n\
                 StartLimitIntervalSec=10min\nStartLimitBurst=5\n\n[Service]\nType=simple\n\
                 ExecStartPre={bin} vpn prepare\nExecStart=/var/lib/upd/vpn/bin/mihomo -d /var/lib/upd/vpn -f /var/lib/upd/vpn/config.yaml\n\
                 Restart=on-failure\nRestartSec=5\nTimeoutStartSec=5min\nLimitNOFILE=1048576\n\n[Install]\nWantedBy=multi-user.target\n"
            ),
        ),
    ];
    if let Some(w) = watch {
        u.push((
            "upd-mirrors.path",
            format!("[Unit]\nDescription=upd: вернуть закреплённые зеркала, если список перезаписали\n\n[Path]\nPathChanged={w}\nUnit=upd-mirrors-apply.service\n\n[Install]\nWantedBy=paths.target\n"),
        ));
        u.push((
            "upd-mirrors-apply.service",
            format!("[Unit]\nDescription=upd: вернуть закреплённые зеркала наверх\n\n[Service]\nType=oneshot\nExecStart={bin} mirrors apply\n"),
        ));
    }
    u
}

fn user_units(bin: &str) -> Vec<(&'static str, String)> {
    vec![
        ("upd-notify.service", format!("[Unit]\nDescription=upd: уведомления\n\n[Service]\nType=oneshot\nExecStart={bin} notify\n")),
        ("upd-notify.timer", "[Unit]\nDescription=upd: уведомления\n\n[Timer]\nOnActiveSec=3min\nOnUnitActiveSec=30min\n\n[Install]\nWantedBy=timers.target\n".to_string()),
    ]
}

fn pacman_hook(bin: &str) -> String {
    format!("# upd: после любой транзакции сверить список доступных обновлений (без сети)\n[Trigger]\nOperation = Install\nOperation = Upgrade\nOperation = Remove\nType = Package\nTarget = *\n\n[Action]\nDescription = upd: сверяю список обновлений...\nWhen = PostTransaction\nExec = {bin} reconcile\n")
}

fn apt_hook(bin: &str) -> String {
    format!("// upd: после любой установки сверить список доступных обновлений (без сети)\nDPkg::Post-Invoke {{ \"{bin} reconcile >/dev/null 2>&1 || true\"; }};\n")
}

fn nm_dispatcher() -> &'static str {
    "#!/bin/sh\n# upd: при смене сети проверить, не пора ли подобрать другие зеркала\ncase \"$2\" in\n  up|down|vpn-up|vpn-down|connectivity-change) systemctl start --no-block upd-net.service ;;\nesac\n"
}

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

fn write_file(path: &str, body: &str, mode: u32) -> Result<(), String> {
    if let Some(d) = Path::new(path).parent() {
        fs::create_dir_all(d).map_err(|e| format!("{}: {e}", d.display()))?;
    }
    fs::write(path, body).map_err(|e| format!("{path}: {e}"))?;
    fs::set_permissions(path, std::os::unix::fs::PermissionsExt::from_mode(mode)).map_err(|e| format!("{path}: {e}"))
}

/// Все системные файлы под корнем root. target: arch | deb | rpm | host (текущая система).
fn write_system_files(root: &str, l: &Layout, target: &str, b: &dyn Backend) -> Result<Vec<String>, String> {
    let watch = match target {
        "arch" => Some("/etc/pacman.d/mirrorlist".to_string()),
        "host" => b.watch_path().filter(|_| b.mirrors_managed()),
        _ => None,
    };
    let mut done = vec![];
    for (n, body) in system_units(l.bin, watch.as_deref()) {
        write_file(&format!("{root}{}/{n}", l.units), &body, 0o644)?;
        done.push(n.to_string());
    }
    for (n, body) in user_units(l.bin) {
        write_file(&format!("{root}{}/{n}", l.user_units), &body, 0o644)?;
    }
    let pacman = target == "arch" || (target == "host" && have("pacman"));
    let apt = target == "deb" || (target == "host" && !pacman && Path::new("/etc/apt/apt.conf.d").is_dir());
    if pacman {
        write_file(&format!("{root}{}", l.pacman_hook), &pacman_hook(l.bin), 0o644)?;
    }
    if apt {
        write_file(&format!("{root}{}", l.apt_hook), &apt_hook(l.bin), 0o644)?;
    }
    if target != "host" || Path::new("/etc/NetworkManager").is_dir() {
        write_file(&format!("{root}{}", l.nm), nm_dispatcher(), 0o755)?;
    }
    Ok(done)
}

/// Для сборки пакетов: upd gen-files <каталог> <arch|deb|rpm>
fn cmd_gen_files(b: &dyn Backend, pos: &[String]) -> i32 {
    let (Some(root), Some(target)) = (pos.first(), pos.get(1)) else {
        eprintln!("upd gen-files <каталог> <arch|deb|rpm>");
        return 2;
    };
    match write_system_files(root.trim_end_matches('/'), &PACKAGE, target, b) {
        Ok(_) => 0,
        Err(e) => err_code(Err(e)),
    }
}

/// Следы ручной установки мешают пакетной: юниты в /etc перекрывают /usr/lib, /usr/local/bin — раньше в PATH.
fn remove_manual_files() {
    let _ = fs::remove_file(LOCAL_BIN);
    for dir in [MANUAL.units, MANUAL.user_units] {
        let Ok(rd) = fs::read_dir(dir) else { continue };
        let files: Vec<(String, std::path::PathBuf)> = rd
            .flatten()
            .map(|e| (e.file_name().to_string_lossy().into_owned(), e.path()))
            .filter(|(n, _)| n.starts_with("upd-") && (n.ends_with(".service") || n.ends_with(".timer") || n.ends_with(".path")))
            .collect();
        if files.is_empty() {
            continue;
        }
        // сначала disable: иначе в *.wants останутся ссылки на удалённые файлы
        let mut args: Vec<&str> = if dir == MANUAL.user_units { vec!["--global", "disable"] } else { vec!["disable"] };
        args.extend(files.iter().map(|(n, _)| n.as_str()));
        let _ = run(true, &[], "systemctl", &args);
        for (_, p) in &files {
            let _ = fs::remove_file(p);
        }
    }
    let _ = fs::remove_file(MANUAL.pacman_hook);
    let _ = fs::remove_file(MANUAL.nm);
}

fn enable_units(b: &dyn Backend, c: &Config, ok: &dyn Fn(String)) -> Result<(), String> {
    let _ = run(true, &[], "systemctl", &["daemon-reload"]);
    let mut enable = vec!["upd-auto.timer", "upd-net.timer"];
    if b.mirrors_managed() && b.watch_path().is_some() {
        enable.push("upd-mirrors.path");
    }
    let mut args = vec!["enable", "--now"];
    args.extend(enable.iter().copied());
    run(false, &[], "systemctl", &args)?;
    let _ = run(true, &[], "systemctl", &["--global", "enable", "upd-notify.timer"]);
    ok(format!("службы: {}", enable.join(", ")));
    ok("уведомления: upd-notify.timer (для всех пользователей, после входа)".into());
    if !vpn::load_subs().list.is_empty() && c.vpn_autostart {
        let _ = vpn::autostart(true);
        ok("VPN: автозапуск включён".into());
    } else {
        ok("VPN: добавь подписку — upd → VPN (или upd vpn add), дальше он будет стартовать сам".into());
    }
    Ok(())
}

fn cmd_install(b: &dyn Backend, c: &Config, pkg: bool) -> i32 {
    let ok = |s: String| println!("\x1b[32m✓\x1b[0m {s}");
    let exe = std::env::current_exe().and_then(fs::canonicalize).unwrap_or_default();
    let l = if pkg { &PACKAGE } else { &MANUAL };
    if !pkg && Path::new(PKG_BIN).exists() && exe != Path::new(PKG_BIN) {
        eprintln!("upd уже установлен пакетом ({PKG_BIN}) — обновляй его через пакетный менеджер.");
        return 1;
    }
    if pkg {
        remove_manual_files();
    } else if exe != Path::new(LOCAL_BIN) {
        let _ = fs::create_dir_all("/usr/local/bin");
        let tmp = format!("{LOCAL_BIN}.new");
        if let Err(e) = fs::copy(&exe, &tmp).and_then(|_| fs::set_permissions(&tmp, std::os::unix::fs::PermissionsExt::from_mode(0o755))).and_then(|_| fs::rename(&tmp, LOCAL_BIN)) {
            eprintln!("не удалось скопировать бинарник: {e}");
            return 1;
        }
    }
    ok(format!("бинарник: {}", l.bin));
    if !Path::new(&conf_path()).exists() {
        let _ = c.save();
    }
    ok(format!("настройки: {}", conf_path()));
    let _ = fs::create_dir_all(state_dir());

    if systemd() {
        if !pkg {
            match write_system_files("", l, "host", b) {
                Ok(_) => ok(format!("юниты, хуки и реакция на смену сети: {}", l.units)),
                Err(e) => return err_code(Err(e)),
            }
        }
        if let Err(e) = enable_units(b, c, &ok) {
            return err_code(Err(e));
        }
    } else if Path::new("/etc/cron.d").is_dir() {
        let _ = fs::write(CRON_PATH, format!("# upd\n*/15 * * * * root {0} net\n17 */6 * * * root {0} auto\n", l.bin));
        ok(format!("systemd нет — задания в {CRON_PATH} (VPN запускай вручную: upd vpn start)"));
    } else {
        println!("⚠ нет ни systemd, ни cron: автоматика не установлена, запускай upd вручную");
    }
    if b.mirrors_managed() && Path::new(GARUDA_CONF).exists() {
        garuda_skip_mirrors(true);
        ok(format!("garuda-update больше не перезаписывает зеркала ({GARUDA_CONF})"));
    }

    // из пакетного менеджера — без долгого подбора зеркал (он держит блокировку); это сделает upd-auto
    if b.mirrors_managed() && !pkg {
        println!("\nПервый подбор зеркал для текущей сети:");
        with_lock(true, || {
            handle_network(b, c, &stdlog, true);
            0
        });
    }
    if systemd() {
        let _ = run(true, &[], "systemctl", &["start", "--no-block", "upd-auto.service"]);
        println!("\nПервая проверка обновлений{} запущена в фоне: journalctl -u upd-auto -f", if pkg { " и подбор зеркал" } else { "" });
    }
    // Garuda: в стандартных настройках fish/bash есть alias upd → garuda-update, он перехватывает команду
    let shadowed = ["/usr/share/garuda/garuda-fish-config/config.fish", "/usr/share/garuda/garuda-bash-config/bashrc"]
        .iter()
        .any(|f| fs::read_to_string(f).map(|t| t.lines().any(|l| l.trim_start().starts_with("alias upd"))).unwrap_or(false));
    if shadowed {
        println!("\n\x1b[33m⚠ В Garuda команда `upd` занята алиасом на garuda-update. Сними его в своих настройках:\x1b[0m");
        println!("   fish: добавь `functions -e upd` в ~/.config/fish/config.fish (ниже строки source …garuda-fish-config…)");
        println!("   bash: добавь `unalias upd 2>/dev/null` в ~/.bashrc (ниже строки source …garuda-bash-config…)");
        println!("   или запускай полным путём: {}", l.bin);
    }
    println!("\nГотово. Запуск интерфейса: upd");
    0
}

fn cmd_uninstall(b: &dyn Backend, pkg: bool) -> i32 {
    if systemd() {
        let mut names: Vec<&str> = system_units(PKG_BIN, Some("-")).iter().map(|x| x.0).collect();
        names.retain(|n| !n.ends_with(".service") || *n == vpn::SERVICE);
        let _ = run(true, &[], "systemctl", &["--global", "disable", "upd-notify.timer"]);
        let mut args = vec!["disable", "--now"];
        args.extend(names.iter().copied());
        let _ = run(true, &[], "systemctl", &args);
        if !pkg {
            remove_manual_files();
            let _ = fs::remove_file(MANUAL.apt_hook);
        }
        let _ = run(true, &[], "systemctl", &["daemon-reload"]);
    }
    let c = Config::load(b.default_mirrors());
    vpn::sysproxy(&c);
    let _ = fs::remove_file(CRON_PATH);
    garuda_skip_mirrors(false);
    if let Err(e) = b.remove_mirrors() {
        println!("не удалось вернуть зеркала: {e}");
    }
    println!(
        "upd удалён. Оставлены настройки и данные: {}, {} (подписки VPN), {} — удали вручную, если не нужны.",
        conf_path(),
        vpn::etc(),
        state_dir()
    );
    0
}
