//! Bounded protocol adaptation. zcall owns AI semantics; this module owns buffers.
//!
//! Same-protocol requests retain all original JSON bytes except an explicitly
//! selected model/stream field. Cross-protocol conversion uses strict zcall
//! policies: unsupported fields are errors, never silently discarded.
use std::io::Write;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api::json;
pub use crate::v1::Protocol;
use crate::v1::{
    Control, EncodeContext, EncoderStatus, EncoderWorkspace, EventContext, EventData, EventEncoder,
    EventEncoderConfig, HttpEndpoint, Json, Limits, Parameter, Policy, Presence, RecordStorage,
    Request, Resource, Response, Session, SessionConfig, StageStatus, StepStatus, StreamWorkspace,
    Text, TokenStorage,
};

const MAX_DOCUMENT: usize = 16 * 1024 * 1024;
const MAX_MODEL: usize = 1024;
const JSON_TOKENS: usize = 32768;
const CONVERSION_TOKENS: usize = 8192;
const CONVERSION_RECORDS: usize = 1024;
const STREAM_FRAME: usize = 64 * 1024;
const STREAM_AGGREGATE: usize = 256 * 1024;
static IDS: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Eq, PartialEq)]
pub struct Inspection {
    pub model: String,
    pub stream: bool,
}

pub fn parse_protocol(value: &str) -> Result<Protocol, String> {
    match value {
        "chat" | "openai" | "openai-chat" | "chat-completions" => Ok(Protocol::OpenAiChat),
        "responses" | "openai-responses" => Ok(Protocol::OpenAiResponses),
        "anthropic" | "messages" | "anthropic-messages" => Ok(Protocol::AnthropicMessages),
        "gemini" | "generate-content" | "gemini-generate-content" => {
            Ok(Protocol::GeminiGenerateContent)
        }
        "completions" | "openai-completions" => Ok(Protocol::OpenAiCompletions),
        "interactions" | "gemini-interactions" => Ok(Protocol::GeminiInteractions),
        _ => Err(format!(
            "unknown protocol {value:?}; expected chat, responses, anthropic, gemini, completions, or interactions"
        )),
    }
}

pub fn protocol_name(protocol: Protocol) -> &'static str {
    match protocol {
        Protocol::OpenAiChat => "chat",
        Protocol::OpenAiResponses => "responses",
        Protocol::AnthropicMessages => "anthropic",
        Protocol::GeminiGenerateContent => "gemini",
        Protocol::OpenAiCompletions => "completions",
        Protocol::GeminiInteractions => "interactions",
    }
}

fn json_limits() -> json::Limits {
    json::Limits {
        max_depth: 64,
        max_input: MAX_DOCUMENT,
        max_tokens: JSON_TOKENS,
        max_object_fields: 4096,
    }
}

fn limits() -> Limits {
    Limits {
        max_depth: 64,
        max_input: MAX_DOCUMENT,
        max_tokens: CONVERSION_TOKENS,
        max_object_fields: 4096,
    }
}

/// A compact borrowed JSON index grows only when the actual document needs it.
/// zcall's JSON parser validates the entire input and rejects duplicate keys.
fn with_json<T>(
    body: &[u8],
    operation: impl FnOnce(json::Value<'_, '_>) -> Result<T, String>,
) -> Result<T, String> {
    let mut tokens = vec![json::Token::EMPTY; 128];
    loop {
        match json::parse(body, &mut tokens, json_limits()) {
            Ok(document) => {
                let root = document.root();
                if root.kind() != json::Kind::Object {
                    return Err("request body must be a JSON object".into());
                }
                return operation(root);
            }
            Err(error) => {
                if let zconv::Error::WorkspaceTooSmall { needed } = error
                    && tokens.len() < JSON_TOKENS
                {
                    let next = tokens.len().saturating_mul(2).max(needed);
                    if next <= JSON_TOKENS {
                        tokens.resize(next, json::Token::EMPTY);
                        continue;
                    }
                }
                return Err(error.to_string());
            }
        }
    }
}

fn valid_model(model: &str) -> Result<(), String> {
    if model.is_empty() || model.len() > MAX_MODEL || model.chars().any(char::is_control) {
        return Err(format!(
            "model must contain 1..={MAX_MODEL} bytes without control characters"
        ));
    }
    Ok(())
}

fn json_model(value: json::Value<'_, '_>) -> Result<String, String> {
    let text = value.as_string().ok_or("model must be a JSON string")?;
    if text.decoded_len() > MAX_MODEL {
        return Err(format!("model exceeds {MAX_MODEL} bytes"));
    }
    let mut decoded = vec![0; text.decoded_len()];
    text.decode_into(&mut decoded).map_err(|e| e.to_string())?;
    let model = String::from_utf8(decoded).map_err(|_| "model is not UTF-8")?;
    valid_model(&model)?;
    Ok(model)
}

pub fn inspect(
    protocol: Protocol,
    body: &[u8],
    path_model: Option<&str>,
) -> Result<Inspection, String> {
    with_json(body, |root| {
        let model = if protocol == Protocol::GeminiGenerateContent {
            let model =
                path_model.ok_or("Gemini GenerateContent requires a model in the request path")?;
            valid_model(model)?;
            model.to_owned()
        } else {
            json_model(root.get("model").ok_or("request body has no model")?)?
        };
        let stream = match root.get("stream") {
            Some(value) => value.as_bool().ok_or("stream must be a JSON boolean")?,
            None => false,
        };
        // GenerateContent's streaming action belongs to the HTTP path. The
        // listener must combine that action with this body-only inspection.
        Ok(Inspection { model, stream })
    })
}

pub fn endpoint(
    base: &str,
    protocol: Protocol,
    model: &str,
    stream: bool,
) -> Result<String, String> {
    valid_model(model)?;
    let endpoint = HttpEndpoint::new(base, protocol.into()).map_err(|e| e.to_string())?;
    let mut buffer = vec![
        0;
        base.len()
            .saturating_add(model.len().saturating_mul(3))
            .saturating_add(128)
    ];
    let url = endpoint
        .url_into(Some(model), stream, &mut buffer)
        .map_err(|e| e.to_string())?;
    Ok(url.to_owned())
}

fn quote(value: &str) -> Result<Vec<u8>, String> {
    let mut output = vec![0; value.len().saturating_mul(6).saturating_add(2)];
    let mut writer = json::Writer::new(&mut output);
    writer.string(value.as_bytes());
    let written = writer.finish().map_err(|e| e.to_string())?;
    output.truncate(written);
    Ok(output)
}

/// Patch selected scalar fields without normalizing provider extensions,
/// whitespace, large numeric values, or message content.
fn patch_request(
    protocol: Protocol,
    body: &[u8],
    model: &str,
    stream: bool,
) -> Result<Vec<u8>, String> {
    with_json(body, |root| {
        if protocol == Protocol::GeminiGenerateContent {
            return Ok(body.to_vec());
        }
        let model_json = quote(model)?;
        let stream_json: &[u8] = if stream { b"true" } else { b"false" };
        let mut edits: Vec<(usize, usize, &[u8])> = Vec::with_capacity(2);
        let mut missing = Vec::new();
        for (key, replacement) in [("model", model_json.as_slice()), ("stream", stream_json)] {
            if let Some(value) = root.get(key) {
                let raw = value.raw();
                if raw != replacement {
                    let start = raw.as_ptr() as usize - body.as_ptr() as usize;
                    edits.push((start, start + raw.len(), replacement));
                }
            } else if key == "model" || stream {
                missing.push((key, replacement));
            }
        }
        edits.sort_by_key(|edit| edit.0);
        let mut result = Vec::with_capacity(body.len() + model_json.len() + 32);
        let mut offset = 0;
        for (start, end, replacement) in edits {
            result.extend_from_slice(&body[offset..start]);
            result.extend_from_slice(replacement);
            offset = end;
        }
        let close = root.raw().as_ptr() as usize - body.as_ptr() as usize + root.raw().len() - 1;
        result.extend_from_slice(&body[offset..close]);
        let mut comma = root.object().next().is_some();
        for (key, value) in missing {
            if comma {
                result.push(b',');
            }
            comma = true;
            result.extend_from_slice(if key == "model" {
                b"\"model\":"
            } else {
                b"\"stream\":"
            });
            result.extend_from_slice(value);
        }
        result.extend_from_slice(&body[close..]);
        Ok(result)
    })
}

fn resize_for_error(
    error: crate::v1::Error,
    output: &mut Vec<u8>,
    scratch: &mut Vec<u8>,
) -> Result<(), String> {
    let Some(capacity) = error.capacity else {
        return Err(error.to_string());
    };
    let buffer = match capacity.region {
        Resource::Output => output,
        Resource::Scratch => scratch,
        _ => return Err(error.to_string()),
    };
    let next = capacity
        .needed
        .max(buffer.len().saturating_mul(2).min(MAX_DOCUMENT))
        .max(4096);
    if next > MAX_DOCUMENT || next <= buffer.len() {
        return Err(format!(
            "conversion buffer limit ({MAX_DOCUMENT} bytes): {error}"
        ));
    }
    buffer.resize(next, 0);
    Ok(())
}

pub fn prepare_request(
    source: Protocol,
    target: Protocol,
    body: &[u8],
    target_model: &str,
    stream: bool,
    default_max_tokens: u64,
) -> Result<Vec<u8>, String> {
    valid_model(target_model)?;
    if source == target {
        return patch_request(source, body, target_model, stream);
    }
    let mut tokens = Box::new(TokenStorage::<CONVERSION_TOKENS>::new());
    let mut records = Box::new(RecordStorage::<CONVERSION_RECORDS>::new());
    let mut request = Request::decode(source.into(), body, &mut tokens, &mut records, limits())
        .map_err(|e| e.to_string())?;
    request.set_model(Presence::Value(Text::new(target_model)));
    let mut scalar_tokens = TokenStorage::<1>::new();
    let stream_value = Json::parse(
        if stream { b"true" } else { b"false" },
        &mut scalar_tokens,
        Limits::default(),
    )
    .map_err(|e| e.to_string())?;
    request
        .set_parameter(Parameter::Stream, Presence::Value(stream_value))
        .map_err(|e| e.to_string())?;
    let mut context = EncodeContext::default().with_model(target_model);
    if default_max_tokens != 0 {
        context = context.with_max_tokens(default_max_tokens);
    }
    let mut output = vec![0; body.len().saturating_add(1024).clamp(4096, MAX_DOCUMENT)];
    let mut scratch = vec![0; body.len().clamp(4096, STREAM_FRAME)];
    loop {
        match request.encode(
            target.into(),
            &mut output,
            &mut scratch,
            &mut tokens,
            context,
            Policy::default(),
        ) {
            Ok(report) => {
                if report.has_losses() {
                    return Err(
                        "strict request conversion unexpectedly reported semantic loss".into(),
                    );
                }
                output.truncate(report.written);
                return Ok(output);
            }
            Err(error) => resize_for_error(error, &mut output, &mut scratch)?,
        }
    }
}

fn now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| duration.as_secs())
}

fn gateway_id(timestamp: u64) -> String {
    format!(
        "zcalld-{timestamp}-{}-{}",
        std::process::id(),
        IDS.fetch_add(1, Ordering::Relaxed)
    )
}

/// UTC timestamp for target formats requiring an ISO timestamp. It identifies
/// this gateway conversion when the source omitted that fact.
fn utc_timestamp(timestamp: u64) -> String {
    let days = (timestamp / 86400) as i64;
    let seconds = timestamp % 86400;
    let z = days + 719468;
    let era = z / 146097;
    let day_of_era = z - era * 146097;
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36524 - day_of_era / 146096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    year += i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        seconds / 3600,
        seconds / 60 % 60,
        seconds % 60
    )
}

pub fn convert_response(
    source: Protocol,
    target: Protocol,
    body: &[u8],
    model: &str,
) -> Result<Vec<u8>, String> {
    if source == target {
        return Ok(body.to_vec());
    }
    valid_model(model)?;
    let timestamp = now();
    let id = gateway_id(timestamp);
    let iso = utc_timestamp(timestamp);
    let mut tokens = Box::new(TokenStorage::<CONVERSION_TOKENS>::new());
    let mut records = Box::new(RecordStorage::<CONVERSION_RECORDS>::new());
    let mut response = Response::decode(source.into(), body, &mut tokens, &mut records, limits())
        .map_err(|e| e.to_string())?;
    // Target-only identities are supplied explicitly, never substituted for
    // source-owned identities. Strict encoding retains/rejects source metadata.
    let item_count = response
        .choices()
        .map(|choice| choice.contents().count() + 1)
        .sum::<usize>();
    let item_ids: Vec<String> = (0..item_count).map(|i| format!("{id}-item-{i}")).collect();
    let item_refs: Vec<&str> = item_ids.iter().map(String::as_str).collect();
    let context = EncodeContext::default()
        .with_model(model)
        .with_response_id(&id)
        .with_created(timestamp)
        .with_created_iso(&iso)
        .with_updated_iso(&iso)
        .with_item_ids(&item_refs);
    let mut output = vec![0; body.len().saturating_add(1024).clamp(4096, MAX_DOCUMENT)];
    let mut scratch = vec![0; body.len().clamp(4096, STREAM_FRAME)];
    loop {
        match response.encode(
            target.into(),
            &mut output,
            &mut scratch,
            &mut tokens,
            context,
            Policy::default(),
        ) {
            Ok(report) => {
                if report.has_losses() {
                    return Err(
                        "strict response conversion unexpectedly reported semantic loss".into(),
                    );
                }
                output.truncate(report.written);
                return Ok(output);
            }
            Err(error) => resize_for_error(error, &mut output, &mut scratch)?,
        }
    }
}

pub type StreamTransform<'a> = dyn FnMut(&[u8], bool, &mut dyn Write) -> Result<(), String> + 'a;

fn drain_encoder(encoder: &mut EventEncoder<'_, '_>, sink: &mut dyn Write) -> Result<(), String> {
    let mut output = [0; 8192];
    loop {
        let step = encoder.drain(&mut output);
        if step.written != 0 {
            sink.write_all(&output[..step.written])
                .map_err(|e| format!("downstream write: {e}"))?;
        }
        match step.status {
            EncoderStatus::Ready | EncoderStatus::Done => return Ok(()),
            EncoderStatus::NeedOutput if step.written != 0 => continue,
            EncoderStatus::NeedOutput => {
                return Err("stream encoder made no output progress".into());
            }
            EncoderStatus::NeedBuffer(capacity) => {
                return Err(format!("stream encoder capacity exceeded: {capacity:?}"));
            }
            EncoderStatus::Failed(error) => return Err(error.to_string()),
        }
    }
}

/// Run a synchronous upstream transport with a safely borrowed converter.
/// Setting eof validates stream termination. The caller must propagate its error
/// and close the response; it must never replay a partly delivered generation.
///
/// Cross-protocol SSE uses fixed-capacity workspaces. Oversized single frames,
/// too many simultaneous items, and retained tool/final-snapshot data exceeding
/// the aggregate budget fail explicitly. The same-protocol path is unbuffered.
pub fn with_stream_converter<T>(
    source: Protocol,
    target: Protocol,
    model: &str,
    run: impl FnOnce(&mut StreamTransform<'_>) -> T,
) -> Result<T, String> {
    valid_model(model)?;
    if source == target {
        let mut complete = false;
        let mut passthrough = |bytes: &[u8], eof: bool, sink: &mut dyn Write| {
            if complete {
                return Err("stream has already ended".into());
            }
            sink.write_all(bytes)
                .map_err(|e| format!("downstream write: {e}"))?;
            sink.flush().map_err(|e| format!("downstream flush: {e}"))?;
            complete = eof;
            Ok(())
        };
        return Ok(run(&mut passthrough));
    }
    let timestamp = now();
    let id = gateway_id(timestamp);
    let mut framing = vec![0; STREAM_FRAME];
    let mut payload = vec![0; STREAM_FRAME];
    let mut aggregate = vec![0; STREAM_AGGREGATE];
    let mut workspace = Box::new(StreamWorkspace::<4096, 128, 128>::new());
    let mut config = SessionConfig::new(source.into());
    config.limits = limits();
    let mut session = Session::new(
        config,
        &mut framing,
        &mut payload,
        &mut aggregate,
        &mut workspace,
    )
    .map_err(|e| e.to_string())?;
    session.prepare().map_err(|e| e.to_string())?;

    let mut pending = vec![0; STREAM_AGGREGATE];
    let mut encoded_aggregate = vec![0; STREAM_AGGREGATE];
    let mut scratch = vec![0; STREAM_FRAME];
    let mut encoder_workspace = Box::new(EncoderWorkspace::<4096, 128>::new());
    let mut encoder_config = EventEncoderConfig::new(source.into(), target.into());
    encoder_config.limits = limits();
    encoder_config.context = EventContext {
        response_id: Some(&id),
        model: Some(model),
        created: Some(timestamp),
        target_item_id_prefix: Some(&id),
        ..EventContext::default()
    };
    let mut encoder = EventEncoder::new(
        encoder_config,
        &mut pending,
        &mut encoded_aggregate,
        &mut scratch,
        &mut encoder_workspace,
    )
    .map_err(|e| e.to_string())?;
    let mut complete = false;
    let mut failed = false;
    let mut transform = |bytes: &[u8], eof: bool, sink: &mut dyn Write| -> Result<(), String> {
        if failed || complete {
            return Err("stream converter is no longer accepting input".into());
        }
        let result = (|| {
            let mut offset = 0;
            loop {
                let mut event_error = None;
                let step = session.step(&bytes[offset..], eof, |event| {
                    if let EventData::Terminal(terminal) = event.data()
                        && let Some(error) = terminal.error
                    {
                        event_error = Some(error.to_string());
                        return Control::Pause;
                    }
                    loop {
                        match encoder.push(event) {
                            StageStatus::Accepted(losses) if losses.is_empty() => {
                                if let Err(error) = drain_encoder(&mut encoder, sink) {
                                    event_error = Some(error);
                                    return Control::Pause;
                                }
                                return Control::Continue;
                            }
                            StageStatus::Accepted(_) => {
                                event_error = Some(
                                    "strict stream conversion unexpectedly reported semantic loss"
                                        .into(),
                                );
                                return Control::Pause;
                            }
                            StageStatus::NeedOutput => {
                                if let Err(error) = drain_encoder(&mut encoder, sink) {
                                    event_error = Some(error);
                                    return Control::Pause;
                                }
                            }
                            StageStatus::NeedBuffer(capacity) => {
                                event_error =
                                    Some(format!("stream encoder capacity exceeded: {capacity:?}"));
                                return Control::Pause;
                            }
                            StageStatus::Failed(error) => {
                                event_error = Some(error.to_string());
                                return Control::Pause;
                            }
                        }
                    }
                });
                offset += step.consumed;
                if let Some(error) = event_error {
                    return Err(error);
                }
                match step.status {
                    StepStatus::Done => {
                        if offset != bytes.len() {
                            return Err("unexpected bytes after terminal stream event".into());
                        }
                        break;
                    }
                    StepStatus::NeedInput if offset == bytes.len() => break,
                    StepStatus::NeedInput | StepStatus::NeedEvents => {
                        if step.consumed == 0 && step.emitted == 0 {
                            return Err("stream decoder made no progress".into());
                        }
                    }
                    StepStatus::NeedBuffer(capacity) => {
                        return Err(format!("stream decoder capacity exceeded: {capacity:?}"));
                    }
                    StepStatus::Failed(error) => return Err(error.to_string()),
                }
            }
            drain_encoder(&mut encoder, sink)?;
            sink.flush().map_err(|e| format!("downstream flush: {e}"))?;
            Ok(())
        })();
        if result.is_err() {
            failed = true;
        } else if eof {
            complete = true;
        }
        result
    };
    Ok(run(&mut transform))
}

pub fn ingress(path: &str) -> Result<(Protocol, Option<String>, bool), String> {
    let protocol = match path {
        "/v1/chat/completions" | "/chat/completions" => Some(Protocol::OpenAiChat),
        "/v1/responses" | "/responses" => Some(Protocol::OpenAiResponses),
        "/v1/messages" | "/messages" => Some(Protocol::AnthropicMessages),
        "/v1/completions" | "/completions" => Some(Protocol::OpenAiCompletions),
        "/v1beta/interactions" | "/v1/interactions" | "/interactions" => {
            Some(Protocol::GeminiInteractions)
        }
        _ => None,
    };
    if let Some(protocol) = protocol {
        return Ok((protocol, None, false));
    }
    let tail = path
        .strip_prefix("/v1beta/models/")
        .or_else(|| path.strip_prefix("/v1/models/"));
    if let Some(tail) = tail {
        let (model, stream) = if let Some(model) = tail.strip_suffix(":streamGenerateContent") {
            (model, true)
        } else if let Some(model) = tail.strip_suffix(":generateContent") {
            (model, false)
        } else {
            return Err("unknown Gemini endpoint".into());
        };
        let bytes = model.as_bytes();
        let mut decoded = Vec::with_capacity(bytes.len());
        let mut index = 0;
        while index < bytes.len() {
            if bytes[index] == b'%' {
                let value = bytes
                    .get(index + 1..index + 3)
                    .and_then(|s| std::str::from_utf8(s).ok())
                    .and_then(|s| u8::from_str_radix(s, 16).ok())
                    .ok_or("invalid model URL escape")?;
                decoded.push(value);
                index += 3;
            } else {
                decoded.push(bytes[index]);
                index += 1;
            }
        }
        let model = String::from_utf8(decoded).map_err(|_| "model must be UTF-8")?;
        if model.is_empty() || model.chars().any(char::is_control) {
            return Err("invalid model".into());
        }
        return Ok((Protocol::GeminiGenerateContent, Some(model), stream));
    }
    Err("unknown gateway endpoint".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn protocol_names_round_trip() {
        for p in [
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::OpenAiCompletions,
            Protocol::GeminiInteractions,
        ] {
            assert_eq!(parse_protocol(protocol_name(p)).unwrap(), p);
        }
        assert!(parse_protocol("auto").is_err());
    }

    #[test]
    fn inspection_rejects_ambiguity_and_decodes_model() {
        let r = inspect(
            Protocol::OpenAiChat,
            br#"{"model":"a\u002fb","stream":true,"messages":[]}"#,
            None,
        )
        .unwrap();
        assert_eq!(
            r,
            Inspection {
                model: "a/b".into(),
                stream: true
            }
        );
        assert!(inspect(Protocol::OpenAiChat, br#"{"model":"a","model":"b"}"#, None).is_err());
        assert!(
            inspect(
                Protocol::OpenAiChat,
                br#"{"model":"a","stream":"true"}"#,
                None
            )
            .is_err()
        );
        assert!(inspect(Protocol::OpenAiChat, b"[]", None).is_err());
        assert_eq!(
            inspect(
                Protocol::GeminiGenerateContent,
                br#"{"contents":[]}"#,
                Some("models/gemini")
            )
            .unwrap()
            .model,
            "models/gemini"
        );
    }

    #[test]
    fn same_protocol_patch_preserves_extensions_and_integers() {
        let body = br#" { "model" : "alias", "opaque":18446744073709551615, "messages":[{"role":"user","content":"model alias"}] } "#;
        let result = prepare_request(
            Protocol::OpenAiChat,
            Protocol::OpenAiChat,
            body,
            "real",
            false,
            100,
        )
        .unwrap();
        assert_eq!(result, br#" { "model" : "real", "opaque":18446744073709551615, "messages":[{"role":"user","content":"model alias"}] } "#);
        assert_eq!(
            prepare_request(
                Protocol::GeminiGenerateContent,
                Protocol::GeminiGenerateContent,
                br#"{"contents":[]}"#,
                "x",
                true,
                100
            )
            .unwrap(),
            br#"{"contents":[]}"#
        );
    }

    #[test]
    fn endpoint_uses_zcall_version_and_gemini_stream_rules() {
        assert_eq!(
            endpoint("https://example.com/v1", Protocol::OpenAiChat, "a", false).unwrap(),
            "https://example.com/v1/chat/completions"
        );
        let url = endpoint(
            "https://example.com",
            Protocol::GeminiGenerateContent,
            "g/a",
            true,
        )
        .unwrap();
        assert!(url.contains("g%2Fa:streamGenerateContent"));
        assert!(url.contains("alt=sse"));
    }

    #[test]
    fn cross_protocol_request_is_real_and_strict() {
        let body = br#"{"model":"alias","messages":[{"role":"user","content":"hello"}]}"#;
        let result = prepare_request(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            body,
            "claude",
            false,
            512,
        )
        .unwrap();
        let text = String::from_utf8(result).unwrap();
        assert!(text.contains("\"max_tokens\":512"));
        assert!(text.contains("\"model\":\"claude\""));
        assert!(text.contains("hello"));
        let unsupported = br#"{"model":"x","messages":[{"role":"user","content":"hi"}],"vendor_private":{"x":1}}"#;
        assert!(
            prepare_request(
                Protocol::OpenAiChat,
                Protocol::AnthropicMessages,
                unsupported,
                "y",
                false,
                512
            )
            .is_err()
        );
    }

    #[test]
    fn cross_protocol_response_has_content_usage_and_explicit_metadata() {
        let response = br#"{"id":"msg_1","type":"message","role":"assistant","model":"claude","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","stop_sequence":null,"usage":{"input_tokens":3,"output_tokens":2}}"#;
        let output = convert_response(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            response,
            "alias",
        )
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("\"content\":\"hello\""));
        assert!(text.contains("\"prompt_tokens\":3"));
        assert!(text.contains("\"completion_tokens\":2"));
        assert!(text.contains("\"created\":"));
    }

    #[test]
    fn fragmented_sse_converts_incrementally() {
        let source = concat!(
            "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"msg_1\",\"type\":\"message\",\"role\":\"assistant\",\"model\":\"m\",\"content\":[],\"usage\":{\"input_tokens\":3,\"output_tokens\":0}}}\n\n",
            "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
            "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"hello 界\"}}\n\n",
            "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
        );
        let mut output = Vec::new();
        with_stream_converter(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "m",
            |convert| -> Result<(), String> {
                for chunk in source.as_bytes().chunks(7) {
                    convert(chunk, false, &mut output)?;
                }
                assert!(!output.is_empty());
                convert(&[], true, &mut output)
            },
        )
        .unwrap()
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("hello 界"));
        assert!(text.contains("[DONE]"));
    }

    #[test]
    fn truncated_sse_is_an_error() {
        let source = b"data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"role\":\"assistant\",\"model\":\"x\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n";
        let result = with_stream_converter(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "m",
            |convert| {
                let mut output = Vec::new();
                convert(source, true, &mut output)
            },
        )
        .unwrap();
        assert!(result.is_err());
    }

    #[test]
    fn timestamp_format_uses_utc_calendar() {
        assert_eq!(utc_timestamp(0), "1970-01-01T00:00:00Z");
        assert_eq!(utc_timestamp(951782400), "2000-02-29T00:00:00Z");
    }
    #[test]
    fn six_protocols_accept_simple_requests_through_zcall() {
        let body = br#"{"model":"input","messages":[{"role":"user","content":"hello"}]}"#;
        for target in [
            Protocol::OpenAiCompletions,
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::GeminiInteractions,
        ] {
            let output = prepare_request(Protocol::OpenAiChat, target, body, "target", false, 512)
                .unwrap_or_else(|e| panic!("{target:?}: {e}"));
            assert!(
                String::from_utf8_lossy(&output).contains("hello"),
                "{target:?}"
            );
            let from_output =
                prepare_request(target, Protocol::OpenAiChat, &output, "return", false, 512)
                    .unwrap_or_else(|e| panic!("return from {target:?}: {e}"));
            assert!(
                String::from_utf8_lossy(&from_output).contains("hello"),
                "{target:?}"
            );
        }
    }

    #[test]
    fn six_protocols_encode_simple_complete_responses() {
        let input = br#"{"id":"msg_1","type":"message","role":"assistant","model":"m","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn","usage":{"input_tokens":3,"output_tokens":2}}"#;
        for target in [
            Protocol::OpenAiCompletions,
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            Protocol::AnthropicMessages,
            Protocol::GeminiGenerateContent,
            Protocol::GeminiInteractions,
        ] {
            let output = convert_response(Protocol::AnthropicMessages, target, input, "m")
                .unwrap_or_else(|e| panic!("{target:?}: {e}"));
            assert!(
                String::from_utf8_lossy(&output).contains("hello"),
                "{target:?}"
            );
        }
    }

    #[test]
    fn chat_to_responses_stream_keeps_text_and_finishes() {
        let input = concat!(
            "data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"hello\"},\"finish_reason\":null}]}\n\n",
            "data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
            "data: [DONE]\n\n"
        );
        let mut output = Vec::new();
        with_stream_converter(
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            "m",
            |convert| {
                for bytes in input.as_bytes().chunks(3) {
                    convert(bytes, false, &mut output)?;
                }
                convert(&[], true, &mut output)
            },
        )
        .unwrap()
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        assert!(text.contains("response.output_text.delta"));
        assert!(text.contains("response.completed"));
        assert!(text.contains("hello"));
    }

    #[test]
    fn tool_stream_preserves_call_name_id_and_arguments() {
        let input = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"r\",\"model\":\"m\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"tool_use\",\"id\":\"tool_1\",\"name\":\"weather\",\"input\":{}}}\n\n",
            "data: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"input_json_delta\",\"partial_json\":\"{\\\"city\\\":\\\"Taipei\\\"}\"}}\n\n",
            "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
            "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"tool_use\"},\"usage\":{\"output_tokens\":2}}\n\n",
            "data: {\"type\":\"message_stop\"}\n\n"
        );
        let mut output = Vec::new();
        with_stream_converter(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "m",
            |convert| {
                for bytes in input.as_bytes().chunks(11) {
                    convert(bytes, false, &mut output)?;
                }
                convert(&[], true, &mut output)
            },
        )
        .unwrap()
        .unwrap();
        let text = String::from_utf8(output).unwrap();
        for expected in ["tool_1", "weather", "Taipei", "tool_calls", "[DONE]"] {
            assert!(text.contains(expected), "missing {expected}: {text}");
        }
    }

    #[test]
    fn strict_stream_rejects_source_timestamp_loss() {
        let input = b"data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello\"},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let result = with_stream_converter(
            Protocol::OpenAiChat,
            Protocol::AnthropicMessages,
            "m",
            |convert| convert(input, true, &mut Vec::new()),
        )
        .and_then(|result| result);
        let error = result.unwrap_err();
        assert!(error.contains("created") || error.contains("initial_usage.input"));
    }

    #[test]
    fn downstream_errors_stop_stream_conversion() {
        struct FailedSink;
        impl Write for FailedSink {
            fn write(&mut self, _: &[u8]) -> std::io::Result<usize> {
                Err(std::io::Error::new(
                    std::io::ErrorKind::BrokenPipe,
                    "closed",
                ))
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let input = concat!(
            "data: {\"type\":\"message_start\",\"message\":{\"id\":\"m\",\"model\":\"x\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
            "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"hello\"}}\n\n"
        ).as_bytes();
        with_stream_converter(
            Protocol::AnthropicMessages,
            Protocol::OpenAiChat,
            "m",
            |convert| {
                assert!(
                    convert(input, false, &mut FailedSink)
                        .unwrap_err()
                        .contains("downstream")
                );
                assert!(convert(&[], true, &mut Vec::new()).is_err());
            },
        )
        .unwrap();
    }

    #[test]
    fn inspection_grows_index_for_real_conversation() {
        let messages = std::iter::repeat_n(r#"{"role":"user","content":"hello"}"#, 200)
            .collect::<Vec<_>>()
            .join(",");
        let input = format!(r#"{{"model":"large","messages":[{messages}]}}"#);
        assert_eq!(
            inspect(Protocol::OpenAiChat, input.as_bytes(), None)
                .unwrap()
                .model,
            "large"
        );
    }
}
