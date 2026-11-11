//! Exercise the Sans-I/O discovery loop through real loopback HTTP exchanges.
//! The test transport is deliberately local and never uses a provider account.
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::thread;
use std::time::{Duration, Instant};
use zcall::v1::*;

const RESPONSE: &[u8] = br#"{"id":"r","object":"response","model":"m","created_at":1,"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}"#;

struct HttpResponse {
    status: u16,
    content_type: Option<String>,
    body: Vec<u8>,
}

fn send(probe: &Probe<'_>) -> HttpResponse {
    let without_scheme = probe.url.strip_prefix("http://").unwrap();
    let (authority, path) = without_scheme.split_once('/').unwrap();
    let mut socket = TcpStream::connect(authority).unwrap();
    socket
        .set_read_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    socket
        .set_write_timeout(Some(Duration::from_secs(5)))
        .unwrap();
    write!(socket, "{} /{} HTTP/1.1\r\nHost: {}\r\nContent-Type: {}\r\nAccept: {}\r\nContent-Length: {}\r\nConnection: close\r\n", probe.method, path, authority, probe.content_type, probe.accept, probe.body.len()).unwrap();
    if let Some((name, value)) = probe.version_header {
        write!(socket, "{name}: {value}\r\n").unwrap();
    }
    socket.write_all(b"\r\n").unwrap();
    socket.write_all(probe.body).unwrap();
    let mut response = Vec::new();
    socket.take(32 * 1024).read_to_end(&mut response).unwrap();
    let split = response
        .windows(4)
        .position(|part| part == b"\r\n\r\n")
        .unwrap();
    let headers = std::str::from_utf8(&response[..split]).unwrap();
    let status = headers
        .lines()
        .next()
        .unwrap()
        .split_whitespace()
        .nth(1)
        .unwrap()
        .parse()
        .unwrap();
    let content_type = headers
        .lines()
        .filter_map(|line| line.split_once(':'))
        .find(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        .map(|(_, value)| value.trim().to_owned());
    HttpResponse {
        status,
        content_type,
        body: response[split + 4..].to_vec(),
    }
}

fn serve(
    listener: TcpListener,
    replies: Vec<(u16, &'static [u8])>,
) -> thread::JoinHandle<Vec<String>> {
    listener.set_nonblocking(true).unwrap();
    thread::spawn(move || {
        let mut paths = Vec::new();
        for (status, body) in replies {
            let deadline = Instant::now() + Duration::from_secs(5);
            let mut socket = loop {
                match listener.accept() {
                    Ok((socket, _)) => break socket,
                    Err(error)
                        if error.kind() == std::io::ErrorKind::WouldBlock
                            && Instant::now() < deadline =>
                    {
                        thread::sleep(Duration::from_millis(2))
                    }
                    Err(error) => panic!("loopback accept failed: {error}"),
                }
            };
            socket
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            socket
                .set_write_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let split = loop {
                let mut bytes = [0; 1024];
                let n = socket.read(&mut bytes).unwrap();
                assert!(n > 0, "request ended before its headers");
                request.extend_from_slice(&bytes[..n]);
                assert!(request.len() <= 8192, "unexpectedly large probe request");
                if let Some(split) = request.windows(4).position(|part| part == b"\r\n\r\n") {
                    break split;
                }
            };
            let headers = std::str::from_utf8(&request[..split]).unwrap();
            let first = headers.lines().next().unwrap();
            assert!(first.starts_with("POST "));
            paths.push(first.split_whitespace().nth(1).unwrap().to_owned());
            let length: usize = headers
                .lines()
                .filter_map(|line| line.split_once(':'))
                .find(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                .unwrap()
                .1
                .trim()
                .parse()
                .unwrap();
            assert!(length < 4096);
            while request.len() < split + 4 + length {
                let mut bytes = [0; 1024];
                let n = socket.read(&mut bytes).unwrap();
                assert!(n > 0, "probe body ended prematurely");
                request.extend_from_slice(&bytes[..n]);
            }
            let mut tokens = TokenStorage::<128>::new();
            assert_eq!(
                Json::parse(
                    &request[split + 4..split + 4 + length],
                    &mut tokens,
                    Limits::default()
                )
                .unwrap()
                .kind(),
                JsonKind::Object
            );
            write!(socket, "HTTP/1.1 {status} Test\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n", body.len()).unwrap();
            socket.write_all(body).unwrap();
        }
        paths
    })
}

#[test]
fn discovers_responses_after_chat_404_over_local_http() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}", listener.local_addr().unwrap());
    let server = serve(listener, vec![(404, b"{}"), (200, RESPONSE)]);
    let mut discovery = Discovery::new(&base, "m").unwrap();
    let mut url = [0; 256];
    let mut body = [0; 512];
    let mut tokens = TokenStorage::<256>::new();
    while let Some(probe) = discovery.next_probe(&mut url, &mut body).unwrap() {
        let response = send(&probe);
        discovery
            .observe(
                response.status,
                response.content_type.as_deref(),
                &response.body,
                &mut tokens,
                Limits::default(),
            )
            .unwrap();
    }
    assert_eq!(
        discovery.selected().unwrap().format().protocol,
        Protocol::OpenAiResponses
    );
    assert_eq!(discovery.attempts().len(), 2);
    assert_eq!(
        server.join().unwrap(),
        ["/v1/chat/completions", "/v1/responses"]
    );
}

#[test]
fn authentication_failures_exhaust_configured_probes_without_a_false_match() {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let base = format!("http://{}/prefix/v1", listener.local_addr().unwrap());
    let server = serve(
        listener,
        vec![
            (401, b"{\"error\":\"unauthorized\"}"),
            (401, b"{\"error\":\"unauthorized\"}"),
        ],
    );
    let formats = [
        Format::new(Protocol::OpenAiChat),
        Format::new(Protocol::OpenAiResponses),
    ];
    let mut config = DiscoveryConfig::new("m");
    config.formats = &formats;
    let mut discovery = Discovery::with_config(&base, config).unwrap();
    let mut url = [0; 256];
    let mut body = [0; 512];
    let mut tokens = TokenStorage::<128>::new();
    while let Some(probe) = discovery.next_probe(&mut url, &mut body).unwrap() {
        let response = send(&probe);
        assert_eq!(
            discovery
                .observe(
                    response.status,
                    response.content_type.as_deref(),
                    &response.body,
                    &mut tokens,
                    Limits::default()
                )
                .unwrap(),
            ProbeOutcome::AuthenticationRequired
        );
    }
    assert!(discovery.is_finished());
    assert!(discovery.selected().is_none());
    assert_eq!(discovery.attempts().len(), 2);
    assert_eq!(
        server.join().unwrap(),
        ["/prefix/v1/chat/completions", "/prefix/v1/responses"]
    );
}
