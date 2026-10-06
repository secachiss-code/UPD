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
        let (mut input, slave) = open_pty_pair(24, 80).unwrap();
        // SAFETY: plain C struct; the all-zero bit pattern is a valid value.
        let mut attributes: libc::termios = unsafe { std::mem::zeroed() };
        // SAFETY: attributes is a live termios and the fd is an open terminal.
        assert_eq!(unsafe { libc::tcgetattr(slave.as_raw_fd(), &mut attributes) }, 0);
        // SAFETY: attributes is a live termios and the fd is an open terminal.
        unsafe { libc::cfmakeraw(&mut attributes); }
        // SAFETY: attributes is a live termios and the fd is an open terminal.
        assert_eq!(unsafe { libc::tcsetattr(slave.as_raw_fd(), libc::TCSANOW, &attributes) }, 0);
        // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
        let flags = unsafe { libc::fcntl(input.as_raw_fd(), libc::F_GETFL) };
        // SAFETY: fcntl on an open fd borrowed for the call; F_GETFL/F_SETFL/F_GETFD do not access memory.
        assert_eq!(unsafe { libc::fcntl(input.as_raw_fd(), libc::F_SETFL, flags | libc::O_NONBLOCK) }, 0);
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
