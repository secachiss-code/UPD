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

/// Real surface sizes, physical DPI and an in-process PNG encoder; errors always fail the test.
fn shot(dir: &std::path::Path, name: &str, el: cosmic::Element<'_, impl Clone>, size: (f32, f32), scale: f32, theme: &cosmic::Theme) {
    use cosmic::iced::advanced::renderer::Headless;
    use cosmic::iced::advanced::{clipboard, mouse};
    use cosmic::iced::runtime::user_interface::{Cache, UserInterface};
    use cosmic::iced::theme::Base;
    use cosmic::iced::{Size, window};
    let el: cosmic::Element<'_, _> = cosmic::widget::container(el).width(Length::Fill).height(Length::Fill).class(cosmic::theme::Container::Background).into();
    let mut renderer = cosmic::iced::futures::executor::block_on(<cosmic::Renderer as Headless>::new(cosmic::font::default(), cosmic::iced::Pixels(14.0), Some("tiny-skia"))).expect("headless renderer must be available");
    let mut ui = UserInterface::build(el, Size::new(size.0, size.1), Cache::default(), &mut renderer);
    let mut messages = vec![];
    let _ = ui.update(&[cosmic::iced::Event::Window(window::Event::RedrawRequested(std::time::Instant::now()))], mouse::Cursor::Unavailable, &mut renderer, &mut clipboard::Null, &mut messages);
    // Traverse the same focus operation used by Tab and activate every operation
    // button with Enter. Bounds checks catch invisible confirmations/footers.
    if (name.contains("operation") && !name.contains("starting") && !name.contains("disconnected")) || name.starts_with("panel-") {
        use cosmic::iced::advanced::widget::{Operation, operation::{Focusable, Outcome}};
        use cosmic::iced::{Rectangle, keyboard};
        #[derive(Default)]
        struct FocusBounds { bounds: Vec<Rectangle>, focused: Option<usize> }
        impl Operation for FocusBounds {
            fn traverse(&mut self, visit: &mut dyn FnMut(&mut dyn Operation)) { visit(self); }
            fn focusable(&mut self, _: Option<&cosmic::iced::advanced::widget::Id>, bounds: Rectangle, state: &mut dyn Focusable) {
                if state.is_focused() { self.focused = Some(self.bounds.len()); }
                self.bounds.push(bounds);
            }
        }
        let mut bounds = FocusBounds::default(); ui.operate(&renderer, &mut bounds);
        assert!(!bounds.bounds.is_empty(), "no operation controls: {name}");
        for b in &bounds.bounds {
            assert!(b.x >= 0.0 && b.y >= 0.0 && b.x + b.width <= size.0 + 1.0 && b.y + b.height <= size.1 + 1.0, "clipped control {b:?}: {name}");
        }
        for expected in 0..bounds.bounds.len() {
            let mut next: Box<dyn Operation> = Box::new(cosmic::iced::advanced::widget::operation::focusable::focus_next());
            loop { ui.operate(&renderer, next.as_mut()); match next.finish() { Outcome::Chain(op) => next = op, _ => break } }
            let mut current = FocusBounds::default(); ui.operate(&renderer, &mut current);
            assert_eq!(current.focused, Some(expected), "Tab order: {name}");
            let key = keyboard::Key::Named(keyboard::key::Named::Enter);
            let enter = cosmic::iced::Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(), modified_key: key, physical_key: keyboard::key::Physical::Code(keyboard::key::Code::Enter),
                location: keyboard::Location::Standard, modifiers: keyboard::Modifiers::empty(), text: None, repeat: false,
            });
            messages.clear();
            let _ = ui.update(&[enter], mouse::Cursor::Unavailable, &mut renderer, &mut clipboard::Null, &mut messages);
            assert_eq!(messages.len(), 1, "Enter must activate one control: {name}");
        }
        ui.operate(&renderer, &mut cosmic::iced::advanced::widget::operation::focusable::unfocus());
    }
    let base = theme.base();
    let style = cosmic::iced::advanced::renderer::Style { icon_color: base.text_color, text_color: base.text_color, scale_factor: scale as f64 };
    ui.draw(&mut renderer, theme, &style, mouse::Cursor::Unavailable);
    let (w, h) = ((size.0 * scale) as u32, (size.1 * scale) as u32);
    let rgba = renderer.screenshot(Size::new(w, h), scale, base.background_color);
    assert_eq!(rgba.len(), w as usize * h as usize * 4);
    assert!(rgba.chunks_exact(4).any(|pixel| pixel != &rgba[..4]), "blank screenshot: {name}");
    let png_path = dir.join(format!("{name}.png"));
    let file = std::fs::File::create(&png_path).unwrap();
    let mut encoder = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    encoder.set_color(png::ColorType::Rgba); encoder.set_depth(png::BitDepth::Eight);
    encoder.write_header().unwrap().write_image_data(&rgba).unwrap();
    assert!(std::fs::metadata(&png_path).unwrap().len() > 100);
    let decoder = png::Decoder::new(std::fs::File::open(&png_path).unwrap());
    let mut reader = decoder.read_info().unwrap(); let mut decoded = vec![0; reader.output_buffer_size()];
    let info = reader.next_frame(&mut decoded).unwrap(); assert_eq!((info.width, info.height), (w, h));
}

#[test]
fn panel_snapshots() {
    use upd::common::contract_fixtures::{TempDirGuard, isolation_lock};
    use upd::summary::Badge;
    use cosmic::iced::{Alignment, Length};
    use cosmic::widget;
    let _isolation = isolation_lock();
    let temporary = TempDirGuard::new("upd-panel-symbols").unwrap();
    let requested = std::env::var_os("UPD_PANEL_SNAPSHOTS").map(std::path::PathBuf::from);
    let dir = requested.as_deref().unwrap_or(temporary.path());
    std::fs::create_dir_all(dir).unwrap();
    for (theme, theme_name) in [(cosmic::Theme::dark(), "dark"), (cosmic::Theme::light(), "light")] {
        for scale in [1.0, 1.25, 1.5, 2.0] {
            let mut rows = widget::column::with_capacity(3).spacing(8);
            for size in [16, 20, 24] {
                let mut row = widget::row::with_capacity(11).spacing(4).align_y(Alignment::Center)
                    .push(widget::text(format!("{size}px")).width(Length::Fixed(38.0)));
                for badge in [Badge::Idle, Badge::Updates(7), Badge::Updates(12), Badge::Updates(1500),
                    Badge::Busy(Some((3, 6))), Badge::Waiting, Badge::Error, Badge::Reboot, Badge::Stale(12), Badge::Unverified] {
                    row = row.push(crate::panel::button(&badge, size, (4, 4), true).on_press(()));
                }
                rows = rows.push(row);
            }
            shot(dir, &format!("panel-{theme_name}-dpi{scale}"), widget::container(rows).padding(12).into(), (680.0, 152.0), scale, &theme);
        }
    }
}
fn operation(case: &str) -> OpView {
    let mut op = OpView::default(); op.apply(Event::Reset { command: "update".into(), started: now() - 95 });
    for n in 0..30 { op.apply(Event::Line { text: format!("[{}] пакет / package / خادم — строка вывода", n) }); }
    op.apply(Event::Stage { n: 2, m: 6, title: "Проверка / Check".into() });
    match case {
        "starting" => op.phase = crate::op::Phase::Starting,
        "disconnected" => op.phase = crate::op::Phase::Disconnected,
        "failed" => op.apply(Event::Exit { code: 7 }),
        "long-prompt" => op.apply(Event::Prompt { text: "Continue? Продолжить? / سؤال طويل مع خادم Proxy-Latin-Server — ".repeat(40), kind: PromptKind::YesNo { default_yes: true } }),
        "history" => { op.scroll(false); for n in 0..6000 { op.apply(Event::Line { text: format!("line {n}") }); } },
        _ => op.apply(Event::Prompt { text: "Прочитал(а), продолжить? [Y/n]".into(), kind: PromptKind::YesNo { default_yes: true } }),
    }
    op
}
#[test]
fn snapshots() {
    use upd::common::contract_fixtures::{TempDirGuard, EnvGuard, isolation_lock};
    let _isolation = isolation_lock();
    let temporary = TempDirGuard::new("upd-cosmic-snapshots").unwrap();
    let requested = std::env::var_os("UPD_SNAPSHOTS").map(std::path::PathBuf::from);
    let smoke_requested = std::env::var_os("UPD_SMOKE_SNAPSHOTS").map(std::path::PathBuf::from);
    let dir = requested.as_deref().or(smoke_requested.as_deref()).unwrap_or(temporary.path());
    std::fs::create_dir_all(dir).unwrap();
    let sock = temporary.path().join("fake.sock"); std::fs::write(&sock, "").unwrap();
    let mut env = EnvGuard::new(); env.set("UPD_HELPER_SOCK", &sock);
    let dark = cosmic::Theme::dark(); i18n::set_thread(Lang::Ru);
    let u = UpdState { checked: now() - 720, list: summary().packages, flatpak: summary().flatpak, ..Default::default() };
    // Mandatory smoke runs even without UPD_SNAPSHOTS. No skipped/no-op visual gate.
    shot(dir, "popup-smoke", Applet::demo(summary(), OpStatus::default(), Some(snapshot_vpn()), true).demo_popup(), (320.0, 480.0), 1.0, &dark);
    let mut vpn_summary = summary();
    vpn_summary.packages.clear(); vpn_summary.flatpak.clear(); vpn_summary.news.clear();
    vpn_summary.reboot = false; vpn_summary.mirrors.failing = 0;
    let mut vpn_applet = Applet::demo(vpn_summary, OpStatus::default(), Some(snapshot_vpn()), true);
    vpn_applet.demo_case("vpn-auto");
    shot(dir, "popup-vpn-auto-smoke", vpn_applet.demo_popup(), (320.0, 480.0), 1.0, &dark);
    shot(dir, "operation-smoke", Window::demo(summary(), u.clone(), operation("prompt"), Page::Updates).demo_view(true), (640.0, 480.0), 1.0, &dark);
    if requested.is_none() { return; }
    for lang in [Lang::Ru, Lang::En, Lang::Ar] {
        i18n::set_thread(lang);
        for (theme, name) in [(cosmic::Theme::dark(), "dark"), (cosmic::Theme::light(), "light")] {
            for scale in [1.0, 2.0] {
                for width in [320.0, 360.0] {
                    let app = Applet::demo(summary(), OpStatus::default(), Some(snapshot_vpn()), true);
                    shot(dir, &format!("popup-{}-{name}-{width}-dpi{scale}", lang.code()), app.demo_popup(), (width, 480.0), scale, &theme);
                }
                for size in [(640.0, 480.0), (900.0, 700.0)] {
                    for (page, page_name) in [(Page::Updates,"updates"),(Page::Settings,"settings"),(Page::Vpn,"vpn")] {
                        let mut w = Window::demo(summary(), u.clone(), OpView::default(), page);
                        w.set_demo_vpn(vpn::VpnState { core_version: "v1.19.14".into(), subs: vec![vpn::SubPub { id: "fixture".into(), name: "خادم Proxy — длинное имя подписки с латинским сервером".into(), host: "sub.example.invalid".into(), nodes: 200, active: true, ..Default::default() }], ..Default::default() }, snapshot_vpn());
                        shot(dir, &format!("window-{page_name}-{}-{name}-{}-dpi{scale}",lang.code(),size.0), w.demo_view(false), size, scale, &theme);
                    }
                    shot(dir, &format!("operation-{}-{name}-{}-dpi{scale}",lang.code(),size.0), Window::demo(summary(), u.clone(), operation("prompt"), Page::Updates).demo_view(true), size, scale, &theme);
                }
            }
        }
    }
    i18n::set_thread(Lang::Ru);
    for case in ["loading","denied","long-error","stale","empty","many"] {
        let mut s = summary();
        if case == "empty" { s.packages.clear(); s.flatpak.clear(); s.news.clear(); }
        if case == "many" { s.packages = (0..500).map(|n| format!("package-{n}-very-long-name-{} 1 -> 2", "long".repeat(20))).collect(); }
        let case_u = UpdState { list: s.packages.clone(), flatpak: s.flatpak.clone(), ..u.clone() };
        let mut w = Window::demo(s.clone(), case_u, OpView::default(), Page::Updates); w.demo_case(case);
        let mut a = Applet::demo(s, OpStatus::default(), Some(snapshot_vpn()), true); a.demo_case(case);
        shot(dir,&format!("case-{case}-window"),w.demo_view(false),(640.0,480.0),1.0,&dark);
        shot(dir,&format!("case-{case}-popup"),a.demo_popup(),(320.0,480.0),1.0,&dark);
    }
    for case in ["starting","disconnected","failed","long-prompt","history"] {
        shot(dir,&format!("case-{case}-operation"),Window::demo(summary(),u.clone(),operation(case),Page::Updates).demo_view(true),(640.0,480.0),1.0,&dark);
    }
    env.set("UPD_HELPER_SOCK", temporary.path().join("missing.sock"));
    shot(dir,"case-helper-unavailable",Window::demo(summary(),u.clone(),OpView::default(),Page::Updates).demo_view(false),(640.0,480.0),1.0,&dark);
    env.set("UPD_HELPER_SOCK", &sock);
    for lang in [Lang::De,Lang::It,Lang::Zh] {
        i18n::set_thread(lang);
        shot(dir,&format!("locale-{}-smoke",lang.code()),Window::demo(summary(),u.clone(),operation("prompt"),Page::Settings).demo_view(true),(640.0,480.0),1.0,&dark);
    }
    i18n::set_thread(Lang::Ru);
    println!("checked screenshots: {}", dir.display());
}
