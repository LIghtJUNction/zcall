//! Incremental model turns. Transport and tool execution remain caller actions.

use crate::common::{Error, ErrorKind, Format, Limits, Protocol, Replay, Resource, Text};
use zconv::{json, sse, stream as wire};

mod encoder;
pub use encoder::*;

const NONE: usize = usize::MAX;
const WORD: usize = core::mem::size_of::<usize>();
const HEADER: usize = WORD * 2;

/// A choice, output item and content part. No identifier is a provider call ID.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventKey {
    pub choice: u32,
    pub item: u32,
    pub part: u32,
}
impl EventKey {
    const fn from_raw(value: wire::Key) -> Self {
        Self {
            choice: value.choice,
            item: value.index,
            part: value.part,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum BlockType {
    #[default]
    Text,
    Tool,
    Reasoning,
    Refusal,
}
impl BlockType {
    fn from_raw(value: wire::BlockKind) -> Self {
        match value {
            wire::BlockKind::Text => Self::Text,
            wire::BlockKind::Tool => Self::Tool,
            wire::BlockKind::Reasoning => Self::Reasoning,
            wire::BlockKind::Refusal => Self::Refusal,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub enum StopReason {
    #[default]
    Stop,
    Length,
    ToolCalls,
    Safety,
    Other,
}

/// Reasoning text semantics verified by the source adapter. Vendor-compatible
/// fields whose contract does not say thought or summary remain unspecified.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamReasoningKind {
    Thought,
    Summary,
    Unspecified,
}

pub(crate) fn reasoning_semantics(format: Format) -> StreamReasoningKind {
    match format.protocol {
        Protocol::AnthropicMessages | Protocol::GeminiGenerateContent => {
            StreamReasoningKind::Thought
        }
        Protocol::OpenAiResponses | Protocol::GeminiInteractions => StreamReasoningKind::Summary,
        Protocol::OpenAiChat | Protocol::OpenAiCompletions => StreamReasoningKind::Unspecified,
    }
}

/// The latest real envelope identity. Late source metadata does not start a new turn.
#[derive(Clone, Copy, Debug)]
pub struct CallIdentity<'a> {
    pub response_id: Option<Text<'a>>,
    pub model: Option<Text<'a>>,
    pub created: Option<u64>,
}
impl StopReason {
    fn from_raw(value: wire::FinishReason) -> Self {
        match value {
            wire::FinishReason::Stop => Self::Stop,
            wire::FinishReason::Length => Self::Length,
            wire::FinishReason::ToolCalls => Self::ToolCalls,
            wire::FinishReason::Safety => Self::Safety,
            wire::FinishReason::Other => Self::Other,
        }
    }
}

/// Cumulative inclusive token counts. Missing counters remain `None`.
/// Cache reads/writes are included in `input`; thoughts are included in `output`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamUsage {
    pub input: Option<u64>,
    pub output: Option<u64>,
    pub total: Option<u64>,
    pub cached: Option<u64>,
    pub cache_write: Option<u64>,
    pub reasoning: Option<u64>,
}
impl StreamUsage {
    fn from_raw(value: wire::Usage) -> Self {
        Self {
            input: value.input,
            output: value.output,
            total: value.total,
            cached: value.cached,
            cache_write: value.cache_write,
            reasoning: value.reasoning,
        }
    }
    pub(crate) fn raw(self) -> wire::Usage {
        wire::Usage {
            input: self.input,
            output: self.output,
            total: self.total,
            cached: self.cached,
            cache_write: self.cache_write,
            reasoning: self.reasoning,
            ..wire::Usage::default()
        }
    }
}

/// Call state refers to a single model turn, never completion of an agent task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CallState {
    Idle,
    Prepared,
    Receiving,
    ToolForming,
    ToolsReady,
    WaitingTools,
    ToolsResolved,
    Completed,
    Incomplete,
    UpstreamFailed,
    CancelRequested,
    Cancelled,
    Truncated,
    Failed,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum TurnOutcome {
    Completed,
    WaitingTools,
    Incomplete,
    Cancelled,
    UpstreamFailed,
    LocalFailure,
}

/// A terminal event is delivered once. `WaitingTools` still needs harness work.
#[derive(Clone, Copy, Debug)]
pub struct Terminal<'a> {
    pub outcome: TurnOutcome,
    pub usage: StreamUsage,
    pub error: Option<Error>,
    pub upstream_code: Option<Text<'a>>,
    pub upstream_message: Option<Text<'a>>,
}

/// The six loss classes currently identified by the underlying stream adapters.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StreamLoss {
    SystemFingerprint,
    ServiceTier,
    SafetyRatings,
    FinishMessage,
    AverageLogprobs,
    CacheCreationTokens,
}
impl StreamLoss {
    fn from_raw(value: wire::DroppedField) -> Self {
        match value {
            wire::DroppedField::SystemFingerprint => Self::SystemFingerprint,
            wire::DroppedField::ServiceTier => Self::ServiceTier,
            wire::DroppedField::SafetyRatings => Self::SafetyRatings,
            wire::DroppedField::FinishMessage => Self::FinishMessage,
            wire::DroppedField::AverageLogprobs => Self::AverageLogprobs,
            wire::DroppedField::CacheCreationTokens => Self::CacheCreationTokens,
        }
    }
    pub const fn field(self) -> &'static str {
        match self {
            Self::SystemFingerprint => "system_fingerprint",
            Self::ServiceTier => "service_tier",
            Self::SafetyRatings => "safetyRatings",
            Self::FinishMessage => "finishMessage",
            Self::AverageLogprobs => "avgLogprobs",
            Self::CacheCreationTokens => "cache_creation_input_tokens",
        }
    }
}

/// A validated complete JSON object, borrowed from caller aggregation storage.
#[derive(Clone, Copy, Debug)]
pub struct ToolArguments<'a> {
    bytes: &'a [u8],
}
impl<'a> ToolArguments<'a> {
    pub fn as_bytes(self) -> &'a [u8] {
        self.bytes
    }
    pub fn as_str(self) -> Result<&'a str, Error> {
        core::str::from_utf8(self.bytes)
            .map_err(|_| Error::invalid("tool arguments", "invalid UTF-8"))
    }
}

/// A ready tool has a real or caller-supplied call ID and valid object arguments.
#[derive(Clone, Copy, Debug)]
pub struct ReadyTool<'a> {
    pub key: EventKey,
    pub call_id: &'a str,
    pub name: &'a str,
    pub item_id: Option<&'a str>,
    pub arguments: ToolArguments<'a>,
}

/// Protocol-independent observations. Snapshots are explicitly distinct from deltas.
#[derive(Clone, Copy, Debug)]
pub enum EventData<'a> {
    Started {
        response_id: Option<Text<'a>>,
        model: Option<Text<'a>>,
        created: Option<u64>,
    },
    IdentityUpdated(CallIdentity<'a>),
    BlockStarted {
        key: EventKey,
        kind: BlockType,
        call_id: Option<Text<'a>>,
        name: Option<Text<'a>>,
        item_id: Option<Text<'a>>,
    },
    Delta {
        key: EventKey,
        kind: BlockType,
        text: Text<'a>,
    },
    Signature {
        key: EventKey,
        signature: Text<'a>,
    },
    BlockEnded {
        key: EventKey,
    },
    ToolReady(ReadyTool<'a>),
    Usage(StreamUsage),
    ChoiceEnded {
        choice: u32,
        reason: StopReason,
    },
    SnapshotVerified {
        key: EventKey,
        kind: BlockType,
        text: Text<'a>,
        exact: bool,
    },
    BlockIncomplete {
        key: EventKey,
    },
    Loss {
        source: Format,
        field: StreamLoss,
    },
    Terminal(Terminal<'a>),
}

/// Data borrows the session only for the consumer call. Copy data that outlives it.
/// The private replay token lets `EventEncoder` use zconv without another parser.
#[derive(Clone, Copy, Debug)]
pub struct Event<'a> {
    source: Format,
    data: EventData<'a>,
    pub(crate) replay: Option<(&'a wire::Event, &'a [u8])>,
}
impl<'a> Event<'a> {
    pub fn source(self) -> Format {
        self.source
    }
    pub fn data(self) -> EventData<'a> {
        self.data
    }
    pub fn reasoning_kind(self) -> Option<StreamReasoningKind> {
        match self.data {
            EventData::BlockStarted {
                kind: BlockType::Reasoning,
                ..
            }
            | EventData::Delta {
                kind: BlockType::Reasoning,
                ..
            }
            | EventData::SnapshotVerified {
                kind: BlockType::Reasoning,
                ..
            }
            | EventData::Signature { .. } => Some(reasoning_semantics(self.source)),
            _ => None,
        }
    }
    /// Replace a delta before encoding; this does not edit the source session.
    pub fn with_delta(self, text: &'a str) -> Result<Self, Error> {
        let EventData::Delta { key, kind, .. } = self.data else {
            return Err(Error::lifecycle(
                "event edit",
                "only a delta can replace its text",
            ));
        };
        if self.replay.is_none() {
            return Err(Error::lifecycle(
                "event edit",
                "event has no encoded representation",
            ));
        }
        Ok(Self {
            data: EventData::Delta {
                key,
                kind,
                text: Text::new(text),
            },
            ..self
        })
    }
}

/// `Pause` rejects this event. Side effects belong only to the `Continue` path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Control {
    Continue,
    Pause,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StepStatus {
    NeedInput,
    NeedEvents,
    NeedBuffer(crate::common::Capacity),
    Done,
    Failed(Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SessionStep {
    pub consumed: usize,
    pub emitted: usize,
    pub status: StepStatus,
}

/// Missing IDs may be provided by the caller; zcall never invents them.
/// Keys address source positions. Repeated keys for Gemini complete tools use
/// distinct supplied IDs in appearance order, skipping IDs already in this turn.
#[derive(Clone, Copy, Debug)]
pub struct ToolIdentity<'a> {
    pub key: EventKey,
    pub call_id: &'a str,
}

#[derive(Clone, Copy, Debug)]
pub struct SessionConfig<'a> {
    pub source: Format,
    pub limits: Limits,
    pub tool_ids: &'a [ToolIdentity<'a>],
    pub allowed_losses: &'a [StreamLoss],
    /// Keep Responses text and refusal bytes for exact final snapshot validation.
    /// Disabling this is an explicit length-only validation contract for text.
    /// Tool arguments are always retained and compared byte for byte.
    pub validate_text_snapshots: bool,
}
impl SessionConfig<'_> {
    pub fn new(source: Format) -> Self {
        Self {
            source,
            limits: Limits::default(),
            tool_ids: &[],
            allowed_losses: &[],
            validate_text_snapshots: true,
        }
    }
}

#[derive(Clone, Copy, Debug, Default)]
struct Span {
    start: usize,
    len: usize,
    present: bool,
}
impl Span {
    const EMPTY: Self = Self {
        start: 0,
        len: 0,
        present: false,
    };
}

#[derive(Clone, Copy, Debug)]
struct Item {
    key: EventKey,
    kind: BlockType,
    id: Span,
    name: Span,
    item_id: Span,
    head: usize,
    tail: usize,
    len: usize,
    complete: Span,
    closed: bool,
    finished: bool,
    incomplete: bool,
    ready_pending: bool,
    ready_emitted: bool,
    result_submitted: bool,
    reason: StopReason,
}
impl Item {
    const EMPTY: Self = Self {
        key: EventKey {
            choice: 0,
            item: 0,
            part: 0,
        },
        kind: BlockType::Text,
        id: Span::EMPTY,
        name: Span::EMPTY,
        item_id: Span::EMPTY,
        head: NONE,
        tail: NONE,
        len: 0,
        complete: Span::EMPTY,
        closed: false,
        finished: false,
        incomplete: false,
        ready_pending: false,
        ready_emitted: false,
        result_submitted: false,
        reason: StopReason::Stop,
    };
}

/// All proportional state is caller sized. Two item tapes enable atomic recovery.
/// Use `memory_bytes()` to account for token/event/item storage before allocation.
pub struct StreamWorkspace<const TOKENS: usize, const EVENTS: usize, const ITEMS: usize> {
    tokens: [json::Token; TOKENS],
    events: [wire::Event; EVENTS],
    items: [Item; ITEMS],
    checkpoints: [Item; ITEMS],
}
impl<const T: usize, const E: usize, const I: usize> StreamWorkspace<T, E, I> {
    pub const fn new() -> Self {
        Self {
            tokens: [json::Token::EMPTY; T],
            events: [wire::Event::EMPTY; E],
            items: [Item::EMPTY; I],
            checkpoints: [Item::EMPTY; I],
        }
    }
    pub const fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
    pub const fn capacities(&self) -> (usize, usize, usize) {
        (T, E, I)
    }
}
impl<const T: usize, const E: usize, const I: usize> Default for StreamWorkspace<T, E, I> {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Clone, Copy, Debug)]
struct State {
    phase: CallState,
    begun: bool,
    id: Span,
    model: Span,
    created: Option<u64>,
    usage: wire::UsageAccumulator,
    aggregate_used: usize,
    aggregate_peak: usize,
    item_count: usize,
    seen_choice_end: bool,
    outcome: Option<TurnOutcome>,
    loss_count: usize,
}
impl State {
    fn new() -> Self {
        Self {
            phase: CallState::Idle,
            begun: false,
            id: Span::EMPTY,
            model: Span::EMPTY,
            created: None,
            usage: wire::UsageAccumulator::default(),
            aggregate_used: 0,
            aggregate_peak: 0,
            item_count: 0,
            seen_choice_end: false,
            outcome: None,
            loss_count: 0,
        }
    }
}

/// Byte and proportional state accounting, excluding caller output/consumer storage.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct StreamMemory {
    pub state_bytes: usize,
    pub framing_bytes: usize,
    pub payload_bytes: usize,
    pub aggregate_bytes: usize,
    pub token_bytes: usize,
    pub event_bytes: usize,
    pub item_bytes: usize,
    pub checkpoint_bytes: usize,
    pub aggregate_used: usize,
    pub aggregate_peak: usize,
    pub items_used: usize,
}

/// One model turn. `step` never performs transport or executes a tool.
pub struct Session<'a, 'c> {
    config: SessionConfig<'c>,
    framer: sse::Decoder<'a>,
    payload: &'a mut [u8],
    aggregate: &'a mut [u8],
    tokens: &'a mut [json::Token],
    events: &'a mut [wire::Event],
    items: &'a mut [Item],
    checkpoints: &'a mut [Item],
    state: State,
    payload_len: usize,
    event_count: usize,
    event_at: usize,
    decoded: bool,
    applied: bool,
    emit_current: bool,
    current_item: Option<usize>,
    terminal_emitted: bool,
    cancel_requested: bool,
    eof: bool,
    failure: Option<Error>,
    local_terminal: bool,
    last_framed: Option<u8>,
    identity_update: bool,
}

impl<'a, 'c> Session<'a, 'c> {
    pub fn new<const T: usize, const E: usize, const I: usize>(
        config: SessionConfig<'c>,
        framing: &'a mut [u8],
        payload: &'a mut [u8],
        aggregate: &'a mut [u8],
        workspace: &'a mut StreamWorkspace<T, E, I>,
    ) -> Result<Self, Error> {
        // Stream schema validation is narrower than complete-document validation.
        zconv::stream::validate_stream_format(config.source.raw()).map_err(Error::from)?;
        workspace.items.fill(Item::EMPTY);
        Ok(Self {
            config,
            framer: sse::Decoder::new(framing),
            payload,
            aggregate,
            tokens: &mut workspace.tokens,
            events: &mut workspace.events,
            items: &mut workspace.items,
            checkpoints: &mut workspace.checkpoints,
            state: State::new(),
            payload_len: 0,
            event_count: 0,
            event_at: 0,
            decoded: false,
            applied: false,
            emit_current: false,
            current_item: None,
            terminal_emitted: false,
            cancel_requested: false,
            eof: false,
            failure: None,
            local_terminal: false,
            last_framed: None,
            identity_update: false,
        })
    }

    /// The harness has prepared and accepted the outgoing request for this turn.
    pub fn prepare(&mut self) -> Result<(), Error> {
        if self.state.phase != CallState::Idle {
            return Err(Error::lifecycle("prepare", "call is not idle"));
        }
        self.state.phase = CallState::Prepared;
        Ok(())
    }
    pub fn state(&self) -> CallState {
        if self.cancel_requested && self.state.outcome.is_none() {
            CallState::CancelRequested
        } else {
            self.state.phase
        }
    }
    pub fn usage(&self) -> StreamUsage {
        StreamUsage::from_raw(self.state.usage.usage())
    }
    pub fn identity(&self) -> CallIdentity<'_> {
        let text = |span: Span| {
            if span.present {
                core::str::from_utf8(bytes(self.aggregate, span))
                    .ok()
                    .map(Text::new)
            } else {
                None
            }
        };
        CallIdentity {
            response_id: text(self.state.id),
            model: text(self.state.model),
            created: self.state.created,
        }
    }
    pub fn loss_count(&self) -> usize {
        self.state.loss_count
    }
    pub fn pending_tool_count(&self) -> usize {
        self.items[..self.state.item_count]
            .iter()
            .filter(|item| {
                item.kind == BlockType::Tool && item.ready_emitted && !item.result_submitted
            })
            .count()
    }
    /// Reborrow an already delivered ready tool. Rust prevents workspace reuse
    /// while the returned argument view is held by the caller.
    pub fn tool(&self, call_id: &str) -> Result<ReadyTool<'_>, Error> {
        let slot = self.items[..self.state.item_count]
            .iter()
            .position(|item| {
                item.kind == BlockType::Tool && bytes(self.aggregate, item.id) == call_id.as_bytes()
            })
            .ok_or_else(|| Error::invalid("tool call id", "unknown call ID"))?;
        if !self.items[slot].ready_emitted || self.items[slot].incomplete {
            return Err(Error::lifecycle(
                "tool view",
                "tool arguments have not become executable",
            ));
        }
        self.ready_tool(slot)
    }
    pub fn memory(&self) -> StreamMemory {
        StreamMemory {
            state_bytes: core::mem::size_of::<Self>(),
            framing_bytes: self.framer.capacity(),
            payload_bytes: self.payload.len(),
            aggregate_bytes: self.aggregate.len(),
            token_bytes: core::mem::size_of_val(self.tokens),
            event_bytes: core::mem::size_of_val(self.events),
            item_bytes: core::mem::size_of_val(self.items),
            checkpoint_bytes: core::mem::size_of_val(self.checkpoints),
            aggregate_used: self.state.aggregate_used,
            aggregate_peak: self.state.aggregate_peak,
            items_used: self.state.item_count,
        }
    }

    /// Marks local intent only. Transport confirmation is a separate action.
    pub fn request_cancel(&mut self) -> Result<(), Error> {
        if self.state.phase == CallState::Idle || self.state.outcome.is_some() {
            return Err(Error::lifecycle("cancel", "call is not active"));
        }
        self.cancel_requested = true;
        Ok(())
    }
    /// Caller reports successful cancellation. Pending decoded events are discarded.
    pub fn confirm_cancel(&mut self) -> Result<(), Error> {
        if !self.cancel_requested || self.state.outcome.is_some() {
            return Err(Error::lifecycle(
                "cancel confirmation",
                "no active local cancellation request",
            ));
        }
        self.state.phase = CallState::Cancelled;
        self.state.outcome = Some(TurnOutcome::Cancelled);
        self.local_terminal = true;
        self.event_count = 0;
        self.decoded = false;
        self.applied = false;
        Ok(())
    }
    /// A caller-reported transport failure is distinct from protocol/model errors.
    pub fn transport_failed(&mut self, detail: &'static str) -> Result<(), Error> {
        if self.state.phase == CallState::Idle || self.state.outcome.is_some() {
            return Err(Error::lifecycle("transport", "call is not active"));
        }
        self.fail(Error::new(ErrorKind::Transport, "transport", detail));
        Ok(())
    }

    /// Register a completed harness tool action by its call ID. No result is executed
    /// or stored. Add the actual result to the caller's next `RequestBuilder`.
    pub fn submit_tool_result(&mut self, call_id: &str) -> Result<(), Error> {
        if !matches!(
            self.state.phase,
            CallState::WaitingTools | CallState::ToolsResolved
        ) || !self.terminal_emitted
        {
            return Err(Error::lifecycle(
                "tool result",
                "the model turn has not requested tool results",
            ));
        }
        let slot = self.items[..self.state.item_count]
            .iter()
            .position(|item| {
                item.kind == BlockType::Tool && bytes(self.aggregate, item.id) == call_id.as_bytes()
            })
            .ok_or_else(|| Error::invalid("tool result id", "unknown call ID"))?;
        let item = &mut self.items[slot];
        if !item.ready_emitted || item.incomplete {
            return Err(Error::lifecycle("tool result", "tool was not executable"));
        }
        if item.result_submitted {
            return Err(Error::lifecycle("tool result", "duplicate result"));
        }
        item.result_submitted = true;
        if self.items[..self.state.item_count]
            .iter()
            .filter(|item| item.kind == BlockType::Tool && item.ready_emitted)
            .all(|item| item.result_submitted)
        {
            self.state.phase = CallState::ToolsResolved;
        }
        Ok(())
    }

    /// Clears this local turn. The caller remains responsible for an active transport.
    pub fn reset(&mut self) -> Result<(), Error> {
        self.framer.finish();
        let buffer = self.framer.replace_buffer(&mut []).map_err(Error::from)?;
        self.framer = sse::Decoder::new(buffer);
        self.items.fill(Item::EMPTY);
        self.state = State::new();
        self.payload_len = 0;
        self.event_count = 0;
        self.event_at = 0;
        self.decoded = false;
        self.applied = false;
        self.emit_current = false;
        self.current_item = None;
        self.terminal_emitted = false;
        self.cancel_requested = false;
        self.eof = false;
        self.failure = None;
        self.local_terminal = false;
        self.last_framed = None;
        self.identity_update = false;
        Ok(())
    }

    /// Replace byte storage while retaining accepted input and committed events.
    pub fn replace_buffer(
        &mut self,
        region: Resource,
        buffer: &'a mut [u8],
    ) -> Result<&'a mut [u8], Error> {
        if region == Resource::Framing {
            return self
                .framer
                .replace_buffer(buffer)
                .map_err(|e| Error::from(e).with_resource(Resource::Framing).retained());
        }
        let (old, used) = match region {
            Resource::Payload => (&mut self.payload, self.payload_len),
            Resource::Aggregate => (&mut self.aggregate, self.state.aggregate_used),
            _ => {
                return Err(Error::invalid(
                    "workspace replacement",
                    "region is not byte storage",
                ))
            }
        };
        if buffer.len() < used {
            return Err(Error::capacity(region, used, true, Replay::Retained));
        }
        buffer[..used].copy_from_slice(&old[..used]);
        Ok(core::mem::replace(old, buffer))
    }

    /// Grow typed storage, including from zero capacity, without replaying input.
    pub fn replace_workspace<const T: usize, const E: usize, const I: usize>(
        &mut self,
        workspace: &'a mut StreamWorkspace<T, E, I>,
    ) -> Result<(), Error> {
        if E < self.event_count {
            return Err(Error::capacity(
                Resource::Events,
                self.event_count,
                true,
                Replay::Retained,
            ));
        }
        if I < self.state.item_count {
            return Err(Error::capacity(
                Resource::Items,
                self.state.item_count,
                true,
                Replay::Retained,
            ));
        }
        workspace.events[..self.event_count].copy_from_slice(&self.events[..self.event_count]);
        workspace.items.fill(Item::EMPTY);
        workspace.items[..self.state.item_count]
            .copy_from_slice(&self.items[..self.state.item_count]);
        self.tokens = &mut workspace.tokens;
        self.events = &mut workspace.events;
        self.items = &mut workspace.items;
        self.checkpoints = &mut workspace.checkpoints;
        Ok(())
    }

    /// Consume an arbitrary fragment. Advance input by `consumed` even on backpressure.
    /// EOF is latched only after all supplied input is consumed. A paused event is
    /// retried, while accepted events are never replayed. References cannot escape
    /// the higher-ranked consumer lifetime.
    pub fn step(
        &mut self,
        input: &[u8],
        eof: bool,
        mut consumer: impl for<'e> FnMut(Event<'e>) -> Control,
    ) -> SessionStep {
        let mut consumed = 0;
        let mut emitted = 0;
        if self.state.phase == CallState::Idle {
            return SessionStep {
                consumed,
                emitted,
                status: StepStatus::Failed(Error::lifecycle("step", "prepare the call first")),
            };
        }
        let status = loop {
            if self.local_terminal && !self.terminal_emitted {
                let event = Event {
                    source: self.config.source,
                    replay: None,
                    data: EventData::Terminal(Terminal {
                        outcome: self.state.outcome.unwrap_or(TurnOutcome::LocalFailure),
                        usage: self.usage(),
                        error: self.failure,
                        upstream_code: None,
                        upstream_message: None,
                    }),
                };
                if consumer(event) == Control::Pause {
                    break StepStatus::NeedEvents;
                }
                emitted += 1;
                self.terminal_emitted = true;
                self.local_terminal = false;
            }
            if let Some(error) = self.failure {
                if self.terminal_emitted {
                    break StepStatus::Failed(error);
                }
            }
            if self.decoded && self.event_at < self.event_count {
                if !self.applied {
                    let raw = self.events[self.event_at];
                    let saved = self.state;
                    self.checkpoints[..saved.item_count]
                        .copy_from_slice(&self.items[..saved.item_count]);
                    match self.apply(raw) {
                        Ok((emit, item)) => {
                            self.emit_current = emit;
                            self.current_item = item;
                            self.applied = true;
                            self.identity_update =
                                raw.kind == wire::EventKind::Begin && saved.begun;
                        }
                        Err(error) => {
                            self.rollback(saved);
                            if let Some(capacity) = error.capacity {
                                break StepStatus::NeedBuffer(capacity);
                            }
                            self.fail(error);
                            continue;
                        }
                    }
                }
                // Tool readiness precedes its enclosing block/choice/turn end.
                if let Some(slot) = self.items[..self.state.item_count]
                    .iter()
                    .position(|item| item.ready_pending)
                {
                    let tool = match self.ready_tool(slot) {
                        Ok(tool) => tool,
                        Err(error) => {
                            self.fail(error);
                            continue;
                        }
                    };
                    let event = Event {
                        source: self.config.source,
                        data: EventData::ToolReady(tool),
                        replay: None,
                    };
                    if consumer(event) == Control::Pause {
                        break StepStatus::NeedEvents;
                    }
                    emitted += 1;
                    self.items[slot].ready_pending = false;
                    self.items[slot].ready_emitted = true;
                    continue;
                }
                if self.emit_current {
                    let event = match self.current_event() {
                        Ok(event) => event,
                        Err(error) => {
                            self.fail(error);
                            continue;
                        }
                    };
                    let terminal = matches!(event.data, EventData::Terminal(_));
                    if consumer(event) == Control::Pause {
                        break StepStatus::NeedEvents;
                    }
                    emitted += 1;
                    if terminal {
                        self.terminal_emitted = true;
                    }
                    if self.events[self.event_at].kind == wire::EventKind::ToolComplete {
                        if let Some(slot) = self.current_item {
                            self.items[slot].ready_emitted = true;
                        }
                    }
                }
                self.event_at += 1;
                self.applied = false;
                continue;
            }
            if self.decoded {
                self.decoded = false;
                self.event_count = 0;
                self.event_at = 0;
                self.payload_len = 0;
                self.framer.consume_event();
            }
            if self.terminal_emitted {
                // zconv dispatches on CR immediately. Its optional LF belongs to
                // that same frame even if the transport splits the CRLF pair.
                if self.last_framed == Some(b'\r') && input.get(consumed) == Some(&b'\n') {
                    consumed += 1;
                    self.last_framed = Some(b'\n');
                }
                if consumed != input.len() {
                    break StepStatus::Failed(Error::lifecycle(
                        "stream",
                        "bytes supplied after a terminal event",
                    ));
                }
                break self.failure.map_or(StepStatus::Done, StepStatus::Failed);
            }
            if self.eof {
                if consumed != input.len() {
                    self.fail(Error::lifecycle("stream", "input supplied after EOF"));
                    continue;
                }
                if self.config.source.protocol == Protocol::GeminiGenerateContent
                    && self.state.seen_choice_end
                {
                    if self.events.is_empty() {
                        break capacity(Resource::Events, 1, true);
                    }
                    self.events[0] = wire::Event {
                        kind: wire::EventKind::Finish,
                        ..wire::Event::EMPTY
                    };
                    self.event_count = 1;
                    self.event_at = 0;
                    self.decoded = true;
                    continue;
                }
                self.fail(Error::new(
                    ErrorKind::Truncated,
                    "stream",
                    "EOF before a valid terminal marker",
                ));
                continue;
            }
            let progress = self.framer.push(&input[consumed..]);
            consumed += progress.consumed;
            if progress.consumed != 0 {
                self.last_framed = input.get(consumed - 1).copied();
            }
            match progress.status {
                sse::Status::NeedBuffer { needed } => {
                    break capacity(Resource::Framing, needed, false)
                }
                sse::Status::Event => {
                    let Some(frame) = self.framer.event() else {
                        continue;
                    };
                    self.payload_len = match frame.data_into(self.payload) {
                        Ok(Some(len)) => len,
                        Ok(None) => {
                            self.framer.consume_event();
                            continue;
                        }
                        Err(zconv::Error::OutputTooSmall { needed }) => {
                            break capacity(Resource::Payload, needed, true)
                        }
                        Err(error) => {
                            self.fail(Error::from(error));
                            continue;
                        }
                    };
                    match wire::decode_frame_format(
                        self.config.source.raw(),
                        frame.event_type(),
                        &self.payload[..self.payload_len],
                        self.tokens,
                        self.events,
                        self.config.limits.raw(),
                    ) {
                        Ok(count) => {
                            self.event_count = count;
                            self.event_at = 0;
                            self.decoded = true;
                        }
                        Err(error) => {
                            let mut error = Error::from(error).retained();
                            if error.capacity.is_some() {
                                if matches!(error.capacity.map(|c| c.region), Some(Resource::Ir)) {
                                    error = error.with_resource(Resource::Events);
                                }
                                if let Some(capacity) = error.capacity {
                                    break StepStatus::NeedBuffer(capacity);
                                }
                            }
                            self.fail(error);
                        }
                    }
                }
                sse::Status::NeedInput | sse::Status::End => {
                    if consumed == input.len() && eof {
                        if self.framer.finish() {
                            self.fail(Error::new(
                                ErrorKind::Truncated,
                                "stream",
                                "incomplete SSE frame at EOF",
                            ));
                            continue;
                        }
                        self.eof = true;
                        if self.config.source.protocol == Protocol::GeminiGenerateContent
                            && self.state.seen_choice_end
                        {
                            if self.events.is_empty() {
                                break capacity(Resource::Events, 1, true);
                            }
                            self.events[0] = wire::Event {
                                kind: wire::EventKind::Finish,
                                ..wire::Event::EMPTY
                            };
                            self.event_count = 1;
                            self.event_at = 0;
                            self.decoded = true;
                            continue;
                        }
                        self.fail(Error::new(
                            ErrorKind::Truncated,
                            "stream",
                            "EOF before a valid terminal marker",
                        ));
                        continue;
                    }
                    break StepStatus::NeedInput;
                }
            }
        };
        SessionStep {
            consumed,
            emitted,
            status,
        }
    }

    fn fail(&mut self, error: Error) {
        self.failure = Some(error);
        self.state.phase = if error.kind == ErrorKind::Truncated {
            CallState::Truncated
        } else {
            CallState::Failed
        };
        self.state.outcome = Some(TurnOutcome::LocalFailure);
        self.local_terminal = !self.terminal_emitted;
        self.event_count = 0;
        self.event_at = 0;
        self.decoded = false;
        self.applied = false;
    }
    fn rollback(&mut self, saved: State) {
        for item in &self.checkpoints[..saved.item_count] {
            if item.tail != NONE {
                write_word(self.aggregate, item.tail, NONE);
            }
        }
        self.items[..saved.item_count].copy_from_slice(&self.checkpoints[..saved.item_count]);
        self.items[saved.item_count..].fill(Item::EMPTY);
        self.state = saved;
    }

    fn apply(&mut self, raw: wire::Event) -> Result<(bool, Option<usize>), Error> {
        Engine {
            config: self.config,
            state: &mut self.state,
            aggregate: self.aggregate,
            items: self.items,
            tokens: self.tokens,
        }
        .apply(raw, &self.payload[..self.payload_len])
    }
    fn ready_tool(&self, slot: usize) -> Result<ReadyTool<'_>, Error> {
        let item = self
            .items
            .get(slot)
            .ok_or_else(|| Error::invalid("tool slot", "missing tool"))?;
        let utf8 = |span| {
            core::str::from_utf8(bytes(self.aggregate, span))
                .map_err(|_| Error::invalid("tool identity", "invalid UTF-8"))
        };
        Ok(ReadyTool {
            key: item.key,
            call_id: utf8(item.id)?,
            name: utf8(item.name)?,
            item_id: if item.item_id.present {
                Some(utf8(item.item_id)?)
            } else {
                None
            },
            arguments: ToolArguments {
                bytes: bytes(self.aggregate, item.complete),
            },
        })
    }
    fn current_event(&self) -> Result<Event<'_>, Error> {
        let raw = self
            .events
            .get(self.event_at)
            .ok_or_else(|| Error::invalid("event", "missing pending event"))?;
        let payload = &self.payload[..self.payload_len];
        let key = EventKey::from_raw(raw.key);
        let data = match raw.kind {
            wire::EventKind::Begin => {
                let identity = self.identity();
                if self.identity_update {
                    EventData::IdentityUpdated(identity)
                } else {
                    EventData::Started {
                        response_id: identity.response_id,
                        model: identity.model,
                        created: identity.created,
                    }
                }
            }
            wire::EventKind::BlockStart => {
                let item = self
                    .current_item
                    .and_then(|slot| self.items.get(slot))
                    .ok_or_else(|| Error::invalid("block", "missing active item"))?;
                let text = |span: Span| -> Result<Option<Text<'_>>, Error> {
                    if !span.present {
                        return Ok(None);
                    }
                    core::str::from_utf8(bytes(self.aggregate, span))
                        .map(|s| Some(Text::new(s)))
                        .map_err(|_| Error::invalid("block identity", "invalid UTF-8"))
                };
                EventData::BlockStarted {
                    key,
                    kind: item.kind,
                    call_id: text(item.id)?,
                    name: text(item.name)?,
                    item_id: text(item.item_id)?,
                }
            }
            wire::EventKind::Delta => EventData::Delta {
                key,
                kind: BlockType::from_raw(raw.block),
                text: required_text(payload, raw.a)?,
            },
            wire::EventKind::Signature => EventData::Signature {
                key,
                signature: required_text(payload, raw.a)?,
            },
            wire::EventKind::BlockEnd => EventData::BlockEnded { key },
            wire::EventKind::Usage => EventData::Usage(self.usage()),
            wire::EventKind::ChoiceEnd => EventData::ChoiceEnded {
                choice: key.choice,
                reason: StopReason::from_raw(raw.reason),
            },
            wire::EventKind::Snapshot => EventData::SnapshotVerified {
                key,
                kind: BlockType::from_raw(raw.block),
                text: required_text(payload, raw.a)?,
                exact: raw.block == wire::BlockKind::Tool
                    || (self.config.source.protocol == Protocol::OpenAiResponses
                        && self.config.validate_text_snapshots),
            },
            wire::EventKind::BlockIncomplete => EventData::BlockIncomplete { key },
            wire::EventKind::Extension => EventData::Loss {
                source: self.config.source,
                field: StreamLoss::from_raw(raw.extension),
            },
            wire::EventKind::ToolComplete => EventData::ToolReady(
                self.ready_tool(
                    self.current_item
                        .ok_or_else(|| Error::invalid("tool", "missing completed tool"))?,
                )?,
            ),
            wire::EventKind::Finish | wire::EventKind::Failure => EventData::Terminal(Terminal {
                outcome: self.state.outcome.unwrap_or(TurnOutcome::LocalFailure),
                usage: self.usage(),
                error: None,
                upstream_code: if raw.kind == wire::EventKind::Failure {
                    event_text(payload, raw.a)?
                } else {
                    None
                },
                upstream_message: if raw.kind == wire::EventKind::Failure {
                    event_text(payload, raw.b)?
                } else {
                    None
                },
            }),
        };
        Ok(Event {
            source: self.config.source,
            data,
            replay: Some((raw, payload)),
        })
    }
}

struct Engine<'a, 'c> {
    config: SessionConfig<'c>,
    state: &'a mut State,
    aggregate: &'a mut [u8],
    items: &'a mut [Item],
    tokens: &'a mut [json::Token],
}
impl<'a, 'c> Engine<'a, 'c> {
    fn apply(&mut self, raw: wire::Event, payload: &[u8]) -> Result<(bool, Option<usize>), Error> {
        if self.state.outcome.is_some() {
            return Err(Error::lifecycle("stream", "event after terminal state"));
        }
        let key = EventKey::from_raw(raw.key);
        let kind = BlockType::from_raw(raw.block);
        if !self.state.begun
            && !matches!(
                raw.kind,
                wire::EventKind::Begin
                    | wire::EventKind::Usage
                    | wire::EventKind::Extension
                    | wire::EventKind::Failure
            )
        {
            return Err(Error::lifecycle(
                "stream",
                "content before the response started",
            ));
        }
        let mut current = None;
        match raw.kind {
            wire::EventKind::Begin => {
                let id = event_text(payload, raw.a)?;
                let model = event_text(payload, raw.b)?;
                if self.state.begun {
                    if !matches!(
                        self.config.source.protocol,
                        Protocol::OpenAiChat
                            | Protocol::OpenAiCompletions
                            | Protocol::GeminiGenerateContent
                    ) {
                        return Err(Error::lifecycle("response start", "duplicate start event"));
                    }
                    for (provided, stored) in [(id, self.state.id), (model, self.state.model)] {
                        if let Some(provided) = provided {
                            if stored.present
                                && !text_equals(provided, bytes(self.aggregate, stored))
                            {
                                return Err(Error::invalid(
                                    "response identity",
                                    "identity changed during a turn",
                                ));
                            }
                        }
                    }
                    if raw.created.is_some()
                        && self.state.created.is_some()
                        && raw.created != self.state.created
                    {
                        return Err(Error::invalid("created", "timestamp changed during a turn"));
                    }
                    let changed = (!self.state.id.present && id.is_some())
                        || (!self.state.model.present && model.is_some())
                        || (self.state.created.is_none() && raw.created.is_some());
                    if !self.state.id.present {
                        self.state.id = self.store_optional(id)?;
                    }
                    if !self.state.model.present {
                        self.state.model = self.store_optional(model)?;
                    }
                    self.state.created = self.state.created.or(raw.created);
                    return Ok((changed, None));
                }
                self.state.id = self.store_optional(id)?;
                self.state.model = self.store_optional(model)?;
                self.state.created = raw.created;
                self.state.begun = true;
                self.state.phase = CallState::Receiving;
            }
            wire::EventKind::BlockStart => {
                let call_id = if kind == BlockType::Tool {
                    event_text(payload, raw.a)?
                } else {
                    None
                };
                let name = event_text(payload, raw.b)?;
                let item_id = event_text(payload, raw.item_id)?.or(
                    if kind != BlockType::Tool
                        && self.config.source.protocol == Protocol::OpenAiResponses
                    {
                        event_text(payload, raw.a)?
                    } else {
                        None
                    },
                );
                current = Some(self.open(key, kind, call_id, name, item_id)?);
            }
            wire::EventKind::Delta => {
                let slot = match self.lookup(key, Some(kind)) {
                    Some(slot) => slot,
                    None if kind != BlockType::Tool
                        && matches!(
                            self.config.source.protocol,
                            Protocol::OpenAiChat
                                | Protocol::OpenAiCompletions
                                | Protocol::GeminiGenerateContent
                        ) =>
                    {
                        self.open(key, kind, None, None, None)?
                    }
                    None => return Err(Error::lifecycle("delta", "content block has not started")),
                };
                if self.items[slot].closed || self.items[slot].finished {
                    return Err(Error::lifecycle("delta", "content block has ended"));
                }
                let text = required_text(payload, raw.a)?;
                self.append(slot, text)?;
                current = Some(slot);
            }
            wire::EventKind::Signature => {
                required_text(payload, raw.a)?;
                let slot = self
                    .lookup(key, Some(BlockType::Reasoning))
                    .ok_or_else(|| {
                        Error::lifecycle("signature", "reasoning block has not started")
                    })?;
                if self.items[slot].closed {
                    return Err(Error::lifecycle("signature", "reasoning block has ended"));
                }
                current = Some(slot);
            }
            wire::EventKind::BlockEnd => {
                let mut found = false;
                let mut open = false;
                for slot in 0..self.state.item_count {
                    let item = self.items[slot];
                    if item.key.choice == key.choice
                        && item.key.item == key.item
                        && (key.part == 0 || item.key.part == key.part)
                    {
                        found = true;
                        open |= !item.closed;
                        self.close(slot)?;
                    }
                }
                if !found || !open {
                    return Err(Error::lifecycle(
                        "block end",
                        "unknown or already ended block",
                    ));
                }
            }
            wire::EventKind::Usage => {
                self.state.usage.update(raw.usage).map_err(Error::from)?;
            }
            wire::EventKind::ChoiceEnd => {
                if self.items[..self.state.item_count]
                    .iter()
                    .any(|item| item.key.choice == key.choice && item.finished)
                {
                    return Err(Error::lifecycle(
                        "choice end",
                        "duplicate choice terminator",
                    ));
                }
                if !self.items[..self.state.item_count]
                    .iter()
                    .any(|item| item.key.choice == key.choice)
                {
                    self.open(
                        EventKey {
                            choice: key.choice,
                            item: u32::MAX,
                            part: 0,
                        },
                        BlockType::Text,
                        None,
                        None,
                        None,
                    )?;
                }
                let reason = StopReason::from_raw(raw.reason);
                for slot in 0..self.state.item_count {
                    if self.items[slot].key.choice == key.choice {
                        if !self.items[slot].closed
                            && matches!(reason, StopReason::Length | StopReason::Safety)
                        {
                            self.items[slot].incomplete = true;
                        }
                        self.close(slot)?;
                        self.items[slot].finished = true;
                        self.items[slot].reason = reason;
                    }
                }
                self.state.seen_choice_end = true;
            }
            wire::EventKind::Finish => {
                let outcome = match raw.completion {
                    wire::Completion::Cancelled => TurnOutcome::Cancelled,
                    wire::Completion::Failed => TurnOutcome::UpstreamFailed,
                    completion => {
                        if !self.state.seen_choice_end
                            || self.items[..self.state.item_count]
                                .iter()
                                .any(|item| !item.finished)
                        {
                            return Err(Error::new(
                                ErrorKind::Truncated,
                                "stream",
                                "terminal marker before every choice finished",
                            ));
                        }
                        if completion == wire::Completion::Incomplete
                            || self.items[..self.state.item_count].iter().any(|item| {
                                item.incomplete
                                    || matches!(
                                        item.reason,
                                        StopReason::Length | StopReason::Safety
                                    )
                            })
                        {
                            TurnOutcome::Incomplete
                        } else if completion == wire::Completion::RequiresAction
                            || self.items[..self.state.item_count]
                                .iter()
                                .any(|item| item.kind == BlockType::Tool)
                        {
                            TurnOutcome::WaitingTools
                        } else {
                            TurnOutcome::Completed
                        }
                    }
                };
                self.state.outcome = Some(outcome);
                self.state.phase = match outcome {
                    TurnOutcome::Completed => CallState::Completed,
                    TurnOutcome::WaitingTools => CallState::WaitingTools,
                    TurnOutcome::Incomplete => CallState::Incomplete,
                    TurnOutcome::Cancelled => CallState::Cancelled,
                    TurnOutcome::UpstreamFailed => CallState::UpstreamFailed,
                    TurnOutcome::LocalFailure => CallState::Failed,
                };
            }
            wire::EventKind::Failure => {
                required_text(payload, raw.b)?;
                self.state.outcome = Some(TurnOutcome::UpstreamFailed);
                self.state.phase = CallState::UpstreamFailed;
            }
            wire::EventKind::Extension => {
                let field = StreamLoss::from_raw(raw.extension);
                if !self.config.allowed_losses.contains(&field) {
                    return Err(Error::new(
                        ErrorKind::ForbiddenLoss,
                        field.field(),
                        "stream adapter needs explicit permission to discard this field",
                    ));
                }
                self.state.loss_count = self.state.loss_count.saturating_add(1);
            }
            wire::EventKind::ToolComplete => {
                let index = u32::try_from(self.state.item_count).map_err(|_| {
                    Error::new(
                        ErrorKind::Limit,
                        "item index",
                        "more items than a u32 index can express",
                    )
                })?;
                let normalized = EventKey {
                    choice: key.choice,
                    item: index,
                    part: 0,
                };
                let id = event_text(payload, raw.a)?.or_else(|| self.binding(key));
                let id = id.ok_or_else(|| {
                    Error::new(
                        ErrorKind::ContextRequired,
                        "tool call id",
                        "supply a tool identity for the source key",
                    )
                })?;
                let name = required_text(payload, raw.b)?;
                let args = required_text(payload, raw.c)?;
                let slot = self.open(
                    normalized,
                    BlockType::Tool,
                    Some(id),
                    Some(name),
                    event_text(payload, raw.item_id)?,
                )?;
                self.append(slot, args)?;
                self.close(slot)?;
                // This raw event itself carries the ready tool and is replayable.
                self.items[slot].ready_pending = false;
                current = Some(slot);
            }
            wire::EventKind::Snapshot => {
                let slot = self.lookup(key, Some(kind)).ok_or_else(|| {
                    Error::lifecycle("snapshot", "content block was never started")
                })?;
                let text = required_text(payload, raw.a)?;
                if self.items[slot].len != text.decoded_len() {
                    return Err(Error::invalid(
                        "snapshot",
                        "final snapshot differs from accumulated deltas",
                    ));
                }
                if self.retains(kind) && !self.chain_equals(self.items[slot], text)? {
                    return Err(Error::invalid(
                        "snapshot",
                        "final snapshot differs from accumulated deltas",
                    ));
                }
                current = Some(slot);
            }
            wire::EventKind::BlockIncomplete => {
                let mut found = false;
                for item in &mut self.items[..self.state.item_count] {
                    if item.key.choice == key.choice && item.key.item == key.item {
                        item.incomplete = true;
                        found = true;
                    }
                }
                if !found {
                    return Err(Error::lifecycle("incomplete block", "unknown output item"));
                }
            }
        }
        Ok((true, current))
    }
    fn lookup(&self, key: EventKey, kind: Option<BlockType>) -> Option<usize> {
        self.items[..self.state.item_count]
            .iter()
            .position(|item| item.key == key && kind.map_or(true, |kind| kind == item.kind))
    }
    fn binding(&self, key: EventKey) -> Option<Text<'c>> {
        self.config
            .tool_ids
            .iter()
            .find(|entry| {
                entry.key == key
                    && !self.items[..self.state.item_count].iter().any(|item| {
                        item.kind == BlockType::Tool
                            && bytes(self.aggregate, item.id) == entry.call_id.as_bytes()
                    })
            })
            .map(|entry| Text::new(entry.call_id))
    }
    fn store_optional(&mut self, text: Option<Text<'_>>) -> Result<Span, Error> {
        text.map(|text| self.store(text))
            .transpose()
            .map(|v| v.unwrap_or(Span::EMPTY))
    }
    fn reserve(&self, length: usize) -> Result<(usize, usize), Error> {
        let start = self.state.aggregate_used;
        let end = start.checked_add(length).ok_or_else(|| {
            Error::capacity(Resource::Aggregate, usize::MAX, false, Replay::Retained)
        })?;
        if end > self.aggregate.len() {
            return Err(Error::capacity(
                Resource::Aggregate,
                end,
                true,
                Replay::Retained,
            ));
        }
        Ok((start, end))
    }
    fn store(&mut self, text: Text<'_>) -> Result<Span, Error> {
        let len = text.decoded_len();
        let (start, end) = self.reserve(len)?;
        text.decode_into(&mut self.aggregate[start..end])?;
        self.state.aggregate_used = end;
        self.state.aggregate_peak = self.state.aggregate_peak.max(end);
        Ok(Span {
            start,
            len,
            present: true,
        })
    }
    fn open(
        &mut self,
        key: EventKey,
        kind: BlockType,
        id: Option<Text<'_>>,
        name: Option<Text<'_>>,
        item_id: Option<Text<'_>>,
    ) -> Result<usize, Error> {
        if self.lookup(key, Some(kind)).is_some() {
            return Err(Error::lifecycle(
                "block start",
                "duplicate block index or repeated tool header",
            ));
        }
        if matches!(
            self.config.source.protocol,
            Protocol::AnthropicMessages | Protocol::OpenAiResponses | Protocol::GeminiInteractions
        ) && self.lookup(key, None).is_some()
        {
            return Err(Error::lifecycle(
                "block start",
                "index reused by another block type",
            ));
        }
        if self.items[..self.state.item_count]
            .iter()
            .any(|item| item.key.choice == key.choice && item.finished)
        {
            return Err(Error::lifecycle("block start", "choice has ended"));
        }
        let slot = self.state.item_count;
        if slot == self.items.len() {
            return Err(Error::capacity(
                Resource::Items,
                slot.saturating_add(1),
                false,
                Replay::Retained,
            ));
        }
        let (id, name) = if kind == BlockType::Tool {
            let id = id.or_else(|| self.binding(key)).ok_or_else(|| {
                Error::new(
                    ErrorKind::ContextRequired,
                    "tool call id",
                    "supply a tool identity for the source key",
                )
            })?;
            let name = name.ok_or_else(|| Error::unsupported("fragmented or missing tool name"))?;
            if id.decoded_len() == 0 || name.decoded_len() == 0 {
                return Err(Error::invalid(
                    "tool identity",
                    "empty call ID or function name",
                ));
            }
            if self.items[..slot].iter().any(|item| {
                item.kind == BlockType::Tool && text_equals(id, bytes(self.aggregate, item.id))
            }) {
                return Err(Error::invalid(
                    "tool call id",
                    "duplicate call ID in this turn",
                ));
            }
            (self.store(id)?, self.store(name)?)
        } else {
            (Span::EMPTY, Span::EMPTY)
        };
        let item_id = self.store_optional(item_id)?;
        self.items[slot] = Item {
            key,
            kind,
            id,
            name,
            item_id,
            ..Item::EMPTY
        };
        self.state.item_count += 1;
        if kind == BlockType::Tool {
            self.state.phase = CallState::ToolForming;
        }
        Ok(slot)
    }
    fn retains(&self, kind: BlockType) -> bool {
        kind == BlockType::Tool
            || (self.config.source.protocol == Protocol::OpenAiResponses
                && self.config.validate_text_snapshots)
    }
    fn append(&mut self, slot: usize, text: Text<'_>) -> Result<(), Error> {
        let length = text.decoded_len();
        let new_length = self.items[slot].len.checked_add(length).ok_or_else(|| {
            Error::capacity(Resource::Aggregate, usize::MAX, false, Replay::Retained)
        })?;
        if self.retains(self.items[slot].kind) && length != 0 {
            let needed = HEADER.checked_add(length).ok_or_else(|| {
                Error::capacity(Resource::Aggregate, usize::MAX, false, Replay::Retained)
            })?;
            let (start, end) = self.reserve(needed)?;
            text.decode_into(&mut self.aggregate[start + HEADER..end])?;
            write_word(self.aggregate, start, NONE);
            write_word(self.aggregate, start + WORD, length);
            if self.items[slot].tail == NONE {
                self.items[slot].head = start;
            } else {
                write_word(self.aggregate, self.items[slot].tail, start);
            }
            self.items[slot].tail = start;
            self.state.aggregate_used = end;
            self.state.aggregate_peak = self.state.aggregate_peak.max(end);
        }
        self.items[slot].len = new_length;
        Ok(())
    }
    fn close(&mut self, slot: usize) -> Result<(), Error> {
        if self.items[slot].closed {
            return Ok(());
        }
        if self.items[slot].kind == BlockType::Tool && !self.items[slot].incomplete {
            if self.items[slot].len == 0
                && matches!(
                    self.config.source.protocol,
                    Protocol::AnthropicMessages | Protocol::GeminiInteractions
                )
            {
                self.append(slot, Text::new("{}"))?;
            }
            let item = self.items[slot];
            if item.head == NONE {
                return Err(Error::invalid(
                    "tool arguments",
                    "missing complete JSON object",
                ));
            }
            let start = if item.head == item.tail {
                item.head + HEADER
            } else {
                let (start, end) = self.reserve(item.len)?;
                let mut next = item.head;
                let mut at = start;
                let mut visits = 0usize;
                while next != NONE {
                    visits += 1;
                    if visits > self.state.aggregate_used / HEADER + 1 {
                        return Err(Error::invalid("aggregate", "invalid segment chain"));
                    }
                    let len = read_word(self.aggregate, next + WORD)?;
                    let source = next
                        .checked_add(HEADER)
                        .ok_or_else(|| Error::invalid("aggregate", "segment offset overflow"))?;
                    let source_end = source
                        .checked_add(len)
                        .ok_or_else(|| Error::invalid("aggregate", "segment length overflow"))?;
                    if source_end > self.state.aggregate_used || at.saturating_add(len) > end {
                        return Err(Error::invalid(
                            "aggregate",
                            "segment outside committed storage",
                        ));
                    }
                    self.aggregate.copy_within(source..source_end, at);
                    at += len;
                    next = read_word(self.aggregate, next)?;
                }
                if at != end {
                    return Err(Error::invalid("aggregate", "segment count mismatch"));
                }
                self.state.aggregate_used = end;
                self.state.aggregate_peak = self.state.aggregate_peak.max(end);
                start
            };
            let end = start
                .checked_add(item.len)
                .ok_or_else(|| Error::invalid("tool arguments", "length overflow"))?;
            let arguments = self
                .aggregate
                .get(start..end)
                .ok_or_else(|| Error::invalid("tool arguments", "outside caller storage"))?;
            let document = json::parse(arguments, self.tokens, self.config.limits.raw())
                .map_err(|error| Error::from(error).retained())?;
            if document.root().kind() != json::Kind::Object {
                return Err(Error::invalid("tool arguments", "expected a JSON object"));
            }
            self.items[slot].complete = Span {
                start,
                len: item.len,
                present: true,
            };
            self.items[slot].ready_pending = true;
        }
        self.items[slot].closed = true;
        if self.items[slot].kind == BlockType::Tool
            && !self.items[slot].incomplete
            && !self.items[..self.state.item_count]
                .iter()
                .any(|item| item.kind == BlockType::Tool && !item.closed)
        {
            self.state.phase = CallState::ToolsReady;
        }
        Ok(())
    }
    fn chain_equals(&self, item: Item, text: Text<'_>) -> Result<bool, Error> {
        match text.raw() {
            zconv::ir::Text::Utf8(value) => {
                self.compare_chain(item, value.as_bytes().iter().copied())
            }
            zconv::ir::Text::Json(value) => self.compare_chain(item, value.bytes()),
        }
    }
    fn compare_chain(
        &self,
        item: Item,
        mut expected: impl Iterator<Item = u8>,
    ) -> Result<bool, Error> {
        let mut next = item.head;
        let mut visits = 0usize;
        while next != NONE {
            visits += 1;
            if visits > self.state.aggregate_used / HEADER + 1 {
                return Err(Error::invalid("aggregate", "invalid segment chain"));
            }
            let len = read_word(self.aggregate, next + WORD)?;
            let start = next
                .checked_add(HEADER)
                .ok_or_else(|| Error::invalid("aggregate", "offset overflow"))?;
            let end = start
                .checked_add(len)
                .ok_or_else(|| Error::invalid("aggregate", "length overflow"))?;
            let segment = self
                .aggregate
                .get(start..end)
                .ok_or_else(|| Error::invalid("aggregate", "segment outside storage"))?;
            for &byte in segment {
                if expected.next() != Some(byte) {
                    return Ok(false);
                }
            }
            next = read_word(self.aggregate, next)?;
        }
        Ok(expected.next().is_none())
    }
}

fn capacity(region: Resource, needed: usize, exact: bool) -> StepStatus {
    StepStatus::NeedBuffer(crate::common::Capacity {
        region,
        needed,
        exact,
        replay: Replay::Retained,
    })
}
fn bytes(buffer: &[u8], span: Span) -> &[u8] {
    buffer
        .get(span.start..span.start.saturating_add(span.len))
        .unwrap_or(&[])
}
fn write_word(buffer: &mut [u8], at: usize, value: usize) {
    if let Some(dst) = buffer.get_mut(at..at.saturating_add(WORD)) {
        dst.copy_from_slice(&value.to_ne_bytes());
    }
}
fn read_word(buffer: &[u8], at: usize) -> Result<usize, Error> {
    let raw = buffer
        .get(at..at.saturating_add(WORD))
        .ok_or_else(|| Error::invalid("aggregate", "segment outside storage"))?;
    let mut value = [0; WORD];
    value.copy_from_slice(raw);
    Ok(usize::from_ne_bytes(value))
}
fn event_text(payload: &[u8], span: wire::EventText) -> Result<Option<Text<'_>>, Error> {
    if !span.present {
        return Ok(None);
    }
    let end = span
        .offset
        .checked_add(span.length)
        .ok_or_else(|| Error::invalid("event span", "overflow"))?;
    let raw = payload
        .get(span.offset..end)
        .ok_or_else(|| Error::invalid("event span", "outside payload"))?;
    if span.quoted {
        json::JsonString::parse(raw)
            .map(|s| Some(Text(zconv::ir::Text::Json(s))))
            .map_err(Error::from)
    } else {
        core::str::from_utf8(raw)
            .map(|s| Some(Text::new(s)))
            .map_err(|_| Error::invalid("event text", "invalid UTF-8"))
    }
}
fn required_text(payload: &[u8], span: wire::EventText) -> Result<Text<'_>, Error> {
    event_text(payload, span)?.ok_or_else(|| Error::invalid("event text", "missing required text"))
}
fn text_equals(text: Text<'_>, expected: &[u8]) -> bool {
    match text.raw() {
        zconv::ir::Text::Utf8(value) => value.as_bytes() == expected,
        zconv::ir::Text::Json(value) => value.bytes().eq(expected.iter().copied()),
    }
}
