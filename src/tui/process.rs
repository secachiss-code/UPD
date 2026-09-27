//! Запуск интерактивной команды в PTY и простой журнал для экрана TUI.

use ratatui::crossterm::event::{KeyCode, KeyModifiers};
use std::collections::VecDeque;
use std::fs::File;
use std::io::{self, Read, Write};
use std::os::fd::{AsRawFd, FromRawFd};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc::{self, Receiver, TryRecvError};
use std::time::Instant;

const MAX_LINES: usize = 3000;
const MAX_RAW: usize = 512 * 1024;

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
    /// группа процессов команды (setsid: равна pid лидера) и время старта лидера из /proc —
    /// по ним после wait видно, не занят ли этот номер уже чужим процессом
    pgid: i32,
    leader_start: Option<u64>,
    /// запись сюда будит поток чтения, чтобы он закрыл свою копию PTY
    wake: std::os::unix::net::UnixStream,
    /// этапы команды: строки вида «[3/6] Загрузка»
    stages: Vec<(u32, u32, String)>,
}

/// Время старта процесса (поле 22 /proc/PID/stat, в тиках с загрузки).
fn start_time(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    // имя процесса в скобках может содержать пробелы — считаем поля после последней ')'
    stat.rsplit_once(')')?.1.split_whitespace().nth(19)?.parse().ok()
}

/// «[3/6] Загрузка» → (3, 6, «Загрузка»).
fn parse_stage(line: &str) -> Option<(u32, u32, String)> {
    let (nm, title) = line.trim().strip_prefix('[')?.split_once("] ")?;
    let (n, m) = nm.split_once('/')?;
    let (n, m): (u32, u32) = (n.parse().ok()?, m.parse().ok()?);
    (n >= 1 && n <= m && m <= 20 && !title.trim().is_empty()).then(|| (n, m, title.trim().to_string()))
}

impl ProcessSession {
    pub fn spawn(args: &[String], rows: u16, cols: u16) -> io::Result<Self> {
        let exe = std::env::current_exe()?;
        let mut s = Self::spawn_program(exe.as_os_str(), args, rows, cols)?;
        s.title = format!("upd {}", args.join(" "));
        Ok(s)
    }

    fn spawn_program(program: &std::ffi::OsStr, args: &[String], rows: u16, cols: u16) -> io::Result<Self> {
        let size = libc::winsize {
            ws_row: rows.max(1),
            ws_col: cols.max(1),
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        let (mut master_fd, mut slave_fd) = (-1, -1);
        // openpty создаёт отдельный терминал: sudo/pacman/apt видят настоящий TTY.
        if unsafe {
            libc::openpty(
                &mut master_fd,
                &mut slave_fd,
                std::ptr::null_mut(),
                std::ptr::null(),
                &size,
            )
        } != 0
        {
            return Err(io::Error::last_os_error());
        }
        let master = unsafe { File::from_raw_fd(master_fd) };
        let slave = unsafe { File::from_raw_fd(slave_fd) };
        if unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut reader = master.try_clone()?;
        if unsafe { libc::fcntl(reader.as_raw_fd(), libc::F_SETFD, libc::FD_CLOEXEC) } < 0 {
            return Err(io::Error::last_os_error());
        }
        let mut command = Command::new(program);
        command
            .args(args)
            .env("NO_COLOR", "1")
            .env("PAGER", "cat")
            .env("GIT_PAGER", "cat");
        command.stdin(Stdio::from(slave.try_clone()?));
        command.stdout(Stdio::from(slave.try_clone()?));
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
        let child = command.spawn()?;
        let pgid = child.id() as i32;
        let leader_start = start_time(pgid);
        let (tx, output) = mpsc::sync_channel(128);
        // Чтение ждёт и PTY, и канал пробуждения: потомок, удерживающий PTY, не оставит поток висеть после закрытия экрана.
        std::thread::Builder::new().name("upd-pty".into()).spawn(move || {
            let mut buf = [0u8; 8192];
            loop {
                let mut fds = [
                    libc::pollfd { fd: reader.as_raw_fd(), events: libc::POLLIN, revents: 0 },
                    libc::pollfd { fd: wake_rx.as_raw_fd(), events: libc::POLLIN, revents: 0 },
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
                    Ok(0) | Err(_) => break,
                    Ok(n) if tx.send(buf[..n].to_vec()).is_err() => break,
                    Ok(_) => {}
                }
            }
        })?;
        Ok(Self {
            title: String::new(),
            child,
            master,
            output,
            lines: VecDeque::new(),
            current: String::new(),
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
            wake,
            stages: Vec::new(),
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
                    self.read_done = true;
                    break;
                }
            }
        }
        if self.exit_code.is_none() {
            if let Some(status) = self.child.try_wait()? {
                self.exit_code = Some(
                    status
                        .code()
                        .or_else(|| status.signal().map(|signal| 128 + signal))
                        .unwrap_or(1),
                );
                self.ended_at = Some(Instant::now());
            }
        }
        Ok(chunks)
    }

    pub fn finished(&self) -> Option<i32> {
        self.exit_code
            .filter(|_| self.read_done || self.ended_at.is_some_and(|t| t.elapsed().as_secs() >= 1))
    }

    pub fn elapsed_secs(&self) -> u64 {
        self.started.elapsed().as_secs()
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
            self.master.write_all(&bytes)?;
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
        self.raw.extend(bytes);
        while self.raw.len() > MAX_RAW {
            self.raw.pop_front();
        }
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
                if self.csi.len() < 64 {
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
                self.escape = if ch == '\\' {
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
        }
        match ch {
            '\x1b' => self.escape = Escape::Start,
            '\r' => self.pending_cr = true,
            '\n' => self.push_line(),
            '\x08' => {
                self.current.pop();
            }
            '\t' => self.current.push_str("    "),
            c if !c.is_control() && self.current.len() < 16_384 => self.current.push(c),
            _ => {}
        }
    }

    fn push_line(&mut self) {
        if let Some(stage) = parse_stage(&self.current) {
            self.stages.push(stage);
        }
        self.lines.push_back(std::mem::take(&mut self.current));
        if self.lines.len() > MAX_LINES {
            self.lines.pop_front();
        }
    }
}

impl ProcessSession {
    /// Группа процессов всё ещё наша. Пока лидер не пожат, его номер занят. После wait номер группы не выдаётся
    /// новому процессу, пока в группе есть живые потомки; если же /proc/PGID принадлежит процессу
    /// с другим временем старта, группа опустела и номер занят чужим — сигнал слать нельзя.
    fn group_is_ours(&self) -> bool {
        if self.exit_code.is_none() {
            return true;
        }
        match start_time(self.pgid) {
            Some(t) => Some(t) == self.leader_start,
            None => true,
        }
    }
}

impl Drop for ProcessSession {
    fn drop(&mut self) {
        // поток чтения закрывает свою копию PTY, даже если потомок команды ещё держит терминал
        unsafe { libc::send(self.wake.as_raw_fd(), [1u8].as_ptr() as *const libc::c_void, 1, libc::MSG_NOSIGNAL | libc::MSG_DONTWAIT) };
        // лидер сеанса и оставшиеся потомки — и тогда, когда код выхода уже записан
        if self.pgid > 0 && self.group_is_ours() {
            unsafe { libc::kill(-self.pgid, libc::SIGHUP) };
            // команда уже завершилась: оставшиеся в группе потомки (в том числе игнорирующие SIGHUP) завершаются
            if self.exit_code.is_some() {
                unsafe { libc::kill(-self.pgid, libc::SIGTERM) };
            }
        }
    }
}

#[cfg(test)]
fn spawn_with(program: &str, args: &[String], rows: u16, cols: u16) -> io::Result<ProcessSession> {
    ProcessSession::spawn_program(std::ffi::OsStr::new(program), args, rows, cols)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stage_lines_are_parsed() {
        assert_eq!(parse_stage("[3/6] Загрузка"), Some((3, 6, "Загрузка".into())));
        assert_eq!(parse_stage("  [1/6] Mirrors  "), Some((1, 6, "Mirrors".into())));
        assert_eq!(parse_stage("[7/6] x"), None);
        assert_eq!(parse_stage("[a/b] x"), None);
        assert_eq!(parse_stage("text"), None);
    }

    fn threads() -> usize {
        std::fs::read_dir("/proc/self/task").map(|d| d.count()).unwrap_or(0)
    }

    /// B06: команда завершилась, её потомок держит PTY; закрытие сессии освобождает поток чтения и убивает потомка.
    #[test]
    fn b06_closing_session_releases_reader_and_group() {
        let _iso = crate::common::contract_fixtures::isolation_lock();
        let dir = std::env::temp_dir().join(format!("upd-pty-{}", std::process::id()));
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
        let child: i32 = std::fs::read_to_string(&pidfile).unwrap().trim().parse().unwrap();
        assert!(std::path::Path::new(&format!("/proc/{child}")).exists(), "потомок жив и держит PTY");
        let before = threads();
        drop(s);
        let t0 = Instant::now();
        while (std::path::Path::new(&format!("/proc/{child}/stat")).exists()
            && std::fs::read_to_string(format!("/proc/{child}/stat")).map(|s| !s.contains(") Z")).unwrap_or(false)
            || threads() >= before)
            && t0.elapsed().as_secs() < 5
        {
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        let gone = std::fs::read_to_string(format!("/proc/{child}/stat")).map(|s| s.contains(") Z")).unwrap_or(true);
        assert!(gone, "потомок завершён сигналом группе");
        assert!(threads() < before, "поток чтения PTY завершился");
        let _ = std::fs::remove_dir_all(&dir);
    }

    /// Закрытие сессии, когда поток чтения уже вышел: при SIGPIPE по умолчанию (как в main) процесс не погибает.
    #[test]
    fn b06_drop_after_reader_exit_does_not_raise_sigpipe() {
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
    fn b06_reused_pid_is_not_our_group() {
        let mut s = spawn_with("true", &[], 24, 80).unwrap();
        let t0 = Instant::now();
        while s.finished().is_none() && t0.elapsed().as_secs() < 10 {
            let _ = s.poll();
            std::thread::sleep(std::time::Duration::from_millis(20));
        }
        assert!(s.group_is_ours() || start_time(s.pgid).is_some());
        // будто номер лидера достался другому процессу: время старта не совпадает — сигнала нет
        s.pgid = std::process::id() as i32;
        s.leader_start = Some(u64::MAX);
        assert!(!s.group_is_ours());
        s.pgid = 0;
    }
}
