#![cfg(feature = "server")]
use std::{
    io::{self, Cursor, Read, Write},
    net::{Shutdown, SocketAddr, TcpStream},
    sync::{
        Arc, RwLock,
        atomic::{AtomicUsize, Ordering},
    },
    time::Duration,
};
use zcall::{
    server::{
        Cancellation, HandlerError, Server, ServerConfig,
        auth::{ApiKey, CallerKeys},
        gateway::{
            Gateway, GatewayConfig, KeyAuthenticator, Target, Transport, UpstreamBody,
            UpstreamRequest, UpstreamResponse,
        },
        protocol::{self, Protocol},
        transport::{Address, Listener},
    },
    v1::{Limits, RecordStorage, Response, TokenStorage},
};

const PROTOCOLS: [Protocol; 6] = [
    Protocol::OpenAiCompletions,
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::AnthropicMessages,
    Protocol::GeminiGenerateContent,
    Protocol::GeminiInteractions,
];
const INPUT: &[u8] = br#"{"model":"alias","messages":[{"role":"user","content":"hello"}]}"#;
const OUTPUT: &[u8] = br#"{"id":"msg_1","type":"message","role":"assistant","model":"upstream","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#;
const EVENTS: &str = concat!(
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"upstream\",\"content\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello\"}}\n\n",
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n",
);

struct ChunkBody {
    source: Cursor<Vec<u8>>,
    cancelled: Arc<AtomicUsize>,
}
impl Read for ChunkBody {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let length = bytes.len().min(7);
        self.source.read(&mut bytes[..length])
    }
}
impl UpstreamBody for ChunkBody {
    fn cancel(&mut self) {
        self.cancelled.fetch_add(1, Ordering::Relaxed);
    }
}

struct Mock {
    protocol: Protocol,
    body: Vec<u8>,
    media: &'static str,
    called: Arc<AtomicUsize>,
    cancelled: Arc<AtomicUsize>,
}
impl Transport for Mock {
    fn execute(
        &self,
        request: &UpstreamRequest,
        cancel: &Cancellation,
    ) -> Result<UpstreamResponse, HandlerError> {
        assert!(!cancel.is_cancelled());
        assert_eq!(request.protocol, self.protocol);
        assert!(String::from_utf8_lossy(&request.body).contains("hello"));
        assert!(request.url.starts_with("http://mock.invalid/"));
        assert!(
            !request
                .headers
                .iter()
                .any(|(name, value)| name.eq_ignore_ascii_case("cookie")
                    || value.contains("caller-secret"))
        );
        self.called.fetch_add(1, Ordering::Relaxed);
        Ok(UpstreamResponse::new(
            200,
            self.media,
            ChunkBody {
                source: Cursor::new(self.body.clone()),
                cancelled: self.cancelled.clone(),
            },
        ))
    }
}

fn common_response(protocol: Protocol) -> Vec<u8> {
    // Common text/usage semantics, without target-specific message identities.
    // The separate strict-loss case below covers an identity-bearing response.
    let body: &[u8] = match protocol {
        Protocol::OpenAiCompletions => br#"{"choices":[{"index":0,"text":"hello","finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        Protocol::OpenAiChat => br#"{"choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}],"usage":{"prompt_tokens":3,"completion_tokens":2}}"#,
        Protocol::OpenAiResponses => br#"{"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}],"usage":{"input_tokens":3,"output_tokens":2}}"#,
        Protocol::AnthropicMessages => OUTPUT,
        Protocol::GeminiGenerateContent => br#"{"candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"hello"}]},"finishReason":"STOP"}],"usageMetadata":{"promptTokenCount":3,"candidatesTokenCount":2}}"#,
        Protocol::GeminiInteractions => br#"{"status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"hello"}]}],"usage":{"total_input_tokens":3,"total_output_tokens":2}}"#,
    };
    body.to_vec()
}

fn bound(server: &mut Server) -> SocketAddr {
    match server
        .listen(Listener::bind_tcp(("127.0.0.1", 0)).unwrap())
        .unwrap()
    {
        Address::Tcp(address) => address,
        #[cfg(unix)]
        _ => unreachable!(),
    }
}
fn post(address: SocketAddr, path: &str, bytes: &[u8], token: Option<&str>) -> Vec<u8> {
    let mut connection = TcpStream::connect(address).unwrap();
    connection
        .set_read_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    connection
        .set_write_timeout(Some(Duration::from_secs(3)))
        .unwrap();
    let auth = token.map_or(String::new(), |token| {
        format!("Authorization: Bearer {token}\r\n")
    });
    write!(connection, "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\nCookie: session=caller-secret\r\n{auth}\r\n", bytes.len()).unwrap();
    connection.write_all(bytes).unwrap();
    connection.shutdown(Shutdown::Write).unwrap();
    let mut response = Vec::new();
    connection.read_to_end(&mut response).unwrap();
    response
}
fn path(protocol: Protocol, stream: bool) -> String {
    protocol::endpoint("http://localhost", protocol, "alias", stream)
        .unwrap()
        .strip_prefix("http://localhost")
        .unwrap()
        .to_owned()
}
fn body(response: &[u8]) -> &[u8] {
    let position = response
        .windows(4)
        .position(|bytes| bytes == b"\r\n\r\n")
        .unwrap();
    &response[position + 4..]
}
fn unchunk(mut input: &[u8]) -> Vec<u8> {
    let mut bytes = Vec::new();
    loop {
        let end = input.windows(2).position(|bytes| bytes == b"\r\n").unwrap();
        let length =
            usize::from_str_radix(std::str::from_utf8(&input[..end]).unwrap(), 16).unwrap();
        input = &input[end + 2..];
        if length == 0 {
            assert_eq!(input, b"\r\n");
            return bytes;
        }
        bytes.extend_from_slice(&input[..length]);
        assert_eq!(&input[length..length + 2], b"\r\n");
        input = &input[length + 2..];
    }
}

#[test]
fn six_by_six_simple_conversations_use_real_http_and_typed_conversion() {
    for source in PROTOCOLS {
        for target in PROTOCOLS {
            let called = Arc::new(AtomicUsize::new(0));
            let cancelled = Arc::new(AtomicUsize::new(0));
            let upstream = common_response(target);
            let transport = Mock {
                protocol: target,
                body: upstream,
                media: "application/json",
                called: called.clone(),
                cancelled: cancelled.clone(),
            };
            let mut route = Target::new("http://mock.invalid", target, "upstream");
            route
                .headers
                .push(("Authorization".into(), "Bearer provider-secret".into()));
            let gateway = Gateway::new(route, transport);
            let mut server = Server::new(
                ServerConfig {
                    workers: 1,
                    ..ServerConfig::default()
                },
                gateway,
            )
            .unwrap();
            let address = bound(&mut server);
            let request =
                protocol::prepare_request(Protocol::OpenAiChat, source, INPUT, "alias", false, 512)
                    .unwrap();
            let reply = post(
                address,
                &path(source, false),
                &request,
                Some("caller-secret"),
            );
            assert!(
                reply.starts_with(b"HTTP/1.1 200"),
                "{source:?} -> {target:?}: {}",
                String::from_utf8_lossy(&reply)
            );
            assert!(String::from_utf8_lossy(body(&reply)).contains("hello"));
            let mut tokens = TokenStorage::<2048>::new();
            let mut records = RecordStorage::<128>::new();
            Response::decode(
                source.into(),
                body(&reply),
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .unwrap();
            assert_eq!(called.load(Ordering::Relaxed), 1);
            assert_eq!(cancelled.load(Ordering::Relaxed), 1);
            assert!(server.shutdown(Duration::from_secs(1)).complete());
        }
    }
}

#[test]
fn caller_key_rotation_and_revocation_gate_real_http_without_forwarding_keys() {
    let mut keys = CallerKeys::new();
    keys.insert(
        ApiKey::new("client", "caller-secret")
            .unwrap()
            .with_name("test client")
            .unwrap(),
    )
    .unwrap();
    let keys = Arc::new(RwLock::new(keys));
    let called = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let gateway = Gateway::new(
        Target::new(
            "http://mock.invalid",
            Protocol::AnthropicMessages,
            "upstream",
        ),
        Mock {
            protocol: Protocol::AnthropicMessages,
            body: OUTPUT.to_vec(),
            media: "application/json",
            called: called.clone(),
            cancelled,
        },
    )
    .with_authorizer(KeyAuthenticator::new(keys.clone()));
    let mut server = Server::new(ServerConfig::default(), gateway).unwrap();
    let address = bound(&mut server);
    let endpoint = path(Protocol::OpenAiChat, false);
    assert!(post(address, &endpoint, INPUT, None).starts_with(b"HTTP/1.1 401"));
    assert_eq!(called.load(Ordering::Relaxed), 0);
    assert!(post(address, &endpoint, INPUT, Some("caller-secret")).starts_with(b"HTTP/1.1 200"));
    keys.write()
        .unwrap()
        .rotate("client", "replacement-key")
        .unwrap();
    assert!(post(address, &endpoint, INPUT, Some("caller-secret")).starts_with(b"HTTP/1.1 401"));
    assert!(post(address, &endpoint, INPUT, Some("replacement-key")).starts_with(b"HTTP/1.1 200"));
    assert!(keys.write().unwrap().revoke("client"));
    assert!(post(address, &endpoint, INPUT, Some("replacement-key")).starts_with(b"HTTP/1.1 401"));
    assert_eq!(called.load(Ordering::Relaxed), 2);
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn fragmented_sse_reaches_a_valid_terminal_event_and_cancels_its_owned_body() {
    let called = Arc::new(AtomicUsize::new(0));
    let cancelled = Arc::new(AtomicUsize::new(0));
    let gateway = Gateway::new(
        Target::new(
            "http://mock.invalid",
            Protocol::AnthropicMessages,
            "upstream",
        ),
        Mock {
            protocol: Protocol::AnthropicMessages,
            body: EVENTS.as_bytes().to_vec(),
            media: "text/event-stream; charset=utf-8",
            called,
            cancelled: cancelled.clone(),
        },
    );
    let mut server = Server::new(ServerConfig::default(), gateway).unwrap();
    let address = bound(&mut server);
    let request = protocol::prepare_request(
        Protocol::OpenAiChat,
        Protocol::OpenAiChat,
        INPUT,
        "alias",
        true,
        512,
    )
    .unwrap();
    let reply = post(address, &path(Protocol::OpenAiChat, true), &request, None);
    assert!(reply.starts_with(b"HTTP/1.1 200"));
    let events = String::from_utf8(unchunk(body(&reply))).unwrap();
    assert!(events.contains("hello"));
    assert!(events.contains("[DONE]"));
    assert_eq!(cancelled.load(Ordering::Relaxed), 1);
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn mime_limits_unknown_routes_and_unsupported_semantics_never_claim_success() {
    for (media, maximum, request_path, request_body, expected, calls) in [
        ("text/html", 4096, path(Protocol::OpenAiChat, false), INPUT.to_vec(), 502, 1),
        ("application/json", 8, path(Protocol::OpenAiChat, false), INPUT.to_vec(), 502, 1),
        ("application/json", 4096, "/custom-unknown".into(), INPUT.to_vec(), 404, 0),
        ("application/json", 4096, path(Protocol::OpenAiChat, false), br#"{"model":"alias","messages":[{"role":"user","content":"hello"}],"vendor_private":true}"#.to_vec(), 422, 0),
    ] {
        let called = Arc::new(AtomicUsize::new(0));
        let cancelled = Arc::new(AtomicUsize::new(0));
        let gateway = Gateway::new(
            Target::new("http://mock.invalid", Protocol::AnthropicMessages, "upstream"),
            Mock { protocol: Protocol::AnthropicMessages, body: OUTPUT.to_vec(), media, called: called.clone(), cancelled: cancelled.clone() },
        ).with_config(GatewayConfig { default_max_tokens: 512, max_upstream_bytes: maximum }).unwrap();
        let mut server = Server::new(ServerConfig::default(), gateway).unwrap();
        let address = bound(&mut server);
        let reply = post(address, &request_path, &request_body, None);
        assert!(reply.starts_with(format!("HTTP/1.1 {expected}").as_bytes()), "{}", String::from_utf8_lossy(&reply));
        assert_eq!(called.load(Ordering::Relaxed), calls);
        assert_eq!(cancelled.load(Ordering::Relaxed), calls);
        assert!(server.shutdown(Duration::from_secs(1)).complete());
    }
}

#[test]
fn explicit_provider_headers_are_validated_before_transport() {
    struct Never;
    impl Transport for Never {
        fn execute(
            &self,
            _: &UpstreamRequest,
            _: &Cancellation,
        ) -> Result<UpstreamResponse, HandlerError> {
            panic!("invalid headers must not reach transport");
        }
    }
    let mut target = Target::new("http://mock.invalid", Protocol::OpenAiChat, "upstream");
    target.headers.push((
        "Authorization".into(),
        "Bearer test\r\nInjected: yes".into(),
    ));
    let mut server = Server::new(ServerConfig::default(), Gateway::new(target, Never)).unwrap();
    let address = bound(&mut server);
    assert!(
        post(address, &path(Protocol::OpenAiChat, false), INPUT, None).starts_with(b"HTTP/1.1 500")
    );
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn a_source_message_identity_is_not_silently_lost_in_a_target_without_one() {
    let body = protocol::convert_response(
        Protocol::AnthropicMessages,
        Protocol::OpenAiResponses,
        OUTPUT,
        "upstream",
    )
    .unwrap();
    let called = Arc::new(AtomicUsize::new(0));
    let gateway = Gateway::new(
        Target::new("http://mock.invalid", Protocol::OpenAiResponses, "upstream"),
        Mock {
            protocol: Protocol::OpenAiResponses,
            body,
            media: "application/json",
            called: called.clone(),
            cancelled: Arc::new(AtomicUsize::new(0)),
        },
    );
    let mut server = Server::new(ServerConfig::default(), gateway).unwrap();
    let address = bound(&mut server);
    let request = protocol::prepare_request(
        Protocol::OpenAiChat,
        Protocol::OpenAiCompletions,
        INPUT,
        "alias",
        false,
        512,
    )
    .unwrap();
    let reply = post(
        address,
        &path(Protocol::OpenAiCompletions, false),
        &request,
        None,
    );
    assert!(reply.starts_with(b"HTTP/1.1 502"));
    assert!(String::from_utf8_lossy(&reply).contains("ForbiddenLoss"));
    assert_eq!(called.load(Ordering::Relaxed), 1);
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}

#[test]
fn upstream_io_error_detail_is_not_exposed_to_the_caller() {
    const CANARY: &str = "https://mock.invalid/?key=canary-private-provider-detail";
    struct FailingBody(Arc<AtomicUsize>);
    impl Read for FailingBody {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other(CANARY))
        }
    }
    impl UpstreamBody for FailingBody {
        fn cancel(&mut self) {
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    struct FailingTransport(Arc<AtomicUsize>);
    impl Transport for FailingTransport {
        fn execute(
            &self,
            _: &UpstreamRequest,
            _: &Cancellation,
        ) -> Result<UpstreamResponse, HandlerError> {
            Ok(UpstreamResponse::new(
                200,
                "application/json",
                FailingBody(self.0.clone()),
            ))
        }
    }
    let cancelled = Arc::new(AtomicUsize::new(0));
    let target = Target::new("http://mock.invalid", Protocol::OpenAiChat, "upstream");
    let mut server = Server::new(
        ServerConfig::default(),
        Gateway::new(target, FailingTransport(cancelled.clone())),
    )
    .unwrap();
    let address = bound(&mut server);
    let reply = post(address, &path(Protocol::OpenAiChat, false), INPUT, None);
    assert!(reply.starts_with(b"HTTP/1.1 502"));
    let text = String::from_utf8(reply).unwrap();
    assert!(text.contains("upstream read failed"));
    assert!(!text.contains(CANARY));
    assert!(!text.contains("canary-private-provider-detail"));
    assert_eq!(cancelled.load(Ordering::Relaxed), 1);
    assert!(server.shutdown(Duration::from_secs(1)).complete());
}
