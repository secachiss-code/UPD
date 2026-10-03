//! Системный помощник для графического интерфейса.
//!
//! Интерфейс работает от обычного пользователя; всё, что требует root (установка, зеркала, VPN,
//! живой снимок ядра VPN через закрытый сокет), он просит у помощника `cm helper`. Помощник
//! запускается systemd по обращению к сокету `/run/cm/helper.sock` и сам завершается без работы.
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
use std::collections::{HashMap, VecDeque};
use std::fs::File;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::Shutdown;
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::net::{UnixListener, UnixStream};
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const SOCKET: &str = "/run/cm/helper.sock";
/// Чтение: состояние операции, живой снимок VPN, списки снапшотов и истории.
pub const ACTION_STATUS: &str = "io.github.cm.status";
/// Проверка обновлений (как обновление кэша в PackageKit): без пароля в активном сеансе.
pub const ACTION_CHECK: &str = "io.github.cm.check";
/// Включить или выключить VPN и выбрать сервер (как VPN в NetworkManager): без пароля в активном сеансе.
pub const ACTION_VPN: &str = "io.github.cm.vpn";
/// Изменение системы: установка, зеркала, подписки и настройки — с паролем администратора.
pub const ACTION_MANAGE: &str = "io.github.cm.manage";
pub const PROTOCOL_VERSION: u32 = 2;

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
const MAX_LINE_BYTES: usize = 16 << 10;
const MAX_LINE_CONTENT_BYTES: usize = MAX_LINE_BYTES - 3;
const MAX_JOURNAL_BYTES: usize = 3 << 20;
const MAX_REQUEST_BYTES: usize = 64 << 10;
/// Events contain bounded terminal lines, including worst-case JSON escaping.
const MAX_EVENT_FRAME_BYTES: usize = 128 << 10;
/// mihomo accepts a /proxies response up to 16 MiB; leave room for the helper JSON envelope.
const MAX_FRAME_BYTES: usize = 20 << 20;
const MAX_REPLAY_BYTES: usize = 4 << 20;
const MAX_SUBSCRIBER_EVENTS: usize = 256;
const MAX_SUBSCRIBER_BYTES: usize = 4 << 20;
const MAX_CLIENTS_GLOBAL: usize = 16;
const MAX_CLIENTS_PER_UID: usize = 8;
const MAX_ANSWER_BYTES: usize = 1024;
const INPUT_WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const REPLY_WRITE_TIMEOUT: Duration = Duration::from_secs(2);
const REPLAY_WRITE_TIMEOUT: Duration = Duration::from_secs(3);
const READER_POLL: i32 = 100;
const RUNNER_POLL: Duration = Duration::from_millis(50);
const RUNNER_DRAIN: Duration = Duration::from_millis(500);
const RUNNER_TERM_GRACE: Duration = Duration::from_millis(500);

pub fn socket_path() -> String {
    env_or("CM_HELPER_SOCK", SOCKET)
}

// ======================= протокол =======================

#[derive(Serialize, Deserialize, Clone, PartialEq)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum Request {
    /// Проверка протокола без выполнения команды.
    Hello,
    /// состояние долгой операции
    Status,
    /// подписаться на вывод текущей (или последней) операции
    Attach,
    /// запустить команду cm из разрешённого списка
    Start { args: Vec<String> },
    /// ответ на вопрос операции (строка без перевода строки)
    Input { operation_id: OperationId, prompt_id: PromptId, data: String },
    Cancel { operation_id: OperationId },
    VpnSnapshot,
    VpnSelect { group: String, name: String },
    VpnDelay { group: String },
    /// добавить подписку: адрес только здесь, не в аргументах
    VpnAdd { url: String, name: String },
    ConfigSet { key: String, value: String },
    /// snapshots | history | restart
    Query { what: String },
}

impl std::fmt::Debug for Request {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Request::Input { operation_id, prompt_id, .. } => f.debug_struct("Input")
                .field("operation_id", operation_id).field("prompt_id", prompt_id).field("data", &"[redacted]").finish(),
            _ => write!(f, "Request({:?})", std::mem::discriminant(self)),
        }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Envelope {
    /// Legacy clients omit this field and are rejected before dispatch.
    #[serde(default)]
    pub protocol_version: u32,
    /// язык ответа и запускаемой команды (код ru, en…; пусто — как в настройках)
    #[serde(default)]
    pub lang: String,
    #[serde(flatten)]
    pub req: Request,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Reply {
    /// Missing on old helper replies; the client treats it as a protocol mismatch.
    #[serde(default)]
    pub protocol_version: u32,
    pub ok: bool,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub error: String,
    /// polkit отказал или пользователь закрыл окно пароля
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub denied: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub control_error: Option<ControlErrorCode>,
    #[serde(default, skip_serializing_if = "serde_json::Value::is_null")]
    pub data: serde_json::Value,
}

impl Reply {
    fn ok(data: serde_json::Value) -> Self {
        Reply { protocol_version: PROTOCOL_VERSION, ok: true, data, ..Default::default() }
    }
    fn err(e: impl Into<String>) -> Self {
        Reply { protocol_version: PROTOCOL_VERSION, error: e.into(), ..Default::default() }
    }
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct OperationId(pub String);

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[serde(transparent)]
pub struct PromptId(pub u64);

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq)]
pub struct OperationEvent {
    pub protocol_version: u32,
    pub operation_id: OperationId,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt_id: Option<PromptId>,
    pub event: Event,
}

#[derive(Serialize, Deserialize, Clone, Debug, PartialEq, Eq)]
pub struct StartReply {
    pub operation_id: OperationId,
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
    /// граница между replay накопленного журнала и последующими живыми событиями
    ReplayComplete,
    /// часть журнала или событий была отброшена в пределах ресурсного бюджета
    Gap,
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
    partial_truncated: bool,
    line_bytes: usize,
    pub history_truncated: bool,
}

impl Term {
    /// Добавить байты; вернуть завершённые строки.
    pub fn feed(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = Vec::new();
        for chunk in bytes.chunks(8192) { done.extend(self.feed_chunk(chunk)); }
        done
    }

    fn feed_chunk(&mut self, bytes: &[u8]) -> Vec<String> {
        let mut done = vec![];
        self.utf8.extend_from_slice(bytes);
        let mut decoded = Vec::with_capacity(self.utf8.len());
        let mut consumed = 0;
        loop {
            let rest = &self.utf8[consumed..];
            match std::str::from_utf8(rest) {
                Ok(_) => {
                    decoded.extend_from_slice(rest);
                    consumed = self.utf8.len();
                    break;
                }
                Err(error) => {
                    let valid = error.valid_up_to();
                    decoded.extend_from_slice(&rest[..valid]);
                    consumed += valid;
                    if let Some(invalid) = error.error_len() {
                        decoded.extend_from_slice("�".as_bytes());
                        consumed += invalid;
                    } else {
                        break;
                    }
                }
            }
        }
        self.utf8.drain(..consumed);
        let text = String::from_utf8(decoded).expect("decoded text must be valid UTF-8");
        for ch in text.chars() {
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
                (4, '\\' | '\x07') => self.esc = 0,
                (4, '\x1b') => self.esc = 4,
                (4, _) => self.esc = 3,
                (_, '\n') => {
                    self.pending_cr = false;
                    let line = std::mem::take(&mut self.partial);
                    let line = if self.partial_truncated {
                        self.partial_truncated = false;
                        format!("{line}…")
                    } else {
                        line
                    };
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
                        self.partial_truncated = false;
                        self.pending_cr = false;
                    }
                    if !self.partial_truncated {
                        if self.partial.len() + c.len_utf8() <= MAX_LINE_CONTENT_BYTES {
                            self.partial.push(c);
                        } else {
                            self.partial_truncated = true;
                        }
                    }
                }
            }
        }
        done
    }

    /// EOF replaces a partial code point and commits the remaining visible line.
    pub fn flush(&mut self) -> Option<String> {
        if !self.utf8.is_empty() { self.utf8.clear(); self.feed("�".as_bytes()); }
        if self.partial.is_empty() { return None; }
        let mut line = std::mem::take(&mut self.partial);
        if self.partial_truncated { line.push('…'); self.partial_truncated = false; }
        self.pending_cr = false;
        self.push(line.clone());
        Some(line)
    }

    fn push(&mut self, line: String) {
        self.line_bytes += line.len();
        self.lines.push_back(line);
        while self.lines.len() > MAX_LINES || self.line_bytes > MAX_JOURNAL_BYTES {
            if let Some(old) = self.lines.pop_front() {
                self.line_bytes -= old.len();
                self.history_truncated = true;
            }
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

/// Команды cm, которые интерфейс может запустить через помощника. Всё прочее отклоняется.
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

impl Peer {
    fn user_context(self) -> Result<Option<UserContext>, String> {
        if self.uid == 0 { Ok(None) } else { UserContext::from_uid(self.uid).map(Some) }
    }
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

/// Проверка через polkit. root разрешено всё; в тестовом режиме (CM_STATE_DIR) можно разрешить явно.
fn authorize(p: Peer, action: &str) -> Auth {
    if p.uid == 0 || (test_mode() && std::env::var("CM_HELPER_ALLOW").as_deref() == Ok("1")) {
        return Auth::Yes;
    }
    let Some(start) = start_time(p.pid) else { return Auth::No(t!("процесс запроса уже завершился").into()) };
    let subject = format!("{},{},{}", p.pid, start, p.uid);
    let mut cmd = Command::new("pkcheck");
    cmd.args(["--action-id", action, "--process", &subject, "--allow-user-interaction"]).stdin(Stdio::null());
    let mut policy = CapturePolicy::background(Some(64 << 10));
    policy.deadline = Instant::now() + Duration::from_secs(300);
    match capture_with_policy(&mut cmd, policy) {
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
    operation_id: OperationId,
    command: String,
    owner: u32,
    runner: Option<SyncSender<RunnerCommand>>,
    phase: OperationPhase,
    term: Term,
    stage: Option<(u32, u32, String)>,
    prompt: Option<(PromptId, String, PromptKind)>,
    next_prompt_id: u64,
    answered_partial: Option<String>,
    started: i64,
    exit: Option<i32>,
    finished: i64,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum OperationPhase {
    Starting,
    Running,
    Waiting,
    Draining,
    Finished,
}

enum RunnerCommand {
    Cancel(SyncSender<Result<(), String>>),
    Input { data: Vec<u8>, reply: SyncSender<Result<(), String>> },
}

enum ReaderOutcome {
    Eof,
    Error(String),
    Stopped,
}

struct RunnerResources {
    child: Option<Child>,
    pgid: Option<i32>,
    reader: Option<std::thread::JoinHandle<()>>,
    reader_done: Receiver<ReaderOutcome>,
    reader_stop: Arc<AtomicBool>,
    reader_wake: UnixStream,
    commands: Receiver<RunnerCommand>,
    input: File,
}

struct QueuedEvent {
    frame: OperationEvent,
    bytes: usize,
}

#[derive(Default)]
struct QueuedEvents {
    frames: VecDeque<QueuedEvent>,
    bytes: usize,
}

struct SubscriberQueue {
    queue: Mutex<QueuedEvents>,
    gap: AtomicBool,
    gap_sent: AtomicBool,
    gap_operation_id: Mutex<Option<OperationId>>,
    wake_write: UnixStream,
}

enum SubscriberRead {
    Event(OperationEvent),
    Gap(OperationEvent),
    Empty,
    Closed,
}

impl SubscriberQueue {
    fn new() -> std::io::Result<(Arc<Self>, UnixStream)> {
        let (wake_read, wake_write) = UnixStream::pair()?;
        wake_read.set_nonblocking(true)?;
        wake_write.set_nonblocking(true)?;
        Ok((
            Arc::new(Self {
                queue: Mutex::new(QueuedEvents::default()),
                gap: AtomicBool::new(false),
                gap_sent: AtomicBool::new(false),
                gap_operation_id: Mutex::new(None),
                wake_write,
            }),
            wake_read,
        ))
    }

    fn wake(&self) {
        let byte = 1u8;
        unsafe {
            libc::send(
                self.wake_write.as_raw_fd(),
                &byte as *const u8 as *const libc::c_void,
                1,
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            );
        }
    }

    fn close_with_gap(&self, operation_id: &OperationId) {
        if self.gap.load(Ordering::Acquire) {
            return;
        }
        *self.gap_operation_id.lock().unwrap_or_else(|e| e.into_inner()) = Some(operation_id.clone());
        self.gap.store(true, Ordering::Release);
        self.wake();
    }

    /// Queueing is non-blocking while the caller owns the global state mutex.
    fn try_push(&self, frame: OperationEvent, bytes: usize) -> bool {
        if self.gap.load(Ordering::Acquire) {
            return false;
        }
        let mut queue = match self.queue.try_lock() {
            Ok(queue) => queue,
            Err(std::sync::TryLockError::Poisoned(error)) => error.into_inner(),
            Err(std::sync::TryLockError::WouldBlock) => {
                self.close_with_gap(&frame.operation_id);
                return false;
            }
        };
        if self.gap.load(Ordering::Acquire) {
            return false;
        }
        let replace_partial = matches!(&frame.event, Event::Partial { .. })
            && queue.frames.back().is_some_and(|queued| matches!(&queued.frame.event, Event::Partial { .. }));
        if replace_partial {
            let old_bytes = queue.frames.back().map(|queued| queued.bytes).unwrap_or(0);
            if queue.bytes.saturating_sub(old_bytes).saturating_add(bytes) > MAX_SUBSCRIBER_BYTES {
                drop(queue);
                self.close_with_gap(&frame.operation_id);
                return false;
            }
            queue.bytes = queue.bytes - old_bytes + bytes;
            if let Some(last) = queue.frames.back_mut() {
                *last = QueuedEvent { frame, bytes };
            }
        } else {
            if queue.frames.len() >= MAX_SUBSCRIBER_EVENTS || queue.bytes.saturating_add(bytes) > MAX_SUBSCRIBER_BYTES {
                drop(queue);
                self.close_with_gap(&frame.operation_id);
                return false;
            }
            queue.bytes += bytes;
            queue.frames.push_back(QueuedEvent { frame, bytes });
        }
        self.wake();
        true
    }

    fn try_pop(&self) -> SubscriberRead {
        let mut queue = self.queue.lock().unwrap_or_else(|e| e.into_inner());
        if self.gap.load(Ordering::Acquire) {
            queue.frames.clear();
            queue.bytes = 0;
            if !self.gap_sent.swap(true, Ordering::AcqRel) {
                let operation_id = self.gap_operation_id.lock().unwrap_or_else(|e| e.into_inner()).clone();
                return operation_id.map_or(SubscriberRead::Closed, |operation_id| {
                    SubscriberRead::Gap(operation_event(&operation_id, None, Event::Gap))
                });
            }
            return SubscriberRead::Closed;
        }
        match queue.frames.pop_front() {
            Some(queued) => {
                queue.bytes = queue.bytes.saturating_sub(queued.bytes);
                SubscriberRead::Event(queued.frame)
            }
            None => SubscriberRead::Empty,
        }
    }
}

struct SubscriptionGuard {
    shared: Shared,
    queue: Arc<SubscriberQueue>,
}

impl Drop for SubscriptionGuard {
    fn drop(&mut self) {
        lock(&self.shared).subs.retain(|sub| !Arc::ptr_eq(sub, &self.queue));
    }
}

struct State {
    instance_nonce: String,
    next_operation_id: u64,
    op: Option<Op>,
    subs: Vec<Arc<SubscriberQueue>>,
    clients: usize,
    clients_by_uid: HashMap<u32, usize>,
    settings: Arc<Mutex<()>>,
    last_activity: Option<Instant>,
}

impl Default for State {
    fn default() -> Self {
        Self {
            instance_nonce: new_instance_nonce(),
            next_operation_id: 0,
            op: None,
            subs: Vec::new(),
            clients: 0,
            clients_by_uid: HashMap::new(),
            settings: Arc::new(Mutex::new(())),
            last_activity: None,
        }
    }
}

fn new_instance_nonce() -> String {
    static FALLBACK: AtomicU64 = AtomicU64::new(0);
    let mut bytes = [0u8; 16];
    if File::open("/dev/urandom").and_then(|mut file| file.read_exact(&mut bytes)).is_ok() {
        return bytes.iter().map(|byte| format!("{byte:02x}")).collect();
    }
    let tick = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or_default();
    format!("{}-{tick}-{}", std::process::id(), FALLBACK.fetch_add(1, Ordering::Relaxed))
}

type Shared = Arc<Mutex<State>>;

fn lock(s: &Shared) -> std::sync::MutexGuard<'_, State> {
    s.lock().unwrap_or_else(|e| e.into_inner())
}

fn reserve_connection(shared: &Shared, uid: u32) -> bool {
    let mut st = lock(shared);
    let per_uid = st.clients_by_uid.get(&uid).copied().unwrap_or(0);
    if st.clients >= MAX_CLIENTS_GLOBAL || per_uid >= MAX_CLIENTS_PER_UID {
        return false;
    }
    st.clients += 1;
    st.clients_by_uid.insert(uid, per_uid + 1);
    st.last_activity = Some(Instant::now());
    true
}

struct ClientGuard {
    shared: Shared,
    uid: u32,
}

impl Drop for ClientGuard {
    fn drop(&mut self) {
        let mut st = lock(&self.shared);
        st.clients = st.clients.saturating_sub(1);
        if let Some(count) = st.clients_by_uid.get_mut(&self.uid) {
            *count = count.saturating_sub(1);
            if *count == 0 {
                st.clients_by_uid.remove(&self.uid);
            }
        }
        st.last_activity = Some(Instant::now());
    }
}

struct ConnectionWorker {
    tid: libc::pid_t,
    handle: Option<std::thread::JoinHandle<()>>,
}

impl ConnectionWorker {
    fn retired(&mut self) -> bool {
        if self.handle.as_ref().is_some_and(|handle| handle.is_finished()) {
            let _ = self.handle.take().unwrap().join();
        }
        // musl join can return just before the kernel removes the exiting task.
        self.handle.is_none() && unsafe { libc::syscall(libc::SYS_tgkill, std::process::id(), self.tid, 0) } < 0
    }
}

fn spawn_connection_worker(f: impl FnOnce() + Send + 'static) -> Result<ConnectionWorker, String> {
    if test_mode() && std::env::var("CM_HELPER_FAIL_CONNECTION_WORKER").as_deref() == Ok("1") {
        return Err("injected helper connection worker spawn failure".into());
    }
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    let handle = spawn_thread("helper-conn", move || {
        let _ = tx.send(unsafe { libc::syscall(libc::SYS_gettid) as libc::pid_t });
        f();
    })?;
    let tid = rx.recv().map_err(|error| error.to_string())?;
    Ok(ConnectionWorker { tid, handle: Some(handle) })
}

fn broadcast(st: &mut State, operation_id: &OperationId, prompt_id: Option<PromptId>, event: Event) {
    if !st.op.as_ref().is_some_and(|op| &op.operation_id == operation_id) {
        return;
    }
    let frame = OperationEvent { protocol_version: PROTOCOL_VERSION, operation_id: operation_id.clone(), prompt_id, event };
    let bytes = serialized_frame_len(&frame).unwrap_or(MAX_SUBSCRIBER_BYTES + 1);
    st.subs.retain(|sub| sub.try_push(frame.clone(), bytes));
}

fn op_status(st: &State) -> OpStatus {
    match &st.op {
        Some(op) => OpStatus {
            running: op.phase != OperationPhase::Finished,
            command: op.command.clone(),
            stage: op.stage.clone(),
            started: op.started,
            last_exit: op.exit,
            last_command: op.command.clone(),
            finished: op.finished,
            waiting: op.phase == OperationPhase::Waiting,
            operation_id: Some(op.operation_id.clone()),
            prompt_id: op.prompt.as_ref().map(|(prompt_id, _, _)| *prompt_id),
        },
        None => OpStatus::default(),
    }
}

/// Вывод операции: строки, этапы, вопросы.
fn on_output(shared: &Shared, operation_id: &OperationId, bytes: &[u8]) {
    let mut st = lock(shared);
    let Some(op) = st.op.as_mut().filter(|op| &op.operation_id == operation_id && op.phase != OperationPhase::Finished) else { return };
    if op.phase == OperationPhase::Starting {
        op.phase = OperationPhase::Running;
    }
    let lines = op.term.feed(bytes);
    if !lines.is_empty() { op.answered_partial = None; }
    let mut evs = vec![];
    if op.prompt.is_some() && (!lines.is_empty() || op.term.partial.is_empty()) {
        let prompt_id = op.prompt.take().map(|(id, _, _)| id);
        if op.phase == OperationPhase::Waiting {
            op.phase = OperationPhase::Running;
        }
        evs.push((prompt_id, Event::Answered));
    }
    for l in lines {
        if let Some((n, m, title)) = parse_stage(&l) {
            op.stage = Some((n, m, title.clone()));
            evs.push((None, Event::Stage { n, m, title }));
        }
        evs.push((None, Event::Line { text: l }));
    }
    evs.push((None, Event::Partial { text: op.term.partial.clone() }));
    for (prompt_id, event) in evs {
        broadcast(&mut st, operation_id, prompt_id, event);
    }
}

/// Нет нового вывода, а строка не закончена и похожа на вопрос — операция ждёт ответа.
fn check_prompt(shared: &Shared, operation_id: &OperationId) {
    let mut st = lock(shared);
    let prompt = {
        let Some(op) = st.op.as_mut().filter(|op| &op.operation_id == operation_id) else { return };
        if !matches!(op.phase, OperationPhase::Running | OperationPhase::Waiting) || op.prompt.is_some() || op.answered_partial.as_ref() == Some(&op.term.partial) {
            return;
        }
        let Some(kind) = prompt_kind(&op.term.partial) else { return };
        let Some(next) = op.next_prompt_id.checked_add(1) else { return };
        op.next_prompt_id = next;
        op.phase = OperationPhase::Waiting;
        let prompt_id = PromptId(next);
        let text = op.term.partial.trim().to_string();
        op.prompt = Some((prompt_id, text.clone(), kind.clone()));
        (prompt_id, text, kind)
    };
    broadcast(&mut st, operation_id, Some(prompt.0), Event::Prompt { text: prompt.1, kind: prompt.2 });
}

fn finish(shared: &Shared, operation_id: &OperationId, code: i32) {
    let mut st = lock(shared);
    let (line, command) = {
        let Some(op) = st.op.as_mut().filter(|op| &op.operation_id == operation_id) else { return };
        if op.phase == OperationPhase::Finished {
            return;
        }
        let line = op.term.flush();
        op.exit = Some(code);
        op.finished = now();
        op.phase = OperationPhase::Finished;
        op.runner = None;
        op.prompt = None;
        (line, op.command.clone())
    };
    if let Some(line) = line {
        broadcast(&mut st, operation_id, None, Event::Line { text: line });
    }
    println!("cm helper: {command} → {code}");
    st.last_activity = Some(Instant::now());
    broadcast(&mut st, operation_id, None, Event::Exit { code });
}

#[derive(Serialize, Deserialize, Clone, Copy, Debug, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ControlErrorCode { StaleOperation, StalePrompt, InvalidAnswer, Unsupported, Busy, InputFailed, SignalFailed }

#[derive(Debug)]
struct ControlError { code: ControlErrorCode, message: String }

impl ControlError {
    fn new(code: ControlErrorCode, message: &str) -> Self { Self { code, message: message.into() } }
}

/// Validate user input without including its contents in diagnostics.
pub fn validate_answer(data: &str) -> Result<(), String> {
    if data.len() > MAX_ANSWER_BYTES || data.chars().any(char::is_control) {
        Err("answer exceeds its budget or contains control characters".into())
    } else { Ok(()) }
}

fn input_operation(st: &mut State, owner: u32, operation_id: &OperationId, prompt_id: PromptId, data: &str)
    -> Result<Receiver<Result<(), String>>, ControlError>
{
    validate_answer(data).map_err(|message| ControlError { code: ControlErrorCode::InvalidAnswer, message })?;
    let Some(op) = st.op.as_mut().filter(|op| &op.operation_id == operation_id && op.owner == owner && op.phase != OperationPhase::Finished) else {
        return Err(ControlError::new(ControlErrorCode::StaleOperation, "stale operation id or owner"));
    };
    if op.phase != OperationPhase::Waiting || op.prompt.as_ref().map(|(id, _, _)| *id) != Some(prompt_id) {
        return Err(ControlError::new(ControlErrorCode::StalePrompt, "stale or already answered prompt"));
    }
    let runner = op.runner.as_ref().ok_or_else(|| ControlError::new(ControlErrorCode::Unsupported, "operation is not accepting input"))?;
    let (reply, result) = mpsc::sync_channel(1);
    let mut bytes = data.as_bytes().to_vec();
    bytes.push(b'\n');
    runner.try_send(RunnerCommand::Input { data: bytes, reply }).map_err(|_| ControlError::new(ControlErrorCode::Busy, "operation runner queue unavailable"))?;
    op.answered_partial = Some(op.term.partial.clone());
    op.prompt = None;
    op.phase = OperationPhase::Running;
    broadcast(st, operation_id, Some(prompt_id), Event::Answered);
    Ok(result)
}

fn submit_input(shared: &Shared, owner: u32, operation_id: &OperationId, prompt_id: PromptId, data: &str) -> Result<(), ControlError> {
    let result = input_operation(&mut lock(shared), owner, operation_id, prompt_id, data)?;
    result.recv_timeout(INPUT_WRITE_TIMEOUT + Duration::from_secs(1))
        .map_err(|_| ControlError::new(ControlErrorCode::InputFailed, "operation runner did not acknowledge input"))?
        .map_err(|error| ControlError { code: ControlErrorCode::InputFailed, message: error })
}

fn cancel_operation(shared: &Shared, owner: u32, operation_id: &OperationId) -> Result<(), ControlError> {
    let runner = {
        let st = lock(shared);
        let Some(op) = st.op.as_ref().filter(|op| &op.operation_id == operation_id && op.owner == owner && op.phase != OperationPhase::Finished) else {
            return Err(ControlError::new(ControlErrorCode::StaleOperation, "stale operation id or nothing to cancel"));
        };
        match &op.runner {
            Some(runner) => runner.clone(),
            None if op.phase == OperationPhase::Starting => return Err(ControlError::new(ControlErrorCode::Busy, "operation runner is still starting")),
            None => return Err(ControlError::new(ControlErrorCode::Unsupported, "cancellation is not supported for this operation")),
        }
    };
    let (reply, result) = mpsc::sync_channel(1);
    runner.try_send(RunnerCommand::Cancel(reply)).map_err(|_| ControlError::new(ControlErrorCode::Busy, "operation runner queue unavailable"))?;
    result.recv_timeout(Duration::from_secs(2))
        .map_err(|_| ControlError::new(ControlErrorCode::Busy, "operation runner did not acknowledge cancellation"))?
        .map_err(|message| ControlError { code: ControlErrorCode::SignalFailed, message })
}

/// Запуск `cm ARGS` в отдельном PTY: pacman, apt и sudo видят настоящий терминал.
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
    // в тестовом режиме вместо cm можно подставить свою программу (сквозной тест протокола)
    let exe = match std::env::var_os("CM_HELPER_EXE").filter(|_| test_mode()) {
        Some(p) => std::path::PathBuf::from(p),
        None => std::env::current_exe()?,
    };
    let mut cmd = Command::new(exe);
    for key in ["SUDO_USER", "SUDO_UID", "DOAS_USER", "PKEXEC_UID", "XDG_RUNTIME_DIR", "DBUS_SESSION_BUS_ADDRESS"] { cmd.env_remove(key); }
    cmd.args(args).env("PAGER", "cat").env("TERM", "xterm-256color").env("CM_GUI", "1");
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
    if test_mode() && std::env::var("CM_HELPER_FAIL_CHILD").as_deref() == Ok("1") {
        return Err(std::io::Error::other("injected child spawn failure"));
    }
    let child = cmd.spawn()?;
    Ok((child, master))
}

fn spawn_operation_worker<T: Send + 'static>(name: &str, f: impl FnOnce() -> T + Send + 'static) -> Result<std::thread::JoinHandle<T>, String> {
    if test_mode() && std::env::var("CM_HELPER_FAIL_WORKER").as_deref() == Ok(name) {
        // Let the child get far enough to create a marker or fork a descendant so
        // the failure fixture verifies cleanup of a real process tree.
        std::thread::sleep(Duration::from_millis(50));
        return Err(format!("injected {name} worker spawn failure"));
    }
    spawn_thread(&format!("helper-{name}"), f)
}

fn set_operation_phase(shared: &Shared, operation_id: &OperationId, phase: OperationPhase) {
    let mut st = lock(shared);
    if let Some(op) = st.op.as_mut().filter(|op| &op.operation_id == operation_id && op.phase != OperationPhase::Finished) {
        op.phase = phase;
    }
}

fn read_operation(
    shared: &Shared,
    operation_id: &OperationId,
    mut reader: File,
    mut wake: UnixStream,
    stop: &AtomicBool,
) -> ReaderOutcome {
    let fd = reader.as_raw_fd();
    let wake_fd = wake.as_raw_fd();
    let mut buf = [0u8; 8192];
    loop {
        if stop.load(Ordering::Acquire) {
            return ReaderOutcome::Stopped;
        }
        let mut fds = [
            libc::pollfd { fd, events: libc::POLLIN | libc::POLLHUP | libc::POLLERR, revents: 0 },
            libc::pollfd { fd: wake_fd, events: libc::POLLIN, revents: 0 },
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, READER_POLL) };
        if ready < 0 {
            let error = std::io::Error::last_os_error();
            if error.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return ReaderOutcome::Error(error.to_string());
        }
        if fds[1].revents != 0 || stop.load(Ordering::Acquire) {
            let mut wake_buf = [0u8; 32];
            let _ = wake.read(&mut wake_buf);
            if stop.load(Ordering::Acquire) {
                return ReaderOutcome::Stopped;
            }
        }
        if ready == 0 {
            check_prompt(shared, operation_id);
            continue;
        }
        if fds[0].revents == 0 {
            continue;
        }
        match reader.read(&mut buf) {
            Ok(0) => return ReaderOutcome::Eof,
            Ok(n) => on_output(shared, operation_id, &buf[..n]),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => continue,
            // Linux PTY masters report EIO after the last slave descriptor closes.
            Err(error) if error.raw_os_error() == Some(libc::EIO) => return ReaderOutcome::Eof,
            Err(error) => return ReaderOutcome::Error(error.to_string()),
        }
    }
}

impl RunnerResources {
    fn wake_reader(&mut self) {
        self.reader_stop.store(true, Ordering::Release);
        let _ = self.reader_wake.write_all(&[1]);
    }

    fn join_reader(&mut self) -> Result<(), String> {
        self.wake_reader();
        match self.reader.take() {
            Some(reader) => reader.join().map_err(|_| "helper reader worker panicked".to_string()),
            None => Ok(()),
        }
    }

    fn terminate_child(&mut self) {
        if let Some(mut child) = self.child.take() {
            if let Some(pgid) = self.pgid.take() {
                unsafe { libc::killpg(pgid, libc::SIGKILL) };
            }
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}

impl Drop for RunnerResources {
    fn drop(&mut self) {
        self.terminate_child();
        let _ = self.join_reader();
    }
}

fn signal_runner_group(resources: &RunnerResources, signal: i32) -> Result<(), String> {
    let pgid = resources.pgid.ok_or_else(|| "operation process group is no longer available".to_string())?;
    if unsafe { libc::killpg(pgid, signal) } == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error().to_string())
    }
}

fn runner_loop(shared: &Shared, operation_id: &OperationId, resources: &mut RunnerResources) -> i32 {
    let mut exit_code = None;
    let mut forced_error = false;
    let mut drain_deadline = None;
    let mut terminate_deadline = None;
    let mut cancel_count = 0u32;
    let mut reader_ended = false;
    let mut commands_disconnected = false;
    let mut pending_input: Option<(Vec<u8>, usize, Instant, SyncSender<Result<(), String>>)> = None;

    loop {
        if !commands_disconnected {
            match resources.commands.recv_timeout(RUNNER_POLL) {
                Ok(RunnerCommand::Cancel(reply)) => {
                    let result = if resources.child.is_none() {
                        Err("operation child has exited; cancellation is no longer supported".into())
                    } else {
                        let signal = match cancel_count {
                            0 => libc::SIGINT,
                            1 => libc::SIGTERM,
                            _ => libc::SIGKILL,
                        };
                        match signal_runner_group(resources, signal) {
                            Ok(()) => {
                                cancel_count = cancel_count.saturating_add(1).min(3);
                                terminate_deadline = (cancel_count < 3).then(|| Instant::now() + RUNNER_TERM_GRACE);
                                Ok(())
                            }
                            Err(error) => Err(error),
                        }
                    };
                    let _ = reply.send(result);
                }
                Ok(RunnerCommand::Input { data, reply }) => {
                    if resources.child.is_none() || pending_input.is_some() {
                        let _ = reply.send(Err("operation cannot accept input now".into()));
                    } else {
                        // Disable terminal echo before delivering any user answer.
                        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
                        if unsafe { libc::tcgetattr(resources.input.as_raw_fd(), &mut attributes) } == 0 {
                            attributes.c_lflag &= !(libc::ECHO | libc::ECHONL);
                            if unsafe { libc::tcsetattr(resources.input.as_raw_fd(), libc::TCSANOW, &attributes) } != 0 {
                                let _ = reply.send(Err("cannot disable terminal echo".into()));
                                continue;
                            }
                        } else {
                            let _ = reply.send(Err("cannot inspect terminal echo".into()));
                            continue;
                        }
                        pending_input = Some((data, 0, Instant::now() + INPUT_WRITE_TIMEOUT, reply));
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {}
                Err(mpsc::RecvTimeoutError::Disconnected) => commands_disconnected = true,
            }
        }

        if let Some((data, offset, deadline, reply)) = pending_input.as_mut() {
            let result = advance_input(&mut resources.input, data, offset, *deadline, resources.child.is_some());
            if let Some(result) = result { let _ = reply.send(result); pending_input = None; }
        }

        let child_status = resources.child.as_mut().map(Child::try_wait);
        if let Some(child_status) = child_status {
            match child_status {
                Ok(Some(status)) => {
                    // try_wait reaps the leader. Clear the group id immediately so no later
                    // request can signal a recycled process group id.
                    resources.pgid = None;
                    resources.child = None;
                    exit_code = Some(process_exit_code(status));
                    drain_deadline = Some(Instant::now() + RUNNER_DRAIN);
                    set_operation_phase(shared, operation_id, OperationPhase::Draining);
                }
                Ok(None) => {}
                Err(error) => {
                    forced_error = true;
                    eprintln!("cm helper: child status failed: {error}");
                    let _ = signal_runner_group(resources, libc::SIGKILL);
                    if let Some(child) = resources.child.as_mut() {
                        let _ = child.kill();
                    }
                    if let Some(mut child) = resources.child.take() {
                        let _ = child.wait();
                    }
                    resources.pgid = None;
                    cancel_count = 3;
                    terminate_deadline = None;
                    exit_code = Some(1);
                    drain_deadline = Some(Instant::now() + RUNNER_DRAIN);
                    set_operation_phase(shared, operation_id, OperationPhase::Draining);
                }
            }
        }

        if !reader_ended {
            match resources.reader_done.try_recv() {
                Ok(ReaderOutcome::Eof | ReaderOutcome::Stopped) => reader_ended = true,
                Ok(ReaderOutcome::Error(error)) => {
                    reader_ended = true;
                    forced_error = true;
                    if resources.child.is_some() && terminate_deadline.is_none() {
                        let _ = signal_runner_group(resources, libc::SIGTERM);
                        cancel_count = 2;
                        terminate_deadline = Some(Instant::now() + RUNNER_TERM_GRACE);
                    }
                    eprintln!("cm helper: reader failed: {error}");
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    reader_ended = true;
                    forced_error = true;
                    if resources.child.is_some() && terminate_deadline.is_none() {
                        let _ = signal_runner_group(resources, libc::SIGTERM);
                        cancel_count = 2;
                        terminate_deadline = Some(Instant::now() + RUNNER_TERM_GRACE);
                    }
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }

        if terminate_deadline.is_some_and(|deadline| Instant::now() >= deadline) && resources.child.is_some() {
            if cancel_count == 1 {
                let _ = signal_runner_group(resources, libc::SIGTERM);
                cancel_count = 2;
                terminate_deadline = Some(Instant::now() + RUNNER_TERM_GRACE);
            } else {
                let _ = signal_runner_group(resources, libc::SIGKILL);
                if let Some(child) = resources.child.as_mut() {
                    let _ = child.kill();
                }
                cancel_count = 3;
                terminate_deadline = None;
            }
        }

        if let Some(code) = exit_code {
            if reader_ended || drain_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
                if resources.join_reader().is_err() {
                    forced_error = true;
                }
                return if forced_error { 1 } else { code };
            }
        }
    }
}

fn advance_input(input: &mut File, data: &[u8], offset: &mut usize, deadline: Instant, child_running: bool) -> Option<Result<(), String>> {
    if !child_running { return Some(Err("operation child exited before accepting input".into())); }
    if Instant::now() >= deadline { return Some(Err("input write deadline exceeded".into())); }
    match input.write(&data[*offset..]) {
        Ok(0) => Some(Err("input write returned zero".into())),
        Ok(n) => { *offset += n; (*offset == data.len()).then_some(Ok(())) },
        Err(error) if matches!(error.kind(), std::io::ErrorKind::WouldBlock | std::io::ErrorKind::Interrupted) => None,
        Err(error) => Some(Err(error.to_string())),
    }
}

fn process_exit_code(status: ExitStatus) -> i32 {
    status.code().unwrap_or_else(|| 128 + std::os::unix::process::ExitStatusExt::signal(&status).unwrap_or(0))
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

fn begin(st: &mut State, command: String, owner: u32) -> Result<OperationId, String> {
    if st.op.as_ref().is_some_and(|op| op.phase != OperationPhase::Finished) {
        return Err(t!("уже идёт операция: {0}", st.op.as_ref().map(|o| o.command.clone()).unwrap_or_default()));
    }
    st.next_operation_id = st.next_operation_id.checked_add(1).ok_or_else(|| "operation id counter exhausted".to_string())?;
    let operation_id = OperationId(format!("{}:{}", st.instance_nonce, st.next_operation_id));
    let started = now();
    st.op = Some(Op {
        operation_id: operation_id.clone(),
        command: command.clone(),
        owner,
        runner: None,
        phase: OperationPhase::Starting,
        term: Term::default(),
        stage: None,
        prompt: None,
        next_prompt_id: 0,
        answered_partial: None,
        started,
        exit: None,
        finished: 0,
    });
    broadcast(st, &operation_id, None, Event::Reset { command, started });
    Ok(operation_id)
}

fn start_command(shared: &Shared, args: Vec<String>, p: Peer, lang: &str) -> Result<OperationId, String> {
    if !allowed(&args) {
        return Err(t!("команда не разрешена: {0}", args.join(" ")));
    }
    let mut env = lang_env(lang);
    if let Some(user) = p.user_context()? { env.extend(user.command_env()); }
    let operation_id = begin(&mut lock(shared), args.join(" "), p.uid)?;
    let (mut child, master) = match spawn_pty(&args, &env) {
        Ok(x) => x,
        Err(e) => {
            finish(shared, &operation_id, 127);
            return Err(t!("не удалось запустить cm: {0}", e));
        }
    };
    let pgid = child.id() as i32;
    let actual_pgid = unsafe { libc::getpgid(pgid) };
    if actual_pgid != pgid {
        let error = if actual_pgid < 0 {
            std::io::Error::last_os_error()
        } else {
            std::io::Error::other(format!("child joined unexpected process group {actual_pgid}"))
        };
        let _ = child.kill();
        let _ = child.wait();
        finish(shared, &operation_id, 127);
        return Err(t!("не удалось запустить cm: {0}", error));
    }
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    if flags < 0 || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        unsafe { libc::killpg(pgid, libc::SIGKILL) };
        let _ = child.kill(); let _ = child.wait(); finish(shared, &operation_id, 127);
        return Err("cannot configure nonblocking PTY input".into());
    }
    let reader = if test_mode() && std::env::var("CM_HELPER_FAIL_CLONE").as_deref() == Ok("1") {
        std::thread::sleep(Duration::from_millis(50));
        Err(std::io::Error::other("injected PTY reader clone failure"))
    } else {
        master.try_clone()
    };
    let reader = match reader {
        Ok(reader) => reader,
        Err(error) => {
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            finish(shared, &operation_id, 127);
            return Err(t!("не удалось запустить cm: {0}", error));
        }
    };
    let (wake_read, wake_write) = match UnixStream::pair() {
        Ok(pair) => pair,
        Err(error) => {
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            finish(shared, &operation_id, 127);
            return Err(t!("не удалось запустить cm: {0}", error));
        }
    };
    let reader_stop = Arc::new(AtomicBool::new(false));
    let reader_stop_worker = reader_stop.clone();
    let (reader_done_tx, reader_done_rx) = mpsc::channel();
    let reader_shared = shared.clone();
    let reader_operation_id = operation_id.clone();
    let reader_handle = match spawn_operation_worker("reader", move || {
        let outcome = read_operation(&reader_shared, &reader_operation_id, reader, wake_read, &reader_stop_worker);
        let _ = reader_done_tx.send(outcome);
    }) {
        Ok(handle) => handle,
        Err(error) => {
            unsafe { libc::killpg(pgid, libc::SIGKILL) };
            let _ = child.kill();
            let _ = child.wait();
            finish(shared, &operation_id, 127);
            return Err(error);
        }
    };
    let (runner_tx, runner_rx) = mpsc::sync_channel(16);
    let installed = {
        let mut st = lock(shared);
        match st.op.as_mut().filter(|op| op.operation_id == operation_id) {
            Some(op) => {
                op.runner = Some(runner_tx);
                op.phase = OperationPhase::Running;
                true
            }
            None => false,
        }
    };
    if !installed {
        unsafe { libc::killpg(pgid, libc::SIGKILL) };
        let _ = child.kill();
        let _ = child.wait();
        reader_stop.store(true, Ordering::Release);
        let mut wake = wake_write;
        let _ = wake.write_all(&[1]);
        let _ = reader_handle.join();
        finish(shared, &operation_id, 127);
        return Err("operation changed while its runner was starting".into());
    }
    let resources = RunnerResources {
        child: Some(child),
        pgid: Some(pgid),
        reader: Some(reader_handle),
        reader_done: reader_done_rx,
        reader_stop,
        reader_wake: wake_write,
        commands: runner_rx,
        input: master,
    };
    let slot = Arc::new(Mutex::new(Some(resources)));
    let worker_slot = slot.clone();
    let runner_shared = shared.clone();
    let runner_operation_id = operation_id.clone();
    if let Err(error) = spawn_operation_worker("runner", move || {
        let mut resources = worker_slot.lock().unwrap_or_else(|e| e.into_inner()).take();
        let code = match resources.as_mut() {
            Some(resources) => match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| runner_loop(&runner_shared, &runner_operation_id, resources))) {
                Ok(code) => code,
                Err(_) => {
                    resources.terminate_child();
                    let _ = resources.join_reader();
                    1
                }
            },
            None => 127,
        };
        finish(&runner_shared, &runner_operation_id, code);
    }) {
        if let Some(mut resources) = slot.lock().unwrap_or_else(|e| e.into_inner()).take() {
            resources.terminate_child();
            let _ = resources.join_reader();
        }
        finish(shared, &operation_id, 127);
        return Err(error);
    }
    println!("cm helper: uid {} → cm {}", p.uid, args.join(" "));
    Ok(operation_id)
}

/// Операция внутри помощника (без отдельного процесса): добавление подписки.
fn start_inproc(shared: &Shared, command: &str, p: Peer, lang: &str, f: impl FnOnce(&dyn Fn(&str)) -> Result<(), String> + Send + 'static) -> Result<OperationId, String> {
    let operation_id = begin(&mut lock(shared), command.into(), p.uid)?;
    set_operation_phase(shared, &operation_id, OperationPhase::Running);
    let sh = shared.clone();
    let op_id = operation_id.clone();
    let lang = crate::i18n::Lang::from_code(lang);
    match spawn_operation_worker("inline", move || {
        if let Some(l) = lang {
            crate::i18n::set_thread(l);
        }
        let code = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let log = |s: &str| on_output(&sh, &op_id, format!("{s}\n").as_bytes());
            match f(&log) {
                Ok(()) => 0,
                Err(e) => {
                    log(&t!("ошибка: {0}", e));
                    1
                }
            }
        }))
        .unwrap_or(1);
        finish(&sh, &op_id, code);
    })
    {
        Ok(_) => Ok(operation_id),
        Err(error) => {
            finish(shared, &operation_id, 127);
            Err(error)
        }
    }
}

// ======================= настройки =======================

#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum SettingsRuntime { NotNeeded, Applied, Pending { reason: String }, SavedButNotApplied { reason: String, retry: String } }

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct SettingsReply { pub config: Config, pub revision: String, pub runtime: SettingsRuntime }

impl SettingsReply {
    pub fn runtime_error(&self) -> Option<String> {
        match &self.runtime {
            SettingsRuntime::SavedButNotApplied { reason, retry } => Some(format!("settings saved, but not applied: {reason}; retry: {retry}")),
            _ => None,
        }
    }
}

/// Validate and commit first; runtime failure is represented separately from persistence.
pub fn apply_setting(b: &dyn Backend, key: &str, value: &str, user: Option<&UserContext>, log: Log) -> Result<SettingsReply, String> {
    let mut pending = None;
    let mut old_autostart = false;
    let c = Config::update(b.default_mirrors(), |candidate| {
        old_autostart = candidate.vpn_autostart;
        candidate.set(key, value)?;
        if key.starts_with("vpn_") {
            if vpn::load_subs()?.list.is_empty() {
                pending = Some("VPN subscription has not been added yet".to_string());
            } else {
                vpn::build_config(candidate)?;
            }
        }
        Ok(())
    })?;
    let revision = c.revision();
    let result = match key {
        "vpn_sub_update_h" | "vpn_core_check_h" => Ok(()),
        "vpn_autostart" if c.vpn_autostart != old_autostart && !unit_state(vpn::SERVICE).is_empty() => vpn::autostart(c.vpn_autostart),
        _ if pending.is_some() => Ok(()),
        "vpn_mode" => vpn::apply_saved_mode(&c, user, log),
        k if k.starts_with("vpn_") => vpn::apply(&c, user, log),
        _ => Ok(()),
    };
    let runtime = match result {
        Err(reason) => SettingsRuntime::SavedButNotApplied { reason, retry: format!("cm vpn restart (revision {revision})") },
        Ok(()) if pending.is_some() => SettingsRuntime::Pending { reason: pending.unwrap() },
        Ok(()) if key.starts_with("vpn_") && !vpn::service_active() => SettingsRuntime::Pending { reason: "VPN is not running; the saved configuration will be used on its next start".into() },
        Ok(()) if key.starts_with("vpn_") => SettingsRuntime::Applied,
        Ok(()) => SettingsRuntime::NotNeeded,
    };
    let current = Config::load(b.default_mirrors())?;
    let current_revision = current.revision();
    let runtime = if key.starts_with("vpn_") && current_revision != revision {
        SettingsRuntime::SavedButNotApplied { reason: "configuration changed during runtime apply".into(), retry: "cm vpn restart".into() }
    } else { runtime };
    Ok(SettingsReply { config: current, revision: current_revision, runtime })
}

// ======================= обработка запросов =======================

fn poll_until(fd: i32, events: i16, deadline: Instant) -> std::io::Result<i16> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline exceeded"));
        }
        let timeout = remaining.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd { fd, events, revents: 0 };
        let ready = unsafe { libc::poll(&mut descriptor, 1, timeout) };
        if ready > 0 {
            return Ok(descriptor.revents);
        }
        if ready == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn read_request_frame(stream: &mut UnixStream) -> std::io::Result<Option<Vec<u8>>> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut frame = Vec::with_capacity(1024);
    let mut buf = [0u8; 4096];
    loop {
        let ready = poll_until(stream.as_raw_fd(), libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLRDHUP, deadline)?;
        if ready & (libc::POLLERR | libc::POLLNVAL) != 0 {
            return Err(std::io::Error::other("request socket failed"));
        }
        let n = stream.read(&mut buf)?;
        if n == 0 {
            return if frame.is_empty() {
                Ok(None)
            } else {
                Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "EOF inside helper request"))
            };
        }
        if let Some(newline) = buf[..n].iter().position(|byte| *byte == b'\n') {
            if frame.len().saturating_add(newline + 1) > MAX_REQUEST_BYTES {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "helper request exceeds byte limit"));
            }
            if buf[newline + 1..n].iter().any(|byte| !byte.is_ascii_whitespace()) {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "multiple helper requests on one connection"));
            }
            frame.extend_from_slice(&buf[..newline]);
            if frame.last() == Some(&b'\r') {
                frame.pop();
            }
            return Ok(Some(frame));
        }
        if frame.len().saturating_add(n).saturating_add(1) > MAX_REQUEST_BYTES {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "helper request exceeds byte limit"));
        }
        frame.extend_from_slice(&buf[..n]);
    }
}

fn handle(shared: &Shared, b: &dyn Backend, mut stream: UnixStream, p: Peer) {
    if stream.set_write_timeout(Some(REPLY_WRITE_TIMEOUT)).is_err() {
        return;
    }
    let line = match read_request_frame(&mut stream) {
        Ok(Some(line)) => line,
        Ok(None) => return,
        Err(error) => {
            let _ = send_server(&stream, &Reply::err(format!("helper request failed: {error}")), REPLY_WRITE_TIMEOUT);
            return;
        }
    };
    let value: serde_json::Value = match serde_json::from_slice(&line) {
        Ok(value) => value,
        Err(_e) => {
            let _ = send_server(&stream, &Reply::err("bad request"), REPLY_WRITE_TIMEOUT);
            return;
        }
    };
    let client_version = value.get("protocol_version").and_then(serde_json::Value::as_u64).and_then(|v| u32::try_from(v).ok()).unwrap_or(0);
    if client_version != PROTOCOL_VERSION {
        let reply = Reply::err(format!("helper protocol mismatch: client {client_version}, helper {PROTOCOL_VERSION}"));
        let _ = send_server(&stream, &reply, REPLY_WRITE_TIMEOUT);
        return;
    }
    let env: Envelope = match serde_json::from_value(value) {
        Ok(envelope) => envelope,
        Err(_e) => {
            let _ = send_server(&stream, &Reply::err("bad request"), REPLY_WRITE_TIMEOUT);
            return;
        }
    };
    if let Some(l) = crate::i18n::Lang::from_code(&env.lang) {
        crate::i18n::set_thread(l);
    }
    let control_owner = match &env.req {
        Request::Input { operation_id, .. } | Request::Cancel { operation_id } => {
            let owner = { let st = lock(shared); st.op.as_ref().filter(|op| &op.operation_id == operation_id && op.phase != OperationPhase::Finished).map(|op| op.owner) };
            if owner.is_none() {
                let _ = send_server(&stream, &Reply { control_error: Some(ControlErrorCode::StaleOperation), ..Reply::err("stale operation id") }, REPLY_WRITE_TIMEOUT);
                return;
            }
            owner
        }
        _ => None,
    };
    let action = match &env.req {
        Request::Hello | Request::Status | Request::Attach | Request::VpnSnapshot | Request::Query { .. } => ACTION_STATUS,
        Request::VpnSelect { .. } | Request::VpnDelay { .. } => ACTION_VPN,
        Request::Start { args } => action_for(args),
        // отвечать на вопросы и отменять может тот, кто запустил операцию, без повторного пароля
        Request::Input { .. } | Request::Cancel { .. } if control_owner == Some(p.uid) => "",
        _ => ACTION_MANAGE,
    };
    if !action.is_empty() {
        if let Auth::No(why) = authorize(p, action) {
            let _ = send_server(&stream, &Reply { denied: true, ..Reply::err(why) }, REPLY_WRITE_TIMEOUT);
            return;
        }
    }
    let reply = match env.req {
        Request::Status => Reply::ok(serde_json::to_value(op_status(&lock(shared))).unwrap_or_default()),
        Request::Attach => return attach(shared, stream),
        Request::Hello => Reply::ok(serde_json::json!({ "protocol_version": PROTOCOL_VERSION })),
        Request::Start { args } => match start_command(shared, args, p, &env.lang) {
            Ok(operation_id) => Reply::ok(serde_json::to_value(StartReply { operation_id }).unwrap_or_default()),
            Err(e) => Reply::err(e),
        },
        Request::Input { operation_id, prompt_id, data } => {
            match submit_input(shared, control_owner.unwrap_or(p.uid), &operation_id, prompt_id, &data) {
                Ok(()) => Reply::ok(serde_json::Value::Null),
                Err(e) => Reply { control_error: Some(e.code), ..Reply::err(e.message) },
            }
        }
        Request::Cancel { operation_id } => {
            match cancel_operation(shared, control_owner.unwrap_or(p.uid), &operation_id) {
                Ok(()) => Reply::ok(serde_json::Value::Null),
                Err(e) => Reply { control_error: Some(e.code), ..Reply::err(e.message) },
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
            let user = match p.user_context() { Ok(user) => user, Err(error) => { let _ = send_server(&stream, &Reply::err(error), REPLY_WRITE_TIMEOUT); return; } };
            let mirrors = b.default_mirrors();
            match start_inproc(shared, "vpn add", p, &env.lang, move |log| {
                let c = Config::load(mirrors)?;
                vpn::add_sub(&url, &name, &c, log)?;
                if vpn::service_active() {
                    vpn::apply(&c, user.as_ref(), log)?;
                } else {
                    log(t!("подписка добавлена. Запуск VPN: cm vpn start"));
                }
                Ok(())
            }) {
                Ok(operation_id) => Reply::ok(serde_json::to_value(StartReply { operation_id }).unwrap_or_default()),
                Err(e) => Reply::err(e),
            }
        }
        Request::ConfigSet { key, value } => {
            let user = match p.user_context() { Ok(user) => user, Err(error) => { let _ = send_server(&stream, &Reply::err(error), REPLY_WRITE_TIMEOUT); return; } };
            let settings = lock(shared).settings.clone();
            let _settings_guard = settings.lock().unwrap_or_else(|error| error.into_inner());
            let begun = begin(&mut lock(shared), format!("config {key}"), p.uid);
            let operation_id = match begun {
                Ok(id) => id, Err(error) => { let _ = send_server(&stream, &Reply::err(error), REPLY_WRITE_TIMEOUT); return; }
            };
            set_operation_phase(shared, &operation_id, OperationPhase::Running);
            let log = |s: &str| on_output(shared, &operation_id, format!("{s}\n").as_bytes());
            let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| apply_setting(b, &key, &value, user.as_ref(), &log)))
                .unwrap_or_else(|_| Err("settings worker panicked".into()));
            finish(shared, &operation_id, if result.as_ref().is_ok_and(|reply| reply.runtime_error().is_none()) { 0 } else { 1 });
            match result {
                Ok(result) => Reply::ok(serde_json::to_value(result).unwrap_or_default()),
                Err(error) => Reply::err(error),
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
    let _ = send_server(&stream, &reply, REPLY_WRITE_TIMEOUT);
}

fn send_limited(stream: &mut impl Write, r: &impl Serialize, limit: usize) -> std::io::Result<()> {
    let frame = encode_frame(r, limit)?;
    stream.write_all(&frame)
}

fn encode_frame(r: &impl Serialize, limit: usize) -> std::io::Result<Vec<u8>> {
    let mut frame = serde_json::to_vec(r).map_err(std::io::Error::other)?;
    frame.push(b'\n');
    if frame.len() > limit {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("helper frame exceeds {limit} byte limit")));
    }
    Ok(frame)
}

fn serialized_frame_len(value: &impl Serialize) -> std::io::Result<usize> {
    encode_frame(value, MAX_FRAME_BYTES).map(|frame| frame.len())
}

fn write_bytes_until(stream: &UnixStream, bytes: &[u8], deadline: Instant) -> std::io::Result<()> {
    let mut written = 0usize;
    while written < bytes.len() {
        let _ = poll_until(stream.as_raw_fd(), libc::POLLOUT | libc::POLLERR | libc::POLLHUP, deadline)?;
        let result = unsafe {
            libc::send(
                stream.as_raw_fd(),
                bytes[written..].as_ptr() as *const libc::c_void,
                bytes.len() - written,
                libc::MSG_DONTWAIT | libc::MSG_NOSIGNAL,
            )
        };
        if result > 0 {
            written += result as usize;
            continue;
        }
        if result == 0 {
            return Err(std::io::Error::new(std::io::ErrorKind::WriteZero, "helper socket closed during frame write"));
        }
        let error = std::io::Error::last_os_error();
        if error.kind() == std::io::ErrorKind::Interrupted || error.kind() == std::io::ErrorKind::WouldBlock {
            continue;
        }
        return Err(error);
    }
    Ok(())
}

fn send_server(stream: &UnixStream, value: &impl Serialize, timeout: Duration) -> std::io::Result<()> {
    send_server_until(stream, value, Instant::now() + timeout)
}

fn send_server_until(stream: &UnixStream, value: &impl Serialize, deadline: Instant) -> std::io::Result<()> {
    let frame = encode_frame(value, MAX_FRAME_BYTES)?;
    write_bytes_until(stream, &frame, deadline)
}

fn replay_event_bytes(frame: &OperationEvent) -> usize {
    serialized_frame_len(frame).unwrap_or(MAX_REPLAY_BYTES.saturating_add(1))
}

/// Replay ограничен общим byte budget; при усечении Gap сообщает клиенту о пропущенной истории.
fn replay_for(op: &Op) -> Vec<OperationEvent> {
    let id = &op.operation_id;
    let reset = operation_event(id, None, Event::Reset { command: op.command.clone(), started: op.started });
    let gap = operation_event(id, None, Event::Gap);
    let complete = operation_event(id, None, Event::ReplayComplete);
    let mut tail = Vec::new();
    if let Some((n, m, title)) = &op.stage {
        tail.push(operation_event(id, None, Event::Stage { n: *n, m: *m, title: title.clone() }));
    }
    tail.push(operation_event(id, None, Event::Partial { text: op.term.partial.clone() }));
    if let Some((prompt_id, text, kind)) = &op.prompt {
        tail.push(operation_event(id, Some(*prompt_id), Event::Prompt { text: text.clone(), kind: kind.clone() }));
    }
    if let Some(code) = op.exit {
        tail.push(operation_event(id, None, Event::Exit { code }));
    }

    let mut used = replay_event_bytes(&reset) + replay_event_bytes(&gap) + replay_event_bytes(&complete);
    used = used.saturating_add(tail.iter().map(replay_event_bytes).sum::<usize>());
    let mut lines = Vec::new();
    let mut truncated = op.term.history_truncated;
    for line in op.term.lines.iter().rev() {
        let event = operation_event(id, None, Event::Line { text: line.clone() });
        let bytes = replay_event_bytes(&event);
        if used.saturating_add(bytes) > MAX_REPLAY_BYTES {
            truncated = true;
            break;
        }
        used += bytes;
        lines.push(event);
    }
    lines.reverse();

    let mut replay = Vec::with_capacity(lines.len() + tail.len() + 3);
    replay.push(reset);
    if truncated {
        replay.push(gap);
    }
    replay.extend(lines);
    replay.extend(tail);
    replay.push(complete);
    replay
}

fn wait_for_subscription(stream: &UnixStream, wake: &mut UnixStream) -> std::io::Result<bool> {
    let mut descriptors = [
        libc::pollfd { fd: stream.as_raw_fd(), events: libc::POLLIN | libc::POLLHUP | libc::POLLERR | libc::POLLRDHUP, revents: 0 },
        libc::pollfd { fd: wake.as_raw_fd(), events: libc::POLLIN | libc::POLLHUP | libc::POLLERR, revents: 0 },
    ];
    loop {
        let ready = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as libc::nfds_t, -1) };
        if ready > 0 {
            break;
        }
        if ready == 0 {
            continue;
        }
        let error = std::io::Error::last_os_error();
        if error.kind() != std::io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
    let peer = descriptors[0].revents;
    if peer & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL | libc::POLLRDHUP) != 0 {
        return Ok(false);
    }
    if peer & libc::POLLIN != 0 {
        let mut byte = 0u8;
        let read = unsafe { libc::recv(stream.as_raw_fd(), &mut byte as *mut u8 as *mut libc::c_void, 1, libc::MSG_PEEK | libc::MSG_DONTWAIT) };
        if read >= 0 || std::io::Error::last_os_error().kind() != std::io::ErrorKind::WouldBlock {
            return Ok(false);
        }
    }
    if descriptors[1].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
        return Err(std::io::Error::other("subscription wake socket failed"));
    }
    if descriptors[1].revents & libc::POLLIN != 0 {
        let mut buf = [0u8; 128];
        loop {
            match wake.read(&mut buf) {
                Ok(0) => break,
                Ok(_) => continue,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => break,
                Err(error) => return Err(error),
            }
        }
    }
    Ok(true)
}

/// Поток событий операции: ограниченный replay, затем bounded live queue.
fn attach(shared: &Shared, stream: UnixStream) {
    let _ = stream.set_write_timeout(Some(REPLY_WRITE_TIMEOUT));
    let (subscriber, mut wake_read) = match SubscriberQueue::new() {
        Ok(pair) => pair,
        Err(error) => {
            let _ = send_server(&stream, &Reply::err(format!("subscription setup failed: {error}")), REPLY_WRITE_TIMEOUT);
            return;
        }
    };
    let _registration = SubscriptionGuard { shared: shared.clone(), queue: subscriber.clone() };
    let replay = {
        let mut st = lock(shared);
        let replay = st.op.as_ref().map(replay_for).unwrap_or_default();
        st.subs.push(subscriber.clone());
        replay
    };
    if send_server(&stream, &Reply::ok(serde_json::Value::Null), REPLY_WRITE_TIMEOUT).is_err() {
        return;
    }
    let replay_deadline = Instant::now() + REPLAY_WRITE_TIMEOUT;
    for frame in replay {
        if subscriber.gap.load(Ordering::Acquire) {
            if let SubscriberRead::Gap(gap) = { let _state = lock(shared); subscriber.try_pop() } {
                let _ = send_server(&stream, &gap, REPLY_WRITE_TIMEOUT);
            }
            return;
        }
        if send_server_until(&stream, &frame, replay_deadline).is_err() {
            return;
        }
    }
    loop {
        // Serialize only queue removal with broadcasts; socket writes happen after both locks are released.
        let next = { let _state = lock(shared); subscriber.try_pop() };
        match next {
            SubscriberRead::Event(frame) => {
                if send_server(&stream, &frame, REPLY_WRITE_TIMEOUT).is_err() {
                    return;
                }
            }
            SubscriberRead::Gap(frame) => {
                let _ = send_server(&stream, &frame, REPLY_WRITE_TIMEOUT);
                return;
            }
            SubscriberRead::Closed => return,
            SubscriberRead::Empty => match wait_for_subscription(&stream, &mut wake_read) {
                Ok(true) => {}
                Ok(false) | Err(_) => return,
            },
        }
    }
}

fn operation_event(operation_id: &OperationId, prompt_id: Option<PromptId>, event: Event) -> OperationEvent {
    OperationEvent { protocol_version: PROTOCOL_VERSION, operation_id: operation_id.clone(), prompt_id, event }
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

/// `cm helper`: служба по сокету; завершается после IDLE_EXIT без клиентов и операций.
pub fn serve() -> i32 {
    if !is_root() && !test_mode() {
        eprintln!("{}", t!("cm helper: нужны права root"));
        return 1;
    }
    let l = match listener() {
        Ok(l) => l,
        Err(e) => {
            eprintln!("cm helper: {e}");
            return 1;
        }
    };
    if let Err(e) = backend::detect() {
        eprintln!("cm helper: {e}");
        return 1;
    }
    let shared: Shared = Arc::new(Mutex::new(State { last_activity: Some(Instant::now()), ..Default::default() }));
    let fd = l.as_raw_fd();
    // Keep permits until pthread join, not merely until the worker closure drops its guard.
    let mut workers: Vec<(u32, ConnectionWorker)> = Vec::new();
    loop {
    let mut pfd = libc::pollfd { fd, events: libc::POLLIN, revents: 0 };
        let r = unsafe { libc::poll(&mut pfd, 1, 1000) };
        if r <= 0 {
            let st = lock(&shared);
            let busy = st.clients > 0 || st.op.as_ref().is_some_and(|o| o.phase != OperationPhase::Finished);
            let idle = if test_mode() { std::env::var("CM_HELPER_IDLE_MS").ok().and_then(|v| v.parse::<u64>().ok()).map(Duration::from_millis).unwrap_or(IDLE_EXIT) } else { IDLE_EXIT };
            if !busy && st.last_activity.is_some_and(|t| t.elapsed() > idle) {
                return 0;
            }
            continue;
        }
        let Ok((stream, _)) = l.accept() else { continue };
        let Some(peer) = peer_of(&stream) else { continue };
        let mut i = 0;
        while i < workers.len() {
            if workers[i].1.retired() {
                workers.swap_remove(i);
            } else {
                i += 1;
            }
        }
        if workers.len() >= MAX_CLIENTS_GLOBAL
            || workers.iter().filter(|(uid, _)| *uid == peer.uid).count() >= MAX_CLIENTS_PER_UID
            || !reserve_connection(&shared, peer.uid) {
            let _ = stream.set_write_timeout(Some(REPLY_WRITE_TIMEOUT));
            let _ = send_server(&stream, &Reply::err("helper connection limit reached"), REPLY_WRITE_TIMEOUT);
            continue;
        }
        let guard = ClientGuard { shared: shared.clone(), uid: peer.uid };
        let sh = shared.clone();
        let spawn = spawn_connection_worker(move || {
            let _guard = guard;
            // Backend не разделяется между потоками; определение дешёвое (PATH и os-release)
            if let Ok(b) = backend::detect() {
                handle(&sh, b.as_ref(), stream, peer);
            }
        });
        match spawn {
            Ok(worker) => workers.push((peer.uid, worker)),
            Err(error) => eprintln!("cm helper: {error}"),
        }
    }
}

// ======================= клиент =======================

pub fn available() -> bool {
    std::path::Path::new(&socket_path()).exists()
}

/// Installer-only read: legacy helpers support Status but not the Hello preflight.
/// No mutation is permitted through this compatibility path.
pub fn running_for_install() -> Result<bool, String> {
    let (_, reply) = connect_once(&Request::Status)?;
    if !reply.ok { return Err(reply.error); }
    reply.data.get("running").and_then(serde_json::Value::as_bool)
        .ok_or_else(|| "helper status is missing its running flag".into())
}

fn check_reply_protocol(reply: &Reply) -> Result<(), String> {
    if reply.protocol_version == PROTOCOL_VERSION {
        Ok(())
    } else {
        Err(format!(
            "helper protocol mismatch: client {PROTOCOL_VERSION}, helper {}; update cm and cm-cosmic together",
            reply.protocol_version
        ))
    }
}

fn connect(req: &Request) -> Result<(BufReader<UnixStream>, Reply), String> {
    if !matches!(req, Request::Hello) {
        let (_reader, hello) = connect_once(&Request::Hello)?;
        check_reply_protocol(&hello)?;
        if !hello.ok {
            return Err(hello.error);
        }
    }
    let (reader, reply) = connect_once(req)?;
    check_reply_protocol(&reply)?;
    Ok((reader, reply))
}

fn connect_once(req: &Request) -> Result<(BufReader<UnixStream>, Reply), String> {
    connect_once_observed(req, &mut |_| {})
}

fn connect_socket(path: &str, deadline: Instant) -> std::io::Result<UnixStream> {
    use std::os::fd::OwnedFd;
    let bytes = path.as_bytes();
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid helper socket path"));
    }
    address.sun_family = libc::AF_UNIX as _;
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) { *target = *byte as _; }
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if raw < 0 { return Err(std::io::Error::last_os_error()); }
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    loop {
        if Instant::now() >= deadline { return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "helper connect deadline exceeded")); }
        let rc = unsafe { libc::connect(fd.as_raw_fd(), &address as *const _ as *const libc::sockaddr, std::mem::size_of_val(&address) as _) };
        if rc == 0 { break; }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINPROGRESS) => {
                poll_until(fd.as_raw_fd(), libc::POLLOUT | libc::POLLERR | libc::POLLHUP, deadline)?;
                let mut error = 0i32;
                let mut len = std::mem::size_of_val(&error) as libc::socklen_t;
                if unsafe { libc::getsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ERROR, &mut error as *mut _ as _, &mut len) } < 0 { return Err(std::io::Error::last_os_error()); }
                if error != 0 { return Err(std::io::Error::from_raw_os_error(error)); }
                break;
            }
            Some(libc::EAGAIN) | Some(libc::EINTR) => { unsafe { libc::poll(std::ptr::null_mut(), 0, 10); } }
            _ => return Err(error),
        }
    }
    let stream = UnixStream::from(fd);
    stream.set_nonblocking(false)?;
    Ok(stream)
}

fn connect_once_observed(req: &Request, observe: &mut impl FnMut(EventCancelHandle)) -> Result<(BufReader<UnixStream>, Reply), String> {
    let path = socket_path();
    let mut s = connect_socket(&path, Instant::now() + Duration::from_secs(2)).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound || e.kind() == std::io::ErrorKind::ConnectionRefused {
            t!("помощник cm недоступен — установите cm заново (sudo cm install)").into()
        } else {
            format!("{path}: {e}")
        }
    })?;
    observe(EventCancelHandle(s.try_clone().map_err(|e| e.to_string())?));
    let env = Envelope { protocol_version: PROTOCOL_VERSION, lang: crate::i18n::cur().code().into(), req: req.clone() };
    s.set_write_timeout(Some(REQUEST_TIMEOUT)).map_err(|e| format!("helper request timeout setup failed: {e}"))?;
    send_limited(&mut s, &env, MAX_REQUEST_BYTES).map_err(|e| format!("helper request write failed: {e}"))?;
    // Read-only polling must not inherit the much longer interactive polkit wait.
    let timeout = if matches!(req, Request::Hello | Request::Status | Request::VpnSnapshot | Request::Query { .. } | Request::Attach) {
        Duration::from_secs(5)
    } else { Duration::from_secs(300) };
    let deadline = Instant::now() + timeout;
    let mut reader = BufReader::new(s);
    let frame = read_frame_before(&mut reader, MAX_FRAME_BYTES, |r| {
        let remaining = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "helper reply deadline exceeded"))?;
        r.get_ref().set_read_timeout(Some(remaining))
    })
        .map_err(|e| format!("helper reply read failed: {e}"))?
        .ok_or_else(|| "helper closed connection before replying".to_string())?;
    let reply: Reply = serde_json::from_slice(&frame).map_err(|e| format!("helper sent an invalid reply frame: {e}"))?;
    Ok((reader, reply))
}

/// Запрос с одним ответом. Ошибка содержит понятную причину (в том числе отказ polkit).
pub fn call(req: &Request) -> Result<serde_json::Value, String> {
    let (_reader, reply) = connect(req)?;
    if reply.ok {
        Ok(reply.data)
    } else {
        Err(reply.error)
    }
}

pub fn call_as<T: for<'de> Deserialize<'de>>(req: &Request) -> Result<T, String> {
    serde_json::from_value(call(req)?).map_err(|e| e.to_string())
}

fn read_frame(reader: &mut impl BufRead, limit: usize) -> std::io::Result<Option<Vec<u8>>> {
    read_frame_before(reader, limit, |_| Ok(()))
}

fn read_frame_before<R: BufRead>(reader: &mut R, limit: usize, mut before: impl FnMut(&mut R) -> std::io::Result<()>) -> std::io::Result<Option<Vec<u8>>> {
    let mut frame = Vec::new();
    loop {
        let (read, complete) = {
            before(reader)?;
            let available = reader.fill_buf()?;
            if available.is_empty() {
                if frame.is_empty() {
                    return Ok(None);
                }
                return Err(std::io::Error::new(std::io::ErrorKind::UnexpectedEof, "EOF inside helper frame"));
            }
            let read = available.iter().position(|byte| *byte == b'\n').map_or(available.len(), |i| i + 1);
            if frame.len().saturating_add(read) > limit {
                return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, format!("helper frame exceeds {limit} bytes")));
            }
            frame.extend_from_slice(&available[..read]);
            (read, frame.last() == Some(&b'\n'))
        };
        reader.consume(read);
        if complete {
            frame.pop();
            return Ok(Some(frame));
        }
    }
}

fn decode_event_frame(frame: &[u8]) -> Result<OperationEvent, String> {
    let event: OperationEvent = serde_json::from_slice(frame).map_err(|error| format!("helper sent an invalid event frame: {error}"))?;
    if event.protocol_version != PROTOCOL_VERSION {
        return Err(format!("helper event protocol mismatch: client {PROTOCOL_VERSION}, helper {}", event.protocol_version));
    }
    if event.operation_id.0.is_empty() {
        return Err("helper event is missing its operation id".into());
    }
    let needs_prompt_id = matches!(&event.event, Event::Prompt { .. } | Event::Answered);
    if needs_prompt_id != event.prompt_id.is_some() || event.prompt_id.is_some_and(|id| id.0 == 0) {
        return Err("helper event has an invalid prompt id".into());
    }
    Ok(event)
}

/// Подписка на события операции; ошибка одного кадра возвращается вызывающей стороне и завершает поток.
pub struct EventStream {
    reader: BufReader<UnixStream>,
    done: bool,
}

pub struct EventCancelHandle(UnixStream);

impl EventCancelHandle {
    pub fn cancel(&self) {
        let _ = self.0.shutdown(Shutdown::Both);
    }
}

impl EventStream {
    pub fn cancel_handle(&self) -> std::io::Result<EventCancelHandle> {
        self.reader.get_ref().try_clone().map(EventCancelHandle)
    }
}

impl Drop for EventStream {
    fn drop(&mut self) {
        let _ = self.reader.get_ref().shutdown(Shutdown::Both);
    }
}

impl Iterator for EventStream {
    type Item = Result<OperationEvent, String>;

    fn next(&mut self) -> Option<Self::Item> {
        if self.done {
            return None;
        }
        let frame = match read_frame(&mut self.reader, MAX_EVENT_FRAME_BYTES) {
            Ok(Some(frame)) => frame,
            Ok(None) => {
                self.done = true;
                return None;
            }
            Err(error) => {
                self.done = true;
                return Some(Err(format!("helper event read failed: {error}")));
            }
        };
        match decode_event_frame(&frame) {
            Ok(event) => Some(Ok(event)),
            Err(error) => {
                self.done = true;
                Some(Err(error))
            }
        }
    }
}

/// Подписка на события операции; iterator сохраняет bytes, прочитанные вместе с reply.
pub fn attach_events() -> Result<EventStream, String> {
    attach_events_observed(|_| {})
}

/// Installs cancellation before each handshake read, including the protocol preflight.
pub fn attach_events_observed(mut observe: impl FnMut(EventCancelHandle)) -> Result<EventStream, String> {
    let (_, hello) = connect_once_observed(&Request::Hello, &mut observe)?;
    check_reply_protocol(&hello)?;
    if !hello.ok { return Err(hello.error); }
    let (reader, reply) = connect_once_observed(&Request::Attach, &mut observe)?;
    check_reply_protocol(&reply)?;
    if !reply.ok {
        return Err(reply.error);
    }
    reader.get_ref().set_read_timeout(None).map_err(|e| format!("helper event timeout setup failed: {e}"))?;
    Ok(EventStream { reader, done: false })
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
        let env = Envelope {
            protocol_version: PROTOCOL_VERSION,
            lang: "en".into(),
            req: Request::VpnAdd { url: "https://secret.example/sub?token=1".into(), name: String::new() },
        };
        let s = serde_json::to_string(&env).unwrap();
        assert!(s.contains("\"op\":\"vpn_add\""), "{s}");
        assert!(s.contains(&format!("\"protocol_version\":{PROTOCOL_VERSION}")), "{s}");
        let back: Envelope = serde_json::from_str(&s).unwrap();
        assert_eq!(back.req, env.req);
        let legacy: Envelope = serde_json::from_str(r#"{"lang":"en","op":"start","args":["check"]}"#).unwrap();
        assert_eq!(legacy.protocol_version, 0, "legacy clients are distinguishable and rejected");
        let ev: Event = serde_json::from_str(r#"{"ev":"prompt","text":"ok?","kind":{"yes_no":{"default_yes":true}}}"#).unwrap();
        assert_eq!(ev, Event::Prompt { text: "ok?".into(), kind: PromptKind::YesNo { default_yes: true } });
    }

    #[test]
    fn operation_state_machine() {
        let shared: Shared = Arc::new(Mutex::new(State::default()));
        let mut st = lock(&shared);
        let operation_id = begin(&mut st, "check".into(), 1000).unwrap();
        assert!(begin(&mut st, "update".into(), 1000).is_err(), "вторая операция не запускается");
        drop(st);
        on_output(&shared, &operation_id, b"\x1b[1;36m[1/6] Mirrors\x1b[0m\nline\nContinue? [Y/n] ");
        check_prompt(&shared, &operation_id);
        {
            let st = lock(&shared);
            let s = op_status(&st);
            assert!(s.running && s.waiting);
            assert_eq!(s.stage, Some((1, 6, "Mirrors".into())));
            assert_eq!(s.operation_id.as_ref(), Some(&operation_id));
            assert_eq!(s.prompt_id, Some(PromptId(1)));
        }
        on_output(&shared, &operation_id, b"y\nok\n");
        finish(&shared, &operation_id, 0);
        let st = lock(&shared);
        let s = op_status(&st);
        assert!(!s.running && !s.waiting);
        assert_eq!(s.last_exit, Some(0));
        assert_eq!(st.op.as_ref().unwrap().term.lines.iter().cloned().collect::<Vec<_>>(), vec!["[1/6] Mirrors", "line", "Continue? [Y/n] y", "ok"]);
    }

    #[test]
    fn operation_ids_are_unique_within_and_across_helper_instances() {
        let mut first = State::default();
        let id_a = begin(&mut first, "check".into(), 1000).unwrap();
        first.op.as_mut().unwrap().phase = OperationPhase::Finished;
        first.op.as_mut().unwrap().exit = Some(0);
        let id_b = begin(&mut first, "check".into(), 1000).unwrap();
        let mut restarted = State::default();
        let id_c = begin(&mut restarted, "check".into(), 1000).unwrap();
        assert_ne!(id_a, id_b, "counter distinguishes starts within the same second");
        assert_ne!(id_a, id_c, "helper instance nonce distinguishes a restart");
        assert_ne!(id_b, id_c);
    }

    #[test]
    fn stale_prompt_from_previous_operation_is_rejected_once() {
        let dir = crate::common::contract_fixtures::TempDirGuard::new("cm-helper-prompt-id").unwrap();
        let old_path = dir.path().join("old-input");
        let new_path = dir.path().join("new-input");
        let mut st = State::default();
        let old_id = begin(&mut st, "check".into(), 1000).unwrap();
        let _old_file = File::create(&old_path).unwrap();
        let old_prompt = PromptId(1);
        {
            let op = st.op.as_mut().unwrap();
            op.prompt = Some((old_prompt, "old question?".into(), PromptKind::Text));
            op.phase = OperationPhase::Finished;
            op.exit = Some(0);
        }
        let new_id = begin(&mut st, "check".into(), 1000).unwrap();
        let new_prompt = PromptId(1);
        let mut new_file = File::create(&new_path).unwrap();
        let (runner, commands) = mpsc::sync_channel(1);
        {
            let op = st.op.as_mut().unwrap();
            op.runner = Some(runner);
            op.prompt = Some((new_prompt, "new question?".into(), PromptKind::Text));
            op.phase = OperationPhase::Waiting;
        }

        assert!(input_operation(&mut st, 1000, &old_id, old_prompt, "stale").is_err());
        assert_eq!(std::fs::read(&new_path).unwrap(), b"");
        let result = input_operation(&mut st, 1000, &new_id, new_prompt, "current").unwrap();
        if let RunnerCommand::Input { data, reply } = commands.recv().unwrap() {
            new_file.write_all(&data).unwrap(); reply.send(Ok(())).unwrap();
        }
        assert!(result.recv().unwrap().is_ok());
        assert!(input_operation(&mut st, 1000, &new_id, new_prompt, "replay").is_err());
        drop(st);
        assert_eq!(std::fs::read(&new_path).unwrap(), b"current\n");
    }

    #[test]
    fn failed_prompt_write_cannot_be_replayed() {
        let mut st = State::default();
        let operation_id = begin(&mut st, "check".into(), 1000).unwrap();
        let prompt_id = PromptId(1);
        let (runner, commands) = mpsc::sync_channel(1);
        {
            let op = st.op.as_mut().unwrap();
            op.runner = Some(runner);
            op.prompt = Some((prompt_id, "question?".into(), PromptKind::Text));
            op.phase = OperationPhase::Waiting;
        }
        let result = input_operation(&mut st, 1000, &operation_id, prompt_id, "answer").unwrap();
        if let RunnerCommand::Input { reply, .. } = commands.recv().unwrap() { reply.send(Err("fixture write error".into())).unwrap(); }
        assert!(result.recv().unwrap().is_err());
        assert!(st.op.as_ref().unwrap().prompt.is_none());
        assert_eq!(input_operation(&mut st, 1000, &operation_id, prompt_id, "replay").unwrap_err().code, ControlErrorCode::StalePrompt);
    }

    #[test]
    fn callbacks_from_old_operation_cannot_change_current_journal() {
        let shared: Shared = Arc::new(Mutex::new(State::default()));
        let old_id = begin(&mut lock(&shared), "check".into(), 1000).unwrap();
        lock(&shared).op.as_mut().unwrap().phase = OperationPhase::Finished;
        lock(&shared).op.as_mut().unwrap().exit = Some(0);
        let current_id = begin(&mut lock(&shared), "update".into(), 1000).unwrap();
        on_output(&shared, &old_id, b"stale output\n");
        check_prompt(&shared, &old_id);
        finish(&shared, &old_id, 9);
        let st = lock(&shared);
        assert_eq!(op_status(&st).operation_id, Some(current_id));
        assert_eq!(st.op.as_ref().unwrap().term.lines.len(), 0);
        assert!(op_status(&st).running);
    }

    #[test]
    fn finish_emits_one_terminal_event() {
        let shared: Shared = Arc::new(Mutex::new(State::default()));
        let (subscriber, _wake) = SubscriberQueue::new().unwrap();
        lock(&shared).subs.push(subscriber.clone());
        let operation_id = begin(&mut lock(&shared), "check".into(), 1000).unwrap();
        finish(&shared, &operation_id, 127);
        finish(&shared, &operation_id, 0);
        let mut exits = 0;
        while let SubscriberRead::Event(frame) = subscriber.try_pop() {
            exits += usize::from(matches!(frame.event, Event::Exit { .. }));
        }
        assert_eq!(exits, 1);
        assert_eq!(op_status(&lock(&shared)).last_exit, Some(127));
    }

    #[test]
    fn inline_operation_reports_cancellation_as_unsupported() {
        let _isolation = crate::common::contract_fixtures::isolation_lock();
        let mut env = crate::common::contract_fixtures::EnvGuard::new();
        env.remove("CM_HELPER_FAIL_WORKER");
        let shared: Shared = Arc::new(Mutex::new(State::default()));
        let (started_tx, started_rx) = mpsc::sync_channel(1);
        let (release_tx, release_rx) = mpsc::sync_channel(1);
        let owner = 1000;
        let operation_id = start_inproc(&shared, "inline fixture", Peer { pid: 1, uid: owner }, "en", move |_| {
            started_tx.send(()).map_err(|e| e.to_string())?;
            release_rx.recv_timeout(Duration::from_secs(2)).map_err(|e| e.to_string())?;
            Ok(())
        })
        .unwrap();
        started_rx.recv_timeout(Duration::from_secs(1)).expect("inline operation started");
        let error = cancel_operation(&shared, owner, &operation_id).unwrap_err();
        assert_eq!(error.code, ControlErrorCode::Unsupported);
        assert!(error.message.contains("cancellation is not supported"), "{error:?}");
        release_tx.send(()).unwrap();
        let deadline = Instant::now() + Duration::from_secs(1);
        while op_status(&lock(&shared)).running && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(!op_status(&lock(&shared)).running, "inline operation did not finish after release");
    }

}

#[cfg(test)]
mod frame_tests {
    use super::*;
    use std::io::Cursor;

    #[test]
    fn helper_frame_accepts_one_byte_fragments() {
        let bytes = b"{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":{\"ev\":\"exit\",\"code\":0}}\n";
        let mut reader = BufReader::with_capacity(1, Cursor::new(bytes));
        let frame = read_frame(&mut reader, bytes.len()).unwrap().unwrap();
        assert_eq!(decode_event_frame(&frame).unwrap().event, Event::Exit { code: 0 });
    }

    #[test]
    fn helper_frame_limit_accepts_exact_and_rejects_one_over() {
        let mut exact = Cursor::new(b"123\n");
        assert_eq!(read_frame(&mut exact, 4).unwrap(), Some(b"123".to_vec()));

        let mut over = Cursor::new(b"1234\n");
        let error = read_frame(&mut over, 4).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidData);
    }

    #[test]
    fn helper_frame_reports_eof_inside_json() {
        let mut reader = Cursor::new(b"{\"ok\":true");
        let error = read_frame(&mut reader, 32).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::UnexpectedEof);
    }

    #[test]
    fn malformed_event_frame_is_an_error() {
        let error = decode_event_frame(b"{\"protocol_version\":2,\"operation_id\":\"fixture-op\",\"event\":").unwrap_err();
        assert!(error.contains("invalid event frame"), "{error}");
    }
    #[test]
    fn subscriber_coalesces_partial_and_reports_count_and_byte_overflow() {
        let id = OperationId("budget-test".into());
        let (queue, _wake) = SubscriberQueue::new().unwrap();
        for n in 0..1000 {
            assert!(queue.try_push(operation_event(&id, None, Event::Partial { text: n.to_string() }), 100));
        }
        assert_eq!(queue.queue.lock().unwrap().frames.len(), 1);
        assert!(matches!(queue.try_pop(), SubscriberRead::Event(OperationEvent { event: Event::Partial { text }, .. }) if text == "999"));
        for _ in 0..MAX_SUBSCRIBER_EVENTS {
            assert!(queue.try_push(operation_event(&id, None, Event::Line { text: "x".into() }), 100));
        }
        assert!(!queue.try_push(operation_event(&id, None, Event::Exit { code: 0 }), 100));
        assert!(matches!(queue.try_pop(), SubscriberRead::Gap(_)));
        assert!(matches!(queue.try_pop(), SubscriberRead::Closed));
        assert_eq!(queue.queue.lock().unwrap().bytes, 0);
        let (queue, _wake) = SubscriberQueue::new().unwrap();
        assert!(!queue.try_push(operation_event(&id, None, Event::Reset { command: "check".into(), started: 0 }), MAX_SUBSCRIBER_BYTES + 1));
        assert!(matches!(queue.try_pop(), SubscriberRead::Gap(_)));
    }

    #[test]
    fn journal_and_escaped_replay_have_byte_budgets() {
        let mut state = State::default();
        begin(&mut state, "check".into(), 1000).unwrap();
        let op = state.op.as_mut().unwrap();
        for _ in 0..MAX_LINES { op.term.push("\u{0001}".repeat(MAX_LINE_CONTENT_BYTES)); }
        assert!(op.term.line_bytes <= MAX_JOURNAL_BYTES);
        assert!(op.term.history_truncated);
        let replay = replay_for(op);
        assert!(replay.iter().any(|event| matches!(event.event, Event::Gap)));
        assert!(matches!(replay.last().unwrap().event, Event::ReplayComplete));
        assert!(replay.iter().map(|frame| serialized_frame_len(frame).unwrap()).sum::<usize>() <= MAX_REPLAY_BYTES);
    }

    #[test]
    fn connection_guards_release_quotas_on_unwind() {
        let shared = Arc::new(Mutex::new(State::default()));
        let mut guards = Vec::new();
        for uid in [1000, 1001] {
            for _ in 0..MAX_CLIENTS_PER_UID {
                assert!(reserve_connection(&shared, uid));
                guards.push(ClientGuard { shared: shared.clone(), uid });
            }
            assert!(!reserve_connection(&shared, uid));
        }
        assert!(!reserve_connection(&shared, 1002));
        drop(guards);
        assert_eq!(lock(&shared).clients, 0);
        assert!(reserve_connection(&shared, 1000));
        let guard = ClientGuard { shared: shared.clone(), uid: 1000 };
        assert!(std::panic::catch_unwind(move || { let _guard = guard; panic!("fixture"); }).is_err());
        assert_eq!(lock(&shared).clients, 0);
        assert!(lock(&shared).clients_by_uid.is_empty());
    }

    #[test]
    fn cancellation_wakes_blocked_event_reader() {
        let (reader, _peer) = UnixStream::pair().unwrap();
        let mut stream = EventStream { reader: BufReader::new(reader), done: false };
        let cancel = stream.cancel_handle().unwrap();
        let (tx, rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || { let _ = tx.send(stream.next()); });
        cancel.cancel();
        assert!(rx.recv_timeout(Duration::from_secs(1)).unwrap().is_none());
        worker.join().unwrap();
    }

    #[test]
    fn slowloris_has_absolute_request_deadline() {
        let (mut server, mut peer) = UnixStream::pair().unwrap();
        let (stop_tx, stop_rx) = mpsc::channel();
        let writer = std::thread::spawn(move || {
            loop {
                if peer.write_all(b" ").is_err() { break; }
                if stop_rx.recv_timeout(Duration::from_millis(50)).is_ok() { break; }
            }
        });
        let started = Instant::now();
        let error = read_request_frame(&mut server).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < REQUEST_TIMEOUT + Duration::from_secs(1));
        let _ = stop_tx.send(());
        drop(server);
        writer.join().unwrap();
    }

    #[test]
    fn slow_reader_has_absolute_write_deadline() {
        let (server, _peer) = UnixStream::pair().unwrap();
        let started = Instant::now();
        let error = write_bytes_until(&server, &vec![b'x'; MAX_REPLAY_BYTES], started + Duration::from_millis(100)).unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
        assert!(started.elapsed() < Duration::from_secs(1));
    }

    #[test]
    fn large_vpn_list_fits_snapshot_frame_budget() {
        let snapshot = vpn::Snapshot {
            groups: vec![vpn::Group { name: "Proxy".into(), kind: "Selector".into(),
                all: (0..100_000).map(|n| format!("server-{n:06}-{}", "x".repeat(140))).collect(), ..Default::default() }],
            ..Default::default()
        };
        let reply = Reply::ok(serde_json::to_value(&snapshot).unwrap());
        let frame = encode_frame(&reply, MAX_FRAME_BYTES).unwrap();
        assert!(frame.len() > 14 << 20);
        assert!(frame.len() < 16 << 20);
        let decoded: Reply = serde_json::from_slice(&frame).unwrap();
        let restored: vpn::Snapshot = serde_json::from_value(decoded.data).unwrap();
        assert_eq!(restored.groups[0].all.len(), 100_000);
    }

    #[test]
    fn connection_spawn_failure_releases_guard() {
        let _isolation = crate::common::contract_fixtures::isolation_lock();
        let mut env = crate::common::contract_fixtures::EnvGuard::new();
        env.set("CM_STATE_DIR", "/tmp/cm-contract-spawn");
        env.set("CM_HELPER_FAIL_CONNECTION_WORKER", "1");
        let shared = Arc::new(Mutex::new(State::default()));
        assert!(reserve_connection(&shared, 1000));
        let guard = ClientGuard { shared: shared.clone(), uid: 1000 };
        assert!(spawn_connection_worker(move || { let _guard = guard; }).is_err());
        assert_eq!(lock(&shared).clients, 0);
    }

    #[test]
    fn authorization_owner_is_rechecked_after_barrier() {
        let shared = Arc::new(Mutex::new(State::default()));
        let id = begin(&mut lock(&shared), "check".into(), 1000).unwrap();
        let (runner, _commands) = mpsc::sync_channel(1);
        {
            let mut st = lock(&shared); let op = st.op.as_mut().unwrap();
            op.runner = Some(runner); op.phase = OperationPhase::Waiting;
            op.prompt = Some((PromptId(1), "Continue? [Y/n]".into(), PromptKind::YesNo { default_yes: true }));
        }
        let (authorized, resume) = mpsc::sync_channel(1);
        let (ready, barrier) = mpsc::sync_channel(1);
        let worker_shared = shared.clone(); let worker_id = id.clone();
        let worker = std::thread::spawn(move || {
            let captured_owner = lock(&worker_shared).op.as_ref().unwrap().owner;
            ready.send(()).unwrap(); resume.recv().unwrap();
            submit_input(&worker_shared, captured_owner, &worker_id, PromptId(1), "answer").unwrap_err().code
        });
        barrier.recv_timeout(Duration::from_secs(1)).unwrap();
        lock(&shared).op.as_mut().unwrap().owner = 1001;
        authorized.send(()).unwrap();
        assert_eq!(worker.join().unwrap(), ControlErrorCode::StaleOperation);
        assert!(lock(&shared).op.as_ref().unwrap().prompt.is_some());
    }

    /// Exercises handle -> external fake pkcheck -> post-auth operation recheck.
    /// This is not an acceptance test of the installed system polkit policy.
    #[test]
    fn delayed_fake_authorizer_denies_foreign_uid_and_cannot_cancel_next_operation() {
        use crate::common::contract_fixtures::{isolation_lock, TempDirGuard, EnvGuard};
        use std::os::unix::fs::PermissionsExt;
        let _isolation = isolation_lock();
        for allowed in [false, true] {
            let dir = TempDirGuard::new("cm-fake-polkit-barrier").unwrap();
            let pkcheck = dir.path().join("pkcheck");
            let ready = dir.path().join("ready"); let release = dir.path().join("release");
            let code = if allowed { 0 } else { 1 };
            std::fs::write(&pkcheck, format!("#!/bin/sh\nprintf '%s' \"$*\" > \"${{0%/*}}/ready\"\ni=0\nwhile [ ! -e \"${{0%/*}}/release\" ]; do i=$((i+1)); [ $i -lt 300 ] || exit 1; /bin/sleep 0.01; done\nexit {code}\n")).unwrap();
            std::fs::set_permissions(&pkcheck, std::fs::Permissions::from_mode(0o755)).unwrap();
            let apt = dir.path().join("apt-get"); std::fs::write(&apt, "#!/bin/sh\nexit 99\n").unwrap();
            std::fs::set_permissions(&apt, std::fs::Permissions::from_mode(0o755)).unwrap();
            let mut env = EnvGuard::new(); env.set("PATH", dir.path()); env.set("CM_HELPER_ALLOW", "0");
            let shared = Arc::new(Mutex::new(State::default()));
            let old = begin(&mut lock(&shared), "check".into(), 65534).unwrap();
            let (runner, commands) = mpsc::sync_channel(1);
            { let mut st = lock(&shared); let op = st.op.as_mut().unwrap(); op.runner = Some(runner); op.phase = OperationPhase::Running; }
            let (mut client, server) = UnixStream::pair().unwrap();
            client.set_read_timeout(Some(Duration::from_secs(4))).unwrap();
            let worker_shared = shared.clone();
            let peer = Peer { pid: std::process::id() as i32, uid: 1000 };
            let worker = std::thread::spawn(move || { let backend = backend::detect().unwrap(); handle(&worker_shared, backend.as_ref(), server, peer); });
            send_limited(&mut client, &Envelope { protocol_version: PROTOCOL_VERSION, lang: "en".into(), req: Request::Cancel { operation_id: old.clone() } }, MAX_REQUEST_BYTES).unwrap();
            let deadline = Instant::now() + Duration::from_secs(2);
            while !ready.exists() { assert!(Instant::now() < deadline, "authorizer did not reach barrier"); std::thread::sleep(Duration::from_millis(5)); }
            let subject = std::fs::read_to_string(&ready).unwrap();
            assert!(subject.contains(ACTION_MANAGE));
            assert!(subject.contains(&format!("--process {},{},1000", peer.pid, start_time(peer.pid).unwrap())));
            if allowed {
                finish(&shared, &old, 0);
                let new = begin(&mut lock(&shared), "check".into(), 65534).unwrap();
                assert_ne!(new, old);
                on_output(&shared, &old, b"stale callback\n");
                assert!(lock(&shared).op.as_ref().unwrap().term.lines.is_empty());
            }
            std::fs::write(release, b"release").unwrap();
            let frame = read_frame(&mut BufReader::new(client), MAX_FRAME_BYTES).unwrap().unwrap();
            let reply: Reply = serde_json::from_slice(&frame).unwrap();
            worker.join().unwrap();
            if allowed { assert_eq!(reply.control_error, Some(ControlErrorCode::StaleOperation)); }
            else { assert!(reply.denied && !reply.ok); }
            assert!(commands.try_recv().is_err(), "foreign/stale authorization reached runner");
        }
    }

    #[test]
    fn answers_are_limited_without_consuming_prompt() {
        let mut st = State::default(); let id = begin(&mut st, "check".into(), 1000).unwrap();
        let (runner, _commands) = mpsc::sync_channel(1);
        let op = st.op.as_mut().unwrap(); op.runner = Some(runner); op.phase = OperationPhase::Waiting;
        op.prompt = Some((PromptId(1), "question".into(), PromptKind::Text));
        for answer in ["x".repeat(MAX_ANSWER_BYTES + 1), "secret\nsecond".into(), "\u{001b}".into()] {
            let error = input_operation(&mut st, 1000, &id, PromptId(1), &answer).unwrap_err();
            assert_eq!(error.code, ControlErrorCode::InvalidAnswer);
            assert!(!error.message.contains(&answer));
            assert!(st.op.as_ref().unwrap().prompt.is_some());
        }
    }

    #[test]
    fn full_pty_input_expires_without_blocking_state_or_cancel() {
        let (mut master, mut slave) = (-1, -1);
        assert_eq!(unsafe { libc::openpty(&mut master, &mut slave, std::ptr::null_mut(), std::ptr::null(), std::ptr::null()) }, 0);
        let mut input = unsafe { File::from_raw_fd(master) };
        let slave = unsafe { File::from_raw_fd(slave) };
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        assert_eq!(unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut attributes) }, 0);
        unsafe { libc::cfmakeraw(&mut attributes); }
        assert_eq!(unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) }, 0);
        let flags = unsafe { libc::fcntl(master, libc::F_GETFL) };
        assert_eq!(unsafe { libc::fcntl(master, libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
        let fill = vec![b'x'; 8192]; let fill_deadline = Instant::now() + Duration::from_secs(2);
        let mut full_rounds = 0;
        while full_rounds < 10 {
            match input.write(&fill) {
                Ok(_) => { full_rounds = 0; }, Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => { full_rounds += 1; std::thread::sleep(Duration::from_millis(20)); },
                Err(error) => panic!("PTY fill failed: {error}"),
            }
            assert!(Instant::now() < fill_deadline);
        }
        let deadline = Instant::now() + Duration::from_millis(100);
        let started = Instant::now(); let mut offset = 0;
        loop {
            if let Some(result) = advance_input(&mut input, &vec![b'y'; MAX_ANSWER_BYTES], &mut offset, deadline, true) {
                assert!(result.unwrap_err().contains("deadline")); break;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
        assert!(started.elapsed() < Duration::from_secs(1));
        let shared = Arc::new(Mutex::new(State::default()));
        let id = begin(&mut lock(&shared), "check".into(), 1000).unwrap();
        let child = Command::new("sleep").arg("30").process_group(0).spawn().unwrap();
        let pgid = child.id() as i32;
        let (runner, commands) = mpsc::sync_channel(16);
        let (reader_done_tx, reader_done) = mpsc::sync_channel(1);
        let (_wake_read, reader_wake) = UnixStream::pair().unwrap();
        let mut resources = RunnerResources { child: Some(child), pgid: Some(pgid), reader: None,
            reader_done, reader_stop: Arc::new(AtomicBool::new(false)), reader_wake, commands, input };
        {
            let mut st = lock(&shared); let op = st.op.as_mut().unwrap();
            op.runner = Some(runner); op.phase = OperationPhase::Waiting;
            op.prompt = Some((PromptId(1), "question".into(), PromptKind::Text));
        }
        let worker_shared = shared.clone(); let worker_id = id.clone();
        let worker = std::thread::spawn(move || {
            let code = runner_loop(&worker_shared, &worker_id, &mut resources);
            finish(&worker_shared, &worker_id, code);
        });
        let result = input_operation(&mut lock(&shared), 1000, &id, PromptId(1), &"y".repeat(MAX_ANSWER_BYTES)).unwrap();
        std::thread::sleep(Duration::from_millis(70));
        let started = Instant::now();
        assert!(op_status(&lock(&shared)).running);
        assert!(started.elapsed() < Duration::from_millis(100));
        cancel_operation(&shared, 1000, &id).unwrap();
        assert!(started.elapsed() < Duration::from_millis(500));
        assert!(result.recv_timeout(Duration::from_secs(2)).unwrap().is_err());
        worker.join().unwrap();
        assert!(!op_status(&lock(&shared)).running);
        drop(reader_done_tx);
        drop(slave);
    }

    #[test]
    fn debug_request_redacts_answer() {
        let request = Request::Input { operation_id: OperationId("fixture".into()), prompt_id: PromptId(1), data: "test-password".into() };
        let diagnostic = format!("{request:?}");
        assert!(!diagnostic.contains("test-password"));
        assert!(diagnostic.contains("redacted"));
    }

}

#[cfg(test)]
mod reply_deadline_tests {
    use super::*;
    #[test]
    fn slow_drip_reply_cannot_extend_absolute_deadline() {
        let (client, mut server) = UnixStream::pair().unwrap();
        let worker = std::thread::spawn(move || {
            for _ in 0..30 {
                if server.write_all(b"x").is_err() { break; }
                std::thread::sleep(Duration::from_millis(20));
            }
        });
        let start = Instant::now();
        let deadline = start + Duration::from_millis(80);
        let mut reader = BufReader::new(client);
        let result = read_frame_before(&mut reader, 100, |r| {
            let remaining = deadline.checked_duration_since(Instant::now()).filter(|d| !d.is_zero())
                .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline"))?;
            r.get_ref().set_read_timeout(Some(remaining))
        });
        assert!(result.is_err());
        assert!(start.elapsed() < Duration::from_millis(300));
        drop(reader);
        worker.join().unwrap();
    }
}

#[cfg(test)]
mod terminal_boundary_tests {
    use super::*;
    #[test]
    fn every_utf8_boundary_invalid_prefix_and_eof_tail_survive() {
        let text = "Ж中🙂 Continue? [Y/n]";
        for split in 0..=text.len() {
            let mut term = Term::default(); term.feed(&text.as_bytes()[..split]);
            assert!(term.utf8.len() <= 3); term.feed(&text.as_bytes()[split..]); assert_eq!(term.partial, text);
        }
        let mut term = Term::default(); term.feed(b"\xffContinue? [Y/n] \xf0\x9f");
        assert_eq!(term.flush().unwrap(), "�Continue? [Y/n] �"); assert!(term.utf8.is_empty());
    }
    #[test]
    fn tabs_ansi_osc_and_cr_are_bounded() {
        let mut term = Term::default(); term.feed(&vec![b'\t'; 1 << 20]);
        assert!(term.partial.len() <= MAX_LINE_CONTENT_BYTES); assert!(term.flush().unwrap().ends_with('…'));
        let mut term = Term::default();
        term.feed(b"old\r\x1b[31mnew\x1b[0m\x1b]0;hidden\x1bXstill hidden\x07\n");
        assert_eq!(term.lines.back().unwrap(), "new");
    }
}
