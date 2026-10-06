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
    // SAFETY: takes no pointers and accesses no memory.
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
    // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
    let flags = unsafe { libc::fcntl(master.as_raw_fd(), libc::F_GETFL) };
    // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
    if flags < 0 || unsafe { libc::fcntl(master.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) } < 0 {
        // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
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
            // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
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
            // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
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
            // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
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
        // SAFETY: sending a signal accesses no memory; the target group is a child we have not reaped yet.
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


// ======================= обработка запросов =======================

fn poll_until(fd: i32, events: i16, deadline: Instant) -> std::io::Result<i16> {
    loop {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "deadline exceeded"));
        }
        let timeout = remaining.as_millis().saturating_add(1).min(i32::MAX as u128) as i32;
        let mut descriptor = libc::pollfd { fd, events, revents: 0 };
        // SAFETY: the pollfd pointer and count describe live entries for the whole call.
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
    if !action.is_empty()
        && let Auth::No(why) = authorize(p, action) {
        let _ = send_server(&stream, &Reply { denied: true, ..Reply::err(why) }, REPLY_WRITE_TIMEOUT);
        return;
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
        // SAFETY: the pointer and length describe a live slice; MSG_NOSIGNAL prevents SIGPIPE.
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
        // SAFETY: the pollfd pointer and count describe live entries for the whole call.
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
        // SAFETY: reads at most one byte into a live stack byte.
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
        // SAFETY: LISTEN_PID/LISTEN_FDS name this process, so systemd passed the listener as fd 3 and nothing else owns it.
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
        // SAFETY: the pollfd pointer and count describe live entries for the whole call.
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
    // SAFETY: plain C struct; the all-zero bit pattern is a valid value.
    let mut address: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    if bytes.len() >= address.sun_path.len() || bytes.contains(&0) {
        return Err(std::io::Error::new(std::io::ErrorKind::InvalidInput, "invalid helper socket path"));
    }
    address.sun_family = libc::AF_UNIX as _;
    for (target, byte) in address.sun_path.iter_mut().zip(bytes) { *target = *byte as _; }
    // SAFETY: socket takes no pointers and returns a new fd or -1.
    let raw = unsafe { libc::socket(libc::AF_UNIX, libc::SOCK_STREAM | libc::SOCK_CLOEXEC | libc::SOCK_NONBLOCK, 0) };
    if raw < 0 { return Err(std::io::Error::last_os_error()); }
    // SAFETY: the fd was just returned by the syscall above, checked >= 0, and nothing else owns it.
    let fd = unsafe { OwnedFd::from_raw_fd(raw) };
    loop {
        if Instant::now() >= deadline { return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "helper connect deadline exceeded")); }
        // SAFETY: address is a fully initialized sockaddr_un and the length is its size.
        let rc = unsafe { libc::connect(fd.as_raw_fd(), &address as *const _ as *const libc::sockaddr, std::mem::size_of_val(&address) as _) };
        if rc == 0 { break; }
        let error = std::io::Error::last_os_error();
        match error.raw_os_error() {
            Some(libc::EINPROGRESS) => {
                poll_until(fd.as_raw_fd(), libc::POLLOUT | libc::POLLERR | libc::POLLHUP, deadline)?;
                let mut error = 0i32;
                let mut len = std::mem::size_of_val(&error) as libc::socklen_t;
                // SAFETY: error and len describe a live c_int buffer.
                if unsafe { libc::getsockopt(fd.as_raw_fd(), libc::SOL_SOCKET, libc::SO_ERROR, &mut error as *mut _ as _, &mut len) } < 0 { return Err(std::io::Error::last_os_error()); }
                if error != 0 { return Err(std::io::Error::from_raw_os_error(error)); }
                break;
            }
            // SAFETY: poll with zero descriptors is a plain sleep; the null pointer is never read.
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

