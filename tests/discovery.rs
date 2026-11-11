use zcall::v1::*;

#[test]
fn metadata_cannot_override_conflicting_choice_shapes_or_roles() {
    for (protocol, response) in [
        (Protocol::OpenAiChat, &br#"{"id":"r","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"text":"ok"}]}"#[..]),
        (Protocol::OpenAiCompletions, &br#"{"id":"r","object":"text_completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"text":"ok"}]}"#[..]),
        (Protocol::GeminiGenerateContent, &br#"{"modelVersion":"m","candidates":[{"content":{"role":"user","parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#[..]),
    ] {
        let formats = [protocol.into()];
        let mut config = DiscoveryConfig::new("m");
        config.formats = &formats;
        let mut discovery = Discovery::with_config("https://example.test", config).unwrap();
        discovery.next_probe(&mut [0; 256], &mut [0; 512]).unwrap();
        assert_eq!(discovery.observe(200, None, response, &mut TokenStorage::<128>::new(), Limits::default()).unwrap(), ProbeOutcome::UnrecognizedResponse);
        assert!(discovery.selected().is_none());
    }
}

const PROTOCOLS: [Protocol; 6] = [
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::AnthropicMessages,
    Protocol::GeminiGenerateContent,
    Protocol::GeminiInteractions,
    Protocol::OpenAiCompletions,
];
const RESPONSES: [&[u8]; 6] = [
    br#"{"id":"r","object":"chat.completion","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#,
    br#"{"id":"r","object":"response","model":"m","created_at":1,"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"ok"}]}]}"#,
    br#"{"id":"r","model":"m","type":"message","role":"assistant","content":[{"type":"text","text":"ok"}],"stop_reason":"end_turn"}"#,
    br#"{"responseId":"r","modelVersion":"m","candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#,
    br#"{"id":"r","model":"m","created":"2026-11-11T00:00:00Z","updated":"2026-11-11T00:00:01Z","status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"ok"}]}]}"#,
    br#"{"id":"r","object":"text_completion","model":"m","created":1,"choices":[{"index":0,"text":"ok","finish_reason":"stop"}]}"#,
];

fn observe(discovery: &mut Discovery<'_>, status: u16, body: &[u8]) -> ProbeOutcome {
    discovery
        .observe(
            status,
            Some("application/json"),
            body,
            &mut TokenStorage::<256>::new(),
            Limits::default(),
        )
        .unwrap()
}

#[test]
fn each_protocol_accepts_its_response_and_rejects_other_protocols() {
    for (candidate, protocol) in PROTOCOLS.into_iter().enumerate() {
        for (response, body) in RESPONSES.into_iter().enumerate() {
            let formats = [Format::new(protocol)];
            let mut config = DiscoveryConfig::new("m");
            config.formats = &formats;
            let mut discovery = Discovery::with_config("https://gateway.example", config).unwrap();
            let mut url = [0; 256];
            let mut payload = [0; 512];
            let probe = discovery
                .next_probe(&mut url, &mut payload)
                .unwrap()
                .unwrap();
            assert_eq!(probe.format.protocol, protocol);
            assert_eq!(probe.method, "POST");
            assert_eq!(probe.content_type, "application/json");
            assert_eq!(probe.accept, "application/json");
            let outcome = observe(&mut discovery, 200, body);
            if candidate == response {
                assert_eq!(outcome, ProbeOutcome::Supported, "{protocol:?}");
                assert_eq!(discovery.selected().unwrap().format().protocol, protocol);
            } else {
                assert_eq!(
                    outcome,
                    ProbeOutcome::UnrecognizedResponse,
                    "{protocol:?}, response {response}"
                );
                assert!(discovery.selected().is_none());
            }
            assert!(
                discovery
                    .next_probe(&mut url, &mut payload)
                    .unwrap()
                    .is_none()
            );
            assert!(discovery.is_finished());
            assert_eq!(discovery.attempts().len(), 1);
            assert_eq!(discovery.attempts()[0].format.protocol, protocol);
            assert_eq!(discovery.attempts()[0].outcome, outcome);
        }
    }
}

#[test]
fn default_search_is_bounded_and_preserves_each_failure() {
    let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
    let mut url = [0; 256];
    let mut body = [0; 512];
    for protocol in PROTOCOLS {
        let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
        assert_eq!(probe.format.protocol, protocol);
        assert_eq!(
            observe(&mut discovery, 404, b"missing"),
            ProbeOutcome::NotFound
        );
    }
    assert!(discovery.is_finished());
    assert!(discovery.selected().is_none());
    assert_eq!(discovery.attempts().len(), PROTOCOLS.len());
    assert!(discovery.next_probe(&mut url, &mut body).unwrap().is_none());
}

#[test]
fn success_stops_search_and_reuses_the_selected_endpoint() {
    let mut discovery = Discovery::new("https://gateway.example/prefix/v1", "m").unwrap();
    let mut url = [0; 256];
    let mut body = [0; 512];
    discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(observe(&mut discovery, 404, b""), ProbeOutcome::NotFound);
    let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(probe.format.protocol, Protocol::OpenAiResponses);
    assert_eq!(probe.url, "https://gateway.example/prefix/v1/responses");
    assert_eq!(
        observe(&mut discovery, 200, RESPONSES[1]),
        ProbeOutcome::Supported
    );
    let selected = discovery.selected().unwrap();
    assert_eq!(selected.base_url(), "https://gateway.example/prefix/v1");
    assert_eq!(
        selected.url_into(Some("m"), true, &mut url).unwrap(),
        "https://gateway.example/prefix/v1/responses"
    );
    assert!(discovery.is_finished());
    assert!(discovery.next_probe(&mut url, &mut body).unwrap().is_none());
    assert_eq!(discovery.attempts().len(), 2);
}

#[test]
fn http_failures_remain_distinct_and_never_confirm_a_protocol() {
    let cases = [
        (401, ProbeOutcome::AuthenticationRequired),
        (403, ProbeOutcome::Forbidden),
        (429, ProbeOutcome::RateLimited),
        (500, ProbeOutcome::ServerError(500)),
        (503, ProbeOutcome::ServerError(503)),
        (301, ProbeOutcome::Redirect(301)),
        (307, ProbeOutcome::Redirect(307)),
        (308, ProbeOutcome::Redirect(308)),
        (404, ProbeOutcome::NotFound),
        (400, ProbeOutcome::Rejected(400)),
        (422, ProbeOutcome::Rejected(422)),
    ];
    for (status, expected) in cases {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        // Even a protocol-shaped error body is not a successful probe.
        assert_eq!(observe(&mut discovery, status, RESPONSES[0]), expected);
        assert!(discovery.selected().is_none());
        assert_eq!(discovery.attempts()[0].outcome, expected);
        assert_eq!(
            discovery
                .next_probe(&mut [0; 256], &mut [0; 512])
                .unwrap()
                .unwrap()
                .format
                .protocol,
            Protocol::OpenAiResponses
        );
    }
}

#[test]
fn generic_ambiguous_and_mixed_json_cannot_confirm_chat() {
    let bodies: &[&[u8]] = &[
        b"{}",
        b"[]",
        br#"{"object":"list","data":[{"id":"model"}]}"#,
        br#"{"choices":[]}"#,
        br#"{"error":{"message":"missing credentials","type":"authentication_error"}}"#,
        br#"{"id":"r","object":"chat.completion","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}],"status":"completed","output":[]}"#,
        br#"{"id":"r","object":"text_completion","choices":[{"index":0,"message":{"role":"assistant","content":"ok"}}]}"#,
    ];
    for body in bodies {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert_eq!(
            observe(&mut discovery, 200, body),
            ProbeOutcome::UnrecognizedResponse,
            "{}",
            String::from_utf8_lossy(body)
        );
        assert!(discovery.selected().is_none());
    }
}

#[test]
fn discriminator_does_not_bypass_semantic_validation() {
    for body in [
        br#"{"object":"chat.completion","choices":[{"index":0,"message":{"role":"user","content":"x"}}]}"#.as_slice(),
        br#"{"object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":42}}]}"#.as_slice(),
    ] {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery.next_probe(&mut [0; 256], &mut [0; 512]).unwrap().unwrap();
        assert_eq!(observe(&mut discovery, 200, body), ProbeOutcome::UnrecognizedResponse);
        assert!(discovery.selected().is_none());
    }
}

#[test]
fn html_content_type_is_not_accepted_even_with_a_json_body() {
    let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
    discovery
        .next_probe(&mut [0; 256], &mut [0; 512])
        .unwrap()
        .unwrap();
    assert_eq!(
        discovery
            .observe(
                200,
                Some("text/html"),
                RESPONSES[0],
                &mut TokenStorage::<128>::new(),
                Limits::default()
            )
            .unwrap(),
        ProbeOutcome::UnrecognizedResponse
    );
    assert!(discovery.selected().is_none());
}

#[test]
fn json_content_type_parameters_and_missing_header_allow_evidence() {
    for content_type in [Some("application/json; charset=utf-8"), None] {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert_eq!(
            discovery
                .observe(
                    200,
                    content_type,
                    RESPONSES[0],
                    &mut TokenStorage::<128>::new(),
                    Limits::default()
                )
                .unwrap(),
            ProbeOutcome::Supported
        );
    }
}

#[test]
fn compatible_chat_and_completion_envelopes_without_object_are_detected() {
    let cases = [
        (Protocol::OpenAiChat, br#"{"id":"r","choices":[{"index":0,"message":{"role":"assistant","content":"ok"},"finish_reason":"stop"}]}"#.as_slice()),
        (Protocol::OpenAiCompletions, br#"{"id":"r","choices":[{"index":0,"text":"ok","finish_reason":"stop"}]}"#.as_slice()),
    ];
    for (protocol, response) in cases {
        let formats = [Format::new(protocol)];
        let mut config = DiscoveryConfig::new("m");
        config.formats = &formats;
        let mut discovery = Discovery::with_config("https://gateway.example", config).unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert_eq!(
            observe(&mut discovery, 200, response),
            ProbeOutcome::Supported
        );
        assert_eq!(discovery.selected().unwrap().format().protocol, protocol);
    }
}

#[test]
fn responses_failure_and_background_acknowledgement_do_not_confirm_support() {
    let formats = [Format::new(Protocol::OpenAiResponses)];
    for response in [
        br#"{"id":"r","object":"response","status":"failed","output":[],"error":null}"#.as_slice(),
        br#"{"id":"r","object":"response","status":"in_progress","output":[],"error":null}"#
            .as_slice(),
    ] {
        let mut config = DiscoveryConfig::new("m");
        config.formats = &formats;
        let mut discovery = Discovery::with_config("https://gateway.example", config).unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert_eq!(
            observe(&mut discovery, 200, response),
            ProbeOutcome::UnrecognizedResponse
        );
        assert!(discovery.selected().is_none());
    }
}

#[test]
fn gemini_candidate_content_is_evidence_without_optional_response_metadata() {
    let formats = [Format::new(Protocol::GeminiGenerateContent)];
    let mut config = DiscoveryConfig::new("m");
    config.formats = &formats;
    let mut discovery = Discovery::with_config("https://gateway.example", config).unwrap();
    discovery
        .next_probe(&mut [0; 256], &mut [0; 512])
        .unwrap()
        .unwrap();
    let response = br#"{"candidates":[{"content":{"role":"model","parts":[{"text":"ok"}]},"finishReason":"STOP"}]}"#;
    assert_eq!(
        observe(&mut discovery, 200, response),
        ProbeOutcome::Supported
    );
    assert_eq!(
        discovery.selected().unwrap().format().protocol,
        Protocol::GeminiGenerateContent
    );
}

#[test]
fn custom_configuration_rejects_empty_duplicate_or_unbounded_candidates() {
    let one = Format::new(Protocol::OpenAiChat);
    for formats in [&[][..], &[one, one][..], &[one; 7][..]] {
        let mut config = DiscoveryConfig::new("m");
        config.formats = formats;
        assert!(Discovery::with_config("https://gateway.example", config).is_err());
    }
    let mut config = DiscoveryConfig::new("m");
    config.max_output_tokens = 0;
    assert!(Discovery::with_config("https://gateway.example", config).is_err());
    for model in ["", " ", "model\nname"] {
        assert!(Discovery::new("https://gateway.example", model).is_err());
    }
}

#[test]
fn insufficient_response_storage_preserves_the_inflight_probe_for_retry() {
    let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
    discovery
        .next_probe(&mut [0; 256], &mut [0; 512])
        .unwrap()
        .unwrap();
    let error = discovery
        .observe(
            200,
            Some("application/json"),
            RESPONSES[0],
            &mut TokenStorage::<1>::new(),
            Limits::default(),
        )
        .unwrap_err();
    assert!(matches!(
        error,
        DiscoveryError::Kernel(Error {
            kind: ErrorKind::Capacity,
            ..
        })
    ));
    assert!(discovery.attempts().is_empty());
    assert!(discovery.selected().is_none());
    assert_eq!(
        observe(&mut discovery, 200, RESPONSES[0]),
        ProbeOutcome::Supported
    );
    assert_eq!(discovery.attempts().len(), 1);
}

#[test]
fn malformed_json_ends_only_the_current_probe() {
    for invalid in [
        b"{".as_slice(),
        br#"{"choices":[],"choices":[]}"#.as_slice(),
    ] {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert_eq!(
            discovery
                .observe(
                    200,
                    Some("application/json"),
                    invalid,
                    &mut TokenStorage::<128>::new(),
                    Limits::default()
                )
                .unwrap(),
            ProbeOutcome::UnrecognizedResponse
        );
        assert_eq!(discovery.attempts().len(), 1);
        assert_eq!(
            discovery
                .next_probe(&mut [0; 256], &mut [0; 512])
                .unwrap()
                .unwrap()
                .format
                .protocol,
            Protocol::OpenAiResponses
        );
    }
}

#[test]
fn resource_limits_leave_the_probe_retryable() {
    let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
    discovery
        .next_probe(&mut [0; 256], &mut [0; 512])
        .unwrap()
        .unwrap();
    let limits = Limits {
        max_input: 8,
        ..Limits::default()
    };
    assert!(matches!(
        discovery.observe(
            200,
            Some("application/json"),
            RESPONSES[0],
            &mut TokenStorage::<128>::new(),
            limits
        ),
        Err(DiscoveryError::Kernel(Error {
            kind: ErrorKind::Limit,
            ..
        }))
    ));
    assert!(discovery.attempts().is_empty());
    assert_eq!(
        observe(&mut discovery, 200, RESPONSES[0]),
        ProbeOutcome::Supported
    );
}

#[test]
fn invalid_http_status_does_not_consume_the_pending_probe() {
    for status in [0, 100, 199, 600, 999] {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .unwrap();
        assert!(matches!(
            discovery.observe(
                status,
                None,
                RESPONSES[0],
                &mut TokenStorage::<128>::new(),
                Limits::default()
            ),
            Err(DiscoveryError::Kernel(Error {
                kind: ErrorKind::InvalidInput,
                ..
            }))
        ));
        assert!(discovery.attempts().is_empty());
        assert_eq!(
            observe(&mut discovery, 200, RESPONSES[0]),
            ProbeOutcome::Supported
        );
    }
}

#[test]
fn full_endpoint_path_limits_candidates_without_confirming_support() {
    let mut discovery = Discovery::new("https://gateway.example/v1/responses", "m").unwrap();
    assert!(discovery.selected().is_none());
    let mut url = [0; 256];
    let mut body = [0; 512];
    let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(probe.format.protocol, Protocol::OpenAiResponses);
    assert_eq!(observe(&mut discovery, 404, b""), ProbeOutcome::NotFound);
    assert!(discovery.is_finished());
    assert!(discovery.selected().is_none());
    assert!(
        discovery
            .next_probe(&mut [0; 256], &mut [0; 512])
            .unwrap()
            .is_none()
    );
}

#[test]
fn short_request_buffers_are_transactional() {
    for (url_capacity, body_capacity) in [(1, 512), (256, 1), (0, 0)] {
        let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
        let mut url = [0xa5; 256];
        let mut body = [0x5a; 512];
        assert!(
            discovery
                .next_probe(&mut url[..url_capacity], &mut body[..body_capacity])
                .is_err()
        );
        assert_eq!(url, [0xa5; 256]);
        assert_eq!(body, [0x5a; 512]);
        assert!(discovery.attempts().is_empty());
        assert_eq!(
            discovery
                .next_probe(&mut url, &mut body)
                .unwrap()
                .unwrap()
                .format
                .protocol,
            Protocol::OpenAiChat
        );
    }
}

#[test]
fn lifecycle_rejects_observation_without_a_probe_and_duplicate_dispatch() {
    let mut discovery = Discovery::new("https://gateway.example", "m").unwrap();
    assert!(discovery.transport_failed().is_err());
    assert!(
        discovery
            .observe(
                200,
                None,
                RESPONSES[0],
                &mut TokenStorage::<128>::new(),
                Limits::default()
            )
            .is_err()
    );
    let mut url = [0; 256];
    let mut body = [0; 512];
    discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert!(discovery.next_probe(&mut url, &mut body).is_err());
    assert_eq!(
        discovery.transport_failed().unwrap(),
        ProbeOutcome::TransportFailed
    );
    assert!(discovery.transport_failed().is_err());
    discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(
        observe(&mut discovery, 200, RESPONSES[1]),
        ProbeOutcome::Supported
    );
    assert!(discovery.transport_failed().is_err());
    assert!(
        discovery
            .observe(
                200,
                None,
                RESPONSES[1],
                &mut TokenStorage::<128>::new(),
                Limits::default()
            )
            .is_err()
    );
}

#[test]
fn caller_order_and_output_budget_control_probe_requests() {
    let formats = [
        Format::new(Protocol::AnthropicMessages),
        Format::new(Protocol::OpenAiChat),
    ];
    let mut config = DiscoveryConfig::new("m");
    config.formats = &formats;
    config.max_output_tokens = 37;
    config.chat_token_limit = ChatTokenLimit::MaxCompletionTokens;
    let mut discovery = Discovery::with_config("https://gateway.example", config).unwrap();
    let mut url = [0; 256];
    let mut body = [0; 512];
    let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(probe.format.protocol, Protocol::AnthropicMessages);
    assert_eq!(
        probe.version_header,
        Some(("anthropic-version", "2023-06-01"))
    );
    assert!(
        std::str::from_utf8(probe.body)
            .unwrap()
            .contains("\"max_tokens\":37")
    );
    observe(&mut discovery, 404, b"");
    let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
    assert_eq!(probe.format.protocol, Protocol::OpenAiChat);
    assert!(
        std::str::from_utf8(probe.body)
            .unwrap()
            .contains("\"max_completion_tokens\":37")
    );
    assert!(
        !std::str::from_utf8(probe.body)
            .unwrap()
            .contains("\"max_tokens\"")
    );
    Request::decode(
        probe.format,
        probe.body,
        &mut TokenStorage::<128>::new(),
        &mut RecordStorage::<16>::new(),
        Limits::default(),
    )
    .unwrap();
}

#[test]
fn generated_probe_payloads_are_valid_requests_for_each_adapter() {
    let mut discovery = Discovery::new("https://gateway.example", "quoted\"model\\name").unwrap();
    let mut url = [0; 512];
    let mut body = [0; 1024];
    for protocol in PROTOCOLS {
        let probe = discovery.next_probe(&mut url, &mut body).unwrap().unwrap();
        assert_eq!(probe.format.protocol, protocol);
        let mut tokens = TokenStorage::<256>::new();
        let mut records = RecordStorage::<16>::new();
        let request = Request::decode(
            probe.format,
            probe.body,
            &mut tokens,
            &mut records,
            Limits::default(),
        )
        .unwrap();
        if protocol != Protocol::GeminiGenerateContent {
            assert!(
                request
                    .model()
                    .value()
                    .unwrap()
                    .eq_str("quoted\"model\\name")
            );
        }
        if matches!(
            protocol,
            Protocol::OpenAiResponses | Protocol::GeminiInteractions
        ) {
            assert!(
                std::str::from_utf8(probe.body)
                    .unwrap()
                    .contains("\"store\":false")
            );
        }
        assert_eq!(observe(&mut discovery, 404, b""), ProbeOutcome::NotFound);
    }
}
