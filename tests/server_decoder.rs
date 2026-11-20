#![cfg(feature = "server")]
use std::{
    io::{self, Cursor, Read, Write},
    net::{Shutdown, TcpStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
    },
    time::{Duration, Instant},
};
use zcall::server::{
    Cancellation, Connection, HandlerError, Response, Server, ServerConfig,
    http::{self, HttpError, Request, RequestPolicy},
    transport::{Address, Listener, Peer},
};

struct Input {
    bytes: Cursor<Vec<u8>>,
    first_read: Arc<Mutex<Option<Instant>>>,
    delay: Duration,
}
impl Input {
    fn new(bytes: &[u8]) -> Self {
        Self {
            bytes: Cursor::new(bytes.to_vec()),
            first_read: Arc::new(Mutex::new(None)),
            delay: Duration::ZERO,
        }
    }
}
impl Read for Input {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let mut first = self.first_read.lock().unwrap();
        if first.is_none() {
            *first = Some(Instant::now());
            std::thread::sleep(self.delay);
        }
        self.bytes.read(bytes)
    }
}
impl Write for Input {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
impl Connection for Input {
    fn set_read_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    fn set_write_timeout(&self, _: Option<Duration>) -> io::Result<()> {
        Ok(())
    }
    fn shutdown_write(&self) -> io::Result<()> {
        Ok(())
    }
}
const WIRE: &[u8] = b"POST /custom HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Encoding: X-Fixture\r\nContent-Length: 2\r\n\r\nxy";

#[test]
fn explicit_decoder_gets_bounded_wire_bytes_and_normalizes_body_headers() {
    let decoder = |encoding: &str, body: &[u8], maximum: usize, _: Instant| {
        assert_eq!(encoding, "x-fixture");
        assert_eq!(body, b"xy");
        assert_eq!(maximum, 64);
        Ok(br#"{"decoded":true}"#.to_vec())
    };
    let mut input = Input::new(WIRE);
    let request = http::read_request_with_decoder(
        &mut input,
        64,
        Duration::from_secs(1),
        RequestPolicy::default(),
        Some(&decoder),
    )
    .unwrap();
    assert_eq!(request.body, br#"{"decoded":true}"#);
    assert_eq!(request.header("content-encoding"), None);
    assert_eq!(request.header("transfer-encoding"), None);
    assert_eq!(request.header("content-length"), Some("16"));
    let mut default = Input::new(WIRE);
    assert_eq!(
        http::read_request(&mut default, 64, Duration::from_secs(1))
            .err()
            .unwrap()
            .status,
        415
    );
}

#[test]
fn wire_and_decoded_limits_are_separate_and_checked_by_the_library() {
    let called = AtomicBool::new(false);
    let decoder = |_: &str, _: &[u8], maximum: usize, _: Instant| {
        called.store(true, Ordering::Relaxed);
        Ok(vec![b'x'; maximum + 1])
    };
    let mut input = Input::new(WIRE);
    assert_eq!(
        http::read_request_with_decoder(
            &mut input,
            32,
            Duration::from_secs(1),
            RequestPolicy::default(),
            Some(&decoder),
        )
        .err()
        .unwrap()
        .status,
        413
    );
    assert!(called.load(Ordering::Relaxed));
    called.store(false, Ordering::Relaxed);
    let oversized = String::from_utf8(WIRE.to_vec())
        .unwrap()
        .replace("Content-Length: 2", "Content-Length: 33");
    let mut input = Input::new(oversized.as_bytes());
    assert_eq!(
        http::read_request_with_decoder(
            &mut input,
            32,
            Duration::from_secs(1),
            RequestPolicy::default(),
            Some(&decoder),
        )
        .err()
        .unwrap()
        .status,
        413
    );
    assert!(!called.load(Ordering::Relaxed));
}

#[test]
fn slow_wire_read_does_not_reset_the_decoder_deadline() {
    let mut input = Input::new(WIRE);
    input.delay = Duration::from_millis(20);
    let first_read = input.first_read.clone();
    let timeout = Duration::from_secs(1);
    let decoder = move |_: &str, _: &[u8], _: usize, deadline: Instant| {
        assert!(deadline <= first_read.lock().unwrap().unwrap() + timeout);
        Ok(b"{}".to_vec())
    };
    http::read_request_with_decoder(
        &mut input,
        64,
        timeout,
        RequestPolicy::default(),
        Some(&decoder),
    )
    .unwrap();
}

#[test]
fn decoder_failure_and_expired_deadline_do_not_return_a_request() {
    let rejected = |_: &str, _: &[u8], _: usize, _: Instant| {
        Err(HttpError {
            status: 415,
            message: "unsupported content coding",
        })
    };
    let mut input = Input::new(WIRE);
    assert_eq!(
        http::read_request_with_decoder(
            &mut input,
            64,
            Duration::from_secs(1),
            RequestPolicy::default(),
            Some(&rejected)
        )
        .err()
        .unwrap()
        .status,
        415
    );
    let slow = |_: &str, _: &[u8], _: usize, deadline: Instant| {
        std::thread::sleep(
            deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(1),
        );
        Ok(b"{}".to_vec())
    };
    let mut input = Input::new(WIRE);
    assert_eq!(
        http::read_request_with_decoder(
            &mut input,
            64,
            Duration::from_millis(30),
            RequestPolicy::default(),
            Some(&slow)
        )
        .err()
        .unwrap()
        .status,
        408
    );
}

#[test]
fn the_complete_server_uses_the_same_explicit_decoder_hook() {
    let decoder = |_: &str, _: &[u8], _: usize, _: Instant| Ok(b"{}".to_vec());
    let handler = |request: Request,
                   _: &Peer,
                   response: &mut Response<'_>,
                   _: &Cancellation|
     -> Result<(), HandlerError> {
        assert_eq!(request.body, b"{}");
        assert_eq!(request.header("content-length"), Some("2"));
        assert_eq!(request.header("content-encoding"), None);
        response.send(200, "application/json", &request.body)
    };
    let mut server = Server::with_hooks(
        ServerConfig::default(),
        handler,
        None,
        Some(Arc::new(decoder)),
    )
    .unwrap();
    let address = match server
        .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
        .unwrap()
    {
        Address::Tcp(address) => address,
        #[cfg(unix)]
        _ => unreachable!(),
    };
    let mut client = TcpStream::connect(address).unwrap();
    client
        .set_read_timeout(Some(Duration::from_secs(2)))
        .unwrap();
    client.write_all(WIRE).unwrap();
    client.shutdown(Shutdown::Write).unwrap();
    let mut reply = String::new();
    client.read_to_string(&mut reply).unwrap();
    assert!(reply.starts_with("HTTP/1.1 200"));
    assert!(reply.ends_with("{}"));
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}
