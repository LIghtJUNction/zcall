//! Private adaptation of zconv primitives into the public v1 contract.

/// Six real, wired adapters. Unknown formats require a new tested adapter.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    OpenAiCompletions,
    OpenAiChat,
    OpenAiResponses,
    AnthropicMessages,
    GeminiGenerateContent,
    GeminiInteractions,
}

impl Protocol {
    pub(crate) const fn raw(self) -> zconv::Protocol {
        match self {
            Self::OpenAiCompletions => zconv::Protocol::OpenAiCompletions,
            Self::OpenAiChat => zconv::Protocol::OpenAiChat,
            Self::OpenAiResponses => zconv::Protocol::OpenAiResponses,
            Self::AnthropicMessages => zconv::Protocol::AnthropicMessages,
            Self::GeminiGenerateContent => zconv::Protocol::GeminiGenerateContent,
            Self::GeminiInteractions => zconv::Protocol::GeminiInteractions,
        }
    }
    pub(crate) const fn from_raw(value: zconv::Protocol) -> Self {
        match value {
            zconv::Protocol::OpenAiCompletions => Self::OpenAiCompletions,
            zconv::Protocol::OpenAiChat => Self::OpenAiChat,
            zconv::Protocol::OpenAiResponses => Self::OpenAiResponses,
            zconv::Protocol::AnthropicMessages => Self::AnthropicMessages,
            zconv::Protocol::GeminiGenerateContent => Self::GeminiGenerateContent,
            zconv::Protocol::GeminiInteractions => Self::GeminiInteractions,
        }
    }
}

/// A declared wire dialect, not a claim about a model's abilities.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Profile {
    #[default]
    Standard,
    OpenAiCompatible(&'static str),
}

/// Protocol, schema version and extension namespace are checked independently.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Format {
    pub protocol: Protocol,
    pub version: &'static str,
    pub profile: Profile,
}
impl Format {
    pub const fn new(protocol: Protocol) -> Self {
        Self {
            protocol,
            version: match protocol {
                Protocol::AnthropicMessages => "2023-06-01",
                Protocol::GeminiGenerateContent => "v1beta",
                _ => "v1",
            },
            profile: Profile::Standard,
        }
    }
    pub fn validate(self) -> Result<(), Error> {
        zconv::protocol::validate_format(self.raw()).map_err(Error::from)
    }
    pub(crate) const fn raw(self) -> zconv::Format {
        zconv::Format {
            protocol: self.protocol.raw(),
            version: self.version,
            profile: match self.profile {
                Profile::Standard => zconv::Profile::Standard,
                Profile::OpenAiCompatible(name) => zconv::Profile::OpenAiCompatible(name),
            },
        }
    }
    pub(crate) const fn from_raw(value: zconv::Format) -> Self {
        Self {
            protocol: Protocol::from_raw(value.protocol),
            version: value.version,
            profile: match value.profile {
                zconv::Profile::Standard => Profile::Standard,
                zconv::Profile::OpenAiCompatible(name) => Profile::OpenAiCompatible(name),
            },
        }
    }
}
impl From<Protocol> for Format {
    fn from(value: Protocol) -> Self {
        Self::new(value)
    }
}

/// Missing, explicit null and present values are distinct. Empty and zero are values.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum Presence<T> {
    #[default]
    Missing,
    Null,
    Value(T),
}
impl<T> Presence<T> {
    pub const fn is_missing(&self) -> bool {
        matches!(self, Self::Missing)
    }
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }
    pub fn value(self) -> Option<T> {
        match self {
            Self::Value(value) => Some(value),
            _ => None,
        }
    }
    pub fn map<U>(self, f: impl FnOnce(T) -> U) -> Presence<U> {
        match self {
            Self::Missing => Presence::Missing,
            Self::Null => Presence::Null,
            Self::Value(value) => Presence::Value(f(value)),
        }
    }
    pub(crate) fn into_raw(self) -> zconv::ir::Presence<T> {
        match self {
            Self::Missing => zconv::ir::Presence::Missing,
            Self::Null => zconv::ir::Presence::Null,
            Self::Value(value) => zconv::ir::Presence::Value(value),
        }
    }
    pub(crate) fn from_raw(value: zconv::ir::Presence<T>) -> Self {
        match value {
            zconv::ir::Presence::Missing => Self::Missing,
            zconv::ir::Presence::Null => Self::Null,
            zconv::ir::Presence::Value(value) => Self::Value(value),
        }
    }
}

/// Borrowed UTF-8 or a validated wire JSON string. Escapes are decoded on demand.
#[derive(Clone, Copy, Debug)]
pub struct Text<'a>(pub(crate) zconv::ir::Text<'a>);
impl<'a> Text<'a> {
    pub const fn new(value: &'a str) -> Self {
        Self(zconv::ir::Text::Utf8(value))
    }
    pub fn as_str(self) -> Option<&'a str> {
        self.0.as_str()
    }
    pub fn eq_str(self, value: &str) -> bool {
        self.0.eq_str(value)
    }
    pub fn decoded_len(self) -> usize {
        self.0.decoded_len()
    }
    /// Required output capacity is exact; failure never consumes input.
    pub fn decode_into(self, output: &mut [u8]) -> Result<usize, Error> {
        match self.0 {
            zconv::ir::Text::Json(value) => value.decode_into(output).map_err(Error::from),
            zconv::ir::Text::Utf8(value) => {
                let dst = output.get_mut(..value.len()).ok_or_else(|| {
                    Error::capacity(Resource::Output, value.len(), true, Replay::Unconsumed)
                })?;
                dst.copy_from_slice(value.as_bytes());
                Ok(value.len())
            }
        }
    }
    pub(crate) const fn raw(self) -> zconv::ir::Text<'a> {
        self.0
    }
}
impl<'a> From<&'a str> for Text<'a> {
    fn from(value: &'a str) -> Self {
        Self::new(value)
    }
}

/// Validated JSON retaining the exact original bytes, including integer digits.
#[derive(Clone, Copy, Debug)]
pub struct Json<'a>(pub(crate) zconv::json::RawValue<'a>);
impl<'a> Json<'a> {
    pub fn parse<const N: usize>(
        input: &'a [u8],
        tokens: &mut TokenStorage<N>,
        limits: Limits,
    ) -> Result<Self, Error> {
        zconv::json::parse(input, &mut tokens.inner, limits.raw())
            .map(|doc| Self(doc.root().raw_value()))
            .map_err(Error::from)
    }
    pub fn raw(self) -> &'a [u8] {
        self.0.raw()
    }
    pub fn kind(self) -> JsonKind {
        match self.0.kind() {
            zconv::json::Kind::Object => JsonKind::Object,
            zconv::json::Kind::Array => JsonKind::Array,
            zconv::json::Kind::String => JsonKind::String,
            zconv::json::Kind::Number => JsonKind::Number,
            zconv::json::Kind::Bool => JsonKind::Bool,
            zconv::json::Kind::Null => JsonKind::Null,
        }
    }
    pub fn as_u64(self) -> Option<u64> {
        self.0.as_u64()
    }
    pub fn as_bool(self) -> Option<bool> {
        self.0.as_bool()
    }
    pub fn as_text(self) -> Option<Text<'a>> {
        self.0
            .as_string()
            .map(|value| Text(zconv::ir::Text::Json(value)))
    }
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum JsonKind {
    Object,
    Array,
    String,
    Number,
    Bool,
    Null,
}

/// Caller-sized token storage. Its representation is private and may change.
pub struct TokenStorage<const N: usize> {
    pub(crate) inner: [zconv::json::Token; N],
}
impl<const N: usize> TokenStorage<N> {
    pub const fn new() -> Self {
        Self {
            inner: [zconv::json::Token::EMPTY; N],
        }
    }
    pub const fn capacity(&self) -> usize {
        N
    }
    pub const fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}
impl<const N: usize> Default for TokenStorage<N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Bounds for untrusted JSON. Storage capacity is a separate, recoverable limit.
#[derive(Clone, Copy, Debug)]
pub struct Limits {
    pub max_depth: usize,
    pub max_input: usize,
    pub max_tokens: usize,
    pub max_object_fields: usize,
}
impl Default for Limits {
    fn default() -> Self {
        let value = zconv::json::Limits::default();
        Self {
            max_depth: value.max_depth,
            max_input: value.max_input,
            max_tokens: value.max_tokens,
            max_object_fields: value.max_object_fields,
        }
    }
}
impl Limits {
    pub(crate) const fn raw(self) -> zconv::json::Limits {
        zconv::json::Limits {
            max_depth: self.max_depth,
            max_input: self.max_input,
            max_tokens: self.max_tokens,
            max_object_fields: self.max_object_fields,
        }
    }
}

/// Stable categories; upstream errors are translated at the adapter boundary.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum ErrorKind {
    InvalidJson,
    InvalidInput,
    Unsupported,
    CapabilityUnknown,
    CapabilityMismatch,
    ContextRequired,
    ForbiddenLoss,
    Capacity,
    Limit,
    Truncated,
    Lifecycle,
    Upstream,
    Transport,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[non_exhaustive]
pub enum Resource {
    Output,
    Tokens,
    Ir,
    Scratch,
    Workspace,
    Framing,
    Payload,
    Events,
    Items,
    ToolCalls,
    Aggregate,
    Reports,
    Context,
    ActiveCalls,
}
/// How to retry after a capacity failure. Always advance stream input by `consumed`.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Replay {
    /// The operation did not take input ownership; retry the complete operation.
    Unconsumed,
    /// Accepted stream bytes remain buffered; send only the unconsumed suffix.
    Retained,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Capacity {
    pub region: Resource,
    pub needed: usize,
    pub exact: bool,
    pub replay: Replay,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Error {
    pub kind: ErrorKind,
    pub field: &'static str,
    pub detail: &'static str,
    pub offset: Option<usize>,
    pub capacity: Option<Capacity>,
}
impl Error {
    pub const fn new(kind: ErrorKind, field: &'static str, detail: &'static str) -> Self {
        Self {
            kind,
            field,
            detail,
            offset: None,
            capacity: None,
        }
    }
    pub const fn invalid(field: &'static str, detail: &'static str) -> Self {
        Self::new(ErrorKind::InvalidInput, field, detail)
    }
    pub const fn unsupported(field: &'static str) -> Self {
        Self::new(
            ErrorKind::Unsupported,
            field,
            "no verified adapter for this semantic feature",
        )
    }
    pub const fn lifecycle(field: &'static str, detail: &'static str) -> Self {
        Self::new(ErrorKind::Lifecycle, field, detail)
    }
    pub const fn capacity(region: Resource, needed: usize, exact: bool, replay: Replay) -> Self {
        Self {
            kind: ErrorKind::Capacity,
            field: "capacity",
            detail: "replace the specified caller storage and retry",
            offset: None,
            capacity: Some(Capacity {
                region,
                needed,
                exact,
                replay,
            }),
        }
    }
    pub(crate) fn with_resource(mut self, region: Resource) -> Self {
        if let Some(ref mut c) = self.capacity {
            c.region = region;
        }
        self
    }
    pub(crate) fn retained(mut self) -> Self {
        if let Some(ref mut c) = self.capacity {
            c.replay = Replay::Retained;
        }
        self
    }
}
impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{:?}: {}: {}", self.kind, self.field, self.detail)?;
        if let Some(c) = self.capacity {
            write!(
                f,
                " ({:?}, needed={}, exact={}, {:?})",
                c.region, c.needed, c.exact, c.replay
            )?;
        }
        Ok(())
    }
}
impl Error {
    pub(crate) fn from(value: zconv::Error) -> Self {
        use zconv::Error as E;
        match value {
            E::InvalidJson { offset, .. } | E::DuplicateKey { offset } => Self {
                offset: Some(offset),
                ..Self::new(
                    ErrorKind::InvalidJson,
                    "json",
                    "invalid JSON or duplicate object key",
                )
            },
            E::DepthLimitExceeded { .. } => {
                Self::new(ErrorKind::Limit, "depth", "JSON nesting limit exceeded")
            }
            E::InputTooLarge { .. } => {
                Self::new(ErrorKind::Limit, "input", "input byte limit exceeded")
            }
            E::WorkspaceTooSmall { needed } => {
                Self::capacity(Resource::Tokens, needed, false, Replay::Unconsumed)
            }
            E::ScratchTooSmall { needed } => {
                Self::capacity(Resource::Scratch, needed, true, Replay::Unconsumed)
            }
            E::OutputTooSmall { needed } => {
                Self::capacity(Resource::Output, needed, true, Replay::Unconsumed)
            }
            E::IrTooSmall { needed } => {
                Self::capacity(Resource::Ir, needed, false, Replay::Unconsumed)
            }
            E::EventTooLarge { needed } => {
                Self::capacity(Resource::Aggregate, needed, false, Replay::Unconsumed)
            }
            E::TooManyActiveItems { needed } => {
                Self::capacity(Resource::Items, needed, false, Replay::Unconsumed)
            }
            E::LimitExceeded { kind, .. } => {
                Self::new(ErrorKind::Limit, kind, "configured resource limit exceeded")
            }
            E::InvalidInput { field, reason } => Self::invalid(field, reason),
            E::Unsupported { feature } | E::DirectUnavailable { feature } => {
                Self::unsupported(feature)
            }
            E::ContextRequired { field } => Self::new(
                ErrorKind::ContextRequired,
                field,
                "caller-provided fact required",
            ),
            E::PolicyViolation { feature } => Self::new(
                ErrorKind::ForbiddenLoss,
                feature,
                "explicit permission required for this semantic loss",
            ),
            E::TruncatedStream => Self::new(
                ErrorKind::Truncated,
                "stream",
                "input ended before a protocol terminal marker",
            ),
            E::AlreadyFinished => Self::lifecycle("stream", "input after terminal state"),
            _ => Self::unsupported("upstream error category"),
        }
    }
}
