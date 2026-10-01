#[macro_use]
extern crate upd;

mod tui;

use upd::backend::{self, Backend};
use upd::common::*;
use upd::mirrors::*;
use upd::{extras, helper, i18n, vpn, VERSION};
use upd::{gather_status, sub_info, vpn_line};
use std::collections::BTreeMap;
use std::fs;
use std::os::unix::process::CommandExt;
use std::path::Path;

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
  upd aur search ЗАПРОС   найти пакет в AUR (Arch и производные)
  upd aur install ПАКЕТ…  собрать и установить из AUR через paru/yay
  upd lang [КОД]          язык интерфейса: ru en de it zh ar auto
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
    i18n::set(i18n::resolve(&conf_lang()));
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let pause = args.iter().any(|a| a == "--pause");
    let no_download = args.iter().any(|a| a == "--no-download");
    let all = args.iter().any(|a| a == "--all");
    let pkg_mode = args.iter().any(|a| a == "--package");
    if let Some(a) = args.first().filter(|a| matches!(a.as_str(), "--version" | "-V" | "--help")) {
        args[0] = if a == "--help" { "help" } else { "version" }.into();
    }
    args.retain(|a| !a.starts_with("--"));
    let cmd = args.first().cloned().unwrap_or_else(|| "tui".into());
    let pos: Vec<String> = args.iter().skip(1).cloned().collect();

    match cmd.as_str() {
        "-h" | "help" => return print!("{}", t!(USAGE)),
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
    let user_cmd = matches!(cmd.as_str(), "status" | "list" | "notify" | "news" | "gen-files" | "aur") || (cmd == "mirrors" && pos.first().map(|s| s == "status").unwrap_or(true))
        || (cmd == "lang" && pos.is_empty());
    if !user_cmd {
        become_root();
    }
    let mut c = match Config::load(b.default_mirrors()) {
        Ok(c) => c,
        Err(e) => {
            eprintln!("upd: {e}");
            std::process::exit(1);
        }
    };
    if is_root() && !Path::new(&conf_path()).exists() {
        let _ = c.save();
    }
    let b = b.as_ref();
    let code = match cmd.as_str() {
        "tui" => tui::run(b),
        "update" => cmd_update(b, &c),
        "check" => with_lock(true, || {
            let user = invoking_user();
            let mut st = gather(b, &c, &stdlog, false, user.as_deref());
            if !no_download && st.error.is_empty() {
                download(b, &c, &mut st, &stdlog, false, false);
            }
            state_check_failure(&st).is_some() as i32
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
        "aur" => cmd_aur(b, &pos),
        "lang" => cmd_lang(&c, &pos),
        "news" => {
            if !b.arch_news() {
                println!("{}", t!("новости есть только для Arch и производных"));
                return;
            }
            let since = if all { 0 } else { b.last_upgrade() };
            match extras::arch_news(since) {
                Ok(n) if n.is_empty() => println!("{}", t!("новых новостей с прошлого обновления ({}) нет", fmt_time(since))),
                Ok(n) => {
                    for x in n.iter().take(if all { 10 } else { usize::MAX }) {
                        println!("{}  {}\n      {}", fmt_time(x.date), x.title, x.link);
                    }
                }
                Err(e) => println!("{}", t!("не удалось загрузить: {0}", e)),
            }
            0
        }
        "clean" => err_code(b.clean()),
        "merge" => err_code(b.merge()),
        "auto" => with_lock(false, || cmd_auto(b, &c)),
        "net" => with_lock(false, || {
            if let Some(why) = background_block(&c).filter(|w| w == t!("лимитная сеть — фоновая загрузка отложена")) {
                println!("{}", t!("{0}: замер зеркал пропущен", why));
                return 0;
            }
            match handle_network(b, &c, &stdlog, false) {
                Ok(_) => 0,
                Err(e) => err_code(Err(e)),
            }
        }),
        "notify" => {
            cmd_notify();
            0
        }
        "helper" => helper::serve(),
        "install" => cmd_install(b, &c, pkg_mode),
        "gen-files" => cmd_gen_files(b, &pos),
        "uninstall" => cmd_uninstall(b, pkg_mode),
        _ => {
            eprint!("{}", t!("неизвестная команда: {0}\n\n{1}", cmd, t!(USAGE)));
            2
        }
    };
    if pause {
        print!("{}", t!("\n\x1b[2mEnter — вернуться в меню\x1b[0m "));
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let _ = std::io::stdin().read_line(&mut String::new());
    }
    std::process::exit(code);
}

fn err_code(r: Result<(), String>) -> i32 {
    match r {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("{}", t!("ошибка: {0}", e));
            1
        }
    }
}

fn state_check_failure(st: &UpdState) -> Option<&str> {
    [st.error.as_str(), st.flatpak_error.as_str(), st.firmware_error.as_str()]
        .into_iter().find(|error| !error.is_empty())
}

fn auto_state_failure(st: &UpdState) -> Option<&str> {
    if st.package_check_failure.is_some() {
        Some(st.error.as_str())
    } else if let Some(error) = st.space_check_error.as_deref() {
        Some(error)
    } else if !st.error.is_empty() && !st.error.starts_with(t!("мало места")) {
        Some(st.error.as_str())
    } else if !st.flatpak_error.is_empty() {
        Some(st.flatpak_error.as_str())
    } else if !st.firmware_error.is_empty() {
        Some(st.firmware_error.as_str())
    } else {
        None
    }
}

fn record_upgrade_error(result: &mut Result<(), String>, error: String) {
    if result.is_ok() {
        *result = Err(error);
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
    eprintln!("{}", t!("upd: нужны права root (sudo/doas не найдены)"));
    std::process::exit(1);
}

fn with_lock(block: bool, f: impl FnOnce() -> i32) -> i32 {
    let l = match lock(false) {
        Ok(l) => l,
        Err(_) if block => {
            println!("{}", t!("идёт фоновая задача upd, жду завершения..."));
            match lock(true) {
                Ok(l) => l,
                Err(e) => {
                    println!("upd: {e}");
                    return 1;
                }
            }
        }
        Err(_) => {
            println!("{}", t!("upd: занято другой задачей upd"));
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
    let metered = block.as_deref().map(|w| w == t!("лимитная сеть — фоновая загрузка отложена")).unwrap_or(false);
    if metered {
        println!("{}", t!("{}: пропускаю всё", block.as_deref().unwrap_or("")));
        let mut st: UpdState = load_json("updates.json");
        st.skipped = block.unwrap_or_default();
        return match save_json("updates.json", &st) {
            Ok(()) => 0,
            Err(e) => {
                eprintln!("{}", t!("upd auto: не удалось сохранить updates.json: {0}", e));
                1
            }
        };
    }
    let mut mirror_error = None;
    let changed = match handle_network(b, c, &stdlog, false) {
        Ok(changed) => changed,
        Err(e) => {
            stdlog(&t!("зеркала не применены: {0}", e));
            mirror_error = Some(e);
            true
        }
    };
    let ms = load_mirror_state();
    if !changed && b.mirrors_managed() && elapsed_at_least(ms.checked, hours_secs(c.mirror_max_age_h)) {
        if let Err(e) = check_mirrors(b, c, &stdlog, None, None) {
            stdlog(&t!("зеркала не применены: {0}", e));
            mirror_error = Some(e);
        }
    }
    vpn::maintain(c, &stdlog);
    let mut st = gather(b, c, &stdlog, true, None);
    match (&block, c.prefetch && st.error.is_empty()) {
        (Some(why), true) => {
            println!("{why}");
            st.skipped = why.clone();
            let _ = save_json("updates.json", &st);
        }
        (None, true) => download(b, c, &mut st, &stdlog, true, false),
        _ => {}
    }
    if c.prefetch && st.error.starts_with(t!("мало места")) && st.skipped.is_empty() {
        st.skipped = st.error.clone();
    }
    if let Err(e) = save_json("updates.json", &st) {
        eprintln!("{}", t!("upd auto: не удалось сохранить updates.json: {0}", e));
        return 1;
    }
    if let Some(error) = auto_state_failure(&st) {
        eprintln!("{}", t!("upd auto: проверка или предзагрузка завершилась с ошибкой: {0}", error));
        return 1;
    }
    if let Some(error) = mirror_error {
        eprintln!("{}", t!("upd auto: зеркала не применены: {0}", error));
        return 1;
    }
    0
}

fn cmd_aur(b: &dyn Backend, pos: &[String]) -> i32 {
    if !b.aur() {
        eprintln!("{}", t!("AUR есть только в Arch и производных"));
        return 1;
    }
    match (pos.first().map(String::as_str), &pos[1.min(pos.len())..]) {
        (Some("search" | "s"), q) if !q.is_empty() => match extras::aur_search(&q.join(" ")) {
            Ok(list) if list.is_empty() => {
                println!("{}", t!("ничего не найдено"));
                0
            }
            Ok(list) => {
                for p in list.iter().rev() {
                    let mut tags = vec![format!("★{}", p.votes)];
                    if let Some(v) = &p.installed {
                        tags.push(t!("установлен {0}", v));
                    }
                    if p.out_of_date {
                        tags.push(t!("устарел").into());
                    }
                    println!("\x1b[1m{}\x1b[0m {}  \x1b[2m{}\x1b[0m\n    {}", p.name, p.version, tags.join(" · "), p.desc);
                }
                println!("{}", t!("\nустановить: upd aur install ИМЯ"));
                0
            }
            Err(e) => err_code(Err(e)),
        },
        (Some("install" | "i"), pkgs) if !pkgs.is_empty() => err_code(extras::aur_install(invoking_user().as_deref(), pkgs)),
        _ => {
            eprintln!("{}", t!("upd aur search ЗАПРОС | upd aur install ПАКЕТ…"));
            2
        }
    }
}

fn cmd_update(b: &dyn Backend, c: &Config) -> i32 {
    with_lock(true, || {
        let user = invoking_user();

        step(1, t!("Зеркала"));
        if b.mirrors_managed() {
            let handled = match handle_network(b, c, &stdlog, false) {
                Ok(handled) => handled,
                Err(e) => {
                    println!("{}", t!("\x1b[31mНе удалось применить зеркала: {0}\x1b[0m", e));
                    return 1;
                }
            };
            if !handled {
                let st = load_mirror_state();
                if elapsed_at_least(st.checked, hours_secs(c.mirror_max_age_h)) {
                    if let Err(e) = check_mirrors(b, c, &stdlog, None, None) {
                        println!("{}", t!("\x1b[31mНе удалось применить зеркала: {0}\x1b[0m", e));
                        return 1;
                    }
                } else {
                    if let Err(e) = apply_mirrors(b, c, &st, None, &stdlog) {
                        println!("{}", t!("\x1b[31mНе удалось применить зеркала: {0}\x1b[0m", e));
                        return 1;
                    }
                    println!("{}", t!("последний замер: {}", fmt_ago(st.checked)));
                }
            }
        } else {
            println!("{}", b.mirror_note());
        }

        step(2, t!("Проверка"));
        let mut st = gather(b, c, &stdlog, false, user.as_deref());
        match st.package_check_failure {
            Some(PackageCheckFailure::Refresh) => {
                println!("{}", t!("\x1b[31mБазы пакетов не обновились — проверь сеть и попробуй ещё раз.\x1b[0m"));
                return 1;
            }
            Some(PackageCheckFailure::UpdateList) => {
                println!("{}", t!("\x1b[31mНе удалось проверить системные пакеты: {}\x1b[0m", st.error));
                return 1;
            }
            None => {}
        }
        // ошибки Flatpak и прошивок gather уже вывел
        let aur_enabled = c.aur && extras::aur_helper().is_some();
        let aur = match (user.as_deref(), aur_enabled) {
            (Some(u), true) => match extras::aur_updates(u) {
                Ok(updates) => updates,
                Err(e) => {
                    println!("{}", t!("\x1b[31mAUR: не удалось проверить обновления: {0}\x1b[0m", e));
                    return 1;
                }
            },
            (None, true) => {
                println!("{}", t!("\x1b[31mAUR: для проверки нужен обычный пользователь; запусти upd через sudo или doas\x1b[0m"));
                return 1;
            }
            _ => vec![],
        };
        if !aur.is_empty() {
            println!("{}", t!("AUR: {} обновлений", aur.len()));
        }
        if let Some(line) = nothing_to_update_line(
            st.list.is_empty(),
            st.flatpak.is_empty(),
            aur.is_empty(),
            st.firmware.is_empty(),
            st.flatpak_error.is_empty() && st.firmware_error.is_empty(),
        ) {
            if line == t!("Система уже обновлена.") {
                println!("\x1b[32m{line}\x1b[0m");
            } else {
                println!("{line}");
            }
            return state_check_failure(&st).is_some() as i32;
        }
        if !st.news.is_empty() {
            println!("{}", t!("\n\x1b[1;33mНовости Arch с прошлого обновления — прочитай, там бывают ручные шаги:\x1b[0m"));
            for n in &st.news {
                println!("  {}  {}\n      {}", fmt_time(n.date), n.title, n.link);
            }
            if !confirm(t!("\nПрочитал(а), продолжить?"), true) {
                return 1;
            }
        }
        let size_unknown = !st.list.is_empty() && st.download_size.is_none();
        let low_space = st.error.starts_with(t!("мало места"));
        let warning = if low_space && size_unknown {
            Some(t!("Мало места даже для заданного резерва, полный объём загрузки неизвестен. Всё равно продолжить?"))
        } else if low_space {
            Some(t!("Мало места на диске. Всё равно продолжить?"))
        } else if size_unknown {
            Some(t!("Полный объём загрузки неизвестен; свободного места может не хватить. Всё равно продолжить?"))
        } else {
            None
        };
        if let Some(warning) = warning {
            if !confirm(warning, false) {
                return 1;
            }
        }

        step(3, t!("Загрузка"));
        download(b, c, &mut st, &stdlog, false, true);
        if !st.list.is_empty() && !st.downloaded && !confirm(t!("Не всё скачалось. Всё равно запустить установку?"), false) {
            return 1;
        }

        step(4, t!("Снапшот"));
        let mut pre = None;
        if !c.snapshot {
            println!("{}", t!("отключено в настройках (snapshot = 0)"));
        } else if b.auto_snapshots() {
            println!("{}", t!("снапшоты «до» и «после» сделает snap-pac автоматически"));
        } else {
            match extras::snap_pre() {
                Ok(n) => {
                    println!("{}", t!("снапшот создан{}", n.as_deref().map(|x| format!(": #{x}")).unwrap_or_default()));
                    pre = n;
                }
                Err(e) => {
                    println!("\x1b[33m{e}\x1b[0m");
                    if !confirm(t!("Продолжить без снапшота?"), true) {
                        return 1;
                    }
                }
            }
        }

        step(5, t!("Установка"));
        let aur_by_installer = c.aur && b.upgrade_handles_aur();
        if !st.list.is_empty() && !b.upgrade_asks() {
            for l in &st.list {
                println!("  {l}");
            }
            if !confirm(&t!("Установить обновления ({} шт.)?", st.list.len()), true) {
                println!("{}", t!("установка отменена"));
                return 1;
            }
        }
        let mut result = if st.list.is_empty() && !aur_by_installer { Ok(()) } else { b.upgrade(aur_by_installer) };
        let packages_installed = result.is_ok();
        if result.is_ok() && !aur.is_empty() && !aur_by_installer {
            if let Some(u) = &user {
                println!("\n→ AUR");
                result = extras::aur_upgrade(u);
            }
        }
        if packages_installed {
            st.list.clear();
            st.downloaded = false;
            st.download_size = None;
            st.package_check_failure = None;
            if st.space_check_error.is_some() || st.error.starts_with(t!("мало места")) || st.error == t!("не удалось скачать обновления") {
                st.error.clear();
            }
            st.space_check_error = None;
        }
        if !st.flatpak.is_empty() {
            println!("\n→ Flatpak");
            match extras::flatpak_upgrade(user.as_deref(), &st.flatpak) {
                Ok(()) => {
                    st.flatpak.clear();
                    st.flatpak_error.clear();
                }
                Err(e) => {
                    println!("Flatpak: {e}");
                    st.flatpak_error = e.clone();
                    record_upgrade_error(&mut result, e);
                }
            }
        }
        if let Some(p) = &pre {
            extras::snap_post(p);
        }

        step(6, t!("После обновления"));
        if !st.firmware.is_empty() {
            println!("{}", t!("Доступны прошивки:"));
            for f in &st.firmware {
                println!("  {f}");
            }
            if confirm(t!("Установить прошивки? (может понадобиться перезагрузка)"), false) {
                match extras::firmware_upgrade() {
                    Ok(()) => {
                        st.firmware.clear();
                        st.firmware_error.clear();
                    }
                    Err(e) => {
                        println!("{}", t!("Прошивки: {0}", e));
                        st.firmware_error = e.clone();
                        record_upgrade_error(&mut result, e);
                    }
                }
            } else {
                println!("{}", t!("Прошивки оставлены в списке."));
            }
        }
        if let Err(e) = save_json("updates.json", &st) {
            println!("{}", t!("не удалось сохранить состояние обновлений: {0}", e));
            if result.is_ok() {
                result = Err(e.to_string());
            }
        }
        if result.is_ok() {
            if let Some(error) = state_check_failure(&st) {
                result = Err(error.to_string());
            }
        }
        match &result {
            Ok(()) => println!("{}", t!("\n\x1b[32m✓ Обновление завершено\x1b[0m")),
            Err(e) => println!("{}", t!("\n\x1b[31m✗ Обновление завершилось с ошибкой: {0}\x1b[0m", e)),
        }
        let r = needs_restart();
        extras::print_restart(&r);
        if !r.services.is_empty() && confirm(t!("Перезапустить эти службы сейчас?"), true) {
            extras::restart_services(&r.services);
        }
        if reboot_needed() || !r.critical.is_empty() {
            println!("{}", t!("\x1b[33m⟳ Нужна перезагрузка\x1b[0m"));
        }
        let p = b.pending_configs();
        if !p.is_empty() {
            println!("{}", t!("\x1b[33m⚙ Новых файлов настроек: {} — upd merge\x1b[0m", p.len()));
        }
        if result.is_ok() && (pre.is_some() || b.auto_snapshots()) {
            println!("{}", t!("\x1b[2mЕсли что-то сломалось: upd snapshots — как откатиться\x1b[0m"));
        }
        result.is_err() as i32
    })
}

fn cmd_list() -> i32 {
    let st: UpdState = load_json("updates.json");
    println!("{}", t!("проверено: {}, пакетов: {}", fmt_time(st.checked), st.list.len()));
    for l in &st.list {
        println!("  {l}");
    }
    if !st.flatpak_error.is_empty() {
        println!("{}", t!("Flatpak: ошибка проверки: {}", st.flatpak_error));
    }
    if !st.firmware_error.is_empty() {
        println!("{}", t!("Прошивки: ошибка проверки: {}", st.firmware_error));
    }
    for (title, list) in [("Flatpak", &st.flatpak), (t!("Прошивки"), &st.firmware)] {
        if !list.is_empty() {
            println!("{title}:");
            for l in list {
                println!("  {l}");
            }
        }
    }
    for n in &st.news {
        println!("{}", t!("новость {}: {} — {}", fmt_time(n.date), n.title, n.link));
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
        println!("{}", t!("upd: доступных обновлений было {}, стало {}", st.list.len(), list.len()));
        if list.is_empty() {
            st.downloaded = false;
            st.download_size = None;
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
    if !r.services.is_empty() && confirm(t!("Перезапустить службы?"), true) {
        extras::restart_services(&r.services);
    }
    0
}

fn cmd_mirrors(b: &dyn Backend, c: &Config, pos: &[String]) -> i32 {
    let sub = pos.first().map(String::as_str).unwrap_or("status");
    match sub {
        "status" => {
            let st = load_mirror_state();
            println!("{}", t!("сеть: {}\nпоследний замер: {}", fingerprint().label, fmt_time(st.checked)));
            if st.pending_apply {
                println!("{}", t!("⚠ зеркала ожидают применения: {}", if st.apply_error.is_empty() { t!("будет повторено при следующей проверке сети") } else { st.apply_error.as_str() }));
            }
            if !b.mirrors_managed() {
                println!("{}", b.mirror_note());
                return 0;
            }
            for m in b.pinned() {
                println!("  ● {m}");
            }
            for p in &st.results {
                let lag = p.lag_h.map(|l| t!(" · отставание {0} ч", l)).unwrap_or_default();
                println!("  {:<12} {:<7} {}{lag}", fmt_speed(p), i18n::tr_data(&p.src), p.url);
            }
            for h in &st.hints {
                println!("💡 {h}");
            }
            0
        }
        "check" => with_lock(true, || {
            match check_mirrors(b, c, &stdlog, None, None) {
                Ok(_) => 0,
                Err(e) => err_code(Err(e)),
            }
        }),
        "rescan" => with_lock(true, || {
            match rescan(b, c, &stdlog) {
                Ok(_) => 0,
                Err(e) => err_code(Err(e)),
            }
        }),
        // вызывается слежением за файлом: без сети и без блокировки, мгновенно
        "apply" => {
            match apply_mirrors(b, c, &load_mirror_state(), None, &stdlog) {
                Ok(_) => 0,
                Err(e) => err_code(Err(e)),
            }
        }
        "add" | "del" => {
            let Some(u) = pos.get(1) else {
                println!("{}", t!("укажи URL"));
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
  upd vpn use N | del N   сделать активной / удалить подписку номер N (или id:ИД)
  upd vpn update          обновить подписки сейчас
  upd vpn start | stop | restart
  upd vpn tun | proxy     режим: вся система (TUN) / только прокси на vpn_port
  upd vpn rule | global | direct   маршрутизация: по правилам / всё через VPN / всё напрямую
  upd vpn servers         группы и серверы с задержками
  upd vpn core [check|update|reinstall]  ядро mihomo (обновляется по релизам FlClash)
  upd vpn geo             обновить геофайлы
  upd vpn rules           править свои правила
";

fn core_update_exit(changed: bool, active: bool, restart: Result<(), String>) -> Result<(), String> {
    if changed && active {
        restart.map_err(|e| t!("ядро обновлено на диске, но VPN не перезапустился: {0}", e))?;
    }
    Ok(())
}

fn nothing_to_update_line(packages_empty: bool, flatpak_empty: bool, aur_empty: bool, firmware_empty: bool, optional_sources_ok: bool) -> Option<&'static str> {
    if packages_empty && flatpak_empty && aur_empty && firmware_empty {
        Some(if optional_sources_ok {
            t!("Система уже обновлена.")
        } else {
            t!("Системные пакеты обновлены; состояние дополнительных источников проверить не удалось.")
        })
    } else {
        None
    }
}

fn cmd_vpn(c: &Config, pos: &[String]) -> i32 {
    let sub = pos.first().map(String::as_str).unwrap_or("status");
    let arg = pos.get(1).map(String::as_str);
    let mut c = c.clone();
    let user = match cli_user_context() { Ok(user) => user, Err(error) => return err_code(Err(error)) };
    let apply = |c: &Config| err_code(vpn::apply_saved(c, user.as_ref(), &stdlog));
    match sub {
        "help" => {
            print!("{}", t!(VPN_USAGE));
            0
        }
        "status" => {
            let s = vpn::snapshot();
            println!("VPN: {}", vpn_line());
            if s.running {
                println!("{}", t!("ядро {} · {} · маршрутизация: {}", s.version, if s.tun { t!("TUN (вся система)") } else { t!("только прокси") }, s.mode));
                println!("{}", t!("цепочка: {}", s.chain().iter().map(|n| vpn::label(n)).collect::<Vec<_>>().join(" → ")));
                println!("{}", t!("трафик: ↓ {} ↑ {} · соединений {}", fmt_bytes(s.down), fmt_bytes(s.up), s.conns));
            }
            if let Some(w) = vpn::conflict(&c) {
                println!("⚠ {w}");
            }
            0
        }
        "add" => {
            let url = match arg {
                Some(u) => u.to_string(),
                None => {
                    print!("{}", t!("адрес подписки: "));
                    let _ = std::io::Write::flush(&mut std::io::stdout());
                    let mut s = String::new();
                    let _ = std::io::stdin().read_line(&mut s);
                    s.trim().to_string()
                }
            };
            let name = pos.get(2).cloned().unwrap_or_default();
            if let Err(e) = vpn::add_sub(&url, &name, &c, &stdlog) {
                eprintln!("{}", t!("ошибка: {0}", e));
                return 1;
            }
            if vpn::service_active() {
                apply(&c)
            } else {
                println!("{}", t!("подписка добавлена. Запуск VPN: upd vpn start"));
                0
            }
        }
        "subs" | "list" => {
            let s = match vpn::load_subs() {
                Ok(s) => s,
                Err(e) => return err_code(Err(e)),
            };
            if s.list.is_empty() {
                println!("{}", t!("подписок нет — upd vpn add"));
            }
            for (i, x) in s.list.iter().enumerate() {
                let mark = if x.id == s.active { "●" } else { " " };
                let info = x.info.as_ref().map(sub_info).unwrap_or_default();
                let err = if x.error.is_empty() { String::new() } else { format!(" ⚠ {}", x.error) };
                println!("{}", t!("{} {}. {} ({}) · серверов {} · обновлена {}{6}{7}", mark, i + 1, x.name, vpn::mask_url(&x.url), x.nodes, fmt_ago(x.updated), info, err));
            }
            0
        }
        "use" | "del" => {
            let Some(target) = arg.and_then(vpn::SubRef::parse) else {
                println!("{}", t!("укажи номер подписки (upd vpn subs)"));
                return 2;
            };
            let r = if sub == "use" { vpn::use_sub(&target) } else { vpn::delete_sub(&target) };
            match r {
                Ok(name) => {
                    println!("{}: «{name}»", if sub == "use" { t!("активна") } else { t!("удалена") });
                    if sub == "del" {
                        match vpn::load_subs() {
                            Ok(subs) if subs.list.is_empty() => {
                                let mut errors = Vec::new();
                                if vpn::service_active() {
                                    if let Err(e) = vpn::stop() {
                                        errors.push(t!("VPN не удалось остановить: {0}", e));
                                    }
                                }
                                if let Err(error) = vpn::sysproxy(&c, user.as_ref()) { errors.push(format!("VPN subscription removed, but user proxy failed: {error}")); }
                                if c.vpn_autostart {
                                    if let Err(e) = vpn::autostart(false) {
                                        errors.push(t!("автозапуск VPN не удалось выключить: {0}", e));
                                    }
                                }
                                if !errors.is_empty() {
                                    eprintln!("{}", t!("подписка уже удалена; {}", errors.join("; ")));
                                    return 1;
                                }
                                println!("{}", t!("последняя подписка удалена: VPN остановлен, автозапуск выключен до добавления подписки"));
                                return 0;
                            }
                            Ok(_) => {}
                            Err(e) => {
                                eprintln!("{}", t!("подписка уже удалена, но список не удалось прочитать: {0}", e));
                                return 1;
                            }
                        }
                    }
                    if vpn::service_active() {
                        match vpn::load_subs() {
                            Ok(subs) if !subs.list.is_empty() => return apply(&c),
                            Ok(_) => {}
                            Err(e) => return err_code(Err(e)),
                        }
                    }
                    0
                }
                Err(e) => err_code(Err(e)),
            }
        }
        "update" => {
            match vpn::update_subs(&c, &stdlog, true) {
                Ok(_) => apply(&c),
                Err(e) => err_code(Err(e)),
            }
        }
        "start" => {
            // ядро, геофайлы и проверка конфига — здесь, с выводом на экран; иначе это молча
            // делает ExecStartPre, а `systemctl start` висит без единой строки до конца загрузки
            // пока FlClash работает, качаем через него — напрямую GitHub бывает недоступен
            if let Err(e) = vpn::fetch_missing(&c, &stdlog) {
                return err_code(Err(e));
            }
            while let Some(w) = vpn::conflict(&c) {
                print!("{}", t!("⚠ {0}.\nУстрани конфликт и нажми Enter (Ctrl+C — отмена) ", w));
                let _ = std::io::Write::flush(&mut std::io::stdout());
                let mut s = String::new();
                if std::io::stdin().read_line(&mut s).unwrap_or(0) == 0 {
                    return err_code(Err(w));
                }
            }
            if let Err(e) = vpn::prepare(&c, &stdlog) {
                return err_code(Err(e));
            }
            println!("{}", t!("запускаю службу..."));
            let mut r = vpn::start(&c);
            if r.is_ok() {
                std::thread::sleep(std::time::Duration::from_secs(2));
                if let Err(error) = vpn::sysproxy(&c, user.as_ref()) { r = Err(format!("VPN started, but user proxy failed: {error}; retry: upd vpn restart")); }
                println!("VPN: {}", vpn_line());
            }
            err_code(r)
        }
        "stop" => {
            let r = vpn::stop();
            let proxy = vpn::sysproxy(&c, user.as_ref()).map(|_| ()).map_err(|error| format!("VPN stopped, but user proxy failed: {error}; retry: upd vpn stop"));
            err_code(r.and(proxy))
        }
        "restart" => err_code(vpn::restart(&c).and_then(|_| vpn::sysproxy(&c, user.as_ref()).map(|_| ()).map_err(|error| format!("VPN restarted, but user proxy failed: {error}; retry: upd vpn restart")))),
        "tun" | "proxy" => {
            c.vpn_tun = sub == "tun";
            if let Err(e) = c.save() {
                return err_code(Err(e.to_string()));
            }
            apply(&c)
        }
        "rule" | "global" | "direct" => {
            c.vpn_mode = ["rule", "global", "direct"].iter().position(|m| *m == sub).unwrap_or(0) as u8;
            if let Err(e) = c.save() {
                return err_code(Err(e.to_string()));
            }
            err_code(vpn::apply_saved_mode(&c, user.as_ref(), &stdlog))
        }
        "servers" => {
            let s = vpn::snapshot();
            if !s.running {
                println!("{}", t!("VPN не запущен"));
                return 1;
            }
            for g in &s.groups {
                println!("[{}] {} → {}", g.kind, vpn::label(&g.name), vpn::label(&g.now));
                for p in &g.all {
                    let d = s.delay.get(p).map(|d| if *d == 0 { "✗".to_string() } else { t!("{0} мс", d) }).unwrap_or_default();
                    println!("   {} {:<40} {d}", if *p == g.now { "●" } else { " " }, vpn::label(p));
                }
            }
            0
        }
        "core" => match arg.unwrap_or("check") {
            a @ ("update" | "reinstall") => {
                let r = vpn::core_install(&c, &stdlog, a == "reinstall").and_then(|changed| {
                    let active = vpn::service_active();
                    let restart = if changed && active { vpn::restart(&c) } else { Ok(()) };
                    core_update_exit(changed, active, restart)
                });
                err_code(r)
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
            print!("{}", t!(VPN_USAGE));
            2
        }
    }
}

fn print_status(b: &dyn Backend) {
    let s = gather_status(b);
    let u = &s.upd;
    let mut upd = t!("пакетов {}", u.list.len());
    if u.downloaded && !u.list.is_empty() {
        upd += t!(" (скачаны)");
    }
    if !u.list.is_empty() {
        upd += &match u.download_size {
            Some(bytes) => format!(" ({})", fmt_bytes(bytes)),
            None => t!(" (размер неизвестен)").into(),
        };
    }
    if !u.flatpak.is_empty() {
        upd += &format!(", Flatpak {}", u.flatpak.len());
    }
    if !u.flatpak_error.is_empty() {
        upd += &t!(", ошибка проверки Flatpak: {}", u.flatpak_error);
    }
    if !u.firmware_error.is_empty() {
        upd += &t!(", ошибка проверки прошивок: {}", u.firmware_error);
    }
    if !u.firmware.is_empty() {
        upd += &t!(", прошивок {}", u.firmware.len());
    }
    if !u.error.is_empty() {
        upd += &format!(" — {}", u.error);
    }
    if !u.skipped.is_empty() {
        upd += &format!(" — {}", u.skipped);
    }
    println!("{}", t!("Система:      {}", s.name));
    println!("{}", t!("Обновления:   {1} · проверка {}", fmt_ago(u.checked), upd));
    if !u.news.is_empty() {
        println!("{}", t!("Новости Arch: {} непрочитанных — upd list", u.news.len()));
    }
    println!("{}", t!("Последнее:    {}", fmt_time(s.last_tx)));
    println!("{}", t!("Перезагрузка: {}", if s.reboot { t!("НУЖНА") } else { t!("не нужна") }));
    let rs = &s.restart;
    if !rs.services.is_empty() || !rs.critical.is_empty() || !rs.unknown.is_empty() {
        println!("{}", t!("Перезапуск:   служб {} (upd restart), требуют перезагрузки {}, cgroup не распознан для {} процессов", rs.services.len(), rs.critical.len(), rs.unknown.len()));
    }
    println!("{}", t!("Сеть:         {}{}{}", s.net_label, if s.metered { t!(" · лимитная") } else { "" }, if s.battery { t!(" · от батареи") } else { "" }));
    if s.managed {
        let first = s.pinned.first().map(|p| host_of(p).to_string()).unwrap_or_else(|| "—".into());
        println!("{}", t!("Зеркала:      закреплено {}, первое {} · замер {}", s.pinned.len(), first, fmt_ago(s.mir.checked)));
        for h in &s.mir.hints {
            println!("              💡 {h}");
        }
    } else {
        println!("{}", t!("Зеркала:      {}", s.mirror_note));
    }
    println!("VPN:          {}", vpn_line());
    println!("{}", t!("Снапшоты:     {}", s.snapshots));
    println!("{}", t!("Обслуживание: новых настроек {} · сирот {} · кэш {} · свободно {} · упавших служб {}", s.pending.len(), s.orphans, fmt_bytes(s.cache), s.free.map(fmt_bytes).unwrap_or_else(|| t!("неизвестно").into()), s.failed));
    println!("{}", t!("Автоматика:   обновления {}, слежение за сетью {}", unit_label(&s.auto_timer), unit_label(&s.net_timer)));
}

// ---------- уведомления (от имени пользователя) ----------

fn notify_once(seen: &mut BTreeMap<String, String>, key: &str, val: String, title: &str, body: &str) -> Result<(), String> {
    if seen.get(key) == Some(&val) { return Ok(()); }
    let mut command = std::process::Command::new("notify-send");
    command.args(["-a", "upd", "-i", "system-software-update", title, body]);
    let mut policy = CapturePolicy::background(Some(256)); policy.stderr_max = 8192;
    let output = capture_with_policy(&mut command, policy).map_err(String::from)?;
    if !output.status.success() { return Err(format!("notify-send exited with {}", output.status)); }
    seen.insert(key.into(), val); Ok(())
}

fn cmd_notify() {
    if !have("notify-send") || upd::summary::applet_running() {
        // апплет COSMIC сам показывает уведомления, с кнопками действий
        return;
    }
    let home = std::env::var("HOME").unwrap_or_default();
    let cache = format!("{home}/.cache/upd-notified.json");
    let mut seen: BTreeMap<String, String> = fs::read(&cache).ok().and_then(|b| serde_json::from_slice(&b).ok()).unwrap_or_default();
    let mut send = |key: &str, val: String, title: &str, body: &str| {
        if let Err(error) = notify_once(&mut seen, key, val, title, body) { eprintln!("upd notify: {error}"); }
    };
    let u: UpdState = load_json("updates.json");
    let total = u.list.len() + u.flatpak.len();
    if total > 0 {
        let h = sha1_smol::Sha1::from((u.list.join("\n") + &u.flatpak.join("\n")).as_bytes()).digest().to_string();
        let body = if u.downloaded { t!("Уже скачаны. Запусти в терминале: upd") } else { t!("Запусти в терминале: upd") };
        send("updates", h, &t!("Доступно обновлений: {0}", total), body);
    }
    if !u.news.is_empty() {
        let h = u.news.iter().map(|n| n.title.clone()).collect::<Vec<_>>().join("|");
        send("news", h, t!("Новости Arch перед обновлением"), &u.news.iter().map(|n| n.title.clone()).collect::<Vec<_>>().join("\n"));
    }
    if !u.firmware.is_empty() {
        send("firmware", u.firmware.join("|"), t!("Доступны прошивки"), &u.firmware.join("\n"));
    }
    if reboot_needed() {
        let boot = fs::read_to_string("/proc/sys/kernel/random/boot_id").unwrap_or_default();
        send("reboot", boot, t!("Нужна перезагрузка"), t!("Обновилось ядро"));
    }
    let v = vpn::load_state();
    if !v.event.is_empty() {
        send("vpn-core", v.event_time.to_string(), "VPN", &v.event);
    }
    for sp in v.subs.iter().filter(|s| s.active) {
        if let Some(i) = &sp.info {
            if i.expire > 0 && i.expire.saturating_sub(now()) < 3 * 86400 {
                send("vpn-expire", i.expire.to_string(), t!("Подписка VPN скоро закончится"), &t!("«{}» — до {}", sp.name, fmt_time(i.expire)));
            }
            if i.nearly_exhausted() {
                send("vpn-traffic", i.total.to_string(), t!("Трафик VPN почти исчерпан"), &t!("«{}»: {} из {}", sp.name, vpn::fmt_bytes_wide(i.used()), fmt_bytes(i.total)));
            }
        }
    }
    let m = load_mirror_state();
    if !m.event.is_empty() {
        send("mirrors", m.event_time.to_string(), t!("Зеркала подобраны заново"), &m.event);
    }
    let _ = fs::create_dir_all(format!("{home}/.cache"));
    let _ = fs::write(&cache, serde_json::to_vec(&seen).unwrap_or_default());
}

// ---------- установка ----------

const LOCAL_BIN: &str = "/usr/local/bin/upd";
const PKG_BIN: &str = "/usr/bin/upd";
const CRON_PATH: &str = "/etc/cron.d/upd";
const GARUDA_CONF: &str = "/etc/garuda/garuda-update/config";
const GARUDA_MARK: &str = "# upd: mirrors are managed by upd, garuda-update must not overwrite them";
/// Метка до 0.2.5 — узнаём и заменяем
const GARUDA_MARK_OLD: &str = "# upd: зеркалами управляет upd — garuda-update их не перезаписывает";
const PACKAGE_SCRIPT_ENV: &str = "UPD_PACKAGE_SCRIPT";

/// Пакетный layout разрешён только для вызова канонического /usr/bin/upd из наших хуков.
fn package_script_context(exe: &Path) -> bool {
    std::env::var(PACKAGE_SCRIPT_ENV).map(|v| v == "1").unwrap_or(false)
        && fs::canonicalize(PKG_BIN).map(|path| path.as_path() == exe).unwrap_or(false)
}

/// Куда кладутся системные файлы: ручная установка (upd install) — в /etc; пакет — в /usr/lib и /usr/share.
struct Layout {
    bin: &'static str,
    units: &'static str,
    user_units: &'static str,
    pacman_hook: &'static str,
    apt_hook: &'static str,
    nm: &'static str,
    polkit: &'static str,
}

const MANUAL: Layout = Layout {
    bin: LOCAL_BIN,
    units: "/etc/systemd/system",
    user_units: "/etc/systemd/user",
    pacman_hook: "/etc/pacman.d/hooks/zz-upd.hook",
    apt_hook: "/etc/apt/apt.conf.d/99upd",
    nm: "/etc/NetworkManager/dispatcher.d/90-upd",
    // действия polkit читаются только из /usr/share
    polkit: POLKIT_POLICY,
};

const PACKAGE: Layout = Layout {
    bin: PKG_BIN,
    units: "/usr/lib/systemd/system",
    user_units: "/usr/lib/systemd/user",
    pacman_hook: "/usr/share/libalpm/hooks/zz-upd.hook",
    apt_hook: "/etc/apt/apt.conf.d/99upd",
    nm: "/usr/lib/NetworkManager/dispatcher.d/90-upd",
    polkit: POLKIT_POLICY,
};

const POLKIT_POLICY: &str = "/usr/share/polkit-1/actions/io.github.upd.policy";

/// Интерфейс COSMIC при ручной установке: ставится, если `upd-cosmic` лежит рядом с upd (dist/).
const GUI_BIN: &str = "/usr/local/bin/upd-cosmic";
const GUI_FILES: [(&str, &str); 4] = [
    ("/usr/local/share/applications/io.github.upd.desktop", include_str!("../cosmic/res/io.github.upd.desktop")),
    ("/usr/local/share/applications/io.github.upd.Applet.desktop", include_str!("../cosmic/res/io.github.upd.Applet.desktop")),
    ("/usr/local/share/icons/hicolor/scalable/apps/io.github.upd.svg", include_str!("../cosmic/res/icons/io.github.upd.svg")),
    ("/usr/local/share/icons/hicolor/symbolic/apps/io.github.upd-symbolic.svg", include_str!("../cosmic/res/icons/io.github.upd-symbolic.svg")),
];

/// Бинарник интерфейса рядом с устанавливаемым upd.
fn gui_source(exe: &Path) -> Option<std::path::PathBuf> {
    let dir = exe.parent()?;
    ["upd-cosmic", "upd-cosmic-linux-amd64"].iter().map(|n| dir.join(n)).find(|p| p.is_file())
}

fn install_binary(src: &Path, target: &Path) -> Result<(), String> {
    let bytes = fs::read(src).map_err(|error| format!("{}: {error}", src.display()))?;
    atomic_write(target, &bytes, 0o755).map_err(|error| format!("{}: {error}", target.display()))
}

fn install_gui(src: &Path) -> Result<(), String> {
    install_binary(src, Path::new(GUI_BIN))?;
    for (path, body) in GUI_FILES {
        write_file(path, body, 0o644)?;
    }
    Ok(())
}

fn remove_gui() {
    let _ = fs::remove_file(GUI_BIN);
    for (path, _) in GUI_FILES {
        let _ = fs::remove_file(path);
    }
}

fn system_units(bin: &str, watch: Option<&str>) -> Vec<(&'static str, String)> {
    let mut u = vec![
        (
            "upd-auto.service",
            format!("[Unit]\nDescription=upd: network change, mirrors, VPN, update check and prefetch\nWants=network-online.target\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} auto\nNice=10\nIOSchedulingClass=idle\n"),
        ),
        (
            "upd-auto.timer",
            "[Unit]\nDescription=upd: periodic update check\n\n[Timer]\nOnBootSec=10min\nOnUnitActiveSec=6h\nRandomizedDelaySec=10min\nPersistent=true\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            "upd-net.service",
            format!("[Unit]\nDescription=upd: mirror selection on network change\nAfter=network-online.target\n\n[Service]\nType=oneshot\nExecStart={bin} net\nNice=10\n"),
        ),
        (
            "upd-net.timer",
            "[Unit]\nDescription=upd: network change check (cheap, no network requests)\n\n[Timer]\nOnBootSec=2min\nOnUnitActiveSec=15min\n\n[Install]\nWantedBy=timers.target\n".to_string(),
        ),
        (
            vpn::SERVICE,
            format!(
                "[Unit]\nDescription=upd: VPN (mihomo core)\nWants=network-online.target\nAfter=network-online.target\n\
                 StartLimitIntervalSec=10min\nStartLimitBurst=5\n\n[Service]\nType=simple\n\
                 ExecStartPre={bin} vpn prepare\nExecStart=/var/lib/upd/vpn/bin/mihomo -d /var/lib/upd/vpn -f /var/lib/upd/vpn/config.yaml\n\
                 Restart=on-failure\nRestartSec=5\nTimeoutStartSec=5min\nLimitNOFILE=1048576\n\
                 UMask=0077\nNoNewPrivileges=yes\n\
                 CapabilityBoundingSet=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
                 AmbientCapabilities=CAP_NET_ADMIN CAP_NET_RAW CAP_NET_BIND_SERVICE\n\
                 ProtectSystem=strict\nReadWritePaths=-/var/lib/upd/vpn\nProtectHome=yes\nPrivateTmp=yes\n\
                 ProtectKernelModules=yes\nProtectControlGroups=yes\n\n[Install]\nWantedBy=multi-user.target\n"
            ),
        ),
    ];
    // помощник графического интерфейса: запускается по обращению к сокету, права проверяет через polkit
    u.push((
        "upd-helper.socket",
        "[Unit]\nDescription=upd: helper socket for the graphical interface\n\n[Socket]\nListenStream=/run/upd/helper.sock\nSocketMode=0666\nDirectoryMode=0755\nRemoveOnStop=yes\n\n[Install]\nWantedBy=sockets.target\n".to_string(),
    ));
    u.push((
        "upd-helper.service",
        format!("[Unit]\nDescription=upd: privileged helper for the graphical interface\nRequires=upd-helper.socket\nAfter=upd-helper.socket\n\n[Service]\nType=simple\nExecStart={bin} helper\nKillMode=process\n"),
    ));
    if let Some(w) = watch {
        u.push((
            "upd-mirrors.path",
            format!("[Unit]\nDescription=upd: restore pinned mirrors if the list is overwritten\n\n[Path]\nPathChanged={w}\nUnit=upd-mirrors-apply.service\n\n[Install]\nWantedBy=paths.target\n"),
        ));
        u.push((
            "upd-mirrors-apply.service",
            format!("[Unit]\nDescription=upd: put pinned mirrors back on top\n\n[Service]\nType=oneshot\nExecStart={bin} mirrors apply\n"),
        ));
    }
    u
}

fn user_units(bin: &str) -> Vec<(&'static str, String)> {
    vec![
        ("upd-notify.service", format!("[Unit]\nDescription=upd: notifications\n\n[Service]\nType=oneshot\nExecStart={bin} notify\n")),
        ("upd-notify.timer", "[Unit]\nDescription=upd: notifications\n\n[Timer]\nOnActiveSec=3min\nOnUnitActiveSec=30min\n\n[Install]\nWantedBy=timers.target\n".to_string()),
    ]
}

/// Действия polkit для помощника: чтение состояния без пароля в активном сеансе, изменения — с паролем администратора.
fn polkit_policy() -> String {
    let msg = |en: &str, tr: &[(&str, &str)]| {
        let mut s = format!("    <message>{en}</message>\n");
        for (l, m) in tr {
            s += &format!("    <message xml:lang=\"{l}\">{m}</message>\n");
        }
        s
    };
    let actions = [
        (
            helper::ACTION_STATUS,
            "yes",
            msg("Read the update and VPN status", &[("ru", "Просмотр состояния обновлений и VPN"), ("de", "Status von Updates und VPN lesen"), ("it", "Leggere lo stato di aggiornamenti e VPN"), ("zh", "读取更新和 VPN 状态"), ("ar", "قراءة حالة التحديثات والشبكة الافتراضية")]),
        ),
        (
            helper::ACTION_CHECK,
            "yes",
            msg("Check for updates", &[("ru", "Проверка обновлений"), ("de", "Nach Updates suchen"), ("it", "Cercare aggiornamenti"), ("zh", "检查更新"), ("ar", "البحث عن تحديثات")]),
        ),
        (
            helper::ACTION_VPN,
            "yes",
            msg("Turn the VPN on or off and choose a server", &[("ru", "Включение и выключение VPN, выбор сервера"), ("de", "VPN ein- oder ausschalten und Server wählen"), ("it", "Attivare o disattivare la VPN e scegliere il server"), ("zh", "开关 VPN 并选择服务器"), ("ar", "تشغيل الشبكة الافتراضية أو إيقافها واختيار الخادم")]),
        ),
        (
            helper::ACTION_MANAGE,
            "auth_admin_keep",
            msg("Install updates and manage mirrors and VPN", &[("ru", "Установка обновлений, управление зеркалами и VPN"), ("de", "Updates installieren sowie Spiegelserver und VPN verwalten"), ("it", "Installare aggiornamenti e gestire mirror e VPN"), ("zh", "安装更新并管理镜像和 VPN"), ("ar", "تثبيت التحديثات وإدارة المرايا والشبكة الافتراضية")]),
        ),
    ];
    let mut s = String::from("<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!DOCTYPE policyconfig PUBLIC \"-//freedesktop//DTD PolicyKit Policy Configuration 1.0//EN\"\n \"http://www.freedesktop.org/standards/PolicyKit/1/policyconfig.dtd\">\n<!-- Managed by upd -->\n<policyconfig>\n  <vendor>upd</vendor>\n  <icon_name>system-software-update</icon_name>\n");
    for (id, active, message) in actions {
        // вне активного сеанса (ssh, другой пользователь за экраном) — только с паролем администратора
        let other = if active == "yes" { "auth_admin" } else { active.trim_end_matches("_keep") };
        s += &format!("  <action id=\"{id}\">\n{message}    <defaults>\n      <allow_any>{other}</allow_any>\n      <allow_inactive>{other}</allow_inactive>\n      <allow_active>{active}</allow_active>\n    </defaults>\n  </action>\n");
    }
    s + "</policyconfig>\n"
}

fn pacman_hook(bin: &str) -> String {
    format!("# upd: reconcile the list of available updates after any transaction (offline)\n[Trigger]\nOperation = Install\nOperation = Upgrade\nOperation = Remove\nType = Package\nTarget = *\n\n[Action]\nDescription = upd: reconciling the update list...\nWhen = PostTransaction\nExec = {bin} reconcile\n")
}

fn apt_hook(bin: &str) -> String {
    format!("// upd: reconcile the list of available updates after any install (offline)\nDPkg::Post-Invoke {{ \"{bin} reconcile >/dev/null 2>&1 || true\"; }};\n")
}

fn nm_dispatcher() -> &'static str {
    "#!/bin/sh\n# upd: on network change, check whether other mirrors should be selected\ncase \"$2\" in\n  up|down|vpn-up|vpn-down|connectivity-change) systemctl start --no-block upd-net.service ;;\nesac\n"
}

/// garuda-update, запущенный напрямую, пересобирает зеркала через rate-mirrors — запрещаем, пока стоит upd.
fn garuda_skip_mirrors(on: bool) -> Option<String> {
    let text = fs::read_to_string(GARUDA_CONF).ok()?;
    let ours = text.contains(GARUDA_MARK) || text.contains(GARUDA_MARK_OLD);
    let cleaned: Vec<&str> = text.lines().filter(|l| *l != GARUDA_MARK && *l != GARUDA_MARK_OLD && !(l.trim() == "SKIP_MIRRORLIST=1" && ours)).collect();
    let mut body = cleaned.join("\n") + "\n";
    if on {
        body += &format!("{GARUDA_MARK}\nSKIP_MIRRORLIST=1\n");
    }
    (body != text).then(|| {
        let _ = atomic_write(Path::new(GARUDA_CONF), body.as_bytes(), 0o644);
        GARUDA_CONF.to_string()
    })
}

// Garuda: в стандартных настройках fish/bash есть alias upd → garuda-update, он перехватывает команду.
// Пользовательский конфиг подключает их строкой source — сразу после неё снимаем алиас.
const UNALIAS_MARK: &str = "# upd: drop the Garuda alias upd -> garuda-update";
/// Метка до 0.2.5 — узнаём и заменяем
const UNALIAS_MARK_OLD: &str = "# upd: снять алиас Garuda upd → garuda-update";
const GARUDA_SHELLS: [(&str, &str, &str); 2] = [
    (".config/fish/config.fish", "/usr/share/garuda/garuda-fish-config/config.fish", "functions -e upd"),
    (".bashrc", "/usr/share/garuda/garuda-bash-config/bashrc", "unalias upd 2>/dev/null"),
];

fn garuda_alias_defined() -> bool {
    GARUDA_SHELLS
        .iter()
        .any(|(_, sys, _)| fs::read_to_string(sys).map(|t| t.lines().any(|l| l.trim_start().starts_with("alias upd"))).unwrap_or(false))
}

/// Конфиги обычных пользователей, которые подключают настройки Garuda: (путь, текст, строка-снятие).
/// Ссылки и чужие файлы пропускаем — пишем от root в домашние каталоги.
fn garuda_user_configs() -> Vec<(std::path::PathBuf, String, &'static str)> {
    use std::os::unix::fs::MetadataExt;
    let passwd = fs::read_to_string("/etc/passwd").unwrap_or_default();
    let mut out = vec![];
    for line in passwd.lines() {
        let p: Vec<&str> = line.split(':').collect();
        let Some(uid) = p.get(2).and_then(|u| u.parse::<u32>().ok()) else { continue };
        if !(1000..60000).contains(&uid) || p.len() < 6 {
            continue;
        }
        for (rel, sys, cmd) in GARUDA_SHELLS {
            let path = Path::new(p[5]).join(rel);
            let Ok(md) = fs::symlink_metadata(&path) else { continue };
            if !md.is_file() || md.uid() != uid {
                continue;
            }
            let Ok(text) = fs::read_to_string(&path) else { continue };
            if text.lines().any(|l| l.trim_start().starts_with("source") && l.contains(sys)) {
                out.push((path, text, cmd));
            }
        }
    }
    out
}

fn has_unalias(text: &str, cmd: &str) -> bool {
    let key = cmd.split(" 2>").next().unwrap_or(cmd);
    text.lines().any(|l| l.trim_start().starts_with(key))
}

/// on: вставить снятие алиаса туда, где его ещё нет; off: убрать только наши строки. Возвращает изменённые файлы.
fn garuda_unalias(on: bool) -> Vec<String> {
    let mut changed = vec![];
    for (path, text, cmd) in garuda_user_configs() {
        let body = unalias_body(&text, cmd, on);
        // fs::write, а не atomic_write: файл остаётся тем же, с владельцем-пользователем и его правами
        if body != text && fs::write(&path, &body).is_ok() {
            changed.push(path.display().to_string());
        }
    }
    changed
}

fn unalias_body(text: &str, cmd: &str, on: bool) -> String {
    // свои строки (с новой или прежней меткой) переписываются заново; снятие алиаса, добавленное вручную, не трогаем
    let ours = |l: &str| l.ends_with(UNALIAS_MARK) || l.ends_with(UNALIAS_MARK_OLD);
    let others: String = text.lines().filter(|l| !ours(l)).map(|l| format!("{l}\n")).collect();
    let insert = on && !has_unalias(&others, cmd);
    let mut body = String::new();
    for l in text.lines().filter(|l| !ours(l)) {
        body += l;
        body.push('\n');
        if insert && l.trim_start().starts_with("source") && l.contains("/usr/share/garuda/") {
            body += &format!("{cmd}  {UNALIAS_MARK}\n");
        }
    }
    if !text.ends_with('\n') {
        body.pop();
    }
    body
}

fn garuda_unalias_install(pkg: bool, bin: &str) {
    use std::io::IsTerminal;
    if !garuda_alias_defined() {
        return;
    }
    let todo: Vec<String> = garuda_user_configs()
        .into_iter()
        .filter(|(_, text, cmd)| !has_unalias(text, cmd))
        .map(|(p, _, _)| p.display().to_string())
        .collect();
    if todo.is_empty() {
        return;
    }
    // из пакетного менеджера не спрашиваем и в домашние каталоги не пишем
    let ask = !pkg && std::io::stdin().is_terminal();
    println!("{}", t!("\n\x1b[33m⚠ В Garuda команда `upd` занята алиасом на garuda-update:\x1b[0m {}", todo.join(", ")));
    if ask && confirm(t!("Снять алиас, чтобы `upd` запускал эту утилиту?"), true) {
        for f in garuda_unalias(true) {
            println!("{}", t!("✓ алиас снят: {0}", f));
        }
        println!("{}", t!("   в уже открытых терминалах: exec fish (или exec bash), новые — сразу"));
        return;
    }
    println!("{}", t!("   сними сам: fish — `functions -e upd`, bash — `unalias upd` в строке после source …garuda…, или запускай полным путём: {0}", bin));
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
        write_file(&format!("{root}{}/{n}", l.units), &format!("# Managed by upd\n{body}"), 0o644)?;
        done.push(n.to_string());
    }
    for (n, body) in user_units(l.bin) {
        write_file(&format!("{root}{}/{n}", l.user_units), &format!("# Managed by upd\n{body}"), 0o644)?;
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
    if target != "host" || Path::new("/usr/share/polkit-1").is_dir() {
        write_file(&format!("{root}{}", l.polkit), &polkit_policy(), 0o644)?;
    }
    Ok(done)
}

/// Для сборки пакетов: upd gen-files <каталог> <arch|deb|rpm>
fn cmd_gen_files(b: &dyn Backend, pos: &[String]) -> i32 {
    let (Some(root), Some(target)) = (pos.first(), pos.get(1)) else {
        eprintln!("{}", t!("upd gen-files <каталог> <arch|deb|rpm>"));
        return 2;
    };
    match write_system_files(root.trim_end_matches('/'), &PACKAGE, target, b) {
        Ok(_) => 0,
        Err(e) => err_code(Err(e)),
    }
}

fn owned_unit_files(dir: &str, names: &[&str]) -> Result<Vec<(String, std::path::PathBuf)>, String> {
    let mut files = vec![];
    for name in names {
        let path = Path::new(dir).join(name);
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => continue,
            Err(e) => return Err(format!("{}: {e}", path.display())),
        };
        if !metadata.file_type().is_file() {
            return Err(t!("{}: не обычный unit-файл, миграция остановлена", path.display()));
        }
        let body = fs::read_to_string(&path).map_err(|e| format!("{}: {e}", path.display()))?;
        let owned = body.lines().any(|line| line.trim() == "# Managed by upd")
            || body.lines().any(|line| line.trim_start().starts_with("Description=upd:"));
        if !owned {
            return Err(t!("{}: нет маркера upd, файл оставлен и миграция остановлена", path.display()));
        }
        files.push(((*name).to_string(), path));
    }
    Ok(files)
}

fn disable_and_remove_units(dir: &str, files: &[(String, std::path::PathBuf)], global: bool) -> Result<(), String> {
    if files.is_empty() {
        return Ok(());
    }
    let enabled_names: Vec<&str> = files
        .iter()
        .filter_map(|(name, _)| (name.ends_with(".timer") || name.ends_with(".path") || name.as_str() == vpn::SERVICE).then_some(name.as_str()))
        .collect();
    if systemd() {
        if !enabled_names.is_empty() {
            let mut args = if global { vec!["--global", "disable"] } else { vec!["disable"] };
            args.extend(enabled_names);
            run(true, &[], "systemctl", &args).map_err(|e| t!("{0}: не удалось отключить unit-файлы upd: {1}", dir, e))?;
        }
    }
    for (name, path) in files {
        fs::remove_file(path).map_err(|e| t!("{}: не удалось удалить {1}: {2}", path.display(), name, e))?;
    }
    Ok(())
}

fn remove_manual_files() -> Result<(), String> {
    // Имена берутся из генераторов, а не из префикса: сторонние upd-* units не трогаем.
    let system_names: Vec<&str> = system_units(MANUAL.bin, Some("")).into_iter().map(|(name, _)| name).collect();
    let user_names: Vec<&str> = user_units(MANUAL.bin).into_iter().map(|(name, _)| name).collect();
    let system_files = owned_unit_files(MANUAL.units, &system_names)?;
    let user_files = owned_unit_files(MANUAL.user_units, &user_names)?;

    // Сначала disable: иначе в *.wants останутся ссылки на удалённые файлы.
    disable_and_remove_units(MANUAL.units, &system_files, false)?;
    disable_and_remove_units(MANUAL.user_units, &user_files, true)?;
    fs::remove_file(LOCAL_BIN).or_else(|e| if e.kind() == std::io::ErrorKind::NotFound { Ok(()) } else { Err(e) })
        .map_err(|e| format!("{LOCAL_BIN}: {e}"))?;
    let _ = fs::remove_file(MANUAL.pacman_hook);
    let _ = fs::remove_file(MANUAL.nm);
    // политику polkit удаляем, только если её записал upd (при пакетной установке она принадлежит пакету)
    if fs::read_to_string(MANUAL.polkit).is_ok_and(|t| t.contains("<!-- Managed by upd -->")) && !Path::new(PKG_BIN).exists() {
        let _ = fs::remove_file(MANUAL.polkit);
    }
    Ok(())
}

fn logged_in_users() -> Vec<String> {
    let (s, _) = out("loginctl", &["list-users", "--no-legend"]);
    s.lines()
        .filter_map(|l| {
            let mut f = l.split_whitespace();
            let uid: u32 = f.next()?.parse().ok()?;
            (uid >= 1000).then(|| f.next().map(String::from)).flatten()
        })
        .collect()
}

fn enable_units(b: &dyn Backend, c: &Config, ok: &dyn Fn(String)) -> Result<(), String> {
    let _ = run(true, &[], "systemctl", &["daemon-reload"]);
    let mut enable = vec!["upd-auto.timer", "upd-net.timer", "upd-helper.socket"];
    if b.mirrors_managed() && b.watch_path().is_some() {
        enable.push("upd-mirrors.path");
    }
    let mut args = vec!["enable", "--now"];
    args.extend(enable.iter().copied());
    run(false, &[], "systemctl", &args)?;
    // Enabling an already active socket does not reload the helper executable.
    // Stop the old protocol process after replacing the installed binaries.
    if helper::available() && helper::running_for_install()? {
        println!("{}", t!("Помощник выполняет операцию; после завершения перезапустите его: sudo systemctl restart upd-helper.service"));
    } else {
        run(true, &[], "systemctl", &["try-restart", "upd-helper.service"])?;
    }
    let _ = run(true, &[], "systemctl", &["--global", "enable", "upd-notify.timer"]);
    // --global действует со следующего входа; тем, кто уже вошёл, перечитываем юниты и запускаем таймер сразу
    for user in logged_in_users() {
        let m = format!("{user}@");
        let _ = run(true, &[], "systemctl", &["--user", "-M", &m, "daemon-reload"]);
        let _ = run(true, &[], "systemctl", &["--user", "-M", &m, "start", "upd-notify.timer"]);
    }
    ok(t!("службы: {}", enable.join(", ")));
    // работающий VPN после обновления upd подхватывает новый unit и конфиг (API через сокет); неработающий не трогаем
    if vpn::service_active() && run(true, &[], "systemctl", &["try-restart", "--no-block", vpn::SERVICE]).is_ok() {
        ok(t!("VPN: служба перезапускается с новыми настройками").into());
    }
    ok(t!("уведомления: upd-notify.timer (для всех пользователей, после входа)").into());
    match vpn::load_subs() {
        Ok(subs) if !subs.list.is_empty() && c.vpn_autostart => {
            let _ = vpn::autostart(true);
            ok(t!("VPN: автозапуск включён").into());
        }
        Ok(_) => ok(t!("VPN: добавь подписку — upd → VPN (или upd vpn add), дальше он будет стартовать сам").into()),
        Err(e) => println!("{}", t!("⚠ VPN: не удалось прочитать подписки: {0}", e)),
    }
    Ok(())
}

/// Выбор языка из списка; Enter — язык системы. None — ввод закрыт.
fn choose_lang() -> Option<i18n::Lang> {
    let def = i18n::from_env();
    println!();
    for (i, l) in i18n::ALL.iter().enumerate() {
        println!("  {}) {}{}", i + 1, l.name(), if *l == def { " *" } else { "" });
    }
    loop {
        print!("{} ", t!("Язык интерфейса (номер, Enter — {}):", def.name()));
        let _ = std::io::Write::flush(&mut std::io::stdout());
        let mut s = String::new();
        if std::io::stdin().read_line(&mut s).ok()? == 0 {
            return None;
        }
        let s = s.trim();
        if s.is_empty() {
            return Some(def);
        }
        if let Some(l) = s.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| i18n::ALL.get(i)).copied().or_else(|| i18n::Lang::from_code(s)) {
            return Some(l);
        }
    }
}

fn cmd_lang(c: &Config, pos: &[String]) -> i32 {
    let Some(code) = pos.first() else {
        println!("{}", t!("язык: {} (настройка lang = {})", i18n::cur().name(), c.lang));
        for l in i18n::ALL {
            println!("  {}  {}", l.code(), l.name());
        }
        println!("{}", t!("сменить: upd lang КОД (auto — по локали системы)"));
        return 0;
    };
    let mut c = c.clone();
    c.lang = match i18n::Lang::from_code(code) {
        Some(l) => l.code().into(),
        None if code == "auto" => "auto".into(),
        None => {
            eprintln!("{}", t!("неизвестный язык: {}", code));
            return 2;
        }
    };
    i18n::set(i18n::resolve(&c.lang));
    match c.save() {
        Ok(()) => {
            println!("{}", t!("язык: {}", i18n::cur().name()));
            0
        }
        Err(e) => err_code(Err(e.to_string())),
    }
}

fn cmd_install(b: &dyn Backend, c: &Config, pkg: bool) -> i32 {
    let ok = |s: String| println!("\x1b[32m✓\x1b[0m {s}");
    let exe = std::env::current_exe().and_then(fs::canonicalize).unwrap_or_default();
    let l = if pkg { &PACKAGE } else { &MANUAL };
    if pkg && !package_script_context(&exe) {
        eprintln!("{}", t!("upd install --package доступен только из установочного сценария пакета."));
        return 1;
    }
    if !pkg && fs::symlink_metadata(PKG_BIN).is_ok() {
        eprintln!("{}", t!("upd уже установлен пакетом ({0}) — обновляй его через пакетный менеджер.", PKG_BIN));
        return 1;
    }
    if pkg {
        if let Err(e) = remove_manual_files() {
            return err_code(Err(e));
        }
    } else if exe != Path::new(LOCAL_BIN) {
        let _ = fs::create_dir_all("/usr/local/bin");
        if let Err(e) = install_binary(&exe, Path::new(LOCAL_BIN)) {
            eprintln!("{}", t!("не удалось скопировать бинарник: {0}", e));
            return 1;
        }
    }
    ok(t!("бинарник: {}", l.bin));
    if !pkg {
        if let Some(src) = gui_source(&exe) {
            match install_gui(&src) {
                Ok(()) => ok(t!("интерфейс COSMIC: {0} — апплет добавляется в Настройках → Рабочий стол → Панель → Апплеты", GUI_BIN)),
                Err(e) => println!("{}", t!("⚠ интерфейс COSMIC не установлен: {0}", e)),
            }
        }
    }
    let mut c = c.clone();
    // язык спрашиваем один раз: пока он не выбран (auto), и только при ручной установке из терминала
    if !pkg && c.lang == "auto" && std::io::IsTerminal::is_terminal(&std::io::stdin()) {
        if let Some(l) = choose_lang() {
            c.lang = l.code().into();
            i18n::set(l);
            let _ = c.save();
        }
    }
    if !Path::new(&conf_path()).exists() {
        let _ = c.save();
    }
    let c = &c;
    ok(t!("настройки: {}", conf_path()));
    let _ = fs::create_dir_all(state_dir());

    if systemd() {
        if !pkg {
            match write_system_files("", l, "host", b) {
                Ok(_) => ok(t!("юниты, хуки и реакция на смену сети: {}", l.units)),
                Err(e) => return err_code(Err(e)),
            }
        }
        if let Err(e) = enable_units(b, c, &ok) {
            return err_code(Err(e));
        }
    } else if Path::new("/etc/cron.d").is_dir() {
        let _ = fs::write(CRON_PATH, format!("# upd\n*/15 * * * * root {0} net\n17 */6 * * * root {0} auto\n", l.bin));
        ok(t!("systemd нет — задания в {0} (VPN запускай вручную: upd vpn start)", CRON_PATH));
    } else {
        println!("{}", t!("⚠ нет ни systemd, ни cron: автоматика не установлена, запускай upd вручную"));
    }
    if b.mirrors_managed() && Path::new(GARUDA_CONF).exists() {
        garuda_skip_mirrors(true);
        ok(t!("garuda-update больше не перезаписывает зеркала ({0})", GARUDA_CONF));
    }

    // из пакетного менеджера — без долгого подбора зеркал (он держит блокировку); это сделает upd-auto
    if b.mirrors_managed() && !pkg {
        println!("{}", t!("\nПервый подбор зеркал для текущей сети:"));
        with_lock(true, || {
            err_code(handle_network(b, c, &stdlog, true).map(|_| ()))
        });
    }
    if systemd() {
        let _ = run(true, &[], "systemctl", &["start", "--no-block", "upd-auto.service"]);
        println!("{}", t!("\nПервая проверка обновлений{} запущена в фоне: journalctl -u upd-auto -f", if pkg { t!(" и подбор зеркал") } else { "" }));
    }
    garuda_unalias_install(pkg, l.bin);
    println!("{}", t!("\nГотово. Запуск интерфейса: upd"));
    0
}

fn cmd_uninstall(b: &dyn Backend, pkg: bool) -> i32 {
    if pkg {
        let exe = std::env::current_exe().and_then(fs::canonicalize).unwrap_or_default();
        if !package_script_context(&exe) {
            eprintln!("{}", t!("upd uninstall --package доступен только из сценария пакетного менеджера."));
            return 1;
        }
    }
    let c = match Config::load(b.default_mirrors()) {
        Ok(c) => c,
        Err(e) => return err_code(Err(e)),
    };
    if let Err(e) = b.remove_mirrors() {
        return err_code(Err(t!("не удалось вернуть зеркала: {0}", e)));
    }
    if systemd() {
        let mut names: Vec<&str> = system_units(PKG_BIN, Some("-")).iter().map(|x| x.0).collect();
        names.retain(|n| !n.ends_with(".service") || *n == vpn::SERVICE || *n == "upd-helper.service");
        let _ = run(true, &[], "systemctl", &["--global", "disable", "upd-notify.timer"]);
        for user in logged_in_users() {
            let _ = run(true, &[], "systemctl", &["--user", "-M", &format!("{user}@"), "stop", "upd-notify.timer"]);
        }
        let mut args = vec!["disable", "--now"];
        args.extend(names.iter().copied());
        let _ = run(true, &[], "systemctl", &args);
        if !pkg {
            if let Err(e) = remove_manual_files() {
                return err_code(Err(e));
            }
            let _ = fs::remove_file(MANUAL.apt_hook);
        }
        let _ = run(true, &[], "systemctl", &["daemon-reload"]);
    }
    if !pkg {
        remove_gui();
    }
    let proxy_result = cli_user_context().and_then(|user| vpn::sysproxy(&c, user.as_ref()).map(|_| ()));
    let _ = fs::remove_file(CRON_PATH);
    garuda_skip_mirrors(false);
    for f in garuda_unalias(false) {
        println!("{}", t!("убрано снятие алиаса upd: {0}", f));
    }
    println!("{}", t!("upd удалён. Оставлены настройки и данные: {}, {} (подписки VPN), {} — удали вручную, если не нужны.", conf_path(), vpn::etc(), state_dir()));
    err_code(proxy_result)
}

#[cfg(test)]
mod contract_tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn install_reloads_helper_and_reports_restart_failure() {
        use upd::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        let _isolation = isolation_lock();
        let dir = TempDirGuard::new("upd-install-helper-reload").unwrap();
        let log = dir.path().join("systemctl.log");
        let mut env = EnvGuard::new();
        env.set("PATH", dir.path());
        env.set("UPD_VPN_ETC", dir.path());
        env.set("UPD_HELPER_SOCK", dir.path().join("helper.sock"));
        env.set("UPD_TEST_SYSTEMCTL_LOG", &log);
        write_stub(dir.path(), "apt-get", "#!/bin/sh\nexit 0\n");
        write_stub(dir.path(), "loginctl", "#!/bin/sh\nexit 0\n");
        write_stub(dir.path(), "systemctl", "#!/bin/sh\necho \"$*\" >> \"$UPD_TEST_SYSTEMCTL_LOG\"\nexit 0\n");
        let backend = backend::detect().unwrap();
        enable_units(backend.as_ref(), &Config::defaults(vec![]), &|_| {}).unwrap();
        let calls = fs::read_to_string(&log).unwrap();
        let enable = calls.find("enable --now").unwrap();
        let reload = calls.find("try-restart upd-helper.service").unwrap();
        assert!(reload > enable, "{calls}");
        // A package post-install can run inside the helper's current operation.
        // A legacy Status reply must suffice to avoid restarting its parent.
        let listener = std::os::unix::net::UnixListener::bind(dir.path().join("helper.sock")).unwrap();
        listener.set_nonblocking(true).unwrap();
        let server = std::thread::spawn(move || {
            use std::io::{BufRead, Write};
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
            let mut stream = loop {
                match listener.accept() {
                    Ok((stream, _)) => break stream,
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        assert!(std::time::Instant::now() < deadline, "installer must query operation status");
                        std::thread::sleep(std::time::Duration::from_millis(5));
                    }
                    Err(error) => panic!("fixture accept failed: {error}"),
                }
            };
            stream.set_read_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            stream.set_write_timeout(Some(std::time::Duration::from_secs(2))).unwrap();
            let mut request = String::new();
            std::io::BufReader::new(stream.try_clone().unwrap()).read_line(&mut request).unwrap();
            assert!(request.contains("\"op\":\"status\""));
            stream.write_all(b"{\"ok\":true,\"data\":{\"running\":true}}\n").unwrap();
        });
        fs::write(&log, "").unwrap();
        enable_units(backend.as_ref(), &Config::defaults(vec![]), &|_| {}).unwrap();
        server.join().unwrap();
        assert!(!fs::read_to_string(&log).unwrap().contains("try-restart upd-helper.service"));
        fs::remove_file(dir.path().join("helper.sock")).unwrap();
        write_stub(dir.path(), "systemctl", "#!/bin/sh\nif [ \"$1 $2\" = 'try-restart upd-helper.service' ]; then echo helper-restart-failed >&2; exit 1; fi\nexit 0\n");
        let error = enable_units(backend.as_ref(), &Config::defaults(vec![]), &|_| {}).unwrap_err();
        assert!(error.contains("helper-restart-failed"), "{error}");
    }

    // --- INSTALL-01 ---
    #[test]
    fn review_binary_install_concurrency_never_mixes_files() {
        use upd::common::contract_fixtures::TempDirGuard;
        use std::os::unix::fs::PermissionsExt;
        let dir = TempDirGuard::new("upd-binary-install").unwrap();
        let source_a = dir.path().join("a"); let source_b = dir.path().join("b");
        std::fs::write(&source_a, vec![b'a'; 65536]).unwrap();
        std::fs::write(&source_b, vec![b'b'; 65536]).unwrap();
        let target = dir.path().join("installed");
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(8));
        let mut workers = Vec::new();
        for n in 0..8 { let source = if n % 2 == 0 { source_a.clone() } else { source_b.clone() };
            let target = target.clone(); let barrier = barrier.clone();
            workers.push(std::thread::spawn(move || { barrier.wait(); install_binary(&source, &target).unwrap(); })); }
        for worker in workers { worker.join().unwrap(); }
        let bytes = std::fs::read(&target).unwrap(); assert_eq!(bytes.len(), 65536);
        assert!(bytes.iter().all(|byte| *byte == bytes[0]));
        assert_eq!(std::fs::metadata(target).unwrap().permissions().mode() & 0o777, 0o755);
        assert!(!dir.path().read_dir().unwrap().any(|entry| entry.unwrap().file_name().to_string_lossy().ends_with(".tmp")));
    }

    #[test]
    fn review_failed_notification_is_not_marked_seen_and_can_retry() {
        use upd::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        use std::os::unix::fs::PermissionsExt;
        let _isolation = isolation_lock(); let dir = TempDirGuard::new("upd-notification-fail").unwrap();
        let notify = dir.path().join("notify-send");
        std::fs::write(&notify, "#!/bin/sh\nexit 1\n").unwrap();
        std::fs::set_permissions(&notify, std::fs::Permissions::from_mode(0o755)).unwrap();
        let mut env = EnvGuard::new(); env.set("PATH", dir.path());
        let mut seen = BTreeMap::new();
        assert!(notify_once(&mut seen, "updates", "version1".into(), "title", "body").is_err());
        assert!(!seen.contains_key("updates"));
        std::fs::write(&notify, "#!/bin/sh\nexit 0\n").unwrap();
        assert!(notify_once(&mut seen, "updates", "version1".into(), "title", "body").is_ok());
        assert_eq!(seen.get("updates").map(String::as_str), Some("version1"));
    }

    #[test]
    fn install01_package_install_requires_package_script_env() {
        let _isolation = upd::common::contract_fixtures::isolation_lock();
        let exe = Path::new(PKG_BIN);
        let previous = std::env::var_os(PACKAGE_SCRIPT_ENV);
        unsafe { std::env::remove_var(PACKAGE_SCRIPT_ENV); }
        assert!(!package_script_context(exe));
        unsafe {
            std::env::set_var(PACKAGE_SCRIPT_ENV, "1");
        }
        let allowed = package_script_context(exe);
        match previous {
            Some(value) => unsafe { std::env::set_var(PACKAGE_SCRIPT_ENV, value); },
            None => unsafe { std::env::remove_var(PACKAGE_SCRIPT_ENV); },
        }
        assert_eq!(allowed, fs::canonicalize(PKG_BIN).map(|p| p.as_path() == exe).unwrap_or(false));
    }

    #[test]
    fn install01_manual_layout_uses_local_bin() {
        assert_eq!(MANUAL.bin, LOCAL_BIN);
        assert_eq!(PACKAGE.bin, PKG_BIN);
        assert_ne!(MANUAL.bin, PACKAGE.bin);
    }

    // --- INSTALL-02 ---
    #[test]
    fn install02_rejects_foreign_unit_file() {
        let dir = std::env::temp_dir().join(format!("upd-units-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap();
        let unit = dir.join("upd-auto.timer");
        fs::write(&unit, "[Timer]\nOnBootSec=1min\n").unwrap();
        let err = owned_unit_files(dir.to_str().unwrap(), &["upd-auto.timer"]).unwrap_err();
        assert!(err.contains("маркера upd"), "{err}");
        let _ = fs::remove_dir_all(&dir);
    }

    // --- UPD-03 ---
    #[test]
    fn upd03_auto_failure_exit_code() {
        let st = UpdState {
            error: "не удалось обновить базы пакетов".into(),
            package_check_failure: Some(PackageCheckFailure::Refresh),
            ..Default::default()
        };
        assert_eq!(auto_state_failure(&st), Some("не удалось обновить базы пакетов"));
    }

    // --- UPD-02A ---
    #[test]
    fn upd04a_flatpak_check_failure_affects_exit() {
        let st = UpdState { flatpak_error: "flatpak: код 9".into(), ..Default::default() };
        assert_eq!(state_check_failure(&st), Some("flatpak: код 9"));
        assert_eq!(auto_state_failure(&st), Some("flatpak: код 9"));
    }

    #[test]
    fn upd02a_flatpak_upgrade_error_is_recorded() {
        let mut result = Ok(());
        record_upgrade_error(&mut result, "flatpak: код 9".into());
        assert_eq!(result.unwrap_err(), "flatpak: код 9");
    }

    // --- UPD-02B ---
    #[test]
    fn upd04b_firmware_check_failure_affects_exit() {
        let st = UpdState { firmware_error: "fwupd: код 7".into(), ..Default::default() };
        assert_eq!(state_check_failure(&st), Some("fwupd: код 7"));
        assert_eq!(auto_state_failure(&st), Some("fwupd: код 7"));
    }

    #[test]
    fn upd02b_firmware_upgrade_error_preserves_first_failure() {
        let mut result = Err("системный пакет".into());
        record_upgrade_error(&mut result, "fwupd: код 7".into());
        assert_eq!(result.unwrap_err(), "системный пакет");
    }

    // --- VPN-03 ---
    #[test]
    fn vpn03_core_update_restart_failure_message() {
        let message = vpn::core_restart_failure("код 1");
        assert!(message.contains("обновлено на диске"));
        assert!(message.contains("код 1"));
        let err = core_update_exit(true, true, Err("код 1".into())).unwrap_err();
        assert!(err.contains("не перезапустился"), "{err}");
        assert!(core_update_exit(true, true, Ok(())).is_ok());
        assert!(core_update_exit(false, true, Err("код 1".into())).is_ok());
    }

    fn write_stub(dir: &Path, name: &str, body: &str) {
        let path = dir.join(name);
        fs::write(&path, body).unwrap();
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o755)).unwrap();
    }

    #[test]
    fn install04_system_identifiers_do_not_depend_on_language() {
        let mut texts: Vec<String> = system_units("/usr/bin/upd", Some("/etc/pacman.d/mirrorlist")).into_iter().map(|u| u.1).collect();
        texts.extend(user_units("/usr/bin/upd").into_iter().map(|u| u.1));
        texts.extend([pacman_hook("/usr/bin/upd"), apt_hook("/usr/bin/upd"), nm_dispatcher().to_string()]);
        // в политике polkit русский есть только в переводах (xml:lang), основной текст — английский
        texts.extend(polkit_policy().lines().filter(|l| !l.contains("xml:lang")).map(String::from));
        texts.extend([GARUDA_MARK, UNALIAS_MARK, backend::PIN_BEGIN, backend::PIN_END, vpn::AUTO_GROUP, extras::SNAP_PRE_DESC, extras::SNAP_POST_DESC].map(String::from));
        for t in texts {
            assert!(t.is_ascii() || !t.chars().any(|c| ('\u{0400}'..='\u{04FF}').contains(&c)), "кириллица в системном тексте: {t}");
        }
    }

    #[test]
    fn gui01_polkit_policy_limits_passwordless_actions() {
        let p = polkit_policy();
        assert!(!p.contains("exec.path"), "pkexec upd не должен работать без пароля");
        let action = |id: &str| {
            let start = p.find(&format!("<action id=\"{id}\">")).unwrap();
            p[start..start + p[start..].find("</action>").unwrap()].to_string()
        };
        for id in [helper::ACTION_STATUS, helper::ACTION_CHECK, helper::ACTION_VPN] {
            let a = action(id);
            assert!(a.contains("<allow_active>yes</allow_active>") && a.contains("<allow_any>auth_admin</allow_any>"), "{a}");
        }
        let m = action(helper::ACTION_MANAGE);
        assert!(m.contains("<allow_active>auth_admin_keep</allow_active>") && m.contains("<allow_inactive>auth_admin</allow_inactive>"), "{m}");
        assert!(p.contains("xml:lang=\"ru\"") && p.contains("<!-- Managed by upd -->"));
    }

    #[test]
    fn gui02_helper_socket_units_and_desktop_files() {
        let units = system_units("/usr/bin/upd", None);
        let socket = &units.iter().find(|u| u.0 == "upd-helper.socket").unwrap().1;
        assert!(socket.contains("ListenStream=/run/upd/helper.sock") && socket.contains("SocketMode=0666"));
        let service = &units.iter().find(|u| u.0 == "upd-helper.service").unwrap().1;
        assert!(service.contains("ExecStart=/usr/bin/upd helper") && !service.contains("[Install]"), "запускается только по сокету");
        let applet = GUI_FILES.iter().find(|f| f.0.ends_with("io.github.upd.Applet.desktop")).unwrap().1;
        assert!(applet.contains("X-CosmicApplet=true") && applet.contains("Exec=upd-cosmic applet") && applet.contains("NoDisplay=true"));
        let app = GUI_FILES.iter().find(|f| f.0.ends_with("io.github.upd.desktop")).unwrap().1;
        assert!(app.contains("Exec=upd-cosmic\n") && app.contains("Icon=io.github.upd\n"));
    }

    #[test]
    fn install03_garuda_unalias_after_source_and_back() {
        let fish = "# conf\nsource /usr/share/garuda/garuda-fish-config/config.fish # defaults\n\n__garuda_fastfetch\n";
        let on = unalias_body(fish, "functions -e upd", true);
        assert_eq!(on, format!("# conf\nsource /usr/share/garuda/garuda-fish-config/config.fish # defaults\nfunctions -e upd  {UNALIAS_MARK}\n\n__garuda_fastfetch\n"));
        assert_eq!(unalias_body(&on, "functions -e upd", true), on);
        assert_eq!(unalias_body(&on, "functions -e upd", false), fish);
        // пользователь снял алиас сам — не дублируем и при удалении не трогаем
        // строка со старой меткой (до 0.2.5) получает новую
        let old = format!("source /usr/share/garuda/garuda-fish-config/config.fish\nfunctions -e upd  {UNALIAS_MARK_OLD}\n");
        assert_eq!(unalias_body(&old, "functions -e upd", true), format!("source /usr/share/garuda/garuda-fish-config/config.fish\nfunctions -e upd  {UNALIAS_MARK}\n"));
        assert_eq!(unalias_body(&old, "functions -e upd", false), "source /usr/share/garuda/garuda-fish-config/config.fish\n");
        let manual = "source /usr/share/garuda/garuda-bash-config/bashrc\nunalias upd\n";
        assert_eq!(unalias_body(manual, "unalias upd 2>/dev/null", true), manual);
        assert_eq!(unalias_body(manual, "unalias upd 2>/dev/null", false), manual);
    }

    #[test]
    fn install02_removes_owned_units_and_keeps_foreign() {
        let base = std::env::temp_dir().join(format!("upd-units-own-{}", std::process::id()));
        let units = base.join("units");
        let bin = base.join("bin");
        fs::create_dir_all(&units).unwrap();
        fs::create_dir_all(&bin).unwrap();
        let timer = units.join("upd-auto.timer");
        fs::write(&timer, "# Managed by upd\n[Timer]\nOnBootSec=1min\n").unwrap();
        let foreign = units.join("upd-custom.service");
        fs::write(&foreign, "[Unit]\nDescription=foreign\n[Service]\nExecStart=/bin/true\n").unwrap();
        let err = owned_unit_files(units.to_str().unwrap(), &["upd-custom.service"]).unwrap_err();
        assert!(err.contains("маркера upd"), "{err}");
        assert!(foreign.exists());
        write_stub(&bin, "systemctl", "#!/bin/sh\nexit 0\n");
        let owned = owned_unit_files(units.to_str().unwrap(), &["upd-auto.timer"]).unwrap();
        upd::common::contract_fixtures::with_prepend_path(&bin, || disable_and_remove_units(units.to_str().unwrap(), &owned, false).unwrap());
        assert!(!timer.exists());
        assert!(foreign.exists());
        let _ = fs::remove_dir_all(&base);
    }

    fn vpn_fixture(name: &str) -> (std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf, std::path::PathBuf) {
        let base = std::env::temp_dir().join(format!("upd-{name}-{}", std::process::id()));
        let bin = base.join("bin");
        let etc = base.join("etc");
        let home = base.join("home");
        let state = base.join("state");
        fs::create_dir_all(&bin).unwrap();
        fs::create_dir_all(&etc).unwrap();
        fs::create_dir_all(home.join("profiles")).unwrap();
        fs::create_dir_all(&state).unwrap();
        (base, bin, etc, home, state)
    }

    #[test]
    fn vpn01_last_subscription_stops_service() {
        let _iso = upd::common::contract_fixtures::isolation_lock();
        let (base, bin, etc, home, state) = vpn_fixture("vpn01-stop");
        let log = base.join("cmd.log");
        let log_s = log.to_str().unwrap().replace('\'', "");
        write_stub(&bin, "systemctl", &format!("#!/bin/sh\necho \"$@\" >> '{log_s}'\nif [ \"$1\" = is-active ]; then echo active; fi\nexit 0\n"));
        write_stub(&bin, "id", "#!/bin/sh\necho 1000\n");
        write_stub(&bin, "gsettings", &format!("#!/bin/sh\necho \"$@\" >> '{log_s}'\nexit 0\n"));
        write_stub(&bin, "runuser", &format!("#!/bin/sh\necho \"$@\" >> '{log_s}'\nexit 0\n"));
        let subs = serde_json::json!({
            "active": "a1",
            "list": [{"id":"a1","name":"one","url":"https://example.com/sub","interval_h":24,"updated":0,"nodes":1,"kind":"yaml"}]
        });
        fs::write(etc.join("subs.json"), subs.to_string()).unwrap();
        fs::write(home.join("profiles").join("a1.yaml"), "proxies: []\n").unwrap();
        unsafe {
            std::env::set_var("UPD_VPN_ETC", etc.to_str().unwrap());
            std::env::set_var("UPD_VPN_HOME", home.to_str().unwrap());
            std::env::set_var("UPD_STATE_DIR", state.to_str().unwrap());
            std::env::set_var("SUDO_USER", UserContext::from_uid(libc::geteuid()).unwrap().name);
        }
        let code = {
            let _path = upd::common::contract_fixtures::prepend_path(&bin);
            cmd_vpn(&Config::defaults(vec![]), &["del".into(), "1".into()])
        };
        let recorded = fs::read_to_string(&log).unwrap_or_default();
        unsafe {
            std::env::remove_var("UPD_VPN_ETC");
            std::env::remove_var("UPD_VPN_HOME");
            std::env::remove_var("UPD_STATE_DIR");
            std::env::remove_var("SUDO_USER");
        }
        assert_eq!(code, 0, "{recorded}");
        assert!(recorded.contains("stop upd-vpn.service"), "{recorded}");
        assert!(recorded.contains("disable upd-vpn.service"), "{recorded}");
        assert!(recorded.contains("mode") && recorded.contains("none"), "{recorded}");
        let left = fs::read_to_string(etc.join("subs.json")).unwrap();
        assert!(left.contains("\"list\":[]") || left.contains("\"list\": []"), "{left}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn vpn01_stop_failure_is_an_error_after_delete() {
        let _iso = upd::common::contract_fixtures::isolation_lock();
        let (base, bin, etc, home, state) = vpn_fixture("vpn01-fail");
        write_stub(&bin, "systemctl", "#!/bin/sh\nif [ \"$1\" = is-active ]; then echo active; exit 0; fi\nif [ \"$1\" = stop ]; then echo fail 1>&2; exit 1; fi\nexit 0\n");
        write_stub(&bin, "id", "#!/bin/sh\nexit 0\n");
        write_stub(&bin, "gsettings", "#!/bin/sh\nexit 0\n");
        write_stub(&bin, "runuser", "#!/bin/sh\nexit 0\n");
        fs::write(etc.join("subs.json"), r#"{"active":"a1","list":[{"id":"a1","name":"one","url":"https://example.com/sub","interval_h":24,"updated":0,"nodes":0,"kind":"yaml"}]}"#).unwrap();
        unsafe {
            std::env::set_var("UPD_VPN_ETC", etc.to_str().unwrap());
            std::env::set_var("UPD_VPN_HOME", home.to_str().unwrap());
            std::env::set_var("UPD_STATE_DIR", state.to_str().unwrap());
            std::env::remove_var("SUDO_USER");
        }
        let code = {
            let _path = upd::common::contract_fixtures::prepend_path(&bin);
            cmd_vpn(&Config::defaults(vec![]), &["del".into(), "1".into()])
        };
        unsafe {
            std::env::remove_var("UPD_VPN_ETC");
            std::env::remove_var("UPD_VPN_HOME");
            std::env::remove_var("UPD_STATE_DIR");
        }
        assert_eq!(code, 1);
        let left = fs::read_to_string(etc.join("subs.json")).unwrap();
        assert!(left.contains("\"list\":[]") || left.contains("\"list\": []"), "{left}");
        let _ = fs::remove_dir_all(&base);
    }

    #[test]
    fn upd04c_full_update_line_needs_successful_empty_aur() {
        assert_eq!(nothing_to_update_line(true, true, true, true, true), Some("Система уже обновлена."));
        assert!(nothing_to_update_line(true, true, false, true, true).is_none());
        assert_eq!(
            nothing_to_update_line(true, true, true, true, false),
            Some("Системные пакеты обновлены; состояние дополнительных источников проверить не удалось.")
        );
    }
}
