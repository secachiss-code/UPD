use cm::sources::{ImportFormat, ParserError, parse_native};

const PIN: &str = "v1.19.32";

fn parse(body: &str) -> Result<cm::sources::ParsedSource, ParserError> {
    parse_native(PIN, ImportFormat::MihomoJson, body.as_bytes())
}

fn node(extra: &str) -> String {
    format!(
        r#"{{"proxies":[{{"name":"n","type":"vless","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000"{extra}}}]}}"#
    )
}

#[test]
fn accepts_ws_http_h2_grpc_and_xhttp() {
    parse(&node(
        r#","network":"ws","ws-opts":{"path":"/v","headers":{"Host":"edge.example"},"max-early-data":2048}"#,
    ))
    .unwrap();
    parse(&node(r#","network":"http","http-opts":{"method":"GET","path":["/"],"headers":{"Host":["edge.example"]}}"#))
        .unwrap();
    parse(&node(r#","network":"h2","tls":true,"h2-opts":{"host":["edge.example"],"path":"/h2"}"#)).unwrap();
    parse(&node(
        r#","network":"grpc","tls":true,"grpc-opts":{"grpc-service-name":"GunService"}"#,
    ))
    .unwrap();
    parse(&node(r#","network":"xhttp","xhttp-opts":{"path":"/x","host":"edge.example","mode":"auto"}"#))
        .unwrap();
}

#[test]
fn rejects_bad_paths_types_and_header_injection() {
    assert!(matches!(
        parse(&node(r#","network":"ws","ws-opts":{"path":"vless","max-early-data":"2048"}"#)).unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(
            r#","network":"ws","ws-opts":{"v2ray-http-upgrade":true,"v2ray-http-upgrade-fast-open":true}"#
        ))
        .unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(r#","network":"http","http-opts":{"path":[],"headers":{"X":["a\r\nB: c"]}}"#)).unwrap_err(),
        ParserError::InvalidNode { .. }
    ));
    assert!(matches!(
        parse(&node(r#","network":"grpc","grpc-opts":{"grpc-user-agent":"x"}"#)).unwrap_err(),
        ParserError::UnsupportedNodeField { .. }
    ));
    let vmess = r#"{"proxies":[{"name":"n","type":"vmess","server":"edge.example","port":443,"uuid":"123e4567-e89b-12d3-a456-426614174000","cipher":"auto","alterId":0,"network":"xhttp"}]}"#;
    assert!(matches!(
        parse(vmess).unwrap_err(),
        ParserError::UnsupportedTransport { .. }
    ));
}

/// I03.T05.c: `negotiate_source` through `FetchTransport` against a loopback HTTP server.
/// Classifications match the mock negotiation tests: HTML retries the next User-Agent,
/// a redirect is terminal, a timeout is `Fetch(Timeout)` inside the deadline, an oversized
/// body is a transport refusal and is not retried.
mod local_fetch {
    use cm::sources::pipeline::{FetchSourceError, negotiate_source};
    use cm::sources::{
        ConfiguredEndpoint, FetchFailure, FetchTransport, HttpResponse, NegotiationError,
        NegotiationPolicy, UserAgent,
    };
    use std::io::{Read, Write};
    use std::net::{TcpListener, TcpStream};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::thread::{self, JoinHandle};
    use std::time::{Duration, Instant};

    const PATH_MARKER: &str = "fixture-token";
    const BODY_MARKER: &str = "pw-local-1";

    struct Hit {
        path: String,
        user_agent: String,
    }

    struct Server {
        port: u16,
        stop: Arc<AtomicBool>,
        hits: Arc<Mutex<Vec<Hit>>>,
        thread: Option<JoinHandle<()>>,
    }

    impl Server {
        fn spawn(
            handler: impl Fn(&AtomicBool, &Hit, &mut TcpStream) + Send + Sync + 'static,
        ) -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").expect("bind 127.0.0.1");
            let port = listener.local_addr().expect("port").port();
            listener.set_nonblocking(true).expect("nonblocking");
            let stop = Arc::new(AtomicBool::new(false));
            let hits = Arc::new(Mutex::new(Vec::new()));
            let stop_flag = Arc::clone(&stop);
            let hit_log = Arc::clone(&hits);
            let handler = Arc::new(handler);
            let thread = thread::spawn(move || {
                while !stop_flag.load(Ordering::Acquire) {
                    match listener.accept() {
                        Ok((mut stream, _)) => {
                            let _ = stream.set_read_timeout(Some(Duration::from_secs(2)));
                            let Ok(hit) = read_hit(&mut stream) else {
                                continue;
                            };
                            hit_log.lock().expect("hits").push(Hit {
                                path: hit.path.clone(),
                                user_agent: hit.user_agent.clone(),
                            });
                            handler(&stop_flag, &hit, &mut stream);
                        }
                        Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                            thread::sleep(Duration::from_millis(5));
                        }
                        Err(_) => break,
                    }
                }
            });
            Self {
                port,
                stop,
                hits,
                thread: Some(thread),
            }
        }

        fn paths(&self) -> Vec<String> {
            self.hits
                .lock()
                .expect("hits")
                .iter()
                .map(|hit| hit.path.clone())
                .collect()
        }
    }

    impl Drop for Server {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::Release);
            if let Some(thread) = self.thread.take() {
                let _ = thread.join();
            }
        }
    }

    fn read_hit(stream: &mut TcpStream) -> std::io::Result<Hit> {
        let mut buf = Vec::new();
        let mut byte = [0u8; 1];
        while buf.len() < 8192 {
            if stream.read(&mut byte)? == 0 {
                break;
            }
            buf.push(byte[0]);
            if buf.ends_with(b"\r\n\r\n") {
                break;
            }
        }
        let text = String::from_utf8_lossy(&buf);
        let mut lines = text.split("\r\n");
        let request = lines.next().unwrap_or("");
        let path = request.split_whitespace().nth(1).unwrap_or("/").to_owned();
        let mut user_agent = String::new();
        for line in lines {
            let Some((name, value)) = line.split_once(':') else {
                continue;
            };
            if name.eq_ignore_ascii_case("user-agent") {
                user_agent = value.trim().to_owned();
            }
        }
        Ok(Hit { path, user_agent })
    }

    fn write_response(stream: &mut TcpStream, status: &str, extra: &str, body: &[u8]) {
        let head = format!(
            "HTTP/1.1 {status}\r\nConnection: close\r\nContent-Length: {}\r\n{extra}\r\n",
            body.len()
        );
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
    }

    fn endpoint(port: u16, path: &str) -> ConfiguredEndpoint {
        ConfiguredEndpoint::new(format!("http://127.0.0.1:{port}{path}"), true).expect("endpoint")
    }

    fn agents(names: &[&str]) -> Vec<UserAgent> {
        names
            .iter()
            .map(|name| UserAgent::new(*name).expect("ua"))
            .collect()
    }

    fn proxy_body() -> Vec<u8> {
        format!(
            r#"{{"proxies":[{{"name":"one","type":"ss","server":"edge.synthetic.invalid","port":443,"cipher":"aes-128-gcm","password":"{BODY_MARKER}"}}]}}"#
        )
        .into_bytes()
    }

    fn negotiation_error(error: FetchSourceError) -> NegotiationError {
        match error {
            FetchSourceError::Negotiation(error) => error,
            other => panic!("expected negotiation error, got {other}"),
        }
    }

    fn assert_hidden(text: &str) {
        assert!(!text.contains(PATH_MARKER), "{text}");
        assert!(!text.contains(BODY_MARKER), "{text}");
    }

    #[test]
    fn user_agent_bodies_match_the_mock_retry() {
        let html = b"<html><body>stub</body></html>".to_vec();
        let ok = proxy_body();
        let html_for_server = html.clone();
        let ok_for_server = ok.clone();
        let server = Server::spawn(move |_stop, hit, stream| {
            let body = if hit.user_agent == "cm-html" {
                html_for_server.as_slice()
            } else {
                ok_for_server.as_slice()
            };
            write_response(stream, "200 OK", "", body);
        });
        let path = format!("/{PATH_MARKER}/feed");
        let endpoint = endpoint(server.port, &path);
        let preferred = agents(&["cm-html", "cm-json"]);
        let policy = NegotiationPolicy {
            per_request_timeout: Duration::from_secs(2),
            total_budget: Duration::from_secs(5),
            ..NegotiationPolicy::default()
        };
        let live = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |request| FetchTransport::new().fetch(request),
        )
        .expect("second UA body is a source");
        let mock = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |request| {
                let body = if request.user_agent().expose_value() == "cm-html" {
                    html.clone()
                } else {
                    ok.clone()
                };
                Ok(HttpResponse::new(200, body))
            },
        )
        .expect("mock");
        assert_eq!(live.actual_user_agent().expose_value(), "cm-json");
        assert_eq!(mock.actual_user_agent().expose_value(), "cm-json");
        assert_eq!(live.request_count(), 2);
        assert_eq!(mock.request_count(), 2);
        assert_eq!(live.source_body_sha256(), mock.source_body_sha256());
        assert_eq!(server.paths(), vec![path.clone(), path]);
        assert_hidden(&format!("{live:?}"));
    }

    #[test]
    fn redirect_chain_is_terminal_like_the_mock() {
        let server = Server::spawn(|_stop, hit, stream| {
            let next = match hit.path.as_str() {
                path if path.ends_with("/chain") => "/hop-2",
                path if path.ends_with("/hop-2") => "/hop-3",
                _ => "/done",
            };
            write_response(
                stream,
                "302 Found",
                &format!("Location: {next}\r\n"),
                b"",
            );
        });
        let path = format!("/{PATH_MARKER}/chain");
        let endpoint = endpoint(server.port, &path);
        let preferred = agents(&["cm-one", "cm-two"]);
        let policy = NegotiationPolicy {
            per_request_timeout: Duration::from_secs(2),
            total_budget: Duration::from_secs(5),
            ..NegotiationPolicy::default()
        };
        let live = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |request| FetchTransport::new().fetch(request),
        )
        .unwrap_err();
        let shown = live.to_string();
        let mock = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |_| Ok(HttpResponse::new(302, Vec::new())),
        )
        .unwrap_err();
        assert_eq!(negotiation_error(live), NegotiationError::RedirectRejected);
        assert_eq!(negotiation_error(mock), NegotiationError::RedirectRejected);
        assert_eq!(server.paths(), vec![path]);
        assert_hidden(&shown);
    }

    #[test]
    fn slow_response_returns_before_the_server_finishes() {
        let server = Server::spawn(|stop, _hit, stream| {
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(5) && !stop.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(20));
            }
            write_response(stream, "200 OK", "", b"late");
        });
        let path = format!("/{PATH_MARKER}/slow");
        let endpoint = endpoint(server.port, &path);
        let preferred = agents(&["cm-one", "cm-two"]);
        let policy = NegotiationPolicy {
            per_request_timeout: Duration::from_millis(400),
            total_budget: Duration::from_secs(5),
            ..NegotiationPolicy::default()
        };
        let started = Instant::now();
        let live = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |request| FetchTransport::new().fetch(request),
        )
        .unwrap_err();
        let elapsed = started.elapsed();
        let mock = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |_| Err(FetchFailure::Timeout),
        )
        .unwrap_err();
        let live_error = negotiation_error(live);
        assert_eq!(live_error, NegotiationError::Fetch(FetchFailure::Timeout));
        assert_eq!(
            negotiation_error(mock),
            NegotiationError::Fetch(FetchFailure::Timeout)
        );
        assert!(
            elapsed < Duration::from_secs(2),
            "deadline 400ms was not kept, elapsed {elapsed:?}"
        );
        assert_eq!(server.paths().len(), 1, "timeout must not try the next UA");
        assert_hidden(&live_error.to_string());
    }

    #[test]
    fn oversized_body_is_not_retried() {
        let server = Server::spawn(|_stop, _hit, stream| {
            let body = vec![b'x'; 64];
            write_response(stream, "200 OK", "", &body);
        });
        let path = format!("/{PATH_MARKER}/huge");
        let endpoint = endpoint(server.port, &path);
        let preferred = agents(&["cm-one", "cm-two"]);
        let policy = NegotiationPolicy {
            max_body_bytes: 32,
            per_request_timeout: Duration::from_secs(2),
            total_budget: Duration::from_secs(5),
            ..NegotiationPolicy::default()
        };
        let live = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |request| FetchTransport::new().fetch(request),
        )
        .unwrap_err();
        let shown = live.to_string();
        let mock = negotiate_source(
            &endpoint,
            "1.19.32",
            &preferred,
            &[],
            None,
            &policy,
            |_| Err(FetchFailure::Transport),
        )
        .unwrap_err();
        assert_eq!(
            negotiation_error(live),
            NegotiationError::Fetch(FetchFailure::Transport)
        );
        assert_eq!(
            negotiation_error(mock),
            NegotiationError::Fetch(FetchFailure::Transport)
        );
        assert_eq!(server.paths(), vec![path]);
        assert_hidden(&shown);
    }
}
