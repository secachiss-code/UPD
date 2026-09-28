//! Системный помощник для графического интерфейса.
//!
//! Интерфейс работает от обычного пользователя; всё, что требует root (установка, зеркала, VPN,
//! живой снимок ядра VPN через закрытый сокет), он просит у помощника `upd helper`. Помощник
//! запускается systemd по обращению к сокету `/run/upd/helper.sock` и сам завершается без работы.
//! Каждый запрос проверяется через polkit (`pkcheck`) для процесса, который прислал запрос
//! (pid, время старта и uid берутся из SO_PEERCRED, а не из запроса).
//!
//! Протокол — строки JSON: запрос `Envelope`, ответ `Reply`; после `attach` помощник шлёт поток `Event`.
//! Адрес подписки передаётся только в теле запроса: он не попадает в аргументы команд, журнал и вывод.

use crate::backend::{self, Backend};
use crate::common::*;
use crate::summary::OpStatus;
use crate::{extras, vpn};
use serde::{Deserialize, Serialize};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SOCKET: &str = "/run/upd/helper.sock";
/// Чтение: состояние операции, живой снимок VPN, списки снапшотов и истории.
pub const ACTION_STATUS: &str = "io.github.upd.status";
/// Проверка обновлений (как обновление кэша в PackageKit): без пароля в активном сеансе.
pub const ACTION_CHECK: &str = "io.github.upd.check";
/// Включить или выключить VPN и выбрать сервер (как VPN в NetworkManager): без пароля в активном сеансе.
pub const ACTION_VPN: &str = "io.github.upd.vpn";
/// Изменение системы: установка, зеркала, подписки и настройки — с паролем администратора.
pub const ACTION_MANAGE: &str = "io.github.upd.manage";

/// Какое действие polkit нужно для запуска команды.
pub fn action_for(args: &[String]) -> &'static str {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["check"] => ACTION_CHECK,
        ["vpn", "start" | "stop" | "restart"] => ACTION_VPN,
        _ => ACTION_MANAGE,
    }
}

/// Без обращений и операций помощник завершается через это время (systemd запустит снова по сокету).
const IDLE_EXIT: Duration = Duration::from_secs(300);
const MAX_LINES: usize = 5000;
const MAX_REQUEST: u64 = 64 << 10;

pub fn socket_path() -> String {
    env_or("UPD_HELPER_SOCK", SOCKET)
}

// ======================= протокол =======================

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// состояние долгой операции
    Status,
    /// подписаться на вывод текущей (или последней) операции
    Attach,
    /// запустить команду upd из разрешённого списка
    Start { args: Vec<String> },
    /// ответ на вопрос операции (строка без перевода строки)
    Input { data: String },
    Cancel,
    VpnSnapshot,
    VpnSelect { group: String, name: String },
    VpnDelay { group: String },
    /// добавить подписку: адрес только здесь, не в аргументах
    VpnAdd { url: String, name: String },
    ConfigSet { key: String, value: String },
    /// snapshots | history | restart
    Query { what: String },
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Envelope {
    /// язык ответа и запускаемой команды (код ru, en…; пусто — как в настройках)
    #[serde(default)]
    pub lang: String,
    #[serde(flatten)]
    pub req: Request,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Reply {
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// polkit отказал или пользователь закрыл окно пароля
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub denied: bool,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

impl Reply {
    fn ok(data: serde_json::Value) -> Self {
        Reply { ok: true, data, ..Default::default() }
    }
    fn err(e: impl Into<String>) -> Self {
        Reply { error: e.into(), ..Default::default() }
    }
}

/// Вопрос, которого ждёт операция.
#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PromptKind {
    /// да/нет; по умолчанию — да
    YesNo { default_yes: bool },
    /// пароль (ввод не показывается)
    Secret,
    /// произвольный ответ или просто Enter
    Text,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
#[serde(tag = "ev", rename_all = "snake_case")]
pub enum Event {
    /// началась операция (или повтор при подключении): вывод с чистого листа
    Reset { command: String, started: i64 },
    Line { text: String },
    /// незавершённая строка (прогресс, вопрос)
    Partial { text: String },
    Stage { n: u32, m: u32, title: String },
    Prompt { text: String, kind: PromptKind },
    /// вопрос снят: операция продолжила вывод
    Answered,
    Exit { code: i32 },
}

// ======================= разбор вывода =======================

/// Сборка строк из вывода терминала: убирает ANSI-последовательности, `\r` переписывает строку.
#[derive(Default)]
pub struct Term {
    pub lines: VecDeque<String>,
    pub partial: String,
    utf8: Vec<u8>,
    esc: u8,
    pending_cr: bool,
}

impl Term {
    /// Добавить байты; вернуть завершённые строки.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = vec![];
        self.utf8.extend_from_slice(bytes);
        let valid = match std::str::from_utf8(&self.utf8) {
            Ok(s) => s.len(),
            Err(e) if e.error_len().is_none() => e.valid_up_to(),
            Err(e) => e.valid_up_to() + e.error_len().unwrap_or(1),
        };
        let chunk: Vec<u8> = self.utf8.drain(..valid).collect();
        for ch in String::from_utf8_lossy(&chunk).chars() {
            // 0 — текст, 1 — после ESC, 2 — CSI, 3 — OSC, 4 — ESC внутри OSC, 5 — выбор набора символов
            match (self.esc, ch) {
                (0, '\x1b') => self.esc = 1,
                (1, '[') => self.esc = 2,
                (1, ']') => self.esc = 3,
                (1, '(' | ')') => self.esc = 5,
                (1, _) | (5, _) => self.esc = 0,
                (2, c) if ('@'..='~').contains(&c) => self.esc = 0,
                (2, _) => {}
                (3, '\x07') => self.esc = 0,
                (3, '\x1b') => self.esc = 4,
                (3, _) => {}
                (4, _) => self.esc = 0,
                (_, '\n') => {
                    self.pending_cr = false;
                    let line = std::mem::take(&mut self.partial);
                    self.push(line.clone());
                    done.push(line);
                }
                (_, '\r') => self.pending_cr = true,
                (_, '\x08') => {
                    self.partial.pop();
                }
                (_, c) if c.is_control() && c != '\t' => {}
                (_, c) => {
                    if self.pending_cr {
                        // прогресс pacman и curl перерисовывает строку через \r
                        self.partial.clear();
                        self.pending_cr = false;
                    }
                    self.partial.push(c);
                }
            }
        }
        done
    }

    fn push(&mut self, line: String) {
        self.lines.push_back(line);
        while self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }
}

/// «[3/6] Загрузка» → (3, 6, «Загрузка»).
pub fn parse_stage(line: &str) -> Option<(u32, u32, String)> {
    let (nm, title) = line.trim().strip_prefix('[')?.split_once("] ")?;
    let (n, m) = nm.split_once('/')?;
    let (n, m): (u32, u32) = (n.parse().ok()?, m.parse().ok()?);
    (n >= 1 && n <= m && m <= 20 && !title.trim().is_empty()).then(|| (n, m, title.trim().to_string()))
}

/// Похожа ли незавершённая строка на вопрос, ждущий ответа.
pub fn prompt_kind(partial: &str) -> Option<PromptKind> {
    let p = partial.trim_end();
    if p.is_empty() {
        return None;
    }
    let low = p.to_lowercase();
    if low.ends_with("[y/n]") || low.ends_with("[y/n]:") {
        // [Y/n] — по умолчанию да, [y/N] — нет
        let default_yes = p.contains("[Y/n]");
        return Some(PromptKind::YesNo { default_yes });
    }
    if low.contains("password") || low.contains("пароль") || low.contains("passwort") || low.contains("密码") || low.contains("كلمة") {
        return Some(PromptKind::Secret);
    }
    if p.ends_with(':') || p.ends_with('?') || p.ends_with('>') || p.ends_with(')') {
        return Some(PromptKind::Text);
    }
    None
}

// ======================= разрешённые команды =======================

fn safe_token(s: &str) -> bool {
    !s.is_empty() && s.len() <= 256 && s.chars().all(|c| c.is_ascii_alphanumeric() || "-_.:".contains(c))
}

fn safe_url(s: &str) -> bool {
    (s.starts_with("https://") || s.starts_with("http://")) && s.len() <= 2048 && !s.chars().any(|c| c.is_whitespace() || c.is_control())
}

/// Команды upd, которые интерфейс может запустить через помощника. Всё прочее отклоняется.
pub fn allowed(args: &[String]) -> bool {
    let a: Vec<&str> = args.iter().map(String::as_str).collect();
    match a.as_slice() {
        ["update"] | ["check"] | ["clean"] | ["restart"] => true,
        ["mirrors", "check" | "rescan" | "apply"] => true,
        ["mirrors", "add" | "del", url] => safe_url(url),
        ["vpn", "start" | "stop" | "restart" | "update" | "tun" | "proxy" | "rule" | "global" | "direct" | "geo"] => true,
        ["vpn", "core", "check" | "update" | "reinstall"] => true,
        ["vpn", "use" | "del", id] => safe_token(id),
        ["aur", "install", pkgs @ ..] => !pkgs.is_empty() && pkgs.len() <= 32 && pkgs.iter().all(|p| extras::valid_pkg_name(p)),
        _ => false,
    }
}

// ======================= права =======================

#[derive(Clone, Copy, Debug)]
struct Peer {
    pid: i32,
    uid: u32,
}

fn peer_of(s: &UnixStream) -> Option<Peer> {
    let mut cred: libc::ucred = unsafe { std::mem::zeroed() };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    let r = unsafe { libc::getsockopt(s.as_raw_fd(), libc::SOL_SOCKET, libc::SO_PEERCRED, &mut cred as *mut _ as *mut libc::c_void, &mut len) };
    (r == 0 && cred.pid > 0).then_some(Peer { pid: cred.pid, uid: cred.uid })
}

/// Время старта процесса (поле 22 /proc/PID/stat): вместе с pid однозначно называет процесс для polkit.
fn start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

pub fn user_name(uid: u32) -> Option<String> {
    let mut pw: libc::passwd = unsafe { std::mem::zeroed() };
    let mut buf = vec![0 as libc::c_char; 4096];
    let mut res: *mut libc::passwd = std::ptr::null_mut();
    let r = unsafe { libc::getpwuid_r(uid, &mut pw, buf.as_mut_ptr(), buf.len(), &mut res) };
    if r != 0 || res.is_null() {
        return None;
    }
    Some(unsafe { std::ffi::CStr::from_ptr(pw.pw_name) }.to_string_lossy().into_owned())
}

enum Auth {
    Yes,
    No(String),
}

/// Проверка через polkit. root разрешено всё; в тестовом режиме (UPD_STATE_DIR) можно разрешить явно.
fn authorize(p: Peer, action: &str) -> Auth {
    if p.uid == 0 || (test_mode() && std::env::var("UPD_HELPER_ALLOW").as_deref() == Ok("1")) {
        return Auth::Yes;
    }
    let Some(start) = start_time(p.pid) else { return Auth::No(t!("процесс запроса уже завершился").into()) };
    let subject = format!("{},{},{}", p.pid, start, p.uid);
    let mut cmd = Command::new("pkcheck");
    cmd.args(["--action-id", action, "--process", &subject, "--allow-user-interaction"]).stdin(Stdio::null());
    match capture(&mut cmd, Some(64 << 10)) {
        Ok(o) => match o.status.code() {
            Some(0) => Auth::Yes,
            Some(3) => Auth::No(t!("окно подтверждения закрыто").into()),
            Some(2) => Auth::No(t!("нет агента polkit для ввода пароля").into()),
            _ => Auth::No(t!("нет прав (polkit: {0})", action)),
        },
        Err(e) => Auth::No(t!("не удалось проверить права: {0}", e)),
    }
}

// ======================= операция =======================

struct Op {
    command: String,
    owner: u32,
    pgid: i32,
    input: Option<File>,
    term: Term,
    stage: Option<(u32, u32, String)>,
    prompt: Option<(String, PromptKind)>,
    started: i64,
    exit: Option<i32>,
    finished: i64,
    cancels: u32,
}

#[derive(Default)]
struct State {
    op: Option<Op>,
    subs: Vec<Sender<Event>>,
    clients: usize,
    last_activity: Option<Instant>,
}

type Shared = Arc<Mutex<State>>;

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn broadcast(st: &mut State, ev: Event) {
    st.subs.retain(|tx| tx.send(ev.clone()).is_ok());
}

fn op_status(st: &State) -> OpStatus {
    match &st.op {
        Some(op) => OpStatus {
            running: op.exit.is_none(),
            command: op.command.clone(),
            stage: op.stage.clone(),
            started: op.started,
            last_exit: op.exit,
            last_command: op.command.clone(),
            finished: op.finished,
            waiting: op.prompt.is_some(),
        },
        None => OpStatus::default(),
    }
}

/// Вывод операции: строки, этапы, вопросы.
fn on_output(shared: &Shared, bytes: &[u8]) {
    let mut st = lock(shared);
    let Some(op) = st.op.as_mut() else { return };
    let lines = op.term.feed(bytes);
    let mut evs = vec![];
    if op.prompt.is_some() && (!lines.is_empty() || op.term.partial.is_empty()) {
        op.prompt = None;
        evs.push(Event::Answered);
    }
    for l in lines {
        if let Some((n, m, title)) = parse_stage(&l) {
            op.stage = Some((n, m, title.clone()));
            evs.push(Event::Stage { n, m, title });
        }
        evs.push(Event::Line { text: l });
    }
    evs.push(Event::Partial { text: op.term.partial.clone() });
    for ev in evs {
        broadcast(&mut st, ev);
    }
}

/// Нет нового вывода, а строка не закончена и похожа на вопрос — операция ждёт ответа.
fn check_prompt(shared: &Shared) {
    let mut st = lock(shared);
    let Some(op) = st.op.as_mut() else { return };
    if op.exit.is_some() || op.prompt.is_some() {
        return;
    }
    if let Some(kind) = prompt_kind(&op.term.partial) {
        let text = op.term.partial.trim().to_string();
        op.prompt = Some((text.clone(), kind.clone()));
        broadcast(&mut st, Event::Prompt { text, kind });
    }
}

fn finish(shared: &Shared, code: i32) {
    let mut st = lock(shared);
    let Some(op) = st.op.as_mut() else { return };
    if !op.term.partial.is_empty() {
        let l = std::mem::take(&mut op.term.partial);
        op.term.push(l.clone());
        broadcast(&mut st, Event::Line { text: l });
    }
    let Some(op) = st.op.as_mut() else { return };
    op.exit = Some(code);
    op.finished = now();
    op.prompt = None;
    op.input = None;
    println!("upd helper: {} → {code}", op.command);
    st.last_activity = Some(Instant::now());
    broadcast(&mut st, Event::Exit { code });
}

/// Запуск `upd ARGS` в отдельном PTY: pacman, apt и sudo видят настоящий терминал.
fn spawn_pty(args: &[String], env: &[(String, String)]) -> std::io::Result<(std::process::Child, File)> {
    let size = libc::winsize { ws_row: 40, ws_col: 120, ws_xpixel: 0, ws_ypixel: 0 };
    let (mut m, mut s) = (-1, -1);
    if unsafe { libc::openpty(&mut m, &mut s, std::ptr::null_mut(), std::ptr::null(), &size) } != 0 {
        return Err(std::io::Error::last_os_error());
    }
    let master = unsafe { File::from_raw_fd(m) };
    let slave = unsafe { File::from_raw_fd(s) };
    unsafe {
        libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC);
    }
    // в тестовом режиме вместо upd можно подставить свою программу (сквозной тест протокола)
    let exe = match std::env::var_os("UPD_HELPER_EXE").filter(|_| test_mode()) {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let mut cmd = Command::new(exe);
    cmd.args(args).env("PAGER", "cat").env("TERM", "xterm-256color").env("UPD_GUI", "1");
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdin(Stdio::from(slave.try_clone()?)).stdout(Stdio::from(slave.try_clone()?)).stderr(Stdio::from(slave));
    unsafe {
        cmd.pre_exec(|| {
            if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY as _, 0) < 0 {
                return Err(std::io::Error::last_os_error());
            }
            Ok(())
        });
    }
    let child = cmd.spawn()?;
    Ok((child, master))
}

fn lang_env(lang: &str) -> Vec<(String, String)> {
    // язык интерфейса «auto» берётся из окружения, а у службы его нет: передаём язык того, кто запустил
    match crate::i18n::Lang::from_code(lang) {
        Some(l) if conf_lang() == "auto" => vec![("LANG".into(), format!("{}.UTF-8", locale_of(l)))],
        _ => vec![],
    }
}

fn locale_of(l: crate::i18n::Lang) -> &'static str {
    use crate::i18n::Lang::*;
    match l {
        Ru => "ru_RU",
        En => "en_US",
        De => "de_DE",
        It => "it_IT",
        Zh => "zh_CN",
        Ar => "ar_EG",
    }
}

fn begin(st: &mut State, command: String, owner: u32) -> Result<(), String> {
    if st.op.as_ref().is_some_and(|o| o.exit.is_none()) {
        return Err(t!("уже идёт операция: {0}", st.op.as_ref().map(|o| o.command.clone()).unwrap_or_default()));
    }
    st.op = Some(Op {
        command: command.clone(),
        owner,
        pgid: 0,
        input: None,
        term: Term::default(),
        stage: None,
        prompt: None,
        started: now(),
        exit: None,
        finished: 0,
        cancels: 0,
    });
    broadcast(st, Event::Reset { command, started: now() });
    Ok(())
}

fn start_command(shared: &Shared, args: Vec<String>, p: Peer, lang: &str) -> Result<(), String> {
    if !allowed(&args) {
        return Err(t!("команда не разрешена: {0}", args.join(" ")));
    }
    let mut env = lang_env(lang);
    // AUR и Flatpak пользователя работают от имени того, кто запросил операцию
    if let Some(name) = user_name(p.uid).filter(|_| p.uid != 0) {
        env.push(("SUDO_USER".into(), name));
        env.push(("SUDO_UID".into(), p.uid.to_string()));
    }
    let mut st = lock(shared);
    begin(&mut st, args.join(" "), p.uid)?;
    let (mut child, master) = match spawn_pty(&args, &env) {
        Ok(x) => x,
        Err(e) => {
            drop(st);
            finish(shared, 127);
            return Err(t!("не удалось запустить upd: {0}", e));
        }
    };
    let reader = master.try_clone().map_err(|e| e.to_string())?;
    if let Some(op) = st.op.as_mut() {
        op.pgid = child.id() as i32;
        op.input = Some(master);
    }
    drop(st);
    println!("upd helper: uid {} → upd {}", p.uid, args.join(" "));
    let (sh, done) = (shared.clone(), Arc::new(std::sync::atomic::AtomicBool::new(false)));
    let done_r = done.clone();
    // чтение вывода; по тишине проверяем, не ждёт ли команда ответа
    let _ = spawn_thread("helper-read", move || {
        let mut reader = reader;
        let fd = reader.as_raw_fd();
        let mut buf = [0u8; 8192];
        loop {
            let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
            let r = unsafe { libc::poll(&mut pfd, 1, 400) };
            if r == 0 {
                if done_r.load(std::sync::atomic::Ordering::Relaxed) {
                    break;
                }
                check_prompt(&sh);
                continue;
            }
            match reader.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => on_output(&sh, &buf[..n]),
            }
        }
    });
    let sh = shared.clone();
    let _ = spawn_thread("helper-wait", move || {
        let code = match child.wait() {
            Ok(s) => s.code().unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&s).unwrap_or(0)),
            Err(_) => 1,
        };
        // дать читателю забрать последний вывод
        std::thread::sleep(Duration::from_millis(300));
        done.store(true, std::sync::atomic::Ordering::Relaxed);
        finish(&sh, code);
    });
    Ok(())
}

/// Операция внутри помощника (без отдельного процесса): добавление подписки.
fn start_inproc(shared: &Shared, command: &str, p: Peer, lang: &str, f: impl FnOnce(&dyn Fn(&str)) -> Result<(), String> + Send + 'static) -> Result<(), String> {
    begin(&mut lock(shared), command.into(), p.uid)?;
    let sh = shared.clone();
    let lang = crate::i18n::Lang::from_code(lang);
    spawn_thread("helper-op", move || {
        if let Some(l) = lang {
            crate::i18n::set_thread(l);
        }
        let log = |s: &str| on_output(&sh, format!("{s}\n").as_bytes());
        let code = match f(&log) {
            Ok(()) => 0,
            Err(e) => {
                log(&t!("ошибка: {0}", e));
                1
            }
        };
        finish(&sh, code);
    })
    .map(|_| ())
}

// ======================= настройки =======================

/// Изменить одну настройку и применить её так же, как это делает TUI.
pub fn apply_setting(b: &dyn Backend, key: &str, value: &str, log: Log) -> Result<(), String> {
    let mut c = Config::load(b.default_mirrors())?;
    let old_autostart = c.vpn_autostart;
    c.set(key, value)?;
    c.save().map_err(|e| t!("не сохранено: {0}", e))?;
    match key {
        "vpn_mode" => {
            vpn::write_config(&c)?;
            if vpn::running() {
                vpn::set_mode(c.vpn_mode_name())?;
            }
        }
        "vpn_autostart" => {
            if c.vpn_autostart != old_autostart && !unit_state(vpn::SERVICE).is_empty() {
                vpn::autostart(c.vpn_autostart)?;
            }
        }
        "vpn_sub_update_h" | "vpn_core_check_h" => {}
        k if k.starts_with("vpn_") => vpn::apply(&c, log)?,
        _ => {}
    }
    Ok(())
}

// ======================= обработка запросов =======================

fn handle(shared: &Shared, b: &dyn Backend, mut stream: UnixStream) {
    let Some(p) = peer_of(&stream) else { return };
    let mut line = String::new();
    let _ = stream.set_read_timeout(Some(Duration::from_secs(30)));
    if BufReader::new((&stream).take(MAX_REQUEST)).read_line(&mut line).unwrap_or(0) == 0 {
        return;
    }
    let env: Envelope = match serde_json::from_str(&line) {
        Ok(e) => e,
        Err(e) => return send(&mut stream, &Reply::err(format!("bad request: {e}"))),
    };
    if let Some(l) = crate::i18n::Lang::from_code(&env.lang) {
        crate::i18n::set_thread(l);
    }
    let action = match &env.req {
        Request::Status | Request::Attach | Request::VpnSnapshot | Request::Query { .. } => ACTION_STATUS,
        Request::VpnSelect { .. } | Request::VpnDelay { .. } => ACTION_VPN,
        Request::Start { args } => action_for(args),
        // отвечать на вопросы и отменять может тот, кто запустил операцию, без повторного пароля
        Request::Input { .. } | Request::Cancel if lock(shared).op.as_ref().is_some_and(|o| o.owner == p.uid) => "",
        _ => ACTION_MANAGE,
    };
    if !action.is_empty() {
        if let Auth::No(why) = authorize(p, action) {
            return send(&mut stream, &Reply { denied: true, ..Reply::err(why) });
        }
    }
    let reply = match env.req {
        Request::Status => Reply::ok(serde_json::to_value(op_status(&lock(shared))).unwrap_or_default()),
        Request::Attach => return attach(shared, stream),
        Request::Start { args } => match start_command(shared, args, p, &env.lang) {
            Ok(()) => Reply::ok(serde_json::Value::Null),
            Err(e) => Reply::err(e),
        },
        Request::Input { data } => {
            let mut st = lock(shared);
            match st.op.as_mut().filter(|o| o.exit.is_none()).and_then(|o| o.input.as_mut()) {
                Some(f) => match f.write_all(format!("{}\n", data.replace(['\n', '\r'], "")).as_bytes()) {
                    Ok(()) => Reply::ok(serde_json::Value::Null),
                    Err(e) => Reply::err(e.to_string()),
                },
                None => Reply::err(t!("операция не ждёт ввода")),
            }
        }
        Request::Cancel => {
            let mut st = lock(shared);
            match st.op.as_mut().filter(|o| o.exit.is_none() && o.pgid > 0) {
                Some(op) => {
                    // первая отмена — как Ctrl+C в терминале, повторная — завершить
                    let sig = if op.cancels == 0 { libc::SIGINT } else { libc::SIGTERM };
                    op.cancels += 1;
                    unsafe { libc::killpg(op.pgid, sig) };
                    Reply::ok(serde_json::Value::Null)
                }
                None => Reply::err(t!("нечего отменять")),
            }
        }
        Request::VpnSnapshot => Reply::ok(serde_json::to_value(vpn::snapshot()).unwrap_or_default()),
        Request::VpnSelect { group, name } => match vpn::select(&group, &name) {
            Ok(()) => Reply::ok(serde_json::Value::Null),
            Err(e) => Reply::err(e),
        },
        Request::VpnDelay { group } => match vpn::group_delay(&group) {
            Ok(d) => Reply::ok(serde_json::to_value(d).unwrap_or_default()),
            Err(e) => Reply::err(e),
        },
        Request::VpnAdd { url, name } => {
            let mirrors = b.default_mirrors();
            match start_inproc(shared, "vpn add", p, &env.lang, move |log| {
                let c = Config::load(mirrors)?;
                vpn::add_sub(&url, &name, &c, log)?;
                if vpn::service_active() {
                    vpn::apply(&c, log)?;
                } else {
                    log(t!("подписка добавлена. Запуск VPN: upd vpn start"));
                }
                Ok(())
            }) {
                Ok(()) => Reply::ok(serde_json::Value::Null),
                Err(e) => Reply::err(e),
            }
        }
        Request::ConfigSet { key, value } => {
            let log = |s: &str| println!("upd helper: {s}");
            match apply_setting(b, &key, &value, &log) {
                Ok(()) => Reply::ok(serde_json::Value::Null),
                Err(e) => Reply::err(e),
            }
        }
        Request::Query { what } => match what.as_str() {
            "snapshots" => {
                let mut v = extras::snap_list(30);
                v.push(String::new());
                v.extend(extras::rollback_hint());
                Reply::ok(serde_json::to_value(v).unwrap_or_default())
            }
            "history" => Reply::ok(serde_json::to_value(b.history(300)).unwrap_or_default()),
            // от имени пользователя /proc/PID/maps чужих процессов не читается — список собирает root
            "restart" => {
                let r = needs_restart();
                Reply::ok(serde_json::json!({ "services": r.services, "critical": r.critical, "apps": r.apps, "unknown": r.unknown }))
            }
            _ => Reply::err("unknown query"),
        },
    };
    send(&mut stream, &reply);
}

fn send(stream: &mut UnixStream, r: &impl Serialize) {
    let mut s = serde_json::to_string(r).unwrap_or_default();
    s.push('\n');
    let _ = stream.write_all(s.as_bytes());
}

/// Поток событий операции: сначала всё накопленное, затем живой вывод.
fn attach(shared: &Shared, mut stream: UnixStream) {
    let (tx, rx): (Sender<Event>, Receiver<Event>) = mpsc::channel();
    let replay = {
        let mut st = lock(shared);
        let mut v = vec![];
        if let Some(op) = &st.op {
            v.push(Event::Reset { command: op.command.clone(), started: op.started });
            v.extend(op.term.lines.iter().map(|l| Event::Line { text: l.clone() }));
            if let Some((n, m, title)) = &op.stage {
                v.push(Event::Stage { n: *n, m: *m, title: title.clone() });
            }
            v.push(Event::Partial { text: op.term.partial.clone() });
            if let Some((text, kind)) = &op.prompt {
                v.push(Event::Prompt { text: text.clone(), kind: kind.clone() });
            }
            if let Some(code) = op.exit {
                v.push(Event::Exit { code });
            }
        }
        st.subs.push(tx);
        v
    };
    send(&mut stream, &Reply::ok(serde_json::Value::Null));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    for ev in replay {
        send(&mut stream, &ev);
    }
    // клиент закрыл соединение — send упадёт, поток завершится, отправитель удалится при следующей рассылке
    while let Ok(ev) = rx.recv() {
        let mut s = serde_json::to_string(&ev).unwrap_or_default();
        s.push('\n');
        if stream.write_all(s.as_bytes()).is_err() {
            break;
        }
    }
}

/// Слушающий сокет: от systemd (LISTEN_FDS) или свой — для ручного запуска.
fn listener() -> Result<UnixListener, String> {
    let from_systemd = std::env::var("LISTEN_PID").ok().and_then(|p| p.parse::<u32>().ok()) == Some(std::process::id())
        && std::env::var("LISTEN_FDS").ok().and_then(|n| n.parse::<u32>().ok()).unwrap_or(0) >= 1;
    if from_systemd {
        return Ok(unsafe { UnixListener::from_raw_fd(3) });
    }
    let path = socket_path();
    if let Some(dir) = std::path::Path::new(&path).parent() {
        std::fs::create_dir_all(dir).map_err(|e| format!("{}: {e}", dir.display()))?;
    }
    let _ = std::fs::remove_file(&path);
    let l = UnixListener::bind(&path).map_err(|e| format!("{path}: {e}"))?;
    use std::os::unix::fs::PermissionsExt;
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o666)).map_err(|e| e.to_string())?;
    Ok(l)
}

/// `upd helper`: служба по сокету; завершается после IDLE_EXIT без клиентов и операций.
pub fn serve() -> i32 {
    if !is_root() && !test_mode() {
        eprintln!("{}", t!("upd helper: нужны права root"));
        return 1;
    }
    let l = match listener() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("upd helper: {e}");
            return 1;
        }
    };
    if let Err(e) = backend::detect() {
        eprintln!("upd helper: {e}");
        return 1;
    }
    let shared: Shared = Arc::new(Mutex::new(State { last_activity: Some(Instant::now()), ..Default::default() }));
    let fd = l.as_raw_fd();
    loop {
        let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
        if r <= 0 {
            let st = lock(&shared);
            let busy = st.clients > 0 || st.op.as_ref().is_some_and(|o| o.exit.is_none());
            if !busy && st.last_activity.is_some_and(|t| t.elapsed() > IDLE_EXIT) {
                return 0;
            }
            continue;
        }
        let Ok((stream, _)) = l.accept() else { continue };
        {
            let mut st = lock(&shared);
            st.clients += 1;
            st.last_activity = Some(Instant::now());
        }
        let sh = shared.clone();
        let _ = spawn_thread("helper-conn", move || {
            // Backend не разделяется между потоками; определение дешёвое (PATH и os-release)
            if let Ok(b) = backend::detect() {
                handle(&sh, b.as_ref(), stream);
            }
            let mut st = lock(&sh);
            st.clients -= 1;
            st.last_activity = Some(Instant::now());
        });
    }
}

// ======================= клиент =======================

pub fn available() -> bool {
    std::path::Path::new(&socket_path()).exists()
}

fn connect(req: &Request) -> Result<(UnixStream, Reply), String> {
    let path = socket_path();
    let mut s = UnixStream::connect(&path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound || e.kind() == std::io::ErrorKind::ConnectionRefused {
            t!("помощник upd недоступен — установите upd заново (sudo upd install)").into()
        } else {
            format!("{path}: {e}")
        }
    })?;
    let env = Envelope { lang: crate::i18n::cur().code().into(), req: req.clone() };
    send(&mut s, &env);
    // ответ может ждать ввода пароля в окне polkit
    let _ = s.set_read_timeout(Some(Duration::from_secs(300)));
    let mut line = String::new();
    BufReader::new(&s).read_line(&mut line).map_err(|e| e.to_string())?;
    let r: Reply = serde_json::from_str(&line).map_err(|_| t!("помощник upd не ответил").to_string())?;
    Ok((s, r))
}

/// Запрос с одним ответом. Ошибка содержит понятную причину (в том числе отказ polkit).
pub fn call(req: &Request) -> Result<serde_json::Value, String> {
    let (_, r) = connect(req)?;
    if r.ok {
        Ok(r.data)
    } else {
        Err(r.error)
    }
}

pub fn call_as<T: for<'de> Deserialize<'de>>(req: &Request) -> Result<T, String> {
    serde_json::from_value(call(req)?).map_err(|e| e.to_string())
}

/// Подписка на события операции; итератор кончается, когда помощник закрыл соединение.
pub fn attach_events() -> Result<impl Iterator<Item = Event>, String> {
    let (s, r) = connect(&Request::Attach)?;
    if !r.ok {
        return Err(r.error);
    }
    let _ = s.set_read_timeout(None);
    Ok(BufReader::new(s).lines().map_while(Result::ok).filter_map(|l| serde_json::from_str(&l).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn term_strips_ansi_and_rewrites_on_cr() {
        let mut t = Term::default();
        let done = t.feed(b"\x1b[1;36m[2/6] \xd0\x9f\xd1\x80\x1b[0m\nprogress 10%\rprogress 50%");
        assert_eq!(done, vec!["[2/6] Пр".to_string()]);
        assert_eq!(t.partial, "progress 50%");
        t.feed(b"\r\n");
        assert_eq!(t.lines.back().map(String::as_str), Some("progress 50%"));
    }

    #[test]
    fn term_keeps_split_utf8() {
        let mut t = Term::default();
        let bytes = "ёж\n".as_bytes();
        t.feed(&bytes[..1]);
        t.feed(&bytes[1..]);
        assert_eq!(t.lines.back().map(String::as_str), Some("ёж"));
    }

    #[test]
    fn stages_and_prompts() {
        assert_eq!(parse_stage("[3/6] Загрузка"), Some((3, 6, "Загрузка".into())));
        assert_eq!(parse_stage("[x] y"), None);
        assert_eq!(prompt_kind(":: Proceed with installation? [Y/n] "), Some(PromptKind::YesNo { default_yes: true }));
        assert_eq!(prompt_kind("Продолжить без снапшота? [y/N] "), Some(PromptKind::YesNo { default_yes: false }));
        assert_eq!(prompt_kind("[sudo] password for me: "), Some(PromptKind::Secret));
        assert_eq!(prompt_kind("Enter a selection (default=all): "), Some(PromptKind::Text));
        assert_eq!(prompt_kind("downloading 45%"), None);
        assert_eq!(prompt_kind(""), None);
    }

    #[test]
    fn only_listed_commands_are_allowed() {
        let v = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        for ok in ["update", "check", "mirrors rescan", "vpn start", "vpn core update", "vpn use id:abc-1", "aur install paru-bin", "mirrors add https://m.example/$repo/os/$arch"] {
            assert!(allowed(&v(ok)), "{ok}");
        }
        for bad in ["install", "uninstall", "vpn add", "vpn rules", "merge", "gen-files /", "aur install -Syu", "vpn use a;b", "mirrors add file:///etc/shadow", "update --pause", "lang en"] {
            assert!(!allowed(&v(bad)), "{bad}");
        }
        assert!(!allowed(&[]));
    }

    #[test]
    fn only_check_and_vpn_toggle_skip_the_password() {
        let v = |s: &str| s.split(' ').map(String::from).collect::<Vec<_>>();
        assert_eq!(action_for(&v("check")), ACTION_CHECK);
        assert_eq!(action_for(&v("vpn start")), ACTION_VPN);
        assert_eq!(action_for(&v("vpn stop")), ACTION_VPN);
        for s in ["update", "clean", "vpn tun", "vpn use id:x", "mirrors apply", "aur install x", "check --no-download"] {
            assert_eq!(action_for(&v(s)), ACTION_MANAGE, "{s}");
        }
    }

    #[test]
    fn protocol_roundtrip_keeps_url_in_body() {
        let env = Envelope { lang: "en".into(), req: Request::VpnAdd { url: "https://secret.example/sub?token=1".into(), name: String::new() } };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"op\":\"vpn_add\""), "{s}");
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.req, env.req);
        let ev: Event = serde_json::from_str(r#"{"ev":"prompt","text":"ok?","kind":{"yes_no":{"default_yes":true}}}"#).unwrap();
        assert_eq!(ev, Event::Prompt { text: "ok?".into(), kind: PromptKind::YesNo { default_yes: true } });
    }

    #[test]
    fn operation_state_machine() {
        let shared: Shared = Arc::new(Mutex::new(State::default()));
        let mut st = lock(&shared);
        begin(&mut st, "check".into(), 1000).unwrap();
        assert!(begin(&mut st, "update".into(), 1000).is_err(), "вторая операция не запускается");
        drop(st);
        on_output(&shared, b"\x1b[1;36m[1/6] Mirrors\x1b[0m\nline\nContinue? [Y/n] ");
        check_prompt(&shared);
        {
            let st = lock(&shared);
            let s = op_status(&st);
            assert!(s.running && s.waiting);
            assert_eq!(s.stage, Some((1, 6, "Mirrors".into())));
        }
        on_output(&shared, b"y\nok\n");
        finish(&shared, 0);
        let st = lock(&shared);
        let s = op_status(&st);
        assert!(!s.running && !s.waiting);
        assert_eq!(s.last_exit, Some(0));
        assert_eq!(st.op.as_ref().unwrap().term.lines.iter().cloned().collect::<Vec<_>>(), vec!["[1/6] Mirrors", "line", "Continue? [Y/n] y", "ok"]);
    }

    /// Сквозной путь через сокет: запуск, поток событий, вопрос, ответ, код завершения; отказ неразрешённой команды.
    #[test]
    fn helper_socket_end_to_end() {
        let _iso = crate::common::contract_fixtures::isolation_lock();
        let dir = std::env::temp_dir().join(format!("upd-helper-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let script = dir.join("fake-upd");
        std::fs::write(&script, "#!/bin/sh\necho \"[1/2] Start $*\"\nprintf 'Go on? [Y/n] '\nread a\necho \"answer=$a\"\necho '[2/2] Done'\nexit 3\n").unwrap();
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        let sock = dir.join("helper.sock");
        unsafe {
            std::env::set_var("UPD_STATE_DIR", &dir);
            std::env::set_var("UPD_HELPER_SOCK", &sock);
            std::env::set_var("UPD_HELPER_ALLOW", "1");
            std::env::set_var("UPD_HELPER_EXE", &script);
        }
        std::thread::spawn(serve);
        for _ in 0..100 {
            if sock.exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let denied = call(&Request::Start { args: vec!["install".into()] });
        assert!(denied.is_err(), "install не из списка разрешённых");
        call(&Request::Start { args: vec!["check".into()] }).unwrap();
        let mut events = attach_events().unwrap();
        let mut seen = vec![];
        let mut code = None;
        for ev in events.by_ref() {
            match &ev {
                Event::Prompt { kind, .. } => {
                    assert_eq!(*kind, PromptKind::YesNo { default_yes: true });
                    call(&Request::Input { data: "y".into() }).unwrap();
                }
                Event::Exit { code: c } => {
                    code = Some(*c);
                    break;
                }
                _ => {}
            }
            seen.push(ev);
        }
        assert_eq!(code, Some(3));
        assert!(seen.contains(&Event::Stage { n: 2, m: 2, title: "Done".into() }), "{seen:?}");
        assert!(seen.iter().any(|e| matches!(e, Event::Line { text } if text == "answer=y")), "{seen:?}");
        let st: OpStatus = call_as(&Request::Status).unwrap();
        assert_eq!((st.running, st.last_exit, st.command.as_str()), (false, Some(3), "check"));
        unsafe {
            for k in ["UPD_STATE_DIR", "UPD_HELPER_SOCK", "UPD_HELPER_ALLOW", "UPD_HELPER_EXE"] {
                std::env::remove_var(k);
            }
        }
        let _ = std::fs::remove_dir_all(&dir);
    }
}
