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
    let (master, slave) = open_pty_pair(40, 120)?;
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
    cmd.stdin(Stdio::from(dup_cloexec(&slave)?)).stdout(Stdio::from(dup_cloexec(&slave)?)).stderr(Stdio::from(slave));
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

type PendingInput = (Vec<u8>, usize, Instant, SyncSender<Result<(), String>>);

fn runner_loop(shared: &Shared, operation_id: &OperationId, resources: &mut RunnerResources) -> i32 {
    let mut exit_code = None;
    let mut forced_error = false;
    let mut drain_deadline = None;
    let mut terminate_deadline = None;
    let mut cancel_count = 0u32;
    let mut reader_ended = false;
    let mut commands_disconnected = false;
    let mut pending_input: Option<PendingInput> = None;

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

        if let Some(code) = exit_code
            && (reader_ended || drain_deadline.is_some_and(|deadline| Instant::now() >= deadline)) {
            if resources.join_reader().is_err() {
                forced_error = true;
            }
            return if forced_error { 1 } else { code };
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

