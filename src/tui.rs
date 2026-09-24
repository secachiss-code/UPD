//! Интерфейс в терминале (ratatui).

use crate::backend::{self, Backend};
use crate::common::*;
use crate::mirrors::{apply_mirrors, candidates, load_mirror_state};
use crate::{extras, gather_status, or_dash, Status};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use std::sync::mpsc;
use std::time::Duration;

#[derive(PartialEq)]
enum Screen {
    Menu,
    Pager,
    Mirrors,
}

const MENU: &[&str] = &[
    "Обновить всё",
    "Проверить и скачать обновления",
    "Что доступно",
    "Зеркала",
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
    quit: bool,
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
    fn exec(&mut self, term: &mut DefaultTerminal, args: &[&str]) -> std::io::Result<()> {
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
        Ok(())
    }

    fn pager(&mut self, title: &str, lines: Vec<String>) {
        self.scr = Screen::Pager;
        self.p_title = title.into();
        self.p_lines = if lines.is_empty() { vec!["пусто".into()] } else { lines };
        self.p_off = 0;
    }

    /// Обработка клавиши; Some(args) — запустить upd с этими аргументами.
    fn key(&mut self, k: KeyCode) -> Option<Vec<&'static str>> {
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
        }
    }

    fn key_menu(&mut self, k: KeyCode) -> Option<Vec<&'static str>> {
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

    fn activate(&mut self) -> Option<Vec<&'static str>> {
        match self.sel {
            0 => return Some(vec!["update"]),
            1 => return Some(vec!["check"]),
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
                let mut l = extras::snap_list(30);
                l.push(String::new());
                l.extend(extras::rollback_hint());
                self.pager("Снапшоты (новые сверху)", l);
            }
            5 => return Some(vec!["restart"]),
            6 => return Some(vec!["clean"]),
            7 => {
                let p = self.b.pending_configs();
                if !p.is_empty() && have("pacdiff") {
                    return Some(vec!["merge"]);
                }
                self.pager("Новые файлы настроек", if p.is_empty() { vec!["нет — всё слито".into()] } else { p });
            }
            8 => self.pager("Журнал пакетов (новые сверху)", self.b.history(500)),
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

    fn key_mirrors(&mut self, k: KeyCode) -> Option<Vec<&'static str>> {
        let managed = self.b.mirrors_managed();
        let rows = self.mirror_rows();
        let sel = self.m_state.selected().unwrap_or(0);
        let mut c = Config::load(self.b.default_mirrors());
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Up | KeyCode::Char('k') => self.m_state.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.m_state.select(Some((sel + 1).min(rows.len().saturating_sub(1)))),
            KeyCode::Char('c') if managed => return Some(vec!["mirrors", "check"]),
            KeyCode::Char('s') if managed => return Some(vec!["mirrors", "rescan"]),
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
            (_, true) => vec![("enter", "добавить"), ("esc", "отмена")],
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
                let s = format!(" {n}  {t} ");
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

fn dim() -> Style {
    Style::new().fg(Color::DarkGray)
}

fn status_lines(s: &Status) -> Vec<Line<'static>> {
    let ok = |t: String| Span::styled(t, Style::new().fg(Color::Green));
    let warn = |t: String| Span::styled(t, Style::new().fg(Color::Yellow));
    let bad = |t: String| Span::styled(t, Style::new().fg(Color::Red));
    let row = |k: &str, v: Vec<Span<'static>>| {
        let mut l = vec![Span::styled(format!("{k:<13}"), dim())];
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
        rb.push(warn(format!(" · служб к перезапуску {} (пункт 6)", rs.services.len())));
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
        out.push(Line::styled("             установи: sudo upd install", dim()));
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
            quit: false,
        };
        let mut t = Terminal::new(TestBackend::new(118, 30)).unwrap();
        for k in [None, Some(KeyCode::Char('4')), Some(KeyCode::Esc), Some(KeyCode::Char('3'))] {
            if let Some(k) = k {
                app.key(k);
            }
            t.draw(|f| app.draw(f)).unwrap();
            println!("{}\n{}", dump(&t), "=".repeat(118));
        }
    }
}
