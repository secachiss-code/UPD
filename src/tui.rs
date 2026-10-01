//! Интерфейс в терминале (ratatui).

mod process;

use upd::backend::{self, Backend};
use upd::common::*;
use upd::mirrors::{apply_mirrors, candidates, load_mirror_state};
use upd::{extras, gather_status, sub_info, vpn, Status};
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

/// Пункты меню; последний (Выход) — клавиша 0. Пункты 1–9 одинаковы на всех системах:
/// необязательный AUR идёт после них, поэтому номера не сдвигаются.
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

// ---------- фоновые задачи ----------

enum Poll<T> {
    Idle,
    Pending,
    Done(T),
    /// поток завершился, не отправив результата (паника, ранний выход)
    Lost,
}

/// Фоновая задача одного типа: не больше одного выполняющегося потока и одного отложенного повтора.
/// Блокирующий сбор (systemctl, HTTP, pacman) изнутри не прерывается — повтор ждёт его конца.
struct Job<T> {
    rx: Option<mpsc::Receiver<T>>,
    again: bool,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Job { rx: None, again: false }
    }
}

impl<T: Send + 'static> Job<T> {
    fn busy(&self) -> bool {
        self.rx.is_some()
    }

    /// Запустить; если задача уже идёт — запомнить один повтор. Ok(true) — поток запущен сейчас.
    fn request(&mut self, f: impl FnOnce() -> T + Send + 'static) -> Result<bool, String> {
        if self.rx.is_some() {
            self.again = true;
            return Ok(false);
        }
        let (tx, rx) = mpsc::channel();
        spawn_thread("upd-tui-job", move || {
            let _ = tx.send(f());
        })?;
        self.rx = Some(rx);
        Ok(true)
    }

    fn poll(&mut self) -> Poll<T> {
        let Some(rx) = &self.rx else { return Poll::Idle };
        match rx.try_recv() {
            Ok(v) => {
                self.rx = None;
                Poll::Done(v)
            }
            Err(mpsc::TryRecvError::Empty) => Poll::Pending,
            Err(mpsc::TryRecvError::Disconnected) => {
                self.rx = None;
                Poll::Lost
            }
        }
    }

    fn take_again(&mut self) -> bool {
        std::mem::take(&mut self.again)
    }
}

// ---------- снимки экранов ----------

/// Пункт вкладки «Настройки»: ключ, подпись, значение.
#[derive(Clone)]
struct Opt {
    key: &'static str,
    label: &'static str,
    value: String,
}

/// Всё, что показывает экран VPN; собирается в фоне, кадр только читает готовые строки.
#[derive(Clone, Default)]
struct VpnPage {
    at: i64,
    service: String,
    warning: Option<String>,
    state: vpn::VpnState,
    opts: Vec<Opt>,
    opts_err: String,
    snap: vpn::Snapshot,
}

/// Всё, что показывает экран зеркал.
#[derive(Clone, Default)]
struct MirrorView {
    at: i64,
    keep: usize,
    net_label: String,
    st: MirrorState,
    pinned: Vec<String>,
    rows: Vec<Probe>,
}

fn vpn_opts(c: &Config, st: &vpn::VpnState, unit: &str) -> Result<Vec<Opt>, String> {
    let rules_count = vpn::user_rules()?.len();
    let geo = vpn::geo_files().iter().map(|g| g.1).filter(|t| *t > 0).min();
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
            match unit {
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

fn collect_vpn_page(mirrors: Vec<String>) -> VpnPage {
    let snap = vpn::snapshot();
    let service = unit_state(vpn::SERVICE);
    let state = vpn::load_state();
    let cfg = Config::load(mirrors);
    let warning = cfg.as_ref().ok().and_then(vpn::conflict);
    let (opts, opts_err) = match cfg.and_then(|c| vpn_opts(&c, &state, &service)) {
        Ok(opts) => (opts, String::new()),
        Err(e) => (vec![], e),
    };
    VpnPage { at: now(), service, warning, state, opts, opts_err, snap }
}

/// Строки таблицы зеркал: кандидаты с последним замером, лучшие сверху.
fn mirror_rows(b: &dyn Backend, c: &Config, st: &MirrorState) -> Vec<Probe> {
    let measured: std::collections::HashMap<&str, &Probe> = st.results.iter().map(|p| (p.url.as_str(), p)).collect();
    let mut r: Vec<Probe> = candidates(b, c, None)
        .into_iter()
        .map(|cd| {
            let mut p = measured.get(cd.url.as_str()).map(|p| (*p).clone()).unwrap_or(Probe { url: cd.url.clone(), err: "not measured".into(), ..Default::default() });
            p.src = cd.src.into();
            p
        })
        .collect();
    let shown = |p: &Probe| if p.score > 0.0 { p.score } else { p.speed };
    r.sort_by(|a, b| b.ok.cmp(&a.ok).then(shown(b).total_cmp(&shown(a))));
    r
}

fn collect_mirror_view() -> Result<MirrorView, String> {
    let b = backend::detect()?;
    let b = b.as_ref();
    let c = Config::load(b.default_mirrors())?;
    let st = load_mirror_state();
    let rows = mirror_rows(b, &c, &st);
    Ok(MirrorView { at: now(), keep: c.keep, net_label: fingerprint().label, pinned: b.pinned(), rows, st })
}

/// Источники данных экранов. Сборщики — обычные функции: их выполняет фоновый поток.
trait UiData {
    fn config(&self, mirrors: Vec<String>) -> Result<Config, String> {
        Config::load(mirrors)
    }
    fn vpn_collector(&self) -> fn(Vec<String>) -> VpnPage {
        collect_vpn_page
    }
    fn mirror_collector(&self) -> fn() -> Result<MirrorView, String> {
        collect_mirror_view
    }
}

struct HostUiData;
impl UiData for HostUiData {}

#[derive(Default)]
struct AurUi {
    /// запрос, к которому относятся list и status
    query: String,
    list: Vec<extras::AurPkg>,
    table: TableState,
    job: Job<(String, Result<Vec<extras::AurPkg>, String>)>,
    status: String,
    /// последний поиск завершился ошибкой
    failed: bool,
}

#[derive(Clone, Copy, PartialEq)]
enum PagerKind {
    List,
    Snapshots,
    Configs,
    History,
}

struct App<'a> {
    b: &'a dyn Backend,
    ui: &'a dyn UiData,
    st: Option<Status>,
    st_job: Job<Result<Status, String>>,
    /// когда собран показанный снимок состояния
    st_at: i64,
    st_err: String,
    scr: Screen,
    sel: usize,
    msg: String,
    p_kind: PagerKind,
    p_title: String,
    p_src: String,
    p_lines: Vec<String>,
    p_off: usize,
    p_job: Job<(PagerKind, Result<Vec<String>, String>)>,
    /// в списке новых настроек есть что слить через pacdiff
    p_merge: bool,
    status_off: usize,
    p_aur: Job<Result<Vec<String>, String>>,
    p_aur_line: Option<usize>,
    m_state: TableState,
    mview: Option<MirrorView>,
    m_job: Job<Result<MirrorView, String>>,
    m_err: String,
    input: Option<String>,
    /// что вводится: "" — адрес зеркала, "aur" — поиск, иначе ключ числовой настройки VPN
    input_key: &'static str,
    /// почему введённое значение не принято; поле остаётся открытым для исправления
    input_err: String,
    /// фоновое действие (применение настроек VPN, зеркал, выбор сервера)
    act: Job<String>,
    act_label: String,
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
    page: Option<VpnPage>,
    job: Job<VpnPage>,
    /// когда запрошен последний снимок; None — запросить сразу
    at: Option<Instant>,
    group: usize,
    /// группа уже выбрана пользователем; до этого показываем главную (первый Selector)
    group_set: bool,
    nodes: TableState,
    subs: TableState,
    opts: TableState,
    testing: Job<String>,
    /// подтверждение удаления: неизменный id и показанное имя
    confirm: Option<(String, String)>,
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

fn lost_msg() -> String {
    t!("фоновая задача прервалась без ответа — r, чтобы повторить").into()
}

pub fn run(b: &dyn Backend) -> i32 {
    let ui = HostUiData;
    let mut app = App::new(b, &ui, None, None);
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
    fn new(b: &'a dyn Backend, ui: &'a dyn UiData, st: Option<Status>, page: Option<VpnPage>) -> Self {
        let v = VpnUi { page, ..Default::default() };
        App {
            b,
            ui,
            st_at: if st.is_some() { now() } else { 0 },
            st,
            st_job: Job::default(),
            st_err: String::new(),
            scr: Screen::Menu,
            sel: 0,
            msg: String::new(),
            p_kind: PagerKind::List,
            p_title: String::new(),
            p_src: String::new(),
            p_lines: vec![],
            p_off: 0,
            p_job: Job::default(),
            p_merge: false,
            status_off: 0,
            p_aur: Job::default(),
            p_aur_line: None,
            m_state: TableState::default().with_selected(Some(0)),
            mview: None,
            m_job: Job::default(),
            m_err: String::new(),
            input: None,
            input_key: "",
            input_err: String::new(),
            act: Job::default(),
            act_label: String::new(),
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
    /// Пока идёт сбор, повторный запрос не создаёт второй поток, а ставит один повтор.
    fn reload(&mut self) {
        if let Err(e) = self.st_job.request(|| backend::detect().map(|b| gather_status(b.as_ref()))) {
            self.st_err = e;
        }
    }

    fn poll_status(&mut self) {
        match self.st_job.poll() {
            Poll::Done(Ok(s)) => {
                self.st = Some(s);
                self.st_at = now();
                self.st_err.clear();
            }
            Poll::Done(Err(e)) => self.st_err = e,
            Poll::Lost => self.st_err = lost_msg(),
            Poll::Idle | Poll::Pending => return,
        }
        if self.st_job.take_again() {
            self.reload();
        }
    }

    fn cfg(&self) -> Result<Config, String> {
        self.ui.config(self.b.default_mirrors())
    }

    /// Долгое действие — в фоне; второе, пока идёт первое, не запускается.
    fn start_action(&mut self, label: String, f: impl FnOnce() -> String + Send + 'static) {
        if self.act.busy() {
            self.msg = t!("подожди: {0}", self.act_label);
            return;
        }
        match self.act.request(f) {
            Ok(_) => {
                self.msg = format!("⏳ {label}...");
                self.act_label = label;
            }
            Err(e) => self.msg = e,
        }
    }

    fn poll_action(&mut self) {
        match self.act.poll() {
            Poll::Done(m) => self.msg = m,
            Poll::Lost => self.msg = t!("{0}: задача прервалась без ответа", self.act_label),
            Poll::Idle | Poll::Pending => return,
        }
        self.act.take_again();
        self.v.at = None;
        if self.scr == Screen::Mirrors {
            self.refresh_mirrors();
        }
        self.reload();
    }

    fn main_loop(&mut self, term: &mut DefaultTerminal) -> std::io::Result<()> {
        while !self.quit {
            self.poll_status();
            self.poll_action();
            self.vpn_poll();
            self.mirrors_poll();
            self.pager_poll();
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
                            let _ = p.resize(h.saturating_sub(7), w);
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
        match ProcessSession::spawn(args, size.height.saturating_sub(7), size.width) {
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
        self.after_command();
        Ok(())
    }

    /// После внешней команды данные экранов могли измениться.
    fn after_command(&mut self) {
        self.reload();
        self.v.at = None;
        let back = if self.scr == Screen::Process { self.process_return } else { self.scr };
        if back == Screen::Mirrors {
            self.refresh_mirrors();
        }
        if back == Screen::Pager && self.p_kind == PagerKind::Configs {
            self.open_pager(PagerKind::Configs);
        }
        // после установки из AUR обновить отметки «установлен»
        if back == Screen::Aur && !self.aur.query.is_empty() {
            self.aur_start(self.aur.query.clone());
        }
    }

    fn report_process_completion(&mut self) {
        if self.process_reported {
            return;
        }
        let Some(code) = self.process.as_ref().and_then(ProcessSession::finished) else { return };
        self.process_reported = true;
        self.msg = if code == 0 { t!("готово").into() } else { format!("{} ({code})", t!("завершилось с ошибкой")) };
        self.after_command();
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
            let _ = p.resize(size.height.saturating_sub(7), size.width);
        }
        result
    }

    // ---------- Pager ----------

    /// Текстовая страница. «Что доступно» — сохранённый файл, читается сразу; остальное собирается в фоне.
    fn open_pager(&mut self, kind: PagerKind) {
        self.scr = Screen::Pager;
        self.p_kind = kind;
        self.p_off = 0;
        self.p_aur_line = None;
        self.p_merge = false;
        self.p_title = match kind {
            PagerKind::List => t!("Что доступно"),
            PagerKind::Snapshots => t!("Снапшоты (новые сверху)"),
            PagerKind::Configs => t!("Новые файлы настроек"),
            PagerKind::History => t!("Журнал пакетов (новые сверху)"),
        }
        .into();
        if kind == PagerKind::List {
            let u: UpdState = load_json("updates.json");
            self.p_src = t!("сохранённая проверка от {0}", fmt_time(u.checked));
            self.p_lines = updates_lines(&u);
            self.start_aur_updates();
            return;
        }
        self.p_src = t!("загружаю...").into();
        self.p_lines = vec![t!("загружаю...").into()];
        let started = self.p_job.request(pager_task(kind));
        if let Err(e) = started {
            self.p_lines = vec![format!("⚠ {e}")];
        }
    }

    fn pager_poll(&mut self) {
        let (kind, result) = match self.p_job.poll() {
            Poll::Done(r) => r,
            Poll::Lost => (self.p_kind, Err(lost_msg())),
            Poll::Idle | Poll::Pending => return,
        };
        let again = self.p_job.take_again();
        if self.scr != Screen::Pager || kind != self.p_kind {
            // ответ для другой страницы устарел; открытая страница ждёт своего
            if self.scr == Screen::Pager && self.p_kind != PagerKind::List {
                let _ = self.p_job.request(pager_task(self.p_kind));
            }
            return;
        }
        if again {
            let _ = self.p_job.request(pager_task(kind));
        }
        self.p_src = t!("получено {0}", fmt_clock(now()));
        match result {
            Ok(lines) => {
                self.p_merge = kind == PagerKind::Configs && !lines.is_empty() && have("pacdiff");
                self.p_lines = if lines.is_empty() {
                    vec![if kind == PagerKind::Configs { t!("нет — всё слито") } else { t!("пусто") }.into()]
                } else {
                    lines
                };
            }
            Err(e) => self.p_lines = vec![format!("⚠ {e}")],
        }
        self.p_off = self.p_off.min(self.p_lines.len().saturating_sub(1));
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
        // идёт прежняя проверка — её ответ попадёт в эту строку, второй поток не нужен
        if let Err(e) = self.p_aur.request(move || extras::aur_updates(&user)) {
            let last = self.p_lines.len() - 1;
            self.p_lines[last] = format!("⚠ {e}");
            self.p_aur_line = None;
        }
    }

    fn poll_aur_updates(&mut self) {
        let result = match self.p_aur.poll() {
            Poll::Done(result) => result,
            Poll::Lost => Err(t!("завершилось с ошибкой").into()),
            Poll::Idle | Poll::Pending => return,
        };
        self.p_aur.take_again();
        if self.scr != Screen::Pager || self.p_kind != PagerKind::List {
            self.p_aur_line = None;
            return;
        }
        if let Some(at) = self.p_aur_line.take() {
            let lines = match result {
                Ok(list) if list.is_empty() => vec![t!("нет").into()],
                Ok(list) => list,
                Err(e) => vec![format!("⚠ {e}")],
            };
            if at < self.p_lines.len() {
                self.p_lines.splice(at..=at, lines);
            }
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
                    KeyCode::Char('r') => self.reload(),
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
                    KeyCode::Char('r') => {
                        let off = self.p_off;
                        self.open_pager(self.p_kind);
                        self.p_off = off.min(self.p_lines.len().saturating_sub(1));
                    }
                    KeyCode::Char('m') if self.p_merge => return cmd(&["merge"]),
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
                self.reload();
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
            Item::List => self.open_pager(PagerKind::List),
            Item::Mirrors => {
                self.scr = Screen::Mirrors;
                self.m_state.select(Some(0));
                self.refresh_mirrors();
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
            Item::Snapshots => self.open_pager(PagerKind::Snapshots),
            Item::Restart => return cmd(&["restart"]),
            Item::Clean => return cmd(&["clean"]),
            Item::Configs => self.open_pager(PagerKind::Configs),
            Item::History => self.open_pager(PagerKind::History),
            Item::Aur => {
                self.scr = Screen::Aur;
                if self.aur.list.is_empty() && !self.aur.job.busy() {
                    self.open_input("aur", self.aur.query.clone());
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
                        // статус и снимок VPN собраны на прежнем языке
                        self.reload();
                        self.v.at = None;
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

    /// Новый поиск: прежние строки убираются сразу — установить можно только результат этого запроса.
    /// Пока идёт прежний поиск, новый запрос ждёт его конца (ответ старого отбрасывается).
    fn aur_start(&mut self, q: String) {
        self.aur.query = q;
        self.aur.list.clear();
        self.aur.table.select(None);
        self.aur.status = t!("ищу в AUR...").into();
        self.aur.failed = false;
        let q2 = self.aur.query.clone();
        if let Err(e) = self.aur.job.request(move || {
            let r = extras::aur_search(&q2);
            (q2, r)
        }) {
            self.aur.status = e;
        }
    }

    fn aur_poll(&mut self) {
        let (q, r) = match self.aur.job.poll() {
            Poll::Done(v) => v,
            Poll::Lost => (self.aur.query.clone(), Err(t!("поиск прервался без ответа — / чтобы искать снова").into())),
            Poll::Idle | Poll::Pending => return,
        };
        let again = self.aur.job.take_again();
        if q != self.aur.query || again {
            // ответ на прежний запрос: показываем только текущий
            let current = self.aur.query.clone();
            if !current.is_empty() {
                self.aur_start(current);
            }
            return;
        }
        match r {
            Ok(list) => {
                self.aur.status = if list.is_empty() { t!("по запросу «{}» ничего не найдено", self.aur.query) } else { t!("найдено: {}", list.len()) };
                self.aur.table.select(if list.is_empty() { None } else { Some(0) });
                self.aur.list = list;
            }
            Err(e) => {
                self.aur.list.clear();
                self.aur.table.select(None);
                self.aur.status = t!("ошибка: {0}", e);
                self.aur.failed = true;
            }
        }
    }

    fn key_aur(&mut self, k: KeyCode) -> Args {
        let n = self.aur.list.len();
        let sel = self.aur.table.selected().unwrap_or(0);
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Char('/') | KeyCode::Char('s') => self.open_input("aur", String::new()),
            KeyCode::Up | KeyCode::Char('k') => self.aur.table.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.aur.table.select(Some((sel + 1).min(n.saturating_sub(1)))),
            KeyCode::PageUp => self.aur.table.select(Some(sel.saturating_sub(15))),
            KeyCode::PageDown => self.aur.table.select(Some((sel + 15).min(n.saturating_sub(1)))),
            KeyCode::Enter | KeyCode::Char('i') => {
                if self.aur.job.busy() {
                    self.msg = t!("поиск ещё идёт").into();
                } else if let Some(p) = self.aur.list.get(sel) {
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
        let head = if self.input.is_some() && self.input_key == "aur" {
            self.input_line(t!("Поиск в AUR: "))
        } else {
            let state = if self.aur.status.is_empty() { t!("нажми / или s, чтобы искать").to_string() } else { format!("«{}» · {}", self.aur.query, self.aur.status) };
            let color = if self.aur.failed { Style::new().fg(Color::Red) } else { dim() };
            Line::styled(state, color)
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

    // ---------- зеркала ----------

    fn refresh_mirrors(&mut self) {
        if !self.b.mirrors_managed() {
            return;
        }
        let collect = self.ui.mirror_collector();
        if let Err(e) = self.m_job.request(collect) {
            self.m_err = e;
        }
    }

    fn mirrors_poll(&mut self) {
        match self.m_job.poll() {
            Poll::Done(Ok(v)) => {
                self.mview = Some(v);
                self.m_err.clear();
            }
            Poll::Done(Err(e)) => self.m_err = e,
            Poll::Lost => self.m_err = lost_msg(),
            Poll::Idle | Poll::Pending => return,
        }
        if self.m_job.take_again() {
            self.refresh_mirrors();
        }
    }

    fn key_mirrors(&mut self, k: KeyCode) -> Args {
        if matches!(k, KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace) {
            self.scr = Screen::Menu;
            return None;
        }
        let managed = self.b.mirrors_managed();
        let sel = self.m_state.selected().unwrap_or(0);
        let n_rows = self.mview.as_ref().map(|v| v.rows.len()).unwrap_or(0);
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.m_state.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.m_state.select(Some((sel + 1).min(n_rows.saturating_sub(1)))),
            KeyCode::Char('r') if managed => self.refresh_mirrors(),
            KeyCode::Char('c') if managed => return cmd(&["mirrors", "check"]),
            KeyCode::Char('s') if managed => return cmd(&["mirrors", "rescan"]),
            KeyCode::Char('a') if managed => {
                let mirrors = self.b.default_mirrors();
                self.start_action(t!("применяю зеркала").into(), move || {
                    let b = match backend::detect() {
                        Ok(b) => b,
                        Err(e) => return e,
                    };
                    let c = match Config::load(mirrors) {
                        Ok(c) => c,
                        Err(e) => return t!("конфиг не прочитан: {0}", e),
                    };
                    let log = std::cell::RefCell::new(vec![]);
                    if let Err(e) = apply_mirrors(b.as_ref(), &c, &load_mirror_state(), None, &|s| log.borrow_mut().push(s.to_string())) {
                        log.borrow_mut().push(e);
                    }
                    log.into_inner().join("; ")
                });
            }
            KeyCode::Char('n') if managed => self.open_input("", String::new()),
            KeyCode::Char('x') | KeyCode::Delete if managed => {
                let Some(url) = self.mview.as_ref().and_then(|v| v.rows.get(sel)).map(|p| p.url.clone()) else { return None };
                let mut c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return None;
                    }
                };
                if contains(&c.mirrors, &url) {
                    c.mirrors.retain(|m| m != &url);
                    self.msg = match c.save() {
                        Ok(()) => t!("удалено из конфига").into(),
                        Err(e) => t!("не сохранено: {0}", e),
                    };
                    self.refresh_mirrors();
                } else {
                    self.msg = t!("это зеркало не из конфига — его подбирает автоматика").into();
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') | KeyCode::Char('-') if managed => {
                let mut c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return None;
                    }
                };
                c.keep = if k == KeyCode::Char('-') { c.keep.saturating_sub(1).max(1) } else { (c.keep + 1).min(10) };
                self.msg = match c.save() {
                    Ok(()) => t!("закреплять {} — нажми a, чтобы применить", c.keep),
                    Err(e) => t!("не сохранено: {0}", e),
                };
                self.refresh_mirrors();
            }
            _ => {}
        }
        None
    }

    // ---------- строка ввода ----------

    fn open_input(&mut self, key: &'static str, value: String) {
        self.input_key = key;
        self.input = Some(value);
        self.input_err.clear();
    }

    /// Что можно ввести — видно до отправки.
    fn input_hint(&self) -> String {
        match self.input_key {
            "aur" => t!("не меньше 2 символов").into(),
            "vpn_port" => t!("1024–65535, не {0} при своём DNS", vpn::DNS_PORT),
            "vpn_sub_update_h" => t!("часы, 1–{0}", MAX_HOURS),
            _ => t!("полный адрес http(s)://… как в списке зеркал").into(),
        }
    }

    /// Поле ввода с подсказкой формата и ошибкой проверки рядом.
    fn input_line(&self, label: &str) -> Line<'static> {
        let buf = self.input.clone().unwrap_or_default();
        let mut spans = vec![Span::raw(label.to_string()), Span::styled(format!("{buf}▏"), Style::new().fg(Color::Cyan)), Span::styled(format!("  {}", self.input_hint()), dim())];
        if !self.input_err.is_empty() {
            spans.push(Span::styled(format!("  ⚠ {}", self.input_err), Style::new().fg(Color::Red)));
        }
        Line::from(spans)
    }

    fn key_input(&mut self, k: KeyCode) {
        let Some(buf) = self.input.as_mut() else { return };
        match k {
            KeyCode::Esc => {
                self.input = None;
                self.input_key = "";
                self.input_err.clear();
            }
            KeyCode::Backspace => {
                buf.pop();
                self.input_err.clear();
            }
            KeyCode::Char(ch) => {
                buf.push(ch);
                self.input_err.clear();
            }
            KeyCode::Enter => {
                let u = buf.trim().to_string();
                if u.is_empty() {
                    self.input = None;
                    self.input_key = "";
                    return;
                }
                match self.submit_input(&u) {
                    Ok(()) => {
                        self.input = None;
                        self.input_key = "";
                        self.input_err.clear();
                    }
                    // значение не принято: поле остаётся с тем же текстом, ошибка — рядом
                    Err(e) => self.input_err = e,
                }
            }
            _ => {}
        }
    }

    fn submit_input(&mut self, u: &str) -> Result<(), String> {
        match self.input_key {
            "aur" => {
                if u.chars().count() < 2 {
                    return Err(t!("для поиска нужно хотя бы 2 символа").into());
                }
                self.aur_start(u.to_string());
                Ok(())
            }
            "" => {
                self.b.valid_mirror(u).map_err(|e| t!("неверный URL: {0}", e))?;
                let mut c = self.cfg().map_err(|e| t!("конфиг не прочитан: {0}", e))?;
                if !contains(&c.mirrors, u) {
                    c.mirrors.push(u.to_string());
                    c.save().map_err(|e| t!("не сохранено: {0}", e))?;
                }
                self.msg = t!("добавлено — нажми c, чтобы замерить").into();
                self.refresh_mirrors();
                Ok(())
            }
            key => self.vpn_set_number(key, u),
        }
    }
}

/// Строки «Что доступно» из сохранённой проверки.
fn updates_lines(u: &UpdState) -> Vec<String> {
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
    l
}

/// Фоновый сбор текстовой страницы: внешние команды (snapper, pacman.log, поиск по /etc) не держат кадр.
fn pager_task(kind: PagerKind) -> impl FnOnce() -> (PagerKind, Result<Vec<String>, String>) + Send + 'static {
    move || {
        let r = match kind {
            PagerKind::Snapshots => {
                let mut l = extras::snap_list(30);
                l.push(String::new());
                l.extend(extras::rollback_hint());
                Ok(l)
            }
            PagerKind::Configs => backend::detect().map(|b| b.pending_configs()),
            PagerKind::History => backend::detect().map(|b| b.history(500)),
            PagerKind::List => Ok(vec![]),
        };
        (kind, r)
    }
}

impl App<'_> {
    // ---------- отрисовка ----------

    fn draw(&mut self, f: &mut Frame) {
        let [head, body, foot] = Layout::vertical([Constraint::Length(1), Constraint::Min(3), Constraint::Length(3)]).areas(f.area());
        f.render_widget(
            Line::from(vec![" upd ".bold().black().on_cyan(), Span::raw(" "), Span::styled(t!("обновление системы · {}", self.b.name()), dim())]),
            head,
        );
        let keys: Vec<(&str, &str)> = match (&self.scr, self.input.is_some()) {
            (_, true) => vec![("enter", t!("сохранить")), ("esc", t!("отмена"))],
            (Screen::Vpn, _) if self.v.confirm.is_some() => vec![("y/enter", t!("удалить")), ("n/esc", t!("отмена"))],
            (Screen::Vpn, _) => {
                let mut k = vec![("q", t!("назад")), ("tab 1-3", t!("вкладка")), ("s", t!("вкл/выкл"))];
                match self.v.tab {
                    0 => k.extend([("←→", t!("группа")), ("enter", t!("выбрать сервер")), ("t", t!("замерить задержку"))]),
                    1 => k.extend([("enter", t!("сделать активной")), ("n", t!("добавить")), ("u", t!("обновить")), ("x", t!("удалить"))]),
                    _ => k.extend([("enter", t!("изменить"))]),
                }
                k
            }
            (Screen::Menu, _) => vec![("↑↓", t!("выбор")), ("enter", t!("выполнить")), ("1-9", t!("пункт")), ("0", t!("выход")), ("i", t!(" Состояние ").trim()), ("r", t!("обновить"))],
            (Screen::Process, _) if self.process.as_ref().and_then(ProcessSession::finished).is_some() => vec![("enter/q", t!("назад")), ("PgUp/PgDn", t!("листать"))],
            (Screen::Process, _) => vec![("F4", t!("полный терминал")), ("Alt+PgUp/PgDn", t!("листать")), ("Ctrl+C", t!("отмена"))],
            (Screen::Status, _) => vec![("↑↓ PgUp PgDn", t!("листать")), ("r", t!("обновить")), ("q", t!("назад"))],
            (Screen::Pager, _) => {
                let mut k = vec![("↑↓ PgUp PgDn", t!("листать"))];
                if self.p_merge {
                    k.push(("m", t!("слить (pacdiff)")));
                }
                k.extend([("r", t!("обновить")), ("q", t!("назад"))]);
                k
            }
            (Screen::Mirrors, _) if self.b.mirrors_managed() => vec![("q", t!("назад")), ("c", t!("замерить")), ("s", t!("искать заново")), ("a", t!("применить")), ("n", t!("добавить")), ("x", t!("удалить")), ("+/-", t!("сколько закреплять")), ("r", t!("обновить"))],
            (Screen::Mirrors, _) => vec![("q", t!("назад"))],
            (Screen::Lang, _) => vec![("↑↓", t!("выбор")), ("enter", t!("выбрать")), ("q", t!("назад"))],
            (Screen::Aur, _) if self.aur.list.is_empty() => vec![("q", t!("назад")), ("/ s", t!("искать"))],
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
        let msg = if self.act.busy() && self.msg.is_empty() { format!("⏳ {}...", self.act_label) } else { self.msg.clone() };
        help.push(Line::from(Span::styled(msg, Style::new().fg(Color::Yellow))));
        f.render_widget(Paragraph::new(help), foot);
        match self.scr {
            Screen::Menu => self.draw_menu(f, body),
            Screen::Process => self.draw_process(f, body),
            Screen::Status => {
                let lines = match &self.st {
                    Some(s) => {
                        let mut l = status_lines(s);
                        if !self.st_err.is_empty() {
                            l.insert(0, Line::styled(t!("⚠ новый снимок не получен: {0}", self.st_err), Style::new().fg(Color::Red)));
                        }
                        l
                    }
                    None if !self.st_err.is_empty() => vec![Line::styled(t!("⚠ состояние не собрано: {0}", self.st_err), Style::new().fg(Color::Red))],
                    None => vec![Line::styled(t!("загружаю состояние..."), dim())],
                };
                let title = format!("{} · {} ", t!(" Состояние ").trim_end(), self.status_age());
                f.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }).scroll((self.status_off.min(u16::MAX as usize) as u16, 0)).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(title)), body);
            }
            Screen::Pager => {
                let h = body.height.saturating_sub(2) as usize;
                let end = (self.p_off + h).min(self.p_lines.len());
                let lines: Vec<Line> = self.p_lines[self.p_off.min(end)..end].iter().map(|l| Line::raw(l.clone())).collect();
                let title = t!(" {} · {}–{} из {} ", format!("{} · {}", self.p_title, self.p_src), self.p_off + 1, end, self.p_lines.len());
                f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).title(title)), body);
            }
            Screen::Mirrors => self.draw_mirrors(f, body),
            Screen::Vpn => self.draw_vpn(f, body),
            Screen::Aur => self.draw_aur(f, body),
            Screen::Lang => self.draw_lang(f, body),
        }
    }

    /// Когда собран снимок состояния, или что он собирается.
    fn status_age(&self) -> String {
        if self.st_job.busy() {
            t!("обновляется...").into()
        } else if self.st_at > 0 {
            t!("снимок {0}", fmt_clock(self.st_at))
        } else {
            String::new()
        }
    }

    fn draw_process(&self, f: &mut Frame, body: ratatui::layout::Rect) {
        let Some(p) = &self.process else { return };
        let [info, log] = Layout::vertical([Constraint::Length(2), Constraint::Min(3)]).areas(body);
        let code = p.finished();
        let state = match code {
            Some(0) => Span::styled(t!("готово"), Style::new().fg(Color::Green)),
            Some(130) => Span::styled(t!("отменено"), Style::new().fg(Color::Yellow)),
            Some(_) => Span::styled(t!("завершилось с ошибкой"), Style::new().fg(Color::Red)),
            None => Span::styled(t!("выполняется..."), Style::new().fg(Color::Yellow)),
        };
        let lines = vec![Line::from(vec![state, Span::styled(format!(" · {}", t!("{} с", p.elapsed_secs())), dim())]), Line::styled(stage_summary(p.stages(), code), dim())];
        f.render_widget(Paragraph::new(lines), info);
        let lines: Vec<Line> = p.lines_for(log.height.saturating_sub(2) as usize).into_iter().map(|line| Line::raw(tui_vpn_label(&line))).collect();
        f.render_widget(Paragraph::new(lines).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(format!(" {} ", p.title))), log);
    }

    fn draw_menu(&self, f: &mut Frame, area: ratatui::layout::Rect) {
        let lines = match &self.st {
            None if !self.st_err.is_empty() => vec![Line::styled(t!("⚠ состояние не собрано: {0}", self.st_err), Style::new().fg(Color::Red))],
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
            Paragraph::new(lines).wrap(Wrap { trim: false }).block(Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(format!("{} · i · {} ", t!(" Состояние ").trim_end(), self.status_age()))),
            top,
        );
        let n_items = items.len();
        let mut selected_row = 0;
        let mut rows = Vec::with_capacity(menu_rows);
        for (i, item) in items.iter().enumerate() {
            if let Some(group) = menu_group(*item) {
                rows.push(Line::styled(format!(" {group}"), Style::new().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
            }
            // номер показан только у пунктов, которые им открываются; остальные — стрелками
            let n = if i + 1 == n_items { "0".to_string() } else if i < 9 { (i + 1).to_string() } else { "·".to_string() };
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
        let Some(v) = &self.mview else {
            let text = if self.m_err.is_empty() { t!("загружаю...").to_string() } else { format!("⚠ {}", self.m_err) };
            f.render_widget(Paragraph::new(text).block(Block::bordered().title(t!(" Зеркала "))), area);
            return;
        };
        let hints = v.st.hints.len() as u16;
        let [info, table, hint_area] = Layout::vertical([Constraint::Length(2), Constraint::Min(3), Constraint::Length(if hints > 0 { hints * 2 + 1 } else { 0 })]).areas(area);
        let mut info_lines = vec![Line::from(vec![
            Span::styled(t!("сеть: "), dim()),
            Span::raw(v.net_label.clone()),
            Span::styled(t!(" · замер: {} · закреплять лучших: {}", fmt_ago(v.st.checked), v.keep), dim()),
        ])];
        if self.input.is_some() {
            info_lines.push(self.input_line(t!("Новое зеркало: ")));
        } else if !self.m_err.is_empty() {
            info_lines.push(Line::styled(t!("⚠ новый снимок не получен: {0}", self.m_err), Style::new().fg(Color::Red)));
        } else if v.st.pending_apply {
            let error = if v.st.apply_error.is_empty() { t!("повтор будет при следующей проверке сети") } else { v.st.apply_error.as_str() };
            info_lines.push(Line::styled(t!("⚠ зеркала ожидают применения: {0}", error), Style::new().fg(Color::Red)));
        } else {
            info_lines.push(Line::styled(t!("● закреплено · скорость сглажена по истории замеров в этой сети · отстающие зеркала не берутся"), dim()));
        }
        f.render_widget(Paragraph::new(info_lines), info);
        // на узком терминале остаются отметка, скорость и адрес; источник и отставание скрываются
        let wide = table.width >= 70;
        let rows: Vec<Row> = v
            .rows
            .iter()
            .map(|p| {
                let color = if p.ok {
                    Color::Green
                } else if p.err == "not measured" {
                    Color::DarkGray
                } else {
                    Color::Red
                };
                let mark = Cell::from(Span::styled(if contains(&v.pinned, &p.url) { "●" } else { "" }, Style::new().fg(Color::Cyan)));
                let speed = Cell::from(Span::styled(fmt_speed(p), Style::new().fg(color)));
                if wide {
                    Row::new(vec![mark, Cell::from(crate::i18n::tr_data(&p.src)), speed, Cell::from(p.lag_h.map(|l| t!("{0} ч", l)).unwrap_or_default()), Cell::from(p.url.clone())])
                } else {
                    Row::new(vec![mark, speed, Cell::from(p.url.clone())])
                }
            })
            .collect();
        let title = format!("{}· {} ", t!(" Зеркала "), t!("снимок {0}", fmt_clock(v.at)));
        let block = Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(title);
        let t = if wide {
            Table::new(rows, [Constraint::Length(3), Constraint::Length(7), Constraint::Length(13), Constraint::Length(8), Constraint::Min(20)])
                .header(Row::new(vec!["", t!("откуда"), t!("скорость"), t!("отстаёт"), t!("адрес")]).style(dim()))
        } else {
            Table::new(rows, [Constraint::Length(2), Constraint::Length(13), Constraint::Min(10)]).header(Row::new(vec!["", t!("скорость"), t!("адрес")]).style(dim()))
        };
        f.render_stateful_widget(t.row_highlight_style(Style::new().add_modifier(Modifier::REVERSED)).block(block), table, &mut self.m_state);
        if hints > 0 {
            let l: Vec<Line> = v.st.hints.iter().map(|h| Line::styled(format!("💡 {h}"), Style::new().fg(Color::Yellow))).collect();
            f.render_widget(Paragraph::new(l).wrap(Wrap { trim: true }), hint_area);
        }
    }
}

/// Этапы команды (строки «[3/6] Загрузка») и итог: сделано, ошибка, не понадобилось.
fn stage_summary(stages: &[(u32, u32, String)], code: Option<i32>) -> String {
    let Some((last_n, total, last_title)) = stages.last() else { return String::new() };
    let done: Vec<&str> = stages.iter().map(|s| s.2.as_str()).collect();
    match code {
        None => t!("этап {0}/{1}: {2}", last_n, total, last_title),
        Some(0) if last_n < total => t!("этапы: ✓ {0} · дальше не понадобилось", done.join(" ✓ ")),
        Some(0) => t!("этапы: ✓ {0}", done.join(" ✓ ")),
        Some(c) => {
            let ok = &done[..done.len() - 1];
            let prefix = if ok.is_empty() { String::new() } else { format!("✓ {} ", ok.join(" ✓ ")) };
            let mark = if c == 130 { "⊘" } else { "✗" };
            let rest = if last_n < total { t!(" · не выполнены: {0}–{1}", last_n + 1, total) } else { String::new() };
            t!("этапы: {0}{1} {2}{3}", prefix, mark, last_title, rest)
        }
    }
}

// ---------- VPN ----------

fn on_off(b: bool) -> String {
    if b { t!("вкл") } else { t!("выкл") }.into()
}

/// Задержка: число и словесная оценка — состояние читается и без цвета.
fn delay_span(d: Option<&u64>) -> Span<'static> {
    match d {
        None => Span::styled(t!("— не замерено"), dim()),
        Some(0) => Span::styled(t!("✗ нет ответа"), Style::new().fg(Color::Red)),
        Some(&d) if d < 300 => Span::styled(t!("{0} мс · быстро", d), Style::new().fg(Color::Green)),
        Some(&d) if d < 800 => Span::styled(t!("{0} мс · средне", d), Style::new().fg(Color::Yellow)),
        Some(&d) => Span::styled(t!("{0} мс · медленно", d), Style::new().fg(Color::Red)),
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
    /// Снимок VPN собирается в фоне раз в 3 секунды, пока открыт экран VPN, и сразу после действий.
    fn vpn_poll(&mut self) {
        match self.v.job.poll() {
            Poll::Done(page) => self.set_vpn_page(page),
            Poll::Lost => self.msg = t!("состояние VPN не получено: {0}", lost_msg()),
            Poll::Idle | Poll::Pending => {}
        }
        self.v.job.take_again();
        match self.v.testing.poll() {
            Poll::Done(m) => {
                self.msg = m;
                self.v.at = None;
            }
            Poll::Lost => self.msg = t!("замер прервался без ответа").into(),
            Poll::Idle | Poll::Pending => {}
        }
        if self.scr == Screen::Vpn && !self.v.job.busy() && self.v.at.map(|t| t.elapsed() > Duration::from_secs(3)).unwrap_or(true) {
            let collect = self.ui.vpn_collector();
            let mirrors = self.b.default_mirrors();
            match self.v.job.request(move || collect(mirrors)) {
                Ok(_) => self.v.at = Some(Instant::now()),
                Err(e) => self.msg = e,
            }
        }
    }

    fn set_vpn_page(&mut self, page: VpnPage) {
        if !self.v.group_set {
            self.v.group = page.snap.groups.iter().position(|g| g.kind == "Selector").unwrap_or(0);
        }
        self.v.group = self.v.group.min(page.snap.groups.len().saturating_sub(1));
        // подтверждённой записи больше нет — подтверждение снимается, а не переходит на соседнюю строку
        if let Some((id, name)) = &self.v.confirm {
            if !page.state.subs.iter().any(|s| &s.id == id) {
                self.msg = t!("«{0}» уже удалена или изменилась — удаление отменено", tui_vpn_label(name));
                self.v.confirm = None;
            }
        }
        // список подписок мог стать короче — выбор остаётся на существующей строке
        if let Some(i) = self.v.subs.selected() {
            self.v.subs.select(Some(i.min(page.state.subs.len().saturating_sub(1))));
        }
        self.v.page = Some(page);
    }

    fn service(&self) -> &str {
        self.v.page.as_ref().map(|p| p.service.as_str()).unwrap_or("")
    }

    /// Сохранить изменение настройки и применить его к ядру — в фоне.
    fn vpn_change(&mut self, label: String, change: impl FnOnce(&mut Config) + Send + 'static) {
        let mirrors = self.b.default_mirrors();
        self.start_action(label, move || {
            let user = match cli_user_context() { Ok(user) => user, Err(error) => return error };
            let mut c = match Config::load(mirrors) {
                Ok(c) => c,
                Err(e) => return t!("конфиг не прочитан: {0}", e),
            };
            change(&mut c);
            if let Err(e) = c.save() {
                return t!("не сохранено: {0}", e);
            }
            let log = std::cell::RefCell::new(vec![]);
            if let Err(e) = vpn::apply_saved(&c, user.as_ref(), &|s| log.borrow_mut().push(s.to_string())) {
                log.borrow_mut().push(t!("ошибка: {0}", e));
            }
            log.into_inner().join("; ")
        });
    }

    fn vpn_set_number(&mut self, key: &str, s: &str) -> Result<(), String> {
        let c = self.cfg().map_err(|e| t!("конфиг не прочитан: {0}", e))?;
        let n: i64 = s.parse().map_err(|_| t!("нужно целое число").to_string())?;
        match key {
            "vpn_port" => {
                // короткие причины — поле ввода в одну строку
                if !(1024..=65535).contains(&n) {
                    return Err(t!("вне диапазона 1024–65535").into());
                }
                let mut probe = c.clone();
                probe.vpn_port = n as u16;
                if vpn::check_port(&probe).is_err() {
                    return Err(t!("{0} — порт своего DNS; выключи свой DNS или выбери другой", n));
                }
                let port = probe.vpn_port;
                self.vpn_change(t!("порт прокси {0}", port), move |c| c.vpn_port = port);
            }
            "vpn_sub_update_h" => {
                if !(1..=MAX_HOURS).contains(&n) {
                    return Err(t!("неверное значение").into());
                }
                self.vpn_change(t!("интервал подписок {0} ч", n), move |c| c.vpn_sub_update_h = n);
            }
            _ => return Err(t!("неверное значение").into()),
        }
        Ok(())
    }

    fn key_vpn(&mut self, k: KeyCode) -> Args {
        if let Some((id, name)) = self.v.confirm.clone() {
            // подтверждение держится до явного ответа: y/Enter — удалить, n/Esc/q — отмена
            match k {
                KeyCode::Char('y') | KeyCode::Enter => {
                    self.v.confirm = None;
                    let present = self.v.page.as_ref().map(|p| p.state.subs.iter().any(|s| s.id == id)).unwrap_or(false);
                    if !present {
                        self.msg = t!("«{0}» уже удалена или изменилась — удаление отменено", tui_vpn_label(&name));
                        return None;
                    }
                    return cmd(&["vpn", "del", &format!("id:{id}")]);
                }
                KeyCode::Char('n') | KeyCode::Esc | KeyCode::Char('q') | KeyCode::Backspace => {
                    self.v.confirm = None;
                    self.msg = t!("удаление отменено").into();
                }
                _ => {}
            }
            return None;
        }
        match k {
            KeyCode::Char('q') | KeyCode::Esc | KeyCode::Backspace => self.scr = Screen::Menu,
            KeyCode::Tab => self.v.tab = (self.v.tab + 1) % VPN_TABS.len(),
            KeyCode::BackTab => self.v.tab = (self.v.tab + VPN_TABS.len() - 1) % VPN_TABS.len(),
            KeyCode::Char(c @ '1'..='3') => self.v.tab = c as usize - '1' as usize,
            KeyCode::Char('r') => self.v.at = None,
            KeyCode::Char('s') => return cmd(&["vpn", if self.service() == "active" { "stop" } else { "start" }]),
            _ => {
                return match self.v.tab {
                    0 => self.key_vpn_servers(k),
                    1 => self.key_vpn_subs(k),
                    _ => self.key_vpn_opts(k),
                }
            }
        }
        None
    }

    fn key_vpn_servers(&mut self, k: KeyCode) -> Args {
        let Some(page) = &self.v.page else {
            self.msg = t!("загружаю состояние VPN...").into();
            return None;
        };
        let groups = page.snap.groups.clone();
        let running = page.snap.running;
        let Some(g) = groups.get(self.v.group).cloned() else {
            self.msg = if running { t!("ядро не отдало групп серверов — проверь подписку") } else { t!("VPN не запущен — s, чтобы включить") }.into();
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
                let Some(n) = g.all.get(sel).cloned() else { return None };
                if g.kind != "Selector" {
                    self.msg = t!("«{}» выбирает сервер сама ({})", tui_vpn_label(&g.name), g.kind);
                    return None;
                }
                let label = format!("{} → {}", tui_vpn_label(&g.name), tui_vpn_label(&n));
                let done = label.clone();
                self.start_action(label, move || match vpn::select(&g.name, &n) {
                    Ok(()) => done,
                    Err(e) => t!("ошибка: {0}", e),
                });
            }
            KeyCode::Char('t') if !self.v.testing.busy() => {
                let name = g.name.clone();
                let started = self.v.testing.request(move || match vpn::group_delay(&name) {
                    Ok(m) => t!("«{2}»: отвечают {} из {}", m.values().filter(|d| **d > 0).count(), m.len(), tui_vpn_label(&name)),
                    Err(e) if e.contains("timeout") || e.contains("timed out") => t!("«{0}»: ни один сервер не ответил", tui_vpn_label(&name)),
                    Err(e) => t!("замер: {0}", e),
                });
                self.msg = match started {
                    Ok(_) => t!("замеряю задержку серверов «{}»...", tui_vpn_label(&g.name)),
                    Err(e) => e,
                };
            }
            _ => {}
        }
        None
    }

    fn key_vpn_subs(&mut self, k: KeyCode) -> Args {
        let subs = self.v.page.as_ref().map(|p| p.state.subs.clone()).unwrap_or_default();
        let sel = self.v.subs.selected().unwrap_or(0);
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.v.subs.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.subs.select(Some((sel + 1).min(subs.len().saturating_sub(1)))),
            // адрес подписки — секрет: его спрашивает `upd vpn add` в терминале, а не TUI
            KeyCode::Char('n') | KeyCode::Char('a') => return cmd(&["vpn", "add"]),
            KeyCode::Char('u') if !subs.is_empty() => return cmd(&["vpn", "update"]),
            // запись адресуется неизменным id: сдвиг строк между выбором и командой не меняет цель
            KeyCode::Enter if sel < subs.len() => return cmd(&["vpn", "use", &format!("id:{}", subs[sel].id)]),
            KeyCode::Char('x') | KeyCode::Delete if sel < subs.len() => {
                self.v.confirm = Some((subs[sel].id.clone(), subs[sel].name.clone()));
                self.msg.clear();
            }
            _ => {}
        }
        None
    }

    fn key_vpn_opts(&mut self, k: KeyCode) -> Args {
        let Some(page) = &self.v.page else {
            self.msg = t!("загружаю состояние VPN...").into();
            return None;
        };
        if page.opts.is_empty() {
            self.msg = t!("не удалось открыть настройки VPN: {0}", page.opts_err);
            return None;
        }
        let n = page.opts.len();
        let sel = self.v.opts.selected().unwrap_or(0).min(n - 1);
        let key = page.opts[sel].key;
        let active = page.service == "active";
        let installed = !page.service.is_empty();
        match k {
            KeyCode::Up | KeyCode::Char('k') => self.v.opts.select(Some(sel.saturating_sub(1))),
            KeyCode::Down | KeyCode::Char('j') => self.v.opts.select(Some((sel + 1).min(n - 1))),
            KeyCode::Enter | KeyCode::Char(' ') => {
                let c = match self.cfg() {
                    Ok(c) => c,
                    Err(e) => {
                        self.msg = t!("конфиг не прочитан: {0}", e);
                        return None;
                    }
                };
                match key {
                    "run" => return cmd(&["vpn", if active { "stop" } else { "start" }]),
                    "tun" => return cmd(&["vpn", if c.vpn_tun { "proxy" } else { "tun" }]),
                    "rules" => return cmd(&["vpn", "rules"]),
                    "geo" => return cmd(&["vpn", "geo"]),
                    "core" => return cmd(&["vpn", "core", "update"]),
                    "mode" => {
                        let mode = (c.vpn_mode + 1) % 3;
                        let mirrors = self.b.default_mirrors();
                        self.start_action(t!("маршрутизация").into(), move || {
                            let user = match cli_user_context() { Ok(user) => user, Err(error) => return error };
                            let mut c = match Config::load(mirrors) {
                                Ok(c) => c,
                                Err(e) => return t!("конфиг не прочитан: {0}", e),
                            };
                            c.vpn_mode = mode;
                            if let Err(e) = c.save() {
                                return t!("не сохранено: {0}", e);
                            }
                            let log = std::cell::RefCell::new(Vec::new());
                            if let Err(error) = vpn::apply_saved_mode(&c, user.as_ref(), &|text| log.borrow_mut().push(text.to_string())) { return error; }
                            let text = log.into_inner().join("; ");
                            if text.is_empty() { t!("маршрутизация сохранена").into() } else { text }
                        });
                    }
                    "autostart" => {
                        let on = !c.vpn_autostart;
                        let mirrors = self.b.default_mirrors();
                        self.start_action(t!("запуск при загрузке").into(), move || {
                            let mut c = match Config::load(mirrors) {
                                Ok(c) => c,
                                Err(e) => return t!("конфиг не прочитан: {0}", e),
                            };
                            c.vpn_autostart = on;
                            if let Err(e) = c.save() {
                                return t!("не сохранено: {0}", e);
                            }
                            if !installed {
                                return t!("сохранено; служба появится после sudo upd install").into();
                            }
                            match vpn::autostart(on) {
                                Ok(()) => t!("запуск при загрузке: {}", on_off(on)),
                                Err(e) => format!("settings saved, but autostart not applied: {e}; retry: upd vpn restart"),
                            }
                        });
                    }
                    key @ ("vpn_port" | "vpn_sub_update_h") => self.open_input(key, String::new()),
                    key => {
                        let label = page.opts[sel].label.to_string();
                        self.vpn_change(label, move |c| {
                            let f = match key {
                                "auto" => &mut c.vpn_auto_select,
                                "ru" => &mut c.vpn_direct_ru,
                                "lan" => &mut c.vpn_direct_lan,
                                "dns" => &mut c.vpn_dns,
                                "ipv6" => &mut c.vpn_ipv6,
                                _ => &mut c.vpn_allow_lan,
                            };
                            *f = !*f;
                        });
                    }
                }
            }
            _ => {}
        }
        None
    }

    fn draw_vpn(&mut self, f: &mut Frame, area: ratatui::layout::Rect) {
        let [info, tabs, body] = Layout::vertical([Constraint::Length(3), Constraint::Length(1), Constraint::Min(3)]).areas(area);
        let Some(page) = &self.v.page else {
            f.render_widget(Paragraph::new(Line::styled(t!("загружаю состояние VPN..."), dim())), info);
            return;
        };
        let s = &page.snap;
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
            let st = page.service.as_str();
            l1.push(Span::styled(if st == "failed" { t!("● ошибка запуска (journalctl -u upd-vpn)") } else { t!("○ выключен") }, Style::new().fg(if st == "failed" { Color::Red } else { Color::DarkGray })));
        }
        let st = &page.state;
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
        let l3 = if self.input.is_some() {
            self.input_line(t!("Новое значение: "))
        } else if let Some((_, name)) = &self.v.confirm {
            Line::styled(t!("Удалить подписку «{0}»? y или Enter — удалить, n или Esc — отмена", tui_vpn_label(name)), Style::new().fg(Color::Yellow).add_modifier(Modifier::BOLD))
        } else if let Some(w) = &page.warning {
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
        // старый снимок виден по времени: обновление раз в 3 секунды
        let stale = now().saturating_sub(page.at) > 10;
        tl.push(Span::styled(format!("  {}", t!("снимок {0}", fmt_clock(page.at))), if stale { Style::new().fg(Color::Yellow) } else { dim() }));
        f.render_widget(Line::from(tl), tabs);
        let blk = |t: String| Block::bordered().border_type(BorderType::Rounded).border_style(dim()).title(t);
        let hl = Style::new().add_modifier(Modifier::REVERSED);
        match self.v.tab {
            0 => {
                if s.groups.is_empty() {
                    // разные причины пустого списка — разными словами
                    let why = if s.running {
                        t!("ядро работает, но не отдало групп серверов — проверь подписку").to_string()
                    } else if !s.error.is_empty() {
                        t!("ядро не отвечает: {0}", s.error)
                    } else if page.service == "active" {
                        t!("ядро запускается...").into()
                    } else if page.service.is_empty() {
                        t!("служба не установлена — sudo upd install").into()
                    } else {
                        t!("служба VPN выключена — s, чтобы включить").into()
                    };
                    f.render_widget(Paragraph::new(Line::styled(why, dim())).block(blk(t!(" Серверы ").into())), body);
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
                let [table, detail] = Layout::vertical([Constraint::Min(3), Constraint::Length(2)]).areas(body);
                // узкий терминал: имя и состояние; хост, узлы и дата — только когда помещаются
                let wide = table.width >= 90;
                let rows: Vec<Row> = st
                    .subs
                    .iter()
                    .enumerate()
                    .map(|(i, x)| {
                        let info = x.info.as_ref().map(sub_info).unwrap_or_default();
                        let info = info.trim_start_matches(" · ").to_string();
                        let mark = Cell::from(Span::styled(if x.active { "●" } else { "" }, Style::new().fg(Color::Cyan)));
                        let name = Cell::from(format!("{}. {}", i + 1, tui_vpn_label(&x.name)));
                        let state = Cell::from(if x.error.is_empty() { Span::raw(info) } else { Span::styled(format!("⚠ {}", x.error), Style::new().fg(Color::Red)) });
                        if wide {
                            Row::new(vec![mark, name, Cell::from(Span::styled(x.host.clone(), dim())), Cell::from(x.nodes.to_string()), Cell::from(fmt_ago(x.updated)), state])
                        } else {
                            Row::new(vec![mark, name, state])
                        }
                    })
                    .collect();
                let t = if wide {
                    Table::new(rows, [Constraint::Length(2), Constraint::Length(22), Constraint::Length(20), Constraint::Length(8), Constraint::Length(12), Constraint::Min(10)])
                        .header(Row::new(vec!["", t!("подписка"), t!("сервер"), t!("узлов"), t!("обновлена"), t!("трафик / срок")]).style(dim()))
                } else {
                    Table::new(rows, [Constraint::Length(2), Constraint::Percentage(50), Constraint::Min(8)]).header(Row::new(vec!["", t!("подписка"), t!("трафик / срок")]).style(dim()))
                };
                f.render_stateful_widget(t.row_highlight_style(hl).block(blk(t!(" Подписки — ● активная ").into())), table, &mut self.v.subs);
                if let Some(x) = self.v.subs.selected().and_then(|i| st.subs.get(i)) {
                    let mut l = format!("«{}» · {} · {} · {}", tui_vpn_label(&x.name), x.host, t!("узлов {0}", x.nodes), t!("обновлена {0}", fmt_ago(x.updated)));
                    if !x.error.is_empty() {
                        l += &format!(" · ⚠ {}", x.error);
                    }
                    f.render_widget(Paragraph::new(l).wrap(Wrap { trim: true }).style(if x.error.is_empty() { dim() } else { Style::new().fg(Color::Red) }), detail);
                }
            }
            _ => {
                if page.opts.is_empty() {
                    f.render_widget(Paragraph::new(t!("не удалось открыть настройки VPN: {0}", page.opts_err)).block(blk(t!(" Настройки ").into())), body);
                } else {
                    let rows: Vec<Row> = page.opts.iter().map(|o| Row::new(vec![Cell::from(o.label), Cell::from(o.value.clone())])).collect();
                    let t = Table::new(rows, [Constraint::Length(30), Constraint::Min(20)]).row_highlight_style(hl).block(blk(t!(" Настройки ").into()));
                    f.render_stateful_widget(t, body, &mut self.v.opts);
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
        fn vpn_collector(&self) -> fn(Vec<String>) -> VpnPage {
            |_| VpnPage::default()
        }
        fn mirror_collector(&self) -> fn() -> Result<MirrorView, String> {
            || Ok(MirrorView::default())
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
        crate::i18n::set_thread(crate::i18n::Lang::De);
        let mut app = App::new(&b, &ui, Some(Status::default()), Some(VpnPage::default()));
        let mut t = Terminal::new(TestBackend::new(118, 30)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Alles aktualisieren"));
        assert!(dump(&t).contains("Sprache / Language"));
        // й — та же клавиша, что q: из меню выходит
        app.key(KeyCode::Char(crate::i18n::latin_key('й')));
        assert!(app.quit);
        crate::i18n::set_thread(crate::i18n::Lang::Ru);
    }

    #[test]
    fn test01_menu_vpn_navigation_uses_fixed_state() {
        let b = FixtureBackend;
        let ui = FixtureUiData;
        let mut app = App::new(&b, &ui, Some(Status::default()), Some(VpnPage::default()));
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

    fn wait<T: Send + 'static>(job: &mut Job<T>) -> Poll<T> {
        let t0 = Instant::now();
        loop {
            match job.poll() {
                Poll::Pending if t0.elapsed() < Duration::from_secs(10) => std::thread::sleep(Duration::from_millis(10)),
                p => return p,
            }
        }
    }

    /// B04/B05: серия запросов держит один поток и один отложенный повтор.
    #[test]
    fn b04_job_runs_once_and_queues_one_repeat() {
        let gate = std::sync::Arc::new(std::sync::Barrier::new(2));
        let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mut job: Job<u32> = Job::default();
        for _ in 0..20 {
            let (g, n) = (gate.clone(), starts.clone());
            job.request(move || {
                n.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                g.wait();
                7
            })
            .unwrap();
        }
        gate.wait();
        assert!(matches!(wait(&mut job), Poll::Done(7)));
        assert_eq!(starts.load(std::sync::atomic::Ordering::SeqCst), 1, "двадцать нажатий — один поток");
        assert!(job.take_again(), "один отложенный повтор");
        assert!(!job.take_again());
    }

    /// B21: поток завершился без ответа — задача не висит в ожидании и запускается снова.
    #[test]
    fn b21_lost_worker_is_reported_and_retryable() {
        let mut job: Job<u32> = Job::default();
        job.request(|| -> u32 { std::panic::panic_any("worker failed") }).unwrap();
        assert!(matches!(wait(&mut job), Poll::Lost));
        assert!(!job.busy());
        job.request(|| 1).unwrap();
        assert!(matches!(wait(&mut job), Poll::Done(1)));
    }

    fn page_with_subs(ids: &[&str]) -> VpnPage {
        let mut p = VpnPage { at: now(), service: "inactive".into(), ..Default::default() };
        p.state.subs = ids
            .iter()
            .map(|id| vpn::SubPub { id: id.to_string(), name: format!("name-{id}"), host: "example.com".into(), error: if *id == "c" { "HTTP 403 — очень длинная причина ошибки подписки".into() } else { String::new() }, ..Default::default() })
            .collect();
        p
    }

    /// B22/U06: подтверждение держит id и имя, не сбрасывается посторонней клавишей и снимается, если записи не стало.
    #[test]
    fn b22_tui_confirm_is_bound_to_id() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        let mut app = App::new(&b, &ui, Some(Status::default()), Some(page_with_subs(&["a", "b", "c"])));
        app.scr = Screen::Vpn;
        app.v.tab = 1;
        app.v.subs.select(Some(2));
        assert_eq!(app.key(KeyCode::Enter), cmd(&["vpn", "use", "id:c"]));
        app.key(KeyCode::Char('x'));
        assert_eq!(app.v.confirm, Some(("c".into(), "name-c".into())));
        app.key(KeyCode::Down);
        assert!(app.v.confirm.is_some(), "другая клавиша не снимает подтверждение");
        let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("Удалить подписку «name-c»"));
        assert_eq!(app.key(KeyCode::Char('y')), cmd(&["vpn", "del", "id:c"]));
        // список изменился между нажатиями: запись удалена другим процессом
        app.key(KeyCode::Char('x'));
        app.set_vpn_page(page_with_subs(&["a", "b"]));
        assert!(app.v.confirm.is_none());
        assert!(app.msg.contains("удаление отменено"));
        app.key(KeyCode::Char('x'));
        assert!(app.key(KeyCode::Esc).is_none());
        assert!(app.v.confirm.is_none() && app.scr == Screen::Vpn, "Esc отменяет подтверждение, а не уходит с экрана");
    }

    /// U03: на узком терминале видны выбранное имя и причина ошибки.
    #[test]
    fn u03_narrow_subscriptions_keep_name_and_error() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        let mut app = App::new(&b, &ui, Some(Status::default()), Some(page_with_subs(&["a", "c"])));
        app.scr = Screen::Vpn;
        app.v.tab = 1;
        app.v.subs.select(Some(1));
        let mut t = Terminal::new(TestBackend::new(60, 20)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        let screen = dump(&t);
        assert!(screen.contains("name-c"), "{screen}");
        assert!(screen.contains("HTTP 403"), "{screen}");
    }

    /// U04: разные причины пустого списка серверов и словесная задержка.
    #[test]
    fn u04_empty_servers_explain_why() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        for (service, running, error, want) in [
            ("", false, "", "служба не установлена"),
            ("inactive", false, "", "служба VPN выключена"),
            ("active", false, "", "ядро запускается"),
            ("active", false, "connection refused", "ядро не отвечает: connection refused"),
            ("active", true, "", "не отдало групп"),
        ] {
            let mut page = VpnPage { at: now(), service: service.into(), ..Default::default() };
            page.snap.running = running;
            page.snap.error = error.into();
            let mut app = App::new(&b, &ui, Some(Status::default()), Some(page));
            app.scr = Screen::Vpn;
            let mut t = Terminal::new(TestBackend::new(110, 20)).unwrap();
            t.draw(|f| app.draw(f)).unwrap();
            assert!(dump(&t).contains(want), "{service}/{running}: {}", dump(&t));
        }
        assert_eq!(delay_span(Some(&120)).content, "120 мс · быстро");
        assert_eq!(delay_span(Some(&0)).content, "✗ нет ответа");
        assert_eq!(delay_span(None).content, "— не замерено");
    }

    /// U07: ограничения видны до отправки, неверное значение остаётся в поле с ошибкой.
    #[test]
    fn u07_input_shows_hint_and_keeps_bad_value() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        let mut app = App::new(&b, &ui, Some(Status::default()), Some(VpnPage::default()));
        app.scr = Screen::Vpn;
        app.open_input("vpn_port", String::new());
        let mut t = Terminal::new(TestBackend::new(120, 20)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("1024–65535"));
        for ch in "1053".chars() {
            app.key(KeyCode::Char(ch));
        }
        app.key(KeyCode::Enter);
        assert_eq!(app.input.as_deref(), Some("1053"), "поле не закрылось");
        assert!(app.input_err.contains("DNS"), "{}", app.input_err);
        app.key(KeyCode::Backspace);
        assert!(app.input_err.is_empty(), "правка убирает ошибку");
        app.key(KeyCode::Esc);
        assert!(app.input.is_none());
        app.scr = Screen::Aur;
        app.open_input("aur", String::new());
        app.key(KeyCode::Char('a'));
        app.key(KeyCode::Enter);
        assert!(app.input.is_some() && app.input_err.contains("2 символа"));
    }

    /// B16/U08: новый поиск сразу убирает прежние строки; после ошибки ставить нечего.
    #[test]
    fn b16_aur_error_does_not_show_previous_results() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        let mut app = App::new(&b, &ui, Some(Status::default()), None);
        app.scr = Screen::Aur;
        app.aur.query = "old".into();
        app.aur.list = vec![extras::AurPkg { name: "oldpkg".into(), version: "1".into(), ..Default::default() }];
        app.aur.table.select(Some(0));
        // новый запрос из одного символа сам по себе ошибка aur_search — без сети
        app.aur_start("x".into());
        assert!(app.aur.list.is_empty(), "прежние строки не выдаются за новые");
        assert!(app.key(KeyCode::Enter).is_none());
        let t0 = Instant::now();
        while app.aur.job.busy() && t0.elapsed() < Duration::from_secs(10) {
            app.aur_poll();
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(app.aur.failed && app.aur.list.is_empty(), "{}", app.aur.status);
        assert!(app.key(KeyCode::Enter).is_none(), "установить нечего");
        let mut t = Terminal::new(TestBackend::new(100, 20)).unwrap();
        t.draw(|f| app.draw(f)).unwrap();
        assert!(dump(&t).contains("«x» · ошибка"), "{}", dump(&t));
    }

    /// U05: этапы команды и итог.
    #[test]
    fn u05_stage_summary() {
        let st = vec![(1, 6, "Зеркала".to_string()), (2, 6, "Проверка".to_string())];
        assert_eq!(stage_summary(&st, None), "этап 2/6: Проверка");
        assert!(stage_summary(&st, Some(0)).contains("дальше не понадобилось"));
        let failed = stage_summary(&st, Some(1));
        assert!(failed.contains("✓ Зеркала") && failed.contains("✗ Проверка") && failed.contains("3–6"), "{failed}");
        assert!(stage_summary(&st, Some(130)).contains("⊘ Проверка"));
        assert_eq!(stage_summary(&[], Some(0)), "");
    }
    #[test]
    fn audit_minimal_tui_surfaces_keep_errors_and_keyboard_footer() {
        let (b, ui) = (FixtureBackend, FixtureUiData);
        for lang in crate::i18n::ALL {
            crate::i18n::set_thread(lang);
            for (width, height) in [(80, 24), (60, 18)] {
                let mut app = App::new(&b, &ui, Some(Status::default()), Some(page_with_subs(&["a", "c"])));
                app.scr = Screen::Vpn; app.v.tab = 1; app.v.subs.select(Some(1));
                let mut terminal = Terminal::new(TestBackend::new(width, height)).unwrap();
                terminal.draw(|f| app.draw(f)).unwrap();
                let screen = dump(&terminal);
                assert!(screen.contains("name-c") && screen.contains("HTTP 403"), "{screen}");
                assert!(screen.lines().rev().take(3).any(|line| line.contains(" q ")), "{screen}");
                if let Some(dir) = std::env::var_os("UPD_TUI_SNAPSHOTS") {
                    let dir = std::path::PathBuf::from(dir); std::fs::create_dir_all(&dir).unwrap();
                    std::fs::write(dir.join(format!("vpn-{}-{width}x{height}.txt", lang.code())), &screen).unwrap();
                }
                app.key(KeyCode::Esc); assert!(app.scr == Screen::Menu);
            }
        }
        crate::i18n::set_thread(crate::i18n::Lang::Ru);
    }

}
