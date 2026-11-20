//! Bounded HTTP/1 request parsing and explicit response framing.
use super::Connection;
use std::{
    io::{self, Write},
    time::{Duration, Instant},
};

pub const HEADER_LIMIT: usize = 16 * 1024;

pub struct Request {
    pub method: String,
    pub path: String,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
}
impl Request {
    pub fn header(&self, name: &str) -> Option<&str> {
        self.headers
            .iter()
            .find(|(key, _)| key == name)
            .map(|(_, value)| value.as_str())
    }
}

#[derive(Debug, Clone, Copy, Eq, PartialEq)]
pub struct HttpError {
    pub status: u16,
    pub message: &'static str,
}
fn failure(status: u16, message: &'static str) -> HttpError {
    HttpError { status, message }
}

struct Input<'a> {
    socket: &'a mut dyn Connection,
    data: Vec<u8>,
    position: usize,
    deadline: Instant,
}
impl Input<'_> {
    fn fill(&mut self) -> Result<(), HttpError> {
        let mut buffer = [0u8; 4096];
        loop {
            if self.socket.cancelled() {
                return Err(failure(503, "gateway is stopping"));
            }
            let Some(remaining) = self.deadline.checked_duration_since(Instant::now()) else {
                return Err(failure(408, "request timeout"));
            };
            self.socket
                .set_read_timeout(Some(remaining.min(Duration::from_secs(1))))
                .map_err(|_| failure(400, "socket timeout configuration failed"))?;
            match self.socket.read(&mut buffer) {
                Ok(0) => return Err(failure(400, "incomplete HTTP request")),
                Ok(count) => {
                    self.data.extend_from_slice(&buffer[..count]);
                    return Ok(());
                }
                Err(error)
                    if matches!(
                        error.kind(),
                        io::ErrorKind::Interrupted
                            | io::ErrorKind::WouldBlock
                            | io::ErrorKind::TimedOut
                    ) =>
                {
                    continue;
                }
                Err(_) => return Err(failure(400, "cannot read request")),
            }
        }
    }
    fn line(&mut self, maximum: usize) -> Result<Vec<u8>, HttpError> {
        loop {
            if let Some(relative) = self.data[self.position..]
                .windows(2)
                .position(|w| w == b"\r\n")
            {
                if relative > maximum {
                    return Err(failure(431, "HTTP line too long"));
                }
                let line = self.data[self.position..self.position + relative].to_vec();
                self.position += relative + 2;
                self.compact();
                return Ok(line);
            }
            if self.data.len() - self.position > maximum {
                return Err(failure(431, "HTTP line too long"));
            }
            self.fill()?;
        }
    }
    fn append(&mut self, output: &mut Vec<u8>, count: usize) -> Result<(), HttpError> {
        let end = output
            .len()
            .checked_add(count)
            .ok_or_else(|| failure(413, "body too large"))?;
        output
            .try_reserve(count)
            .map_err(|_| failure(503, "body allocation failed"))?;
        while output.len() < end {
            if self.position == self.data.len() {
                self.data.clear();
                self.position = 0;
                self.fill()?;
            }
            let count = (end - output.len()).min(self.data.len() - self.position);
            output.extend_from_slice(&self.data[self.position..self.position + count]);
            self.position += count;
            self.compact();
        }
        Ok(())
    }
    fn compact(&mut self) {
        if self.position >= 4096 {
            self.data.drain(..self.position);
            self.position = 0;
        }
    }
}

/// Application policy applied after syntactic HTTP framing checks.
///
/// The default accepts JSON API clients and rejects browser-origin and encoded
/// bodies. A custom handler can explicitly permit these inputs without relaxing
/// duplicate-header, framing, body-size or deadline validation.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct RequestPolicy {
    pub allow_origin: bool,
    pub allow_encoded_body: bool,
    pub require_json_post: bool,
}

impl Default for RequestPolicy {
    fn default() -> Self {
        Self {
            allow_origin: false,
            allow_encoded_body: false,
            require_json_post: true,
        }
    }
}

pub fn read_request(
    socket: &mut dyn Connection,
    max_body: usize,
    timeout: Duration,
) -> Result<Request, HttpError> {
    read_request_with_policy(socket, max_body, timeout, RequestPolicy::default())
}

/// Parse one request with host-selected application policy.
/// HTTP upgrades, request trailers and Expect handshakes are not implemented.
pub fn read_request_with_policy(
    socket: &mut dyn Connection,
    max_body: usize,
    timeout: Duration,
    policy: RequestPolicy,
) -> Result<Request, HttpError> {
    let mut input = Input {
        socket,
        data: Vec::with_capacity(4096),
        position: 0,
        deadline: Instant::now() + timeout,
    };
    let first = input.line(2048)?;
    let first = std::str::from_utf8(&first).map_err(|_| failure(400, "invalid request line"))?;
    let fields: Vec<_> = first.split(' ').collect();
    if fields.len() != 3 || !matches!(fields[2], "HTTP/1.0" | "HTTP/1.1") || fields[0].is_empty() {
        return Err(failure(400, "invalid HTTP request line"));
    }
    let method = fields[0].to_owned();
    if !method.bytes().all(|b| b.is_ascii_uppercase()) {
        return Err(failure(400, "invalid method"));
    }
    let path = fields[1].to_owned();
    if !path.starts_with('/')
        || path.starts_with("//")
        || path.bytes().any(|b| b.is_ascii_control() || b == b' ')
    {
        return Err(failure(400, "origin-form request path required"));
    }
    let mut headers = Vec::new();
    let mut header_bytes = first.len() + 2;
    loop {
        let line = input.line(HEADER_LIMIT)?;
        header_bytes += line.len() + 2;
        if header_bytes > HEADER_LIMIT {
            return Err(failure(431, "headers too large"));
        }
        if line.is_empty() {
            break;
        }
        if headers.len() >= 64 {
            return Err(failure(431, "too many headers"));
        }
        let line =
            std::str::from_utf8(&line).map_err(|_| failure(400, "invalid header encoding"))?;
        let (name, value) = line
            .split_once(':')
            .ok_or_else(|| failure(400, "malformed header"))?;
        if name.is_empty() || !name.bytes().all(token_char) {
            return Err(failure(400, "invalid header name"));
        }
        let name = name.to_ascii_lowercase();
        let value = value.trim_matches([' ', '\t']);
        if value.bytes().any(|b| (b < 32 && b != b'\t') || b == 127) {
            return Err(failure(400, "invalid header value"));
        }
        if headers.iter().any(|(key, _)| key == &name) {
            return Err(failure(400, "duplicate HTTP headers are not accepted"));
        }
        headers.push((name, value.to_string()));
    }
    let mut request = Request {
        method,
        path,
        headers,
        body: Vec::new(),
    };
    if fields[2] == "HTTP/1.1" && request.header("host").is_none() {
        return Err(failure(400, "Host header required"));
    }
    if !policy.allow_origin && request.header("origin").is_some() {
        return Err(failure(403, "browser cross-origin requests are disabled"));
    }
    if request.header("expect").is_some() {
        return Err(failure(417, "Expect is not supported"));
    }
    if request.header("upgrade").is_some() {
        return Err(failure(400, "HTTP upgrades are not supported"));
    }
    if !policy.allow_encoded_body
        && request
            .header("content-encoding")
            .is_some_and(|value| !value.eq_ignore_ascii_case("identity"))
    {
        return Err(failure(415, "compressed request bodies are not supported"));
    }
    let length = request
        .header("content-length")
        .map(|value| {
            if value.is_empty() || !value.bytes().all(|b| b.is_ascii_digit()) {
                return Err(failure(400, "invalid Content-Length"));
            }
            value
                .parse::<usize>()
                .map_err(|_| failure(413, "body too large"))
        })
        .transpose()?;
    let chunked = match request.header("transfer-encoding") {
        None => false,
        Some(value) if value.eq_ignore_ascii_case("chunked") => true,
        Some(_) => return Err(failure(501, "unsupported Transfer-Encoding")),
    };
    if chunked && length.is_some() {
        return Err(failure(400, "ambiguous request framing"));
    }
    if request.method == "POST" {
        let content_type = request
            .header("content-type")
            .unwrap_or("")
            .split(';')
            .next()
            .unwrap_or("")
            .trim();
        if policy.require_json_post && !content_type.eq_ignore_ascii_case("application/json") {
            return Err(failure(415, "Content-Type application/json required"));
        }
        if !chunked && length.is_none() {
            return Err(failure(411, "request body length required"));
        }
    }
    if let Some(count) = length {
        if count > max_body {
            return Err(failure(413, "request body exceeds max_body_bytes"));
        }
        input.append(&mut request.body, count)?;
    } else if chunked {
        loop {
            let line = input.line(128)?;
            let length = line.split(|b| *b == b';').next().unwrap_or(&[]);
            if length.is_empty() || !length.iter().all(u8::is_ascii_hexdigit) {
                return Err(failure(400, "invalid chunk size"));
            }
            let count = usize::from_str_radix(std::str::from_utf8(length).expect("hex ASCII"), 16)
                .map_err(|_| failure(413, "chunk too large"))?;
            if count == 0 {
                if !input.line(HEADER_LIMIT)?.is_empty() {
                    return Err(failure(400, "request trailers are not supported"));
                }
                break;
            }
            if count > max_body.saturating_sub(request.body.len()) {
                return Err(failure(413, "request body exceeds max_body_bytes"));
            }
            input.append(&mut request.body, count)?;
            if !input.line(0)?.is_empty() {
                return Err(failure(400, "invalid chunk delimiter"));
            }
        }
    }
    Ok(request)
}

fn token_char(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"!#$%&'*+-.^_`|~".contains(&byte)
}

pub fn quote(value: &str) -> String {
    let mut result = String::with_capacity(value.len() + 2);
    result.push('"');
    for ch in value.chars() {
        match ch {
            '"' => result.push_str("\\\""),
            '\\' => result.push_str("\\\\"),
            '\n' => result.push_str("\\n"),
            '\r' => result.push_str("\\r"),
            '\t' => result.push_str("\\t"),
            c if c.is_control() => {
                use std::fmt::Write;
                let _ = write!(result, "\\u{:04x}", c as u32);
            }
            c => result.push(c),
        }
    }
    result.push('"');
    result
}

pub fn respond(
    socket: &mut dyn Connection,
    status: u16,
    content_type: &str,
    body: &[u8],
) -> io::Result<()> {
    validate_response_head(status, content_type, &[])?;
    let header = format!(
        "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\nX-Content-Type-Options: nosniff\r\n\r\n",
        reason(status),
        body.len()
    );
    write_socket(socket, header.as_bytes())?;
    write_socket(socket, body)
}

fn validate_response_head(
    status: u16,
    content_type: &str,
    extra: &[(String, String)],
) -> io::Result<()> {
    let valid_value = |value: &str| {
        !value
            .bytes()
            .any(|byte| (byte < 32 && byte != b'\t') || byte == 127)
    };
    if !(200..=599).contains(&status) || !valid_value(content_type) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "invalid response head",
        ));
    }
    for (index, (name, value)) in extra.iter().enumerate() {
        if name.is_empty()
            || !name.bytes().all(token_char)
            || !valid_value(value)
            || [
                "content-length",
                "transfer-encoding",
                "connection",
                "content-type",
            ]
            .iter()
            .any(|reserved| name.eq_ignore_ascii_case(reserved))
            || extra[..index]
                .iter()
                .any(|(previous, _)| previous.eq_ignore_ascii_case(name))
        {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "invalid extra response header",
            ));
        }
    }
    Ok(())
}

fn write_socket(socket: &mut dyn Connection, mut bytes: &[u8]) -> io::Result<()> {
    while !bytes.is_empty() {
        if socket.cancelled() {
            return Err(io::Error::other("gateway is stopping"));
        }
        match socket.write(bytes) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "downstream closed",
                ));
            }
            Ok(count) => bytes = &bytes[count..],
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => return Err(error),
        }
    }
    Ok(())
}

pub fn error(socket: &mut dyn Connection, status: u16, message: &str) -> io::Result<()> {
    let body = format!(
        "{{\"error\":{{\"message\":{},\"type\":{},\"code\":{status}}}}}",
        quote(message),
        quote(socket.error_type())
    );
    respond(socket, status, "application/json", body.as_bytes())
}

/// Send an early rejection before discarding a bounded amount of in-flight input.
/// A write half-close preserves the response when the peer sent the body after
/// its headers (RFC 9112 section 9.6). Dropping a socket with unread input can
/// instead reset TCP and erase the response on Windows.
pub fn reject_and_close(socket: &mut dyn Connection, status: u16, message: &str) {
    if error(socket, status, message).is_err() || socket.shutdown_write().is_err() {
        return;
    }
    discard_rejected_input(socket);
}

fn discard_rejected_input(socket: &mut dyn Connection) -> usize {
    let deadline = Instant::now() + Duration::from_millis(200);
    let mut remaining = 64 * 1024;
    let mut buffer = [0u8; 4096];
    while remaining != 0 && !socket.cancelled() {
        let Some(time_left) = deadline.checked_duration_since(Instant::now()) else {
            break;
        };
        if time_left.is_zero()
            || socket
                .set_read_timeout(Some(time_left.min(Duration::from_millis(50))))
                .is_err()
        {
            break;
        }
        let capacity = remaining.min(buffer.len());
        match socket.read(&mut buffer[..capacity]) {
            Ok(0) => break,
            Ok(count) => remaining -= count,
            Err(error)
                if matches!(
                    error.kind(),
                    io::ErrorKind::Interrupted
                        | io::ErrorKind::TimedOut
                        | io::ErrorKind::WouldBlock
                ) => {}
            Err(_) => break,
        }
    }
    64 * 1024 - remaining
}

pub fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        202 => "Accepted",
        204 => "No Content",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        405 => "Method Not Allowed",
        408 => "Request Timeout",
        411 => "Length Required",
        413 => "Content Too Large",
        415 => "Unsupported Media Type",
        417 => "Expectation Failed",
        429 => "Too Many Requests",
        431 => "Request Header Fields Too Large",
        500 => "Internal Server Error",
        501 => "Not Implemented",
        502 => "Bad Gateway",
        503 => "Service Unavailable",
        504 => "Gateway Timeout",
        _ => "Upstream Response",
    }
}

pub struct Chunked<'a> {
    pub socket: &'a mut dyn Connection,
    pub started: bool,
    pub status: u16,
    pub content_type: String,
    pub extra: Vec<(String, String)>,
    pub bytes: usize,
    pub maximum: usize,
}
impl Chunked<'_> {
    pub fn start(&mut self) -> io::Result<()> {
        if self.started {
            return Ok(());
        }
        validate_response_head(self.status, &self.content_type, &self.extra)?;
        self.started = true;
        if matches!(self.status, 204 | 304) {
            return write_socket(
                self.socket,
                format!(
                    "HTTP/1.1 {} {}\r\nConnection: close\r\n\r\n",
                    self.status,
                    reason(self.status)
                )
                .as_bytes(),
            );
        }
        let header = format!(
            "HTTP/1.1 {} {}\r\nContent-Type: {}\r\nTransfer-Encoding: chunked\r\nConnection: close\r\nCache-Control: no-store\r\nX-Accel-Buffering: no\r\nX-Content-Type-Options: nosniff\r\n",
            self.status,
            reason(self.status),
            self.content_type
        );
        write_socket(self.socket, header.as_bytes())?;
        for (name, value) in &self.extra {
            write_socket(self.socket, format!("{name}: {value}\r\n").as_bytes())?;
        }
        write_socket(self.socket, b"\r\n")
    }
    pub fn finish(&mut self) -> io::Result<()> {
        self.start()?;
        if matches!(self.status, 204 | 304) {
            Ok(())
        } else {
            write_socket(self.socket, b"0\r\n\r\n")
        }
    }
}
impl Write for Chunked<'_> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        if matches!(self.status, 204 | 304) {
            return Err(io::Error::other("body forbidden for upstream status"));
        }
        if bytes.len() > self.maximum.saturating_sub(self.bytes) {
            return Err(io::Error::other("response exceeds max_response_bytes"));
        }
        self.start()?;
        write_socket(self.socket, format!("{:x}\r\n", bytes.len()).as_bytes())?;
        write_socket(self.socket, bytes)?;
        write_socket(self.socket, b"\r\n")?;
        self.bytes += bytes.len();
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        self.socket.flush()
    }
}

pub fn constant_eq(left: &[u8], right: &[u8]) -> bool {
    let mut different = left.len() ^ right.len();
    for (a, b) in left.iter().zip(right) {
        different |= (a ^ b) as usize;
    }
    different == 0
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{io::Read, net::TcpStream as Stream};
    struct MemoryConnection {
        input: io::Cursor<Vec<u8>>,
        output: Vec<u8>,
        cancelled: bool,
    }
    impl MemoryConnection {
        fn new(input: &[u8]) -> Self {
            Self {
                input: io::Cursor::new(input.to_vec()),
                output: Vec::new(),
                cancelled: false,
            }
        }
    }
    impl Read for MemoryConnection {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            self.input.read(buffer)
        }
    }
    impl Write for MemoryConnection {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            self.output.extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    impl Connection for MemoryConnection {
        fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
        fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
            Ok(())
        }
        fn shutdown_write(&self) -> io::Result<()> {
            Ok(())
        }
        fn cancelled(&self) -> bool {
            self.cancelled
        }
    }

    #[test]
    fn host_can_accept_custom_media_and_origin_without_ambiguous_framing() {
        let bytes = b"POST /custom HTTP/1.1\r\nHost: localhost\r\nOrigin: https://example.invalid\r\nContent-Type: application/octet-stream\r\nContent-Length: 3\r\n\r\nabc";
        let mut strict = MemoryConnection::new(bytes);
        assert_eq!(
            read_request(&mut strict, 64, Duration::from_secs(1))
                .err()
                .unwrap()
                .status,
            403
        );
        let policy = RequestPolicy {
            allow_origin: true,
            require_json_post: false,
            ..RequestPolicy::default()
        };
        let mut custom = MemoryConnection::new(bytes);
        let request =
            read_request_with_policy(&mut custom, 64, Duration::from_secs(1), policy).unwrap();
        assert_eq!(request.path, "/custom");
        assert_eq!(request.body, b"abc");
        let mut ambiguous = MemoryConnection::new(b"POST /custom HTTP/1.1\r\nHost: localhost\r\nContent-Length: 3\r\nTransfer-Encoding: chunked\r\n\r\n");
        assert_eq!(
            read_request_with_policy(&mut ambiguous, 64, Duration::from_secs(1), policy)
                .err()
                .unwrap()
                .status,
            400
        );
    }

    #[test]
    fn injected_response_headers_fail_before_any_bytes_are_written() {
        let mut connection = MemoryConnection::new(&[]);
        assert!(respond(&mut connection, 200, "text/plain\r\nInjected: yes", b"ok").is_err());
        assert!(connection.output.is_empty());
        for extra in [
            vec![("X-Example".into(), "ok\r\nInjected: yes".into())],
            vec![("Content-Length".into(), "0".into())],
            vec![("X-One".into(), "a".into()), ("x-one".into(), "b".into())],
        ] {
            let mut chunked = Chunked {
                socket: &mut connection,
                started: false,
                status: 200,
                content_type: "text/event-stream".into(),
                extra,
                bytes: 0,
                maximum: 64,
            };
            assert!(chunked.write_all(b"event").is_err());
            assert!(!chunked.started);
            assert!(connection.output.is_empty());
        }
    }

    #[test]
    fn cancellation_hook_prevents_reads_and_writes() {
        let mut connection = MemoryConnection::new(b"GET / HTTP/1.1\r\nHost: localhost\r\n\r\n");
        connection.cancelled = true;
        assert_eq!(
            read_request(&mut connection, 64, Duration::from_secs(1))
                .err()
                .unwrap()
                .status,
            503
        );
        assert!(respond(&mut connection, 200, "text/plain", b"ok").is_err());
        assert!(connection.output.is_empty());
        assert_eq!(connection.input.position(), 0);
    }

    #[cfg(unix)]
    #[test]
    fn http_framing_and_half_close_work_on_a_unix_socket() {
        use std::os::unix::net::UnixStream;
        let (mut client, mut server) = UnixStream::pair().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(2)))
            .unwrap();
        client.write_all(b"POST /custom HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nTransfer-Encoding: chunked\r\n\r\n2\r\n{}\r\n0\r\n\r\n").unwrap();
        let request = read_request(&mut server, 64, Duration::from_secs(1)).unwrap();
        assert_eq!(request.body, b"{}");
        respond(&mut server, 200, "application/json", b"{}").unwrap();
        server.shutdown_write().unwrap();
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(response.ends_with("{}"));
    }

    fn tcp_pair() -> (std::net::TcpStream, Stream) {
        let listener = std::net::TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let client = std::net::TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let (server, _) = listener.accept().unwrap();
        client
            .set_read_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        client
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server
            .set_write_timeout(Some(Duration::from_secs(3)))
            .unwrap();
        server.set_nodelay(true).unwrap();
        client.set_nodelay(true).unwrap();
        (client, server)
    }

    #[test]
    fn rejection_arrives_before_a_delayed_request_body() {
        let (mut client, mut server) = tcp_pair();
        let worker = std::thread::spawn(move || {
            let error = match read_request(&mut server, 1024, Duration::from_secs(3)) {
                Ok(_) => panic!("Origin request must be rejected"),
                Err(error) => error,
            };
            assert_eq!(error.status, 403);
            reject_and_close(&mut server, error.status, error.message);
        });
        client.write_all(b"POST /v1/chat/completions HTTP/1.1\r\nHost: localhost\r\nOrigin: https://example.invalid\r\nContent-Type: application/json\r\nContent-Length: 2\r\n\r\n").unwrap();
        // FIN on the response direction must arrive without waiting for "{}".
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        assert!(response.ends_with("\"code\":403}}"));
        // A scheduler pause can exceed the bounded drain grace period. The
        // response is already complete; late input may then be refused.
        for result in [
            client.write_all(b"{}"),
            client.shutdown(std::net::Shutdown::Write),
        ] {
            if let Err(error) = result {
                assert!(matches!(
                    error.kind(),
                    io::ErrorKind::BrokenPipe
                        | io::ErrorKind::ConnectionReset
                        | io::ErrorKind::ConnectionAborted
                        | io::ErrorKind::NotConnected
                ));
            }
        }
        worker.join().unwrap();
    }

    #[test]
    fn rejected_incomplete_body_cannot_hold_a_worker() {
        let (mut client, mut server) = tcp_pair();
        let (done, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            reject_and_close(&mut server, 403, "blocked");
            done.send(()).unwrap();
        });
        let mut response = String::new();
        client.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 403 Forbidden\r\n"));
        // The peer deliberately keeps its sending side open indefinitely.
        result.recv_timeout(Duration::from_secs(2)).unwrap();
        worker.join().unwrap();
    }

    #[test]
    fn rejected_input_discard_has_a_byte_budget() {
        let (mut client, mut server) = tcp_pair();
        // Queue an over-budget body before starting the deadline. This tests
        // bytes consumed, without depending on how fast the peer is scheduled.
        client.write_all(&[b'x'; 64 * 1024]).unwrap();
        client.write_all(b"!").unwrap();
        let (done, result) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let count = discard_rejected_input(&mut server);
            server
                .set_read_timeout(Some(Duration::from_secs(3)))
                .unwrap();
            let mut tail = [0u8; 1];
            server.read_exact(&mut tail).unwrap();
            done.send((count, tail)).unwrap();
        });
        assert_eq!(
            result.recv_timeout(Duration::from_secs(4)).unwrap(),
            (64 * 1024, *b"!")
        );
        worker.join().unwrap();
    }

    #[test]
    fn json_escaping() {
        assert_eq!(quote("a\n\"b"), "\"a\\n\\\"b\"");
    }
    #[test]
    fn authentication_compare() {
        assert!(constant_eq(b"token", b"token"));
        assert!(!constant_eq(b"token", b"token2"));
        assert!(!constant_eq(b"token", b"tokes"));
    }
}
