//! Снимки интерфейса без экрана: `UPD_SNAPSHOTS=каталог cargo test snapshots -- --nocapture`.
//! Рендер tiny-skia в PNG — для проверки вёрстки всплывающего окна и страниц в тёмной и светлой темах,
//! по-русски и по-арабски (справа налево).

use crate::applet::Applet;
use crate::op::OpView;
use crate::window::{Page, Window};
use cosmic::iced::Length;
use upd::common::{News, UpdState};
use upd::helper::{Event, PromptKind};
use upd::i18n::{self, Lang};
use upd::summary::{Features, MirrorSummary, OpStatus, Summary, VpnSummary};
use upd::vpn;

fn now() -> i64 {
    upd::common::now()
}

fn summary() -> Summary {
    let packages: Vec<String> = ["linux 6.16.8 -> 6.16.9", "mesa 25.2.3 -> 25.2.4", "firefox 143.0 -> 143.0.1", "systemd 258 -> 258.1", "glibc 2.42-1 -> 2.42-2", "cosmic-panel 1.0.0 -> 1.0.1", "python 3.13.7 -> 3.13.8", "openssl 3.5.3 -> 3.5.4"]
        .iter()
        .map(|s| s.to_string())
        .collect();
    Summary {
        checked: now() - 720,
        taken: now(),
        packages,
        flatpak: vec!["org.telegram.desktop".into(), "org.gimp.GIMP".into()],
        downloaded: true,
        download_size: Some(327 << 20),
        news: vec![News { title: "glibc 2.42: manual intervention required".into(), date: now() - 2 * 86400, link: "https://archlinux.org/news/".into() }],
        reboot: true,
        mirrors: MirrorSummary { managed: true, pinned: 3, checked: now() - 3600, failing: 2, ..Default::default() },
        vpn: VpnSummary { installed: true, active: true, has_subs: true, subscription: "Home".into(), ..Default::default() },
        features: Features { backend: "pacman (Garuda Linux)".into(), flatpak: true, aur: true, firmware: true, news: true, merge: true, vpn: true },
        ..Default::default()
    }
}

fn snapshot_vpn() -> vpn::Snapshot {
    let servers = ["🇩🇪 Germany 1", "🇳🇱 Netherlands", "🇫🇮 Finland", "🇺🇸 USA West", "🇯🇵 Japan", "🇸🇬 Singapore"];
    let mut delay = std::collections::BTreeMap::new();
    for (i, s) in servers.iter().enumerate() {
        delay.insert(s.to_string(), [42u64, 57, 63, 148, 0, 212][i]);
    }
    vpn::Snapshot {
        running: true,
        mode: "rule".into(),
        tun: true,
        version: "v1.19.14".into(),
        groups: vec![vpn::Group { name: "Proxy".into(), kind: "Selector".into(), now: servers[0].into(), all: servers.iter().map(|s| s.to_string()).collect() }],
        delay,
        down: 1_300_000_000,
        up: 95_000_000,
        conns: 37,
        error: String::new(),
    }
}

/// Рендер элемента без окна (tiny-skia) в PNG: UserInterface → draw → screenshot, как в iced_test.
fn shot(dir: &std::path::Path, name: &str, el: cosmic::Element<'_, impl Clone>, size: (f32, f32), theme: &cosmic::Theme) {
    use cosmic::iced::advanced::renderer::Headless;
    use cosmic::iced::advanced::{clipboard, mouse};
    use cosmic::iced::runtime::user_interface::{Cache, UserInterface};
    use cosmic::iced::theme::Base;
    use cosmic::iced::{Size, window};
    let el: cosmic::Element<'_, _> = cosmic::widget::container(el).width(Length::Fill).height(Length::Fill).class(cosmic::theme::Container::Background).into();
    let mut renderer = cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(cosmic::font::default(), cosmic::iced::Pixels(14.0), Some("tiny-skia"))).expect("headless renderer");
    let mut ui = UserInterface::build(el, Size::new(size.0, size.1), Cache::default(), &mut renderer);
    let mut messages = vec![];
    let _ = ui.update(
        &[cosmic::iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))],
        mouse::Cursor::Unavailable,
        &mut renderer,
        &mut clipboard::Null,
        &mut messages,
    );
    let base = theme.base();
    let style = cosmic::iced::advanced::renderer::Style { icon_color: base.text_color, text_color: base.text_color, scale_factor: 1.0 };
    ui.draw(&mut renderer, theme, &style, mouse::Cursor::Unavailable);
    let (w, h) = ((size.0 * 2.0) as u32, (size.1 * 2.0) as u32);
    let rgba = renderer.screenshot(Size::new(w, h), 2.0, base.background_color);
    let raw = dir.join(format!("{name}.rgba"));
    std::fs::write(&raw, rgba).unwrap();
    let png = dir.join(format!("{name}.png"));
    let ok = std::process::Command::new("magick")
        .args(["-size", &format!("{w}x{h}"), "-depth", "8"])
        .arg(format!("rgba:{}", raw.display()))
        .args(["-resize", "50%"])
        .arg(&png)
        .status()
        .map(|s| s.success())
        .unwrap_or(false);
    if ok {
        let _ = std::fs::remove_file(raw);
    }
}

#[test]
fn snapshots() {
    let Some(dir) = std::env::var_os("UPD_SNAPSHOTS").map(std::path::PathBuf::from) else { return };
    std::fs::create_dir_all(&dir).unwrap();
    // помощник «доступен»: файл на месте сокета
    let sock = dir.join("fake.sock");
    std::fs::write(&sock, "").unwrap();
    unsafe { std::env::set_var("UPD_HELPER_SOCK", &sock) };

    for (lang, tag) in [(Lang::Ru, "ru"), (Lang::Ar, "ar"), (Lang::En, "en")] {
        i18n::set_thread(lang);
        for (theme, tname) in [(cosmic::Theme::dark(), "dark"), (cosmic::Theme::light(), "light")] {
            if tag != "ru" && tname == "light" {
                continue;
            }
            let a = Applet::demo(summary(), OpStatus::default(), Some(snapshot_vpn()), true);
            shot(&dir, &format!("popup-{tag}-{tname}"), a.demo_popup(), (360.0, 820.0), &theme);
        }
    }
    i18n::set_thread(Lang::Ru);
    let dark = cosmic::Theme::dark();

    // состояния заголовка
    let busy = OpStatus { running: true, command: "update".into(), stage: Some((3, 6, "Загрузка".into())), waiting: true, ..Default::default() };
    let a = Applet::demo(summary(), busy, None, false);
    shot(&dir, "popup-ru-installing", a.demo_popup(), (360.0, 700.0), &dark);
    let calm = Summary { checked: now() - 300, taken: now(), features: summary().features, ..Default::default() };
    shot(&dir, "popup-ru-uptodate", Applet::demo(calm, OpStatus::default(), None, false).demo_popup(), (360.0, 320.0), &dark);
    let failed = Summary { checked: now() - 90000, taken: now(), error: "не удалось обновить базы пакетов: нет сети".into(), features: summary().features, ..Default::default() };
    shot(&dir, "popup-ru-failed", Applet::demo(failed, OpStatus::default(), None, false).demo_popup(), (360.0, 360.0), &dark);

    // окно
    let u = UpdState { checked: now() - 720, list: summary().packages, flatpak: summary().flatpak, downloaded: true, download_size: Some(327 << 20), ..Default::default() };
    let w = Window::demo(summary(), u.clone(), OpView::default(), Page::Updates);
    shot(&dir, "window-updates", w.demo_view(false), (900.0, 1100.0), &dark);
    let mut w = Window::demo(summary(), u.clone(), OpView::default(), Page::Vpn);
    let st = vpn::VpnState {
        core_version: "v1.19.14".into(),
        geo_updated: now() - 86400,
        subs: vec![vpn::SubPub { id: "a1".into(), name: "Home".into(), host: "sub.example.net".into(), updated: now() - 7200, nodes: 42, active: true, ..Default::default() }],
        ..Default::default()
    };
    w.set_demo_vpn(st, snapshot_vpn());
    shot(&dir, "window-vpn", w.demo_view(false), (900.0, 1500.0), &dark);
    shot(&dir, "window-settings", Window::demo(summary(), u.clone(), OpView::default(), Page::Settings).demo_view(false), (900.0, 1400.0), &dark);
    shot(&dir, "window-mirrors", Window::demo(summary(), u.clone(), OpView::default(), Page::Mirrors).demo_view(false), (900.0, 700.0), &dark);
    shot(&dir, "window-maintenance", Window::demo(summary(), u.clone(), OpView::default(), Page::Maintenance).demo_view(false), (900.0, 1100.0), &dark);

    let mut op = OpView::default();
    for ev in [
        Event::Reset { command: "update".into(), started: now() - 95 },
        Event::Line { text: "[1/6] Зеркала".into() },
        Event::Line { text: "последний замер: 2 ч назад".into() },
        Event::Line { text: "[2/6] Проверка".into() },
        Event::Line { text: ":: Synchronizing package databases...".into() },
        Event::Line { text: " core is up to date".into() },
        Event::Line { text: " extra                 8.4 MiB  9.1 MiB/s 00:01 [######################] 100%".into() },
        Event::Line { text: "Новости Arch с прошлого обновления — прочитай, там бывают ручные шаги:".into() },
        Event::Line { text: "  2026-09-26 10:00  glibc 2.42: manual intervention required".into() },
        Event::Stage { n: 2, m: 6, title: "Проверка".into() },
        Event::Partial { text: "Прочитал(а), продолжить? [Y/n] ".into() },
        Event::Prompt { text: "Прочитал(а), продолжить? [Y/n]".into(), kind: PromptKind::YesNo { default_yes: true } },
    ] {
        op.apply(ev);
    }
    shot(&dir, "window-operation", Window::demo(summary(), u, op, Page::Updates).demo_view(true), (900.0, 700.0), &dark);
    println!("снимки: {}", dir.display());
}
