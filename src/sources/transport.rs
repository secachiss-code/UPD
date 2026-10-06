//! Bounded HTTP fetch for [`RequestSpec`] (H.04 / ureq 2.12 variant B).
//!
//! DNS runs in a side thread with `recv_timeout`; connect/TLS/body share the remaining
//! deadline. Client errors are never formatted: only [`FetchFailure`] leaves this module.

use super::negotiation::{FetchFailure, HttpResponse, RequestSpec};
use std::error::Error;
use std::io::{self, Read};
use std::net::{SocketAddr, ToSocketAddrs};
use std::sync::mpsc;
use std::time::{Duration, Instant};
use ureq::{Agent, AgentBuilder, ErrorKind, OrAnyStatus, RedirectAuthHeaders, Resolver};

#[cfg(test)]
use std::sync::{Arc, Mutex};
use url::Url;

/// HTTP adapter that enforces negotiation fetch contracts.
#[derive(Clone, Default)]
pub struct FetchTransport {
    proxy: Option<(u16, bool)>,
    /// Extra DNS wait inserted before lookup (tests only).
    #[cfg(test)]
    resolve_delay: Arc<Mutex<Option<Duration>>>,
}

impl FetchTransport {
    pub fn new() -> Self {
        Self {
            proxy: None,
            #[cfg(test)]
            resolve_delay: Arc::new(Mutex::new(None)),
        }
    }

    /// Route via loopback HTTP CONNECT proxy (`127.0.0.1` or `[::1]`), same shape as `vpn.rs`.
    pub fn with_loopback_proxy(port: u16, ipv6: bool) -> Self {
        Self {
            proxy: Some((port, ipv6)),
            #[cfg(test)]
            resolve_delay: Arc::new(Mutex::new(None)),
        }
    }

    #[cfg(test)]
    pub fn set_resolve_delay(&self, delay: Duration) {
        *self.resolve_delay.lock().unwrap() = Some(delay);
    }

    pub fn fetch(&self, spec: RequestSpec<'_>) -> Result<HttpResponse, FetchFailure> {
        let deadline = Instant::now() + spec.request_timeout();
        let url = spec.endpoint().expose_url();
        let parsed = Url::parse(url).map_err(|_| FetchFailure::Transport)?;
        let started_https = parsed.scheme() == "https";
        self.fetch_url(
            &parsed,
            spec.user_agent().expose_value(),
            deadline,
            spec.max_body_bytes(),
            spec.max_redirects(),
            started_https,
            0,
        )
    }

    #[allow(clippy::too_many_arguments)] // per-hop recursion state shares one deadline
    fn fetch_url(
        &self,
        url: &Url,
        user_agent: &str,
        deadline: Instant,
        max_body_bytes: usize,
        max_redirects: u8,
        started_https: bool,
        hops: u8,
    ) -> Result<HttpResponse, FetchFailure> {
        let remaining = remaining_until(deadline);
        if remaining.is_zero() {
            return Err(FetchFailure::Timeout);
        }

        let resolver = BoundedResolver {
            deadline,
            #[cfg(test)]
            delay: self.resolve_delay.clone(),
        };
        let agent = build_agent(remaining, resolver, self.proxy)?;
        let response = agent
            .get(url.as_str())
            .set("User-Agent", user_agent)
            .set("Accept-Encoding", "identity")
            .call()
            .or_any_status()
            .map_err(|err| classify_transport(&err))?;

        let status = response.status();
        if (300..400).contains(&status) {
            if max_redirects == 0 {
                let body = read_body_bounded(response, max_body_bytes, deadline)?;
                return Ok(HttpResponse::new(status, body));
            }
            if hops >= max_redirects {
                return Err(FetchFailure::Redirect);
            }
            let location = response.header("location").ok_or(FetchFailure::Redirect)?;
            let next = resolve_redirect(url, location, started_https)?;
            // Consume redirect body without storing it.
            let _ = read_body_bounded(response, max_body_bytes, deadline);
            return self.fetch_url(
                &next,
                user_agent,
                deadline,
                max_body_bytes,
                max_redirects,
                started_https,
                hops + 1,
            );
        }

        let body = read_body_bounded(response, max_body_bytes, deadline)?;
        Ok(HttpResponse::new(status, body))
    }
}

struct BoundedResolver {
    deadline: Instant,
    #[cfg(test)]
    delay: Arc<Mutex<Option<Duration>>>,
}

impl Resolver for BoundedResolver {
    fn resolve(&self, netloc: &str) -> io::Result<Vec<SocketAddr>> {
        let remaining = remaining_until(self.deadline);
        if remaining.is_zero() {
            return Err(io::Error::from(io::ErrorKind::TimedOut));
        }
        let netloc = netloc.to_owned();
        #[cfg(test)]
        let delay = *self.delay.lock().unwrap();
        #[cfg(not(test))]
        let delay: Option<Duration> = None;
        let (tx, rx) = mpsc::channel();
        std::thread::spawn(move || {
            if let Some(delay) = delay {
                std::thread::sleep(delay);
            }
            let result = netloc
                .to_socket_addrs()
                .map(|iter| iter.collect::<Vec<_>>());
            let _ = tx.send(result);
        });
        match rx.recv_timeout(remaining) {
            Ok(Ok(addrs)) => Ok(addrs),
            Ok(Err(err)) => Err(err),
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                Err(io::Error::from(io::ErrorKind::BrokenPipe))
            }
            Err(mpsc::RecvTimeoutError::Timeout) => Err(io::Error::from(io::ErrorKind::TimedOut)),
        }
    }
}

fn remaining_until(deadline: Instant) -> Duration {
    deadline.saturating_duration_since(Instant::now())
}

fn build_agent(
    remaining: Duration,
    resolver: BoundedResolver,
    proxy: Option<(u16, bool)>,
) -> Result<Agent, FetchFailure> {
    let mut builder = AgentBuilder::new()
        .resolver(resolver)
        .timeout(remaining)
        .timeout_connect(remaining)
        .timeout_read(remaining)
        .timeout_write(remaining)
        // ureq must not follow redirects itself: every hop goes through `resolve_redirect`
        // (https→http refusal) and the shared hop/deadline/body limits in `fetch_url`.
        .redirects(0)
        .redirect_auth_headers(RedirectAuthHeaders::Never);
    if let Some((port, ipv6)) = proxy {
        let host = if ipv6 { "[::1]" } else { "127.0.0.1" };
        let proxy_url = format!("http://{host}:{port}");
        let px = ureq::Proxy::new(&proxy_url).map_err(|_| FetchFailure::Transport)?;
        builder = builder.proxy(px);
    }
    Ok(builder.build())
}

fn resolve_redirect(base: &Url, location: &str, started_https: bool) -> Result<Url, FetchFailure> {
    let next = base.join(location).map_err(|_| FetchFailure::Redirect)?;
    if started_https && next.scheme() == "http" {
        return Err(FetchFailure::Redirect);
    }
    Ok(next)
}

fn read_body_bounded(
    response: ureq::Response,
    max_body_bytes: usize,
    deadline: Instant,
) -> Result<Vec<u8>, FetchFailure> {
    let remaining = remaining_until(deadline);
    if remaining.is_zero() {
        return Err(FetchFailure::Timeout);
    }
    let limit = max_body_bytes as u64;
    if response
        .header("content-length")
        .and_then(|v| v.parse::<u64>().ok())
        .is_some_and(|len| len > limit)
    {
        return Err(FetchFailure::Transport);
    }
    let mut body = Vec::new();
    let mut reader = response.into_reader();
    let mut chunk = [0u8; 8192];
    while body.len() <= max_body_bytes {
        if remaining_until(deadline).is_zero() {
            return Err(FetchFailure::Timeout);
        }
        let read = match reader.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => n,
            Err(err) if err.kind() == io::ErrorKind::TimedOut => {
                return Err(FetchFailure::Timeout);
            }
            Err(_) => return Err(FetchFailure::Transport),
        };
        body.extend_from_slice(&chunk[..read]);
        if body.len() > max_body_bytes {
            return Err(FetchFailure::Transport);
        }
    }
    Ok(body)
}

fn classify_transport(transport: &ureq::Transport) -> FetchFailure {
    if transport_timed_out(transport) {
        return FetchFailure::Timeout;
    }
    match transport.kind() {
        ErrorKind::TooManyRedirects => FetchFailure::Redirect,
        ErrorKind::InsecureRequestHttpsOnly => FetchFailure::Redirect,
        ErrorKind::Io | ErrorKind::ConnectionFailed if transport_tls(transport) => {
            FetchFailure::Tls
        }
        ErrorKind::Dns if transport_timed_out(transport) => FetchFailure::Timeout,
        ErrorKind::Dns => FetchFailure::Transport,
        _ => FetchFailure::Transport,
    }
}

fn transport_timed_out(transport: &ureq::Transport) -> bool {
    if let Some(message) = transport.message()
        && (message.contains("timeout") || message.contains("Timeout"))
    {
        return true;
    }
    if let Some(source) = transport.source() {
        return error_chain_timed_out(source);
    }
    false
}

fn transport_tls(transport: &ureq::Transport) -> bool {
    if let Some(message) = transport.message() {
        let lower = message.to_ascii_lowercase();
        if lower.contains("tls") || lower.contains("certificate") || lower.contains("cert") {
            return true;
        }
    }
    false
}

fn error_chain_timed_out(err: &(dyn Error + 'static)) -> bool {
    let mut current: Option<&(dyn Error + 'static)> = Some(err);
    while let Some(entry) = current {
        if let Some(io) = entry.downcast_ref::<io::Error>()
            && io.kind() == io::ErrorKind::TimedOut
        {
            return true;
        }
        current = entry.source();
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sources::negotiation::{
        ConfiguredEndpoint, NegotiationError, UserAgent, test_request_spec,
    };
    use std::io::{BufRead, BufReader, Write};
    use std::net::TcpListener;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::thread;
    use std::time::Duration;

    const SECRET: &str = "SECRET-URL-MARKER-H04";

    fn assert_no_secret(text: &str) {
        assert!(
            !text.contains(SECRET),
            "secret marker leaked into output: {text}"
        );
    }

    fn local_server(
        handler: impl Fn(&mut std::net::TcpStream) + Send + 'static,
    ) -> (u16, thread::JoinHandle<()>) {
        let listener = TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().unwrap().port();
        let handle = thread::spawn(move || {
            if let Ok((mut stream, _)) = listener.accept() {
                handler(&mut stream);
            }
        });
        (port, handle)
    }

    fn leaked_spec(url: &str, timeout: Duration, max_body: usize) -> RequestSpec<'static> {
        let endpoint = ConfiguredEndpoint::new(url, true).expect("endpoint");
        let agent = UserAgent::new("test-agent/1.0").expect("ua");
        let endpoint: &'static ConfiguredEndpoint = Box::leak(Box::new(endpoint));
        let agent: &'static UserAgent = Box::leak(Box::new(agent));
        test_request_spec(endpoint, agent, timeout, max_body)
    }

    #[test]
    fn slow_dns_returns_before_budget_without_accept() {
        let accepted = Arc::new(AtomicBool::new(false));
        let accepted_flag = Arc::clone(&accepted);
        let (port, _server) = local_server(move |_| {
            accepted_flag.store(true, Ordering::SeqCst);
        });
        let transport = FetchTransport::new();
        transport.set_resolve_delay(Duration::from_secs(3));
        let url = format!("http://127.0.0.1:{port}/{SECRET}");
        let spec = leaked_spec(&url, Duration::from_millis(200), 4096);
        let started = Instant::now();
        let err = transport.fetch(spec).unwrap_err();
        assert_eq!(err, FetchFailure::Timeout);
        assert!(
            started.elapsed() < Duration::from_millis(500),
            "elapsed {:?}",
            started.elapsed()
        );
        assert!(!accepted.load(Ordering::SeqCst));
    }

    #[test]
    fn slow_loris_hits_timeout() {
        let (port, server) = local_server(|stream| {
            stream
                .write_all(
                    b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\nConnection: close\r\n\r\n",
                )
                .unwrap();
            thread::sleep(Duration::from_secs(5));
        });
        let transport = FetchTransport::new();
        let url = format!("http://127.0.0.1:{port}/");
        let spec = leaked_spec(&url, Duration::from_millis(300), 4096);
        let started = Instant::now();
        assert_eq!(transport.fetch(spec).unwrap_err(), FetchFailure::Timeout);
        assert!(started.elapsed() < Duration::from_secs(2));
        let _ = server.join();
    }

    #[test]
    fn oversized_body_is_rejected() {
        let body = "x".repeat(64);
        let (port, server) = local_server(move |stream| {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                body.len(),
                body
            );
            stream.write_all(response.as_bytes()).unwrap();
        });
        let transport = FetchTransport::new();
        let url = format!("http://127.0.0.1:{port}/");
        let spec = leaked_spec(&url, Duration::from_secs(2), 32);
        assert_eq!(transport.fetch(spec).unwrap_err(), FetchFailure::Transport);
        let _ = server.join();
    }

    #[test]
    fn redirect_chain_with_limit_zero_returns_status() {
        let (port, server) = local_server(|stream| {
            let response = "HTTP/1.1 302 Found\r\nLocation: /final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(response.as_bytes()).unwrap();
        });
        let transport = FetchTransport::new();
        let url = format!("http://127.0.0.1:{port}/start");
        let spec = leaked_spec(&url, Duration::from_secs(2), 4096);
        let response = transport.fetch(spec).expect("fetch");
        assert_eq!(response.status(), 302);
        let _ = server.join();
    }

    #[test]
    fn fetch_failure_display_hides_secret_url() {
        let transport = FetchTransport::new();
        let url = format!("https://127.0.0.1:9/{SECRET}");
        let spec = leaked_spec(&url, Duration::from_millis(200), 4096);
        let err = transport.fetch(spec).unwrap_err();
        let display = NegotiationError::Fetch(err).to_string();
        assert_no_secret(&display);
        assert_no_secret(&format!("{err:?}"));
    }

    #[test]
    fn happy_path_reads_body() {
        let (port, server) = local_server(|stream| {
            let mut reader = BufReader::new(stream.try_clone().unwrap());
            loop {
                let mut line = String::new();
                reader.read_line(&mut line).unwrap();
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }
            stream
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
                .unwrap();
        });
        let transport = FetchTransport::new();
        let url = format!("http://127.0.0.1:{port}/");
        let spec = leaked_spec(&url, Duration::from_secs(2), 4096);
        let response = transport.fetch(spec).expect("ok");
        assert_eq!(response.status(), 200);
        assert_eq!(response.body(), b"ok");
        let _ = server.join();
    }

    #[test]
    fn https_downgrade_redirect_is_refused() {
        let (port, server) = local_server(|stream| {
            let response = "HTTP/1.1 302 Found\r\nLocation: http://127.0.0.1:1/downgrade\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";
            stream.write_all(response.as_bytes()).unwrap();
        });
        let transport = FetchTransport::new();
        let url = format!("http://127.0.0.1:{port}/");
        let parsed = Url::parse(&url).unwrap();
        let agent = UserAgent::new("test/1").unwrap();
        let agent: &'static UserAgent = Box::leak(Box::new(agent));
        let err = transport
            .fetch_url(
                &parsed,
                agent.expose_value(),
                Instant::now() + Duration::from_secs(2),
                4096,
                1,
                true,
                0,
            )
            .unwrap_err();
        assert_eq!(err, FetchFailure::Redirect);
        let _ = server.join();
    }
}
