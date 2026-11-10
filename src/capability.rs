//! Capability evidence and strict preparation. A wire field is not model evidence.
use crate::common::*;
use crate::document::{
    Content, EncodeContext, EncodeReport, MediaKind, Parameter, Policy, Request, Role, ToolChoice,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Support {
    Supported,
    Unsupported,
    Unknown,
}

/// Portable semantic requirements, independent of wire field names.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[repr(u8)]
pub enum Feature {
    Text,
    Image,
    Audio,
    File,
    Video,
    Tools,
    ToolChoice,
    ParallelTools,
    Reasoning,
    ReasoningSignature,
    Refusal,
    StructuredOutput,
    MultipleChoices,
    ServerContext,
    Extensions,
    Streaming,
    SystemMessages,
    DeveloperMessages,
    GenerationControls,
}
impl Feature {
    pub const ALL: [Self; 19] = [
        Self::Text,
        Self::Image,
        Self::Audio,
        Self::File,
        Self::Video,
        Self::Tools,
        Self::ToolChoice,
        Self::ParallelTools,
        Self::Reasoning,
        Self::ReasoningSignature,
        Self::Refusal,
        Self::StructuredOutput,
        Self::MultipleChoices,
        Self::ServerContext,
        Self::Extensions,
        Self::Streaming,
        Self::SystemMessages,
        Self::DeveloperMessages,
        Self::GenerationControls,
    ];
    pub const fn name(self) -> &'static str {
        match self {
            Self::Text => "text",
            Self::Image => "image",
            Self::Audio => "audio",
            Self::File => "file",
            Self::Video => "video",
            Self::Tools => "tools",
            Self::ToolChoice => "tool choice",
            Self::ParallelTools => "parallel tools",
            Self::Reasoning => "reasoning",
            Self::ReasoningSignature => "reasoning signature",
            Self::Refusal => "refusal",
            Self::StructuredOutput => "structured output",
            Self::MultipleChoices => "multiple choices",
            Self::ServerContext => "server context",
            Self::Extensions => "extensions",
            Self::Streaming => "streaming",
            Self::SystemMessages => "system messages",
            Self::DeveloperMessages => "developer messages",
            Self::GenerationControls => "generation controls",
        }
    }
}

/// A small value, not an allocation or a hidden model registry.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct FeatureSet(u32);
impl FeatureSet {
    pub const fn empty() -> Self {
        Self(0)
    }
    pub const fn with(self, feature: Feature) -> Self {
        Self(self.0 | (1 << feature as u8))
    }
    pub const fn contains(self, feature: Feature) -> bool {
        self.0 & (1 << feature as u8) != 0
    }
    pub fn iter(self) -> impl Iterator<Item = Feature> {
        Feature::ALL.into_iter().filter(move |f| self.contains(*f))
    }
}

/// Evidence supplied by an operator or discovery service; the kernel never fetches it.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct Provenance<'a> {
    pub source: &'a str,
    pub revision: Option<&'a str>,
}
impl<'a> Provenance<'a> {
    pub const fn new(source: &'a str) -> Self {
        Self {
            source,
            revision: None,
        }
    }
}
#[derive(Clone, Copy, Debug)]
pub struct Declaration<'a> {
    pub feature: Feature,
    pub support: Support,
    pub provenance: Provenance<'a>,
    /// A stated restriction is evidence, but does not automatically satisfy it.
    pub condition: Option<&'a str>,
}
impl<'a> Declaration<'a> {
    pub const fn supported(feature: Feature, provenance: Provenance<'a>) -> Self {
        Self {
            feature,
            support: Support::Supported,
            provenance,
            condition: None,
        }
    }
}

/// Unknown capacities remain unknown; zero is an explicit zero limit.
#[derive(Clone, Copy, Debug, Default)]
pub enum TokenLimit<'a> {
    #[default]
    Unknown,
    Known {
        tokens: u64,
        provenance: Provenance<'a>,
    },
}
#[derive(Clone, Copy, Debug, Default)]
pub struct CapabilitySet<'a> {
    pub declarations: &'a [Declaration<'a>],
    pub context_window: TokenLimit<'a>,
    pub output_limit: TokenLimit<'a>,
    /// None means unknown; an empty slice explicitly declares no named levels.
    pub reasoning_levels: Option<&'a [&'a str]>,
    /// Attribution for the named-level list. If absent, the Reasoning feature's
    /// declaration supplies its provenance.
    pub reasoning_provenance: Option<Provenance<'a>>,
}
impl<'a> CapabilitySet<'a> {
    fn declaration(self, feature: Feature) -> Option<Declaration<'a>> {
        self.declarations
            .iter()
            .find(|d| d.feature == feature)
            .copied()
    }
    fn validate(self) -> Result<(), Error> {
        fn provenance(value: Provenance<'_>) -> Result<(), Error> {
            if value.source.trim().is_empty() {
                return Err(Error::invalid(
                    "capability provenance",
                    "evidence source must not be empty",
                ));
            }
            Ok(())
        }
        for (i, d) in self.declarations.iter().enumerate() {
            provenance(d.provenance)?;
            if d.condition
                .is_some_and(|condition| condition.trim().is_empty())
            {
                return Err(Error::invalid(
                    "capability condition",
                    "a stated condition must not be empty",
                ));
            }
            if self.declarations[..i]
                .iter()
                .any(|old| old.feature == d.feature)
            {
                return Err(Error::invalid(
                    "capability declarations",
                    "duplicate feature evidence",
                ));
            }
        }
        for limit in [self.context_window, self.output_limit] {
            if let TokenLimit::Known {
                provenance: evidence,
                ..
            } = limit
            {
                provenance(evidence)?;
            }
        }
        if let Some(evidence) = self.reasoning_provenance {
            provenance(evidence)?;
        }
        if let Some(levels) = self.reasoning_levels {
            let evidence = self
                .reasoning_provenance
                .or_else(|| self.declaration(Feature::Reasoning).map(|d| d.provenance));
            if evidence.is_none() {
                return Err(Error::invalid(
                    "reasoning level provenance",
                    "named levels require attributed caller evidence",
                ));
            }
            for (index, level) in levels.iter().enumerate() {
                if level.is_empty() || levels[..index].contains(level) {
                    return Err(Error::invalid(
                        "reasoning levels",
                        "empty or duplicate named level",
                    ));
                }
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, Default)]
pub struct Requirements<'a> {
    pub features: FeatureSet,
    /// Tokenization is caller-owned. Bytes and characters are not token counts.
    pub input_tokens: Option<u64>,
    pub max_output_tokens: Option<u64>,
    pub reasoning_level: Option<&'a str>,
}
#[derive(Clone, Copy, Debug)]
pub struct Endpoint<'a> {
    pub format: Format,
    pub model: CapabilitySet<'a>,
    pub deployment: CapabilitySet<'a>,
}

/// Unknown model/deployment evidence and unevaluated conditions require explicit acceptance.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CapabilityPolicy {
    pub accept_unknown_model: bool,
    pub accept_unknown_deployment: bool,
    pub accept_unverified_conditions: bool,
    pub accept_unknown_token_limits: bool,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Layer {
    Adapter,
    Protocol,
    Model,
    Deployment,
    Input,
}
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Requirement {
    Feature(Feature),
    ContextWindow,
    OutputLimit,
    ReasoningLevel,
    Representability,
}

/// Parsing one operation does not imply that the encoder or SSE adapter is wired.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AdapterOperation {
    DecodeRequest,
    EncodeRequest,
    DecodeResponse,
    EncodeResponse,
    DecodeStream,
    EncodeStream,
}

/// Every decision includes its layer, evidence and the exact accepted uncertainty.
#[derive(Clone, Copy, Debug)]
pub struct Diagnostic<'a> {
    pub requirement: Requirement,
    pub layer: Layer,
    pub support: Support,
    pub provenance: Provenance<'a>,
    pub condition: Option<&'a str>,
    pub accepted_uncertainty: bool,
    /// Only Request::prepare can establish an input-dependent adapter condition.
    /// Model/deployment conditions remain unverified unless the caller accepts them.
    pub condition_verified: bool,
}
impl Diagnostic<'_> {
    pub const EMPTY: Self = Self {
        requirement: Requirement::Feature(Feature::Text),
        layer: Layer::Input,
        support: Support::Unknown,
        provenance: Provenance::new("unspecified"),
        condition: None,
        accepted_uncertainty: false,
        condition_verified: false,
    };
}
#[derive(Clone, Copy, Debug)]
pub struct Assessment {
    /// A capacity error takes precedence; grow the report and repeat the check.
    pub outcome: Result<(), Error>,
    pub written: usize,
    pub needed: usize,
    pub accepted_uncertainties: usize,
    /// True only after the actual target encoder successfully checks this input.
    pub input_verified: bool,
}

/// Wire-level semantic families, independent of a particular model or adapter
/// operation. Supported means the format has a representation within its declared
/// profile; it does not promise that every value or shape is representable.
pub fn protocol_support(format: Format, feature: Feature) -> Support {
    use Feature as F;
    use Protocol as P;
    if format.validate().is_err() {
        return Support::Unsupported;
    }
    let yes = match feature {
        F::Text | F::GenerationControls | F::Streaming | F::Extensions => true,
        F::Tools | F::ToolChoice | F::SystemMessages | F::Image | F::File => {
            format.protocol != P::OpenAiCompletions
        }
        F::Audio => matches!(
            format.protocol,
            P::OpenAiChat | P::GeminiGenerateContent | P::GeminiInteractions
        ),
        F::Video => matches!(
            format.protocol,
            P::GeminiGenerateContent | P::GeminiInteractions
        ),
        F::ParallelTools => matches!(format.protocol, P::OpenAiChat | P::OpenAiResponses),
        F::Reasoning => {
            matches!(
                format.protocol,
                P::OpenAiResponses
                    | P::AnthropicMessages
                    | P::GeminiGenerateContent
                    | P::GeminiInteractions
            ) || matches!(
                (format.protocol, format.profile),
                (P::OpenAiChat, Profile::OpenAiCompatible(_))
            )
        }
        F::ReasoningSignature => matches!(
            format.protocol,
            P::AnthropicMessages | P::GeminiGenerateContent | P::GeminiInteractions
        ),
        F::Refusal => matches!(format.protocol, P::OpenAiChat | P::OpenAiResponses),
        F::StructuredOutput => matches!(
            format.protocol,
            P::OpenAiChat | P::OpenAiResponses | P::GeminiGenerateContent
        ),
        F::MultipleChoices => matches!(
            format.protocol,
            P::OpenAiChat | P::OpenAiCompletions | P::GeminiGenerateContent
        ),
        F::ServerContext => matches!(format.protocol, P::OpenAiResponses | P::GeminiInteractions),
        F::DeveloperMessages => matches!(format.protocol, P::OpenAiChat | P::OpenAiResponses),
    };
    if yes {
        Support::Supported
    } else {
        Support::Unsupported
    }
}

/// The request encoder's implemented semantic families. Use
/// `adapter_support_for` when inspecting decoding or streaming separately.
pub fn adapter_support(format: Format, feature: Feature) -> Support {
    adapter_support_for(format, AdapterOperation::EncodeRequest, feature)
}

/// Conservative coverage of the operation that will actually run. In particular,
/// a nonstream format version may be valid while its SSE adapter is unsupported.
pub fn adapter_support_for(
    format: Format,
    operation: AdapterOperation,
    feature: Feature,
) -> Support {
    use AdapterOperation as A;
    use Feature as F;
    if format.validate().is_err()
        || matches!(format.profile, Profile::OpenAiCompatible(name) if name.is_empty())
    {
        return Support::Unsupported;
    }
    let streaming = matches!(operation, A::DecodeStream | A::EncodeStream);
    if (streaming || feature == F::Streaming)
        && zconv::stream::validate_stream_format(format.raw()).is_err()
    {
        return Support::Unsupported;
    }
    if streaming
        && matches!(
            feature,
            F::Image
                | F::Audio
                | F::File
                | F::Video
                | F::ToolChoice
                | F::ParallelTools
                | F::StructuredOutput
                | F::ServerContext
                | F::Extensions
                | F::SystemMessages
                | F::DeveloperMessages
                | F::GenerationControls
        )
    {
        return Support::Unsupported;
    }
    if matches!(operation, A::DecodeResponse | A::EncodeResponse)
        && matches!(
            feature,
            F::ToolChoice
                | F::ParallelTools
                | F::StructuredOutput
                | F::ServerContext
                | F::Streaming
                | F::SystemMessages
                | F::DeveloperMessages
                | F::GenerationControls
        )
    {
        return Support::Unsupported;
    }
    protocol_support(format, feature)
}

fn adapter_condition(feature: Feature) -> Option<&'static str> {
    match feature {
        Feature::Image | Feature::Audio | Feature::File | Feature::Video => Some("the current media reference and encoding must be representable"),
        Feature::Reasoning | Feature::ReasoningSignature => Some("the current reasoning kind and signature namespace must match the target"),
        Feature::StructuredOutput => Some("the current output constraint must use a verified schema mapping"),
        Feature::ServerContext | Feature::Extensions => Some("source ownership must match the target or an allowed loss must be reported"),
        Feature::GenerationControls => Some("every explicit control, including null, must be encodable in this version"),
        Feature::Tools | Feature::ToolChoice | Feature::ParallelTools => Some("the actual tool contract and selection must be representable without changing strict defaults"),
        _ => None,
    }
}

fn requirement_name(requirement: Requirement) -> &'static str {
    match requirement {
        Requirement::Feature(f) => f.name(),
        Requirement::ContextWindow => "context window",
        Requirement::OutputLimit => "output limit",
        Requirement::ReasoningLevel => "reasoning level",
        Requirement::Representability => "input representability",
    }
}

/// Check declared requirements for the request encoder without I/O. The result
/// is explicitly input-unverified. Input-dependent conditions require acceptance
/// here; `Request::prepare` instead verifies them with the actual encoder.
pub fn check_capabilities<'a>(
    endpoint: Endpoint<'a>,
    requirements: Requirements<'_>,
    policy: CapabilityPolicy,
    output: &mut [Diagnostic<'a>],
) -> Assessment {
    check_capabilities_for(
        endpoint,
        AdapterOperation::EncodeRequest,
        requirements,
        policy,
        output,
    )
}

/// Inspect adapter operation, wire representation, model and deployment evidence
/// independently. This does not parse, encode or send any request.
pub fn check_capabilities_for<'a>(
    endpoint: Endpoint<'a>,
    operation: AdapterOperation,
    requirements: Requirements<'_>,
    policy: CapabilityPolicy,
    output: &mut [Diagnostic<'a>],
) -> Assessment {
    assess(endpoint, operation, requirements, policy, output, false)
}

fn assess<'a>(
    endpoint: Endpoint<'a>,
    operation: AdapterOperation,
    requirements: Requirements<'_>,
    policy: CapabilityPolicy,
    output: &mut [Diagnostic<'a>],
    verify_input: bool,
) -> Assessment {
    let mut result = Assessment {
        outcome: endpoint
            .format
            .validate()
            .and_then(|()| endpoint.model.validate())
            .and_then(|()| endpoint.deployment.validate()),
        written: 0,
        needed: 0,
        accepted_uncertainties: 0,
        input_verified: false,
    };
    if result.outcome.is_err() {
        return result;
    }
    if requirements.reasoning_level.is_some_and(str::is_empty) {
        result.outcome = Err(Error::invalid(
            "reasoning level",
            "the requested named level must not be empty",
        ));
        return result;
    }
    let mut emit = |mut diagnostic: Diagnostic<'a>, allow: bool| {
        let deferred =
            verify_input && diagnostic.layer == Layer::Adapter && diagnostic.condition.is_some();
        let uncertain =
            diagnostic.support == Support::Unknown || (diagnostic.condition.is_some() && !deferred);
        diagnostic.accepted_uncertainty =
            uncertain && allow && diagnostic.support != Support::Unsupported;
        if let Some(slot) = output.get_mut(result.needed) {
            *slot = diagnostic;
            result.written += 1;
        }
        result.needed += 1;
        if diagnostic.accepted_uncertainty {
            result.accepted_uncertainties += 1;
        }
        if result.outcome.is_ok()
            && (diagnostic.support == Support::Unsupported || (uncertain && !allow))
        {
            result.outcome = Err(Error::new(
                if diagnostic.support == Support::Unsupported {
                    ErrorKind::CapabilityMismatch
                } else {
                    ErrorKind::CapabilityUnknown
                },
                requirement_name(diagnostic.requirement),
                "inspect capability diagnostics for evidence, condition and layer",
            ));
        }
    };
    let features = if requirements.reasoning_level.is_some() {
        requirements.features.with(Feature::Reasoning)
    } else {
        requirements.features
    };
    for feature in features.iter() {
        emit(
            Diagnostic {
                requirement: Requirement::Feature(feature),
                layer: Layer::Adapter,
                support: adapter_support_for(endpoint.format, operation, feature),
                provenance: Provenance::new(match operation {
                    AdapterOperation::DecodeRequest => {
                        "zcall request import / zconv request decoder"
                    }
                    AdapterOperation::EncodeRequest => {
                        "zcall request export / zconv request encoder"
                    }
                    AdapterOperation::DecodeResponse => {
                        "zcall response import / zconv response decoder"
                    }
                    AdapterOperation::EncodeResponse => {
                        "zcall response export / zconv response encoder"
                    }
                    AdapterOperation::DecodeStream => "zcall session / zconv framed event decoder",
                    AdapterOperation::EncodeStream => "zcall event encoder / zconv event staging",
                }),
                condition: adapter_condition(feature),
                ..Diagnostic::EMPTY
            },
            policy.accept_unverified_conditions,
        );
        emit(Diagnostic {
            requirement: Requirement::Feature(feature), layer: Layer::Protocol,
            support: protocol_support(endpoint.format, feature),
            provenance: Provenance::new("declared format semantic representation, independent of model and adapter operation"),
            ..Diagnostic::EMPTY
        }, false);
        for (layer, caps, unknown_allowed) in [
            (Layer::Model, endpoint.model, policy.accept_unknown_model),
            (
                Layer::Deployment,
                endpoint.deployment,
                policy.accept_unknown_deployment,
            ),
        ] {
            let declaration = caps.declaration(feature);
            let support = declaration.map_or(Support::Unknown, |value| value.support);
            let condition = declaration.and_then(|value| value.condition);
            let allow = (support != Support::Unknown || unknown_allowed)
                && (condition.is_none() || policy.accept_unverified_conditions);
            emit(
                Diagnostic {
                    requirement: Requirement::Feature(feature),
                    layer,
                    support,
                    provenance: declaration
                        .map_or(Provenance::new("no caller feature evidence"), |value| {
                            value.provenance
                        }),
                    condition,
                    ..Diagnostic::EMPTY
                },
                allow,
            );
        }
    }
    let token_check =
        requirements.input_tokens.is_some() || requirements.max_output_tokens.is_some();
    let total = match (requirements.input_tokens, requirements.max_output_tokens) {
        (Some(input), Some(output)) => input.checked_add(output),
        _ => None,
    };
    let overflow = requirements.input_tokens.is_some()
        && requirements.max_output_tokens.is_some()
        && total.is_none();
    for (layer, caps) in [
        (Layer::Model, endpoint.model),
        (Layer::Deployment, endpoint.deployment),
    ] {
        if let Some(requested) = requirements.max_output_tokens {
            let (support, provenance) = match caps.output_limit {
                TokenLimit::Unknown => (
                    Support::Unknown,
                    Provenance::new("no caller limit evidence"),
                ),
                TokenLimit::Known { tokens, provenance } => (
                    if requested <= tokens {
                        Support::Supported
                    } else {
                        Support::Unsupported
                    },
                    provenance,
                ),
            };
            emit(
                Diagnostic {
                    requirement: Requirement::OutputLimit,
                    layer,
                    support,
                    provenance,
                    ..Diagnostic::EMPTY
                },
                policy.accept_unknown_token_limits,
            );
        }
        if token_check {
            let (support, provenance) = match (total, caps.context_window) {
                (_, _) if overflow => (
                    Support::Unsupported,
                    Provenance::new("caller token count overflow"),
                ),
                (_, TokenLimit::Unknown) => (
                    Support::Unknown,
                    Provenance::new("no caller limit evidence"),
                ),
                (None, TokenLimit::Known { provenance, .. }) => (Support::Unknown, provenance),
                (Some(n), TokenLimit::Known { tokens, provenance }) => (
                    if n <= tokens {
                        Support::Supported
                    } else {
                        Support::Unsupported
                    },
                    provenance,
                ),
            };
            emit(
                Diagnostic {
                    requirement: Requirement::ContextWindow,
                    layer,
                    support,
                    provenance,
                    ..Diagnostic::EMPTY
                },
                policy.accept_unknown_token_limits,
            );
        }
        if let Some(level) = requirements.reasoning_level {
            emit(
                Diagnostic {
                    requirement: Requirement::ReasoningLevel,
                    layer,
                    support: caps.reasoning_levels.map_or(Support::Unknown, |levels| {
                        if levels.contains(&level) {
                            Support::Supported
                        } else {
                            Support::Unsupported
                        }
                    }),
                    provenance: caps
                        .reasoning_provenance
                        .or_else(|| caps.declaration(Feature::Reasoning).map(|d| d.provenance))
                        .unwrap_or(Provenance::new("no caller reasoning level evidence")),
                    ..Diagnostic::EMPTY
                },
                match layer {
                    Layer::Model => policy.accept_unknown_model,
                    _ => policy.accept_unknown_deployment,
                },
            );
        }
    }
    if token_check && total.is_none() {
        emit(
            Diagnostic {
                requirement: Requirement::ContextWindow,
                layer: Layer::Input,
                support: if overflow {
                    Support::Unsupported
                } else {
                    Support::Unknown
                },
                provenance: Provenance::new("caller-provided exact token counts"),
                condition: Some(if overflow {
                    "input plus output token budget exceeds u64"
                } else if requirements.input_tokens.is_none() {
                    "input token count was not supplied"
                } else {
                    "maximum output token count was not supplied; absence is not zero"
                }),
                ..Diagnostic::EMPTY
            },
            policy.accept_unknown_token_limits,
        );
    }
    if result.needed > output.len() {
        result.outcome = Err(Error::capacity(
            Resource::Reports,
            result.needed,
            true,
            Replay::Unconsumed,
        ));
    }
    result
}

/// Additional caller facts used during preparation. No tokenizer or discovery
/// call runs implicitly; absent values do not claim a context-size check.
#[derive(Clone, Copy, Debug, Default)]
pub struct CallRequirements<'a> {
    pub input_tokens: Option<u64>,
    pub reasoning_level: Option<&'a str>,
}

#[derive(Clone, Copy, Debug)]
pub struct Prepared<'a> {
    pub encoding: EncodeReport<'a>,
    pub capabilities: Assessment,
}

impl<'a> Request<'a, '_> {
    /// Derive feature requirements from the actual IR, including nested content.
    pub fn requirements(&self) -> Requirements<'_> {
        let mut features = FeatureSet::empty();
        for message in self.messages() {
            features = match message.role() {
                Role::System => features.with(Feature::SystemMessages),
                Role::Developer => features.with(Feature::DeveloperMessages),
                _ => features,
            };
            for block in message.contents() {
                features = match block.content() {
                    Content::Text(_) => features.with(Feature::Text),
                    Content::Media(media) => features.with(match media.kind {
                        MediaKind::Image => Feature::Image,
                        MediaKind::Audio => Feature::Audio,
                        MediaKind::File => Feature::File,
                        MediaKind::Video => Feature::Video,
                    }),
                    Content::ToolCall { .. } | Content::ToolResult { .. } => {
                        features.with(Feature::Tools)
                    }
                    Content::Reasoning { signature, .. } => {
                        let f = features.with(Feature::Reasoning);
                        if signature.is_missing() {
                            f
                        } else {
                            f.with(Feature::ReasoningSignature)
                        }
                    }
                    Content::Refusal(_) => features.with(Feature::Refusal),
                    Content::Opaque { .. } => features.with(Feature::Extensions),
                };
            }
        }
        if !self.tool_presence().is_missing() || self.tools().next().is_some() {
            features = features.with(Feature::Tools);
        }
        if !matches!(self.tool_choice(), ToolChoice::Missing) {
            features = features.with(Feature::ToolChoice);
        }
        if !self.output_constraint().is_missing() {
            features = features.with(Feature::StructuredOutput);
        }
        if !self.context().is_missing() {
            features = features.with(Feature::ServerContext);
        }
        if self.extensions().next().is_some() {
            features = features.with(Feature::Extensions);
        }
        if self
            .parameter(Parameter::Stream)
            .value()
            .and_then(Json::as_bool)
            == Some(true)
        {
            features = features.with(Feature::Streaming);
        }
        if !self.parameter(Parameter::ParallelToolCalls).is_missing() {
            features = features.with(Feature::ParallelTools);
        }
        if self
            .parameter(Parameter::Candidates)
            .value()
            .and_then(Json::as_u64)
            .is_some_and(|n| n > 1)
        {
            features = features.with(Feature::MultipleChoices);
        }
        if [
            Parameter::Temperature,
            Parameter::TopP,
            Parameter::TopK,
            Parameter::MaxOutputTokens,
            Parameter::Stop,
            Parameter::Seed,
            Parameter::FrequencyPenalty,
            Parameter::PresencePenalty,
            Parameter::Candidates,
        ]
        .iter()
        .any(|&parameter| !self.parameter(parameter).is_missing())
        {
            features = features.with(Feature::GenerationControls);
        }
        let max_output_tokens = self
            .parameter(Parameter::MaxOutputTokens)
            .value()
            .and_then(Json::as_u64);
        Requirements {
            features,
            input_tokens: None,
            max_output_tokens,
            reasoning_level: None,
        }
    }

    /// Validate capabilities, then encode the actual IR and check representability.
    /// No request bytes may be sent until this operation succeeds. The output
    /// buffer is caller-owned and its contents on an error are uncommitted.
    #[allow(clippy::too_many_arguments)]
    pub fn prepare<'e, const T: usize>(
        &mut self,
        endpoint: Endpoint<'e>,
        facts: CallRequirements<'_>,
        capability_policy: CapabilityPolicy,
        diagnostics: &mut [Diagnostic<'e>],
        output: &mut [u8],
        scratch: &mut [u8],
        tokens: &mut TokenStorage<T>,
        context: EncodeContext<'a>,
        loss_policy: Policy,
    ) -> Result<Prepared<'a>, Error> {
        let inferred = self.requirements();
        let mut features = inferred.features;
        if loss_policy.drop_reasoning {
            features.0 &=
                !((1 << Feature::Reasoning as u8) | (1 << Feature::ReasoningSignature as u8));
        }
        if loss_policy.drop_context
            && match self.context() {
                Presence::Value(reference) => reference.source != endpoint.format,
                Presence::Null => !matches!(
                    endpoint.format.protocol,
                    Protocol::OpenAiResponses | Protocol::GeminiInteractions
                ),
                Presence::Missing => true,
            }
        {
            features.0 &= !(1 << Feature::ServerContext as u8);
        }
        if loss_policy.drop_extensions
            && !self
                .extensions()
                .any(|extension| extension.source == endpoint.format)
            && !self.messages().any(|message| {
                message
                    .contents()
                    .any(|block| matches!(block.content(), Content::Opaque { .. }))
            })
        {
            features.0 &= !(1 << Feature::Extensions as u8);
        }
        let max_output_tokens = if endpoint.format.protocol == Protocol::AnthropicMessages
            && self.parameter(Parameter::MaxOutputTokens).is_missing()
        {
            let supplied = context.max_tokens();
            if supplied.is_some() {
                features = features.with(Feature::GenerationControls);
            }
            supplied
        } else {
            inferred.max_output_tokens
        };
        let requirements = Requirements {
            features,
            input_tokens: facts.input_tokens,
            max_output_tokens,
            reasoning_level: facts.reasoning_level,
        };
        let mut assessment = assess(
            endpoint,
            AdapterOperation::EncodeRequest,
            requirements,
            capability_policy,
            diagnostics,
            true,
        );
        let input_slot = assessment.needed;
        assessment.needed += 1;
        if assessment.needed > diagnostics.len() {
            return Err(Error::capacity(
                Resource::Reports,
                assessment.needed,
                true,
                Replay::Unconsumed,
            ));
        }
        diagnostics[input_slot] = Diagnostic {
            requirement: Requirement::Representability,
            layer: Layer::Input,
            support: Support::Unknown,
            provenance: Provenance::new("actual zconv IR request encoder"),
            condition: Some("encoding has not run because capability checking must succeed first"),
            ..Diagnostic::EMPTY
        };
        assessment.written += 1;
        assessment.outcome?;
        // This is the actual target encoder: it checks required context, strict
        // defaults, namespaced references, current media shapes and exact losses.
        let encoded = self.encode(
            endpoint.format,
            output,
            scratch,
            tokens,
            context,
            loss_policy,
        );
        diagnostics[input_slot] = Diagnostic {
            requirement: Requirement::Representability,
            layer: Layer::Input,
            support: match encoded {
                Ok(_) => Support::Supported,
                Err(error) if error.kind == ErrorKind::Capacity => Support::Unknown,
                Err(_) => Support::Unsupported,
            },
            provenance: Provenance::new("actual zconv IR request encoder"),
            condition: encoded.as_ref().err().map(|error| error.detail),
            condition_verified: encoded.is_ok(),
            ..Diagnostic::EMPTY
        };
        let encoding = encoded?;
        for diagnostic in &mut diagnostics[..input_slot] {
            if diagnostic.layer == Layer::Adapter && diagnostic.condition.is_some() {
                diagnostic.condition_verified = true;
                diagnostic.accepted_uncertainty = false;
            }
        }
        assessment.input_verified = true;
        assessment.outcome = Ok(());
        Ok(Prepared {
            encoding,
            capabilities: assessment,
        })
    }
}
