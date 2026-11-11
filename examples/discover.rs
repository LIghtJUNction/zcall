//! A std application using curl as its transport; zcall remains Sans-I/O.
//! Run: cargo run --example discover -- BASE_URL MODEL [auto|chat|responses|...]
//! API_KEY is optional for unauthenticated local servers. Auto sends generation probes.
use std::error::Error as StdError;
use std::io::{self, Read, Write};
use std::process::{Command, Stdio};
use zcall::v1::*;

type Result<T> = std::result::Result<T, Box<dyn StdError>>;
const MAX_BODY: usize = 64 * 1024;
const MAX_CAPTURE: usize = MAX_BODY + 2048;
const STATUS_MARKER: &[u8] = b"\nZCALL_HTTP_STATUS:";

struct HttpResponse {
    status: u16,
    content_type: String,
    body: Vec<u8>,
}

fn main() -> Result<()> {
    let args: Vec<_> = std::env::args().skip(1).collect();
    if !(2..=3).contains(&args.len()) {
        return Err("usage: discover BASE_URL MODEL [auto|chat|responses|completions|anthropic|gemini|interactions]".into());
    }
    let (base_url, model) = (&args[0], &args[1]);
    let mode = args.get(2).map(String::as_str).unwrap_or("auto");
    let mut url = [0; 4096];
    if mode != "auto" {
        let protocol = parse_protocol(mode)?;
        let endpoint = HttpEndpoint::new(base_url, protocol.into())?;
        display_endpoint(endpoint, model, &mut url)?;
        return Ok(());
    }

    let api_key = std::env::var("API_KEY").ok();
    let mut config = DiscoveryConfig::new(model);
    // Some Chat-compatible models require the modern token-limit field.
    if std::env::var("CHAT_TOKEN_LIMIT").as_deref() == Ok("max_completion_tokens") {
        config.chat_token_limit = ChatTokenLimit::MaxCompletionTokens;
    }
    let mut discovery = Discovery::with_config(base_url, config)?;
    let mut body = [0; 4096];
    let mut tokens = TokenStorage::<2048>::new();
    let limits = Limits {
        max_input: MAX_BODY,
        max_tokens: 2048,
        max_depth: 32,
        max_object_fields: 1024,
    };
    eprintln!("Auto discovery sends bounded generation probes and may consume provider credits.");
    while let Some(probe) = discovery.next_probe(&mut url, &mut body)? {
        let format = probe.format;
        let outcome = match send(&probe, api_key.as_deref()) {
            Ok(response) => discovery.observe(
                response.status,
                (!response.content_type.is_empty()).then_some(response.content_type.as_str()),
                &response.body,
                &mut tokens,
                limits,
            )?,
            Err(error) => {
                eprintln!("{:?}: {error}", format.protocol);
                discovery.transport_failed()?
            }
        };
        eprintln!("{:?}: {outcome:?}", format.protocol);
    }
    let endpoint = discovery
        .selected()
        .ok_or("no candidate returned a verified protocol response")?;
    display_endpoint(endpoint, model, &mut url)?;
    Ok(())
}

fn parse_protocol(name: &str) -> Result<Protocol> {
    match name {
        "chat" => Ok(Protocol::OpenAiChat),
        "responses" => Ok(Protocol::OpenAiResponses),
        "completions" => Ok(Protocol::OpenAiCompletions),
        "anthropic" => Ok(Protocol::AnthropicMessages),
        "gemini" => Ok(Protocol::GeminiGenerateContent),
        "interactions" => Ok(Protocol::GeminiInteractions),
        _ => Err("unknown protocol; use auto, chat, responses, completions, anthropic, gemini or interactions".into()),
    }
}

fn display_endpoint(endpoint: HttpEndpoint<'_>, model: &str, output: &mut [u8]) -> Result<()> {
    let url = endpoint.url_into(Some(model), false, output)?;
    // Query values may contain routing secrets; omit them from diagnostic output.
    println!("protocol={:?}", endpoint.format().protocol);
    println!("endpoint={}", url.split('?').next().unwrap_or(url));
    Ok(())
}

fn send(probe: &Probe<'_>, api_key: Option<&str>) -> Result<HttpResponse> {
    let mut config = String::new();
    option(&mut config, "url", probe.url)?;
    option(
        &mut config,
        "header",
        &format!("Content-Type: {}", probe.content_type),
    )?;
    option(&mut config, "header", &format!("Accept: {}", probe.accept))?;
    if let Some((name, value)) = probe.version_header {
        option(&mut config, "header", &format!("{name}: {value}"))?;
    }
    if let Some(key) = api_key {
        let header = match probe.format.protocol {
            Protocol::AnthropicMessages => format!("x-api-key: {key}"),
            Protocol::GeminiGenerateContent | Protocol::GeminiInteractions => {
                format!("x-goog-api-key: {key}")
            }
            _ => format!("Authorization: Bearer {key}"),
        };
        option(&mut config, "header", &header)?;
    }
    option(&mut config, "data-binary", std::str::from_utf8(probe.body)?)?;
    // --disable ignores ~/.curlrc, so it cannot enable redirects or verbose logs.
    // Credentials and URL are passed through stdin, never command-line arguments.
    let mut child = Command::new("curl")
        .args([
            "--disable",
            "--config",
            "-",
            "--silent",
            "--globoff",
            "--request",
            probe.method,
            "--connect-timeout",
            "5",
            "--max-time",
            "20",
            "--max-redirs",
            "0",
            "--proto",
            "=http,https",
            "--max-filesize",
            "65536",
            "--write-out",
            "\nZCALL_HTTP_STATUS:%{http_code}\nZCALL_CONTENT_TYPE:%{content_type}\n",
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()?;
    let result = (|| -> Result<Vec<u8>> {
        child
            .stdin
            .take()
            .ok_or("curl stdin unavailable")?
            .write_all(config.as_bytes())?;
        let mut capture = Vec::with_capacity(MAX_CAPTURE + 1);
        child
            .stdout
            .take()
            .ok_or("curl stdout unavailable")?
            .take((MAX_CAPTURE + 1) as u64)
            .read_to_end(&mut capture)?;
        if capture.len() > MAX_CAPTURE {
            return Err("HTTP response exceeded the capture limit".into());
        }
        Ok(capture)
    })();
    let capture = match result {
        Ok(capture) => capture,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let status = child.wait()?;
    if !status.success() {
        return Err(io::Error::other(format!("curl transport failed ({status})")).into());
    }
    parse_response(capture)
}

fn option(config: &mut String, name: &str, value: &str) -> Result<()> {
    if value.chars().any(char::is_control) {
        return Err("control characters are not allowed in HTTP configuration".into());
    }
    config.push_str(name);
    config.push_str(" = \"");
    for c in value.chars() {
        if matches!(c, '\\' | '"') {
            config.push('\\');
        }
        config.push(c);
    }
    config.push_str("\"\n");
    Ok(())
}

fn parse_response(mut capture: Vec<u8>) -> Result<HttpResponse> {
    let split = capture
        .windows(STATUS_MARKER.len())
        .rposition(|w| w == STATUS_MARKER)
        .ok_or("curl returned no HTTP status")?;
    if split > MAX_BODY {
        return Err("HTTP response body exceeded the limit".into());
    }
    let metadata = std::str::from_utf8(&capture[split + STATUS_MARKER.len()..])?;
    let (status, content_type) = metadata
        .split_once("\nZCALL_CONTENT_TYPE:")
        .ok_or("curl returned invalid HTTP metadata")?;
    let status = status.parse()?;
    let content_type = content_type.trim_end_matches('\n').to_owned();
    capture.truncate(split);
    Ok(HttpResponse {
        status,
        content_type,
        body: capture,
    })
}
