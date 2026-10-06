//! Запуск интерактивной команды в PTY и простой журнал для экрана TUI.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::process::CommandExt;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError, TrySendError};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use std::time::Instant;

const MAX_LINES: usize = 3000;
const MAX_RAW: usize = 512 * 1024;
const MAX_LINE: usize = 16_384;
const MAX_JOURNAL: usize = 3 << 20;

#[derive(Clone, Copy, Default)]
enum Escape {
    #[default]
    Text,
    Start,
    Charset,
    Csi,
    Osc,
    OscEscape,
}

pub(super) struct ProcessSession {
    pub title: String,
    child: Child,
    master: File,
    output: Receiver<Vec<u8>>,
    lines: VecDeque<String>,
    current: String,
    current_truncated: bool,
    line_bytes: usize,
    history_truncated: bool,
    utf8: Vec<u8>,
    escape: Escape,
    csi: String,
    attach_requested: bool,
    alternate_screen: bool,
    pending_cr: bool,
    raw: VecDeque<u8>,
    exit_code: Option<i32>,
    ended_at: Option<Instant>,
    read_done: bool,
    scroll_back: usize,
    started: Instant,
    /// Owned process group; WNOWAIT reserves the leader PID until group cleanup/reap.
    pgid: i32,
    leader_start: Option<u64>,
    child_reaped: bool,
    reader: Option<std::thread::JoinHandle<()>>,
    reader_stop: Arc<AtomicBool>,
    #[cfg(test)]
    reader_blocked: Arc<AtomicBool>,
    /// запись сюда будит поток чтения, чтобы он закрыл свою копию PTY
    wake: std::os::unix::net::UnixStream,
    /// этапы команды: строки вида «[3/6] Загрузка»
    stages: Vec<(u32, u32, String)>,
    progress: super::progress::Progress,
    line_sequence: u64,
    answered_question: Option<(u64, String)>,
}

/// Время старта процесса (поле 22 /proc/PID/stat, в тиках с загрузки).
fn start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // имя процесса в скобках может содержать пробелы — считаем поля после последней ')'
    stat.rsplit_once(')')?
        .1
        .split_whitespace()
        .nth(19)?
        .parse()
        .ok()
}

/// «[3/6] Загрузка» → (3, 6, «Загрузка»).
fn parse_stage(line: &str) -> Option<(u32, u32, String)> {
    let (nm, title) = line.trim().strip_prefix('[')?.split_once("] ")?;
    let (n, m) = nm.split_once('/')?;
    let (n, m): (u32, u32) = (n.parse().ok()?, m.parse().ok()?);
    (n >= 1 && n <= m && m <= 20 && !title.trim().is_empty())
        .then(|| (n, m, title.trim().to_string()))
}

pub(super) fn is_update_stage(line: &str) -> bool {
    parse_stage(line).is_some_and(|(_, _, title)| {
        [
            t!("Зеркала"),
            t!("Проверка"),
            t!("Загрузка"),
            t!("Снапшот"),
            t!("Установка"),
            t!("После обновления"),
        ]
        .contains(&title.as_str())
    })
}

impl ProcessSession {
    pub fn spawn(args: &[String], rows: u16, cols: u16) -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut s = Self::spawn_program(exe.as_os_str(), args, rows, cols)?;
        s.title = format!("cm {}", args.join(" "));
        if args.first().is_some_and(|arg| arg == "aur") {
            s.progress.aur = true;
            s.progress.phase = "Загрузка исходников";
        }
        Ok(s)
    }

    fn spawn_program(
        program: &std::ffi::OsStr,
        args: &[String],
        rows: u16,
        cols: u16,
    ) -> io::Result<Self> {
        // Отдельный терминал: sudo/pacman/apt видят настоящий TTY. Оба конца сразу с FD_CLOEXEC.
        let (master, slave) = cm::common::open_pty_pair(rows, cols)?;
        let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
        if flags < 0
            || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }
                < 0
        {
            return Err(io::Error::last_os_error());
        }
        let mut reader = cm::common::dup_cloexec(&master)?;
        let mut command = Command::new(program);
        command
            .args(args)
            .env("NO_COLOR", "1")
            .env("PAGER", "cat")
            .env("GIT_PAGER", "cat");
        command.stdin(Stdio::from(cm::common::dup_cloexec(&slave)?));
        command.stdout(Stdio::from(cm::common::dup_cloexec(&slave)?));
        command.stderr(Stdio::from(slave));
        // Новый сеанс делает slave управляющим терминалом и сохраняет /dev/tty для дочерних программ.
        unsafe {
            command.pre_exec(|| {
                if libc::setsid() < 0 || libc::ioctl(0, libc::TIOCSCTTY, 0) < 0 {
                    return Err(io::Error::last_os_error());
                }
                Ok(())
            });
        }
        // пара сокетов, а не pipe: запись с MSG_NOSIGNAL не даёт SIGPIPE, если поток чтения уже вышел
        let (wake, wake_rx) = std::os::unix::net::UnixStream::pair()?;
        let mut child = command.spawn()?;
        let pgid = child.id() as i32;
        let leader_start = start_time(pgid);
        #[cfg(test)]
        if let Some(path) = std::env::var_os("CM_TUI_CHILD_PID_FILE") {
            if let Err(error) = std::fs::write(path, pgid.to_string()) {
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        }
        let (tx, output) = mpsc::sync_channel(128);
        // Чтение ждёт и PTY, и канал пробуждения: потомок, удерживающий PTY, не оставит поток висеть после закрытия экрана.
        let reader_stop = Arc::new(AtomicBool::new(false));
        let worker_stop = reader_stop.clone();
        #[cfg(test)]
        let reader_blocked = Arc::new(AtomicBool::new(false));
        #[cfg(test)]
        let worker_blocked = reader_blocked.clone();
        let read_work = move || {
            let mut buf = [0u8; 8192];
            loop {
                if worker_stop.load(Ordering::Acquire) {
                    break;
                }
                let mut fds = [
                    libc::pollfd {
                        fd: reader.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                    libc::pollfd {
                        fd: wake_rx.as_raw_fd(),
                        events: libc::POLLIN,
                        revents: 0,
                    },
                ];
                if unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) } < 0 {
                    if io::Error::last_os_error().kind() == io::ErrorKind::Interrupted {
                        continue;
                    }
                    break;
                }
                if fds[1].revents != 0 {
                    break;
                }
                if fds[0].revents == 0 {
                    continue;
                }
                match reader.read(&mut buf) {
                    Ok(0) => break,
                    Err(error)
                        if matches!(
                            error.kind(),
                            io::ErrorKind::WouldBlock | io::ErrorKind::Interrupted
                        ) =>
                    {
                        continue;
                    }
                    Err(_) => break,
                    Ok(n) => {
                        let mut bytes = buf[..n].to_vec();
                        loop {
                            if worker_stop.load(Ordering::Acquire) {
                                return;
                            }
                            match tx.try_send(bytes) {
                                Ok(()) => break,
                                Err(TrySendError::Disconnected(_)) => return,
                                Err(TrySendError::Full(pending)) => {
                                    #[cfg(test)]
                                    worker_blocked.store(true, Ordering::Release);
                                    bytes = pending;
                                }
                            }
                            let mut wake_fd = libc::pollfd {
                                fd: wake_rx.as_raw_fd(),
                                events: libc::POLLIN,
                                revents: 0,
                            };
                            if unsafe { libc::poll(&mut wake_fd, 1, 10) } > 0 {
                                return;
                            }
                        }
                    }
                }
            }
        };
        #[cfg(test)]
        let injected = std::env::var("CM_TUI_FAIL_READER").as_deref() == Ok("1");
        #[cfg(not(test))]
        let injected = false;
        let reader = if injected {
            Err(io::Error::other("injected reader spawn failure"))
        } else {
            std::thread::Builder::new()
                .name("cm-pty".into())
                .spawn(read_work)
        };
        let reader = match reader {
            Ok(reader) => reader,
            Err(error) => {
                // The child is still owned/unreaped, so its PGID cannot be recycled.
                unsafe {
                    libc::killpg(pgid, libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                return Err(error);
            }
        };
        Ok(Self {
            title: String::new(),
            child,
            master,
            output,
            lines: VecDeque::new(),
            current: String::new(),
            current_truncated: false,
            line_bytes: 0,
            history_truncated: false,
            utf8: Vec::new(),
            escape: Escape::Text,
            csi: String::new(),
            attach_requested: false,
            alternate_screen: false,
            pending_cr: false,
            raw: VecDeque::new(),
            exit_code: None,
            ended_at: None,
            read_done: false,
            scroll_back: 0,
            started: Instant::now(),
            pgid,
            leader_start,
            child_reaped: false,
            reader: Some(reader),
            reader_stop,
            #[cfg(test)]
            reader_blocked,
            wake,
            stages: Vec::new(),
            progress: super::progress::Progress::default(),
            line_sequence: 0,
            answered_question: None,
        })
    }

    pub fn poll(&mut self) -> io::Result<Vec<Vec<u8>>> {
        let mut chunks = Vec::new();
        for _ in 0..64 {
            match self.output.try_recv() {
                Ok(bytes) => {
                    self.record(&bytes);
                    chunks.push(bytes);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => {
                    if !self.read_done {
                        self.flush_output();
                    }
                    self.read_done = true;
                    break;
                }
            }
        }
        if self.exit_code.is_none() && !self.child_reaped {
            // Observe without reaping: reserve leader PID until group cleanup.
            let mut info: libc::siginfo_t = unsafe { std::mem::zeroed() };
            if unsafe {
                libc::waitid(
                    libc::P_PID,
                    self.child.id(),
                    &mut info,
                    libc::WEXITED | libc::WNOHANG | libc::WNOWAIT,
                )
            } < 0
            {
                return Err(io::Error::last_os_error());
            }
            if unsafe { info.si_pid() } != 0 {
                let status = unsafe { info.si_status() };
                self.exit_code = Some(if info.si_code == libc::CLD_EXITED {
                    status
                } else {
                    128 + status
                });
                self.ended_at = Some(Instant::now());
            }
        }
        if self.exit_code.is_some()
            && !self.child_reaped
            && (self.read_done || self.ended_at.is_some_and(|t| t.elapsed().as_secs() >= 1))
        {
            self.reap_group();
            self.stop_reader();
            while let Ok(bytes) = self.output.try_recv() {
                self.record(&bytes);
                chunks.push(bytes);
            }
            self.flush_output();
            self.read_done = true;
        }
        Ok(chunks)
    }

    pub fn finished(&self) -> Option<i32> {
        self.exit_code
            .filter(|_| self.child_reaped && self.read_done)
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
    }

    pub fn animation_tick(&self) -> usize {
        (self.started.elapsed().as_millis() / 150) as usize
    }
    pub fn progress(&self) -> &super::progress::Progress {
        &self.progress
    }
    pub fn current_line(&self) -> &str {
        if !self.current.is_empty() {
            &self.current
        } else {
            self.lines.back().map_or("", String::as_str)
        }
    }
    pub fn question(&self) -> Option<(u64, String, bool)> {
        if self.finished().is_some() || self.alternate_screen {
            return None;
        }
        let text = self.current_line();
        let cm::helper::PromptKind::YesNo { default_yes } = cm::helper::prompt_kind(text)? else {
            return None;
        };
        if self
            .answered_question
            .as_ref()
            .is_some_and(|(id, previous)| *id == self.line_sequence && previous == text)
        {
            return None;
        }
        Some((self.line_sequence, text.to_string(), default_yes))
    }
    pub fn answer_question(&mut self, yes: bool) -> io::Result<()> {
        let Some((id, text, _)) = self.question() else {
            return Ok(());
        };
        self.write_input(if yes { b"y\r" } else { b"n\r" })?;
        self.answered_question = Some((id, text));
        Ok(())
    }

    pub fn send_key(&mut self, key: KeyCode, modifiers: KeyModifiers) -> io::Result<()> {
        if self.exit_code.is_some() {
            return Ok(());
        }
        let mut bytes = Vec::new();
        match key {
            KeyCode::Char(c)
                if modifiers.contains(KeyModifiers::CONTROL) && c.is_ascii_alphabetic() =>
            {
                bytes.push((c.to_ascii_lowercase() as u8) - b'a' + 1);
            }
            KeyCode::Char(c) => {
                if modifiers.contains(KeyModifiers::ALT) {
                    bytes.push(0x1b);
                }
                let mut buf = [0; 4];
                bytes.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
            KeyCode::Enter => bytes.push(b'\r'),
            KeyCode::Backspace => bytes.push(0x7f),
            KeyCode::Tab => bytes.push(b'\t'),
            KeyCode::Esc => bytes.push(0x1b),
            KeyCode::Up => bytes.extend_from_slice(b"\x1b[A"),
            KeyCode::Down => bytes.extend_from_slice(b"\x1b[B"),
            KeyCode::Right => bytes.extend_from_slice(b"\x1b[C"),
            KeyCode::Left => bytes.extend_from_slice(b"\x1b[D"),
            KeyCode::Home => bytes.extend_from_slice(b"\x1b[H"),
            KeyCode::End => bytes.extend_from_slice(b"\x1b[F"),
            KeyCode::PageUp => bytes.extend_from_slice(b"\x1b[5~"),
            KeyCode::PageDown => bytes.extend_from_slice(b"\x1b[6~"),
            KeyCode::Delete => bytes.extend_from_slice(b"\x1b[3~"),
            KeyCode::Insert => bytes.extend_from_slice(b"\x1b[2~"),
            _ => {}
        }
        if !bytes.is_empty() {
            self.write_input(&bytes)?;
        }
        Ok(())
    }

    fn write_input(&mut self, bytes: &[u8]) -> io::Result<()> {
        let deadline = Instant::now() + std::time::Duration::from_millis(100);
        let mut offset = 0;
        while offset < bytes.len() {
            match self.master.write(&bytes[offset..]) {
                Ok(0) => return Err(io::Error::new(io::ErrorKind::WriteZero, "PTY input closed")),
                Ok(n) => offset += n,
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                    let remaining = deadline.saturating_duration_since(Instant::now());
                    if remaining.is_zero() {
                        return Err(io::Error::new(
                            io::ErrorKind::TimedOut,
                            "PTY input deadline exceeded",
                        ));
                    }
                    let mut fd = libc::pollfd {
                        fd: self.master.as_raw_fd(),
                        events: libc::POLLOUT,
                        revents: 0,
                    };
                    let result = unsafe {
                        libc::poll(&mut fd, 1, remaining.as_millis().saturating_add(1) as i32)
                    };
                    if result < 0 && io::Error::last_os_error().kind() != io::ErrorKind::Interrupted
                    {
                        return Err(io::Error::last_os_error());
                    }
                }
                Err(error) => return Err(error),
            }
        }
        Ok(())
    }

    pub fn resize(&self, rows: u16, cols: u16) -> io::Result<()> {
        let size = libc::winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        if unsafe { libc::ioctl(self.master.as_raw_fd(), libc::TIOCSWINSZ, &size) } < 0 {
            return Err(io::Error::last_os_error());
        }
        Ok(())
    }

    pub fn scroll_up(&mut self, amount: usize) {
        self.scroll_back = self
            .scroll_back
            .saturating_add(amount)
            .min(self.lines.len());
    }

    pub fn scroll_down(&mut self, amount: usize) {
        self.scroll_back = self.scroll_back.saturating_sub(amount);
    }

    pub fn lines_for(&self, height: usize) -> Vec<String> {
        let total = self.lines.len() + usize::from(!self.current.is_empty());
        let end = total.saturating_sub(self.scroll_back.min(total));
        let start = end.saturating_sub(height);
        let mut visible: Vec<String> = self
            .lines
            .iter()
            .skip(start)
            .take(
                end.saturating_sub(start)
                    .min(self.lines.len().saturating_sub(start)),
            )
            .cloned()
            .collect();
        if end == total && !self.current.is_empty() && visible.len() < height {
            visible.push(self.current.clone());
        }
        visible
    }

    pub fn stages(&self) -> &[(u32, u32, String)] {
        &self.stages
    }

    pub fn raw_history(&self) -> Vec<u8> {
        self.raw.iter().copied().collect()
    }

    pub fn take_attach_request(&mut self) -> bool {
        std::mem::take(&mut self.attach_requested)
    }

    pub fn alternate_screen_active(&self) -> bool {
        self.alternate_screen
    }

    fn record(&mut self, bytes: &[u8]) {
        for chunk in bytes.chunks(8192) {
            self.record_chunk(chunk);
        }
    }
    fn record_chunk(&mut self, bytes: &[u8]) {
        let excess = (self.raw.len() + bytes.len()).saturating_sub(MAX_RAW);
        self.raw.drain(..excess);
        self.raw.extend(bytes);
        self.utf8.extend_from_slice(bytes);
        let pending = std::mem::take(&mut self.utf8);
        let mut pos = 0;
        while pos < pending.len() {
            match std::str::from_utf8(&pending[pos..]) {
                Ok(s) => {
                    self.record_text(s);
                    break;
                }
                Err(e) => {
                    let valid = e.valid_up_to();
                    if valid > 0 {
                        self.record_text(
                            std::str::from_utf8(&pending[pos..pos + valid]).unwrap_or(""),
                        );
                        pos += valid;
                    }
                    if let Some(len) = e.error_len() {
                        self.record_char('�');
                        pos += len;
                    } else {
                        self.utf8.extend_from_slice(&pending[pos..]);
                        break;
                    }
                }
            }
        }
        self.progress.observe(&self.current);
    }

    fn record_text(&mut self, text: &str) {
        for ch in text.chars() {
            self.record_char(ch);
        }
    }

    fn record_char(&mut self, ch: char) {
        match self.escape {
            Escape::Start => {
                self.escape = match ch {
                    '[' => {
                        self.csi.clear();
                        Escape::Csi
                    }
                    ']' => Escape::Osc,
                    '(' | ')' => Escape::Charset,
                    _ => Escape::Text,
                };
                return;
            }
            Escape::Charset => {
                self.escape = Escape::Text;
                return;
            }
            Escape::Csi => {
                if self.csi.len() + ch.len_utf8() <= 64 {
                    self.csi.push(ch);
                }
                if ('@'..='~').contains(&ch) {
                    if self.csi == "?1049h" || self.csi == "?47h" || self.csi == "?1047h" {
                        self.attach_requested = !self.alternate_screen;
                        self.alternate_screen = true;
                    } else if self.csi == "?1049l" || self.csi == "?47l" || self.csi == "?1047l" {
                        self.alternate_screen = false;
                    } else if self.csi == "1G" || self.csi == "0G" || self.csi == "2K" {
                        self.current.clear();
                        self.current_truncated = false;
                    }
                    self.escape = Escape::Text;
                }
                return;
            }
            Escape::Osc => {
                self.escape = match ch {
                    '\x07' => Escape::Text,
                    '\x1b' => Escape::OscEscape,
                    _ => Escape::Osc,
                };
                return;
            }
            Escape::OscEscape => {
                self.escape = if ch == '\\' || ch == '\x07' {
                    Escape::Text
                } else {
                    Escape::Osc
                };
                return;
            }
            Escape::Text => {}
        }
        if self.alternate_screen {
            if ch == '\x1b' {
                self.escape = Escape::Start;
            }
            return;
        }
        if self.pending_cr {
            self.pending_cr = false;
            if ch == '\n' {
                self.push_line();
                return;
            }
            self.current.clear();
            self.current_truncated = false;
        }
        match ch {
            '\x1b' => self.escape = Escape::Start,
            '\r' => self.pending_cr = true,
            '\n' => self.push_line(),
            '\x08' => {
                self.current.pop();
            }
            '\t' => {
                for _ in 0..4 {
                    self.append(' ');
                }
            }
            c if !c.is_control() => self.append(c),
            _ => {}
        }
    }

    fn append(&mut self, ch: char) {
        if self.current_truncated {
            return;
        }
        if self.current.len() + ch.len_utf8() <= MAX_LINE - '…'.len_utf8() {
            self.current.push(ch);
        } else {
            self.current_truncated = true;
        }
    }
    fn flush_output(&mut self) {
        if !self.utf8.is_empty() {
            self.utf8.clear();
            self.record_char('�');
        }
        if !self.current.is_empty() {
            self.push_line();
        }
    }
    fn push_line(&mut self) {
        self.progress.observe(&self.current);
        if let Some(stage) = parse_stage(&self.current).filter(|_| is_update_stage(&self.current)) {
            self.progress.update_stage();
            if stage.0 == 1 || self.stages.last().is_some_and(|last| last.1 != stage.1) {
                self.stages.clear();
            }
            self.stages.retain(|previous| previous.0 != stage.0);
            self.stages.push(stage);
        }
        if self.current_truncated {
            self.current.push('…');
            self.current_truncated = false;
        }
        self.line_bytes += self.current.len();
        self.lines.push_back(std::mem::take(&mut self.current));
        self.line_sequence = self.line_sequence.wrapping_add(1);
        while self.lines.len() > MAX_LINES || self.line_bytes > MAX_JOURNAL {
            if let Some(line) = self.lines.pop_front() {
                self.line_bytes -= line.len();
                self.history_truncated = true;
            }
        }
    }
}

impl ProcessSession {
    /// Only an owned, unreaped leader reserves the process-group ID. Missing
    /// /proc/PGID after reaping never proves ownership of a recycled group.
    fn group_is_ours(&self) -> bool {
        !self.child_reaped
            && self.pgid > 0
            && self.pgid == self.child.id() as i32
            && start_time(self.pgid).is_some_and(|time| Some(time) == self.leader_start)
    }
    fn stop_reader(&mut self) {
        self.reader_stop.store(true, Ordering::Release);
        unsafe {
            libc::send(
                self.wake.as_raw_fd(),
                [1u8].as_ptr() as *const libc::c_void,
                1,
                libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT,
            );
        }
        if let Some(reader) = self.reader.take() {
            let _ = reader.join();
        }
    }
    fn reap_group(&mut self) {
        if self.child_reaped {
            return;
        }
        if self.group_is_ours() {
            unsafe {
                libc::killpg(self.pgid, libc::SIGKILL);
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.child_reaped = true;
        self.pgid = 0;
    }
}

impl Drop for ProcessSession {
    fn drop(&mut self) {
        self.stop_reader();
        self.reap_group();
    }
}

#[cfg(test)]
pub(super) fn spawn_with(
    program: &str,
    args: &[String],
    rows: u16,
    cols: u16,
) -> io::Result<ProcessSession> {
    ProcessSession::spawn_program(std::ffi::OsStr::new(program), args, rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audit_resize_ctrl_c_and_utf8_terminal_session() {
        let _isolation = cm::common::contract_fixtures::isolation_lock();
        let mut session = spawn_with(
            "sh",
            &[
                "-c".into(),
                "trap 'exit 130' INT; printf 'ready Ж中🙂\\n'; while :; do sleep 1; done".into(),
            ],
            24,
            80,
        )
        .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !session.lines_for(20).join(" ").contains("ready") {
            session.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert!(session.lines_for(20).join(" ").contains("Ж中🙂"));
        session.resize(18, 60).unwrap();
        session.resize(24, 80).unwrap();
        session
            .send_key(KeyCode::Char('c'), KeyModifiers::CONTROL)
            .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while session.finished().is_none() {
            session.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        assert_eq!(session.finished(), Some(130));
    }
    #[test]
    fn stage_lines_are_parsed() {
        assert_eq!(
            parse_stage("[3/6] Загрузка"),
            Some((3, 6, "Загрузка".into()))
        );
        assert_eq!(
            parse_stage("  [1/6] Mirrors  "),
            Some((1, 6, "Mirrors".into()))
        );
        assert_eq!(parse_stage("[7/6] x"), None);
        assert_eq!(parse_stage("[a/b] x"), None);
        assert_eq!(parse_stage("text"), None);
    }

    fn threads() -> usize {
        std::fs::read_dir("/proc/self/task")
            .map(|d| d.count())
            .unwrap_or(0)
    }

    /// B06: команда завершилась, её потомок держит PTY; закрытие сессии освобождает поток чтения и убивает потомка.
    #[test]
    fn b06_closing_session_releases_reader_and_group() {
        let _iso = cm::common::contract_fixtures::isolation_lock();
        let dir = std::env::temp_dir().join(format!("cm-pty-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let pidfile = dir.join("pid");
        // лидер сразу выходит, фоновый sleep остаётся с открытым PTY
        // потомок игнорирует SIGHUP, который ядро шлёт при выходе лидера сеанса
        let ready = dir.join("ready");
        let script = format!(
            "(trap '' HUP; touch {r}; exec sleep 300) & while [ ! -e {r} ]; do sleep 0.02; done; echo $! > {p}; echo started",
            r = ready.display(),
            p = pidfile.display()
        );
        let mut s = spawn_with("sh", &["-c".into(), script], 24, 80).unwrap();
        let t0 = Instant::now();
        while s.finished().is_none() && t0.elapsed().as_secs() < 10 {
            let _ = s.poll();
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        assert_eq!(s.finished(), Some(0), "лидер завершился");
        let child: i32 = std::fs::read_to_string(&pidfile)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert!(
            s.child_reaped && s.reader.is_none(),
            "finished session already cleaned up its owned leader and reader"
        );
        let before = threads();
        drop(s);
        let t0 = Instant::now();
        while (std::path::Path::new(&format!("/proc/{child}/stat")).exists()
            && std::fs::read_to_string(format!("/proc/{child}/stat"))
                .map(|s| !s.contains(") Z"))
                .unwrap_or(false))
            && t0.elapsed().as_secs() < 5
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let gone = std::fs::read_to_string(format!("/proc/{child}/stat"))
            .map(|s| s.contains(") Z"))
            .unwrap_or(true);
        assert!(gone, "потомок завершён сигналом группе");
        assert!(
            threads() <= before,
            "поток чтения PTY завершился до завершения/drop сессии"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Закрытие сессии, когда поток чтения уже вышел: при SIGPIPE по умолчанию (как в main) процесс не погибает.
    #[test]
    fn b06_drop_after_reader_exit_does_not_raise_sigpipe() {
        let _isolation = cm::common::contract_fixtures::isolation_lock();
        let old = unsafe { libc::signal(libc::SIGPIPE, libc::SIG_DFL) };
        let mut s = spawn_with("true", &[], 24, 80).unwrap();
        let t0 = Instant::now();
        while s.finished().is_none() && t0.elapsed().as_secs() < 10 {
            let _ = s.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        std::thread::sleep(std::time::Duration::from_millis(200));
        drop(s);
        unsafe { libc::signal(libc::SIGPIPE, old) };
    }

    #[test]
    fn review_full_pty_input_has_a_deadline_and_keeps_poll_responsive() {
        let _isolation = cm::common::contract_fixtures::isolation_lock();
        let mut s = spawn_with(
            "sh",
            &[
                "-c".into(),
                "stty -echo -icanon; printf 'ready\\n'; exec sleep 30".into(),
            ],
            24,
            80,
        )
        .unwrap();
        let deadline = Instant::now() + std::time::Duration::from_secs(2);
        while !s.lines.iter().any(|line| line == "ready") {
            s.poll().unwrap();
            assert!(Instant::now() < deadline);
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let start = Instant::now();
        assert_eq!(
            s.write_input(&vec![b'x'; 1 << 20]).unwrap_err().kind(),
            io::ErrorKind::TimedOut
        );
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        s.poll().unwrap();
    }

    #[test]
    fn review_full_reader_queue_drop_joins_and_reaps_active_child() {
        let _isolation = cm::common::contract_fixtures::isolation_lock();
        let s = spawn_with(
            "sh",
            &[
                "-c".into(),
                "trap '' HUP TERM; exec head -c 16777216 /dev/zero".into(),
            ],
            24,
            80,
        )
        .unwrap();
        let pid = s.child.id();
        let stopped = s.reader_stop.clone();
        let deadline = Instant::now() + std::time::Duration::from_secs(3);
        while !s.reader_blocked.load(Ordering::Acquire) {
            assert!(
                Instant::now() < deadline,
                "fixture never filled bounded output queue"
            );
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        let start = Instant::now();
        drop(s);
        assert!(start.elapsed() < std::time::Duration::from_secs(1));
        assert!(stopped.load(Ordering::Acquire));
        assert!(
            !std::path::Path::new(&format!("/proc/{pid}")).exists(),
            "owned child must be reaped, not just signalled"
        );
    }

    #[test]
    fn review_reader_spawn_failure_rolls_back_child() {
        use cm::common::contract_fixtures::{EnvGuard, TempDirGuard, isolation_lock};
        let _isolation = isolation_lock();
        let dir = TempDirGuard::new("cm-reader-fail").unwrap();
        let pidfile = dir.path().join("pid");
        let mut env = EnvGuard::new();
        env.set("CM_TUI_FAIL_READER", "1");
        env.set("CM_TUI_CHILD_PID_FILE", &pidfile);
        assert!(spawn_with("sleep", &["30".into()], 24, 80).is_err());
        let pid = std::fs::read_to_string(pidfile).unwrap();
        assert!(!std::path::Path::new(&format!("/proc/{pid}")).exists());
    }

    #[test]
    fn b06_reused_pid_is_not_our_group() {
        let _isolation = cm::common::contract_fixtures::isolation_lock();
        let mut s = spawn_with("true", &[], 24, 80).unwrap();
        let t0 = Instant::now();
        while s.finished().is_none() && t0.elapsed().as_secs() < 10 {
            let _ = s.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(
            s.child_reaped && s.pgid == 0,
            "finished leader already reaped safely"
        );
        // будто номер лидера достался другому процессу: время старта не совпадает — сигнала нет
        s.pgid = std::process::id() as i32;
        s.leader_start = Some(u64::MAX);
        assert!(!s.group_is_ours());
        s.pgid = 0;
    }
}

#[cfg(test)]
mod parser_limit_tests {
    use super::*;
    fn parser() -> ProcessSession {
        let mut p = ProcessSession::spawn_program(
            std::ffi::OsStr::new("/bin/sh"),
            &["-c".into(), "exit 0".into()],
            24,
            80,
        )
        .unwrap();
        p.child.wait().unwrap();
        p.child_reaped = true;
        p.exit_code = Some(0);
        p.pgid = 0;
        p
    }
    #[test]
    fn utf8_invalid_prefix_boundaries_and_eof_tail() {
        let mut p = parser();
        let text = "Ж中🙂 Continue? [Y/n]";
        for split in 0..=text.len() {
            p.current.clear();
            p.utf8.clear();
            p.record(&text.as_bytes()[..split]);
            assert!(p.utf8.len() <= 3);
            p.record(&text.as_bytes()[split..]);
            assert_eq!(p.current, text);
        }
        p.current.clear();
        p.record(b"\xffContinue? [Y/n] \xf0\x9f");
        p.flush_output();
        assert_eq!(p.lines.back().unwrap(), "�Continue? [Y/n] �");
        assert!(p.utf8.is_empty());
    }
    #[test]
    fn million_tabs_and_hundred_thousand_stages_remain_bounded() {
        let mut p = parser();
        p.record(&vec![b'\t'; 1 << 20]);
        assert!(p.current.len() <= MAX_LINE);
        p.record(b"\n");
        assert!(p.lines.back().unwrap().ends_with('…'));
        assert!(p.raw.len() <= MAX_RAW);
        for n in 0..100000 {
            p.record(format!("[{}/6] {}\n", n % 6 + 1, t!("Загрузка")).as_bytes());
        }
        assert!(p.stages.len() <= 6);
        assert!(p.lines.len() <= MAX_LINES);
        assert!(p.line_bytes <= MAX_JOURNAL);
        assert_eq!(p.stages.last().unwrap().2, t!("Загрузка"));
        p.record(b"[1/5] Building CXX object\n");
        assert_eq!(p.stages.last().unwrap().2, t!("Загрузка"));
        assert_eq!(p.progress.ratio, Some(0.2));
    }
    #[test]
    fn alternate_screen_osc_cr_and_unicode_limits_are_preserved() {
        let mut p = parser();
        p.record(
            b"before\n\x1b[?1049heditor\n\x1b[?1049lold\r\x1b[31mnew\x1b[0m\x1b]0;hidden\x07\n",
        );
        assert!(!p.alternate_screen);
        assert!(p.take_attach_request());
        assert_eq!(
            p.lines.iter().map(String::as_str).collect::<Vec<_>>(),
            ["before", "new"]
        );
        p.record("🙂".repeat(10000).as_bytes());
        p.record(b"\n");
        let line = p.lines.back().unwrap();
        assert!(line.len() <= MAX_LINE);
        assert!(line.ends_with('…'));
    }
}
