//! Registered conversation routes with host-owned upstream transport.
//! Unknown protocols require another Handler or explicit adapter. Inbound
//! credentials, Cookie and arbitrary headers are never forwarded upstream.
use super::{
    Cancellation, Handler, HandlerError, Response,
    auth::{self, AuthPolicy, Authentication, CallerKeys},
    http::Request,
    protocol::{self, Inspection, Protocol},
    transport::Peer,
};
use std::{
    io::{self, Read},
    sync::{Arc, RwLock},
};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Identity {
    pub key_id: Option<String>,
    pub workspace: Option<String>,
    pub authentication: Authentication,
}

pub trait Authorizer: Send + Sync + 'static {
    fn authorize(&self, request: &Request, peer: &Peer) -> Result<Identity, HandlerError>;
}

/// Default authentication accepts only a host-established local peer.
pub struct LoopbackOnly;
impl Authorizer for LoopbackOnly {
    fn authorize(&self, _: &Request, peer: &Peer) -> Result<Identity, HandlerError> {
        if !peer.loopback {
            return Err(HandlerError::new(401, "an inbound caller key is required"));
        }
        Ok(Identity {
            key_id: None,
            workspace: None,
            authentication: Authentication::TrustedLoopback,
        })
    }
}

/// Shared state can be rotated/revoked between requests. Persistence and
/// OS-secure secret generation remain host responsibilities.
pub struct KeyAuthenticator {
    pub keys: Arc<RwLock<CallerKeys>>,
    trust_loopback: bool,
    token: Option<String>,
}
impl KeyAuthenticator {
    pub fn new(keys: Arc<RwLock<CallerKeys>>) -> Self {
        Self {
            keys,
            trust_loopback: false,
            token: None,
        }
    }
    pub fn trust_loopback(mut self, enabled: bool) -> Self {
        self.trust_loopback = enabled;
        self
    }
    pub fn with_shared_token(mut self, token: impl Into<String>) -> Result<Self, String> {
        let token = token.into();
        if token.is_empty()
            || token.len() > 16384
            || token.bytes().any(|byte| byte.is_ascii_control())
        {
            return Err("shared token must be nonempty without control characters".into());
        }
        self.token = Some(token);
        Ok(self)
    }
}
impl Authorizer for KeyAuthenticator {
    fn authorize(&self, request: &Request, peer: &Peer) -> Result<Identity, HandlerError> {
        let keys = self
            .keys
            .read()
            .map_err(|_| HandlerError::new(503, "caller key state is unavailable"))?;
        let policy = AuthPolicy {
            keys: Some(&keys),
            token: self.token.as_deref(),
            trust_loopback: self.trust_loopback,
        };
        let caller = auth::authenticate(request, &policy, peer.loopback)
            .ok_or_else(|| HandlerError::new(401, "invalid or missing caller key"))?;
        Ok(Identity {
            key_id: caller.key_id.map(str::to_owned),
            workspace: caller.workspace.map(str::to_owned),
            authentication: caller.authentication,
        })
    }
}

/// Credentials are explicit provider headers, never inbound ones.
/// Debug is absent because headers may contain secrets.
#[derive(Clone)]
pub struct Target {
    pub base_url: String,
    pub protocol: Protocol,
    pub model: String,
    pub headers: Vec<(String, String)>,
}
impl Target {
    pub fn new(base_url: impl Into<String>, protocol: Protocol, model: impl Into<String>) -> Self {
        Self {
            base_url: base_url.into(),
            protocol,
            model: model.into(),
            headers: Vec::new(),
        }
    }
}
pub trait Router: Send + Sync + 'static {
    fn route(
        &self,
        protocol: Protocol,
        request: &Inspection,
        caller: &Identity,
    ) -> Result<Target, HandlerError>;
}
impl Router for Target {
    fn route(&self, _: Protocol, _: &Inspection, _: &Identity) -> Result<Target, HandlerError> {
        Ok(self.clone())
    }
}
impl<F> Router for F
where
    F: Fn(Protocol, &Inspection, &Identity) -> Result<Target, HandlerError> + Send + Sync + 'static,
{
    fn route(
        &self,
        protocol: Protocol,
        request: &Inspection,
        caller: &Identity,
    ) -> Result<Target, HandlerError> {
        self(protocol, request, caller)
    }
}

/// Fully prepared request. Transport may send HTTP/TLS, use a Unix socket or a
/// host backend; it must honor the supplied cancellation and deadline.
pub struct UpstreamRequest {
    pub url: String,
    pub protocol: Protocol,
    pub headers: Vec<(String, String)>,
    pub body: Vec<u8>,
    pub stream: bool,
}

/// cancel() must be idempotent and nonblocking. It is called on response Drop,
/// including downstream disconnect or conversion failure.
pub trait UpstreamBody: Read + Send {
    fn cancel(&mut self) {}
}

/// Ordinary owned reader whose Drop closes its resource.
pub struct ReaderBody<R>(pub R);
impl<R: Read + Send> Read for ReaderBody<R> {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        self.0.read(bytes)
    }
}
impl<R: Read + Send> UpstreamBody for ReaderBody<R> {}

pub struct UpstreamResponse {
    pub status: u16,
    pub content_type: String,
    body: Box<dyn UpstreamBody>,
}
impl UpstreamResponse {
    pub fn new(
        status: u16,
        content_type: impl Into<String>,
        body: impl UpstreamBody + 'static,
    ) -> Self {
        Self {
            status,
            content_type: content_type.into(),
            body: Box::new(body),
        }
    }
}
impl Drop for UpstreamResponse {
    fn drop(&mut self) {
        self.body.cancel();
    }
}
pub trait Transport: Send + Sync + 'static {
    fn execute(
        &self,
        request: &UpstreamRequest,
        cancellation: &Cancellation,
    ) -> Result<UpstreamResponse, HandlerError>;
}

#[derive(Clone, Debug)]
pub struct GatewayConfig {
    pub default_max_tokens: u64,
    pub max_upstream_bytes: usize,
}
impl Default for GatewayConfig {
    fn default() -> Self {
        Self {
            default_max_tokens: 512,
            max_upstream_bytes: 16 * 1024 * 1024,
        }
    }
}

/// AI handler for six conversation protocols. Other paths/formats use Handler.
pub struct Gateway<R, T> {
    router: R,
    transport: T,
    authorizer: Arc<dyn Authorizer>,
    config: GatewayConfig,
}
impl<R: Router, T: Transport> Gateway<R, T> {
    pub fn new(router: R, transport: T) -> Self {
        Self {
            router,
            transport,
            authorizer: Arc::new(LoopbackOnly),
            config: GatewayConfig::default(),
        }
    }
    pub fn with_authorizer(mut self, authorizer: impl Authorizer) -> Self {
        self.authorizer = Arc::new(authorizer);
        self
    }
    pub fn with_config(mut self, config: GatewayConfig) -> Result<Self, String> {
        if config.max_upstream_bytes == 0 {
            return Err("max_upstream_bytes must be nonzero".into());
        }
        self.config = config;
        Ok(self)
    }
}

impl<R: Router, T: Transport> Handler for Gateway<R, T> {
    fn handle(
        &self,
        request: Request,
        peer: &Peer,
        response: &mut Response<'_>,
        cancellation: &Cancellation,
    ) -> Result<(), HandlerError> {
        let caller = self.authorizer.authorize(&request, peer)?;
        if request.method != "POST" {
            return Err(HandlerError::new(
                405,
                "conversation endpoints require POST",
            ));
        }
        let (source, path_model, path_stream) = protocol::ingress(&request.path).map_err(|_| {
            HandlerError::new(
                404,
                "unknown conversation endpoint; install an explicit handler or adapter",
            )
        })?;
        let mut inspection = protocol::inspect(source, &request.body, path_model.as_deref())
            .map_err(|error| HandlerError::new(400, error))?;
        inspection.stream |= path_stream;
        let target = self.router.route(source, &inspection, &caller)?;
        let body = protocol::prepare_request(
            source,
            target.protocol,
            &request.body,
            &target.model,
            inspection.stream,
            self.config.default_max_tokens,
        )
        .map_err(|error| HandlerError::new(422, error))?;
        let url = protocol::endpoint(
            &target.base_url,
            target.protocol,
            &target.model,
            inspection.stream,
        )
        .map_err(|error| HandlerError::new(500, error))?;
        let mut headers = target.headers;
        validate_upstream_headers(&headers)?;
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("content-type"))
        {
            headers.push(("content-type".into(), "application/json".into()));
        }
        if !headers
            .iter()
            .any(|(name, _)| name.eq_ignore_ascii_case("accept"))
        {
            headers.push((
                "accept".into(),
                if inspection.stream {
                    "text/event-stream"
                } else {
                    "application/json"
                }
                .into(),
            ));
        }
        let outgoing = UpstreamRequest {
            url,
            protocol: target.protocol,
            headers,
            body,
            stream: inspection.stream,
        };
        if cancellation.is_cancelled() {
            return Err(HandlerError::new(504, "request was cancelled"));
        }
        let mut upstream = self.transport.execute(&outgoing, cancellation)?;
        if !(200..=599).contains(&upstream.status) {
            return Err(HandlerError::new(
                502,
                "upstream did not return a final HTTP status",
            ));
        }
        let success = (200..300).contains(&upstream.status);
        let media = upstream.content_type.split(';').next().unwrap_or("").trim();
        if success && inspection.stream {
            if !media.eq_ignore_ascii_case("text/event-stream") {
                return Err(HandlerError::new(
                    502,
                    "upstream streaming response must be text/event-stream",
                ));
            }
            let maximum = self.config.max_upstream_bytes;
            protocol::with_stream_converter(
                target.protocol,
                source,
                &inspection.model,
                |convert| {
                    response.stream(upstream.status, "text/event-stream", &[], |sink| {
                        let mut bytes = [0u8; 8192];
                        let mut consumed = 0usize;
                        loop {
                            let count = read_body(&mut *upstream.body, &mut bytes, cancellation)?;
                            if count == 0 {
                                break;
                            }
                            consumed = consumed
                                .checked_add(count)
                                .filter(|total| *total <= maximum)
                                .ok_or_else(|| {
                                    HandlerError::new(
                                        502,
                                        "upstream body exceeds max_upstream_bytes",
                                    )
                                })?;
                            convert(&bytes[..count], false, sink)
                                .map_err(|error| HandlerError::new(502, error))?;
                        }
                        convert(&[], true, sink).map_err(|error| HandlerError::new(502, error))
                    })
                },
            )
            .map_err(|error| HandlerError::new(502, error))??;
            return Ok(());
        }
        let bytes = collect_body(
            &mut *upstream.body,
            self.config.max_upstream_bytes,
            cancellation,
        )?;
        if !success {
            return response.send(upstream.status, &upstream.content_type, &bytes);
        }
        if !(media.eq_ignore_ascii_case("application/json")
            || media.to_ascii_lowercase().ends_with("+json"))
        {
            return Err(HandlerError::new(502, "upstream response must be JSON"));
        }
        let converted =
            protocol::convert_response(target.protocol, source, &bytes, &inspection.model)
                .map_err(|error| HandlerError::new(502, error))?;
        response.send(upstream.status, "application/json", &converted)
    }
}

fn validate_upstream_headers(headers: &[(String, String)]) -> Result<(), HandlerError> {
    for (index, (name, value)) in headers.iter().enumerate() {
        let token = |byte: u8| {
            byte.is_ascii_alphanumeric() || byte == 96 || b"!#$%&'*+-.^_|~".contains(&byte)
        };
        if name.is_empty()
            || !name.bytes().all(token)
            || value
                .bytes()
                .any(|byte| (byte < 32 && byte != b'\t') || byte == 127)
            || ["content-length", "transfer-encoding", "connection", "host"]
                .iter()
                .any(|reserved| name.eq_ignore_ascii_case(reserved))
            || headers[..index]
                .iter()
                .any(|(previous, _)| previous.eq_ignore_ascii_case(name))
        {
            return Err(HandlerError::new(500, "invalid upstream request header"));
        }
    }
    Ok(())
}
fn read_body(
    body: &mut dyn UpstreamBody,
    bytes: &mut [u8],
    cancellation: &Cancellation,
) -> Result<usize, HandlerError> {
    loop {
        if cancellation.is_cancelled() {
            return Err(HandlerError::new(504, "upstream request was cancelled"));
        }
        match body.read(bytes) {
            Ok(count) => return Ok(count),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(_) => return Err(HandlerError::new(502, "upstream read failed")),
        }
    }
}
fn collect_body(
    body: &mut dyn UpstreamBody,
    maximum: usize,
    cancellation: &Cancellation,
) -> Result<Vec<u8>, HandlerError> {
    let mut output = Vec::new();
    let mut bytes = [0u8; 8192];
    loop {
        let count = read_body(body, &mut bytes, cancellation)?;
        if count == 0 {
            return Ok(output);
        }
        if count > maximum.saturating_sub(output.len()) {
            return Err(HandlerError::new(
                502,
                "upstream body exceeds max_upstream_bytes",
            ));
        }
        output
            .try_reserve(count)
            .map_err(|_| HandlerError::new(503, "upstream buffer allocation failed"))?;
        output.extend_from_slice(&bytes[..count]);
    }
}
