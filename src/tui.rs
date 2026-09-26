//! Интерфейс в терминале (ratatui).

mod process;

use crate::backend::{self, Backend};
use crate::common::*;
use crate::mirrors::{apply_mirrors, candidates, load_mirror_state};
use crate::{extras, gather_status, sub_info, vpn, Status};
use ratatui::crossterm::event::{self, Event, KeyCode, KeyEventKind, KeyModifiers};
use ratatui::crossterm::terminal::{disable_raw_mode, enable_raw_mode};
use ratatui::layout::{Constraint, Layout};
use ratatui::style::{Color, Modifier, Style, Stylize};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Cell, Paragraph, Row, Table, TableState, Wrap};
use ratatui::{DefaultTerminal, Frame};
use process::ProcessSession;
use std::io::Write;
use std::sync::mpsc;
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq)]
enum Screen {
    Menu,
    Status,
    Process,
    Pager,
    Mirrors,
    Vpn,
    Aur,
    Lang,
}

#[derive(Clone, Copy, PartialEq, Debug)]
enum Item {
    Update,
    Check,
    List,
    Mirrors,
    Vpn,
    Snapshots,
    Restart,
    Clean,
    Configs,
    History,
    Aur,
    Lang,
    Quit,
}

impl Item {
    fn label(self) -> &'static str {
        match self {
            Item::Update => t!("Обновить всё"),
            Item::Check => t!("Проверить и скачать обновления"),
            Item::List => t!("Что доступно"),
            Item::Mirrors => t!("Зеркала"),
            Item::Vpn => "VPN",
            Item::Snapshots => t!("Снапшоты и откат"),
            Item::Restart => t!("Перезапуск служб"),
            Item::Clean => t!("Очистка: кэш и ненужные пакеты"),
            Item::Configs => t!("Новые файлы настроек"),
            Item::History => t!("Журнал пакетов"),
            Item::Aur => t!("AUR: поиск и установка"),
            // на двух языках — чтобы найти пункт, даже не читая текущий
            Item::Lang => if crate::i18n::cur() == crate::i18n::Lang::En { "Language" } else { t!("Язык / Language") },
            Item::Quit => t!("Выход"),
        }
    }
}

/// Пункты меню; последний (Выход) — клавиша 0.
fn menu_items(b: &dyn Backend) -> Vec<Item> {
    use Item::*;
    let mut v = vec![Update, Check, List, Mirrors, Vpn, Snapshots, Restart, Clean, Configs, History];
    if b.aur() {
        v.push(Aur);
    }
    v.push(Lang);
    v.push(Quit);
    v
}

fn menu_group(item: Item) -> Option<&'static str> {
    match item {
        Item::Update => Some(t!("Обновления")),
        Item::Mirrors => Some(t!("Сеть и VPN")),
        Item::Snapshots => Some(t!("Обслуживание")),
        Item::Aur => Some("AUR"),
        Item::Lang => Some(t!("Настройки")),
        _ => None,
    }
}

#[derive(Default)]
struct AurUi {
    query: String,
    list: Vec<extras::AurPkg>,
    table: TableState,
    rx: Option<mpsc::Receiver<Result<Vec<extras::AurPkg>, String>>>,
    status: String,
}

trait UiData {
    fn config(&self, mirrors: Vec<String>) -> Result<Config, String> {
        Config::load(mirrors)
    }
    fn mirror_state(&self) -> MirrorState {
        load_mirror_state()
    }
    fn fingerprint(&self) -> NetInfo {
        crate::common::fingerprint()
    }
    fn vpn_state(&self) -> vpn::VpnState {
        vpn::load_state()
    }
    fn service_state(&self, unit: &str) -> String {
        crate::common::unit_state(unit)
    }
    fn flclash_warning(&self) -> Option<String> {
        vpn::flclash_running()
    }
    fn geo_files(&self) -> Vec<(String, i64, u64)> {
        vpn::geo_files()
    }
    fn user_rules(&self) -> Result<Vec<String>, String> {
        vpn::user_rules()
    }
}

struct HostUiData;
impl UiData for HostUiData {}

struct App<'a> {
    b: &'a dyn Backend,
    ui: &'a dyn UiData,
    st: Option<Status>,
    rx: Option<mpsc::Receiver<Status>>,
    scr: Screen,
    sel: usize,
    msg: String,
    p_title: String,
    p_lines: Vec<String>,
    p_off: usize,
    status_off: usize,
    p_aur_rx: Option<mpsc::Receiver<Result<Vec<String>, String>>>,
    p_aur_line: Option<usize>,
    m_state: TableState,
    input: Option<String>,
    /// что вводится: "" — адрес зеркала, "aur" — поиск, иначе ключ числовой настройки VPN
    input_key: &'static str,
    v: VpnUi,
    aur: AurUi,
    lang_sel: TableState,
    process: Option<ProcessSession>,
    process_return: Screen,
    process_reported: bool,
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

fn process_key_code(code: KeyCode, modifiers: KeyModifiers) -> KeyCode {
    match code {
        KeyCode::Char(c) if modifiers.contains(KeyModifiers::CONTROL) => KeyCode::Char(crate::i18n::latin_key(c)),
        _ => code,
    }
}

pub fn run(b: &dyn Backend) -> i32 {
    let ui = HostUiData;
    let mut app = App::new(b, &ui, None, vpn::Snapshot::default());
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

impl<'a> App<'a> {
    fn new(b: &'a dyn Backend, ui: &'a dyn UiData, st: Option<Status>, snap: vpn::Snapshot) -> Self {
        let mut v = VpnUi::default();
        v.snap = snap;
        App {
            b,
            ui,
            st,
            rx: None,
            scr: Screen::Menu,
            sel: 0,
            msg: String::new(),
            p_title: String::new(),
            p_lines: vec![],
            p_off: 0,
            status_off: 0,
            p_aur_rx: None,
            p_aur_line: None,
            m_state: TableState::default().with_selected(Some(0)),
            input: None,
            input_key: "",
            v,
            aur: AurUi::default(),
            lang_sel: TableState::default(),
            process: None,
            process_return: Screen::Menu,
            process_reported: false,
            quit: false,
        }
    }

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
            self.aur_poll();
            self.poll_aur_updates();
            if self.scr == Screen::Process {
                let auto_attach = if let Some(p) = &mut self.process {
                    let _ = p.poll()?;
                    p.take_attach_request() && p.alternate_screen_active()
                } else {
                    false
                };
                if auto_attach {
                    self.attach_process(term, true)?;
                }
                self.report_process_completion();
            }
            term.draw(|f| self.draw(f))?;
            if event::poll(Duration::from_millis(200))? {
                match event::read()? {
                    Event::Resize(w, h) => {
                        if let Some(p) = &self.process {
                            let _ = p.resize(h.saturating_sub(6), w);
                        }
                    }
                    Event::Key(k) => {
                        if k.kind != KeyEventKind::Press {
                            continue;
                        }
                        if self.scr == Screen::Process {
                            if k.code == KeyCode::F(4) && self.process.as_ref().and_then(ProcessSession::finished).is_none() {
                                self.attach_process(term, false)?;
                            } else {
                                self.key_process(k.code, k.modifiers, term)?;
                            }
                            continue;
                        }
                        if k.code == KeyCode::Char('c') && k.modifiers.contains(KeyModifiers::CONTROL) {
                            break;
                        }
                        // горячие клавиши работают в любой раскладке: й → q, ن → k …; в поле ввода — как набрано
                        let code = match k.code {
                            KeyCode::Char(c) if self.input.is_none() => KeyCode::Char(crate::i18n::latin_key(c)),
                            c => c,
                        };
                        let old_screen = self.scr;
                        let old_tab = self.v.tab;
                        if let Some(args) = self.key(code) {
                            self.exec(term, &args)?;
                        }
                        if self.scr != old_screen || (self.scr == Screen::Vpn && self.v.tab != old_tab) {
                            term.clear()?;
                        }
                    }
                    _ => {}
                }
            }
        }
        Ok(())
    }

    /// Запускает команду во встроенном PTY. Редакторы и ввод URL оставляем в полном терминале.
    fn exec(&mut self, term: &mut DefaultTerminal, args: &[String]) -> std::io::Result<()> {
        if args.first().map(String::as_str) == Some("merge")
            || matches!(args, [a, b] if a == "vpn" && (b == "rules" || b == "add"))
        {
            return self.exec_external(term, args);
        }
        let size = term.size()?;
        match ProcessSession::spawn(args, size.height.saturating_sub(6), size.width) {
            Ok(process) => {
                self.process_return = self.scr;
                self.process = Some(process);
                self.process_reported = false;
                self.scr = Screen::Process;
                self.msg.clear();
            }
            Err(e) => self.msg = t!("не удалось запустить: {0}", e),
        }
        Ok(())
    }

    fn exec_external(&mut self, term: &mut DefaultTerminal, args: &[String]) -> std::io::Result<()> {
        ratatui::restore();
        let exe = std::env::current_exe().unwrap_or_else(|_| "upd".into());
        let st = std::process::Command::new(exe).args(args).arg("--pause").status();
        *term = ratatui::init();
        term.clear()?;
        self.msg = match st {
            Ok(s) if s.success() => t!("готово").into(),
            _ => t!("завершилось с ошибкой").into(),
        };
        self.reload();
        self.v.at = None;
        // после установки из AUR обновить отметки «установлен»
        if self.scr == Screen::Aur && !self.aur.query.is_empty() {
            self.aur_start(self.aur.query.clone());
        }
        Ok(())
    }

    fn report_process_completion(&mut self) {
        if self.process_reported {
            return;
        }
        let Some(code) = self.process.as_ref().and_then(ProcessSession::finished) else { return };
        self.process_reported = true;
        self.msg = if code == 0 { t!("готово").into() } else { format!("{} ({code})", t!("завершилось с ошибкой")) };
        self.reload();
        self.v.at = None;
        if self.process_return == Screen::Aur && !self.aur.query.is_empty() {
            self.aur_start(self.aur.query.clone());
        }
    }

    fn key_process(&mut self, key: KeyCode, modifiers: KeyModifiers, term: &mut DefaultTerminal) -> std::io::Result<()> {
        let Some(p) = &mut self.process else { return Ok(()) };
        if p.finished().is_some() {
            match key {
                KeyCode::Enter | KeyCode::Esc | KeyCode::Char('q') => {
                    self.process = None;
                    self.scr = self.process_return;
                    term.clear()?;
                }
                KeyCode::Up | KeyCode::PageUp => p.scroll_up(15),
                KeyCode::Down | KeyCode::PageDown => p.scroll_down(15),
                KeyCode::Home => p.scroll_up(usize::MAX),
                KeyCode::End => p.scroll_down(usize::MAX),
                _ => {}
            }
        } else if modifiers.contains(KeyModifiers::ALT) && matches!(key, KeyCode::PageUp | KeyCode::PageDown) {
            if key == KeyCode::PageUp { p.scroll_up(15) } else { p.scroll_down(15) }
        } else if let Err(e) = p.send_key(process_key_code(key, modifiers), modifiers) {
            self.msg = t!("ошибка: {0}", e);
        }
        Ok(())
    }

    /// Полный терминал включается вручную по F4 или автоматически для полноэкранного редактора.
    fn attach_process(&mut self, term: &mut DefaultTerminal, auto: bool) -> std::io::Result<()> {
        let size = term.size()?;
        if let Some(p) = &self.process {
            let _ = p.resize(size.height, size.width);
        }
        ratatui::restore();
        enable_raw_mode()?;
        let result = (|| -> std::io::Result<()> {
            let mut stdout = std::io::stdout();
            stdout.write_all(b"\x1b[2J\x1b[H")?;
            if let Some(p) = &self.process {
                if auto {
                    let raw = p.raw_history();
                    let start = [b"\x1b[?1049h".as_slice(), b"\x1b[?1047h".as_slice(), b"\x1b[?47h".as_slice()]
                        .iter()
                        .filter_map(|marker| raw.windows(marker.len()).rposition(|w| w == *marker))
                        .max()
                        .unwrap_or(0);
                    stdout.write_all(&raw[start..])?;
                } else {
                    for line in p.lines_for(40) {
                        writeln!(stdout, "{line}")?;
                    }
                }
            }
            stdout.flush()?;
            loop {
                let Some(p) = &mut self.process else { break };
                for chunk in p.poll()? {
                    stdout.write_all(&chunk)?;
                }
                stdout.flush()?;
                if p.finished().is_some() || (auto && !p.alternate_screen_active()) {
                    break;
                }
                if event::poll(Duration::from_millis(100))? {
                    match event::read()? {
                        Event::Key(k) if k.kind == KeyEventKind::Press && k.code == KeyCode::F(4) && !p.alternate_screen_active() => break,
                        Event::Key(k) if k.kind == KeyEventKind::Press => { let _ = p.send_key(process_key_code(k.code, k.modifiers), k.modifiers); }
                        Event::Resize(w, h) => { let _ = p.resize(h, w); }
                        _ => {}
                    }
                }
            }
            Ok(())
        })();
        let _ = disable_raw_mode();
        *term = ratatui::init();
        term.clear()?;
        if let Some(p) = &self.process {
            let _ = p.resize(size.height.saturating_sub(6), size.width);
        }
        result
    }

    fn pager(&mut self, title: &str, lines: Vec<String>) {
        self.scr = Screen::Pager;
        self.p_title = title.into();
        self.p_lines = if lines.is_empty() { vec![t!("пусто").into()] } else { lines };
        self.p_off = 0;
        self.p_aur_rx = None;
        self.p_aur_line = None;
    }

    fn start_aur_updates(&mut self) {
        if !self.b.aur() || !self.cfg().map(|c| c.aur).unwrap_or(false) {
            return;
        }
        self.p_lines.push(String::new());
        self.p_lines.push("── AUR ──".into());
        if extras::aur_helper().is_none() {
            self.p_lines.push(t!("нет paru/yay").into());
            return;
        }
        let Some(user) = invoking_user() else {
            self.p_lines.push(t!("AUR: нужен обычный пользователь").into());
            return;
        };
        self.p_aur_line = Some(self.p_lines.len());
        self.p_lines.push(t!("обновляю состояние...").into());
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            let _ = tx.send(extras::aur_updates(&user));
        });
        self.p_aur_rx = Some(rx);
    }

    fn poll_aur_updates(&mut self) {
        let Some(rx) = &self.p_aur_rx else { return };
        let result = match rx.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(t!("завершилось с ошибкой").into()),
        };
        self.p_aur_rx = None;
        if let Some(at) = self.p_aur_line.take() {
            let lines = match result {
                Ok(list) if list.is_empty() => vec![t!("нет").into()],
                Ok(list) => list,
                Err(e) => vec![format!("⚠ {e}")],
            };
            self.p_lines.splice(at..=at, lines);
        }
    }

    /// Обработка клавиши; Some(args) — запустить upd с этими аргументами.
    fn key(&mut self, k: KeyCode) -> Args {
        if self.input.is_some() {
            self.key_input(k);
            return None;
        }
        match self.scr {
            Screen::Menu => self.key_menu(k),
            Screen::Process => None,
            Screen::Status => {
                let n = self.st.as_ref().map(|s| status_lines(s).len()).unwrap_or(1);
                match k {
                    KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
                    KeyCode::Up | KeyCode::Char('k') => self.status_off = self.status_off.saturating_sub(1),
                    KeyCode::Down | KeyCode::Char('j') => self.status_off = (self.status_off + 1).min(n.saturating_sub(1)),
                    KeyCode::PageUp => self.status_off = self.status_off.saturating_sub(10),
                    KeyCode::PageDown | KeyCode::Char(' ') => self.status_off = (self.status_off + 10).min(n.saturating_sub(1)),
                    KeyCode::Home => self.status_off = 0,
                    _ => {}
                }
                None
            }
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
            Screen::Aur => self.key_aur(k),
            Screen::Lang => self.key_lang(k),
        }
    }

    fn key_menu(&mut self, k: KeyCode) -> Args {
        let n_items = menu_items(self.b).len();
        match k {
            KeyCode::Char('q') | KeyCode::Esc => self.quit = true,
            KeyCode::Up | KeyCode::Char('k') => self.sel = (self.sel + n_items - 1) % n_items,
            KeyCode::Down | KeyCode::Char('j') | KeyCode::Tab => self.sel = (self.sel + 1) % n_items,
            KeyCode::Char('r') => {
                self.msg = t!("обновляю состояние...").into();
                self.reload();
            }
            KeyCode::Char('i') => {
                self.status_off = 0;
                self.scr = Screen::Status;
            }
            KeyCode::Enter => return self.activate(),
            KeyCode::Char(c) if c.is_ascii_digit() => {
                let n = c.to_digit(10).unwrap() as usize;
                self.sel = if n == 0 { n_items - 1 } else { (n - 1).min(n_items - 1) };
                return self.activate();
            }
            _ => {}
        }
        None
    }

    fn activate(&mut self) -> Args {
        let item = menu_items(self.b).get(self.sel).copied().unwrap_or(Item::Quit);
        match item {
            Item::Update => return cmd(&["update"]),
            Item::Check => return cmd(&["check"]),
            Item::List => {
                let u: UpdState = load_json("updates.json");
                let mut l = vec![t!("проверено: {}", fmt_time(u.checked))];
                if !u.skipped.is_empty() {
                    l.push(format!("⏸ {}", u.skipped));
                }
                if !u.error.is_empty() {
                    l.push(format!("⚠ {}", u.error));
                }
                if !u.flatpak_error.is_empty() {
                    l.push(t!("⚠ Flatpak: ошибка проверки: {}", u.flatpak_error));
                }
                if !u.firmware_error.is_empty() {
                    l.push(t!("⚠ Прошивки: ошибка проверки: {}", u.firmware_error));
                }
                if !u.news.is_empty() {
                    l.push(String::new());
                    l.push(t!("── Новости Arch (прочитай до обновления) ──").into());
                    for n in &u.news {
                        l.push(format!("{}  {}", fmt_time(n.date), n.title));
                        l.push(format!("      {}", n.link));
                    }
                }
                let size = match u.download_size {
                    Some(bytes) => format!(", {}", fmt_bytes(bytes)),
                    None if !u.list.is_empty() => t!(", размер неизвестен").into(),
                    None => String::new(),
                };
                l.push(String::new());
                l.push(t!("── Пакеты: {}{2}{} ──", u.list.len(), if u.downloaded { t!(", скачаны") } else { "" }, size));
                l.extend(u.list.iter().cloned());
                for (t, list) in [("Flatpak", &u.flatpak), (t!("Прошивки"), &u.firmware)] {
                    if !list.is_empty() {
                        l.push(String::new());
                        l.push(format!("── {t}: {} ──", list.len()));
                        l.extend(list.iter().cloned());
                    }
                }
                self.pager(t!("Что доступно"), l);
                self.start_aur_updates();
            }
            Item::Mirrors => {
                self.scr = Screen::Mirrors;
                self.m_state.select(Some(0));
            }
            Item::Vpn => {
                self.scr = Screen::Vpn;
                self.v.at = None;
                for t in [&mut self.v.nodes, &mut self.v.subs, &mut self.v.opts] {
                    if t.selected().is_none() {
                        t.select(Some(0));
                    }
                }
            }
            Item::Snapshots => {
                let mut l = extras::snap_list(30);
                l.push(String::new());
                l.extend(extras::rollback_hint());
                self.pager(t!("Снапшоты (новые сверху)"), l);
            }
            Item::Restart => return cmd(&["restart"]),
            Item::Clean => return cmd(&["clean"]),
            Item::Configs => {
                let p = self.b.pending_configs();
                if !p.is_empty() && have("pacdiff") {
                    return cmd(&["merge"]);
                }
                self.pager(t!("Новые файлы настроек"), if p.is_empty() { vec![t!("нет — всё слито").into()] } else { p });
            }
            Item::History => self.pager(t!("Журнал пакетов (новые сверху)"), self.b.history(500)),
            Item::Aur => {
                self.scr = Screen::Aur;
                if self.aur.list.is_empty() {
                    self.input_key = "aur";
                    self.input = Some(self.aur.query.clone());
                }
            }
            Item::Lang => {
                self.scr = Screen::Lang;
                let cur = crate::i18n::ALL.iter().position(|l| *l == crate::i18n::cur()).unwrap_or(0);
                self.lang_sel.select(Some(cur));
            }
            Item::Quit => self.quit = true,
        }
        None
    }

    // ---------- язык ----------

    fn key_lang(&mut self, k: KeyCode) -> Args {
        let n = crate::i18n::ALL.len();
        let sel = self.lang_sel.selected().unwrap_or(0);
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Up | KeyCode::Char('k') => self.lang_sel.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.lang_sel.select(Some((sel + 1).min(n - 1))),
            KeyCode::Enter => {
                let l = crate::i18n::ALL[sel];
                let mut c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return None;
                    }
                };
                c.lang = l.code().into();
                match c.save() {
                    Ok(()) => {
                        crate::i18n::set(l);
                        self.msg = t!("язык: {}", l.name());
                        self.scr = Screen::Menu;
                        // статус собран на прежнем языке
                        self.reload();
                    }
                    Err(e) => self.msg = t!("не удалось сохранить настройки: {}", e),
                }
            }
            _ => {}
        }
        None
    }

    fn draw_lang(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let rows: Vec<Row> = crate::i18n::ALL
            .iter()
            .map(|l| Row::new(vec![Cell::from(if *l == crate::i18n::cur() { "●" } else { "" }), Cell::from(l.code()), Cell::from(l.name())]))
            .collect();
        let t = Table::new(rows, [Constraint::Length(2), Constraint::Length(4), Constraint::Min(10)])
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(" Язык / Language "));
        f.render_stateful_widget(t, area, &mut self.lang_sel);
    }

    // ---------- AUR ----------

    fn aur_start(&mut self, q: String) {
        let (tx, rx) = mpsc::channel();
        let q2 = q.clone();
        std::thread::spawn(move || {
            let _ = tx.send(extras::aur_search(&q2));
        });
        self.aur.query = q;
        self.aur.rx = Some(rx);
        self.aur.status = t!("ищу в AUR...").into();
    }

    fn aur_poll(&mut self) {
        let Some(rx) = &self.aur.rx else { return };
        let Ok(r) = rx.try_recv() else { return };
        self.aur.rx = None;
        match r {
            Ok(list) => {
                self.aur.status = if list.is_empty() { t!("по запросу «{}» ничего не найдено", self.aur.query) } else { t!("найдено: {}", list.len()) };
                self.aur.list = list;
                self.aur.table.select(Some(0));
            }
            Err(e) => self.aur.status = e,
        }
    }

    fn key_aur(&mut self, k: KeyCode) -> Args {
        let n = self.aur.list.len();
        let sel = self.aur.table.selected().unwrap_or(0);
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Char('/') | KeyCode::Char('s') => {
                self.input_key = "aur";
                self.input = Some(String::new());
            }
            KeyCode::Up | KeyCode::Char('k') => self.aur.table.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.aur.table.select(Some((sel + 1).min(n.saturating_sub(1)))),
            KeyCode::PageUp => self.aur.table.select(Some(sel.saturating_sub(15))),
            KeyCode::PageDown => self.aur.table.select(Some((sel + 15).min(n.saturating_sub(1)))),
            KeyCode::Enter | KeyCode::Char('i') => {
                if let Some(p) = self.aur.list.get(sel) {
                    let name = p.name.clone();
                    return cmd(&["aur", "install", &name]);
                }
            }
            _ => {}
        }
        None
    }

    fn draw_aur(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let [info, table, desc] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)]).areas(area);
        let head = match &self.input {
            Some(buf) if self.input_key == "aur" => Line::from(vec![Span::raw(t!("Поиск в AUR: ")), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan))]),
            _ => Line::styled(
                if self.aur.status.is_empty() { t!("нажми / или s, чтобы искать").to_string() } else { format!("«{}» · {}", self.aur.query, self.aur.status) },
                dim(),
            ),
        };
        f.render_widget(Paragraph::new(head), info);
        let rows: Vec<Row> = self
            .aur
            .list
            .iter()
            .map(|p| {
                let mark = match &p.installed {
                    Some(v) if v == &p.version => Span::styled("●", Style::new().fg(Color::Green)),
                    Some(_) => Span::styled("↑", Style::new().fg(Color::Yellow)),
                    None => Span::raw(""),
                };
                let ver = if p.out_of_date { Span::styled(p.version.clone(), Style::new().fg(Color::Red)) } else { Span::raw(p.version.clone()) };
                Row::new(vec![Cell::from(mark), Cell::from(p.name.clone()), Cell::from(ver), Cell::from(p.votes.to_string()), Cell::from(format!("{:.2}", p.popularity))])
            })
            .collect();
        let t = Table::new(rows, [Constraint::Length(2), Constraint::Min(20), Constraint::Length(22), Constraint::Length(7), Constraint::Length(8)])
            .header(Row::new(vec!["", t!("пакет"), t!("версия"), t!("голоса"), t!("попул.")]).style(dim()))
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(" AUR "));
        f.render_stateful_widget(t, table, &mut self.aur.table);
        if let Some(p) = self.aur.table.selected().and_then(|i| self.aur.list.get(i)) {
            let mut l = vec![Line::raw(p.desc.clone())];
            let mut s = vec![];
            if let Some(v) = &p.installed {
                s.push(t!("установлен {0}", v));
            }
            if p.out_of_date {
                s.push(t!("помечен устаревшим").into());
            }
            s.push(format!("https://aur.archlinux.org/packages/{}", p.name));
            l.push(Line::styled(s.join(" · "), dim()));
            f.render_widget(Paragraph::new(l).wrap(Wrap { trim: true }), desc);
        }
    }

    fn mirror_rows(&self, c: &Config) -> Vec<Probe> {
        let st = self.ui.mirror_state();
        let mut r: Vec<Probe> = candidates(self.b, c, None)
            .into_iter()
            .map(|cd| {
                let mut p = st.results.iter().find(|p| p.url == cd.url).cloned().unwrap_or(Probe { url: cd.url.clone(), err: "not measured".into(), ..Default::default() });
                p.src = cd.src.into();
                p
            })
            .collect();
        let shown = |p: &Probe| if p.score > 0.0 { p.score } else { p.speed };
        r.sort_by(|a, b| b.ok.cmp(&a.ok).then(shown(b).total_cmp(&shown(a))));
        r
    }

    fn key_mirrors(&mut self, k: KeyCode) -> Args {
        if matches!(k, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace) {
            self.scr = Screen::Menu;
            return None;
        }
        let managed = self.b.mirrors_managed();
        let mut c = match self.cfg() {
            Ok(c) => c,
            Err(e) => {
                self.msg = t!("конфиг не прочитан: {0}", e);
                return None;
            }
        };
        let rows = self.mirror_rows(&c);
        let sel = self.m_state.selected().unwrap_or(0);
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.m_state.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.m_state.select(Some((sel + 1).min(rows.len().saturating_sub(1)))),
            KeyCode::Char('c') if managed => return cmd(&["mirrors", "check"]),
            KeyCode::Char('s') if managed => return cmd(&["mirrors", "rescan"]),
            KeyCode::Char('a') if managed => {
                let log = std::cell::RefCell::new(vec![]);
                if let Err(e) = apply_mirrors(self.b, &c, &self.ui.mirror_state(), None, &|s| log.borrow_mut().push(s.to_string())) {
                    log.borrow_mut().push(e);
                }
                self.msg = log.into_inner().join("; ");
                self.reload();
            }
            KeyCode::Char('n') if managed => {
                self.input_key = "";
                self.input = Some(String::new());
            }
            KeyCode::Char('x') | KeyCode::Delete if managed => {
                if let Some(p) = rows.get(sel) {
                    if contains(&c.mirrors, &p.url) {
                        c.mirrors.retain(|m| m != &p.url);
                        self.msg = match c.save() {
                            Ok(()) => t!("удалено из конфига").into(),
                            Err(e) => t!("не сохранено: {0}", e),
                        };
                    } else {
                        self.msg = t!("это зеркало не из конфига — его подбирает автоматика").into();
                    }
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') if managed => {
                c.keep = (c.keep + 1).min(10);
                self.msg = match c.save() {
                    Ok(()) => t!("закреплять {} — нажми a, чтобы применить", c.keep),
                    Err(e) => t!("не сохранено: {0}", e),
                };
            }
            KeyCode::Char('-') if managed => {
                c.keep = c.keep.saturating_sub(1).max(1);
                self.msg = match c.save() {
                    Ok(()) => t!("закреплять {} — нажми a, чтобы применить", c.keep),
                    Err(e) => t!("не сохранено: {0}", e),
                };
            }
            _ => {}
        }
        None
    }

    fn key_input(&mut self, k: KeyCode) {
        let Some(buf) = self.input.as_mut() else { return };
        match k {
            KeyCode::Esc => {
                self.input = None;
                self.input_key = "";
            }
            KeyCode::Backspace => {
                buf.pop();
            }
            KeyCode::Char(ch) => buf.push(ch),
            KeyCode::Enter => {
                let u = buf.trim().to_string();
                self.input = None;
                let input_key = std::mem::take(&mut self.input_key);
                if u.is_empty() {
                    return;
                }
                if input_key == "aur" {
                    return self.aur_start(u);
                }
                if !input_key.is_empty() {
                    return self.vpn_set_number(input_key, &u);
                }
                if let Err(e) = self.b.valid_mirror(&u) {
                    self.msg = t!("неверный URL: {0}", e);
                    return;
                }
                let mut c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return;
                    }
                };
                if !contains(&c.mirrors, &u) {
                    c.mirrors.push(u);
                    if let Err(e) = c.save() {
                        self.msg = t!("не сохранено: {0}", e);
                        return;
                    }
                }
                self.msg = t!("добавлено — нажми c, чтобы замерить").into();
            }
            _ => {}
        }
    }

    // ---------- отрисовка ----------

    fn draw(&mut self, f: &mut Frame) {
        let [head, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)]).areas(f.area());
        f.render_widget(
            Line::from(vec![" upd ".bold().black().on_cyan(), Span::raw(" "), Span::styled(t!("обновление системы · {}", self.b.name()), dim())]),
            head,
        );
        let keys: Vec<(&str, &str)> = match (&self.scr, self.input.is_some()) {
            (_, true) => vec![("enter", t!("сохранить")), ("esc", t!("отмена"))],
            (Screen::Vpn, _) => {
                let mut k = vec![("q", t!("назад")), ("tab 1-3", t!("вкладка")), ("s", t!("вкл/выкл"))];
                match self.v.tab {
                    0 => k.extend([("←→", t!("группа")), ("enter", t!("выбрать сервер")), ("t", t!("замерить задержку"))]),
                    1 => k.extend([("enter", t!("сделать активной")), ("n", t!("добавить")), ("u", t!("обновить")), ("x", t!("удалить"))]),
                    _ => k.extend([("enter", t!("изменить"))]),
                }
                k
            }
            (Screen::Menu, _) => vec![("↑↓", t!("выбор")), ("enter", t!("выполнить")), ("1-9,0", t!("сразу")), ("i", t!(" Состояние ").trim()), ("r", t!("обновить")), ("q", t!("выход"))],
            (Screen::Process, _) if self.process.as_ref().and_then(ProcessSession::finished).is_some() => vec![("enter/q", t!("назад")), ("PgUp/PgDn", t!("листать"))],
            (Screen::Process, _) => vec![("F4", t!("полный терминал")), ("Alt+PgUp/PgDn", t!("листать")), ("Ctrl+C", t!("отмена"))],
            (Screen::Status, _) => vec![("↑↓ PgUp PgDn", t!("листать")), ("q", t!("назад"))],
            (Screen::Pager, _) => vec![("↑↓ PgUp PgDn", t!("листать")), ("q", t!("назад"))],
            (Screen::Mirrors, _) if self.b.mirrors_managed() => vec![("q", t!("назад")), ("c", t!("замерить")), ("s", t!("искать заново")), ("a", t!("применить")), ("n", t!("добавить")), ("x", t!("удалить")), ("+/-", t!("сколько закреплять"))],
            (Screen::Mirrors, _) => vec![("q", t!("назад"))],
            (Screen::Lang, _) => vec![("↑↓", t!("выбор")), ("enter", t!("выбрать")), ("q", t!("назад"))],
            (Screen::Aur, _) => vec![("q", t!("назад")), ("/ s", t!("искать")), ("↑↓", t!("выбор")), ("enter", t!("установить"))],
        };
        let mut help: Vec<Line> = vec![];
        let mut kl: Vec<Span> = vec![];
        let mut used = 0;
        let width = foot.width as usize;
        for (k, d) in keys {
            let key = Span::styled(format!(" {k} "), Style::new().fg(Color::Cyan));
            let desc = Span::styled(d.to_string(), dim());
            let item_width = key.width() + desc.width();
            if used > 0 && used + item_width > width {
                help.push(Line::from(std::mem::take(&mut kl)));
                used = 0;
            }
            if help.len() >= 2 {
                break;
            }
            used += item_width;
            kl.push(key);
            kl.push(desc);
        }
        help.push(Line::from(kl));
        help.push(Line::from(Span::styled(self.msg.clone(), Style::new().fg(Color::Yellow))));
        f.render_widget(Paragraph::new(help), foot);
        match self.scr {
            Screen::Menu => self.draw_menu(f, body),
            Screen::Process => {
                if let Some(p) = &self.process {
                    let [info, log] = Layout::vertical([Constraint::Length(1), Constraint::Min(3)]).areas(body);
                    let state = match p.finished() {
                        Some(0) => Span::styled(t!("готово"), Style::new().fg(Color::Green)),
                        Some(_) => Span::styled(t!("завершилось с ошибкой"), Style::new().fg(Color::Red)),
                        None => Span::styled(t!("выполняется..."), Style::new().fg(Color::Yellow)),
                    };
                    f.render_widget(Line::from(vec![state, Span::styled(format!(" · {}", t!("{} с", p.elapsed_secs())), dim())]), info);
                    let lines: Vec<Line> = p.lines_for(log.height.saturating_sub(2) as usize).into_iter().map(|line| Line::raw(tui_vpn_label(&line))).collect();
                    f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(format!(" {} ", p.title))), log);
                }
            }
            Screen::Status => {
                let lines = self.st.as_ref().map(status_lines).unwrap_or_else(|| vec![Line::styled(t!("загружаю состояние..."), dim())]);
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((self.status_off.min(u16::MAX as usize) as u16, 0)).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(t!(" Состояние "))), body);
            }
            Screen::Pager => {
                let h = body.height.saturating_sub(2) as usize;
                let end = (self.p_off + h).min(self.p_lines.len());
                let lines: Vec<Line> = self.p_lines[self.p_off.min(end)..end].iter().map(|l| Line::raw(l.clone())).collect();
                let title = t!(" {} · {}–{} из {} ", self.p_title, self.p_off + 1, end, self.p_lines.len());
                f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).title(title)), body);
            }
            Screen::Mirrors => self.draw_mirrors(f, body),
            Screen::Vpn => self.draw_vpn(f, body),
            Screen::Aur => self.draw_aur(f, body),
            Screen::Lang => self.draw_lang(f, body),
        }
    }

    fn draw_menu(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let lines = match &self.st {
            None => vec![Line::styled(t!("загружаю состояние..."), dim())],
            Some(s) => status_lines(s),
        };
        let items = menu_items(self.b);
        let menu_rows = items.len() + items.iter().filter(|item| menu_group(**item).is_some()).count();
        let [top, bottom] = if area.width >= 110 && area.height >= 20 {
            Layout::horizontal([Constraint::Percentage(56), Constraint::Percentage(44)]).areas(area)
        } else {
            let top_height = area.height.saturating_sub(menu_rows as u16).clamp(5, 7).min(area.height.saturating_sub(3));
            Layout::vertical([Constraint::Length(top_height), Constraint::Min(3)]).areas(area)
        };
        f.render_widget(
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(format!("{} · i", t!(" Состояние ").trim()))),
            top,
        );
        let n_items = items.len();
        let mut selected_row = 0;
        let mut rows = Vec::with_capacity(menu_rows);
        for (i, item) in items.iter().enumerate() {
            if let Some(group) = menu_group(*item) {
                rows.push(Line::styled(format!(" {group}"), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
            }
            let n = if i + 1 == n_items { "0".to_string() } else if i < 9 { (i + 1).to_string() } else { String::new() };
            let line = format!(" {n:>2}  {} ", item.label());
            if i == self.sel {
                selected_row = rows.len();
                rows.push(Line::styled(line, Style::new().add_modifier(Modifier::REVERSED)));
            } else {
                rows.push(Line::raw(line));
            }
        }
        let visible = bottom.height as usize;
        let offset = selected_row.saturating_sub(visible.saturating_sub(1)).min(rows.len().saturating_sub(visible));
        f.render_widget(Paragraph::new(rows).scroll((offset.min(u16::MAX as usize) as u16, 0)), bottom);
    }

    fn draw_mirrors(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        if !self.b.mirrors_managed() {
            f.render_widget(Paragraph::new(self.b.mirror_note()).block(Block::bordered().title(t!(" Зеркала "))), area);
            return;
        }
        let c = match self.cfg() {
            Ok(c) => c,
            Err(e) => {
                f.render_widget(Paragraph::new(t!("конфиг не прочитан: {0}", e)).block(Block::bordered().title(t!(" Зеркала "))), area);
                return;
            }
        };
        let st = self.ui.mirror_state();
        let net = self.ui.fingerprint();
        let pinned = self.b.pinned();
        let hints = st.hints.len() as u16;
        let [info, table, hint_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(3), Constraint::Length(if hints > 0 { hints * 2 + 1 } else { 0 })]).areas(area);
        let mut info_lines = vec![Line::from(vec![
            Span::styled(t!("сеть: "), dim()),
            Span::raw(net.label.clone()),
            Span::styled(t!(" · замер: {} · закреплять лучших: {}", fmt_ago(st.checked), c.keep), dim()),
        ])];
        if let Some(buf) = &self.input {
            info_lines.push(Line::from(vec![Span::raw(t!("Новое зеркало: ")), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan))]));
        } else if st.pending_apply {
            let error = if st.apply_error.is_empty() { t!("повтор будет при следующей проверке сети") } else { st.apply_error.as_str() };
            info_lines.push(Line::styled(t!("⚠ зеркала ожидают применения: {0}", error), Style::new().fg(Color::Red)));
        } else {
            info_lines.push(Line::styled(t!("● закреплено · скорость сглажена по истории замеров в этой сети · отстающие зеркала не берутся"), dim()));
        }
        f.render_widget(Paragraph::new(info_lines), info);
        let rows: Vec<Row> = self
            .mirror_rows(&c)
            .into_iter()
            .map(|p| {
                let color = if p.ok {
                    Color::Green
                } else if p.err == "not measured" {
                    Color::DarkGray
                } else {
                    Color::Red
                };
                Row::new(vec![
                    Cell::from(Span::styled(if contains(&pinned, &p.url) { "●" } else { "" }, Style::new().fg(Color::Cyan))),
                    Cell::from(crate::i18n::tr_data(&p.src)),
                    Cell::from(Span::styled(fmt_speed(&p), Style::new().fg(color))),
                    Cell::from(p.lag_h.map(|l| t!("{0} ч", l)).unwrap_or_default()),
                    Cell::from(p.url.clone()),
                ])
            })
            .collect();
        let t = Table::new(rows, [Constraint::Length(3), Constraint::Length(7), Constraint::Length(13), Constraint::Length(8), Constraint::Min(20)])
            .header(Row::new(vec!["", t!("откуда"), t!("скорость"), t!("отстаёт"), t!("адрес")]).style(dim()))
            .row_highlight_style(Style::new().add_modifier(Modifier::REVERSED))
            .block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(t!(" Зеркала ")));
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
    if b { t!("вкл") } else { t!("выкл") }.into()
}

fn delay_span(d: Option<&u64>) -> Span<'static> {
    match d {
        None => Span::styled("—", dim()),
        Some(0) => Span::styled("✗", Style::new().fg(Color::Red)),
        Some(&d) => Span::styled(t!("{0} мс", d), Style::new().fg(if d < 300 { Color::Green } else if d < 800 { Color::Yellow } else { Color::Red })),
    }
}

/// Флаги из двух regional-indicator символов терминалы рисуют с разной шириной.
/// В TUI показываем код страны обычными символами, чтобы при перерисовке не оставались хвосты.
fn tui_vpn_label(name: &str) -> String {
    let label = vpn::label(name);
    let mut chars = label.chars().peekable();
    let mut out = String::with_capacity(label.len());
    while let Some(ch) = chars.next() {
        let first = ch as u32;
        if (0x1f1e6..=0x1f1ff).contains(&first) {
            if let Some(next) = chars.peek().copied() {
                let second = next as u32;
                if (0x1f1e6..=0x1f1ff).contains(&second) {
                    chars.next();
                    out.push('[');
                    out.push(char::from_u32(u32::from(b'A') + first - 0x1f1e6).unwrap_or('?'));
                    out.push(char::from_u32(u32::from(b'A') + second - 0x1f1e6).unwrap_or('?'));
                    out.push(']');
                    continue;
                }
            }
        }
        out.push(ch);
    }
    out
}

impl App<'_> {
    fn cfg(&self) -> Result<Config, String> {
        self.ui.config(self.b.default_mirrors())
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

    fn vpn_opts(&self) -> Result<Vec<Opt>, String> {
        let c = self.cfg()?;
        let rules_count = self.ui.user_rules()?.len();
        let st = self.ui.vpn_state();
        let unit = self.ui.service_state(vpn::SERVICE);
        let geo = self.ui.geo_files().iter().map(|g| g.1).filter(|t| *t > 0).min();
        let core = match (st.core_version.as_str(), st.core_latest.as_str()) {
            ("", _) => t!("не скачано — скачать").to_string(),
            (v, l) if !l.is_empty() && l != v => t!("{0} → есть {1}, обновить", v, l),
            (v, _) => t!("{1}, проверено {}", fmt_ago(st.checked), v),
        };
        let o = |key, label, value| Opt { key, label, value };
        Ok(vec![
            o(
                "run",
                "VPN",
                match unit.as_str() {
                    "active" => t!("работает — выключить"),
                    "" => t!("служба не установлена — sudo upd install"),
                    "failed" => t!("ошибка запуска — включить снова"),
                    _ => t!("выключен — включить"),
                }
                .into(),
            ),
            o("tun", t!("Режим"), if c.vpn_tun { t!("TUN — вся система").into() } else { t!("только прокси 127.0.0.1:{}", c.vpn_port) }),
            o("mode", t!("Маршрутизация"), [t!("по правилам"), t!("всё через VPN"), t!("всё напрямую")][c.vpn_mode.min(2) as usize].into()),
            o("autostart", t!("Запуск при загрузке"), on_off(c.vpn_autostart)),
            o("auto", t!("Автовыбор сервера (⚡ Авто)"), on_off(c.vpn_auto_select)),
            o("ru", t!("Россия напрямую (геофайлы)"), on_off(c.vpn_direct_ru)),
            o("lan", t!("Локальная сеть напрямую"), on_off(c.vpn_direct_lan)),
            o("dns", t!("Свой DNS (fake-ip)"), on_off(c.vpn_dns)),
            o("ipv6", "IPv6", on_off(c.vpn_ipv6)),
            o("allow_lan", t!("Прокси для устройств в сети"), on_off(c.vpn_allow_lan)),
            o("vpn_port", t!("Порт прокси"), c.vpn_port.to_string()),
            o("vpn_sub_update_h", t!("Обновлять подписки, ч"), c.vpn_sub_update_h.to_string()),
            o("rules", t!("Свои правила"), t!("{} шт. — открыть редактор", rules_count)),
            o("core", t!("Ядро mihomo"), core),
            o("geo", t!("Геофайлы"), geo.map(|t| t!("от {} — обновить", fmt_ago(t))).unwrap_or_else(|| t!("не скачаны — скачать").into())),
        ])
    }

    /// Сохранить настройки и применить к работающему ядру; итог — в строку сообщений.
    fn vpn_apply(&mut self, c: &Config) {
        if let Err(e) = c.save() {
            self.msg = t!("не сохранено: {0}", e);
            return;
        }
        let log = std::cell::RefCell::new(vec![]);
        if let Err(e) = vpn::apply(c, &|s| log.borrow_mut().push(s.to_string())) {
            log.borrow_mut().push(t!("ошибка: {0}", e));
        }
        self.msg = log.into_inner().join("; ");
        self.v.at = None;
    }

    fn vpn_set_number(&mut self, key: &str, s: &str) {
        let mut c = match self.cfg() {
            Ok(c) => c,
            Err(e) => {
                self.msg = t!("конфиг не прочитан: {0}", e);
                return;
            }
        };
        match (key, s.parse::<i64>()) {
            ("vpn_port", Ok(n)) if (1024..=65535).contains(&n) && n != 9097 => c.vpn_port = n as u16,
            ("vpn_sub_update_h", Ok(n)) if n >= 1 => c.vpn_sub_update_h = n,
            _ => {
                self.msg = t!("неверное значение").into();
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
            KeyCode::Char('s') => return cmd(&["vpn", if self.ui.service_state(vpn::SERVICE) == "active" { "stop" } else { "start" }]),
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
            self.msg = t!("VPN не запущен — s, чтобы включить").into();
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
                    self.msg = t!("«{}» выбирает сервер сама ({})", tui_vpn_label(&g.name), g.kind);
                    return None;
                }
                self.msg = match vpn::select(&g.name, n) {
                    Ok(()) => format!("{} → {}", tui_vpn_label(&g.name), tui_vpn_label(n)),
                    Err(e) => t!("ошибка: {0}", e),
                };
                self.v.at = None;
            }
            KeyCode::Char('t') if self.v.testing.is_none() => {
                let (tx, rx) = mpsc::channel();
                let name = g.name.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(match vpn::group_delay(&name) {
                        Ok(m) => t!("«{2}»: отвечают {} из {}", m.values().filter(|d| **d > 0).count(), m.len(), tui_vpn_label(&name)),
                        Err(e) if e.contains("timeout") => t!("«{0}»: ни один сервер не ответил", tui_vpn_label(&name)),
                        Err(e) => t!("замер: {0}", e),
                    });
                });
                self.v.testing = Some(rx);
                self.msg = t!("замеряю задержку серверов «{}»...", tui_vpn_label(&g.name));
            }
            _ => {}
        }
        None
    }

    fn key_vpn_subs(&mut self, k: KeyCode, del: Option<usize>) -> Args {
        let subs = self.ui.vpn_state().subs;
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
                self.msg = t!("удалить «{}»? нажми x ещё раз", tui_vpn_label(&subs[sel].name));
            }
            _ => {}
        }
        None
    }

    fn key_vpn_opts(&mut self, k: KeyCode) -> Args {
        let opts = match self.vpn_opts() {
            Ok(opts) => opts,
            Err(e) => {
                self.msg = t!("не удалось открыть настройки VPN: {0}", e);
                return None;
            }
        };
        let sel = self.v.opts.selected().unwrap_or(0).min(opts.len() - 1);
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.v.opts.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.opts.select(Some((sel + 1).min(opts.len() - 1))),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let mut c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return None;
                    }
                };
                match opts[sel].key {
                    "run" => return cmd(&["vpn", if self.ui.service_state(vpn::SERVICE) == "active" { "stop" } else { "start" }]),
                    "tun" => return cmd(&["vpn", if c.vpn_tun { "proxy" } else { "tun" }]),
                    "rules" => return cmd(&["vpn", "rules"]),
                    "geo" => return cmd(&["vpn", "geo"]),
                    "core" => return cmd(&["vpn", "core", "update"]),
                    "mode" => {
                        c.vpn_mode = (c.vpn_mode + 1) % 3;
                        if let Err(e) = c.save() {
                            self.msg = t!("не сохранено: {0}", e);
                            return None;
                        }
                        if let Err(e) = vpn::write_config(&c) {
                            self.msg = t!("ошибка: {0}", e);
                            return None;
                        }
                        self.msg = match vpn::running().then(|| vpn::set_mode(c.vpn_mode_name())) {
                            Some(Err(e)) => t!("ошибка: {0}", e),
                            _ => t!("маршрутизация сохранена").into(),
                        };
                        self.v.at = None;
                    }
                    "autostart" => {
                        c.vpn_autostart = !c.vpn_autostart;
                        if let Err(e) = c.save() {
                            self.msg = t!("не сохранено: {0}", e);
                            return None;
                        }
                        self.msg = match self.ui.service_state(vpn::SERVICE).as_str() {
                            "" => t!("сохранено; служба появится после sudo upd install").into(),
                            _ => match vpn::autostart(c.vpn_autostart) {
                                Ok(()) => t!("запуск при загрузке: {}", on_off(c.vpn_autostart)),
                                Err(e) => t!("ошибка: {0}", e),
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
            l1.push(Span::styled(t!("● работает"), Style::new().fg(Color::Green)));
            let mode = match s.mode.as_str() {
                "global" => t!("всё через VPN"),
                "direct" => t!("всё напрямую"),
                _ => t!("по правилам"),
            };
            l1.push(Span::raw(format!(" · {} · {mode} · mihomo {}", if s.tun { "TUN" } else { t!("прокси") }, s.version)));
        } else {
            let st = self.ui.service_state(vpn::SERVICE);
            l1.push(Span::styled(if st == "failed" { t!("● ошибка запуска (journalctl -u upd-vpn)") } else { t!("○ выключен") }, Style::new().fg(if st == "failed" { Color::Red } else { Color::DarkGray })));
        }
        let st = self.ui.vpn_state();
        if let Some(a) = st.subs.iter().find(|x| x.active) {
            l1.push(Span::styled(format!(" · «{}»{}", tui_vpn_label(&a.name), a.info.as_ref().map(sub_info).unwrap_or_default()), dim()));
        }
        let l2 = if s.running {
            Line::from(vec![Span::styled(t!("маршрут: "), dim()), Span::raw(s.chain().iter().map(|n| tui_vpn_label(n)).collect::<Vec<_>>().join(" → ")), Span::styled(t!(" · ↓ {} ↑ {} · соединений {}", fmt_bytes(s.down), fmt_bytes(s.up), s.conns), dim())])
        } else if st.subs.is_empty() {
            Line::styled(t!("нет подписки — вкладка 2, клавиша n"), Style::new().fg(Color::Yellow))
        } else {
            Line::styled(t!("s — включить"), dim())
        };
        let l3 = if let Some(buf) = &self.input {
            Line::from(vec![Span::raw(t!("Новое значение: ")), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan))])
        } else if let Some(w) = self.ui.flclash_warning() {
            Line::styled(format!("⚠ {w}"), Style::new().fg(Color::Red))
        } else {
            Line::raw("")
        };
        f.render_widget(Paragraph::new(vec![Line::from(l1), l2, l3]), info);
        let mut tl = vec![];
        for (i, t) in VPN_TABS.iter().enumerate() {
            let txt = format!(" {} {} ", i + 1, t!(*t));
            tl.push(if i == self.v.tab { Span::styled(txt, Style::new().add_modifier(Modifier::REVERSED)) } else { Span::styled(txt, dim()) });
        }
        f.render_widget(Line::from(tl), tabs);
        let blk = |t: String| Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(t);
        let hl = Style::new().add_modifier(Modifier::REVERSED);
        match self.v.tab {
            0 => {
                if s.groups.is_empty() {
                    f.render_widget(Paragraph::new(Line::styled(t!("серверы видны, когда VPN работает"), dim())).block(blk(t!(" Серверы ").into())), body);
                    return;
                }
                let [gl, nl] = Layout::horizontal([Constraint::Percentage(35), Constraint::Min(20)]).areas(body);
                let glines: Vec<Line> = s
                    .groups
                    .iter()
                    .enumerate()
                    .map(|(i, g)| {
                        let t = format!(" {} → {}", tui_vpn_label(&g.name), tui_vpn_label(&g.now));
                        if i == self.v.group {
                            Line::styled(t, Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD))
                        } else {
                            Line::raw(t)
                        }
                    })
                    .collect();
                let visible = gl.height.saturating_sub(2) as usize;
                let offset = self.v.group.saturating_sub(visible.saturating_sub(1)).min(s.groups.len().saturating_sub(visible));
                f.render_widget(Paragraph::new(glines).scroll((offset.min(u16::MAX as usize) as u16, 0)).block(blk(t!(" Группы ←→ ").into())), gl);
                let g = &s.groups[self.v.group.min(s.groups.len() - 1)];
                let rows: Vec<Row> = g
                    .all
                    .iter()
                    .map(|n| {
                        let sub = s.groups.iter().find(|x| &x.name == n);
                        Row::new(vec![
                            Cell::from(Span::styled(if *n == g.now { "●" } else { "" }, Style::new().fg(Color::Cyan))),
                            Cell::from(tui_vpn_label(n)),
                            Cell::from(match sub {
                                Some(x) => Span::styled(t!("группа → {}", tui_vpn_label(&x.now)), dim()),
                                None => delay_span(s.delay.get(n)),
                            }),
                        ])
                    })
                    .collect();
                let kind = match g.kind.as_str() {
                    "Selector" => t!("выбор вручную"),
                    "URLTest" => t!("самый быстрый"),
                    "Fallback" => t!("первый живой"),
                    _ => t!("балансировка"),
                };
                let t = Table::new(rows, [Constraint::Length(2), Constraint::Min(20), Constraint::Length(24)]).row_highlight_style(hl).block(blk(format!(" {} · {kind} ", tui_vpn_label(&g.name))));
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
                            Cell::from(format!("{}. {}", i + 1, tui_vpn_label(&x.name))),
                            Cell::from(Span::styled(x.host.clone(), dim())),
                            Cell::from(x.nodes.to_string()),
                            Cell::from(fmt_ago(x.updated)),
                            Cell::from(if x.error.is_empty() { Span::raw(info) } else { Span::styled(format!("⚠ {}", x.error), Style::new().fg(Color::Red)) }),
                        ])
                    })
                    .collect();
                let t = Table::new(rows, [Constraint::Length(2), Constraint::Length(22), Constraint::Length(20), Constraint::Length(8), Constraint::Length(12), Constraint::Min(10)])
                    .header(Row::new(vec!["", t!("подписка"), t!("сервер"), t!("узлов"), t!("обновлена"), t!("трафик / срок")]).style(dim()))
                    .row_highlight_style(hl)
                    .block(blk(t!(" Подписки — ● активная ").into()));
                f.render_stateful_widget(t, body, &mut self.v.subs);
            }
            _ => {
                match self.vpn_opts() {
                    Ok(opts) => {
                        let rows: Vec<Row> = opts.into_iter().map(|o| Row::new(vec![Cell::from(o.label), Cell::from(o.value)])).collect();
                        let t = Table::new(rows, [Constraint::Length(30), Constraint::Min(20)]).row_highlight_style(hl).block(blk(t!(" Настройки ").into()));
                        f.render_stateful_widget(t, body, &mut self.v.opts);
                    }
                    Err(e) => f.render_widget(Paragraph::new(t!("не удалось открыть настройки VPN: {0}", e)).block(blk(t!(" Настройки ").into())), body),
                }
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
    if total == 0 && u.flatpak_error.is_empty() {
        upd.push(ok(t!("нет").into()));
    } else {
        upd.push(warn(t!("пакетов {}", u.list.len())));
        if !u.flatpak.is_empty() {
            upd.push(warn(format!(", Flatpak {}", u.flatpak.len())));
        }
        if u.downloaded {
            upd.push(ok(t!(" · скачаны").into()));
        }
    }
    if !u.flatpak_error.is_empty() {
        upd.push(bad(t!(" · ошибка проверки Flatpak: {}", u.flatpak_error)));
    }
    if !u.firmware.is_empty() {
        upd.push(warn(t!(" · прошивок {}", u.firmware.len())));
    }
    if !u.firmware_error.is_empty() {
        upd.push(bad(t!(" · ошибка проверки прошивок: {}", u.firmware_error)));
    }
    if !u.error.is_empty() {
        upd.push(bad(format!(" · {}", u.error)));
    }
    if !u.skipped.is_empty() {
        upd.push(warn(format!(" · {}", u.skipped)));
    }
    upd.push(Span::styled(t!(" · проверка {}", fmt_ago(u.checked)), dim()));
    let mut out = vec![row(t!("Обновления"), upd)];
    if !u.news.is_empty() {
        out.push(row(t!("Новости Arch"), vec![bad(t!("{} непрочитанных — прочитай до обновления (пункт 3)", u.news.len()))]));
    }
    out.push(row(t!("Последнее"), vec![Span::raw(fmt_time(s.last_tx))]));
    let rs = &s.restart;
    let mut rb = vec![if s.reboot || !rs.critical.is_empty() { bad(t!("перезагрузка НУЖНА").into()) } else { ok(t!("перезагрузка не нужна").into()) }];
    if !rs.services.is_empty() {
        rb.push(warn(t!(" · служб к перезапуску {} (пункт 7)", rs.services.len())));
    }
    if !rs.unknown.is_empty() {
        rb.push(bad(t!(" · процессов с нераспознанным cgroup {}", rs.unknown.len())));
    }
    out.push(row(t!("После обновл."), rb));
    let mut net = vec![Span::raw(s.net_label.clone())];
    if s.metered {
        net.push(warn(t!(" · лимитная").into()));
    }
    if s.battery {
        net.push(warn(t!(" · от батареи").into()));
    }
    out.push(row(t!("Сеть"), net));
    if s.managed {
        let first = match s.pinned.first() {
            Some(p) => vec![ok(host_of(p).to_string()), Span::styled(format!(" +{}", s.pinned.len().saturating_sub(1)), dim())],
            None => vec![bad(t!("не закреплены").into())],
        };
        let mut v = first;
        v.push(Span::styled(t!(" · замер {}", fmt_ago(s.mir.checked)), dim()));
        if !s.mir.hints.is_empty() {
            v.push(warn(t!(" · подсказок {} (пункт 4)", s.mir.hints.len())));
        }
        out.push(row(t!("Зеркала"), v));
    } else {
        out.push(row(t!("Зеркала"), vec![Span::styled(s.mirror_note.clone(), dim())]));
    }
    out.push(row(t!("Снапшоты"), vec![Span::raw(s.snapshots.clone())]));
    if !s.vpn.is_empty() {
        let v = if s.vpn.starts_with(t!("работает")) { ok(tui_vpn_label(&s.vpn)) } else if s.vpn.starts_with(t!("ОШИБКА (journalctl -u upd-vpn)")) { bad(tui_vpn_label(&s.vpn)) } else { Span::raw(tui_vpn_label(&s.vpn)) };
        out.push(row("VPN", vec![v, Span::styled(t!(" (пункт 5)"), dim())]));
    }
    let pend = if s.pending.is_empty() { Span::raw("0") } else { warn(s.pending.len().to_string()) };
    let fail = if s.failed == 0 { Span::raw("0") } else { bad(s.failed.to_string()) };
    out.push(row(
        t!("Обслуживание"),
        vec![
            Span::raw(t!("новых настроек ")),
            pend,
            Span::raw(t!(" · сирот {} · кэш {} · свободно {} · упавших служб ", s.orphans, fmt_bytes(s.cache), s.free.map(fmt_bytes).unwrap_or_else(|| t!("неизвестно").into()))),
            fail,
        ],
    ));
    let st = |v: &str| if v == "active" { ok(unit_label(v)) } else { bad(unit_label(v)) };
    out.push(row(t!("Автоматика"), vec![Span::raw(t!("обновления ")), st(&s.auto_timer), Span::raw(t!(" · слежение за сетью ")), st(&s.net_timer)]));
    if s.auto_timer != "active" {
        out.push(Line::styled(t!("              установи: sudo upd install"), dim()));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;

    struct FixtureBackend;

    impl Backend for FixtureBackend {
        fn name(&self) -> String {
            "Fixture Linux".into()
        }
        fn mirrors_managed(&self) -> bool {
            false
        }
        fn mirror_note(&self) -> String {
            "fixture".into()
        }
        fn probe_url(&self, mirror: &str) -> String {
            mirror.into()
        }
        fn valid_mirror(&self, _mirror: &str) -> Result<(), String> {
            Ok(())
        }
        fn default_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn pinned(&self) -> Vec<String> {
            vec![]
        }
        fn list_mirrors(&self) -> Vec<String> {
            vec![]
        }
        fn discover(&self, _n: usize, _log: Log) -> Result<(Vec<String>, Option<Vec<String>>), String> {
            Err("fixture".into())
        }
        fn apply_mirrors(&self, _best: &[String], _fallback: Option<&[String]>) -> Result<bool, String> {
            Ok(false)
        }
        fn remove_mirrors(&self) -> Result<(), String> {
            Ok(())
        }
        fn refresh(&self, _quiet: bool) -> Result<(), String> {
            Ok(())
        }
        fn updates(&self) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
        fn prefetch(&self, _pkgs: &[String], _quiet: bool) -> Result<(), String> {
            Ok(())
        }
        fn upgrade(&self, _aur: bool) -> Result<(), String> {
            Ok(())
        }
        fn clean(&self) -> Result<(), String> {
            Ok(())
        }
        fn orphans(&self) -> Vec<String> {
            vec![]
        }
        fn pending_configs(&self) -> Vec<String> {
            vec![]
        }
        fn merge(&self) -> Result<(), String> {
            Ok(())
        }
        fn cache_dirs(&self) -> Vec<&'static str> {
            vec![]
        }
        fn db_path(&self) -> &'static str {
            ""
        }
        fn history(&self, _n: usize) -> Vec<String> {
            vec![]
        }
    }

    struct FixtureUiData;

    impl UiData for FixtureUiData {
        fn config(&self, mirrors: Vec<String>) -> Result<Config, String> {
            Ok(Config::defaults(mirrors))
        }
        fn mirror_state(&self) -> MirrorState {
            MirrorState::default()
        }
        fn fingerprint(&self) -> NetInfo {
            NetInfo { id: "fixture".into(), label: "fixture network".into(), online: false, vpn: false, dev: String::new() }
        }
        fn vpn_state(&self) -> vpn::VpnState {
            vpn::VpnState::default()
        }
        fn service_state(&self, _unit: &str) -> String {
            String::new()
        }
        fn flclash_warning(&self) -> Option<String> {
            None
        }
        fn geo_files(&self) -> Vec<(String, i64, u64)> {
            vec![]
        }
        fn user_rules(&self) -> Result<Vec<String>, String> {
            Ok(vec![])
        }
    }

    fn dump(t: &Terminal<TestBackend>) -> String {
        let b = t.backend().buffer();
        (0..b.area.height)
            .map(|y| (0..b.area.width).map(|x| b[(x, y)].symbol().to_string()).collect::<String>().trim_end().to_string())
            .collect::<Vec<_>>()
            .join("\n")
    }

    #[test]
    fn test02_german_menu_and_russian_layout_hotkeys() {
        let b = FixtureBackend;
        let ui = FixtureUiData;
        crate::i18n::set(crate::i18n::Lang::De);
        let mut app = App::new(&b, &ui, Some(Status::default()), vpn::Snapshot::default());
        let mut t = Terminal::new(TestBackend::new(118, 30)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Alles aktualisieren"));
        assert!(dump(&t).contains("Sprache / Language"));
        // й — та же клавиша, что q: из меню выходит
        app.key(KeyCode::Char(crate::i18n::latin_key('й')));
        assert!(app.quit);
        crate::i18n::set(crate::i18n::Lang::Ru);
    }

    #[test]
    fn test01_menu_vpn_navigation_uses_fixed_state() {
        let b = FixtureBackend;
        let ui = FixtureUiData;
        let mut app = App::new(&b, &ui, Some(Status::default()), vpn::Snapshot::default());
        let mut t = Terminal::new(TestBackend::new(118, 30)).unwrap();

        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Обновить всё"));
        assert!(dump(&t).contains("Fixture Linux"));

        app.key(KeyCode::Down);
        assert_eq!(app.sel, 1, "стрелка вниз выбирает следующую строку меню");
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Проверить и скачать обновления"));

        app.key(KeyCode::Char('5'));
        assert!(matches!(&app.scr, Screen::Vpn), "клавиша 5 открывает VPN");
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Серверы"));

        app.key(KeyCode::Down);
        assert_eq!(app.msg, "VPN не запущен — s, чтобы включить");
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("VPN не запущен — s, чтобы включить"));

        app.key(KeyCode::Esc);
        assert!(matches!(&app.scr, Screen::Menu), "Esc возвращает в меню");
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Обновить всё"));
    }
}
