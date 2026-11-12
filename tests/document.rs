use zcall::v1::*;

const PROTOCOLS: [Protocol; 6] = [
    Protocol::OpenAiCompletions,
    Protocol::OpenAiChat,
    Protocol::OpenAiResponses,
    Protocol::AnthropicMessages,
    Protocol::GeminiGenerateContent,
    Protocol::GeminiInteractions,
];
const REQUESTS: [&[u8]; 6] = [
    br#"{"model":"m","prompt":"hello"}"#,
    br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#,
    br#"{"model":"m","input":"hello"}"#,
    br#"{"model":"m","max_tokens":32,"messages":[{"role":"user","content":"hello"}]}"#,
    br#"{"contents":[{"role":"user","parts":[{"text":"hello"}]}]}"#,
    br#"{"model":"m","input":"hello"}"#,
];
const RESPONSES: [&[u8]; 6] = [
    br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"text":"hello","finish_reason":"stop"}]}"#,
    br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"hello"},"finish_reason":"stop"}]}"#,
    br#"{"id":"r","model":"m","created_at":1,"status":"completed","output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"hello"}]}]}"#,
    br#"{"id":"r","model":"m","type":"message","role":"assistant","content":[{"type":"text","text":"hello"}],"stop_reason":"end_turn"}"#,
    br#"{"responseId":"r","modelVersion":"m","candidates":[{"index":0,"content":{"role":"model","parts":[{"text":"hello"}]},"finishReason":"STOP"}]}"#,
    br#"{"id":"r","model":"m","created":"2026-11-10T00:00:00Z","updated":"2026-11-10T00:00:01Z","status":"completed","steps":[{"type":"model_output","content":[{"type":"text","text":"hello"}]}]}"#,
];
fn context<'a>() -> EncodeContext<'a> {
    EncodeContext::default()
        .with_model("m")
        .with_response_id("r")
        .with_created(1)
        .with_created_iso("2026-11-10T00:00:00Z")
        .with_updated_iso("2026-11-10T00:00:01Z")
        .with_max_tokens(32)
        .with_item_ids(&["item_0", "item_1", "item_2"])
}
fn collect_text(request: &Request<'_, '_>) -> String {
    let mut text = String::new();
    let mut bytes = [0; 512];
    for message in request.messages() {
        for block in message.contents() {
            if let Content::Text(value) = block.content() {
                let n = value.decode_into(&mut bytes).unwrap();
                text.push_str(std::str::from_utf8(&bytes[..n]).unwrap());
            }
        }
    }
    text
}

#[test]
fn all_36_request_directions_preserve_uniform_business_view() {
    for (source, input) in PROTOCOLS.into_iter().zip(REQUESTS) {
        for target in PROTOCOLS {
            let mut tokens = TokenStorage::<256>::new();
            let mut records = RecordStorage::<128>::new();
            let mut request = Request::decode(
                source.into(),
                input,
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .unwrap();
            assert_eq!(collect_text(&request), "hello");
            request
                .replace_content(0, 0, Content::text("edited 中文😀\n\"\\"))
                .unwrap();
            let mut output = [0; 2048];
            let report = request
                .encode(
                    target.into(),
                    &mut output,
                    &mut [0; 256],
                    &mut tokens,
                    context(),
                    Policy::default(),
                )
                .unwrap_or_else(|e| panic!("{source:?} -> {target:?}: {e}"));
            assert!(!report.has_losses());
            let mut target_records = RecordStorage::<128>::new();
            let target_request = Request::decode(
                target.into(),
                &output[..report.written],
                &mut tokens,
                &mut target_records,
                Limits::default(),
            )
            .unwrap();
            assert_eq!(collect_text(&target_request), "edited 中文😀\n\"\\");
            assert_eq!(target_request.messages().next().unwrap().role(), Role::User);
        }
    }
}

#[test]
fn all_36_response_directions_preserve_ordered_contents() {
    for (source, input) in PROTOCOLS.into_iter().zip(RESPONSES) {
        for target in PROTOCOLS {
            let mut tokens = TokenStorage::<256>::new();
            let mut records = RecordStorage::<128>::new();
            let mut response = Response::decode(
                source.into(),
                input,
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .unwrap();
            response
                .replace_content(0, 0, Content::text("answer 中文"))
                .unwrap();
            let policy = Policy {
                drop_metadata: true,
                ..Policy::default()
            };
            let expected_losses = response.losses(target.into(), policy).count();
            let mut output = [0; 2048];
            let report = response
                .encode(
                    target.into(),
                    &mut output,
                    &mut [0; 256],
                    &mut tokens,
                    context(),
                    policy,
                )
                .unwrap_or_else(|e| panic!("{source:?} -> {target:?}: {e}"));
            assert_eq!(expected_losses, report.dropped_metadata);
            let mut target_records = RecordStorage::<128>::new();
            let decoded = Response::decode(
                target.into(),
                &output[..report.written],
                &mut tokens,
                &mut target_records,
                Limits::default(),
            )
            .unwrap();
            let candidate = decoded.choices().next().unwrap();
            assert_eq!(candidate.index(), 0);
            assert_eq!(candidate.finish(), Finish::Stop);
            assert!(
                matches!(candidate.contents().next().unwrap().content(), Content::Text(value) if value.eq_str("answer 中文"))
            );
        }
    }
}

#[test]
fn building_requests_uses_actual_storage_and_can_switch_all_backends() {
    let mut storage = RecordStorage::<8>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut storage,
    )
    .unwrap();
    builder
        .message(Role::User, &[Content::text("hello")])
        .unwrap();
    let mut request = builder.finish();
    assert_eq!(request.records_used(), 2);
    for target in PROTOCOLS {
        let mut output = [0; 512];
        let mut tokens = TokenStorage::<64>::new();
        let report = request
            .encode(
                target.into(),
                &mut output,
                &mut [],
                &mut tokens,
                context(),
                Policy::default(),
            )
            .unwrap();
        let mut parsed = RecordStorage::<8>::new();
        let view = Request::decode(
            target.into(),
            &output[..report.written],
            &mut tokens,
            &mut parsed,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(collect_text(&view), "hello");
    }
}

#[test]
fn building_responses_uses_real_choices_and_caller_facts() {
    let mut storage = RecordStorage::<12>::new();
    let mut builder = ResponseBuilder::new(Protocol::OpenAiChat.into(), &mut storage).unwrap();
    builder
        .id(Presence::Value(Text::new("r")))
        .model(Presence::Value(Text::new("m")))
        .created(Presence::Value(Integer::new(1)))
        .status(Finish::Stop);
    builder
        .choice(0, Finish::Stop, &[Content::text("hello")])
        .unwrap();
    let mut response = builder.finish();
    assert_eq!(response.records_used(), 3);
    for target in PROTOCOLS {
        let mut output = [0; 2048];
        let mut tokens = TokenStorage::<128>::new();
        let report = response
            .encode(
                target.into(),
                &mut output,
                &mut [],
                &mut tokens,
                context(),
                Policy {
                    drop_metadata: true,
                    ..Policy::default()
                },
            )
            .unwrap();
        let mut parsed = RecordStorage::<32>::new();
        let view = Response::decode(
            target.into(),
            &output[..report.written],
            &mut tokens,
            &mut parsed,
            Limits::default(),
        )
        .unwrap();
        assert_eq!(view.choices().count(), 1);
        assert!(
            matches!(view.choices().next().unwrap().contents().next().unwrap().content(), Content::Text(v) if v.eq_str("hello"))
        );
    }
}

#[test]
fn missing_null_empty_and_zero_stay_distinct() {
    let input = br#"{"model":"m","temperature":0,"top_p":null,"messages":[{"role":"assistant"},{"role":"assistant","content":null},{"role":"assistant","content":""},{"role":"user","content":[]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let shapes: Vec<_> = request.messages().map(|m| m.shape()).collect();
    assert_eq!(
        shapes,
        [
            ContentShape::Missing,
            ContentShape::Null,
            ContentShape::String,
            ContentShape::Array
        ]
    );
    assert_eq!(
        request
            .parameter(Parameter::Temperature)
            .value()
            .unwrap()
            .as_u64(),
        Some(0)
    );
    assert!(request.parameter(Parameter::TopP).is_null());
    assert!(request.parameter(Parameter::Seed).is_missing());
    let mut output = [0; 1024];
    let report = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    let mut reparsed = RecordStorage::<32>::new();
    let check = Request::decode(
        Protocol::OpenAiChat.into(),
        &output[..report.written],
        &mut tokens,
        &mut reparsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        check.messages().map(|m| m.shape()).collect::<Vec<_>>(),
        shapes
    );
}

#[test]
fn tool_result_linkage_is_checked_before_mutation() {
    let input = br#"{"model":"m","messages":[{"role":"assistant","tool_calls":[{"id":"c\u0031","type":"function","function":{"name":"lookup","arguments":"{\"key\":1}"}}]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let before = request.records_used();
    assert_eq!(
        request
            .append_tool_result("missing", Data::EmptyObject, Presence::Missing)
            .unwrap_err()
            .kind,
        ErrorKind::Lifecycle
    );
    assert_eq!(request.records_used(), before);
    request
        .append_tool_result("c1", Data::Text(Text::new("ok")), Presence::Missing)
        .unwrap();
    let after = request.records_used();
    assert_eq!(after, before + 2);
    assert_eq!(
        request
            .append_tool_result("c1", Data::EmptyObject, Presence::Missing)
            .unwrap_err()
            .kind,
        ErrorKind::Lifecycle
    );
    assert_eq!(request.records_used(), after);
    let mut output = [0; 1024];
    let report = request
        .encode(
            Protocol::AnthropicMessages.into(),
            &mut output,
            &mut [0; 128],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut target = RecordStorage::<32>::new();
    let target = Request::decode(
        Protocol::AnthropicMessages.into(),
        &output[..report.written],
        &mut tokens,
        &mut target,
        Limits::default(),
    )
    .unwrap();
    let last = target.messages().last().unwrap();
    assert!(
        matches!(last.contents().next().unwrap().content(), Content::ToolResult { id: Presence::Value(v), .. } if v.eq_str("c1"))
    );
}

#[test]
fn capacity_failures_are_precise_retryable_and_transactional() {
    let mut tokens = TokenStorage::<0>::new();
    let mut records = RecordStorage::<0>::new();
    let e = Request::decode(
        Protocol::OpenAiChat.into(),
        REQUESTS[1],
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.capacity.unwrap().region, Resource::Tokens);
    let mut tokens = TokenStorage::<64>::new();
    let e = Request::decode(
        Protocol::OpenAiChat.into(),
        REQUESTS[1],
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .err()
    .unwrap();
    assert_eq!(e.capacity.unwrap().region, Resource::Ir);
    let mut records = RecordStorage::<2>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        REQUESTS[1],
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let e = request
        .append_message(Role::User, &[Content::text("second")])
        .unwrap_err();
    assert_eq!(
        e.capacity.unwrap(),
        Capacity {
            region: Resource::Ir,
            needed: 4,
            exact: true,
            replay: Replay::Unconsumed
        }
    );
    assert_eq!(collect_text(&request), "hello");
    let e = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap_err();
    let n = e.capacity.unwrap().needed;
    assert!(e.capacity.unwrap().exact);
    let mut out = vec![0; n];
    assert_eq!(
        request
            .encode(
                Protocol::OpenAiChat.into(),
                &mut out,
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .unwrap()
            .written,
        n
    );
    let mut insufficient = RecordStorage::<1>::new();
    let failed = request.relocate(&mut insufficient).err().unwrap();
    assert_eq!(failed.error.capacity.unwrap().needed, 2);
    assert_eq!(collect_text(&failed.original), "hello");
    let mut grown = RecordStorage::<8>::new();
    let mut request = failed.original.relocate(&mut grown).unwrap();
    request
        .append_message(Role::User, &[Content::text("second")])
        .unwrap();
    assert_eq!(collect_text(&request), "hellosecond");
}

#[test]
fn provider_owned_extensions_context_and_signatures_do_not_leak() {
    let input = br#"{"model":"m","previous_response_id":"state-1","input":[{"role":"user","content":"hello"}],"vendor_field":0}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let target = Format::new(Protocol::AnthropicMessages);
    let error = request
        .encode(
            target,
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::ForbiddenLoss | ErrorKind::ContextRequired
    ));
    let policy = Policy {
        drop_context: true,
        drop_extensions: true,
        ..Policy::default()
    };
    let losses: Vec<_> = request.losses(target, policy).collect();
    assert_eq!(losses.len(), 2);
    assert!(
        matches!(losses[0], Loss::Context { source } if source.protocol == Protocol::OpenAiResponses)
    );
    assert!(
        matches!(losses[1], Loss::Extension(e) if e.name.eq_str("vendor_field") && e.source.protocol == Protocol::OpenAiResponses)
    );
    let report = request
        .encode(
            target,
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            policy,
        )
        .unwrap();
    assert_eq!((report.dropped_context, report.dropped_extensions), (1, 1));
}

#[test]
fn usage_integers_and_cache_arithmetic_are_preserved() {
    let input = br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"x"},"finish_reason":"stop"}],"usage":{"prompt_tokens":9007199254740993,"completion_tokens":0,"total_tokens":9007199254740993}}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let response = Response::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let usage = response.usage().value().unwrap();
    assert_eq!(
        usage.input_tokens.value().unwrap().as_u64(),
        Some(9007199254740993)
    );
    assert_eq!(usage.output_tokens.value().unwrap().as_u64(), Some(0));
    assert!(usage.cached_tokens.is_missing());
    let input = br#"{"id":"r","model":"m","role":"assistant","content":[{"type":"text","text":"x"}],"stop_reason":"end_turn","usage":{"input_tokens":5,"output_tokens":7,"cache_read_input_tokens":11,"cache_creation_input_tokens":13}}"#;
    let mut records = RecordStorage::<32>::new();
    let response = Response::decode(
        Protocol::AnthropicMessages.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let usage = response.usage().value().unwrap();
    assert_eq!(usage.input_tokens.value().unwrap().as_u64(), Some(29));
    assert_eq!(usage.output_tokens.value().unwrap().as_u64(), Some(7));
    assert_eq!(usage.cached_tokens.value().unwrap().as_u64(), Some(11));
    assert_eq!(usage.cache_write_tokens.value().unwrap().as_u64(), Some(13));
}

#[test]
fn multiple_candidates_and_separate_identity_namespaces() {
    let input = br#"{"id":"response-id","model":"m","created":1,"choices":[{"index":2,"message":{"role":"assistant","content":"first"},"finish_reason":"length"},{"index":5,"message":{"role":"assistant","content":"second"},"finish_reason":"stop"}]}"#;
    let mut tokens = TokenStorage::<256>::new();
    let mut records = RecordStorage::<64>::new();
    let mut response = Response::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert!(response.id().value().unwrap().eq_str("response-id"));
    assert_eq!(
        response
            .choices()
            .map(|c| (c.index(), c.finish()))
            .collect::<Vec<_>>(),
        [(2, Finish::Length), (5, Finish::Stop)]
    );
    assert_eq!(
        response
            .encode(
                Protocol::AnthropicMessages.into(),
                &mut [0; 2048],
                &mut [],
                &mut tokens,
                context(),
                Policy {
                    drop_metadata: true,
                    ..Policy::default()
                }
            )
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let input = br#"{"id":"response-id","model":"m","created_at":1,"status":"completed","output":[{"id":"item-id","type":"function_call","call_id":"call-id","name":"lookup","arguments":"{}","status":"completed"}]}"#;
    let mut records = RecordStorage::<32>::new();
    let response = Response::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let block = response
        .choices()
        .next()
        .unwrap()
        .contents()
        .next()
        .unwrap();
    assert!(block.item_id().value().unwrap().eq_str("item-id"));
    assert!(
        matches!(block.content(), Content::ToolCall { id: Presence::Value(id), .. } if id.eq_str("call-id"))
    );
}

#[test]
fn strict_tool_defaults_are_inherited_and_can_be_explicitly_edited() {
    let input = br#"{"model":"m","input":"x","tools":[{"type":"function","name":"f","parameters":{"type":"object"}}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert!(request.tools().next().unwrap().strict.is_missing());
    let error = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap_err();
    assert!(matches!(
        error.kind,
        ErrorKind::ContextRequired | ErrorKind::Unsupported
    ));
    request.set_tool_strict(0, Presence::Value(false)).unwrap();
    request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
}

#[test]
fn malformed_inputs_are_not_upstream_failures_or_panics() {
    let inputs: [&[u8]; 7] = [
        b"",
        b"null",
        b"{}",
        b"{\"model\":\"x\",\"model\":\"y\"}",
        &[255, 0],
        b"[[]",
        br#"{"model":"x","messages":[{"role":"wrong"}]}"#,
    ];
    for input in inputs {
        for format in PROTOCOLS {
            let mut tokens = TokenStorage::<64>::new();
            let mut records = RecordStorage::<16>::new();
            let error = Request::decode(
                format.into(),
                input,
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .err()
            .unwrap();
            assert_ne!(error.kind, ErrorKind::Upstream);
        }
    }
    let mut bytes = [0; 96];
    let mut seed = 0xabcdu32;
    for length in 0..bytes.len() {
        for byte in bytes.iter_mut().take(length) {
            seed = seed.wrapping_mul(1664525).wrapping_add(1013904223);
            *byte = (seed >> 24) as u8;
        }
        for format in PROTOCOLS {
            let mut tokens = TokenStorage::<64>::new();
            let mut records = RecordStorage::<16>::new();
            let _ = Request::decode(
                format.into(),
                &bytes[..length],
                &mut tokens,
                &mut records,
                Limits::default(),
            );
            let _ = Response::decode(
                format.into(),
                &bytes[..length],
                &mut tokens,
                &mut records,
                Limits::default(),
            );
        }
    }
}

#[test]
fn builder_rejects_invalid_edits_and_rolls_back_capacity_failures() {
    let mut storage = RecordStorage::<2>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut storage,
    )
    .unwrap();
    assert!(
        builder
            .message_with(
                MessageOptions {
                    shape: ContentShape::Null,
                    ..MessageOptions::new(Role::User)
                },
                &[Content::text("x")]
            )
            .is_err()
    );
    builder.message(Role::User, &[Content::text("x")]).unwrap();
    assert!(builder.message(Role::User, &[Content::text("y")]).is_err());
    let mut request = builder.finish();
    assert_eq!(request.records_used(), 2);
    assert_eq!(collect_text(&request), "x");
    assert!(
        request
            .replace_content(usize::MAX, usize::MAX, Content::text("x"))
            .is_err()
    );
    assert!(
        request
            .set_tool_strict(usize::MAX, Presence::Value(false))
            .is_err()
    );
    let mut tokens = TokenStorage::<16>::new();
    let value = Json::parse(b"true", &mut tokens, Limits::default()).unwrap();
    assert!(
        request
            .set_parameter(Parameter::MaxOutputTokens, Presence::Value(value))
            .is_err()
    );
}

#[test]
fn all_25_modern_tool_request_directions_are_connected() {
    let cases = [
        (Protocol::OpenAiChat, br#"{"model":"m","messages":[{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{\"x\":1}"}}]},{"role":"tool","tool_call_id":"c","content":"{\"ok\":true}"}],"tools":[{"type":"function","function":{"name":"f","parameters":{"type":"object","properties":{"x":{"type":"integer"}}}}}]}"#.as_slice()),
        (Protocol::OpenAiResponses, br#"{"model":"m","input":[{"type":"function_call","call_id":"c","name":"f","arguments":"{\"x\":1}"},{"type":"function_call_output","call_id":"c","output":"{\"ok\":true}"}],"tools":[{"type":"function","name":"f","strict":false,"parameters":{"type":"object","properties":{"x":{"type":"integer"}}}}]}"#.as_slice()),
        (Protocol::AnthropicMessages, br#"{"model":"m","max_tokens":32,"messages":[{"role":"assistant","content":[{"type":"tool_use","id":"c","name":"f","input":{"x":1}}]},{"role":"user","content":[{"type":"tool_result","tool_use_id":"c","content":"{\"ok\":true}"}]}],"tools":[{"name":"f","input_schema":{"type":"object","properties":{"x":{"type":"integer"}}}}]}"#.as_slice()),
        (Protocol::GeminiGenerateContent, br#"{"contents":[{"role":"model","parts":[{"functionCall":{"id":"c","name":"f","args":{"x":1}}}]},{"role":"user","parts":[{"functionResponse":{"id":"c","name":"f","response":{"ok":true}}}]}],"tools":[{"functionDeclarations":[{"name":"f","parameters":{"type":"object","properties":{"x":{"type":"integer"}}}}]}]}"#.as_slice()),
        (Protocol::GeminiInteractions, br#"{"model":"m","input":[{"type":"function_call","id":"c","name":"f","arguments":{"x":1}},{"type":"function_result","call_id":"c","name":"f","result":{"ok":true}}],"tools":[{"type":"function","name":"f","parameters":{"type":"object","properties":{"x":{"type":"integer"}}}}]}"#.as_slice()),
    ];
    let mut bindings = ToolBindingStorage::<1>::new();
    bindings.push("c", "f").unwrap();
    for (source, input) in cases {
        for target in PROTOCOLS.into_iter().skip(1) {
            let mut tokens = TokenStorage::<256>::new();
            let mut records = RecordStorage::<64>::new();
            let mut request = Request::decode(
                source.into(),
                input,
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .unwrap();
            let mut output = [0; 4096];
            let report = request
                .encode(
                    target.into(),
                    &mut output,
                    &mut [0; 512],
                    &mut tokens,
                    context().with_tool_bindings(&bindings),
                    Policy::default(),
                )
                .unwrap_or_else(|e| panic!("{source:?}->{target:?}: {e}"));
            assert!(!report.has_losses());
            let mut target_records = RecordStorage::<64>::new();
            let decoded = Request::decode(
                target.into(),
                &output[..report.written],
                &mut tokens,
                &mut target_records,
                Limits::default(),
            )
            .unwrap();
            assert_eq!(decoded.tools().count(), 1);
            let contents: Vec<_> = decoded
                .messages()
                .flat_map(|m| m.contents())
                .map(|b| b.content())
                .collect();
            assert_eq!(
                contents
                    .iter()
                    .filter(|c| matches!(c, Content::ToolCall { .. }))
                    .count(),
                1
            );
            assert_eq!(
                contents
                    .iter()
                    .filter(|c| matches!(c, Content::ToolResult { .. }))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn complete_tool_arguments_are_validated_even_for_same_format() {
    let input = br#"{"model":"m","messages":[{"role":"assistant","tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{\"x\":"}}]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<16>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let error = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap_err();
    assert_eq!(
        error.capacity.unwrap(),
        Capacity {
            region: Resource::Scratch,
            needed: 5,
            exact: true,
            replay: Replay::Unconsumed
        }
    );
    let error = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [0; 1024],
            &mut [0; 16],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap_err();
    assert_eq!(error.kind, ErrorKind::InvalidJson);
    assert_eq!(error.field, "tool arguments");
    request
        .replace_content(
            0,
            0,
            Content::ToolCall {
                id: Presence::Value(Text::new("c")),
                name: Text::new("f"),
                arguments: Data::EmptyObject,
                legacy: false,
            },
        )
        .unwrap();
    request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut [0; 1024],
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
}

#[test]
fn new_extensions_preserve_existing_records_and_are_reported_individually() {
    let mut tokens = TokenStorage::<16>::new();
    let k1 = Json::parse(br#""vendor_a""#, &mut tokens, Limits::default()).unwrap();
    let k2 = Json::parse(br#""vendor_b""#, &mut tokens, Limits::default()).unwrap();
    let value = Json::parse(b"0", &mut tokens, Limits::default()).unwrap();
    let mut storage = RecordStorage::<8>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("m")),
        &mut storage,
    )
    .unwrap();
    builder.extension(k1, value).unwrap();
    builder
        .message(Role::User, &[Content::text("first")])
        .unwrap();
    builder.extension(k2, value).unwrap();
    builder
        .message(Role::Assistant, &[Content::text("second")])
        .unwrap();
    let mut request = builder.finish();
    assert_eq!(collect_text(&request), "firstsecond");
    assert_eq!(request.extensions().count(), 2);
    assert!(request.add_extension(k1, value).is_err());
    let mut output = [0; 1024];
    let mut tokens = TokenStorage::<128>::new();
    let report = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    assert!(!report.has_losses());
    let mut parsed = RecordStorage::<16>::new();
    let check = Request::decode(
        Protocol::OpenAiChat.into(),
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(check.extensions().count(), 2);
    assert_eq!(collect_text(&check), "firstsecond");
    let policy = Policy {
        drop_extensions: true,
        ..Policy::default()
    };
    assert_eq!(
        request
            .losses(Protocol::OpenAiResponses.into(), policy)
            .count(),
        2
    );
    assert_eq!(
        request
            .encode(
                Protocol::OpenAiResponses.into(),
                &mut output,
                &mut [],
                &mut tokens,
                context(),
                policy
            )
            .unwrap()
            .dropped_extensions,
        2
    );
}

#[test]
fn structured_output_and_media_keep_source_namespaces() {
    let input = br#"{"model":"m","messages":[{"role":"user","content":[{"type":"image_url","image_url":{"url":"https://example.test/a.png"}}]}],"response_format":{"type":"json_schema","json_schema":{"name":"answer","strict":true,"schema":{"type":"object","properties":{"answer":{"type":"string"}},"required":["answer"],"additionalProperties":false}}}}"#;
    let mut tokens = TokenStorage::<256>::new();
    let mut storage = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    let constraint = request.output_constraint().value().unwrap();
    assert_eq!(constraint.mode, OutputMode::JsonSchema);
    assert_eq!(constraint.source.protocol, Protocol::OpenAiChat);
    assert!(constraint.name.value().unwrap().eq_str("answer"));
    assert!(
        matches!(request.messages().next().unwrap().contents().next().unwrap().content(), Content::Media(Media { kind: MediaKind::Image, source: MediaSource::Url(value), .. }) if value.eq_str("https://example.test/a.png"))
    );
    let mut output = [0; 4096];
    let report = request
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut parsed = RecordStorage::<32>::new();
    let check = Request::decode(
        Protocol::OpenAiResponses.into(),
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        check.output_constraint().value().unwrap().mode,
        OutputMode::JsonSchema
    );
    assert!(
        request
            .encode(
                Protocol::AnthropicMessages.into(),
                &mut [0; 4096],
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .is_err()
    );
}

#[test]
fn opaque_signatures_are_visible_and_cross_provider_transfer_is_strict() {
    let input = br#"{"model":"m","max_tokens":32,"messages":[{"role":"assistant","content":[{"type":"thinking","thinking":"hidden text","signature":"opaque-signature"},{"type":"text","text":"answer"}]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut storage = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::AnthropicMessages.into(),
        input,
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    assert!(
        matches!(request.messages().next().unwrap().contents().next().unwrap().content(), Content::Reasoning { signature: Presence::Value(v), .. } if v.eq_str("opaque-signature"))
    );
    assert!(
        request
            .encode(
                Protocol::OpenAiResponses.into(),
                &mut [0; 2048],
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .is_err()
    );
    let policy = Policy {
        drop_reasoning: true,
        ..Policy::default()
    };
    let losses: Vec<_> = request
        .losses(Protocol::OpenAiResponses.into(), policy)
        .collect();
    assert!(matches!(
        losses.as_slice(),
        [Loss::Reasoning {
            location: Location {
                message: Some(0),
                content: Some(0),
                ..
            }
        }]
    ));
    let report = request
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut [0; 2048],
            &mut [],
            &mut tokens,
            context(),
            policy,
        )
        .unwrap();
    assert_eq!(report.dropped_reasoning, 1);
}

#[test]
fn all_25_modern_tool_response_directions_preserve_action_requirement() {
    let cases = [
        (Protocol::OpenAiChat, br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":null,"tool_calls":[{"id":"c","type":"function","function":{"name":"f","arguments":"{\"x\":1}"}}]},"finish_reason":"tool_calls"}]}"#.as_slice()),
        (Protocol::OpenAiResponses, br#"{"id":"r","model":"m","created_at":1,"status":"completed","output":[{"id":"item","type":"function_call","call_id":"c","name":"f","arguments":"{\"x\":1}"}]}"#.as_slice()),
        (Protocol::AnthropicMessages, br#"{"id":"r","model":"m","type":"message","role":"assistant","content":[{"type":"tool_use","id":"c","name":"f","input":{"x":1}}],"stop_reason":"tool_use"}"#.as_slice()),
        (Protocol::GeminiGenerateContent, br#"{"responseId":"r","modelVersion":"m","candidates":[{"index":0,"content":{"role":"model","parts":[{"functionCall":{"id":"c","name":"f","args":{"x":1}}}]},"finishReason":"STOP"}]}"#.as_slice()),
        (Protocol::GeminiInteractions, br#"{"id":"r","model":"m","created":"2026-11-10T00:00:00Z","updated":"2026-11-10T00:00:01Z","status":"requires_action","steps":[{"type":"function_call","id":"c","name":"f","arguments":{"x":1}}]}"#.as_slice()),
    ];
    for (source, input) in cases {
        for target in PROTOCOLS.into_iter().skip(1) {
            let mut tokens = TokenStorage::<256>::new();
            let mut records = RecordStorage::<64>::new();
            let mut response = Response::decode(
                source.into(),
                input,
                &mut tokens,
                &mut records,
                Limits::default(),
            )
            .unwrap();
            let policy = Policy {
                drop_metadata: true,
                ..Policy::default()
            };
            let losses = response.losses(target.into(), policy).count();
            let mut output = [0; 4096];
            let report = response
                .encode(
                    target.into(),
                    &mut output,
                    &mut [0; 256],
                    &mut tokens,
                    context(),
                    policy,
                )
                .unwrap_or_else(|e| panic!("{source:?}->{target:?}: {e}"));
            assert_eq!(report.dropped_metadata, losses);
            let mut target_records = RecordStorage::<64>::new();
            let decoded = Response::decode(
                target.into(),
                &output[..report.written],
                &mut tokens,
                &mut target_records,
                Limits::default(),
            )
            .unwrap();
            let candidate = decoded.choices().next().unwrap();
            assert_eq!(candidate.finish(), Finish::ToolCalls);
            assert_eq!(
                candidate
                    .contents()
                    .filter(|b| matches!(b.content(), Content::ToolCall { .. }))
                    .count(),
                1
            );
        }
    }
}

#[test]
fn file_ids_audio_and_safety_remain_typed() {
    let input = br#"{"model":"m","messages":[{"role":"user","content":[{"type":"input_audio","input_audio":{"data":"AA==","format":"wav"}},{"type":"file","file":{"file_id":"provider-file"}}]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let mut blocks = request.messages().next().unwrap().contents();
    assert!(
        matches!(blocks.next().unwrap().content(), Content::Media(Media { kind: MediaKind::Audio, source: MediaSource::Base64 { data, .. }, .. }) if data.eq_str("AA=="))
    );
    assert!(
        matches!(blocks.next().unwrap().content(), Content::Media(Media { kind: MediaKind::File, source: MediaSource::FileId { namespace, id }, .. }) if namespace.protocol == Protocol::OpenAiChat && id.eq_str("provider-file"))
    );
    let mut output = [0; 2048];
    request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    assert_eq!(
        request
            .encode(
                Protocol::GeminiGenerateContent.into(),
                &mut output,
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::ContextRequired
    );
    let blocked = br#"{"promptFeedback":{"blockReason":"SAFETY"}}"#;
    let mut storage = RecordStorage::<16>::new();
    let response = Response::decode(
        Protocol::GeminiGenerateContent.into(),
        blocked,
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(response.status(), Finish::Safety);
    assert_eq!(response.choices().next().unwrap().finish(), Finish::Safety);
    let failed = br#"{"id":"r","model":"m","status":"failed","output":[],"error":{"code":"model_error","message":"failed"}}"#;
    let mut storage = RecordStorage::<16>::new();
    let response = Response::decode(
        Protocol::OpenAiResponses.into(),
        failed,
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(response.status(), Finish::Error);
    assert_eq!(response.error().value().unwrap().kind(), JsonKind::Object);
}

#[test]
fn edits_keep_message_markers_coherent_and_reject_bad_types() {
    let input = br#"{"model":"m","messages":[{"role":"assistant","content":"old"}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut storage = RecordStorage::<16>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    request
        .replace_content(
            0,
            0,
            Content::ToolCall {
                id: Presence::Value(Text::new("call")),
                name: Text::new("f"),
                arguments: Data::EmptyObject,
                legacy: false,
            },
        )
        .unwrap();
    assert!(request.set_message_role(0, Role::User).is_err());
    let mut output = [0; 2048];
    let report = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut parsed = RecordStorage::<16>::new();
    let check = Request::decode(
        Protocol::OpenAiChat.into(),
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        check
            .messages()
            .next()
            .unwrap()
            .contents()
            .next()
            .unwrap()
            .content(),
        Content::ToolCall { .. }
    ));
    let fraction = Json::parse(b"1.5", &mut tokens, Limits::default()).unwrap();
    assert!(
        request
            .set_parameter(Parameter::Seed, Presence::Value(fraction))
            .is_err()
    );
    let wrong_stop = Json::parse(b"[1]", &mut tokens, Limits::default()).unwrap();
    request
        .set_parameter(Parameter::Stop, Presence::Value(wrong_stop))
        .unwrap();
    assert_eq!(
        request
            .encode(
                Protocol::OpenAiChat.into(),
                &mut output,
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::InvalidInput
    );
}

#[test]
fn explicit_finish_edits_change_encoded_status_and_keep_cancellation_distinct() {
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut response = Response::decode(
        Protocol::OpenAiChat.into(),
        RESPONSES[1],
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    response.set_choice_finish(0, Finish::Length).unwrap();
    let mut output = [0; 2048];
    let report = response
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut decoded_records = RecordStorage::<32>::new();
    let check = Response::decode(
        Protocol::OpenAiChat.into(),
        &output[..report.written],
        &mut tokens,
        &mut decoded_records,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(check.choices().next().unwrap().finish(), Finish::Length);
    response.set_status(Finish::Cancelled);
    assert_eq!(
        response.choices().next().unwrap().finish(),
        Finish::Cancelled
    );
    let report = response
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut decoded_records = RecordStorage::<32>::new();
    let check = Response::decode(
        Protocol::OpenAiResponses.into(),
        &output[..report.written],
        &mut tokens,
        &mut decoded_records,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(check.status(), Finish::Cancelled);
    assert_eq!(
        response
            .encode(
                Protocol::OpenAiChat.into(),
                &mut [0; 2048],
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn refusal_history_cannot_silently_become_ordinary_text() {
    let input =
        br#"{"model":"m","messages":[{"role":"assistant","content":null,"refusal":"declined"}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<16>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        request
            .encode(
                Protocol::AnthropicMessages.into(),
                &mut [0; 1024],
                &mut [],
                &mut tokens,
                context(),
                Policy::default()
            )
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    let response = br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":null,"refusal":"declined"},"finish_reason":"stop"}]}"#;
    let mut records = RecordStorage::<16>::new();
    let mut response = Response::decode(
        Protocol::OpenAiChat.into(),
        response,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        response
            .encode(
                Protocol::AnthropicMessages.into(),
                &mut [0; 1024],
                &mut [],
                &mut tokens,
                context(),
                Policy {
                    drop_metadata: true,
                    ..Policy::default()
                }
            )
            .unwrap_err()
            .kind,
        ErrorKind::Unsupported
    );
    response.set_choice_finish(0, Finish::Refusal).unwrap();
    let mut output = [0; 1024];
    let report = response
        .encode(
            Protocol::AnthropicMessages.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy {
                drop_metadata: true,
                ..Policy::default()
            },
        )
        .unwrap();
    let mut parsed = RecordStorage::<16>::new();
    let check = Response::decode(
        Protocol::AnthropicMessages.into(),
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(check.choices().next().unwrap().finish(), Finish::Refusal);
    assert!(
        matches!(check.choices().next().unwrap().contents().next().unwrap().content(), Content::Refusal(v) if v.eq_str("declined"))
    );
}

#[test]
fn replacing_storage_in_place_preserves_edits_after_failure_and_growth() {
    let mut tokens = TokenStorage::<128>::new();
    let mut storage = RecordStorage::<2>::new();
    let mut rejected = RecordStorage::<1>::new();
    let mut grown = RecordStorage::<16>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        REQUESTS[1],
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    request
        .replace_content(0, 0, Content::text("edited"))
        .unwrap();
    let error = request.replace_storage(&mut rejected).unwrap_err();
    assert_eq!(error.capacity.unwrap().needed, 2);
    assert_eq!(collect_text(&request), "edited");
    let mut output = [0; 2048];
    request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    request.replace_storage(&mut grown).unwrap();
    request
        .append_message(Role::User, &[Content::text(" extra")])
        .unwrap();
    assert_eq!(collect_text(&request), "edited extra");

    let mut storage = RecordStorage::<3>::new();
    let mut rejected = RecordStorage::<2>::new();
    let mut grown = RecordStorage::<16>::new();
    let mut response = Response::decode(
        Protocol::OpenAiChat.into(),
        RESPONSES[1],
        &mut tokens,
        &mut storage,
        Limits::default(),
    )
    .unwrap();
    response
        .replace_content(0, 0, Content::text("edited response"))
        .unwrap();
    response.set_choice_finish(0, Finish::Length).unwrap();
    assert!(response.replace_storage(&mut rejected).is_err());
    response
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    response.replace_storage(&mut grown).unwrap();
    response
        .append_choice(1, Finish::Stop, &[Content::text("another")])
        .unwrap();
    let first = response.choices().next().unwrap();
    assert_eq!(first.finish(), Finish::Length);
    assert!(
        matches!(first.contents().next().unwrap().content(), Content::Text(value) if value.eq_str("edited response"))
    );
    assert_eq!(response.choices().count(), 2);
}

#[test]
fn changing_incomplete_choice_clears_stale_provider_snapshot() {
    let input = br#"{"id":"r","model":"m","created_at":1,"status":"incomplete","incomplete_details":{"reason":"max_output_tokens"},"output":[{"type":"message","role":"assistant","content":[{"type":"output_text","text":"partial"}]}]}"#;
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut response = Response::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(response.status(), Finish::Length);
    response.set_choice_finish(0, Finish::Stop).unwrap();
    assert_eq!(response.status(), Finish::Stop);
    assert!(response.incomplete_details().is_missing());
    let mut output = [0; 2048];
    let report = response
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [],
            &mut tokens,
            context(),
            Policy::default(),
        )
        .unwrap();
    let mut parsed = RecordStorage::<32>::new();
    let check = Response::decode(
        Protocol::OpenAiResponses.into(),
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(check.status(), Finish::Stop);
    assert_eq!(check.choices().next().unwrap().finish(), Finish::Stop);
}

#[test]
fn schema_dialects_and_provider_signatures_are_public_and_round_trip() {
    let input = br#"{"contents":[{"role":"model","parts":[{"text":"answer","thoughtSignature":"sig"}]}],"tools":[{"functionDeclarations":[{"name":"json","parametersJsonSchema":{"type":"object"}},{"name":"typed","parameters":{"type":"OBJECT"}}]}]}"#;
    let mut tokens = TokenStorage::<256>::new();
    let mut records = RecordStorage::<64>::new();
    let mut request = Request::decode(
        Protocol::GeminiGenerateContent.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let tools: Vec<_> = request.tools().collect();
    assert_eq!(tools[0].schema_dialect, SchemaDialect::JsonSchema);
    assert_eq!(tools[1].schema_dialect, SchemaDialect::GeminiSchema);
    let block = request
        .messages()
        .next()
        .unwrap()
        .contents()
        .next()
        .unwrap();
    let signature = block.signature().value().unwrap();
    assert_eq!(signature.source, Protocol::GeminiGenerateContent.into());
    assert!(signature.value.eq_str("sig"));
    request
        .set_block_signature(
            0,
            0,
            Presence::Value(ProviderSignature {
                source: signature.source,
                value: Text::new("replacement"),
            }),
        )
        .unwrap();
    let mut output = [0; 4096];
    let mut scratch = [0; 4096];
    let report = request
        .encode(
            Protocol::GeminiGenerateContent.into(),
            &mut output,
            &mut scratch,
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    let output = std::str::from_utf8(&output[..report.written]).unwrap();
    assert!(output.contains("parametersJsonSchema"));
    assert!(output.contains("\"thoughtSignature\":\"replacement\""));
}
