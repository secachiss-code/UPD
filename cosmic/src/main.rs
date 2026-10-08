//! cm для COSMIC: индикатор на панели и запуск терминального интерфейса.

mod applet;
mod panel;
mod model;
mod jobs;
mod launch;
mod notifications;
mod tui_launch;
mod tunnel_icons;

fn main() -> cosmic::iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("applet") {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("{}", cm::t!("Интерфейс cm запускается от обычного пользователя, без sudo: действия с правами root выполняет помощник cm."));
            std::process::exit(1);
        }
        return cosmic::applet::run::<applet::Applet>(());
    }
    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("cm-cosmic {}", env!("CARGO_PKG_VERSION"));
        return Ok(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("cm-cosmic applet                      — {}", cm::t!("апплет на панели COSMIC"));
        println!("cm-cosmic [--new-window] — TUI cm");
        return Ok(());
    }
    model::init_lang();
    // Старые ссылки --page/--run открывают главное меню TUI.
    let known = ["--page", "--run"];
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if a == "--new-window" { continue; }
        if known.contains(&a.as_str()) {
            rest.next();
            continue;
        }
        if a == "install" || a == "uninstall" {
            let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.display().to_string())).unwrap_or_else(|| ".".into());
            eprintln!("{}", cm::t!("cm-cosmic не устанавливает себя сам. Установка: sudo {0}/cm-linux-amd64 {1} — интерфейс COSMIC, лежащий рядом, установится вместе с cm.", dir, a));
        } else {
            eprintln!("{}", cm::t!("неизвестный аргумент: {0} (cm-cosmic --help)", a));
        }
        std::process::exit(2);
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("{}", cm::t!("Интерфейс cm запускается от обычного пользователя, без sudo: действия с правами root выполняет помощник cm."));
        std::process::exit(1);
    }
    match model::open_tui_new(args.iter().any(|a| a == "--new-window")) {
        Ok(()) => Ok(()),
        Err(error) => { eprintln!("cm: {error}"); std::process::exit(1); }
    }
}
