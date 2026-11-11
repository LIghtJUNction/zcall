//! Borrowed HTTP endpoint configuration and caller-buffered URL construction.

use crate::common::{Error, Format, Protocol};

/// A URL configuration failure. No variant contains credentials or response data.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum EndpointError {
    InvalidUrl(&'static str),
    InvalidFormat(Error),
    ProtocolMismatch { expected: Protocol, found: Protocol },
    MissingModel,
    InvalidModel(&'static str),
    BufferTooSmall { needed: usize },
}

impl core::fmt::Display for EndpointError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidUrl(reason) => write!(f, "invalid base URL: {reason}"),
            Self::InvalidFormat(error) => write!(f, "invalid endpoint format: {error}"),
            Self::ProtocolMismatch { expected, found } => {
                write!(
                    f,
                    "endpoint path is {found:?}, configured protocol is {expected:?}"
                )
            }
            Self::MissingModel => {
                f.write_str("GenerateContent requires a model or a complete model URL")
            }
            Self::InvalidModel(reason) => write!(f, "invalid model: {reason}"),
            Self::BufferTooSmall { needed } => write!(f, "URL output requires {needed} bytes"),
        }
    }
}

impl core::error::Error for EndpointError {}

/// A validated HTTP(S) base URL with an explicitly selected wire format.
///
/// The URL is borrowed; construction and rendering perform no allocation or I/O.
/// A bare origin receives `/v1`, or `/v1beta` for either Gemini adapter. An
/// existing path is the caller's exact API prefix; a recognized complete call
/// path is reused. The wire schema version and the HTTP path are independent.
///
/// URLs must be ASCII URIs: use a punycode host and percent-encoded path/query
/// when needed. Fragments, user information, whitespace, control characters,
/// backslashes, malformed escapes and dot path segments are rejected. Query
/// parameters are preserved; GenerateContent normalizes its `alt=sse` parameter
/// to the requested streaming mode.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct HttpEndpoint<'a> {
    base_url: &'a str,
    format: Format,
}

impl<'a> HttpEndpoint<'a> {
    pub fn new(base_url: &'a str, format: Format) -> Result<Self, EndpointError> {
        format.validate().map_err(EndpointError::InvalidFormat)?;
        let parts = UrlParts::parse(base_url)?;
        if let Some(found) = path_protocol(parts.path)
            && found != format.protocol
        {
            return Err(EndpointError::ProtocolMismatch {
                expected: format.protocol,
                found,
            });
        }
        Ok(Self { base_url, format })
    }

    pub const fn base_url(self) -> &'a str {
        self.base_url
    }

    pub const fn format(self) -> Format {
        self.format
    }

    /// Build the complete call URL in caller storage.
    ///
    /// GenerateContent accepts a raw model ID or `models/ID`; raw reserved and
    /// UTF-8 bytes are percent-encoded as one path segment. A model supplied here
    /// replaces a model in a complete URL; `None` retains the complete URL's
    /// model. `stream` always selects the GenerateContent action and SSE query.
    /// Other adapters carry the model/stream choice in their request body.
    ///
    /// A short output buffer is unchanged and reports the exact required size.
    pub fn url_into<'o>(
        self,
        model: Option<&str>,
        stream: bool,
        output: &'o mut [u8],
    ) -> Result<&'o str, EndpointError> {
        let parts = UrlParts::parse(self.base_url)?;
        let route = Route::new(parts.path, self.format.protocol, model, stream)?;
        let mut count = Writer::new(None);
        render(parts, route, self.format.protocol, stream, &mut count)?;
        let needed = count.written;
        if output.len() < needed {
            return Err(EndpointError::BufferTooSmall { needed });
        }
        render(
            parts,
            route,
            self.format.protocol,
            stream,
            &mut Writer::new(Some(&mut output[..needed])),
        )?;
        core::str::from_utf8(&output[..needed])
            .map_err(|_| EndpointError::InvalidUrl("constructed URL is not UTF-8"))
    }
}

/// A path is a candidate-ordering hint, never evidence that a server supports it.
pub(crate) fn url_protocol(base_url: &str) -> Option<Protocol> {
    UrlParts::parse(base_url)
        .ok()
        .and_then(|parts| path_protocol(parts.path))
}

#[derive(Clone, Copy)]
struct UrlParts<'a> {
    origin: &'a str,
    path: &'a str,
    query: Option<&'a str>,
}

impl<'a> UrlParts<'a> {
    fn parse(url: &'a str) -> Result<Self, EndpointError> {
        if !url.is_ascii()
            || url
                .bytes()
                .any(|b| b.is_ascii_control() || b.is_ascii_whitespace() || b == b'\\')
        {
            return Err(EndpointError::InvalidUrl(
                "expected an ASCII URI without whitespace or backslashes",
            ));
        }
        if url.contains('#') {
            return Err(EndpointError::InvalidUrl(
                "fragments are not sent to HTTP servers",
            ));
        }
        let (scheme, rest) = url.split_once("://").ok_or(EndpointError::InvalidUrl(
            "absolute http:// or https:// URL required",
        ))?;
        if !scheme.eq_ignore_ascii_case("http") && !scheme.eq_ignore_ascii_case("https") {
            return Err(EndpointError::InvalidUrl(
                "only HTTP and HTTPS are supported",
            ));
        }
        let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
        validate_authority(&rest[..authority_end])?;
        let origin_end = scheme.len() + 3 + authority_end;
        let tail = &url[origin_end..];
        let (path, query) = match tail.split_once('?') {
            Some((path, query)) => (path, Some(query)),
            None => (tail, None),
        };
        validate_component(path, false)?;
        if let Some(query) = query {
            validate_component(query, true)?;
        }
        for segment in path.split('/') {
            if decoded_eq(segment, ".") || decoded_eq(segment, "..") {
                return Err(EndpointError::InvalidUrl(
                    "dot path segments are not allowed",
                ));
            }
        }
        Ok(Self {
            origin: &url[..origin_end],
            path: path.trim_end_matches('/'),
            query,
        })
    }
}

fn validate_authority(authority: &str) -> Result<(), EndpointError> {
    if authority.is_empty() || authority.contains('@') {
        return Err(EndpointError::InvalidUrl(
            "host required; user information is not allowed",
        ));
    }
    let port = if let Some(ipv6) = authority.strip_prefix('[') {
        let (host, tail) = ipv6
            .split_once(']')
            .ok_or(EndpointError::InvalidUrl("unclosed IPv6 address"))?;
        host.parse::<core::net::Ipv6Addr>()
            .map_err(|_| EndpointError::InvalidUrl("invalid IPv6 address"))?;
        if tail.is_empty() {
            None
        } else {
            Some(
                tail.strip_prefix(':')
                    .ok_or(EndpointError::InvalidUrl("invalid IPv6 port separator"))?,
            )
        }
    } else {
        let (host, port) = match authority.split_once(':') {
            Some((host, port)) => (host, Some(port)),
            None => (authority, None),
        };
        let host = host.strip_suffix('.').unwrap_or(host);
        if host.is_empty()
            || host.len() > 253
            || host.split('.').any(|label| {
                label.is_empty()
                    || label.len() > 63
                    || label.starts_with('-')
                    || label.ends_with('-')
                    || !label
                        .bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b == b'-')
            })
        {
            return Err(EndpointError::InvalidUrl("invalid host name"));
        }
        if host.contains('.') && host.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
            host.parse::<core::net::Ipv4Addr>()
                .map_err(|_| EndpointError::InvalidUrl("invalid IPv4 address"))?;
        }
        port
    };
    if let Some(port) = port
        && (port.is_empty()
            || !port.bytes().all(|b| b.is_ascii_digit())
            || port.parse::<u16>().is_err())
    {
        return Err(EndpointError::InvalidUrl(
            "port must be an integer from 0 to 65535",
        ));
    }
    Ok(())
}

fn validate_component(value: &str, query: bool) -> Result<(), EndpointError> {
    let bytes = value.as_bytes();
    let mut index = 0;
    while index < bytes.len() {
        let byte = bytes[index];
        if byte == b'%' {
            if bytes.get(index + 1).and_then(|b| hex(*b)).is_none()
                || bytes.get(index + 2).and_then(|b| hex(*b)).is_none()
            {
                return Err(EndpointError::InvalidUrl("malformed percent escape"));
            }
            index += 3;
        } else if unreserved(byte) || b"!$&'()*+,;=:@/".contains(&byte) || (query && byte == b'?') {
            index += 1;
        } else {
            return Err(EndpointError::InvalidUrl("invalid path or query character"));
        }
    }
    Ok(())
}

fn path_protocol(path: &str) -> Option<Protocol> {
    if path.ends_with("/chat/completions") {
        Some(Protocol::OpenAiChat)
    } else if path.ends_with("/completions") {
        Some(Protocol::OpenAiCompletions)
    } else if path.ends_with("/responses") {
        Some(Protocol::OpenAiResponses)
    } else if path.ends_with("/messages") {
        Some(Protocol::AnthropicMessages)
    } else if path.ends_with("/interactions") {
        Some(Protocol::GeminiInteractions)
    } else if gemini_path(path).is_some() {
        Some(Protocol::GeminiGenerateContent)
    } else {
        None
    }
}

/// Return the API prefix and the already encoded model in a complete URL.
fn gemini_path(path: &str) -> Option<(&str, &str)> {
    let model_path = path
        .strip_suffix(":generateContent")
        .or_else(|| path.strip_suffix(":streamGenerateContent"))?;
    let (prefix, model) = model_path.rsplit_once("/models/")?;
    (!model.is_empty() && !model.contains('/')).then_some((prefix, model))
}

#[derive(Clone, Copy)]
enum Model<'a> {
    Encoded(&'a str),
    Raw(&'a str),
}

#[derive(Clone, Copy)]
enum Route<'a> {
    Exact(&'a str),
    Append {
        prefix: &'a str,
        suffix: &'static str,
    },
    Gemini {
        prefix: &'a str,
        model: Model<'a>,
        action: &'static str,
    },
}

impl<'a> Route<'a> {
    fn new(
        path: &'a str,
        protocol: Protocol,
        model: Option<&'a str>,
        stream: bool,
    ) -> Result<Self, EndpointError> {
        if protocol == Protocol::GeminiGenerateContent {
            let complete = gemini_path(path);
            let model = match model {
                Some(model) => {
                    let model = model.strip_prefix("models/").unwrap_or(model);
                    if model.is_empty() || model == "." || model == ".." {
                        return Err(EndpointError::InvalidModel("nonempty model ID required"));
                    }
                    if model.chars().any(char::is_control) {
                        return Err(EndpointError::InvalidModel(
                            "control characters are not allowed",
                        ));
                    }
                    Model::Raw(model)
                }
                None => Model::Encoded(complete.ok_or(EndpointError::MissingModel)?.1),
            };
            let prefix = match complete {
                Some((prefix, _)) => prefix,
                None if path.is_empty() => "/v1beta",
                None => path.strip_suffix("/models").unwrap_or(path),
            };
            return Ok(Self::Gemini {
                prefix,
                model,
                action: if stream {
                    ":streamGenerateContent"
                } else {
                    ":generateContent"
                },
            });
        }
        if path_protocol(path).is_some() {
            return Ok(Self::Exact(path));
        }
        Ok(Self::Append {
            prefix: if path.is_empty() {
                if protocol == Protocol::GeminiInteractions {
                    "/v1beta"
                } else {
                    "/v1"
                }
            } else {
                path
            },
            suffix: match protocol {
                Protocol::OpenAiCompletions => "/completions",
                Protocol::OpenAiChat => "/chat/completions",
                Protocol::OpenAiResponses => "/responses",
                Protocol::AnthropicMessages => "/messages",
                Protocol::GeminiInteractions => "/interactions",
                Protocol::GeminiGenerateContent => unreachable!(),
            },
        })
    }
}

fn render(
    parts: UrlParts<'_>,
    route: Route<'_>,
    protocol: Protocol,
    stream: bool,
    writer: &mut Writer<'_>,
) -> Result<(), EndpointError> {
    writer.bytes(parts.origin.as_bytes())?;
    match route {
        Route::Exact(path) => writer.bytes(path.as_bytes())?,
        Route::Append { prefix, suffix } => {
            writer.bytes(prefix.as_bytes())?;
            writer.bytes(suffix.as_bytes())?;
        }
        Route::Gemini {
            prefix,
            model,
            action,
        } => {
            writer.bytes(prefix.as_bytes())?;
            writer.bytes(b"/models/")?;
            match model {
                Model::Encoded(model) => writer.bytes(model.as_bytes())?,
                Model::Raw(model) => {
                    const HEX: &[u8; 16] = b"0123456789ABCDEF";
                    for byte in model.bytes() {
                        if unreserved(byte) {
                            writer.bytes(&[byte])?;
                        } else {
                            writer.bytes(&[
                                b'%',
                                HEX[(byte >> 4) as usize],
                                HEX[(byte & 15) as usize],
                            ])?;
                        }
                    }
                }
            }
            writer.bytes(action.as_bytes())?;
        }
    }
    if protocol != Protocol::GeminiGenerateContent {
        if let Some(query) = parts.query.filter(|q| !q.is_empty()) {
            writer.bytes(b"?")?;
            writer.bytes(query.as_bytes())?;
        }
        return Ok(());
    }
    let mut first = true;
    if let Some(query) = parts.query {
        for item in query.split('&').filter(|item| !item.is_empty()) {
            let (key, value) = item.split_once('=').unwrap_or((item, ""));
            if decoded_eq(key, "alt") && (stream || decoded_eq(value, "sse")) {
                continue;
            }
            writer.bytes(if first { b"?" } else { b"&" })?;
            writer.bytes(item.as_bytes())?;
            first = false;
        }
    }
    if stream {
        writer.bytes(if first { b"?alt=sse" } else { b"&alt=sse" })?;
    }
    Ok(())
}

struct Writer<'a> {
    output: Option<&'a mut [u8]>,
    written: usize,
}

impl<'a> Writer<'a> {
    fn new(output: Option<&'a mut [u8]>) -> Self {
        Self { output, written: 0 }
    }

    fn bytes(&mut self, bytes: &[u8]) -> Result<(), EndpointError> {
        let end = self
            .written
            .checked_add(bytes.len())
            .ok_or(EndpointError::InvalidUrl("constructed URL length overflow"))?;
        if let Some(output) = &mut self.output {
            output
                .get_mut(self.written..end)
                .ok_or(EndpointError::BufferTooSmall { needed: end })?
                .copy_from_slice(bytes);
        }
        self.written = end;
        Ok(())
    }
}

fn unreserved(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || b"-._~".contains(&byte)
}

fn hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn decoded_eq(value: &str, expected: &str) -> bool {
    let mut bytes = value.bytes();
    for expected in expected.bytes() {
        let actual = match bytes.next() {
            Some(b'%') => match (bytes.next().and_then(hex), bytes.next().and_then(hex)) {
                (Some(high), Some(low)) => (high << 4) | low,
                _ => return false,
            },
            Some(byte) => byte,
            None => return false,
        };
        if actual != expected {
            return false;
        }
    }
    bytes.next().is_none()
}
