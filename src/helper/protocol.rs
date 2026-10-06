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

