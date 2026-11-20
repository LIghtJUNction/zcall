//! A real local HTTP gateway with an in-process fake upstream.
//! cargo run --features server --example server -- --check
use std::{
    io::{self, Cursor, Read, Write},
    net::{Shutdown, TcpStream},
    time::Duration,
};
use zcall::server::{
    Cancellation, HandlerError, Server, ServerConfig,
    gateway::{Gateway, ReaderBody, Target, Transport, UpstreamRequest, UpstreamResponse},
    protocol::{self, Protocol},
    transport::{Address, Listener},
};

struct FakeUpstream;
impl Transport for FakeUpstream {
    fn execute(
        &self,
        request: &UpstreamRequest,
        _: &Cancellation,
    ) -> Result<UpstreamResponse, HandlerError> {
        if request.stream {
            return Err(HandlerError::new(
                501,
                "this small example uses finite JSON; streaming is covered by server_gateway tests",
            ));
        }
        let value = br#"{"id":"mock_1","type":"message","role":"assistant","model":"fake-local","content":[{"type":"text","text":"local mock response"}],"stop_reason":"end_turn","usage":{"input_tokens":1,"output_tokens":3}}"#;
        Ok(UpstreamResponse::new(
            200,
            "application/json",
            ReaderBody(Cursor::new(value)),
        ))
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let check = std::env::args()
        .skip(1)
        .any(|argument| argument == "--check");
    let gateway = Gateway::new(
        Target::new(
            "http://in-process.invalid",
            Protocol::AnthropicMessages,
            "fake-local",
        ),
        FakeUpstream,
    );
    let mut server = Server::new(
        ServerConfig {
            workers: 2,
            queue: 4,
            ..ServerConfig::default()
        },
        gateway,
    )?;
    let address = server.listen(Listener::bind_tcp((
        "127.0.0.1",
        if check { 0 } else { 9416 },
    ))?)?;
    #[cfg(unix)]
    let Address::Tcp(address) = address else {
        unreachable!()
    };
    #[cfg(not(unix))]
    let Address::Tcp(address) = address;
    if check {
        let original = br#"{"model":"demo","messages":[{"role":"user","content":"hello"}]}"#;
        for protocol in [
            Protocol::OpenAiCompletions,
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::GeminiInteractions,
        ] {
            let body = protocol::prepare_request(
                Protocol::OpenAiChat,
                protocol,
                original,
                "demo",
                false,
                512,
            )?;
            let endpoint = protocol::endpoint("http://localhost", protocol, "demo", false)?;
            let path = endpoint
                .strip_prefix("http://localhost")
                .ok_or("invalid example endpoint")?;
            let mut client = TcpStream::connect(address)?;
            client.set_read_timeout(Some(Duration::from_secs(3)))?;
            client.set_write_timeout(Some(Duration::from_secs(3)))?;
            write!(
                client,
                "POST {path} HTTP/1.1\r\nHost: localhost\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n",
                body.len()
            )?;
            client.write_all(&body)?;
            client.shutdown(Shutdown::Write)?;
            let mut response = String::new();
            client.read_to_string(&mut response)?;
            if !response.starts_with("HTTP/1.1 200") || !response.contains("local mock response") {
                return Err(format!("local conversion failed for {protocol:?}: {response}").into());
            }
            println!("PASS {}", protocol::protocol_name(protocol));
        }
    } else {
        println!("Local mock gateway: http://{address}");
        println!("Example: POST /v1/chat/completions with model=demo and a messages array.");
        println!("No network provider or account is used. Press Enter to stop.");
        let mut input = String::new();
        io::stdin().read_line(&mut input)?;
    }
    let result = server.shutdown(Duration::from_secs(2));
    if !result.complete() {
        return Err(format!("cooperative shutdown incomplete: {result:?}").into());
    }
    Ok(())
}
