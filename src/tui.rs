//! Интерфейс в терминале (ratatui).

use crate::backend::{self, Backend};
use crate::common::*;
use crate::mirrors::{apply_mirrors, candidates, load_mirror_state};
use crate::{extras, gather_status, or_dash, sub_info, vpn, Status};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(PartialEq)]
enum Screen {
    Menu,
    Pager,
    Mirrors,
    Vpn,
}

const MENU: &[&str] = &[
    "Обновить всё",
    "Проверить и скачать обновления",
    "Что доступно",
    "Зеркала",
    "VPN",
    "Снапшоты и откат",
    "Перезапуск служб",
    "Очистка: кэш и ненужные пакеты",
    "Новые файлы настроек",
    "Журнал пакетов",
    "Выход",
];

struct App<'a> {
    b: &'a dyn Backend,
    st: Option<Status>,
    rx: Option<mpsc::Receiver<Status>>,
    scr: Screen,
    sel: usize,
    msg: String,
    p_title: String,
    p_lines: Vec<String>,
    p_off: usize,
    m_state: TableState,
    input: Option<String>,
    /// что вводится: "" — адрес зеркала, иначе ключ числовой настройки VPN
    input_key: &'static str,
    v: VpnUi,
    quit: bool,
}

const VPN_TABS: [&str; 3] = ["Серверы", "Подписки", "Настройки"];

#[derive(Default)]
struct VpnUi {
    tab: usize,
    snap: vpn::Snapshot,
    rx: Option<mpsc::Receiver<vpn::Snapshot>>,
    at: Option<Instant>,
    group: usize,
    /// группа уже выбрана пользователем; до этого показываем главную (первый Selector)
    group_set: bool,
    nodes: TableState,
    subs: TableState,
    opts: TableState,
    testing: Option<mpsc::Receiver<String>>,
    confirm_del: Option<usize>,
}

type Args = Option<Vec<String>>;

fn cmd(a: &[&str]) -> Args {
    Some(a.iter().map(|s| s.to_string()).collect())
}

pub fn run(b: &dyn Backend) -> i32 {
    let mut app = App {
        b,
        st: None,
        rx: None,
        scr: Screen::Menu,
        sel: 0,
        msg: String::new(),
        p_title: String::new(),
        p_lines: vec![],
        p_off: 0,
        m_state: TableState::default().with_selected(Some(0)),
        input: None,
        input_key: "",
        v: VpnUi::default(),
        quit: false,
    };
    app.reload();
    let mut term = ratatui::init();
    let r = app.main_loop(&mut term);
    ratatui::restore();
    if let Err(e) = r {
        eprintln!("upd: {e}");
        return 1;
    }
    0
}

impl App<'_> {
    /// Состояние собирается в фоне: там обход кэша, /proc и вызовы пакетного менеджера.
    fn reload(&mut self) {
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Ok(b) = backend::detect() {
                let _ = tx.send(gather_status(b.as_ref()));
            }
        });
        self.rx = Some(rx);
    }

    fn main_loop(&mut self, term: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.quit {
            if let Some(rx) = &self.rx {
                if let Ok(s) = rx.try_recv() {
                    self.st = Some(s);
                    self.rx = None;
                }
            }
            if self.scr == Screen::Vpn {
                self.vpn_poll();
            }
            term.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(200))? {
                if let Event::Key(k) = event::read()? {
                    if k.kind != KeyEventKind::Press {
                        continue;
                    }
                    if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                        break;
                    }
                    if let Some(args) = self.key(k.code) {
                        self.exec(term, &args)?;
                    }
                }
            }
        }
        Ok(())
    }

    /// Запускает `upd <args> --pause` в этом же терминале и возвращается в меню.
    fn exec(&mut self, term: &mut DefaultTerminal, args: &[String]) -> std::io::Result<()> {
        ratatui::restore();
        let exe = std::env::current_exe().unwrap_or_else(|_| "upd".into());
        let st = std::process::Command::new(exe).args(args).arg("--pause").status();
        *term = ratatui::init();
        term.clear()?;
        self.msg = match st {
            Ok(s) if s.success() => "готово".into(),
            _ => "завершилось с ошибкой".into(),
        };
        self.reload();
        self.v.at = None;
        Ok(())
    }

    fn pager(&mut self, title: &str, lines: Vec<String>) {
        self.scr = Screen::Pager;
        self.p_title = title.into();
        self.p_lines = if lines.is_empty() { vec!["пусто".into()] } else { lines };
        self.p_off = 0;
    }

    /// Обработка клавиши; Some(args) — запустить upd с этими аргументами.
    fn key(&mut self, k: KeyCode) -> Args {
        if self.input.is_some() {
            self.key_input(k);
            return None;
        }
        match self.scr {
            Screen::Menu => self.key_menu(k),
            Screen::Pager => {
                let n = self.p_lines.len();
                match k {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
                    KeyCode::Up | KeyCode::Char('k') => self.p_off = self.p_off.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => self.p_off = (self.p_off + 1).min(n.saturating_sub(1)),
                    KeyCode::PageUp => self.p_off = self.p_off.saturating_sub(20),
                    KeyCode::PageDown | KeyCode::Char(' ') => self.p_off = (self.p_off + 20).min(n.saturating_sub(1)),
                    KeyCode::Home => self.p_off = 0,
                    _ => {}
                }
                None
            }
            Screen::Mirrors => self.key_mirrors(k),
            Screen::Vpn => self.key_vpn(k),
        }
    }

    fn key_menu(&mut self, k: KeyCode) -> Args {
        match k {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.sel = (self.sel + MENU.len() - 1) % MENU.len(),
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.sel = (self.sel + 1) % MENU.len(),
            KeyCode::Char('r') => {
                self.msg = "обновляю состояние...".into();
                self.reload();
            }
            KeyCode::Enter => return self.activate(),
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap() as usize;
                self.sel = if n == 0 { MENU.len() - 1 } else { (n - 1).min(MENU.len() - 1) };
                return self.activate();
            }
            _ => {}
        }
        None
    }

    fn activate(&mut self) -> Args {
        match self.sel {
            0 => return cmd(&["update"]),
            1 => return cmd(&["check"]),
            2 => {
                let u: UpdState = load_json("updates.json");
                let mut l = vec![format!("проверено: {}", fmt_time(u.checked))];
                if !u.skipped.is_empty() {
                    l.push(format!("⏸ {}", u.skipped));
                }
                if !u.error.is_empty() {
                    l.push(format!("⚠ {}", u.error));
                }
                if !u.news.is_empty() {
                    l.push(String::new());
                    l.push("── Новости Arch (прочитай до обновления) ──".into());
                    for n in &u.news {
                        l.push(format!("{}  {}", fmt_time(n.date), n.title));
                        l.push(format!("      {}", n.link));
                    }
                }
                let size = if u.download_size > 0 { format!(", {}", fmt_bytes(u.download_size)) } else { String::new() };
                l.push(String::new());
                l.push(format!("── Пакеты: {}{size}{} ──", u.list.len(), if u.downloaded { ", скачаны" } else { "" }));
                l.extend(u.list.iter().cloned());
                for (t, list) in [("Flatpak", &u.flatpak), ("Прошивки", &u.firmware)] {
                    if !list.is_empty() {
                        l.push(String::new());
                        l.push(format!("── {t}: {} ──", list.len()));
                        l.extend(list.iter().cloned());
                    }
                }
                self.pager("Что доступно", l);
            }
            3 => {
                self.scr = Screen::Mirrors;
                self.m_state.select(Some(0));
            }
            4 => {
                self.scr = Screen::Vpn;
                self.v.at = None;
                for t in [&mut self.v.nodes, &mut self.v.subs, &mut self.v.opts] {
                    if t.selected().is_none() {
                        t.select(Some(0));
                    }
                }
            }
            5 => {
                let mut l = extras::snap_list(30);
                l.push(String::new());
                l.extend(extras::rollback_hint());
                self.pager("Снапшоты (новые сверху)", l);
            }
            6 => return cmd(&["restart"]),
            7 => return cmd(&["clean"]),
            8 => {
                let p = self.b.pending_configs();
                if !p.is_empty() && have("pacdiff") {
                    return cmd(&["merge"]);
                }
                self.pager("Новые файлы настроек", if p.is_empty() { vec!["нет — всё слито".into()] } else { p });
            }
            9 => self.pager("Журнал пакетов (новые сверху)", self.b.history(500)),
            _ => self.quit = true,
        }
        None
    }

    fn mirror_rows(&self) -> Vec<Probe> {
        let c = Config::load(self.b.default_mirrors());
        let st = load_mirror_state();
        let mut r: Vec<Probe> = candidates(self.b, &c, None)
            .into_iter()
            .map(|cd| {
                let mut p = st.results.iter().find(|p| p.url == cd.url).cloned().unwrap_or(Probe { url: cd.url.clone(), err: "не замерено".into(), ..Default::default() });
                p.src = cd.src.into();
                p
            })
            .collect();
        let shown = |p: &Probe| if p.score > 0.0 { p.score } else { p.speed };
        r.sort_by(|a, b| b.ok.cmp(&a.ok).then(shown(b).total_cmp(&shown(a))));
        r
    }

    fn key_mirrors(&mut self, k: KeyCode) -> Args {
        let managed = self.b.mirrors_managed();
        let rows = self.mirror_rows();
        let sel = self.m_state.selected().unwrap_or(0);
        let mut c = Config::load(self.b.default_mirrors());
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Up | KeyCode::Char('k') => self.m_state.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.m_state.select(Some((sel + 1).min(rows.len().saturating_sub(1)))),
            KeyCode::Char('c') if managed => return cmd(&["mirrors", "check"]),
            KeyCode::Char('s') if managed => return cmd(&["mirrors", "rescan"]),
            KeyCode::Char('a') if managed => {
                let log = std::cell::RefCell::new(vec![]);
                apply_mirrors(self.b, &c, &load_mirror_state(), None, &|s| log.borrow_mut().push(s.to_string()));
                self.msg = log.into_inner().join("; ");
                self.reload();
            }
            KeyCode::Char('n') if managed => self.input = Some(String::new()),
            KeyCode::Char('x') | KeyCode::Delete if managed => {
                if let Some(p) = rows.get(sel) {
                    if contains(&c.mirrors, &p.url) {
                        c.mirrors.retain(|m| m != &p.url);
                        let _ = c.save();
                        self.msg = "удалено из конфига".into();
                    } else {
                        self.msg = "это зеркало не из конфига — его подбирает автоматика".into();
                    }
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                c.keep = (c.keep + 1).min(10);
                let _ = c.save();
                self.msg = format!("закреплять {} — нажми a, чтобы применить", c.keep);
            }
            KeyCode::Char('-') => {
                c.keep = c.keep.saturating_sub(1).max(1);
                let _ = c.save();
                self.msg = format!("закреплять {} — нажми a, чтобы применить", c.keep);
            }
            _ => {}
        }
        None
    }

    fn key_input(&mut self, k: KeyCode) {
        let Some(buf) = self.input.as_mut() else { return };
        match k {
            KeyCode::Esc => self.input = None,
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(ch) => buf.push(ch),
            KeyCode::Enter => {
                let u = buf.trim().to_string();
                self.input = None;
                if u.is_empty() {
                    return;
                }
                if !self.input_key.is_empty() {
                    return self.vpn_set_number(&u);
                }
                if let Err(e) = self.b.valid_mirror(&u) {
                    self.msg = format!("неверный URL: {e}");
                    return;
                }
                let mut c = Config::load(self.b.default_mirrors());
                if !contains(&c.mirrors, &u) {
                    c.mirrors.push(u);
                    let _ = c.save();
                }
                self.msg = "добавлено — нажми c, чтобы замерить".into();
            }
            _ => {}
        }
    }

    // ---------- отрисовка ----------

    fn draw(&mut self, f: &mut Frame) {
        let [head, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(2)]).areas(f.area());
        f.render_widget(
            Line::from(vec![" upd ".bold().black().on_cyan(), Span::raw(" "), Span::styled(format!("обновление системы · {}", self.b.name()), dim())]),
            head,
        );
        let keys: Vec<(&str, &str)> = match (&self.scr, self.input.is_some()) {
            (_, true) => vec![("enter", "сохранить"), ("esc", "отмена")],
            (Screen::Vpn, _) => {
                let mut k = vec![("tab 1-3", "вкладка"), ("s", "вкл/выкл")];
                match self.v.tab {
                    0 => k.extend([("←→", "группа"), ("enter", "выбрать сервер"), ("t", "замерить задержку")]),
                    1 => k.extend([("enter", "сделать активной"), ("n", "добавить"), ("u", "обновить"), ("x", "удалить")]),
                    _ => k.extend([("enter", "изменить")]),
                }
                k.push(("q", "назад"));
                k
            }
            (Screen::Menu, _) => vec![("↑↓", "выбор"), ("enter", "выполнить"), ("1-9", "сразу"), ("r", "обновить"), ("q", "выход")],
            (Screen::Pager, _) => vec![("↑↓ PgUp PgDn", "листать"), ("q", "назад")],
            (Screen::Mirrors, _) => vec![("c", "замерить"), ("s", "искать заново"), ("a", "применить"), ("n", "добавить"), ("x", "удалить"), ("+/-", "сколько закреплять"), ("q", "назад")],
        };
        let mut kl: Vec<Span> = vec![];
        for (k, d) in keys {
            kl.push(Span::styled(format!(" {k} "), Style::new().fg(Color::Cyan)));
            kl.push(Span::styled(d.to_string(), dim()));
        }
        f.render_widget(Paragraph::new(vec![Line::from(kl), Line::from(Span::styled(self.msg.clone(), Style::new().fg(Color::Yellow)))]), foot);
        match self.scr {
            Screen::Menu => self.draw_menu(f, body),
            Screen::Pager => {
                let h = body.height.saturating_sub(2) as usize;
                let end = (self.p_off + h).min(self.p_lines.len());
                let lines: Vec<Line> = self.p_lines[self.p_off.min(end)..end].iter().map(|l| Line::raw(l.clone())).collect();
                let title = format!(" {} · {}–{} из {} ", self.p_title, self.p_off + 1, end, self.p_lines.len());
                f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).title(title)), body);
            }
            Screen::Mirrors => self.draw_mirrors(f, body),
            Screen::Vpn => self.draw_vpn(f, body),
        }
    }

    fn draw_menu(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let lines = match &self.st {
            None => vec![Line::styled("загружаю состояние...", dim())],
            Some(s) => status_lines(s),
        };
        let [top, bottom] = Layout::vertical([Constraint::Length(lines.len() as u16 + 2), Constraint::Min(MENU.len() as u16)]).areas(area);
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(" Состояние ")),
            top,
        );
        let items: Vec<Line> = MENU
            .iter()
            .enumerate()
            .map(|(i, t)| {
                let n = if i + 1 == MENU.len() { 0 } else { i + 1 };
                let s = format!(" {n:>2}  {t} ");
                if i == self.sel {
                    Line::styled(s, Style::new().add_modifier(Modifier::REVERSED))
                } else {
                    Line::raw(s)
                }
            })
            .collect();
        f.render_widget(Paragraph::new(items), bottom);
    }

    fn draw_mirrors(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        if !self.b.mirrors_managed() {
            f.render_widget(Paragraph::new(self.b.mirror_note()).block(Block::bordered().title(" Зеркала ")), area);
            return;
        }
        let c = Config::load(self.b.default_mirrors());
        let st = load_mirror_state();
        let net = fingerprint();
        let pinned = self.b.pinned();
        let hints = st.hints.len() as u16;
        let [info, table, hint_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(3), Constraint::Length(if hints > 0 { hints * 2 + 1 } else { 0 })]).areas(area);
        let mut info_lines = vec![Line::from(vec![
            Span::styled("сеть: ", dim()),
            Span::raw(net.label.clone()),
            Span::styled(format!(" · замер: {} · закреплять лучших: {}", fmt_ago(st.checked), c.keep), dim()),
        ])];
        if let Some(buf) = &self.input {
            info_lines.push(Line::from(vec![Span::raw("Новое зеркало: "), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan))]));
        } else {
            info_lines.push(Line::styled("● закреплено · скорость сглажена по истории замеров в этой сети · отстающие зеркала не берутся", dim()));
        }
        f.render_widget(Paragraph::new(info_lines), info);
        let rows: Vec<Row> = self
            .mirror_rows()
            .into_iter()
            .map(|p| {
                let color = if p.ok {
                    Color::Green
                } else if p.err == "не замерено" {
                    Color::DarkGray
                } else {
                    Color::Red
                };
                Row::new(vec![
                    Cell::from(Span::styled(if contains(&pinned, &p.url) { "●" } else { "" }, Style::new().fg(Color::Cyan))),
                    Cell::from(p.src.clone()),
                    Cell::from(Span::styled(fmt_speed(&p), Style::new().fg(color))),
                    Cell::from(p.lag_h.map(|l| format!("{l} ч")).unwrap_or_default()),
                    Cell::from(p.url.clone()),
                ])
            })
            .collect();
        let t = Table::new(rows, [Constraint::Length(3), Constraint::Length(7), Constraint::Length(13), Constraint::Length(8), Constraint::Min(20)])
            .header(Row::new(vec!["", "откуда", "скорость", "отстаёт", "адрес"]).style(dim()))
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(" Зеркала "));
        f.render_stateful_widget(t, table, &mut self.m_state);
        if hints > 0 {
            let l: Vec<Line> = st.hints.iter().map(|h| Line::styled(format!("💡 {h}"), Style::new().fg(Color::Yellow))).collect();
            f.render_widget(Paragraph::new(l).wrap(Wrap { trim: true }), hint_area);
        }
    }
}

// ---------- VPN ----------

/// Пункт вкладки «Настройки»: ключ, подпись, значение.
struct Opt {
    key: &'static str,
    label: &'static str,
    value: String,
}

fn on_off(b: bool) -> String {
    if b { "вкл" } else { "выкл" }.into()
}

fn delay_span(d: Option<&u64>) -> Span<'static> {
    match d {
        None => Span::styled("—", dim()),
        Some(0) => Span::styled("✗", Style::new().fg(Color::Red)),
        Some(&d) => Span::styled(format!("{d} мс"), Style::new().fg(if d < 300 { Color::Green } else if d < 800 { Color::Yellow } else { Color::Red })),
    }
}

impl App<'_> {
    fn cfg(&self) -> Config {
        Config::load(self.b.default_mirrors())
    }

    /// Состояние ядра опрашивается в фоне раз в 3 секунды, пока открыт экран VPN.
    fn vpn_poll(&mut self) {
        if let Some(rx) = &self.v.rx {
            if let Ok(s) = rx.try_recv() {
                self.v.snap = s;
                self.v.rx = None;
                if !self.v.group_set {
                    self.v.group = self.v.snap.groups.iter().position(|g| g.kind == "Selector").unwrap_or(0);
                }
                self.v.group = self.v.group.min(self.v.snap.groups.len().saturating_sub(1));
            }
        }
        if let Some(rx) = &self.v.testing {
            if let Ok(m) = rx.try_recv() {
                self.msg = m;
                self.v.testing = None;
                self.v.at = None;
            }
        }
        if self.v.rx.is_none() && self.v.at.map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true) {
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let _ = tx.send(vpn::snapshot());
            });
            self.v.rx = Some(rx);
            self.v.at = Some(Instant::now());
        }
    }

    fn vpn_opts(&self) -> Vec<Opt> {
        let c = self.cfg();
        let st = vpn::load_state();
        let unit = unit_state(vpn::SERVICE);
        let geo = vpn::geo_files().iter().map(|g| g.1).filter(|t| *t > 0).min();
        let core = match (st.core_version.as_str(), st.core_latest.as_str()) {
            ("", _) => "не скачано — скачать".to_string(),
            (v, l) if !l.is_empty() && l != v => format!("{v} → есть {l}, обновить"),
            (v, _) => format!("{v}, проверено {}", fmt_ago(st.checked)),
        };
        let o = |key, label, value| Opt { key, label, value };
        vec![
            o(
                "run",
                "VPN",
                match unit.as_str() {
                    "active" => "работает — выключить",
                    "" => "служба не установлена — sudo upd install",
                    "failed" => "ошибка запуска — включить снова",
                    _ => "выключен — включить",
                }
                .into(),
            ),
            o("tun", "Режим", if c.vpn_tun { "TUN — вся система".into() } else { format!("только прокси 127.0.0.1:{}", c.vpn_port) }),
            o("mode", "Маршрутизация", ["по правилам", "всё через VPN", "всё напрямую"][c.vpn_mode.min(2) as usize].into()),
            o("autostart", "Запуск при загрузке", on_off(c.vpn_autostart)),
            o("auto", "Автовыбор сервера (⚡ Авто)", on_off(c.vpn_auto_select)),
            o("ru", "Россия напрямую (геофайлы)", on_off(c.vpn_direct_ru)),
            o("lan", "Локальная сеть напрямую", on_off(c.vpn_direct_lan)),
            o("dns", "Свой DNS (fake-ip)", on_off(c.vpn_dns)),
            o("ipv6", "IPv6", on_off(c.vpn_ipv6)),
            o("allow_lan", "Прокси для устройств в сети", on_off(c.vpn_allow_lan)),
            o("vpn_port", "Порт прокси", c.vpn_port.to_string()),
            o("vpn_sub_update_h", "Обновлять подписки, ч", c.vpn_sub_update_h.to_string()),
            o("rules", "Свои правила", format!("{} шт. — открыть редактор", vpn::user_rules().len())),
            o("core", "Ядро mihomo", core),
            o("geo", "Геофайлы", geo.map(|t| format!("от {} — обновить", fmt_ago(t))).unwrap_or_else(|| "не скачаны — скачать".into())),
        ]
    }

    /// Сохранить настройки и применить к работающему ядру; итог — в строку сообщений.
    fn vpn_apply(&mut self, c: &Config) {
        if let Err(e) = c.save() {
            self.msg = format!("не сохранено: {e}");
            return;
        }
        let log = std::cell::RefCell::new(vec![]);
        if let Err(e) = vpn::apply(c, &|s| log.borrow_mut().push(s.to_string())) {
            log.borrow_mut().push(format!("ошибка: {e}"));
        }
        self.msg = log.into_inner().join("; ");
        self.v.at = None;
    }

    fn vpn_set_number(&mut self, s: &str) {
        let key = std::mem::take(&mut self.input_key);
        let mut c = self.cfg();
        match (key, s.parse::<i64>()) {
            ("vpn_port", Ok(n)) if (1024..=65535).contains(&n) && n != 9097 => c.vpn_port = n as u16,
            ("vpn_sub_update_h", Ok(n)) if n >= 1 => c.vpn_sub_update_h = n,
            _ => {
                self.msg = "неверное значение".into();
                return;
            }
        }
        self.vpn_apply(&c);
    }

    fn key_vpn(&mut self, k: KeyCode) -> Args {
        let del = self.v.confirm_del.take();
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Tab => self.v.tab = (self.v.tab + 1) % VPN_TABS.len(),
            KeyCode::BackTab => self.v.tab = (self.v.tab + VPN_TABS.len() - 1) % VPN_TABS.len(),
            KeyCode::Char(c @ '1'..='3') => self.v.tab = c as usize - '1' as usize,
            KeyCode::Char('s') => return cmd(&["vpn", if vpn::service_active() { "stop" } else { "start" }]),
            _ => {
                return match self.v.tab {
                    0 => self.key_vpn_servers(k),
                    1 => self.key_vpn_subs(k, del),
                    _ => self.key_vpn_opts(k),
                }
            }
        }
        None
    }

    fn key_vpn_servers(&mut self, k: KeyCode) -> Args {
        let groups = &self.v.snap.groups;
        let Some(g) = groups.get(self.v.group).cloned() else {
            self.msg = "VPN не запущен — s, чтобы включить".into();
            return None;
        };
        let sel = self.v.nodes.selected().unwrap_or(0);
        match k {
            KeyCode::Left | KeyCode::Char('h') => {
                self.v.group_set = true;
                self.v.group = (self.v.group + groups.len() - 1) % groups.len();
                self.v.nodes.select(Some(0));
            }
            KeyCode::Right | KeyCode::Char('l') => {
                self.v.group_set = true;
                self.v.group = (self.v.group + 1) % groups.len();
                self.v.nodes.select(Some(0));
            }
            KeyCode::Up | KeyCode::Char('k') => self.v.nodes.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.nodes.select(Some((sel + 1).min(g.all.len().saturating_sub(1)))),
            KeyCode::Enter => {
                let Some(n) = g.all.get(sel) else { return None };
                if g.kind != "Selector" {
                    self.msg = format!("«{}» выбирает сервер сама ({})", g.name, g.kind);
                    return None;
                }
                self.msg = match vpn::select(&g.name, n) {
                    Ok(()) => format!("{} → {n}", g.name),
                    Err(e) => format!("ошибка: {e}"),
                };
                self.v.at = None;
            }
            KeyCode::Char('t') if self.v.testing.is_none() => {
                let (tx, rx) = mpsc::channel();
                let name = g.name.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(match vpn::group_delay(&name) {
                        Ok(m) => format!("«{name}»: отвечают {} из {}", m.values().filter(|d| **d > 0).count(), m.len()),
                        Err(e) if e.contains("timeout") => format!("«{name}»: ни один сервер не ответил"),
                        Err(e) => format!("замер: {e}"),
                    });
                });
                self.v.testing = Some(rx);
                self.msg = format!("замеряю задержку серверов «{}»...", g.name);
            }
            _ => {}
        }
        None
    }

    fn key_vpn_subs(&mut self, k: KeyCode, del: Option<usize>) -> Args {
        let subs = vpn::load_state().subs;
        let sel = self.v.subs.selected().unwrap_or(0);
        let n = (sel + 1).to_string();
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.v.subs.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.subs.select(Some((sel + 1).min(subs.len().saturating_sub(1)))),
            // адрес подписки — секрет: его спрашивает `upd vpn add` в терминале, а не TUI
            KeyCode::Char('n') | KeyCode::Char('a') => return cmd(&["vpn", "add"]),
            KeyCode::Char('u') if !subs.is_empty() => return cmd(&["vpn", "update"]),
            KeyCode::Enter if sel < subs.len() => return cmd(&["vpn", "use", &n]),
            KeyCode::Char('x') | KeyCode::Delete if sel < subs.len() => {
                if del == Some(sel) {
                    return cmd(&["vpn", "del", &n]);
                }
                self.v.confirm_del = Some(sel);
                self.msg = format!("удалить «{}»? нажми x ещё раз", subs[sel].name);
            }
            _ => {}
        }
        None
    }

    fn key_vpn_opts(&mut self, k: KeyCode) -> Args {
        let opts = self.vpn_opts();
        let sel = self.v.opts.selected().unwrap_or(0).min(opts.len() - 1);
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.v.opts.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.opts.select(Some((sel + 1).min(opts.len() - 1))),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let mut c = self.cfg();
                match opts[sel].key {
                    "run" => return cmd(&["vpn", if vpn::service_active() { "stop" } else { "start" }]),
                    "tun" => return cmd(&["vpn", if c.vpn_tun { "proxy" } else { "tun" }]),
                    "rules" => return cmd(&["vpn", "rules"]),
                    "geo" => return cmd(&["vpn", "geo"]),
                    "core" => return cmd(&["vpn", "core", "update"]),
                    "mode" => {
                        c.vpn_mode = (c.vpn_mode + 1) % 3;
                        let _ = c.save();
                        let _ = vpn::write_config(&c);
                        self.msg = match vpn::running().then(|| vpn::set_mode(c.vpn_mode_name())) {
                            Some(Err(e)) => format!("ошибка: {e}"),
                            _ => "маршрутизация сохранена".into(),
                        };
                        self.v.at = None;
                    }
                    "autostart" => {
                        c.vpn_autostart = !c.vpn_autostart;
                        let _ = c.save();
                        self.msg = match unit_state(vpn::SERVICE).as_str() {
                            "" => "сохранено; служба появится после sudo upd install".into(),
                            _ => match vpn::autostart(c.vpn_autostart) {
                                Ok(()) => format!("запуск при загрузке: {}", on_off(c.vpn_autostart)),
                                Err(e) => format!("ошибка: {e}"),
                            },
                        };
                    }
                    key @ ("vpn_port" | "vpn_sub_update_h") => {
                        self.input_key = key;
                        self.input = Some(String::new());
                    }
                    key => {
                        let f = match key {
                            "auto" => &mut c.vpn_auto_select,
                            "ru" => &mut c.vpn_direct_ru,
                            "lan" => &mut c.vpn_direct_lan,
                            "dns" => &mut c.vpn_dns,
                            "ipv6" => &mut c.vpn_ipv6,
                            _ => &mut c.vpn_allow_lan,
                        };
                        *f = !*f;
                        self.vpn_apply(&c);
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn draw_vpn(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let s = &self.v.snap;
        let [info, tabs, body] = Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Min(3)]).areas(area);
        let mut l1 = vec![];
        if s.running {
            l1.push(Span::styled("● работает", Style::new().fg(Color::Green)));
            let mode = match s.mode.as_str() {
                "global" => "всё через VPN",
                "direct" => "всё напрямую",
                _ => "по правилам",
            };
            l1.push(Span::raw(format!(" · {} · {mode} · mihomo {}", if s.tun { "TUN" } else { "прокси" }, s.version)));
        } else {
            let st = unit_state(vpn::SERVICE);
            l1.push(Span::styled(if st == "failed" { "● ошибка запуска (journalctl -u upd-vpn)" } else { "○ выключен" }, Style::new().fg(if st == "failed" { Color::Red } else { Color::DarkGray })));
        }
        let st = vpn::load_state();
        if let Some(a) = st.subs.iter().find(|x| x.active) {
            l1.push(Span::styled(format!(" · «{}»{}", a.name, a.info.as_ref().map(sub_info).unwrap_or_default()), dim()));
        }
        let l2 = if s.running {
            Line::from(vec![Span::styled("маршрут: ", dim()), Span::raw(s.chain().join(" → ")), Span::styled(format!(" · ↓ {} ↑ {} · соединений {}", fmt_bytes(s.down), fmt_bytes(s.up), s.conns), dim())])
        } else if st.subs.is_empty() {
            Line::styled("нет подписки — вкладка 2, клавиша n", Style::new().fg(Color::Yellow))
        } else {
            Line::styled("s — включить", dim())
        };
        let l3 = if let Some(buf) = &self.input {
            Line::from(vec![Span::raw("Новое значение: "), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan))])
        } else if let Some(w) = vpn::flclash_running() {
            Line::styled(format!("⚠ {w}"), Style::new().fg(Color::Red))
        } else {
            Line::raw("")
        };
        f.render_widget(Paragraph::new(vec![Line::from(l1), l2, l3]), info);
        let mut tl = vec![];
        for (i, t) in VPN_TABS.iter().enumerate() {
            let txt = format!(" {} {t} ", i + 1);
            tl.push(if i == self.v.tab { Span::styled(txt, Style::new().add_modifier(Modifier::REVERSED)) } else { Span::styled(txt, dim()) });
        }
        f.render_widget(Line::from(tl), tabs);
        let blk = |t: String| Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(t);
        let hl = Style::new().add_modifier(Modifier::REVERSED);
        match self.v.tab {
            0 => {
                if s.groups.is_empty() {
                    f.render_widget(Paragraph::new(Line::styled("серверы видны, когда VPN работает", dim())).block(blk(" Серверы ".into())), body);
                    return;
                }
                let [gl, nl] = Layout::horizontal([Constraint::Percentage(35), Constraint::Min(20)]).areas(body);
                let glines: Vec<Line> = s
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let t = format!(" {} → {}", g.name, g.now);
                        if i == self.v.group {
                            Line::styled(t, Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                        } else {
                            Line::raw(t)
                        }
                    })
                    .collect();
                f.render_widget(Paragraph::new(glines).block(blk(" Группы ←→ ".into())), gl);
                let g = &s.groups[self.v.group.min(s.groups.len() - 1)];
                let rows: Vec<Row> = g
                    .all
                    .iter()
                    .map(|n| {
                        let sub = s.groups.iter().find(|x| &x.name == n);
                        Row::new(vec![
                            Cell::from(Span::styled(if *n == g.now { "●" } else { "" }, Style::new().fg(Color::Cyan))),
                            Cell::from(n.clone()),
                            Cell::from(match sub {
                                Some(x) => Span::styled(format!("группа → {}", x.now), dim()),
                                None => delay_span(s.delay.get(n)),
                            }),
                        ])
                    })
                    .collect();
                let kind = match g.kind.as_str() {
                    "Selector" => "выбор вручную",
                    "URLTest" => "самый быстрый",
                    "Fallback" => "первый живой",
                    _ => "балансировка",
                };
                let t = Table::new(rows, [Constraint::Length(2), Constraint::Min(20), Constraint::Length(24)]).row_highlight_style(hl).block(blk(format!(" {} · {kind} ", g.name)));
                f.render_stateful_widget(t, nl, &mut self.v.nodes);
            }
            1 => {
                let rows: Vec<Row> = st
                    .subs
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let info = x.info.as_ref().map(sub_info).unwrap_or_default();
                        let info = info.trim_start_matches(" · ").to_string();
                        Row::new(vec![
                            Cell::from(Span::styled(if x.active { "●" } else { "" }, Style::new().fg(Color::Cyan))),
                            Cell::from(format!("{}. {}", i + 1, x.name)),
                            Cell::from(Span::styled(x.host.clone(), dim())),
                            Cell::from(x.nodes.to_string()),
                            Cell::from(fmt_ago(x.updated)),
                            Cell::from(if x.error.is_empty() { Span::raw(info) } else { Span::styled(format!("⚠ {}", x.error), Style::new().fg(Color::Red)) }),
                        ])
                    })
                    .collect();
                let t = Table::new(rows, [Constraint::Length(2), Constraint::Length(22), Constraint::Length(20), Constraint::Length(8), Constraint::Length(12), Constraint::Min(10)])
                    .header(Row::new(vec!["", "подписка", "сервер", "узлов", "обновлена", "трафик / срок"]).style(dim()))
                    .row_highlight_style(hl)
                    .block(blk(" Подписки — ● активная ".into()));
                f.render_stateful_widget(t, body, &mut self.v.subs);
            }
            _ => {
                let rows: Vec<Row> = self.vpn_opts().into_iter().map(|o| Row::new(vec![Cell::from(o.label), Cell::from(o.value)])).collect();
                let t = Table::new(rows, [Constraint::Length(30), Constraint::Min(20)]).row_highlight_style(hl).block(blk(" Настройки ".into()));
                f.render_stateful_widget(t, body, &mut self.v.opts);
            }
        }
    }
}

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn status_lines(s: &Status) -> Vec<Line<'static>> {
    let ok = |t: String| Span::styled(t, Style::new().fg(Color::Green));
    let warn = |t: String| Span::styled(t, Style::new().fg(Color::Yellow));
    let bad = |t: String| Span::styled(t, Style::new().fg(Color::Red));
    let row = |k: &str, v: Vec<Span<'static>>| {
        let mut l = vec![Span::styled(format!("{k:<14}"), dim())];
        l.extend(v);
        Line::from(l)
    };
    let u = &s.upd;
    let mut upd = vec![];
    let total = u.list.len() + u.flatpak.len();
    if total == 0 {
        upd.push(ok("нет".into()));
    } else {
        upd.push(warn(format!("пакетов {}", u.list.len())));
        if !u.flatpak.is_empty() {
            upd.push(warn(format!(", Flatpak {}", u.flatpak.len())));
        }
        if u.downloaded {
            upd.push(ok(" · скачаны".into()));
        }
    }
    if !u.firmware.is_empty() {
        upd.push(warn(format!(" · прошивок {}", u.firmware.len())));
    }
    if !u.error.is_empty() {
        upd.push(bad(format!(" · {}", u.error)));
    }
    if !u.skipped.is_empty() {
        upd.push(warn(format!(" · {}", u.skipped)));
    }
    upd.push(Span::styled(format!(" · проверка {}", fmt_ago(u.checked)), dim()));
    let mut out = vec![row("Обновления", upd)];
    if !u.news.is_empty() {
        out.push(row("Новости Arch", vec![bad(format!("{} непрочитанных — прочитай до обновления (пункт 3)", u.news.len()))]));
    }
    out.push(row("Последнее", vec![Span::raw(fmt_time(s.last_tx))]));
    let rs = &s.restart;
    let mut rb = vec![if s.reboot || !rs.critical.is_empty() { bad("перезагрузка НУЖНА".into()) } else { ok("перезагрузка не нужна".into()) }];
    if !rs.services.is_empty() {
        rb.push(warn(format!(" · служб к перезапуску {} (пункт 7)", rs.services.len())));
    }
    out.push(row("После обновл.", rb));
    let mut net = vec![Span::raw(s.net_label.clone())];
    if s.metered {
        net.push(warn(" · лимитная".into()));
    }
    if s.battery {
        net.push(warn(" · от батареи".into()));
    }
    out.push(row("Сеть", net));
    if s.managed {
        let first = match s.pinned.first() {
            Some(p) => vec![ok(host_of(p).to_string()), Span::styled(format!(" +{}", s.pinned.len().saturating_sub(1)), dim())],
            None => vec![bad("не закреплены".into())],
        };
        let mut v = first;
        v.push(Span::styled(format!(" · замер {}", fmt_ago(s.mir.checked)), dim()));
        if !s.mir.hints.is_empty() {
            v.push(warn(format!(" · подсказок {} (пункт 4)", s.mir.hints.len())));
        }
        out.push(row("Зеркала", v));
    } else {
        out.push(row("Зеркала", vec![Span::styled(s.mirror_note.clone(), dim())]));
    }
    out.push(row("Снапшоты", vec![Span::raw(s.snapshots.clone())]));
    if !s.vpn.is_empty() {
        let v = if s.vpn.starts_with("работает") { ok(s.vpn.clone()) } else if s.vpn.starts_with("ОШИБКА") { bad(s.vpn.clone()) } else { Span::raw(s.vpn.clone()) };
        out.push(row("VPN", vec![v, Span::styled(" (пункт 5)", dim())]));
    }
    let pend = if s.pending.is_empty() { Span::raw("0") } else { warn(s.pending.len().to_string()) };
    let fail = if s.failed == 0 { Span::raw("0") } else { bad(s.failed.to_string()) };
    out.push(row(
        "Обслуживание",
        vec![
            Span::raw("новых настроек "),
            pend,
            Span::raw(format!(" · сирот {} · кэш {} · свободно {} · упавших служб ", s.orphans, fmt_bytes(s.cache), fmt_bytes(s.free))),
            fail,
        ],
    ));
    let st = |v: &str| if v == "active" { ok("вкл".into()) } else if v.is_empty() { bad("не установлено".into()) } else { bad(or_dash(v).to_string()) };
    out.push(row("Автоматика", vec![Span::raw("обновления "), st(&s.auto_timer), Span::raw(" · слежение за сетью "), st(&s.net_timer)]));
    if s.auto_timer != "active" {
        out.push(Line::styled("              установи: sudo upd install", dim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    fn dump(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn screens() {
        let b = backend::detect().unwrap();
        let mut app = App {
            b: b.as_ref(),
            st: Some(gather_status(b.as_ref())),
            rx: None,
            scr: Screen::Menu,
            sel: 0,
            msg: String::new(),
            p_title: String::new(),
            p_lines: vec![],
            p_off: 0,
            m_state: TableState::default().with_selected(Some(0)),
            input: None,
            input_key: "",
            v: VpnUi::default(),
            quit: false,
        };
        let mut t = Terminal::new(TestBackend::new(118, 30)).unwrap();
        for k in [None, Some(KeyCode::Char('4')), Some(KeyCode::Esc), Some(KeyCode::Char('5')), Some(KeyCode::Right), Some(KeyCode::Down), Some(KeyCode::Enter), Some(KeyCode::Char('2')), Some(KeyCode::Char('3')), Some(KeyCode::Esc), Some(KeyCode::Char('3'))] {
            if let Some(k) = k {
                app.key(k);
            }
            if app.scr == Screen::Vpn {
                app.v.snap = vpn::snapshot();
            }
            t.draw(|f| app.draw(f)).unwrap();
            println!("{}\n{}", dump(&t), "=".repeat(118));
        }
    }
}
