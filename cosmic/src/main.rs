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
        return cosmic::applet::run::<applet::Applet>(());
    }
    if args.iter().any(|a| a == "--help" || a == "-h") {
        println!("upd-cosmic applet                      — {}", upd::t!("апплет на панели COSMIC"));
        println!("upd-cosmic [--page РАЗДЕЛ] [--run КОМАНДА] — {}", upd::t!("окно upd"));
        println!("  {}: updates, news, mirrors, vpn, maintenance, settings, operation", upd::t!("разделы"));
        return Ok(());
    }
    model::init_lang();
    let settings = cosmic::app::Settings::default().size(cosmic::iced::Size::new(960.0, 700.0)).size_limits(cosmic::iced::Limits::NONE.min_width(360.0).min_height(420.0));
    cosmic::app::run_single_instance::<window::Window>(settings, window::Flags::new(value("--page"), value("--run")))
}
