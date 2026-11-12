//! Borrowed complete requests and responses. Only a fixed-size header wraps the
//! underlying IR; all message, block, tool and choice records remain in the
//! caller's one typed storage region.

use crate::common::*;
use zconv::ir;

/// Capacity is measured in internal records, not messages. A plain message uses
/// one message record plus one record per content block. Extensions consume one
/// further record each. `memory_bytes` reports the actual storage cost.
pub struct RecordStorage<'a, const N: usize> {
    inner: [ir::Record<'a>; N],
}
impl<'a, const N: usize> RecordStorage<'a, N> {
    pub const fn new() -> Self {
        Self {
            inner: [ir::Record::EMPTY; N],
        }
    }
    pub const fn capacity(&self) -> usize {
        N
    }
    pub const fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}
impl<const N: usize> Default for RecordStorage<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

macro_rules! mapped_enum {
    ($name:ident => $raw:ident { $($variant:ident),+ $(,)? }) => {
        #[derive(Clone, Copy, Debug, Eq, PartialEq)]
        pub enum $name { $($variant),+ }
        impl $name {
            fn raw(self) -> ir::$raw { match self { $(Self::$variant => ir::$raw::$variant),+ } }
            fn from_raw(value: ir::$raw) -> Self { match value { $(ir::$raw::$variant => Self::$variant),+ } }
        }
    };
}
mapped_enum!(SchemaDialect => SchemaDialect { JsonSchema, GeminiSchema });
mapped_enum!(Role => Role { System, Developer, User, Assistant, Tool });
mapped_enum!(ContentShape => ContentShape { Missing, Null, String, Array });
mapped_enum!(MediaKind => MediaKind { Image, Audio, File, Video });
mapped_enum!(ReasoningKind => ReasoningKind { Thought, Summary });
mapped_enum!(Finish => Finish { Missing, Null, Stop, Length, ToolCalls, Safety, Refusal, Error, Cancelled, InProgress, Other });
mapped_enum!(OutputMode => OutputMode { Text, JsonObject, JsonSchema, Opaque });

/// Exact, nonnegative integer. A source count larger than u64 remains a borrowed
/// decimal JSON number; it is never rounded through floating point.
#[derive(Clone, Copy, Debug)]
pub struct Integer<'a>(ir::Number<'a>);
impl<'a> Integer<'a> {
    pub const fn new(value: u64) -> Self {
        Self(ir::Number::Unsigned(value))
    }
    pub fn as_u64(self) -> Option<u64> {
        self.0.as_u64()
    }
    pub fn original_json(self) -> Option<&'a [u8]> {
        match self.0 {
            ir::Number::Json(value) => Some(value.raw()),
            ir::Number::Unsigned(_) => None,
        }
    }
}

/// Data is borrowed and its representation remains explicit. `JsonText` is a
/// JSON document carried inside a wire string; it is not ordinary prose. Cross
/// protocol encoders validate it using the caller's scratch and token storage.
#[derive(Clone, Copy, Debug)]
pub enum Data<'a> {
    Json(Json<'a>),
    JsonText(Text<'a>),
    Text(Text<'a>),
    ObjectOrText { text: Text<'a>, key: Text<'a> },
    EmptyObject,
}
impl<'a> Data<'a> {
    fn raw(self) -> ir::Data<'a> {
        match self {
            Self::Json(v) => ir::Data::Json(v.0),
            Self::JsonText(v) => ir::Data::JsonText(v.0),
            Self::Text(v) => ir::Data::Text(v.0),
            Self::ObjectOrText { text, key } => ir::Data::ObjectOrText {
                text: text.0,
                key: key.0,
            },
            Self::EmptyObject => ir::Data::EmptyObject,
        }
    }
    fn from_raw(value: ir::Data<'a>) -> Self {
        match value {
            ir::Data::Json(v) => Self::Json(Json(v)),
            ir::Data::JsonText(v) => Self::JsonText(Text(v)),
            ir::Data::Text(v) => Self::Text(Text(v)),
            ir::Data::ObjectOrText { text, key } => Self::ObjectOrText {
                text: Text(text),
                key: Text(key),
            },
            ir::Data::EmptyObject => Self::EmptyObject,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum MediaSource<'a> {
    Url(Text<'a>),
    DataUrl(Text<'a>),
    Base64 { data: Text<'a>, mime_type: Text<'a> },
    FileId { id: Text<'a>, namespace: Format },
}
impl<'a> MediaSource<'a> {
    fn raw(self) -> ir::MediaSource<'a> {
        match self {
            Self::Url(v) => ir::MediaSource::Url(v.0),
            Self::DataUrl(v) => ir::MediaSource::DataUrl(v.0),
            Self::Base64 { data, mime_type } => ir::MediaSource::Base64 {
                data: data.0,
                mime_type: mime_type.0,
            },
            Self::FileId { id, namespace } => ir::MediaSource::FileId {
                id: id.0,
                namespace: namespace.raw(),
            },
        }
    }
    fn from_raw(value: ir::MediaSource<'a>) -> Self {
        match value {
            ir::MediaSource::Url(v) => Self::Url(Text(v)),
            ir::MediaSource::DataUrl(v) => Self::DataUrl(Text(v)),
            ir::MediaSource::Base64 { data, mime_type } => Self::Base64 {
                data: Text(data),
                mime_type: Text(mime_type),
            },
            ir::MediaSource::FileId { id, namespace } => Self::FileId {
                id: Text(id),
                namespace: Format::from_raw(namespace),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Media<'a> {
    pub kind: MediaKind,
    pub source: MediaSource<'a>,
    pub mime_type: Presence<Text<'a>>,
    pub detail: Presence<Text<'a>>,
    pub filename: Presence<Text<'a>>,
}
impl<'a> Media<'a> {
    fn raw(self) -> ir::Media<'a> {
        ir::Media {
            kind: self.kind.raw(),
            source: self.source.raw(),
            mime_type: text_raw(self.mime_type),
            detail: text_raw(self.detail),
            filename: text_raw(self.filename),
        }
    }
    fn from_raw(v: ir::Media<'a>) -> Self {
        Self {
            kind: MediaKind::from_raw(v.kind),
            source: MediaSource::from_raw(v.source),
            mime_type: text_view(v.mime_type),
            detail: text_view(v.detail),
            filename: text_view(v.filename),
        }
    }
}

/// One ordered content block. IDs here identify tool invocations; output item
/// IDs are separate metadata available through `BlockView::item_id`.
#[derive(Clone, Copy, Debug)]
pub enum Content<'a> {
    Text(Text<'a>),
    Reasoning {
        kind: ReasoningKind,
        text: Presence<Text<'a>>,
        signature: Presence<Text<'a>>,
    },
    Refusal(Text<'a>),
    Media(Media<'a>),
    ToolCall {
        id: Presence<Text<'a>>,
        name: Text<'a>,
        arguments: Data<'a>,
        legacy: bool,
    },
    ToolResult {
        id: Presence<Text<'a>>,
        name: Presence<Text<'a>>,
        result: Data<'a>,
        is_error: Presence<bool>,
    },
    Opaque {
        source: Format,
        value: Json<'a>,
    },
}
impl<'a> Content<'a> {
    pub const fn text(text: &'a str) -> Self {
        Self::Text(Text::new(text))
    }
    fn raw(self) -> ir::Content<'a> {
        match self {
            Self::Text(v) => ir::Content::Text(v.0),
            Self::Reasoning {
                kind,
                text,
                signature,
            } => ir::Content::Reasoning {
                kind: kind.raw(),
                text: text_raw(text),
                signature: text_raw(signature),
            },
            Self::Refusal(v) => ir::Content::Refusal(v.0),
            Self::Media(v) => ir::Content::Media(v.raw()),
            Self::ToolCall {
                id,
                name,
                arguments,
                legacy,
            } => ir::Content::ToolCall {
                id: text_raw(id),
                name: name.0,
                arguments: arguments.raw(),
                legacy,
            },
            Self::ToolResult {
                id,
                name,
                result,
                is_error,
            } => ir::Content::ToolResult {
                id: text_raw(id),
                name: text_raw(name),
                result: result.raw(),
                is_error: is_error.into_raw(),
            },
            Self::Opaque { source, value } => ir::Content::Opaque {
                source: source.raw(),
                value: value.0,
            },
        }
    }
    fn from_raw(value: ir::Content<'a>) -> Self {
        match value {
            ir::Content::Text(v) => Self::Text(Text(v)),
            ir::Content::Reasoning {
                kind,
                text,
                signature,
            } => Self::Reasoning {
                kind: ReasoningKind::from_raw(kind),
                text: text_view(text),
                signature: text_view(signature),
            },
            ir::Content::Refusal(v) => Self::Refusal(Text(v)),
            ir::Content::Media(v) => Self::Media(Media::from_raw(v)),
            ir::Content::ToolCall {
                id,
                name,
                arguments,
                legacy,
            } => Self::ToolCall {
                id: text_view(id),
                name: Text(name),
                arguments: Data::from_raw(arguments),
                legacy,
            },
            ir::Content::ToolResult {
                id,
                name,
                result,
                is_error,
            } => Self::ToolResult {
                id: text_view(id),
                name: text_view(name),
                result: Data::from_raw(result),
                is_error: Presence::from_raw(is_error),
            },
            ir::Content::Opaque { source, value } => Self::Opaque {
                source: Format::from_raw(source),
                value: Json(value),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ToolDefinition<'a> {
    pub name: Text<'a>,
    pub description: Presence<Text<'a>>,
    pub parameters: Presence<Json<'a>>,
    pub schema_dialect: SchemaDialect,
    /// Missing/null retain the authoring format's strict-default semantics.
    pub strict: Presence<bool>,
    pub legacy: bool,
}
impl<'a> ToolDefinition<'a> {
    pub const fn new(name: &'a str) -> Self {
        Self {
            name: Text::new(name),
            description: Presence::Missing,
            parameters: Presence::Missing,
            schema_dialect: SchemaDialect::JsonSchema,
            strict: Presence::Missing,
            legacy: false,
        }
    }
    fn raw(self) -> ir::Tool<'a> {
        ir::Tool {
            name: self.name.0,
            description: text_raw(self.description),
            parameters: json_raw(self.parameters),
            schema_dialect: self.schema_dialect.raw(),
            strict: self.strict.into_raw(),
            extensions: ir::Span::EMPTY,
            legacy: self.legacy,
        }
    }
    fn from_raw(value: ir::Tool<'a>) -> Self {
        Self {
            name: Text(value.name),
            description: text_view(value.description),
            parameters: json_view(value.parameters),
            schema_dialect: SchemaDialect::from_raw(value.schema_dialect),
            strict: Presence::from_raw(value.strict),
            legacy: value.legacy,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub enum ToolChoice<'a> {
    Missing,
    Null,
    Auto,
    None,
    Required,
    Named(Text<'a>),
    Opaque { source: Format, value: Json<'a> },
}
impl<'a> ToolChoice<'a> {
    fn raw(self) -> ir::ToolChoice<'a> {
        match self {
            Self::Missing => ir::ToolChoice::Missing,
            Self::Null => ir::ToolChoice::Null,
            Self::Auto => ir::ToolChoice::Auto,
            Self::None => ir::ToolChoice::None,
            Self::Required => ir::ToolChoice::Required,
            Self::Named(v) => ir::ToolChoice::Named(v.0),
            Self::Opaque { source, value } => ir::ToolChoice::Opaque {
                source: source.raw(),
                value: value.0,
            },
        }
    }
    fn from_raw(v: ir::ToolChoice<'a>) -> Self {
        match v {
            ir::ToolChoice::Missing => Self::Missing,
            ir::ToolChoice::Null => Self::Null,
            ir::ToolChoice::Auto => Self::Auto,
            ir::ToolChoice::None => Self::None,
            ir::ToolChoice::Required => Self::Required,
            ir::ToolChoice::Named(v) => Self::Named(Text(v)),
            ir::ToolChoice::Opaque { source, value } => Self::Opaque {
                source: Format::from_raw(source),
                value: Json(value),
            },
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct ContextRef<'a> {
    pub source: Format,
    pub id: Text<'a>,
}
impl<'a> ContextRef<'a> {
    fn raw(self) -> ir::ContextRef<'a> {
        ir::ContextRef {
            source: self.source.raw(),
            id: self.id.0,
        }
    }
    fn from_raw(v: ir::ContextRef<'a>) -> Self {
        Self {
            source: Format::from_raw(v.source),
            id: Text(v.id),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct OutputConstraint<'a> {
    pub source: Format,
    pub value: Json<'a>,
    pub mode: OutputMode,
    pub schema: Presence<Json<'a>>,
    pub schema_dialect: SchemaDialect,
    pub name: Presence<Text<'a>>,
    pub description: Presence<Text<'a>>,
    pub strict: Presence<bool>,
}
impl<'a> OutputConstraint<'a> {
    fn raw(self) -> ir::OutputConstraint<'a> {
        ir::OutputConstraint {
            source: self.source.raw(),
            value: self.value.0,
            mode: self.mode.raw(),
            schema: json_raw(self.schema),
            schema_dialect: self.schema_dialect.raw(),
            name: text_raw(self.name),
            description: text_raw(self.description),
            strict: self.strict.into_raw(),
        }
    }
    fn from_raw(v: ir::OutputConstraint<'a>) -> Self {
        Self {
            source: Format::from_raw(v.source),
            value: Json(v.value),
            mode: OutputMode::from_raw(v.mode),
            schema: json_view(v.schema),
            schema_dialect: SchemaDialect::from_raw(v.schema_dialect),
            name: text_view(v.name),
            description: text_view(v.description),
            strict: Presence::from_raw(v.strict),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Parameter {
    Temperature,
    TopP,
    TopK,
    MaxOutputTokens,
    Stop,
    Seed,
    FrequencyPenalty,
    PresencePenalty,
    Candidates,
    Stream,
    ParallelToolCalls,
}

fn parameter_get<'a>(
    p: ir::Parameters<'a>,
    field: Parameter,
) -> ir::Presence<zconv::json::RawValue<'a>> {
    match field {
        Parameter::Temperature => p.temperature,
        Parameter::TopP => p.top_p,
        Parameter::TopK => p.top_k,
        Parameter::MaxOutputTokens => p.max_output_tokens,
        Parameter::Stop => p.stop,
        Parameter::Seed => p.seed,
        Parameter::FrequencyPenalty => p.frequency_penalty,
        Parameter::PresencePenalty => p.presence_penalty,
        Parameter::Candidates => p.n,
        Parameter::Stream => p.stream,
        Parameter::ParallelToolCalls => p.parallel_tool_calls,
    }
}
fn parameter_set<'a>(
    p: &mut ir::Parameters<'a>,
    field: Parameter,
    value: Presence<Json<'a>>,
) -> Result<(), Error> {
    if let Presence::Value(v) = value {
        let valid = match field {
            Parameter::Stream | Parameter::ParallelToolCalls => v.kind() == JsonKind::Bool,
            Parameter::Stop => matches!(v.kind(), JsonKind::String | JsonKind::Array),
            Parameter::TopK | Parameter::MaxOutputTokens | Parameter::Candidates => {
                v.as_u64().is_some()
            }
            Parameter::Seed => {
                v.kind() == JsonKind::Number
                    && v.raw()
                        .strip_prefix(b"-")
                        .unwrap_or(v.raw())
                        .iter()
                        .all(u8::is_ascii_digit)
            }
            _ => v.kind() == JsonKind::Number,
        };
        if !valid {
            return Err(Error::invalid("parameter", "incorrect JSON value type"));
        }
    }
    let value = json_raw(value);
    match field {
        Parameter::Temperature => p.temperature = value,
        Parameter::TopP => p.top_p = value,
        Parameter::TopK => p.top_k = value,
        Parameter::MaxOutputTokens => p.max_output_tokens = value,
        Parameter::Stop => p.stop = value,
        Parameter::Seed => p.seed = value,
        Parameter::FrequencyPenalty => p.frequency_penalty = value,
        Parameter::PresencePenalty => p.presence_penalty = value,
        Parameter::Candidates => p.n = value,
        Parameter::Stream => p.stream = value,
        Parameter::ParallelToolCalls => p.parallel_tool_calls = value,
    }
    Ok(())
}

fn text_view(value: ir::Presence<ir::Text<'_>>) -> Presence<Text<'_>> {
    Presence::from_raw(value).map(Text)
}
fn text_raw(value: Presence<Text<'_>>) -> ir::Presence<ir::Text<'_>> {
    value.map(|v| v.0).into_raw()
}
fn json_view(value: ir::Presence<zconv::json::RawValue<'_>>) -> Presence<Json<'_>> {
    Presence::from_raw(value).map(Json)
}
fn json_raw(value: Presence<Json<'_>>) -> ir::Presence<zconv::json::RawValue<'_>> {
    value.map(|v| v.0).into_raw()
}

/// Narrow, opt-in permissions. Every loss can be enumerated with `losses`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Policy {
    pub drop_extensions: bool,
    pub drop_reasoning: bool,
    pub drop_context: bool,
    pub drop_metadata: bool,
}
impl Policy {
    fn raw(self) -> ir::Policy {
        ir::Policy {
            drop_extensions: self.drop_extensions,
            drop_reasoning: self.drop_reasoning,
            drop_context: self.drop_context,
            drop_metadata: self.drop_metadata,
        }
    }
}

/// A caller-provided ID/name map. It holds borrowed strings and has no hidden
/// allocation. Missing mappings remain errors instead of generated identities.
pub struct ToolBindingStorage<'a, const N: usize> {
    inner: [ir::ToolBinding<'a>; N],
    used: usize,
}
impl<'a, const N: usize> ToolBindingStorage<'a, N> {
    pub const fn new() -> Self {
        Self {
            inner: [ir::ToolBinding { id: "", name: "" }; N],
            used: 0,
        }
    }
    pub fn push(&mut self, id: &'a str, name: &'a str) -> Result<(), Error> {
        if id.is_empty() || name.is_empty() {
            return Err(Error::invalid(
                "tool binding",
                "ID and name must be nonempty",
            ));
        }
        if self.inner[..self.used].iter().any(|v| v.id == id) {
            return Err(Error::invalid("tool binding", "duplicate ID"));
        }
        let slot = self.inner.get_mut(self.used).ok_or_else(|| {
            Error::capacity(
                Resource::Context,
                self.used.saturating_add(1),
                true,
                Replay::Unconsumed,
            )
        })?;
        *slot = ir::ToolBinding { id, name };
        self.used += 1;
        Ok(())
    }
    pub const fn len(&self) -> usize {
        self.used
    }
    pub const fn is_empty(&self) -> bool {
        self.used == 0
    }
    pub const fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}
impl<const N: usize> Default for ToolBindingStorage<'_, N> {
    fn default() -> Self {
        Self::new()
    }
}

/// Facts absent from the input. Values remain borrowed; no clock or identifier
/// generator is consulted. An item ID has its own namespace from a tool call ID.
#[derive(Clone, Copy, Debug, Default)]
pub struct EncodeContext<'a> {
    inner: ir::Context<'a>,
}
impl<'a> EncodeContext<'a> {
    pub(crate) fn max_tokens(&self) -> Option<u64> {
        self.inner.max_tokens
    }
    pub fn with_model(mut self, model: &'a str) -> Self {
        self.inner.model = Some(model);
        self
    }
    pub fn with_response_id(mut self, id: &'a str) -> Self {
        self.inner.response_id = Some(id);
        self
    }
    pub fn with_created(mut self, timestamp: u64) -> Self {
        self.inner.created = Some(timestamp);
        self
    }
    pub fn with_created_iso(mut self, value: &'a str) -> Self {
        self.inner.created_iso = Some(value);
        self
    }
    pub fn with_updated_iso(mut self, value: &'a str) -> Self {
        self.inner.updated_iso = Some(value);
        self
    }
    pub fn with_max_tokens(mut self, limit: u64) -> Self {
        self.inner.max_tokens = Some(limit);
        self
    }
    pub fn with_item_ids(mut self, ids: &'a [&'a str]) -> Self {
        self.inner.item_ids = ids;
        self
    }
    pub fn with_tool_bindings<const N: usize>(
        mut self,
        bindings: &'a ToolBindingStorage<'a, N>,
    ) -> Self {
        self.inner.tool_calls = &bindings.inner[..bindings.used];
        self
    }
}

#[derive(Clone, Copy, Debug)]
pub struct EncodeReport<'a> {
    pub written: usize,
    pub dropped_extensions: usize,
    pub dropped_reasoning: usize,
    pub dropped_context: usize,
    pub dropped_metadata: usize,
    /// GEMINI model and streaming mode belong to the caller's transport URL.
    pub transport_model: Presence<Text<'a>>,
    pub transport_stream: Presence<bool>,
}
impl<'a> EncodeReport<'a> {
    fn from_raw(v: ir::EncodeReport<'a>) -> Self {
        Self {
            written: v.written,
            dropped_extensions: v.dropped_extensions,
            dropped_reasoning: v.dropped_reasoning,
            dropped_context: v.dropped_context,
            dropped_metadata: v.dropped_metadata,
            transport_model: text_view(v.transport_model),
            transport_stream: Presence::from_raw(v.transport_stream),
        }
    }
    pub fn has_losses(self) -> bool {
        self.dropped_extensions != 0
            || self.dropped_reasoning != 0
            || self.dropped_context != 0
            || self.dropped_metadata != 0
    }
}

#[derive(Clone, Copy)]
struct RequestHead<'a> {
    source: zconv::Format,
    model: ir::Presence<ir::Text<'a>>,
    tools: ir::Presence<()>,
    functions: ir::Presence<()>,
    instruction_presence: ir::Presence<()>,
    parameters: ir::Parameters<'a>,
    tool_choice: ir::ToolChoice<'a>,
    context: ir::Presence<ir::ContextRef<'a>>,
    output_constraint: ir::Presence<ir::OutputConstraint<'a>>,
    extensions: ir::Span,
}
impl<'a> RequestHead<'a> {
    fn new(source: Format, model: Presence<Text<'a>>) -> Self {
        Self {
            source: source.raw(),
            model: text_raw(model),
            tools: ir::Presence::Missing,
            functions: ir::Presence::Missing,
            instruction_presence: ir::Presence::Missing,
            parameters: ir::Parameters::default(),
            tool_choice: ir::ToolChoice::Missing,
            context: ir::Presence::Missing,
            output_constraint: ir::Presence::Missing,
            extensions: ir::Span::EMPTY,
        }
    }
    fn take(raw: ir::Request<'a, '_>) -> (Self, usize) {
        (
            Self {
                source: raw.source,
                model: raw.model,
                tools: raw.tools,
                functions: raw.functions,
                instruction_presence: raw.instruction_presence,
                parameters: raw.parameters,
                tool_choice: raw.tool_choice,
                context: raw.context,
                output_constraint: raw.output_constraint,
                extensions: raw.extensions,
            },
            raw.records.len(),
        )
    }
    fn view<'s>(self, records: &'s mut [ir::Record<'a>]) -> ir::Request<'a, 's> {
        ir::Request {
            source: self.source,
            model: self.model,
            tools: self.tools,
            functions: self.functions,
            instruction_presence: self.instruction_presence,
            parameters: self.parameters,
            tool_choice: self.tool_choice,
            context: self.context,
            output_constraint: self.output_constraint,
            extensions: self.extensions,
            records,
        }
    }
}

/// A borrowed, editable request. Iterators borrow it, preventing edits while a
/// record view is in use. Appending writes directly into the same IR storage.
pub struct Request<'a, 's> {
    head: RequestHead<'a>,
    records: &'s mut [ir::Record<'a>],
    used: usize,
}
impl<'a, 's> Request<'a, 's> {
    pub fn decode<const T: usize, const R: usize>(
        source: Format,
        input: &'a [u8],
        tokens: &mut TokenStorage<T>,
        storage: &'s mut RecordStorage<'a, R>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let document =
            zconv::json::parse(input, &mut tokens.inner, limits.raw()).map_err(Error::from)?;
        let (head, used) = RequestHead::take(
            zconv::protocol::decode_request(source.raw(), document.root(), &mut storage.inner)
                .map_err(Error::from)?,
        );
        Ok(Self {
            head,
            records: &mut storage.inner,
            used,
        })
    }
    pub fn source(&self) -> Format {
        Format::from_raw(self.head.source)
    }
    pub fn model(&self) -> Presence<Text<'a>> {
        text_view(self.head.model)
    }
    pub fn set_model(&mut self, value: Presence<Text<'a>>) {
        self.head.model = text_raw(value);
    }
    pub fn tool_presence(&self) -> Presence<()> {
        Presence::from_raw(self.head.tools)
    }
    pub fn parameter(&self, parameter: Parameter) -> Presence<Json<'a>> {
        json_view(parameter_get(self.head.parameters, parameter))
    }
    pub fn set_parameter(
        &mut self,
        parameter: Parameter,
        value: Presence<Json<'a>>,
    ) -> Result<(), Error> {
        parameter_set(&mut self.head.parameters, parameter, value)
    }
    pub fn tool_choice(&self) -> ToolChoice<'a> {
        ToolChoice::from_raw(self.head.tool_choice)
    }
    pub fn set_tool_choice(&mut self, value: ToolChoice<'a>) {
        self.head.tool_choice = value.raw();
    }
    pub fn context(&self) -> Presence<ContextRef<'a>> {
        Presence::from_raw(self.head.context).map(ContextRef::from_raw)
    }
    pub fn set_context(&mut self, value: Presence<ContextRef<'a>>) {
        self.head.context = value.map(ContextRef::raw).into_raw();
    }
    pub fn output_constraint(&self) -> Presence<OutputConstraint<'a>> {
        Presence::from_raw(self.head.output_constraint).map(OutputConstraint::from_raw)
    }
    pub fn set_output_constraint(&mut self, value: Presence<OutputConstraint<'a>>) {
        self.head.output_constraint = value.map(OutputConstraint::raw).into_raw();
    }
    pub fn records_used(&self) -> usize {
        self.used
    }
    pub fn record_capacity(&self) -> usize {
        self.records.len()
    }
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>() + core::mem::size_of_val(self.records)
    }
    pub fn messages(&self) -> Messages<'_, 'a> {
        Messages {
            records: &self.records[..self.used],
            at: 0,
            ordinal: 0,
        }
    }
    pub fn tools(&self) -> Tools<'_, 'a> {
        Tools {
            records: &self.records[..self.used],
            at: 0,
        }
    }
    pub fn extensions(&self) -> Extensions<'_, 'a> {
        Extensions {
            records: &self.records[..self.used],
            at: 0,
            owner: None,
        }
    }

    /// A capacity failure leaves the request unchanged and consumes no content.
    pub fn append_message(&mut self, role: Role, contents: &[Content<'a>]) -> Result<(), Error> {
        self.append_message_with(MessageOptions::new(role), contents)
    }
    pub fn append_message_with(
        &mut self,
        options: MessageOptions<'a>,
        contents: &[Content<'a>],
    ) -> Result<(), Error> {
        append_message(self.records, &mut self.used, options, contents)
    }
    pub fn append_tool(&mut self, tool: ToolDefinition<'a>) -> Result<(), Error> {
        if tool.name.decoded_len() == 0 {
            return Err(Error::invalid("tool name", "empty tool name"));
        }
        if self.tools().any(|v| texts_equal(v.name, tool.name)) {
            return Err(Error::invalid("tool name", "duplicate tool definition"));
        }
        if let Presence::Value(schema) = tool.parameters
            && schema.kind() != JsonKind::Object
        {
            return Err(Error::invalid(
                "tool parameters",
                "expected a JSON schema object",
            ));
        }
        let slot = self.records.get_mut(self.used).ok_or_else(|| {
            Error::capacity(
                Resource::Ir,
                self.used.saturating_add(1),
                true,
                Replay::Unconsumed,
            )
        })?;
        *slot = ir::Record::Tool(tool.raw());
        self.used += 1;
        if tool.legacy {
            self.head.functions = ir::Presence::Value(());
        } else {
            self.head.tools = ir::Presence::Value(());
        }
        Ok(())
    }
    pub fn replace_content(
        &mut self,
        message: usize,
        block: usize,
        content: Content<'a>,
    ) -> Result<(), Error> {
        let parent = locate_message(&self.records[..self.used], message)?;
        let role = match self.records[parent] {
            ir::Record::Message(v) => Role::from_raw(v.role),
            _ => return Err(Error::invalid("message", "unknown message")),
        };
        validate_content(role, content)?;
        let index = locate_block(&self.records[..self.used], message, block)?;
        if let ir::Record::Block(ref mut slot) = self.records[index] {
            slot.content = content.raw();
        }
        refresh_message(&mut self.records[..self.used], parent);
        Ok(())
    }
    pub fn set_block_signature(
        &mut self,
        message: usize,
        block: usize,
        signature: Presence<ProviderSignature<'a>>,
    ) -> Result<(), Error> {
        let index = locate_block(&self.records[..self.used], message, block)?;
        if let ir::Record::Block(ref mut slot) = self.records[index] {
            slot.signature = signature.map(ProviderSignature::raw).into_raw();
        }
        Ok(())
    }
    pub fn set_message_role(&mut self, message: usize, role: Role) -> Result<(), Error> {
        let index = locate_message(&self.records[..self.used], message)?;
        let end = match self.records[index] {
            ir::Record::Message(v) => v.end,
            _ => return Err(Error::invalid("message", "unknown message")),
        };
        for record in &self.records[index + 1..end] {
            if let ir::Record::Block(block) = record {
                validate_content(role, Content::from_raw(block.content))?;
            }
        }
        if let ir::Record::Message(ref mut slot) = self.records[index] {
            slot.role = role.raw();
        }
        Ok(())
    }
    /// Add a source-owned root field without borrowing or constructing another IR.
    /// The key must be a validated JSON string. Encoding rejects collisions with
    /// normalized protocol fields; cross-format use requires explicit loss policy.
    pub fn add_extension(&mut self, key: Json<'a>, value: Json<'a>) -> Result<(), Error> {
        let name = key
            .0
            .as_string()
            .ok_or_else(|| Error::invalid("extension key", "expected a JSON string"))?;
        if self
            .extensions()
            .any(|e| e.scope == "request" && texts_equal(e.name, Text(ir::Text::Json(name))))
        {
            return Err(Error::invalid("extension key", "duplicate root extension"));
        }
        let old = self.head.extensions;
        let at = if old.start == old.end {
            self.used
        } else {
            old.end
        };
        let record = ir::Record::Extension(ir::Extension {
            source: self.head.source,
            key: name,
            value: value.0,
            location: "request",
            owner: usize::MAX,
        });
        insert_record(self.records, &mut self.used, at, record)?;
        shift_span(&mut self.head.parameters.extensions, at);
        self.head.extensions = if old.start == old.end {
            ir::Span {
                start: at,
                end: at + 1,
            }
        } else {
            ir::Span {
                start: old.start,
                end: old.end + 1,
            }
        };
        Ok(())
    }
    pub fn set_tool_strict(&mut self, tool: usize, value: Presence<bool>) -> Result<(), Error> {
        let mut ordinal = 0;
        for record in &mut self.records[..self.used] {
            if let ir::Record::Tool(slot) = record {
                if ordinal == tool {
                    slot.strict = value.into_raw();
                    return Ok(());
                }
                ordinal += 1;
            }
        }
        Err(Error::invalid("tool", "tool ordinal is out of range"))
    }
    /// Append a result only for an existing tool call in this request history.
    /// IDs are compared after JSON escape decoding. Unknown IDs, duplicated call
    /// IDs and repeated results fail without modifying the request.
    pub fn append_tool_result(
        &mut self,
        id: &'a str,
        result: Data<'a>,
        is_error: Presence<bool>,
    ) -> Result<(), Error> {
        let mut name = None;
        let mut seen_result = false;
        for record in &self.records[..self.used] {
            if let ir::Record::Block(b) = record {
                match b.content {
                    ir::Content::ToolCall {
                        id: ir::Presence::Value(call_id),
                        name: call_name,
                        ..
                    } if call_id.eq_str(id) => {
                        if name.is_some() {
                            return Err(Error::lifecycle(
                                "tool result",
                                "ambiguous duplicate call ID",
                            ));
                        }
                        name = Some(Text(call_name));
                    }
                    ir::Content::ToolResult {
                        id: ir::Presence::Value(call_id),
                        ..
                    } if call_id.eq_str(id) => seen_result = true,
                    _ => {}
                }
            }
        }
        let name = name.ok_or_else(|| Error::lifecycle("tool result", "unknown call ID"))?;
        if seen_result {
            return Err(Error::lifecycle(
                "tool result",
                "a result was already supplied for this call ID",
            ));
        }
        let role = if matches!(
            self.source().protocol,
            Protocol::AnthropicMessages
                | Protocol::GeminiGenerateContent
                | Protocol::GeminiInteractions
        ) {
            Role::User
        } else {
            Role::Tool
        };
        self.append_message(
            role,
            &[Content::ToolResult {
                id: Presence::Value(Text::new(id)),
                name: Presence::Value(name),
                result,
                is_error,
            }],
        )
    }
    /// Encode from the inspected/edited IR. No direct path bypasses the view.
    /// On failure discard output; capacity errors permit retrying this operation.
    pub fn encode<const T: usize>(
        &mut self,
        target: Format,
        output: &mut [u8],
        scratch: &mut [u8],
        tokens: &mut TokenStorage<T>,
        context: EncodeContext<'a>,
        policy: Policy,
    ) -> Result<EncodeReport<'a>, Error> {
        if !matches!(
            target.protocol,
            Protocol::OpenAiChat | Protocol::OpenAiResponses
        ) && self.records[..self.used].iter().any(|r| {
            matches!(
                r,
                ir::Record::Block(ir::Block {
                    content: ir::Content::Refusal(_),
                    ..
                })
            )
        }) {
            return Err(Error::unsupported("refusal in request history"));
        }
        validate_tool_arguments(&self.records[..self.used], scratch, &mut tokens.inner)?;
        if let ir::Presence::Value(stop) = self.head.parameters.stop
            && stop.kind() == zconv::json::Kind::Array
        {
            let document = zconv::json::parse(
                stop.raw(),
                &mut tokens.inner,
                zconv::json::Limits::default(),
            )
            .map_err(Error::from)?;
            if document
                .root()
                .array()
                .any(|v| v.kind() != zconv::json::Kind::String)
            {
                return Err(Error::invalid("stop", "expected an array of strings"));
            }
        }
        let raw = self.head.view(&mut self.records[..self.used]);
        zconv::protocol::encode_request(
            &raw,
            target.raw(),
            &context.inner,
            policy.raw(),
            output,
            scratch,
            &mut tokens.inner,
        )
        .map(EncodeReport::from_raw)
        .map_err(encode_error)
    }
    pub fn losses(&self, target: Format, policy: Policy) -> Losses<'_, 'a> {
        let context = match self.head.context {
            ir::Presence::Value(v) if v.source != target.raw() && policy.drop_context => {
                Some(Format::from_raw(v.source))
            }
            ir::Presence::Null if self.head.source != target.raw() && policy.drop_context => {
                Some(self.source())
            }
            _ => None,
        };
        Losses::new(&self.records[..self.used], target, policy, context, 0)
    }
    /// Replace storage in place, copying only used IR records. On failure the
    /// edited request and all its storage remain unchanged. The new region is
    /// borrowed for this request's existing storage lifetime.
    pub fn replace_storage<const N: usize>(
        &mut self,
        storage: &'s mut RecordStorage<'a, N>,
    ) -> Result<(), Error> {
        if N < self.used {
            return Err(Error::capacity(
                Resource::Ir,
                self.used,
                true,
                Replay::Unconsumed,
            ));
        }
        storage.inner[..self.used].copy_from_slice(&self.records[..self.used]);
        self.records = &mut storage.inner;
        Ok(())
    }
    /// Move the bounded IR into larger caller storage without reparsing input.
    /// The old storage is released on success. On failure `original` retains
    /// the edited request so the caller can retry without losing state.
    // Returning the owner preserves recoverability without a heap allocation.
    #[allow(clippy::result_large_err)]
    pub fn relocate<'n, const N: usize>(
        self,
        storage: &'n mut RecordStorage<'a, N>,
    ) -> Result<Request<'a, 'n>, RelocationError<Self>> {
        if N < self.used {
            let error = Error::capacity(Resource::Ir, self.used, true, Replay::Unconsumed);
            return Err(RelocationError {
                original: self,
                error,
            });
        }
        storage.inner[..self.used].copy_from_slice(&self.records[..self.used]);
        Ok(Request {
            head: self.head,
            records: &mut storage.inner,
            used: self.used,
        })
    }
}

/// Authoring format declares the source semantics (including strict defaults).
/// It is a caller choice, never guessed or substituted to bypass a conversion.
pub struct RequestBuilder<'a, 's> {
    request: Request<'a, 's>,
}
impl<'a, 's> RequestBuilder<'a, 's> {
    pub fn new<const N: usize>(
        authoring: Format,
        model: Presence<Text<'a>>,
        storage: &'s mut RecordStorage<'a, N>,
    ) -> Result<Self, Error> {
        authoring.validate()?;
        Ok(Self {
            request: Request {
                head: RequestHead::new(authoring, model),
                records: &mut storage.inner,
                used: 0,
            },
        })
    }
    pub fn message(&mut self, role: Role, contents: &[Content<'a>]) -> Result<&mut Self, Error> {
        self.request.append_message(role, contents)?;
        Ok(self)
    }
    pub fn message_with(
        &mut self,
        options: MessageOptions<'a>,
        contents: &[Content<'a>],
    ) -> Result<&mut Self, Error> {
        self.request.append_message_with(options, contents)?;
        Ok(self)
    }
    pub fn tool(&mut self, tool: ToolDefinition<'a>) -> Result<&mut Self, Error> {
        self.request.append_tool(tool)?;
        Ok(self)
    }
    pub fn parameter(
        &mut self,
        parameter: Parameter,
        value: Presence<Json<'a>>,
    ) -> Result<&mut Self, Error> {
        self.request.set_parameter(parameter, value)?;
        Ok(self)
    }
    pub fn tool_choice(&mut self, value: ToolChoice<'a>) -> &mut Self {
        self.request.set_tool_choice(value);
        self
    }
    pub fn context(&mut self, value: Presence<ContextRef<'a>>) -> &mut Self {
        self.request.set_context(value);
        self
    }
    pub fn output_constraint(&mut self, value: Presence<OutputConstraint<'a>>) -> &mut Self {
        self.request.set_output_constraint(value);
        self
    }
    pub fn extension(&mut self, key: Json<'a>, value: Json<'a>) -> Result<&mut Self, Error> {
        self.request.add_extension(key, value)?;
        Ok(self)
    }
    pub fn finish(self) -> Request<'a, 's> {
        self.request
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MessageOptions<'a> {
    pub role: Role,
    pub name: Presence<Text<'a>>,
    pub shape: ContentShape,
    pub item_id: Presence<Text<'a>>,
    pub item_status: Presence<Text<'a>>,
    pub system_instruction: bool,
}
impl MessageOptions<'_> {
    pub const fn new(role: Role) -> Self {
        Self {
            role,
            name: Presence::Missing,
            shape: ContentShape::Array,
            item_id: Presence::Missing,
            item_status: Presence::Missing,
            system_instruction: false,
        }
    }
}
fn append_message<'a>(
    records: &mut [ir::Record<'a>],
    used: &mut usize,
    options: MessageOptions<'a>,
    contents: &[Content<'a>],
) -> Result<(), Error> {
    let needed = used
        .checked_add(1)
        .and_then(|v| v.checked_add(contents.len()))
        .ok_or_else(|| Error::capacity(Resource::Ir, usize::MAX, false, Replay::Unconsumed))?;
    if needed > records.len() {
        return Err(Error::capacity(
            Resource::Ir,
            needed,
            true,
            Replay::Unconsumed,
        ));
    }
    if matches!(options.shape, ContentShape::Missing | ContentShape::Null) && !contents.is_empty() {
        return Err(Error::invalid(
            "message content",
            "missing/null content cannot contain blocks",
        ));
    }
    if options.shape == ContentShape::String && !matches!(contents, [Content::Text(_)]) {
        return Err(Error::invalid(
            "message content",
            "string content requires exactly one text block",
        ));
    }
    for &content in contents {
        validate_content(options.role, content)?;
    }
    let start = *used;
    let marker = |yes| {
        if yes {
            ir::Presence::Value(())
        } else {
            ir::Presence::Missing
        }
    };
    records[start] = ir::Record::Message(ir::Message {
        role: options.role.raw(),
        legacy_function: false,
        system_instruction: options.system_instruction,
        tool_calls: marker(
            contents
                .iter()
                .any(|c| matches!(c, Content::ToolCall { legacy: false, .. })),
        ),
        function_call: marker(
            contents
                .iter()
                .any(|c| matches!(c, Content::ToolCall { legacy: true, .. })),
        ),
        refusal: marker(contents.iter().any(|c| matches!(c, Content::Refusal(_)))),
        reasoning: marker(
            contents
                .iter()
                .any(|c| matches!(c, Content::Reasoning { .. })),
        ),
        name: text_raw(options.name),
        shape: options.shape.raw(),
        end: needed,
        extensions: ir::Span::EMPTY,
        item_id: text_raw(options.item_id),
        item_status: text_raw(options.item_status),
    });
    for (offset, content) in contents.iter().enumerate() {
        let at = start + 1 + offset;
        records[at] = ir::Record::Block(ir::Block {
            content: content.raw(),
            signature: ir::Presence::Missing,
            extensions: ir::Span::EMPTY,
            end: at + 1,
            item_id: ir::Presence::Missing,
            item_status: ir::Presence::Missing,
        });
    }
    *used = needed;
    Ok(())
}
fn texts_equal(a: Text<'_>, b: Text<'_>) -> bool {
    fn bytes(text: Text<'_>) -> TextBytes<'_> {
        match text.0 {
            ir::Text::Utf8(v) => TextBytes::Plain(v.as_bytes().iter()),
            ir::Text::Json(v) => TextBytes::Json(v.bytes()),
        }
    }
    bytes(a).eq(bytes(b))
}
enum TextBytes<'a> {
    Plain(core::slice::Iter<'a, u8>),
    Json(zconv::json::DecodedBytes<'a>),
}
impl Iterator for TextBytes<'_> {
    type Item = u8;
    fn next(&mut self) -> Option<u8> {
        match self {
            Self::Plain(v) => v.next().copied(),
            Self::Json(v) => v.next(),
        }
    }
}
fn locate_message(records: &[ir::Record<'_>], ordinal: usize) -> Result<usize, Error> {
    records
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, ir::Record::Message(_)))
        .nth(ordinal)
        .map(|(i, _)| i)
        .ok_or_else(|| Error::invalid("message", "message ordinal is out of range"))
}
fn locate_block(records: &[ir::Record<'_>], message: usize, block: usize) -> Result<usize, Error> {
    let at = locate_message(records, message)?;
    let end = match records[at] {
        ir::Record::Message(v) => v.end,
        _ => return Err(Error::invalid("message", "invalid message")),
    };
    records
        .get(at + 1..end)
        .ok_or_else(|| Error::invalid("message", "invalid message bounds"))?
        .iter()
        .enumerate()
        .filter(|(_, v)| matches!(v, ir::Record::Block(_)))
        .nth(block)
        .map(|(i, _)| i + at + 1)
        .ok_or_else(|| Error::invalid("content", "content ordinal is out of range"))
}
fn encode_error(value: zconv::Error) -> Error {
    match value {
        zconv::Error::EventTooLarge { needed } => {
            Error::capacity(Resource::Scratch, needed, false, Replay::Unconsumed)
        }
        other => Error::from(other),
    }
}

/// Stable ordinals in the semantic document. These are not arena offsets.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct Location {
    pub choice: Option<u64>,
    pub message: Option<usize>,
    pub content: Option<usize>,
    pub tool: Option<usize>,
}

#[derive(Clone, Copy, Debug)]
pub struct Extension<'a> {
    pub source: Format,
    pub name: Text<'a>,
    pub value: Json<'a>,
    pub scope: &'static str,
    pub location: Location,
}
pub struct Extensions<'s, 'a> {
    records: &'s [ir::Record<'a>],
    at: usize,
    owner: Option<usize>,
}
impl<'a> Iterator for Extensions<'_, 'a> {
    type Item = Extension<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(record) = self.records.get(self.at) {
            self.at += 1;
            if let ir::Record::Extension(v) = record
                && self.owner.is_none_or(|owner| owner == v.owner)
            {
                return Some(Extension {
                    source: Format::from_raw(v.source),
                    name: Text(ir::Text::Json(v.key)),
                    value: Json(v.value),
                    scope: v.location,
                    location: location_for(self.records, v.owner),
                });
            }
        }
        None
    }
}

pub struct Messages<'s, 'a> {
    records: &'s [ir::Record<'a>],
    at: usize,
    ordinal: usize,
}
impl<'s, 'a> Iterator for Messages<'s, 'a> {
    type Item = MessageView<'s, 'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(record) = self.records.get(self.at) {
            let at = self.at;
            self.at += 1;
            if let ir::Record::Message(value) = record {
                let ordinal = self.ordinal;
                self.ordinal += 1;
                self.at = value.end.min(self.records.len());
                return Some(MessageView {
                    records: self.records,
                    value: *value,
                    at,
                    ordinal,
                });
            }
        }
        None
    }
}
#[derive(Clone, Copy)]
pub struct MessageView<'s, 'a> {
    records: &'s [ir::Record<'a>],
    value: ir::Message<'a>,
    at: usize,
    ordinal: usize,
}
impl<'s, 'a> MessageView<'s, 'a> {
    pub fn role(self) -> Role {
        Role::from_raw(self.value.role)
    }
    pub fn ordinal(self) -> usize {
        self.ordinal
    }
    pub fn name(self) -> Presence<Text<'a>> {
        text_view(self.value.name)
    }
    pub fn shape(self) -> ContentShape {
        ContentShape::from_raw(self.value.shape)
    }
    pub fn item_id(self) -> Presence<Text<'a>> {
        text_view(self.value.item_id)
    }
    pub fn item_status(self) -> Presence<Text<'a>> {
        text_view(self.value.item_status)
    }
    pub fn system_instruction(self) -> bool {
        self.value.system_instruction
    }
    pub fn contents(self) -> Blocks<'s, 'a> {
        Blocks {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.at.saturating_add(1),
            ordinal: 0,
        }
    }
    pub fn extensions(self) -> Extensions<'s, 'a> {
        Extensions {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.value.extensions.start,
            owner: Some(self.at),
        }
    }
}

pub struct Blocks<'s, 'a> {
    records: &'s [ir::Record<'a>],
    at: usize,
    ordinal: usize,
}
impl<'s, 'a> Iterator for Blocks<'s, 'a> {
    type Item = BlockView<'s, 'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(record) = self.records.get(self.at) {
            let at = self.at;
            self.at += 1;
            if let ir::Record::Block(value) = record {
                let ordinal = self.ordinal;
                self.ordinal += 1;
                self.at = value.end.min(self.records.len());
                return Some(BlockView {
                    records: self.records,
                    value: *value,
                    at,
                    ordinal,
                });
            }
        }
        None
    }
}
/// Provider-bound metadata that must not move silently across wire formats.
#[derive(Clone, Copy, Debug)]
pub struct ProviderSignature<'a> {
    pub source: Format,
    pub value: Text<'a>,
}
impl<'a> ProviderSignature<'a> {
    fn raw(self) -> ir::ProviderSignature<'a> {
        ir::ProviderSignature {
            source: self.source.raw(),
            value: self.value.0,
        }
    }
    fn from_raw(value: ir::ProviderSignature<'a>) -> Self {
        Self {
            source: Format::from_raw(value.source),
            value: Text(value.value),
        }
    }
}

#[derive(Clone, Copy)]
pub struct BlockView<'s, 'a> {
    records: &'s [ir::Record<'a>],
    value: ir::Block<'a>,
    at: usize,
    ordinal: usize,
}
impl<'s, 'a> BlockView<'s, 'a> {
    pub fn signature(self) -> Presence<ProviderSignature<'a>> {
        Presence::from_raw(self.value.signature).map(ProviderSignature::from_raw)
    }
    pub fn content(self) -> Content<'a> {
        Content::from_raw(self.value.content)
    }
    pub fn ordinal(self) -> usize {
        self.ordinal
    }
    pub fn item_id(self) -> Presence<Text<'a>> {
        text_view(self.value.item_id)
    }
    pub fn item_status(self) -> Presence<Text<'a>> {
        text_view(self.value.item_status)
    }
    pub fn extensions(self) -> Extensions<'s, 'a> {
        Extensions {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.value.extensions.start,
            owner: Some(self.at),
        }
    }
}

pub struct Tools<'s, 'a> {
    records: &'s [ir::Record<'a>],
    at: usize,
}
impl<'a> Iterator for Tools<'_, 'a> {
    type Item = ToolDefinition<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(record) = self.records.get(self.at) {
            self.at += 1;
            if let ir::Record::Tool(value) = record {
                return Some(ToolDefinition::from_raw(*value));
            }
        }
        None
    }
}

/// Complete-response counters use inclusive input/output semantics. Missing and
/// null counters remain distinct. The converter owns cache/thought arithmetic.
#[derive(Clone, Copy, Debug, Default)]
pub struct ResponseUsage<'a> {
    pub input_tokens: Presence<Integer<'a>>,
    pub output_tokens: Presence<Integer<'a>>,
    pub total_tokens: Presence<Integer<'a>>,
    pub cached_tokens: Presence<Integer<'a>>,
    pub cache_write_tokens: Presence<Integer<'a>>,
    pub reasoning_tokens: Presence<Integer<'a>>,
}
impl<'a> ResponseUsage<'a> {
    fn from_raw(value: ir::Usage<'a>) -> Presence<Self> {
        if !value.present {
            return Presence::Missing;
        }
        if value.is_null {
            return Presence::Null;
        }
        Presence::Value(Self {
            input_tokens: Presence::from_raw(value.input_tokens).map(Integer),
            output_tokens: Presence::from_raw(value.output_tokens).map(Integer),
            total_tokens: Presence::from_raw(value.total_tokens).map(Integer),
            cached_tokens: Presence::from_raw(value.cached_tokens).map(Integer),
            cache_write_tokens: Presence::from_raw(value.cache_write_tokens).map(Integer),
            reasoning_tokens: Presence::from_raw(value.reasoning_tokens).map(Integer),
        })
    }
    fn raw(value: Presence<Self>) -> ir::Usage<'a> {
        match value {
            Presence::Missing => ir::Usage::default(),
            Presence::Null => ir::Usage {
                present: true,
                is_null: true,
                ..ir::Usage::default()
            },
            Presence::Value(v) => ir::Usage {
                input_tokens: v.input_tokens.map(|v| v.0).into_raw(),
                output_tokens: v.output_tokens.map(|v| v.0).into_raw(),
                total_tokens: v.total_tokens.map(|v| v.0).into_raw(),
                cached_tokens: v.cached_tokens.map(|v| v.0).into_raw(),
                cache_write_tokens: v.cache_write_tokens.map(|v| v.0).into_raw(),
                reasoning_tokens: v.reasoning_tokens.map(|v| v.0).into_raw(),
                present: true,
                is_null: false,
                extensions: ir::Span::EMPTY,
            },
        }
    }
}

#[derive(Clone, Copy)]
struct ResponseHead<'a> {
    source: zconv::Format,
    id: ir::Presence<ir::Text<'a>>,
    model: ir::Presence<ir::Text<'a>>,
    created: ir::Presence<ir::Number<'a>>,
    created_iso: ir::Presence<ir::Text<'a>>,
    updated_iso: ir::Presence<ir::Text<'a>>,
    status: ir::Finish,
    usage: ir::Usage<'a>,
    error: ir::Presence<zconv::json::RawValue<'a>>,
    stop_sequence: ir::Presence<ir::Text<'a>>,
    incomplete_details: ir::Presence<zconv::json::RawValue<'a>>,
    extensions: ir::Span,
}
impl<'a> ResponseHead<'a> {
    fn new(source: Format) -> Self {
        Self {
            source: source.raw(),
            id: ir::Presence::Missing,
            model: ir::Presence::Missing,
            created: ir::Presence::Missing,
            created_iso: ir::Presence::Missing,
            updated_iso: ir::Presence::Missing,
            status: ir::Finish::Missing,
            usage: ir::Usage::default(),
            error: ir::Presence::Missing,
            stop_sequence: ir::Presence::Missing,
            incomplete_details: ir::Presence::Missing,
            extensions: ir::Span::EMPTY,
        }
    }
    fn take(raw: ir::Response<'a, '_>) -> (Self, usize) {
        (
            Self {
                source: raw.source,
                id: raw.id,
                model: raw.model,
                created: raw.created,
                created_iso: raw.created_iso,
                updated_iso: raw.updated_iso,
                status: raw.status,
                usage: raw.usage,
                error: raw.error,
                stop_sequence: raw.stop_sequence,
                incomplete_details: raw.incomplete_details,
                extensions: raw.extensions,
            },
            raw.records.len(),
        )
    }
    fn view<'s>(self, records: &'s mut [ir::Record<'a>]) -> ir::Response<'a, 's> {
        ir::Response {
            source: self.source,
            id: self.id,
            model: self.model,
            created: self.created,
            created_iso: self.created_iso,
            updated_iso: self.updated_iso,
            status: self.status,
            usage: self.usage,
            error: self.error,
            stop_sequence: self.stop_sequence,
            incomplete_details: self.incomplete_details,
            extensions: self.extensions,
            records,
        }
    }
}

/// A response contains ordered candidates, independently indexed from messages
/// and tool calls. No response is silently flattened into the first candidate.
pub struct Response<'a, 's> {
    head: ResponseHead<'a>,
    records: &'s mut [ir::Record<'a>],
    used: usize,
}
impl<'a, 's> Response<'a, 's> {
    pub fn decode<const T: usize, const R: usize>(
        source: Format,
        input: &'a [u8],
        tokens: &mut TokenStorage<T>,
        storage: &'s mut RecordStorage<'a, R>,
        limits: Limits,
    ) -> Result<Self, Error> {
        let document =
            zconv::json::parse(input, &mut tokens.inner, limits.raw()).map_err(Error::from)?;
        let (head, used) = ResponseHead::take(
            zconv::protocol::decode_response(source.raw(), document.root(), &mut storage.inner)
                .map_err(Error::from)?,
        );
        Ok(Self {
            head,
            records: &mut storage.inner,
            used,
        })
    }
    pub fn source(&self) -> Format {
        Format::from_raw(self.head.source)
    }
    pub fn id(&self) -> Presence<Text<'a>> {
        text_view(self.head.id)
    }
    pub fn set_id(&mut self, id: Presence<Text<'a>>) {
        self.head.id = text_raw(id);
    }
    pub fn model(&self) -> Presence<Text<'a>> {
        text_view(self.head.model)
    }
    pub fn set_model(&mut self, model: Presence<Text<'a>>) {
        self.head.model = text_raw(model);
    }
    pub fn created(&self) -> Presence<Integer<'a>> {
        Presence::from_raw(self.head.created).map(Integer)
    }
    pub fn set_created(&mut self, value: Presence<Integer<'a>>) {
        self.head.created = value.map(|v| v.0).into_raw();
    }
    pub fn created_iso(&self) -> Presence<Text<'a>> {
        text_view(self.head.created_iso)
    }
    pub fn set_created_iso(&mut self, value: Presence<Text<'a>>) {
        self.head.created_iso = text_raw(value);
    }
    pub fn updated_iso(&self) -> Presence<Text<'a>> {
        text_view(self.head.updated_iso)
    }
    pub fn set_updated_iso(&mut self, value: Presence<Text<'a>>) {
        self.head.updated_iso = text_raw(value);
    }
    pub fn status(&self) -> Finish {
        Finish::from_raw(self.head.status)
    }
    /// Explicitly replace the envelope and every candidate's finish state.
    /// Old wire spellings and incomplete-details snapshots are cleared. Use
    /// `set_choice_finish` to update a single candidate instead.
    pub fn set_status(&mut self, status: Finish) {
        self.head.status = status.raw();
        self.head.incomplete_details = ir::Presence::Missing;
        for record in &mut self.records[..self.used] {
            if let ir::Record::Choice(choice) = record {
                choice.finish = status.raw();
                choice.raw_finish = ir::Presence::Missing;
            }
        }
    }
    pub fn error(&self) -> Presence<Json<'a>> {
        json_view(self.head.error)
    }
    pub fn set_error(&mut self, value: Presence<Json<'a>>) {
        self.head.error = json_raw(value);
    }
    pub fn incomplete_details(&self) -> Presence<Json<'a>> {
        json_view(self.head.incomplete_details)
    }
    pub fn stop_sequence(&self) -> Presence<Text<'a>> {
        text_view(self.head.stop_sequence)
    }
    pub fn usage(&self) -> Presence<ResponseUsage<'a>> {
        ResponseUsage::from_raw(self.head.usage)
    }
    pub fn set_usage(&mut self, usage: Presence<ResponseUsage<'a>>) {
        let extensions = self.head.usage.extensions;
        self.head.usage = ResponseUsage::raw(usage);
        self.head.usage.extensions = extensions;
    }
    pub fn choices(&self) -> Choices<'_, 'a> {
        Choices {
            records: &self.records[..self.used],
            at: 0,
        }
    }
    pub fn extensions(&self) -> Extensions<'_, 'a> {
        Extensions {
            records: &self.records[..self.used],
            at: 0,
            owner: None,
        }
    }
    pub fn records_used(&self) -> usize {
        self.used
    }
    pub fn record_capacity(&self) -> usize {
        self.records.len()
    }
    pub fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>() + core::mem::size_of_val(self.records)
    }
    pub fn append_choice(
        &mut self,
        index: u64,
        finish: Finish,
        contents: &[Content<'a>],
    ) -> Result<(), Error> {
        if self.choices().any(|c| c.index() == index) {
            return Err(Error::invalid("choice index", "duplicate candidate index"));
        }
        let needed = self
            .used
            .checked_add(2)
            .and_then(|v| v.checked_add(contents.len()))
            .ok_or_else(|| Error::capacity(Resource::Ir, usize::MAX, false, Replay::Unconsumed))?;
        if needed > self.records.len() {
            return Err(Error::capacity(
                Resource::Ir,
                needed,
                true,
                Replay::Unconsumed,
            ));
        }
        let at = self.used;
        let mut end = at + 1;
        append_message(
            self.records,
            &mut end,
            MessageOptions::new(Role::Assistant),
            contents,
        )?;
        self.records[at] = ir::Record::Choice(ir::Choice {
            index,
            finish: finish.raw(),
            raw_finish: ir::Presence::Missing,
            end,
            extensions: ir::Span::EMPTY,
        });
        self.used = end;
        Ok(())
    }
    pub fn set_choice_finish(&mut self, index: u64, finish: Finish) -> Result<(), Error> {
        let single = self.choices().count() == 1;
        for record in &mut self.records[..self.used] {
            if let ir::Record::Choice(choice) = record
                && choice.index == index
            {
                choice.finish = finish.raw();
                choice.raw_finish = ir::Presence::Missing;
                if single {
                    self.head.status = finish.raw();
                    self.head.incomplete_details = ir::Presence::Missing;
                }
                return Ok(());
            }
        }
        Err(Error::invalid("choice index", "unknown candidate index"))
    }
    pub fn replace_content(
        &mut self,
        choice: u64,
        block: usize,
        content: Content<'a>,
    ) -> Result<(), Error> {
        let choice = self
            .choices()
            .find(|c| c.index() == choice)
            .ok_or_else(|| Error::invalid("choice index", "unknown candidate index"))?;
        let index = choice
            .contents()
            .nth(block)
            .map(|b| b.at)
            .ok_or_else(|| Error::invalid("content", "content ordinal is out of range"))?;
        validate_content(Role::Assistant, content)?;
        if let ir::Record::Block(ref mut slot) = self.records[index] {
            slot.content = content.raw();
        }
        let parent = self.records[..index]
            .iter()
            .enumerate()
            .rev()
            .find_map(|(at, record)| match record {
                ir::Record::Message(m) if m.end > index => Some(at),
                _ => None,
            });
        if let Some(parent) = parent {
            refresh_message(&mut self.records[..self.used], parent);
        }
        Ok(())
    }
    pub fn set_block_signature(
        &mut self,
        choice: u64,
        block: usize,
        signature: Presence<ProviderSignature<'a>>,
    ) -> Result<(), Error> {
        let index = self
            .choices()
            .find(|c| c.index() == choice)
            .ok_or_else(|| Error::invalid("choice index", "unknown candidate index"))?
            .contents()
            .nth(block)
            .map(|b| b.at)
            .ok_or_else(|| Error::invalid("content", "content ordinal is out of range"))?;
        if let ir::Record::Block(ref mut slot) = self.records[index] {
            slot.signature = signature.map(ProviderSignature::raw).into_raw();
        }
        Ok(())
    }
    pub fn set_item_metadata(
        &mut self,
        choice: u64,
        block: usize,
        id: Presence<Text<'a>>,
        status: Presence<Text<'a>>,
    ) -> Result<(), Error> {
        let choice = self
            .choices()
            .find(|c| c.index() == choice)
            .ok_or_else(|| Error::invalid("choice index", "unknown candidate index"))?;
        let index = choice
            .contents()
            .nth(block)
            .map(|b| b.at)
            .ok_or_else(|| Error::invalid("content", "content ordinal is out of range"))?;
        if let ir::Record::Block(ref mut slot) = self.records[index] {
            slot.item_id = text_raw(id);
            slot.item_status = text_raw(status);
        }
        Ok(())
    }
    pub fn encode<const T: usize>(
        &mut self,
        target: Format,
        output: &mut [u8],
        scratch: &mut [u8],
        tokens: &mut TokenStorage<T>,
        context: EncodeContext<'a>,
        policy: Policy,
    ) -> Result<EncodeReport<'a>, Error> {
        if matches!(
            self.head.status,
            ir::Finish::Cancelled | ir::Finish::Error | ir::Finish::InProgress
        ) && !matches!(
            target.protocol,
            Protocol::OpenAiResponses | Protocol::GeminiInteractions
        ) {
            return Err(Error::unsupported(
                "response lifecycle in target success envelope",
            ));
        }
        for choice in self.choices() {
            let refused = choice
                .contents()
                .any(|b| matches!(b.content(), Content::Refusal(_)));
            let plain = choice
                .contents()
                .any(|b| matches!(b.content(), Content::Text(_)));
            if target.protocol == Protocol::AnthropicMessages
                && ((refused && choice.finish() != Finish::Refusal)
                    || (plain && choice.finish() == Finish::Refusal))
            {
                return Err(Error::unsupported(
                    "refusal block requires an unambiguous Anthropic refusal finish",
                ));
            }
            if matches!(
                target.protocol,
                Protocol::OpenAiChat | Protocol::OpenAiCompletions
            ) && choice.finish() == Finish::Refusal
                && !refused
            {
                return Err(Error::unsupported(
                    "refusal finish requires explicit target refusal content",
                ));
            }
        }
        validate_tool_arguments(&self.records[..self.used], scratch, &mut tokens.inner)?;
        let raw = self.head.view(&mut self.records[..self.used]);
        zconv::protocol::encode_response(
            &raw,
            target.raw(),
            &context.inner,
            policy.raw(),
            output,
            scratch,
            &mut tokens.inner,
        )
        .map(EncodeReport::from_raw)
        .map_err(encode_error)
    }
    pub fn losses(&self, target: Format, policy: Policy) -> Losses<'_, 'a> {
        let mut metadata = 0;
        if policy.drop_metadata {
            if !matches!(
                target.protocol,
                Protocol::OpenAiChat | Protocol::OpenAiResponses | Protocol::OpenAiCompletions
            ) && !self.head.created.is_missing()
            {
                metadata |= 1;
            }
            if target.protocol != Protocol::GeminiInteractions {
                if !self.head.created_iso.is_missing() {
                    metadata |= 8;
                }
                if !self.head.updated_iso.is_missing() {
                    metadata |= 16;
                }
            }
        }
        Losses::new(&self.records[..self.used], target, policy, None, metadata)
    }
    /// Replace storage in place. Insufficient capacity leaves this response
    /// and all edited fields intact; successful growth copies only used records.
    pub fn replace_storage<const N: usize>(
        &mut self,
        storage: &'s mut RecordStorage<'a, N>,
    ) -> Result<(), Error> {
        if N < self.used {
            return Err(Error::capacity(
                Resource::Ir,
                self.used,
                true,
                Replay::Unconsumed,
            ));
        }
        storage.inner[..self.used].copy_from_slice(&self.records[..self.used]);
        self.records = &mut storage.inner;
        Ok(())
    }
    /// Move into a new bounded region. Failure returns the original edited
    /// response in `RelocationError::original` for a later retry.
    // Returning the owner preserves recoverability without a heap allocation.
    #[allow(clippy::result_large_err)]
    pub fn relocate<'n, const N: usize>(
        self,
        storage: &'n mut RecordStorage<'a, N>,
    ) -> Result<Response<'a, 'n>, RelocationError<Self>> {
        if N < self.used {
            let error = Error::capacity(Resource::Ir, self.used, true, Replay::Unconsumed);
            return Err(RelocationError {
                original: self,
                error,
            });
        }
        storage.inner[..self.used].copy_from_slice(&self.records[..self.used]);
        Ok(Response {
            head: self.head,
            records: &mut storage.inner,
            used: self.used,
        })
    }
}

pub struct ResponseBuilder<'a, 's> {
    response: Response<'a, 's>,
}
impl<'a, 's> ResponseBuilder<'a, 's> {
    pub fn new<const N: usize>(
        authoring: Format,
        storage: &'s mut RecordStorage<'a, N>,
    ) -> Result<Self, Error> {
        authoring.validate()?;
        Ok(Self {
            response: Response {
                head: ResponseHead::new(authoring),
                records: &mut storage.inner,
                used: 0,
            },
        })
    }
    pub fn id(&mut self, value: Presence<Text<'a>>) -> &mut Self {
        self.response.set_id(value);
        self
    }
    pub fn model(&mut self, value: Presence<Text<'a>>) -> &mut Self {
        self.response.set_model(value);
        self
    }
    pub fn created(&mut self, value: Presence<Integer<'a>>) -> &mut Self {
        self.response.set_created(value);
        self
    }
    pub fn created_iso(&mut self, value: Presence<Text<'a>>) -> &mut Self {
        self.response.set_created_iso(value);
        self
    }
    pub fn updated_iso(&mut self, value: Presence<Text<'a>>) -> &mut Self {
        self.response.set_updated_iso(value);
        self
    }
    pub fn status(&mut self, value: Finish) -> &mut Self {
        self.response.set_status(value);
        self
    }
    pub fn usage(&mut self, value: Presence<ResponseUsage<'a>>) -> &mut Self {
        self.response.set_usage(value);
        self
    }
    pub fn error(&mut self, value: Presence<Json<'a>>) -> &mut Self {
        self.response.set_error(value);
        self
    }
    pub fn choice(
        &mut self,
        index: u64,
        finish: Finish,
        contents: &[Content<'a>],
    ) -> Result<&mut Self, Error> {
        self.response.append_choice(index, finish, contents)?;
        Ok(self)
    }
    pub fn finish(self) -> Response<'a, 's> {
        self.response
    }
}

pub struct Choices<'s, 'a> {
    records: &'s [ir::Record<'a>],
    at: usize,
}
impl<'s, 'a> Iterator for Choices<'s, 'a> {
    type Item = ChoiceView<'s, 'a>;
    fn next(&mut self) -> Option<Self::Item> {
        while let Some(record) = self.records.get(self.at) {
            let at = self.at;
            self.at += 1;
            if let ir::Record::Choice(value) = record {
                self.at = value.end.min(self.records.len());
                return Some(ChoiceView {
                    records: self.records,
                    value: *value,
                    at,
                });
            }
        }
        None
    }
}
#[derive(Clone, Copy)]
pub struct ChoiceView<'s, 'a> {
    records: &'s [ir::Record<'a>],
    value: ir::Choice<'a>,
    at: usize,
}
impl<'s, 'a> ChoiceView<'s, 'a> {
    pub fn index(self) -> u64 {
        self.value.index
    }
    pub fn finish(self) -> Finish {
        Finish::from_raw(self.value.finish)
    }
    pub fn raw_finish(self) -> Presence<Text<'a>> {
        text_view(self.value.raw_finish)
    }
    pub fn messages(self) -> Messages<'s, 'a> {
        Messages {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.at.saturating_add(1),
            ordinal: 0,
        }
    }
    pub fn contents(self) -> Blocks<'s, 'a> {
        Blocks {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.at.saturating_add(1),
            ordinal: 0,
        }
    }
    pub fn extensions(self) -> Extensions<'s, 'a> {
        Extensions {
            records: self.records.get(..self.value.end).unwrap_or(&[]),
            at: self.value.extensions.start,
            owner: Some(self.at),
        }
    }
}

/// One exact dropped field or semantic block. Locations use stable semantic
/// ordinals and retain the original extension/context namespace.
#[derive(Clone, Copy, Debug)]
pub enum Loss<'a> {
    Extension(Extension<'a>),
    Reasoning {
        location: Location,
    },
    Context {
        source: Format,
    },
    Metadata {
        field: &'static str,
        location: Location,
    },
}
/// Borrowed loss enumeration. A successful encode report counts the same losses;
/// rejected conversions return their error and produce no committed output.
pub struct Losses<'s, 'a> {
    records: &'s [ir::Record<'a>],
    target: Format,
    policy: Policy,
    at: usize,
    context: Option<Format>,
    metadata: u32,
    metadata_owner: usize,
    reasoning: Option<usize>,
}
impl<'s, 'a> Losses<'s, 'a> {
    fn new(
        records: &'s [ir::Record<'a>],
        target: Format,
        policy: Policy,
        context: Option<Format>,
        metadata: u32,
    ) -> Self {
        Self {
            records,
            target,
            policy,
            at: 0,
            context,
            metadata,
            metadata_owner: usize::MAX,
            reasoning: None,
        }
    }
}
impl<'a> Iterator for Losses<'_, 'a> {
    type Item = Loss<'a>;
    fn next(&mut self) -> Option<Self::Item> {
        if let Some(source) = self.context.take() {
            return Some(Loss::Context { source });
        }
        loop {
            if self.metadata != 0 {
                let bit = 1 << self.metadata.trailing_zeros();
                self.metadata &= !bit;
                let field = match bit {
                    1 => "created",
                    2 => "item.id",
                    4 => "item.status",
                    8 => "created (ISO 8601)",
                    _ => "updated (ISO 8601)",
                };
                return Some(Loss::Metadata {
                    field,
                    location: location_for(self.records, self.metadata_owner),
                });
            }
            if let Some(index) = self.reasoning.take() {
                return Some(Loss::Reasoning {
                    location: location_for(self.records, index),
                });
            }
            let record = self.records.get(self.at)?;
            let index = self.at;
            self.at += 1;
            let (id, status) = match record {
                ir::Record::Block(block) => {
                    if matches!(block.content, ir::Content::Reasoning { .. })
                        && self.policy.drop_reasoning
                    {
                        self.reasoning = Some(index);
                    }
                    (block.item_id, block.item_status)
                }
                ir::Record::Message(message) => (message.item_id, message.item_status),
                ir::Record::Extension(v)
                    if v.source != self.target.raw()
                        && self.policy.drop_extensions
                        && !v.is_empty_annotation() =>
                {
                    return Some(Loss::Extension(Extension {
                        source: Format::from_raw(v.source),
                        name: Text(ir::Text::Json(v.key)),
                        value: Json(v.value),
                        scope: v.location,
                        location: location_for(self.records, v.owner),
                    }));
                }
                _ => (ir::Presence::Missing, ir::Presence::Missing),
            };
            if self.target.protocol != Protocol::OpenAiResponses && self.policy.drop_metadata {
                self.metadata_owner = index;
                if !id.is_missing() {
                    self.metadata |= 2;
                }
                if !status.is_missing() {
                    self.metadata |= 4;
                }
            }
        }
    }
}
fn location_for(records: &[ir::Record<'_>], index: usize) -> Location {
    if index >= records.len() {
        return Location::default();
    }
    let mut result = Location::default();
    let mut message = 0;
    let mut content = 0;
    let mut tool = 0;
    let mut choice_end = records.len();
    let mut message_end = records.len();
    for (at, record) in records.iter().enumerate().take(index + 1) {
        if at >= choice_end {
            result.choice = None;
            result.message = None;
            result.content = None;
        }
        if at >= message_end {
            result.message = None;
            result.content = None;
        }
        match record {
            ir::Record::Choice(v) => {
                result.choice = Some(v.index);
                choice_end = v.end;
                result.message = None;
                result.content = None;
                message = 0;
                content = 0;
            }
            ir::Record::Message(v) => {
                result.message = Some(message);
                message += 1;
                message_end = v.end;
                result.content = None;
                content = 0;
            }
            ir::Record::Block(_) => {
                result.content = Some(content);
                content += 1;
            }
            ir::Record::Tool(_) => {
                result.tool = Some(tool);
                tool += 1;
                result.message = None;
                result.content = None;
            }
            _ => {}
        }
    }
    result
}

/// Recoverable storage relocation failure. The original edited document is
/// returned intact so the caller can select a larger region and retry.
pub struct RelocationError<T> {
    pub original: T,
    pub error: Error,
}
impl<T> core::fmt::Debug for RelocationError<T> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("RelocationError")
            .field("error", &self.error)
            .finish_non_exhaustive()
    }
}

fn validate_content(role: Role, content: Content<'_>) -> Result<(), Error> {
    match content {
        Content::ToolCall {
            name, arguments, ..
        } => {
            if role != Role::Assistant {
                return Err(Error::invalid(
                    "tool call",
                    "tool calls require the assistant role",
                ));
            }
            if name.decoded_len() == 0 {
                return Err(Error::invalid("tool name", "empty tool name"));
            }
            match arguments {
                Data::Json(value) if value.kind() != JsonKind::Object => {
                    return Err(Error::invalid("tool arguments", "expected a JSON object"));
                }
                Data::Text(_) => {
                    return Err(Error::invalid(
                        "tool arguments",
                        "ordinary text is not JSON arguments",
                    ));
                }
                _ => {}
            }
        }
        Content::ToolResult { .. } if !matches!(role, Role::User | Role::Tool) => {
            return Err(Error::invalid(
                "tool result",
                "tool results require a user or tool role",
            ));
        }
        Content::Reasoning { .. } | Content::Refusal(_) if role != Role::Assistant => {
            return Err(Error::invalid(
                "assistant content",
                "reasoning and refusal require the assistant role",
            ));
        }
        _ => {}
    }
    Ok(())
}
fn refresh_message(records: &mut [ir::Record<'_>], at: usize) {
    let Some(ir::Record::Message(mut message)) = records.get(at).copied() else {
        return;
    };
    let Some(contents) = records.get(at + 1..message.end) else {
        return;
    };
    let mut tool = false;
    let mut legacy = false;
    let mut refusal = false;
    let mut reasoning = false;
    let mut only_text = true;
    for record in contents {
        if let ir::Record::Block(block) = record {
            match block.content {
                ir::Content::ToolCall { legacy: false, .. } => tool = true,
                ir::Content::ToolCall { legacy: true, .. } => legacy = true,
                ir::Content::Refusal(_) => refusal = true,
                ir::Content::Reasoning { .. } => reasoning = true,
                _ => {}
            }
            only_text &= matches!(block.content, ir::Content::Text(_));
        }
    }
    fn marker(old: ir::Presence<()>, present: bool) -> ir::Presence<()> {
        if present {
            ir::Presence::Value(())
        } else if matches!(old, ir::Presence::Value(())) {
            ir::Presence::Missing
        } else {
            old
        }
    }
    message.tool_calls = marker(message.tool_calls, tool);
    message.function_call = marker(message.function_call, legacy);
    message.refusal = marker(message.refusal, refusal);
    message.reasoning = marker(message.reasoning, reasoning);
    if !only_text && message.shape == ir::ContentShape::String {
        message.shape = ir::ContentShape::Array;
    }
    records[at] = ir::Record::Message(message);
}
fn validate_tool_arguments(
    records: &[ir::Record<'_>],
    scratch: &mut [u8],
    tokens: &mut [zconv::json::Token],
) -> Result<(), Error> {
    for record in records {
        let ir::Record::Block(ir::Block {
            content: ir::Content::ToolCall { arguments, .. },
            ..
        }) = record
        else {
            continue;
        };
        match *arguments {
            ir::Data::EmptyObject => {}
            ir::Data::Json(value) if value.kind() == zconv::json::Kind::Object => {}
            ir::Data::JsonText(value) => {
                let n = Text(value)
                    .decode_into(scratch)
                    .map_err(|e| e.with_resource(Resource::Scratch))?;
                let document =
                    zconv::json::parse(&scratch[..n], tokens, zconv::json::Limits::default())
                        .map_err(|e| {
                            let mut error = Error::from(e);
                            error.field = "tool arguments";
                            error
                        })?;
                if document.root().kind() != zconv::json::Kind::Object {
                    return Err(Error::invalid(
                        "tool arguments",
                        "expected a complete JSON object",
                    ));
                }
            }
            _ => {
                return Err(Error::invalid(
                    "tool arguments",
                    "expected a complete JSON object",
                ));
            }
        }
    }
    Ok(())
}

fn shift_span(span: &mut ir::Span, at: usize) {
    if span.start == span.end {
        return;
    }
    if span.start >= at {
        span.start += 1;
    }
    if span.end > at {
        span.end += 1;
    }
}
fn insert_record<'a>(
    records: &mut [ir::Record<'a>],
    used: &mut usize,
    at: usize,
    new: ir::Record<'a>,
) -> Result<(), Error> {
    let needed = used
        .checked_add(1)
        .ok_or_else(|| Error::capacity(Resource::Ir, usize::MAX, false, Replay::Unconsumed))?;
    if needed > records.len() {
        return Err(Error::capacity(
            Resource::Ir,
            needed,
            true,
            Replay::Unconsumed,
        ));
    }
    if at > *used {
        return Err(Error::invalid("extension", "invalid insertion point"));
    }
    records.copy_within(at..*used, at + 1);
    records[at] = new;
    for (index, record) in records[..needed].iter_mut().enumerate() {
        if index == at {
            continue;
        }
        match record {
            ir::Record::Message(v) => {
                if v.end > at {
                    v.end += 1;
                }
                shift_span(&mut v.extensions, at);
            }
            ir::Record::Block(v) => {
                if v.end > at {
                    v.end += 1;
                }
                shift_span(&mut v.extensions, at);
            }
            ir::Record::Choice(v) => {
                if v.end > at {
                    v.end += 1;
                }
                shift_span(&mut v.extensions, at);
            }
            ir::Record::Tool(v) => shift_span(&mut v.extensions, at),
            ir::Record::Extension(v) => {
                if v.owner >= at && v.owner != usize::MAX {
                    v.owner += 1;
                }
            }
            ir::Record::Empty => {}
        }
    }
    *used = needed;
    Ok(())
}
