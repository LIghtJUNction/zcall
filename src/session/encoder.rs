//! Event encoding uses zconv's stateful IR writer and explicit caller facts.

use super::*;

#[derive(Clone, Copy, Debug, Default)]
pub struct EventContext<'a> {
    pub response_id: Option<&'a str>,
    pub model: Option<&'a str>,
    pub created: Option<u64>,
    /// Caller-selected namespace for newly encoded Responses item IDs.
    /// Existing source item IDs need `allow_item_id_loss` because zconv emits
    /// target IDs in this namespace rather than copying source identifiers.
    pub target_item_id_prefix: Option<&'a str>,
    pub initial_usage: StreamUsage,
}

#[derive(Clone, Copy, Debug)]
pub struct EventEncoderConfig<'a> {
    pub source: Format,
    pub target: Format,
    pub limits: Limits,
    pub context: EventContext<'a>,
    pub allow_item_id_loss: bool,
    pub allow_timestamp_loss: bool,
    pub allow_reasoning_loss: bool,
    pub allowed_losses: &'a [StreamLoss],
}
impl EventEncoderConfig<'_> {
    pub fn new(source: Format, target: Format) -> Self {
        Self {
            source,
            target,
            limits: Limits::default(),
            context: EventContext::default(),
            allow_item_id_loss: false,
            allow_timestamp_loss: false,
            allow_reasoning_loss: false,
            allowed_losses: &[],
        }
    }
}

/// Encoder JSON-validation and rollback state. Capacity is explicit in the type.
pub struct EncoderWorkspace<const TOKENS: usize, const ITEMS: usize> {
    tokens: [json::Token; TOKENS],
    items: [wire::ActiveItem; ITEMS],
    checkpoints: [wire::ActiveItem; ITEMS],
}
impl<const T: usize, const I: usize> EncoderWorkspace<T, I> {
    pub const fn new() -> Self {
        Self {
            tokens: [json::Token::EMPTY; T],
            items: [wire::ActiveItem::EMPTY; I],
            checkpoints: [wire::ActiveItem::EMPTY; I],
        }
    }
    pub const fn memory_bytes(&self) -> usize {
        core::mem::size_of::<Self>()
    }
}
impl<const T: usize, const I: usize> Default for EncoderWorkspace<T, I> {
    fn default() -> Self {
        Self::new()
    }
}

/// Exact losses for the event accepted by `push`; retain this report if needed.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventLosses {
    pub item_id: bool,
    pub created: bool,
    pub reasoning: bool,
    pub extension: Option<StreamLoss>,
}
impl EventLosses {
    pub const fn is_empty(self) -> bool {
        !self.item_id && !self.created && !self.reasoning && self.extension.is_none()
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum StageStatus {
    Accepted(EventLosses),
    NeedOutput,
    NeedBuffer(crate::common::Capacity),
    Failed(Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EncoderStatus {
    Ready,
    NeedOutput,
    Done,
    NeedBuffer(crate::common::Capacity),
    Failed(Error),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EncoderStep {
    pub written: usize,
    pub status: EncoderStatus,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct EventEncodingReport {
    pub accepted_events: usize,
    pub dropped_item_ids: usize,
    pub dropped_timestamps: usize,
    pub dropped_reasoning: usize,
    pub dropped_extensions: usize,
}

/// A stateful, resumable encoder. The source is the actual event provenance.
/// Payload packing is plain UTF-8 spans, not a second vendor JSON serializer.
pub struct EventEncoder<'a, 'c> {
    converter: wire::StreamConverter<'a, 'c>,
    config: EventEncoderConfig<'c>,
    scratch: &'a mut [u8],
    pending: bool,
    report: EventEncodingReport,
}
impl<'a, 'c> EventEncoder<'a, 'c> {
    pub fn new<const T: usize, const I: usize>(
        config: EventEncoderConfig<'c>,
        pending: &'a mut [u8],
        aggregate: &'a mut [u8],
        scratch: &'a mut [u8],
        workspace: &'a mut EncoderWorkspace<T, I>,
    ) -> Result<Self, Error> {
        check_cache_write_permission(config, config.context.initial_usage)?;
        let converter = wire::StreamConverter::new(
            wire::StreamConfig {
                source: config.source.raw(),
                target: config.target.raw(),
                strategy: zconv::Strategy::Ir,
                json_limits: config.limits.raw(),
                context: wire::StreamContext {
                    response_id: config.context.response_id,
                    model: config.context.model,
                    created: config.context.created,
                    item_id_prefix: config.context.target_item_id_prefix,
                    initial_usage: config.context.initial_usage.raw(),
                    tool_ids: &[],
                },
                policy: wire::StreamPolicy {
                    drop_reasoning: config.allow_reasoning_loss,
                    drop_extensions: !config.allowed_losses.is_empty(),
                },
                validate_snapshots: true,
                allow_partial_usage: true,
            },
            wire::StreamBuffers {
                framing: &mut [],
                data: &mut [],
                pending,
                tokens: &mut workspace.tokens,
                items: &mut workspace.items,
                checkpoints: &mut workspace.checkpoints,
                events: &mut [],
                aggregate,
            },
        )
        .map_err(Error::from)?;
        Ok(Self {
            converter,
            config,
            scratch,
            pending: false,
            report: EventEncodingReport::default(),
        })
    }
    pub fn report(&self) -> EventEncodingReport {
        self.report
    }
    pub fn usage(&self) -> StreamUsage {
        StreamUsage::from_raw(self.converter.usage())
    }

    /// Success accepts the event atomically; drain before staging another wire event.
    /// Informational ToolReady notifications already represented by a block close
    /// are accepted without emitting duplicate protocol bytes.
    pub fn push(&mut self, event: Event<'_>) -> StageStatus {
        if event.source != self.config.source {
            return StageStatus::Failed(Error::invalid(
                "event source",
                "encoder source differs from event provenance",
            ));
        }
        let Some((original, _)) = event.replay else {
            return if matches!(event.data, EventData::ToolReady(_)) {
                StageStatus::Accepted(EventLosses::default())
            } else {
                StageStatus::Failed(Error::unsupported(
                    "encoding a local transport/cancellation terminal",
                ))
            };
        };
        let mut raw = *original;
        if self.pending {
            return StageStatus::NeedOutput;
        }
        if let Some(semantics) = event.reasoning_kind()
            && self.config.source != self.config.target
            && accepts_reasoning(self.config.target)
            && (semantics == StreamReasoningKind::Unspecified
                || semantics != reasoning_semantics(self.config.target))
        {
            return StageStatus::Failed(Error::unsupported(
                "stream reasoning thought/summary mapping",
            ));
        }
        let mut loss = EventLosses::default();
        let mut parts = [None; 5];
        match event.data {
            EventData::Started {
                response_id,
                model,
                created,
            } => {
                parts[0] = response_id;
                parts[1] = model;
                raw.created = created;
                if created.is_some()
                    && !matches!(
                        self.config.target.protocol,
                        Protocol::OpenAiChat
                            | Protocol::OpenAiCompletions
                            | Protocol::OpenAiResponses
                    )
                {
                    if !self.config.allow_timestamp_loss {
                        return StageStatus::Failed(Error::new(
                            ErrorKind::ForbiddenLoss,
                            "created",
                            "target stream has no timestamp field",
                        ));
                    }
                    loss.created = true;
                }
            }
            EventData::IdentityUpdated(identity) => {
                parts[0] = identity.response_id;
                parts[1] = identity.model;
                raw.created = identity.created;
                if identity.created.is_some()
                    && !matches!(
                        self.config.target.protocol,
                        Protocol::OpenAiChat
                            | Protocol::OpenAiCompletions
                            | Protocol::OpenAiResponses
                    )
                {
                    if !self.config.allow_timestamp_loss {
                        return StageStatus::Failed(Error::new(
                            ErrorKind::ForbiddenLoss,
                            "created",
                            "target stream has no timestamp field",
                        ));
                    }
                    loss.created = true;
                }
            }
            EventData::BlockStarted {
                kind,
                call_id,
                name,
                item_id,
                ..
            } => {
                parts[0] = if kind == BlockType::Tool {
                    call_id
                } else {
                    item_id
                };
                parts[1] = name;
                parts[3] = item_id;
                if item_id.is_some() {
                    if !self.config.allow_item_id_loss {
                        return StageStatus::Failed(Error::new(
                            ErrorKind::ForbiddenLoss,
                            "item.id",
                            "the encoder needs permission to omit or replace the source item ID",
                        ));
                    }
                    loss.item_id = true;
                }
            }
            EventData::Delta { text, .. } | EventData::SnapshotVerified { text, .. } => {
                parts[0] = Some(text)
            }
            EventData::Signature {
                signature, origin, ..
            } => {
                parts[0] = Some(signature);
                if original.b.present {
                    let label = match origin.protocol {
                        Protocol::AnthropicMessages => "anthropic",
                        Protocol::GeminiGenerateContent => "gemini",
                        Protocol::OpenAiChat => "chat",
                        Protocol::OpenAiResponses => "responses",
                        _ => {
                            return StageStatus::Failed(Error::invalid(
                                "signature provider",
                                "unsupported origin protocol",
                            ));
                        }
                    };
                    parts[1] = Some(Text::new(label));
                }
            }
            EventData::ToolReady(tool) => {
                parts[0] = Some(Text::new(tool.call_id));
                parts[1] = Some(Text::new(tool.name));
                let arguments = match tool.arguments.as_str() {
                    Ok(value) => value,
                    Err(error) => return StageStatus::Failed(error),
                };
                parts[2] = Some(Text::new(arguments));
                parts[3] = tool.item_id.map(Text::new);
                parts[4] = tool.signature.map(|signature| signature.value);
                if tool.item_id.is_some() {
                    if !self.config.allow_item_id_loss {
                        return StageStatus::Failed(Error::new(
                            ErrorKind::ForbiddenLoss,
                            "item.id",
                            "source item ID cannot be preserved by this encoder",
                        ));
                    }
                    loss.item_id = true;
                }
            }
            EventData::Images { json, .. } | EventData::Grounding { json, .. } => {
                let value = match core::str::from_utf8(json) {
                    Ok(value) => value,
                    Err(_) => {
                        return StageStatus::Failed(Error::invalid("metadata", "invalid UTF-8"));
                    }
                };
                parts[0] = Some(Text::new(value));
            }
            EventData::Continuation { token, .. } => parts[0] = Some(token),
            EventData::StopSequence { sequence } => parts[0] = Some(sequence),
            EventData::Usage(usage) => {
                if let Err(error) = check_cache_write_permission(self.config, usage) {
                    return StageStatus::Failed(error);
                }
                raw.usage = usage.raw();
            }
            EventData::Loss { field, .. } => {
                if !self.config.allowed_losses.contains(&field) {
                    return StageStatus::Failed(Error::new(
                        ErrorKind::ForbiddenLoss,
                        field.field(),
                        "explicit field loss permission required",
                    ));
                }
                loss.extension = Some(field);
            }
            EventData::Terminal(terminal) => {
                if let Err(error) = check_cache_write_permission(self.config, terminal.usage) {
                    return StageStatus::Failed(error);
                }
                parts[0] = terminal.upstream_code;
                parts[1] = terminal.upstream_message;
            }
            EventData::BlockEnded { .. }
            | EventData::ChoiceEnded { .. }
            | EventData::BlockIncomplete { .. } => {}
        }
        let needed = match parts
            .iter()
            .flatten()
            .try_fold(0usize, |n, value| n.checked_add(value.decoded_len()))
        {
            Some(needed) => needed,
            None => {
                return StageStatus::Failed(Error::capacity(
                    Resource::Scratch,
                    usize::MAX,
                    false,
                    Replay::Unconsumed,
                ));
            }
        };
        if needed > self.scratch.len() {
            return StageStatus::NeedBuffer(crate::common::Capacity {
                region: Resource::Scratch,
                needed,
                exact: true,
                replay: Replay::Unconsumed,
            });
        }
        let mut spans = [wire::EventText::default(); 5];
        let mut at = 0;
        for (index, text) in parts.into_iter().enumerate() {
            if let Some(text) = text {
                let len = text.decoded_len();
                if let Err(error) = text.decode_into(&mut self.scratch[at..at + len]) {
                    return StageStatus::Failed(error);
                }
                spans[index] = wire::EventText {
                    offset: at,
                    length: len,
                    present: true,
                    quoted: false,
                };
                at += len;
            }
        }
        raw.a = spans[0];
        raw.b = spans[1];
        raw.c = spans[2];
        raw.item_id = spans[3];
        raw.signature = spans[4];
        let before = self.converter.report();
        match self.converter.stage_events(&self.scratch[..at], &[raw]) {
            Ok(()) => {
                self.pending = true;
                let after = self.converter.report();
                loss.reasoning = after.dropped_reasoning > before.dropped_reasoning;
                if after.dropped_extensions > before.dropped_extensions
                    && loss.extension.is_none()
                    && after.dropped_fields & (wire::DroppedField::CacheCreationTokens as u32) != 0
                {
                    loss.extension = Some(StreamLoss::CacheCreationTokens);
                }
                self.report.accepted_events = self.report.accepted_events.saturating_add(1);
                self.report.dropped_item_ids = self
                    .report
                    .dropped_item_ids
                    .saturating_add(usize::from(loss.item_id));
                self.report.dropped_timestamps = self
                    .report
                    .dropped_timestamps
                    .saturating_add(usize::from(loss.created));
                self.report.dropped_reasoning = after.dropped_reasoning;
                self.report.dropped_extensions = after.dropped_extensions;
                StageStatus::Accepted(loss)
            }
            Err(status) => match map_status(status) {
                EncoderStatus::NeedBuffer(capacity) => StageStatus::NeedBuffer(capacity),
                EncoderStatus::NeedOutput => StageStatus::NeedOutput,
                EncoderStatus::Failed(error) => StageStatus::Failed(error),
                _ => StageStatus::Failed(Error::lifecycle(
                    "event encoder",
                    "unexpected staging state",
                )),
            },
        }
    }

    /// Drain output with any capacity, including zero or one byte.
    pub fn drain(&mut self, output: &mut [u8]) -> EncoderStep {
        let step = self.converter.step(&[], output, false);
        let status = map_status(step.status);
        if matches!(status, EncoderStatus::Ready | EncoderStatus::Done) {
            self.pending = false;
        }
        EncoderStep {
            written: step.written,
            status,
        }
    }
    pub fn replace_buffer(
        &mut self,
        region: Resource,
        buffer: &'a mut [u8],
    ) -> Result<&'a mut [u8], Error> {
        if region == Resource::Scratch {
            return Ok(core::mem::replace(&mut self.scratch, buffer));
        }
        let kind = match region {
            Resource::Output => wire::BufferKind::Pending,
            Resource::Aggregate => wire::BufferKind::Aggregate,
            _ => {
                return Err(Error::invalid(
                    "encoder buffer",
                    "use byte output, aggregate or scratch storage",
                ));
            }
        };
        self.converter
            .replace_buffer(kind, buffer)
            .map_err(|error| Error::from(error).with_resource(region).retained())
    }
    pub fn replace_workspace<const T: usize, const I: usize>(
        &mut self,
        workspace: &'a mut EncoderWorkspace<T, I>,
    ) -> Result<(), Error> {
        self.converter
            .replace_items(&mut workspace.items, &mut workspace.checkpoints)
            .map_err(|error| Error::from(error).retained())?;
        self.converter.replace_tokens(&mut workspace.tokens);
        Ok(())
    }
}

fn check_cache_write_permission(
    config: EventEncoderConfig<'_>,
    usage: StreamUsage,
) -> Result<(), Error> {
    if config.source != config.target
        && config.target.protocol != Protocol::AnthropicMessages
        && usage.cache_write.is_some()
        && !config
            .allowed_losses
            .contains(&StreamLoss::CacheCreationTokens)
    {
        return Err(Error::new(
            ErrorKind::ForbiddenLoss,
            "cache_creation_input_tokens",
            "this target requires explicit cache-write detail loss permission",
        ));
    }
    Ok(())
}

fn accepts_reasoning(format: Format) -> bool {
    matches!(
        format.protocol,
        Protocol::AnthropicMessages
            | Protocol::GeminiGenerateContent
            | Protocol::GeminiInteractions
    ) || (format.protocol == Protocol::OpenAiChat
        && matches!(format.profile, crate::common::Profile::OpenAiCompatible(_)))
}

fn map_status(status: wire::Status) -> EncoderStatus {
    match status {
        wire::Status::NeedInput => EncoderStatus::Ready,
        wire::Status::NeedOutput => EncoderStatus::NeedOutput,
        wire::Status::Done => EncoderStatus::Done,
        wire::Status::Failed(error) => EncoderStatus::Failed(Error::from(error)),
        wire::Status::NeedBuffer {
            buffer,
            needed,
            exact,
        } => EncoderStatus::NeedBuffer(crate::common::Capacity {
            region: match buffer {
                wire::BufferKind::Framing => Resource::Framing,
                wire::BufferKind::Data => Resource::Payload,
                wire::BufferKind::Pending => Resource::Output,
                wire::BufferKind::Tokens => Resource::Tokens,
                wire::BufferKind::Items => Resource::Items,
                wire::BufferKind::Events => Resource::Events,
                wire::BufferKind::Aggregate => Resource::Aggregate,
            },
            needed,
            exact,
            replay: Replay::Unconsumed,
        }),
    }
}
