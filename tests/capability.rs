use zcall::v1::*;

const PROVENANCE: Provenance<'static> = Provenance::new("test model and deployment manifest");
const TEXT: [Declaration<'static>; 1] = [Declaration::supported(Feature::Text, PROVENANCE)];

fn endpoint<'a>(model: &'a [Declaration<'a>], deployment: &'a [Declaration<'a>]) -> Endpoint<'a> {
    Endpoint {
        format: Protocol::OpenAiChat.into(),
        model: CapabilitySet {
            declarations: model,
            ..Default::default()
        },
        deployment: CapabilitySet {
            declarations: deployment,
            ..Default::default()
        },
    }
}
#[test]
fn unknown_is_never_implicit_support() {
    let mut report = [Diagnostic::EMPTY; 16];
    let requirements = Requirements {
        features: FeatureSet::empty().with(Feature::Image),
        ..Default::default()
    };
    let assessment = check_capabilities(
        endpoint(&[], &[]),
        requirements,
        CapabilityPolicy::default(),
        &mut report,
    );
    assert_eq!(
        assessment.outcome.unwrap_err().kind,
        ErrorKind::CapabilityUnknown
    );
    assert_eq!(report[0].support, Support::Supported);
    assert_eq!(report[2].support, Support::Unknown);
    assert_eq!(report[2].layer, Layer::Model);
    assert_eq!(report[3].layer, Layer::Deployment);
}
#[test]
fn explicit_uncertainty_is_reported_per_layer() {
    let mut report = [Diagnostic::EMPTY; 8];
    let requirements = Requirements {
        features: FeatureSet::empty().with(Feature::Text),
        ..Default::default()
    };
    let assessment = check_capabilities(
        endpoint(&[], &[]),
        requirements,
        CapabilityPolicy {
            accept_unknown_model: true,
            accept_unknown_deployment: true,
            ..Default::default()
        },
        &mut report,
    );
    assert!(assessment.outcome.is_ok());
    assert_eq!(assessment.accepted_uncertainties, 2);
    assert!(report[2].accepted_uncertainty && report[3].accepted_uncertainty);
}
#[test]
fn zero_report_recovers_and_needed_is_exact() {
    let r = Requirements {
        features: FeatureSet::empty().with(Feature::Text),
        ..Default::default()
    };
    let a = check_capabilities(
        endpoint(&TEXT, &TEXT),
        r,
        CapabilityPolicy::default(),
        &mut [],
    );
    let c = a.outcome.unwrap_err().capacity.unwrap();
    assert_eq!((c.region, c.needed, c.exact), (Resource::Reports, 4, true));
    let mut out = [Diagnostic::EMPTY; 4];
    assert!(check_capabilities(
        endpoint(&TEXT, &TEXT),
        r,
        CapabilityPolicy::default(),
        &mut out
    )
    .outcome
    .is_ok());
}
#[test]
fn explicit_zero_limit_and_overflow_are_rejected() {
    let mut e = endpoint(&TEXT, &TEXT);
    e.model.output_limit = TokenLimit::Known {
        tokens: 0,
        provenance: PROVENANCE,
    };
    let mut report = [Diagnostic::EMPTY; 16];
    let a = check_capabilities(
        e,
        Requirements {
            max_output_tokens: Some(1),
            ..Default::default()
        },
        CapabilityPolicy::default(),
        &mut report,
    );
    assert_eq!(a.outcome.unwrap_err().kind, ErrorKind::CapabilityMismatch);
    e.model.context_window = TokenLimit::Known {
        tokens: u64::MAX,
        provenance: PROVENANCE,
    };
    e.model.output_limit = TokenLimit::Known {
        tokens: u64::MAX,
        provenance: PROVENANCE,
    };
    let a = check_capabilities(
        e,
        Requirements {
            input_tokens: Some(u64::MAX),
            max_output_tokens: Some(1),
            ..Default::default()
        },
        CapabilityPolicy::default(),
        &mut report,
    );
    assert_eq!(a.outcome.unwrap_err().kind, ErrorKind::CapabilityMismatch);
}
#[test]
fn conditional_evidence_needs_explicit_acceptance() {
    let claims = [Declaration {
        feature: Feature::Text,
        support: Support::Supported,
        provenance: PROVENANCE,
        condition: Some("deployment entitlement required"),
    }];
    let mut report = [Diagnostic::EMPTY; 4];
    let req = Requirements {
        features: FeatureSet::empty().with(Feature::Text),
        ..Default::default()
    };
    assert_eq!(
        check_capabilities(
            endpoint(&claims, &TEXT),
            req,
            CapabilityPolicy::default(),
            &mut report
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::CapabilityUnknown
    );
    assert!(check_capabilities(
        endpoint(&claims, &TEXT),
        req,
        CapabilityPolicy {
            accept_unverified_conditions: true,
            ..Default::default()
        },
        &mut report
    )
    .outcome
    .is_ok());
}
#[test]
fn unknown_protocol_versions_and_duplicate_evidence_fail() {
    let mut e = endpoint(&TEXT, &TEXT);
    e.format.version = "unknown";
    assert_eq!(
        check_capabilities(
            e,
            Requirements::default(),
            CapabilityPolicy::default(),
            &mut []
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::Unsupported
    );
    let duplicates = [TEXT[0], TEXT[0]];
    assert_eq!(
        check_capabilities(
            endpoint(&duplicates, &TEXT),
            Requirements::default(),
            CapabilityPolicy::default(),
            &mut []
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::InvalidInput
    );
}
#[test]
fn prepare_checks_actual_request_before_send() {
    let mut records = RecordStorage::<8>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("model")),
        &mut records,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::Text(Text::new("hello"))])
        .unwrap();
    let mut request = builder.finish();
    let mut diagnostics = [Diagnostic::EMPTY; 8];
    let mut output = [0; 512];
    let mut tokens = TokenStorage::<32>::new();
    let result = request
        .prepare(
            endpoint(&TEXT, &TEXT),
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut diagnostics,
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    assert!(result.encoding.written > 0);
    assert_eq!(result.capabilities.accepted_uncertainties, 0);
    assert!(result.capabilities.input_verified);
}

#[test]
fn protocol_expression_does_not_claim_an_unwired_stream_adapter() {
    let format = Format {
        protocol: Protocol::AnthropicMessages,
        version: "2026-10-21",
        profile: Profile::Standard,
    };
    assert_eq!(
        protocol_support(format, Feature::Streaming),
        Support::Supported
    );
    assert_eq!(
        adapter_support_for(format, AdapterOperation::DecodeStream, Feature::Streaming),
        Support::Unsupported
    );
    assert_eq!(
        adapter_support_for(
            Protocol::GeminiGenerateContent.into(),
            AdapterOperation::DecodeRequest,
            Feature::Image
        ),
        Support::Supported
    );
    assert_eq!(
        adapter_support_for(
            Protocol::GeminiGenerateContent.into(),
            AdapterOperation::DecodeStream,
            Feature::Image
        ),
        Support::Unsupported
    );
    let declarations = [Declaration::supported(Feature::Streaming, PROVENANCE)];
    let mut e = endpoint(&declarations, &declarations);
    e.format = format;
    let mut diagnostics = [Diagnostic::EMPTY; 8];
    let result = check_capabilities(
        e,
        Requirements {
            features: FeatureSet::empty().with(Feature::Streaming),
            ..Default::default()
        },
        CapabilityPolicy::default(),
        &mut diagnostics,
    );
    assert_eq!(
        result.outcome.unwrap_err().kind,
        ErrorKind::CapabilityMismatch
    );
    assert_eq!(
        (diagnostics[0].layer, diagnostics[0].support),
        (Layer::Adapter, Support::Unsupported)
    );
    assert_eq!(
        (diagnostics[1].layer, diagnostics[1].support),
        (Layer::Protocol, Support::Supported)
    );
}

#[test]
fn request_and_response_adapter_operations_are_independent() {
    let format = Protocol::OpenAiChat.into();
    assert_eq!(
        adapter_support_for(format, AdapterOperation::EncodeRequest, Feature::ToolChoice),
        Support::Supported
    );
    assert_eq!(
        adapter_support_for(
            format,
            AdapterOperation::EncodeResponse,
            Feature::ToolChoice
        ),
        Support::Unsupported
    );
    let declaration = [Declaration::supported(Feature::ToolChoice, PROVENANCE)];
    let mut report = [Diagnostic::EMPTY; 8];
    let result = check_capabilities_for(
        endpoint(&declaration, &declaration),
        AdapterOperation::DecodeResponse,
        Requirements {
            features: FeatureSet::empty().with(Feature::ToolChoice),
            ..Default::default()
        },
        CapabilityPolicy {
            accept_unverified_conditions: true,
            ..Default::default()
        },
        &mut report,
    );
    assert_eq!(
        result.outcome.unwrap_err().kind,
        ErrorKind::CapabilityMismatch
    );
}

#[test]
fn standalone_adapter_conditions_are_explicit_then_prepare_verifies_the_shape() {
    let image = [Declaration::supported(Feature::Image, PROVENANCE)];
    let e = endpoint(&image, &image);
    let requirements = Requirements {
        features: FeatureSet::empty().with(Feature::Image),
        ..Default::default()
    };
    let mut report = [Diagnostic::EMPTY; 8];
    let strict = check_capabilities(e, requirements, CapabilityPolicy::default(), &mut report);
    assert_eq!(
        strict.outcome.unwrap_err().kind,
        ErrorKind::CapabilityUnknown
    );
    let accepted = check_capabilities(
        e,
        requirements,
        CapabilityPolicy {
            accept_unverified_conditions: true,
            ..Default::default()
        },
        &mut report,
    );
    assert!(accepted.outcome.is_ok());
    assert!(!accepted.input_verified);
    assert_eq!(accepted.accepted_uncertainties, 1);
    let mut records = RecordStorage::<8>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(
            Role::User,
            &[Content::Media(Media {
                kind: MediaKind::Image,
                mime_type: Presence::Missing,
                source: MediaSource::Url(Text::new("https://example.invalid/image.png")),
                detail: Presence::Missing,
                filename: Presence::Missing,
            })],
        )
        .unwrap();
    let mut request = builder.finish();
    let mut output = [0; 512];
    let mut tokens = TokenStorage::<32>::new();
    let prepared = request
        .prepare(
            e,
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut report,
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    assert!(prepared.capabilities.input_verified);
    assert_eq!(prepared.capabilities.accepted_uncertainties, 0);
    assert!(report[0].condition_verified);
    assert_eq!(report[4].layer, Layer::Input);
    assert_eq!(report[4].support, Support::Supported);
}

#[test]
fn every_explicit_generation_control_including_null_is_required() {
    let mut records = RecordStorage::<4>::new();
    let mut request = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap()
    .finish();
    for parameter in [
        Parameter::Temperature,
        Parameter::TopP,
        Parameter::TopK,
        Parameter::MaxOutputTokens,
        Parameter::Stop,
        Parameter::Seed,
        Parameter::FrequencyPenalty,
        Parameter::PresencePenalty,
        Parameter::Candidates,
    ] {
        request.set_parameter(parameter, Presence::Null).unwrap();
        assert!(request
            .requirements()
            .features
            .contains(Feature::GenerationControls));
        assert_eq!(request.requirements().max_output_tokens, None);
        request.set_parameter(parameter, Presence::Missing).unwrap();
        assert!(!request
            .requirements()
            .features
            .contains(Feature::GenerationControls));
    }
    let mut tokens = TokenStorage::<4>::new();
    let zero = Json::parse(b"0", &mut tokens, Limits::default()).unwrap();
    request
        .set_parameter(Parameter::MaxOutputTokens, Presence::Value(zero))
        .unwrap();
    assert_eq!(request.requirements().max_output_tokens, Some(0));
    request
        .set_parameter(Parameter::MaxOutputTokens, Presence::Missing)
        .unwrap();
    let disabled = Json::parse(b"false", &mut tokens, Limits::default()).unwrap();
    request
        .set_parameter(Parameter::ParallelToolCalls, Presence::Value(disabled))
        .unwrap();
    assert!(request
        .requirements()
        .features
        .contains(Feature::ParallelTools));
    request.set_tool_choice(ToolChoice::Auto);
    assert!(request
        .requirements()
        .features
        .contains(Feature::ToolChoice));
}

#[test]
fn actual_parameter_rejection_is_an_input_diagnostic() {
    let declarations = [
        TEXT[0],
        Declaration::supported(Feature::GenerationControls, PROVENANCE),
    ];
    let mut e = endpoint(&declarations, &declarations);
    e.format = Protocol::GeminiInteractions.into();
    let mut tokens = TokenStorage::<32>::new();
    let temperature = Json::parse(b"0.5", &mut tokens, Limits::default()).unwrap();
    let mut records = RecordStorage::<4>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::Text(Text::new("hi"))])
        .unwrap();
    builder
        .parameter(Parameter::Temperature, Presence::Value(temperature))
        .unwrap();
    let mut request = builder.finish();
    let mut report = [Diagnostic::EMPTY; 16];
    let error = request
        .prepare(
            e,
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert_eq!(report[8].requirement, Requirement::Representability);
    assert_eq!(report[8].support, Support::Unsupported);
    assert!(!report[8].condition_verified);
}

#[test]
fn missing_output_count_is_unknown_and_never_assumed_zero() {
    let mut e = endpoint(&[], &[]);
    for caps in [&mut e.model, &mut e.deployment] {
        caps.context_window = TokenLimit::Known {
            tokens: 128,
            provenance: PROVENANCE,
        };
    }
    let requirements = Requirements {
        input_tokens: Some(128),
        ..Default::default()
    };
    let mut report = [Diagnostic::EMPTY; 8];
    let strict = check_capabilities(e, requirements, CapabilityPolicy::default(), &mut report);
    assert_eq!(
        strict.outcome.unwrap_err().kind,
        ErrorKind::CapabilityUnknown
    );
    assert_eq!(report[2].layer, Layer::Input);
    assert_eq!(report[2].support, Support::Unknown);
    let allowed = check_capabilities(
        e,
        requirements,
        CapabilityPolicy {
            accept_unknown_token_limits: true,
            ..Default::default()
        },
        &mut report,
    );
    assert!(allowed.outcome.is_ok());
    assert_eq!(allowed.accepted_uncertainties, 3);
}

#[test]
fn explicit_zero_and_large_integer_limits_are_compared_exactly() {
    let mut e = endpoint(&[], &[]);
    let mut report = [Diagnostic::EMPTY; 8];
    for limit in [0, 9_007_199_254_740_993] {
        for caps in [&mut e.model, &mut e.deployment] {
            caps.context_window = TokenLimit::Known {
                tokens: limit,
                provenance: PROVENANCE,
            };
            caps.output_limit = TokenLimit::Known {
                tokens: limit,
                provenance: PROVENANCE,
            };
        }
        assert!(check_capabilities(
            e,
            Requirements {
                input_tokens: Some(0),
                max_output_tokens: Some(limit),
                ..Default::default()
            },
            CapabilityPolicy::default(),
            &mut report
        )
        .outcome
        .is_ok());
        assert_eq!(
            check_capabilities(
                e,
                Requirements {
                    input_tokens: Some(0),
                    max_output_tokens: Some(limit + 1),
                    ..Default::default()
                },
                CapabilityPolicy::default(),
                &mut report
            )
            .outcome
            .unwrap_err()
            .kind,
            ErrorKind::CapabilityMismatch
        );
    }
}

#[test]
fn context_supplied_output_budget_cannot_bypass_model_limits() {
    let declarations = [
        TEXT[0],
        Declaration::supported(Feature::GenerationControls, PROVENANCE),
    ];
    let mut e = endpoint(&declarations, &declarations);
    e.format = Protocol::AnthropicMessages.into();
    for caps in [&mut e.model, &mut e.deployment] {
        caps.context_window = TokenLimit::Known {
            tokens: 128,
            provenance: PROVENANCE,
        };
        caps.output_limit = TokenLimit::Known {
            tokens: 10,
            provenance: PROVENANCE,
        };
    }
    let mut records = RecordStorage::<4>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::Text(Text::new("hi"))])
        .unwrap();
    let mut request = builder.finish();
    let mut report = [Diagnostic::EMPTY; 16];
    let mut tokens = TokenStorage::<32>::new();
    let error = request
        .prepare(
            e,
            CallRequirements {
                input_tokens: Some(2),
                ..Default::default()
            },
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(11),
            Policy::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::CapabilityMismatch);
    assert_eq!(error.field, "output limit");
    let okay = request
        .prepare(
            e,
            CallRequirements {
                input_tokens: Some(2),
                ..Default::default()
            },
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(10),
            Policy::default(),
        )
        .unwrap();
    assert!(okay.capabilities.input_verified);
}

#[test]
fn requested_budget_overrides_context_and_null_does_not_use_context() {
    let declarations = [
        TEXT[0],
        Declaration::supported(Feature::GenerationControls, PROVENANCE),
    ];
    let mut e = endpoint(&declarations, &declarations);
    e.format = Protocol::AnthropicMessages.into();
    for caps in [&mut e.model, &mut e.deployment] {
        caps.context_window = TokenLimit::Known {
            tokens: 128,
            provenance: PROVENANCE,
        };
        caps.output_limit = TokenLimit::Known {
            tokens: 1,
            provenance: PROVENANCE,
        };
    }
    let mut records = RecordStorage::<4>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::Text(Text::new("hi"))])
        .unwrap();
    let mut request = builder.finish();
    let mut tokens = TokenStorage::<32>::new();
    let one = Json::parse(b"1", &mut tokens, Limits::default()).unwrap();
    request
        .set_parameter(Parameter::MaxOutputTokens, Presence::Value(one))
        .unwrap();
    let mut report = [Diagnostic::EMPTY; 16];
    assert!(request
        .prepare(
            e,
            CallRequirements {
                input_tokens: Some(2),
                ..Default::default()
            },
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(999),
            Policy::default()
        )
        .is_ok());
    request
        .set_parameter(Parameter::MaxOutputTokens, Presence::Null)
        .unwrap();
    let error = request
        .prepare(
            e,
            CallRequirements {
                input_tokens: Some(2),
                ..Default::default()
            },
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(1),
            Policy::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::CapabilityUnknown);
    assert_eq!(error.field, "context window");
}

#[test]
fn a_loss_permission_does_not_remove_same_source_extension_requirements() {
    let claims = [
        TEXT[0],
        Declaration {
            feature: Feature::Extensions,
            support: Support::Unsupported,
            provenance: PROVENANCE,
            condition: None,
        },
    ];
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<32>::new();
    let input =
        br#"{"model":"m","messages":[{"role":"user","content":"hi"}],"provider_flag":true}"#;
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let mut report = [Diagnostic::EMPTY; 16];
    let policy = Policy {
        drop_extensions: true,
        ..Default::default()
    };
    let error = request
        .prepare(
            endpoint(&claims, &claims),
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            policy,
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::CapabilityMismatch);
    let mut target = endpoint(&TEXT, &TEXT);
    target.format = Protocol::GeminiGenerateContent.into();
    let prepared = request
        .prepare(
            target,
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut report,
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            policy,
        )
        .unwrap();
    assert_eq!(prepared.encoding.dropped_extensions, 1);
}

#[test]
fn preparation_reserves_an_input_report_before_encoding() {
    let mut records = RecordStorage::<4>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::Text(Text::new("hi"))])
        .unwrap();
    let mut request = builder.finish();
    let mut tokens = TokenStorage::<32>::new();
    let mut output = [0xa5; 512];
    let capacity = request
        .prepare(
            endpoint(&TEXT, &TEXT),
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut [],
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap_err()
        .capacity
        .unwrap();
    assert_eq!(
        (capacity.region, capacity.needed, capacity.exact),
        (Resource::Reports, 5, true)
    );
    assert!(output.iter().all(|byte| *byte == 0xa5));
}

#[test]
fn limits_and_named_levels_require_attributed_evidence() {
    let mut e = endpoint(&[], &[]);
    e.model.output_limit = TokenLimit::Known {
        tokens: 1,
        provenance: Provenance::new(""),
    };
    assert_eq!(
        check_capabilities(
            e,
            Requirements::default(),
            CapabilityPolicy::default(),
            &mut []
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::InvalidInput
    );
    e.model.output_limit = TokenLimit::Unknown;
    e.model.reasoning_levels = Some(&["low"]);
    assert_eq!(
        check_capabilities(
            e,
            Requirements::default(),
            CapabilityPolicy::default(),
            &mut []
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::InvalidInput
    );
    e.model.reasoning_provenance = Some(PROVENANCE);
    assert!(check_capabilities(
        e,
        Requirements::default(),
        CapabilityPolicy::default(),
        &mut []
    )
    .outcome
    .is_ok());
}

#[test]
fn named_reasoning_requires_the_feature_and_empty_known_levels_are_unsupported() {
    let reasoning = [Declaration::supported(Feature::Reasoning, PROVENANCE)];
    let mut e = endpoint(&reasoning, &reasoning);
    e.format = Protocol::OpenAiResponses.into();
    e.model.reasoning_levels = Some(&[]);
    e.deployment.reasoning_levels = Some(&["low"]);
    let mut report = [Diagnostic::EMPTY; 16];
    let result = check_capabilities(
        e,
        Requirements {
            reasoning_level: Some("low"),
            ..Default::default()
        },
        CapabilityPolicy {
            accept_unverified_conditions: true,
            accept_unknown_model: true,
            accept_unknown_deployment: true,
            ..Default::default()
        },
        &mut report,
    );
    assert_eq!(
        result.outcome.unwrap_err().kind,
        ErrorKind::CapabilityMismatch
    );
    assert_eq!(
        report[0].requirement,
        Requirement::Feature(Feature::Reasoning)
    );
    assert_eq!(report[4].requirement, Requirement::ReasoningLevel);
    assert_eq!(report[4].support, Support::Unsupported);
}

#[test]
fn accepting_a_condition_does_not_override_unknown_or_unsupported_evidence() {
    let claims = [Declaration {
        feature: Feature::Text,
        support: Support::Unknown,
        provenance: PROVENANCE,
        condition: Some("caller must verify endpoint entitlement"),
    }];
    let requirements = Requirements {
        features: FeatureSet::empty().with(Feature::Text),
        ..Default::default()
    };
    let mut report = [Diagnostic::EMPTY; 8];
    let conditional = CapabilityPolicy {
        accept_unverified_conditions: true,
        ..Default::default()
    };
    assert_eq!(
        check_capabilities(
            endpoint(&claims, &TEXT),
            requirements,
            conditional,
            &mut report
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::CapabilityUnknown
    );
    let both = CapabilityPolicy {
        accept_unknown_model: true,
        ..conditional
    };
    assert!(
        check_capabilities(endpoint(&claims, &TEXT), requirements, both, &mut report)
            .outcome
            .is_ok()
    );
    assert_eq!(report[2].support, Support::Unknown);
    assert!(report[2].accepted_uncertainty);
    let unsupported = [Declaration {
        support: Support::Unsupported,
        ..claims[0]
    }];
    assert_eq!(
        check_capabilities(
            endpoint(&unsupported, &TEXT),
            requirements,
            both,
            &mut report
        )
        .outcome
        .unwrap_err()
        .kind,
        ErrorKind::CapabilityMismatch
    );
    assert!(!report[2].accepted_uncertainty);
}

#[test]
fn a_context_loss_permission_does_not_remove_a_retained_context_requirement() {
    let claims = [
        TEXT[0],
        Declaration {
            feature: Feature::ServerContext,
            support: Support::Unsupported,
            provenance: PROVENANCE,
            condition: None,
        },
    ];
    let mut e = endpoint(&claims, &claims);
    e.format = Protocol::OpenAiResponses.into();
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<32>::new();
    let input = br#"{"model":"m","input":"hi","previous_response_id":"stored-history"}"#;
    let mut request = Request::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let error = request
        .prepare(
            e,
            CallRequirements::default(),
            CapabilityPolicy::default(),
            &mut [Diagnostic::EMPTY; 16],
            &mut [0; 512],
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy {
                drop_context: true,
                ..Default::default()
            },
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::CapabilityMismatch);
    assert_eq!(error.field, "server context");
}
