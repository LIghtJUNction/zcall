//! Bounded, caller-driven protocol discovery. The caller sends each probe once.

use crate::common::*;
use crate::endpoint::{EndpointError, HttpEndpoint, url_protocol};
use zconv::json::{Kind, Value, Writer};

/// Default preference when a service supports several wire protocols.
pub const DEFAULT_DISCOVERY_FORMATS: [Format; 6] = [
    Format::new(Protocol::OpenAiChat),
    Format::new(Protocol::OpenAiResponses),
    Format::new(Protocol::AnthropicMessages),
    Format::new(Protocol::GeminiGenerateContent),
    Format::new(Protocol::GeminiInteractions),
    Format::new(Protocol::OpenAiCompletions),
];

/// Select the output limit accepted by the caller's Chat deployment/model.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ChatTokenLimit {
    MaxTokens,
    MaxCompletionTokens,
}

/// A small generation probe, using a model selected by the caller.
/// Sending a probe can incur provider charges. No user conversation is sent.
#[derive(Clone, Copy, Debug)]
pub struct DiscoveryConfig<'a> {
    pub model: &'a str,
    /// Preference order, with one to six distinct protocols.
    pub formats: &'a [Format],
    pub max_output_tokens: u32,
    pub chat_token_limit: ChatTokenLimit,
}

impl<'a> DiscoveryConfig<'a> {
    pub const fn new(model: &'a str) -> Self {
        Self {
            model,
            formats: &DEFAULT_DISCOVERY_FORMATS,
            max_output_tokens: 16,
            chat_token_limit: ChatTokenLimit::MaxTokens,
        }
    }
}

/// A complete HTTP request description borrowing the caller's output buffers.
/// Add credentials in the transport; disable automatic cross-origin redirects.
#[derive(Clone, Copy, Debug)]
pub struct Probe<'a> {
    pub format: Format,
    pub method: &'static str,
    pub url: &'a str,
    pub body: &'a [u8],
    pub content_type: &'static str,
    pub accept: &'static str,
    /// Additional protocol header, currently `anthropic-version`.
    pub version_header: Option<(&'static str, &'static str)>,
}

/// Evidence for one attempted route. Only `Supported` selects an endpoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProbeOutcome {
    Supported,
    /// HTTP 404, 405 or 410 for this probe; it can also reflect model routing.
    NotFound,
    AuthenticationRequired,
    Forbidden,
    RateLimited,
    ServerError(u16),
    Redirect(u16),
    /// A request, model or configuration rejection; protocol support is unknown.
    Rejected(u16),
    UnrecognizedResponse,
    TransportFailed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ProbeAttempt {
    pub format: Format,
    pub outcome: ProbeOutcome,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DiscoveryError {
    Endpoint(EndpointError),
    Kernel(Error),
}

impl core::fmt::Display for DiscoveryError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::Endpoint(error) => error.fmt(f),
            Self::Kernel(error) => error.fmt(f),
        }
    }
}

impl core::error::Error for DiscoveryError {}

impl From<EndpointError> for DiscoveryError {
    fn from(error: EndpointError) -> Self {
        Self::Endpoint(error)
    }
}

impl From<Error> for DiscoveryError {
    fn from(error: Error) -> Self {
        Self::Kernel(error)
    }
}

/// A serial, allocation-free discovery session. At most one probe is in flight.
/// Cache the selected endpoint in the caller's deployment/authentication context.
pub struct Discovery<'a> {
    base_url: &'a str,
    config: DiscoveryConfig<'a>,
    hint: Option<Protocol>,
    next: usize,
    pending: Option<HttpEndpoint<'a>>,
    selected: Option<HttpEndpoint<'a>>,
    attempts: [ProbeAttempt; 6],
    completed: usize,
}

impl<'a> Discovery<'a> {
    pub fn new(base_url: &'a str, model: &'a str) -> Result<Self, DiscoveryError> {
        Self::with_config(base_url, DiscoveryConfig::new(model))
    }

    pub fn with_config(
        base_url: &'a str,
        config: DiscoveryConfig<'a>,
    ) -> Result<Self, DiscoveryError> {
        if config.model.trim().is_empty() || config.model.chars().any(char::is_control) {
            return Err(
                Error::invalid("model", "provide a nonempty model without controls").into(),
            );
        }
        if config.max_output_tokens == 0 {
            return Err(
                Error::invalid("max_output_tokens", "probe budget must be positive").into(),
            );
        }
        if config.formats.is_empty() || config.formats.len() > 6 {
            return Err(Error::invalid("formats", "provide one to six distinct protocols").into());
        }
        let hint = url_protocol(base_url);
        let mut eligible = false;
        for (index, format) in config.formats.iter().copied().enumerate() {
            format.validate()?;
            if config.formats[..index]
                .iter()
                .any(|previous| previous.protocol == format.protocol)
            {
                return Err(Error::invalid("formats", "duplicate protocol").into());
            }
            if hint.is_none_or(|hint| hint == format.protocol) {
                HttpEndpoint::new(base_url, format)?;
                eligible = true;
            }
        }
        if !eligible {
            return Err(
                Error::invalid("formats", "full endpoint protocol is not a candidate").into(),
            );
        }
        Ok(Self {
            base_url,
            config,
            hint,
            next: 0,
            pending: None,
            selected: None,
            attempts: [ProbeAttempt {
                format: Format::new(Protocol::OpenAiChat),
                outcome: ProbeOutcome::UnrecognizedResponse,
            }; 6],
            completed: 0,
        })
    }

    fn candidate(&self) -> Option<(usize, Format)> {
        self.config.formats[self.next..]
            .iter()
            .copied()
            .enumerate()
            .find(|(_, format)| self.hint.is_none_or(|hint| hint == format.protocol))
            .map(|(index, format)| (self.next + index, format))
    }

    /// Build the next bounded generation probe. Buffer failures change neither
    /// buffer nor session state. After sending, call `observe` or `transport_failed`.
    /// Returns `None` after success or exhaustion; it never retries a user call.
    pub fn next_probe<'o>(
        &mut self,
        url: &'o mut [u8],
        body: &'o mut [u8],
    ) -> Result<Option<Probe<'o>>, DiscoveryError> {
        if self.pending.is_some() {
            return Err(Error::lifecycle("discovery", "observe the pending probe first").into());
        }
        if self.selected.is_some() {
            return Ok(None);
        }
        let Some((index, format)) = self.candidate() else {
            return Ok(None);
        };
        let endpoint = HttpEndpoint::new(self.base_url, format)?;
        // Preflight both caller buffers before writing either of them.
        match endpoint.url_into(Some(self.config.model), false, &mut []) {
            Err(EndpointError::BufferTooSmall { needed }) if needed <= url.len() => {}
            Err(error) => return Err(error.into()),
            Ok(_) => unreachable!("an absolute URL cannot be empty"),
        }
        let needed = match probe_body(format.protocol, self.config, &mut []) {
            Err(zconv::Error::OutputTooSmall { needed }) => needed,
            Err(error) => return Err(Error::from(error).into()),
            Ok(needed) => needed,
        };
        if needed > body.len() {
            return Err(Error::capacity(Resource::Output, needed, true, Replay::Unconsumed).into());
        }
        let url = endpoint.url_into(Some(self.config.model), false, url)?;
        let written = probe_body(format.protocol, self.config, body).map_err(Error::from)?;
        self.next = index + 1;
        self.pending = Some(endpoint);
        Ok(Some(Probe {
            format,
            method: "POST",
            url,
            body: &body[..written],
            content_type: "application/json",
            accept: "application/json",
            version_header: (format.protocol == Protocol::AnthropicMessages)
                .then_some(("anthropic-version", format.version)),
        }))
    }

    /// Inspect the complete HTTP response for the pending probe. The transport
    /// supplies the final status, content type and decoded (dechunked) body.
    /// Resource errors retain the pending probe: enlarge storage/limits and
    /// replay this response without sending another request.
    pub fn observe<const N: usize>(
        &mut self,
        status: u16,
        content_type: Option<&str>,
        body: &[u8],
        tokens: &mut TokenStorage<N>,
        limits: Limits,
    ) -> Result<ProbeOutcome, DiscoveryError> {
        let endpoint = self
            .pending
            .ok_or_else(|| Error::lifecycle("discovery", "no pending probe to observe"))?;
        let outcome = match status {
            200..=299 => inspect_response(endpoint.format(), content_type, body, tokens, limits)?,
            300..=399 => ProbeOutcome::Redirect(status),
            401 => ProbeOutcome::AuthenticationRequired,
            403 => ProbeOutcome::Forbidden,
            404 | 405 | 410 => ProbeOutcome::NotFound,
            429 => ProbeOutcome::RateLimited,
            400..=499 => ProbeOutcome::Rejected(status),
            500..=599 => ProbeOutcome::ServerError(status),
            _ => {
                return Err(Error::invalid(
                    "status",
                    "expected a final HTTP status from 200 to 599",
                )
                .into());
            }
        };
        self.finish_attempt(endpoint, outcome);
        Ok(outcome)
    }

    /// Record a timeout or transport error without inferring protocol support.
    pub fn transport_failed(&mut self) -> Result<ProbeOutcome, DiscoveryError> {
        let endpoint = self
            .pending
            .ok_or_else(|| Error::lifecycle("discovery", "no pending probe to fail"))?;
        self.finish_attempt(endpoint, ProbeOutcome::TransportFailed);
        Ok(ProbeOutcome::TransportFailed)
    }

    fn finish_attempt(&mut self, endpoint: HttpEndpoint<'a>, outcome: ProbeOutcome) {
        self.attempts[self.completed] = ProbeAttempt {
            format: endpoint.format(),
            outcome,
        };
        self.completed += 1;
        self.pending = None;
        if outcome == ProbeOutcome::Supported {
            self.selected = Some(endpoint);
        }
    }

    pub const fn selected(&self) -> Option<HttpEndpoint<'a>> {
        self.selected
    }

    pub fn is_finished(&self) -> bool {
        self.selected.is_some() || (self.pending.is_none() && self.candidate().is_none())
    }

    pub fn attempts(&self) -> &[ProbeAttempt] {
        &self.attempts[..self.completed]
    }
}

fn probe_body(
    protocol: Protocol,
    config: DiscoveryConfig<'_>,
    output: &mut [u8],
) -> Result<usize, zconv::Error> {
    let mut writer = Writer::new(output);
    if protocol == Protocol::GeminiGenerateContent {
        writer.raw(br#"{"contents":[{"role":"user","parts":[{"text":"Reply OK."}]}],"generationConfig":{"maxOutputTokens":"#);
    } else {
        writer.raw(br#"{"model":"#);
        writer.string(config.model.as_bytes());
        match protocol {
            Protocol::OpenAiChat | Protocol::AnthropicMessages => {
                writer.raw(br#","messages":[{"role":"user","content":"Reply OK."}],"#);
                if protocol == Protocol::OpenAiChat
                    && config.chat_token_limit == ChatTokenLimit::MaxCompletionTokens
                {
                    writer.raw(br#""max_completion_tokens":"#);
                } else {
                    writer.raw(br#""max_tokens":"#);
                }
            }
            Protocol::OpenAiCompletions => writer.raw(br#","prompt":"Reply OK.","max_tokens":"#),
            Protocol::OpenAiResponses => {
                writer.raw(br#","input":"Reply OK.","max_output_tokens":"#)
            }
            Protocol::GeminiInteractions => {
                writer.raw(br#","input":"Reply OK.","generation_config":{"max_output_tokens":"#)
            }
            Protocol::GeminiGenerateContent => unreachable!(),
        }
    }
    let mut digits = [0; 10];
    let mut position = digits.len();
    let mut value = config.max_output_tokens;
    loop {
        position -= 1;
        digits[position] = b'0' + (value % 10) as u8;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    writer.raw(&digits[position..]);
    match protocol {
        Protocol::GeminiGenerateContent => writer.raw(b"}}"),
        Protocol::GeminiInteractions => writer.raw(br#"},"stream":false,"store":false}"#),
        Protocol::OpenAiResponses => writer.raw(br#","stream":false,"store":false}"#),
        _ => writer.raw(br#","stream":false}"#),
    }
    writer.finish()
}

fn inspect_response<const N: usize>(
    format: Format,
    content_type: Option<&str>,
    body: &[u8],
    tokens: &mut TokenStorage<N>,
    limits: Limits,
) -> Result<ProbeOutcome, Error> {
    if let Some(content_type) = content_type {
        let mime = content_type.split(';').next().unwrap_or("").trim();
        let json_suffix = mime
            .get(mime.len().saturating_sub(5)..)
            .is_some_and(|suffix| suffix.eq_ignore_ascii_case("+json"));
        if !mime.eq_ignore_ascii_case("application/json")
            && !(mime
                .get(..12)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("application/"))
                && json_suffix)
        {
            return Ok(ProbeOutcome::UnrecognizedResponse);
        }
    }
    let document = match zconv::json::parse(body, &mut tokens.inner, limits.raw()) {
        Ok(document) => document,
        Err(error) => {
            let error = Error::from(error);
            return match error.kind {
                ErrorKind::Capacity | ErrorKind::Limit => Err(error),
                _ => Ok(ProbeOutcome::UnrecognizedResponse),
            };
        }
    };
    let root = document.root();
    if root.kind() != Kind::Object || root.get("error").is_some_and(|error| !error.is_null()) {
        return Ok(ProbeOutcome::UnrecognizedResponse);
    }
    let envelopes = usize::from(root.get("choices").is_some())
        + usize::from(root.get("output").is_some())
        + usize::from(root.get("steps").is_some())
        + usize::from(root.get("content").is_some())
        + usize::from(root.get("candidates").is_some() || root.get("promptFeedback").is_some());
    if envelopes != 1 {
        return Ok(ProbeOutcome::UnrecognizedResponse);
    }
    // Contradictory/mixed envelopes stay unknown, even when an adapter would
    // preserve the extra fields as opaque extensions.
    let mut identified = None;
    for candidate in DEFAULT_DISCOVERY_FORMATS {
        if response_signature(candidate.protocol, root) {
            if identified.is_some() {
                return Ok(ProbeOutcome::UnrecognizedResponse);
            }
            identified = Some(candidate.protocol);
        }
    }
    if identified != Some(format.protocol) {
        return Ok(ProbeOutcome::UnrecognizedResponse);
    }
    match zconv::protocol::validate(format.raw(), zconv::Operation::Response, root) {
        Ok(()) => Ok(ProbeOutcome::Supported),
        Err(error) => {
            let error = Error::from(error);
            match error.kind {
                ErrorKind::Capacity | ErrorKind::Limit => Err(error),
                _ => Ok(ProbeOutcome::UnrecognizedResponse),
            }
        }
    }
}

fn text_is(root: Value<'_, '_>, field: &str, expected: &str) -> bool {
    root.get(field)
        .and_then(Value::as_string)
        .is_some_and(|value| value.eq_str(expected))
}

fn string_field(root: Value<'_, '_>, field: &str) -> bool {
    root.get(field)
        .and_then(Value::as_string)
        .is_some_and(|value| value.decoded_len() > 0)
}

fn array_field(root: Value<'_, '_>, field: &str) -> bool {
    root.get(field)
        .is_some_and(|value| value.kind() == Kind::Array)
}

fn finished_response(root: Value<'_, '_>) -> bool {
    text_is(root, "status", "completed") || text_is(root, "status", "incomplete")
}

fn response_signature(protocol: Protocol, root: Value<'_, '_>) -> bool {
    match protocol {
        Protocol::OpenAiChat | Protocol::OpenAiCompletions => {
            let chat = protocol == Protocol::OpenAiChat;
            let object = if chat {
                "chat.completion"
            } else {
                "text_completion"
            };
            if !array_field(root, "choices") || !string_field(root, "id") {
                return false;
            }
            let choices = root.get("choices").expect("checked above");
            if choices.array().any(|choice| {
                choice.get(if chat { "text" } else { "message" }).is_some()
                    || choice.get("delta").is_some()
            }) {
                return false;
            }
            if root.get("object").is_some() {
                return text_is(root, "object", object);
            }
            // Compatible gateways sometimes omit object. Require a nonempty,
            // homogeneous choice list that is unique to this dialect.
            choices.array().next().is_some()
                && choices.array().all(|choice| {
                    if chat {
                        choice.get("text").is_none()
                            && choice
                                .get("message")
                                .is_some_and(|message| text_is(message, "role", "assistant"))
                    } else {
                        choice.get("message").is_none()
                            && choice.get("text").and_then(Value::as_string).is_some()
                    }
                })
        }
        Protocol::OpenAiResponses => {
            string_field(root, "id")
                && finished_response(root)
                && array_field(root, "output")
                && (root.get("object").is_none() || text_is(root, "object", "response"))
                && (text_is(root, "object", "response")
                    || root
                        .get("output")
                        .is_some_and(|output| output.array().next().is_some()))
        }
        Protocol::AnthropicMessages => {
            text_is(root, "type", "message")
                && text_is(root, "role", "assistant")
                && string_field(root, "id")
                && array_field(root, "content")
        }
        Protocol::GeminiGenerateContent => {
            (array_field(root, "candidates")
                && root.get("candidates").is_some_and(|candidates| {
                    candidates.array().all(|candidate| {
                        candidate.get("content").is_none_or(|content| {
                            (content.get("role").is_none() || text_is(content, "role", "model"))
                                && array_field(content, "parts")
                        })
                    })
                })
                && (string_field(root, "modelVersion")
                    || string_field(root, "responseId")
                    || root.get("candidates").is_some_and(|candidates| {
                        candidates.array().next().is_some()
                            && candidates.array().all(|candidate| {
                                candidate.get("content").is_some_and(|content| {
                                    (content.get("role").is_none()
                                        || text_is(content, "role", "model"))
                                        && array_field(content, "parts")
                                })
                            })
                    })))
                || root
                    .get("promptFeedback")
                    .is_some_and(|feedback| string_field(feedback, "blockReason"))
        }
        Protocol::GeminiInteractions => {
            string_field(root, "id")
                && finished_response(root)
                && string_field(root, "created")
                && string_field(root, "updated")
                && array_field(root, "steps")
                && (root.get("object").is_none() || text_is(root, "object", "interaction"))
        }
    }
}
