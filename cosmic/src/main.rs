//! upd для COSMIC: апплет на панели (`upd-cosmic applet`) и окно (`upd-cosmic [--page РАЗДЕЛ] [--run КОМАНДА]`).

mod applet;
mod model;
mod op;
mod ui;
mod window;

#[cfg(test)]
mod snapshots;

fn main() -> cosmic::iced::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let value = |k: &str| args.iter().position(|a| a == k).and_then(|i| args.get(i + 1)).cloned();
    if args.first().map(String::as_str) == Some("applet") {
        if unsafe { libc::geteuid() } == 0 {
            eprintln!("{}", upd::t!("Интерфейс upd запускается от обычного пользователя, без sudo: действия с правами root выполняет помощник upd."));
            std::process::exit(1);
        }
        return cosmic::applet::run::<applet::Applet>(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("upd-cosmic applet                      — {}", upd::t!("апплет на панели COSMIC"));
        println!("upd-cosmic [--page РАЗДЕЛ] [--run КОМАНДА] — {}", upd::t!("окно upd"));
        println!("  {}: updates, news, mirrors, vpn, maintenance, settings, operation", upd::t!("разделы"));
        return Ok(());
    }
    model::init_lang();
    // лишние слова (например, `install`) — это не команды интерфейса: подсказать, а не открывать окно
    let known = ["--page", "--run"];
    let mut rest = args.iter();
    while let Some(a) = rest.next() {
        if known.contains(&a.as_str()) {
            rest.next();
            continue;
        }
        if a == "install" || a == "uninstall" {
            let dir = std::env::current_exe().ok().and_then(|p| p.parent().map(|d| d.display().to_string())).unwrap_or_else(|| ".".into());
            eprintln!("{}", upd::t!("upd-cosmic не устанавливает себя сам. Установка: sudo {0}/upd-linux-amd64 {1} — интерфейс COSMIC, лежащий рядом, установится вместе с upd.", dir, a));
        } else {
            eprintln!("{}", upd::t!("неизвестный аргумент: {0} (upd-cosmic --help)", a));
        }
        std::process::exit(2);
    }
    if unsafe { libc::geteuid() } == 0 {
        eprintln!("{}", upd::t!("Интерфейс upd запускается от обычного пользователя, без sudo: действия с правами root выполняет помощник upd."));
        std::process::exit(1);
    }
    let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(960.0, 700.0)).size_limits(cosmic::iced::Limits::NONE.min_width(360.0).min_height(420.0));
    cosmic::app::run_single_instance::<window::Window>(settings, window::Flags::new(value("--page"), value("--run")))
}
