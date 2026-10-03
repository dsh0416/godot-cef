//! Bounded loopback HTTP fixture for browser integration cases.

use serde_json::{Value, json};
use std::collections::HashMap;
use std::error::Error;
use std::io::{self, Read, Write};
use std::net::{Shutdown, SocketAddr, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

const BODY_LIMIT: usize = 65_536;
const HEADER_LIMIT: usize = 16_384;
const EVENT_LIMIT: usize = 1024;
const CLIENT_LIMIT: usize = 16;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(2);
const WRITE_TIMEOUT: Duration = Duration::from_secs(1);
const POLL_INTERVAL: Duration = Duration::from_millis(5);

#[derive(Default)]
struct State {
    runs: Mutex<HashMap<String, Vec<Value>>>,
    clients: Mutex<HashMap<u64, TcpStream>>,
    stopping: AtomicBool,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl State {
    fn close_clients(&self) {
        for stream in lock(&self.clients).values() {
            let _ = stream.shutdown(Shutdown::Both);
        }
    }
}

pub(super) struct FixtureServer {
    origin: String,
    state: Arc<State>,
    worker: Option<JoinHandle<()>>,
}

impl FixtureServer {
    pub(super) fn start(page: Vec<u8>) -> Result<Self, Box<dyn Error>> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        listener.set_nonblocking(true)?;
        let state = Arc::new(State::default());
        let thread_state = Arc::clone(&state);
        let worker = thread::Builder::new()
            .name("gdcef-fixture".into())
            .spawn(move || serve(listener, thread_state, Arc::new(page)))?;
        let server = Self {
            origin: format!("http://{address}"),
            state,
            worker: Some(worker),
        };
        // Verify a real loopback HTTP handshake before launching the browser.
        wait_until_reachable(address)?;
        Ok(server)
    }

    pub(super) fn origin(&self) -> &str {
        &self.origin
    }

    pub(super) fn register(&self, id: &str) {
        lock(&self.state.runs).insert(id.to_owned(), Vec::new());
    }

    pub(super) fn events(&self, id: &str) -> Vec<Value> {
        lock(&self.state.runs).get(id).cloned().unwrap_or_default()
    }
}

impl Drop for FixtureServer {
    fn drop(&mut self) {
        self.state.stopping.store(true, Ordering::Release);
        self.state.close_clients();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

fn serve(listener: TcpListener, state: Arc<State>, page: Arc<Vec<u8>>) {
    let mut workers: Vec<JoinHandle<()>> = Vec::new();
    let mut next_id = 0_u64;
    while !state.stopping.load(Ordering::Acquire) {
        // Reap completed workers; both live sockets and retained JoinHandles
        // stay bounded even when a browser makes many polling requests.
        let mut index = 0;
        while index < workers.len() {
            if workers[index].is_finished() {
                let _ = workers.swap_remove(index).join();
            } else {
                index += 1;
            }
        }
        match listener.accept() {
            Ok((stream, _)) => {
                if state.stopping.load(Ordering::Acquire) || workers.len() >= CLIENT_LIMIT {
                    let _ = stream.shutdown(Shutdown::Both);
                    continue;
                }
                // Explicit nonblocking I/O gives deadlines and cancellation the
                // same behavior on Unix and Winsock, including stalled clients.
                if stream.set_nonblocking(true).is_err() {
                    continue;
                }
                let Ok(tracked) = stream.try_clone() else {
                    continue;
                };
                let id = next_id;
                next_id = next_id.wrapping_add(1);
                lock(&state.clients).insert(id, tracked);
                let client_state = Arc::clone(&state);
                let client_page = Arc::clone(&page);
                match thread::Builder::new()
                    .name("gdcef-fixture-client".into())
                    .spawn(move || {
                        handle_client(stream, &client_state, &client_page);
                        lock(&client_state.clients).remove(&id);
                    }) {
                    Ok(worker) => workers.push(worker),
                    Err(_) => {
                        lock(&state.clients).remove(&id);
                    }
                }
            }
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(POLL_INTERVAL);
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                eprintln!("Loopback fixture accept failed: {error}");
                break;
            }
        }
    }
    // Covers an accept/spawn racing with Drop's first socket shutdown pass.
    state.close_clients();
    for worker in workers {
        let _ = worker.join();
    }
}

struct Request {
    method: String,
    target: String,
    body: Vec<u8>,
}

fn remaining(deadline: Instant, stopping: Option<&AtomicBool>) -> io::Result<Duration> {
    if stopping.is_some_and(|stopping| stopping.load(Ordering::Acquire)) {
        return Err(io::Error::new(
            io::ErrorKind::Interrupted,
            "Fixture is stopping",
        ));
    }
    deadline
        .checked_duration_since(Instant::now())
        .filter(|remaining| !remaining.is_zero())
        .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "HTTP deadline exceeded"))
}

fn timed_read(
    stream: &mut TcpStream,
    bytes: &mut [u8],
    deadline: Instant,
    stopping: Option<&AtomicBool>,
) -> io::Result<usize> {
    loop {
        let remaining = remaining(deadline, stopping)?;
        match stream.read(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "Incomplete HTTP request",
                ));
            }
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(POLL_INTERVAL.min(remaining))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
}

fn read_request(
    stream: &mut TcpStream,
    stopping: &AtomicBool,
) -> Result<Request, (u16, &'static str)> {
    let deadline = Instant::now() + REQUEST_TIMEOUT;
    let mut bytes = Vec::with_capacity(4096);
    let mut buffer = [0_u8; 4096];
    let (offset, method, target, length) = loop {
        let mut headers = [httparse::EMPTY_HEADER; 64];
        let mut parsed = httparse::Request::new(&mut headers);
        match parsed.parse(&bytes) {
            Ok(httparse::Status::Complete(offset)) => {
                if offset > HEADER_LIMIT {
                    return Err((431, "Request headers are too large"));
                }
                let method = parsed.method.ok_or((400, "Missing method"))?.to_owned();
                let target = parsed
                    .path
                    .ok_or((400, "Missing request target"))?
                    .to_owned();
                let mut length = None;
                for header in parsed.headers {
                    // The fixture's browser fetches have known JSON body sizes.
                    // Reject ambiguous/unsupported framing instead of implementing
                    // another general-purpose chunked HTTP parser here.
                    if header.name.eq_ignore_ascii_case("transfer-encoding") {
                        return Err((400, "Transfer-Encoding is not supported"));
                    }
                    if header.name.eq_ignore_ascii_case("content-length") {
                        let value = std::str::from_utf8(header.value)
                            .map_err(|_| (400, "Invalid Content-Length"))?;
                        if length.is_some()
                            || value.is_empty()
                            || !value.bytes().all(|byte| byte.is_ascii_digit())
                        {
                            return Err((400, "Invalid or duplicate Content-Length"));
                        }
                        length = Some(
                            value
                                .parse::<usize>()
                                .map_err(|_| (413, "Report is too large"))?,
                        );
                    }
                }
                break (offset, method, target, length.unwrap_or(0));
            }
            Ok(httparse::Status::Partial) => {
                if bytes.len() >= HEADER_LIMIT {
                    return Err((431, "Request headers are too large"));
                }
                let count = timed_read(stream, &mut buffer, deadline, Some(stopping))
                    .map_err(read_error)?;
                bytes.extend_from_slice(&buffer[..count]);
            }
            Err(httparse::Error::TooManyHeaders) => return Err((431, "Too many request headers")),
            Err(_) => return Err((400, "Malformed HTTP request")),
        }
    };
    if length > BODY_LIMIT {
        return Err((413, "Report is too large"));
    }
    while bytes.len() - offset < length {
        let needed = (length - (bytes.len() - offset)).min(buffer.len());
        let count = timed_read(stream, &mut buffer[..needed], deadline, Some(stopping))
            .map_err(read_error)?;
        bytes.extend_from_slice(&buffer[..count]);
    }
    Ok(Request {
        method,
        target,
        body: bytes[offset..offset + length].to_vec(),
    })
}

fn read_error(error: io::Error) -> (u16, &'static str) {
    match error.kind() {
        io::ErrorKind::TimedOut | io::ErrorKind::WouldBlock => (408, "Request deadline exceeded"),
        _ => (400, "Incomplete HTTP request"),
    }
}

fn response(
    stream: &mut TcpStream,
    status: u16,
    content_type: &str,
    body: &[u8],
    stopping: &AtomicBool,
) -> io::Result<()> {
    let reason = match status {
        200 => "OK",
        204 => "No Content",
        400 => "Bad Request",
        404 => "Not Found",
        408 => "Request Timeout",
        413 => "Content Too Large",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        _ => "Internal Server Error",
    };
    let headers = format!(
        "HTTP/1.1 {status} {reason}\r\nContent-Length: {}\r\nContent-Type: {content_type}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Gdcef-Fixture: 1\r\n\r\n",
        body.len()
    );
    let deadline = Instant::now() + WRITE_TIMEOUT;
    for bytes in [headers.as_bytes(), body] {
        timed_write(stream, bytes, deadline, Some(stopping))?;
    }
    Ok(())
}

fn timed_write(
    stream: &mut TcpStream,
    mut bytes: &[u8],
    deadline: Instant,
    stopping: Option<&AtomicBool>,
) -> io::Result<()> {
    while !bytes.is_empty() {
        let remaining = remaining(deadline, stopping)?;
        match stream.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "Closed HTTP client",
                ));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                thread::sleep(POLL_INTERVAL.min(remaining))
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

fn handle_client(mut stream: TcpStream, state: &State, page: &[u8]) {
    if state.stopping.load(Ordering::Acquire) {
        return;
    }
    let request = match read_request(&mut stream, &state.stopping) {
        Ok(request) => request,
        Err((status, message)) => {
            let _ = response(
                &mut stream,
                status,
                "text/plain",
                message.as_bytes(),
                &state.stopping,
            );
            return;
        }
    };
    let (path, query) = request
        .target
        .split_once('?')
        .unwrap_or((&request.target, ""));
    if request.method == "GET" && path == "/health" {
        let _ = response(&mut stream, 204, "text/plain", &[], &state.stopping);
        return;
    }
    let run = url::form_urlencoded::parse(query.as_bytes())
        .find_map(|(key, value)| (key == "run").then(|| value.into_owned()));
    let Some(run) = run.filter(|id| lock(&state.runs).contains_key(id)) else {
        let _ = response(
            &mut stream,
            404,
            "text/plain",
            b"Unknown test run",
            &state.stopping,
        );
        return;
    };
    let (status, content_type, body) = match (request.method.as_str(), path) {
        ("GET", "/case" | "/blank" | "/js" | "/lifecycle") => {
            let _ = response(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                page,
                &state.stopping,
            );
            return;
        }
        ("GET", "/state") => {
            let runs = lock(&state.runs);
            let events = runs.get(&run).cloned().unwrap_or_default();
            drop(runs);
            (
                200,
                "application/json",
                json!({ "events": events }).to_string().into_bytes(),
            )
        }
        ("POST", "/event") => {
            let event = serde_json::from_slice::<Value>(&request.body).ok();
            match event {
                Some(Value::Object(mut event))
                    if event.get("type").is_some_and(Value::is_string) =>
                {
                    let mut runs = lock(&state.runs);
                    let events = runs.entry(run).or_default();
                    if events.len() >= EVENT_LIMIT {
                        (429, "text/plain", b"Too many test events".to_vec())
                    } else {
                        let now = SystemTime::now()
                            .duration_since(UNIX_EPOCH)
                            .unwrap_or_default()
                            .as_millis();
                        event.insert("received_ms".into(), json!(now));
                        events.push(Value::Object(event));
                        (204, "text/plain", Vec::new())
                    }
                }
                _ => (400, "text/plain", b"Invalid test event".to_vec()),
            }
        }
        _ => (404, "text/plain", b"Unknown fixture endpoint".to_vec()),
    };
    let _ = response(&mut stream, status, content_type, &body, &state.stopping);
}

fn health_check(address: SocketAddr, timeout: Duration) -> io::Result<()> {
    let deadline = Instant::now() + timeout;
    let mut stream = TcpStream::connect_timeout(&address, timeout)?;
    stream.set_nonblocking(true)?;
    timed_write(
        &mut stream,
        b"GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
        deadline,
        None,
    )?;
    let mut bytes = Vec::new();
    let mut buffer = [0_u8; 1024];
    loop {
        let mut headers = [httparse::EMPTY_HEADER; 16];
        let mut parsed = httparse::Response::new(&mut headers);
        match parsed.parse(&bytes) {
            Ok(httparse::Status::Complete(_)) => {
                let identity = parsed.headers.iter().any(|header| {
                    header.name.eq_ignore_ascii_case("x-gdcef-fixture") && header.value == b"1"
                });
                return if parsed.code == Some(204) && identity {
                    Ok(())
                } else {
                    Err(io::Error::new(
                        io::ErrorKind::InvalidData,
                        "Unexpected fixture health response",
                    ))
                };
            }
            Ok(httparse::Status::Partial) if bytes.len() < HEADER_LIMIT => {
                let count = timed_read(&mut stream, &mut buffer, deadline, None)?;
                bytes.extend_from_slice(&buffer[..count]);
            }
            _ => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidData,
                    "Malformed fixture health response",
                ));
            }
        }
    }
}

fn wait_until_reachable(address: SocketAddr) -> io::Result<()> {
    let deadline = Instant::now() + Duration::from_secs(2);
    let mut last_error = None;
    loop {
        let timeout = deadline
            .saturating_duration_since(Instant::now())
            .min(Duration::from_millis(250));
        if timeout.is_zero() {
            return Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("Loopback fixture {address} did not become reachable: {last_error:?}"),
            ));
        }
        match health_check(address, timeout) {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionRefused
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                ) =>
            {
                last_error = Some(error);
                thread::sleep(
                    Duration::from_millis(25)
                        .min(deadline.saturating_duration_since(Instant::now())),
                );
            }
            Err(error) => return Err(error),
        }
    }
}

#[cfg(test)]
#[allow(clippy::panic_in_result_fn)]
mod tests {
    use super::*;

    fn exchange(server: &FixtureServer, request: &[u8]) -> Result<(u16, Vec<u8>), Box<dyn Error>> {
        let address: SocketAddr = server.origin().trim_start_matches("http://").parse()?;
        let mut stream = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        stream.set_read_timeout(Some(Duration::from_secs(4)))?;
        stream.set_write_timeout(Some(Duration::from_secs(1)))?;
        stream.write_all(request)?;
        let mut bytes = Vec::new();
        let mut buffer = [0_u8; 4096];
        loop {
            let mut headers = [httparse::EMPTY_HEADER; 16];
            let mut response = httparse::Response::new(&mut headers);
            if let httparse::Status::Complete(offset) = response.parse(&bytes)? {
                let length = response
                    .headers
                    .iter()
                    .find(|header| header.name.eq_ignore_ascii_case("content-length"))
                    .ok_or("Missing Content-Length")?;
                let length: usize = std::str::from_utf8(length.value)?.parse()?;
                if bytes.len() >= offset + length {
                    return Ok((
                        response.code.ok_or("Missing status")?,
                        bytes[offset..offset + length].to_vec(),
                    ));
                }
            }
            match stream.read(&mut buffer) {
                Ok(0) => {
                    return Err(format!(
                        "HTTP response ended early; received {} bytes: {:?}",
                        bytes.len(),
                        String::from_utf8_lossy(&bytes)
                    )
                    .into());
                }
                Ok(count) => bytes.extend_from_slice(&buffer[..count]),
                Err(error) => {
                    return Err(format!(
                        "HTTP response read failed: {error}; received {} bytes: {:?}",
                        bytes.len(),
                        String::from_utf8_lossy(&bytes)
                    )
                    .into());
                }
            }
        }
    }

    fn request(
        server: &FixtureServer,
        method: &str,
        path: &str,
        body: &[u8],
    ) -> Result<(u16, Vec<u8>), Box<dyn Error>> {
        let mut bytes = format!("{method} {path} HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).into_bytes();
        bytes.extend_from_slice(body);
        exchange(server, &bytes)
    }

    #[test]
    fn fixture_routes_isolate_runs_and_validate_events() -> Result<(), Box<dyn Error>> {
        let server = FixtureServer::start(b"<p>fixture</p>".to_vec())?;
        server.register("first");
        server.register("second run");
        assert!(server.origin().starts_with("http://127.0.0.1:"));
        assert_eq!(request(&server, "GET", "/case?run=unknown", &[])?.0, 404);
        for path in ["case", "blank", "js", "lifecycle"] {
            let (status, page) = request(&server, "GET", &format!("/{path}?run=first"), &[])?;
            assert_eq!(status, 200);
            assert_eq!(page, b"<p>fixture</p>");
        }
        assert_eq!(
            request(
                &server,
                "POST",
                "/event?run=first",
                br#"{"type":"js_ready","preload":true}"#
            )?
            .0,
            204
        );
        for invalid in [
            b"not json".as_slice(),
            br#"{"wrong":"shape"}"#,
            b"[]",
            b"null",
        ] {
            assert_eq!(
                request(&server, "POST", "/event?run=first", invalid)?.0,
                400
            );
        }
        // The declared oversized body is rejected without reading or allocating it.
        assert_eq!(exchange(&server, b"POST /event?run=first HTTP/1.1\r\nHost: 127.0.0.1\r\nContent-Length: 65537\r\n\r\n")?.0, 413);
        let first: Value =
            serde_json::from_slice(&request(&server, "GET", "/state?run=first", &[])?.1)?;
        assert_eq!(first["events"].as_array().map(Vec::len), Some(1));
        assert_eq!(first["events"][0]["type"], "js_ready");
        assert!(first["events"][0]["received_ms"].as_u64().is_some());
        let second: Value =
            serde_json::from_slice(&request(&server, "GET", "/state?run=second%20run", &[])?.1)?;
        assert_eq!(second["events"], json!([]));
        assert_eq!(server.events("first").len(), 1);
        Ok(())
    }

    #[test]
    fn event_limit_rejects_additional_events() -> Result<(), Box<dyn Error>> {
        let server = FixtureServer::start(Vec::new())?;
        server.register("full");
        // Seed the bounded history to avoid 1024 round trips in a unit test.
        lock(&server.state.runs).insert("full".into(), vec![json!({"type":"prior"}); EVENT_LIMIT]);
        assert_eq!(
            request(
                &server,
                "POST",
                "/event?run=full",
                br#"{"type":"overflow"}"#
            )?
            .0,
            429
        );
        assert_eq!(server.events("full").len(), EVENT_LIMIT);
        Ok(())
    }

    #[test]
    fn malformed_or_stalled_clients_do_not_block_other_clients_or_drop()
    -> Result<(), Box<dyn Error>> {
        let server = FixtureServer::start(Vec::new())?;
        let address: SocketAddr = server.origin().trim_start_matches("http://").parse()?;
        let mut stalled_header = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        stalled_header.write_all(b"GET /health HTTP/1.1\r\nHost:")?;
        let mut stalled_body = TcpStream::connect_timeout(&address, Duration::from_secs(1))?;
        stalled_body
            .write_all(b"POST /event HTTP/1.1\r\nHost: local\r\nContent-Length: 10\r\n\r\nx")?;
        assert_eq!(exchange(&server, b"not a request\r\n\r\n")?.0, 400);
        assert_eq!(
            exchange(
                &server,
                b"POST /event HTTP/1.1\r\nContent-Length: 1\r\nContent-Length: 1\r\n\r\nx"
            )?
            .0,
            400
        );
        assert_eq!(
            exchange(
                &server,
                b"POST /event HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n"
            )?
            .0,
            400
        );
        assert_eq!(request(&server, "GET", "/health", &[])?.0, 204);
        let before_drop = Instant::now();
        drop(server);
        assert!(
            before_drop.elapsed() < Duration::from_secs(1),
            "Drop must interrupt open connections"
        );
        for mut stream in [stalled_header, stalled_body] {
            stream.set_read_timeout(Some(Duration::from_millis(250)))?;
            let mut bytes = [0_u8; 1024];
            match stream.read(&mut bytes) {
                Ok(count) => assert_eq!(count, 0, "Drop must close the connection"),
                Err(error) => assert!(matches!(
                    error.kind(),
                    io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::BrokenPipe
                )),
            }
        }
        Ok(())
    }

    #[test]
    fn stalled_header_reaches_a_deadline() -> Result<(), Box<dyn Error>> {
        let server = FixtureServer::start(Vec::new())?;
        let started = Instant::now();
        assert_eq!(exchange(&server, b"GET /health HTTP/1.1\r\nHost:")?.0, 408);
        assert!(started.elapsed() < Duration::from_secs(4));
        Ok(())
    }
}
