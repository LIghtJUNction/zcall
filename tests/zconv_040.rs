use zcall::v1::*;

#[test]
fn typed_tool_results_survive_storage_insertion_and_encode_as_content_arrays() {
    let source = Protocol::OpenAiResponses.into();
    let mut tokens = TokenStorage::<256>::new();
    let mut records = RecordStorage::<32>::new();
    let key_a = Json::parse(br#""vendor_a""#, &mut tokens, Limits::default()).unwrap();
    let key_b = Json::parse(br#""vendor_b""#, &mut tokens, Limits::default()).unwrap();
    let value = Json::parse(b"1", &mut tokens, Limits::default()).unwrap();
    let mut builder =
        RequestBuilder::new(source, Presence::Value(Text::new("m")), &mut records).unwrap();
    builder.extension(key_a, value).unwrap();
    builder
        .message(
            Role::Assistant,
            &[Content::ToolCall {
                id: Presence::Value(Text::new("call_1")),
                name: Text::new("lookup"),
                arguments: Data::EmptyObject,
                legacy: false,
            }],
        )
        .unwrap();
    let mut request = builder.finish();
    let contents = [
        Content::text("found"),
        Content::Media(Media {
            kind: MediaKind::Image,
            source: MediaSource::Url(Text::new("https://example.test/result.png")),
            mime_type: Presence::Missing,
            detail: Presence::Missing,
            filename: Presence::Missing,
        }),
    ];
    request
        .append_tool_result_contents("call_1", &contents, Presence::Missing)
        .unwrap();
    request.add_extension(key_b, value).unwrap();
    request
        .append_message(Role::User, &[Content::text("continue")])
        .unwrap();
    let result_message = request.messages().nth(1).unwrap();
    assert_eq!(result_message.contents().count(), 1);
    let result = result_message.contents().next().unwrap();
    assert!(matches!(
        result.content(),
        Content::ToolResult {
            result: Data::Content(_),
            ..
        }
    ));
    let children: Vec<_> = result.result_contents().unwrap().collect();
    assert_eq!(children.len(), 2);
    assert!(matches!(children[0].content(), Content::Text(text) if text.eq_str("found")));
    assert!(request.requirements().features.contains(Feature::Image));
    assert!(request.requirements().features.contains(Feature::Tools));
    let copied = result.content();
    assert!(request.replace_content(1, 0, copied).is_err());
    request
        .replace_content(2, 0, Content::text("edited"))
        .unwrap();
    request.set_message_role(1, Role::Tool).unwrap();
    let mut output = [0; 4096];
    let report = request
        .encode(
            source,
            &mut output,
            &mut [0; 512],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    let body = std::str::from_utf8(&output[..report.written]).unwrap();
    assert!(body.contains("\"type\":\"function_call_output\""), "{body}");
    assert!(body.contains("\"output\":["), "{body}");
    assert!(body.contains("\"type\":\"input_image\""), "{body}");
    assert!(body.contains("edited"), "{body}");
    let mut parsed = RecordStorage::<32>::new();
    let decoded = Request::decode(
        source,
        &output[..report.written],
        &mut tokens,
        &mut parsed,
        Limits::default(),
    )
    .unwrap();
    assert_eq!(
        decoded
            .messages()
            .nth(1)
            .unwrap()
            .contents()
            .next()
            .unwrap()
            .result_contents()
            .unwrap()
            .count(),
        2
    );
}

#[test]
fn nested_tool_result_capacity_failure_does_not_consume_the_call() {
    let mut records = RecordStorage::<4>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiResponses.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder
        .message(
            Role::Assistant,
            &[Content::ToolCall {
                id: Presence::Value(Text::new("c")),
                name: Text::new("lookup"),
                arguments: Data::EmptyObject,
                legacy: false,
            }],
        )
        .unwrap();
    let mut request = builder.finish();
    assert!(
        request
            .append_tool_result_contents("c", &[Content::text("ok")], Presence::Missing)
            .is_err()
    );
    assert_eq!(request.records_used(), 2);
    request
        .append_tool_result("c", Data::Text(Text::new("ok")), Presence::Missing)
        .unwrap();
}

#[test]
fn responses_verbosity_remains_typed_when_encoded_for_chat() {
    let input = br#"{"model":"m","input":"hello","text":{"verbosity":"low"}}"#;
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<128>::new();
    let mut request = Request::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert!(matches!(request.text_verbosity(), Presence::Value(value) if value.eq_str("low")));
    assert!(
        request
            .requirements()
            .features
            .contains(Feature::GenerationControls)
    );
    request.set_text_verbosity(Presence::Value(Text::new("high")));
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
    assert!(
        std::str::from_utf8(&output[..report.written])
            .unwrap()
            .contains("\"verbosity\":\"high\"")
    );
}

#[test]
fn resource_codecs_are_usable_without_a_second_dependency() {
    use zcall::api;
    let mut output = [0; 512];
    let n = api::EmbeddingsRequest::new("embedding-model", api::EmbeddingInput::Text("hello"))
        .encode(&mut output)
        .unwrap();
    let mut tokens = [api::json::Token::EMPTY; 32];
    let request = api::parse(
        api::Schema::EmbeddingsRequest,
        &output[..n],
        &mut tokens,
        api::json::Limits::default(),
    )
    .unwrap();
    assert!(request.model().unwrap().eq_str("embedding-model"));
    assert_eq!(api::Endpoint::Embeddings.method(), "POST");
}

#[test]
fn stream_usage_preferences_survive_an_endpoint_without_stream_options() {
    let input = br#"{"model":"m","stream":true,"stream_options":{"include_usage":true,"continuous_usage_stats":true},"messages":[{"role":"user","content":"hello"}]}"#;
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<128>::new();
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert!(matches!(
        request.stream_options(),
        Presence::Value(StreamOptions {
            continuous_usage_stats: Presence::Value(true),
            ..
        })
    ));
    let mut output = [0; 1024];
    let report = request
        .encode(
            Protocol::AnthropicMessages.into(),
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(32),
            Policy::default(),
        )
        .unwrap();
    assert!(matches!(
        report.transport_stream_options,
        Presence::Value(StreamOptions {
            include_usage: Presence::Value(true),
            continuous_usage_stats: Presence::Value(true)
        })
    ));
    assert!(!report.has_losses());
}

#[test]
fn empty_tool_result_array_keeps_its_boundary_when_records_move() {
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<12>::new();
    let key_a = Json::parse(br#""vendor_a""#, &mut tokens, Limits::default()).unwrap();
    let key_b = Json::parse(br#""vendor_b""#, &mut tokens, Limits::default()).unwrap();
    let value = Json::parse(b"0", &mut tokens, Limits::default()).unwrap();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiResponses.into(),
        Presence::Value(Text::new("m")),
        &mut records,
    )
    .unwrap();
    builder.extension(key_a, value).unwrap();
    builder
        .message(
            Role::Assistant,
            &[Content::ToolCall {
                id: Presence::Value(Text::new("c")),
                name: Text::new("lookup"),
                arguments: Data::EmptyObject,
                legacy: false,
            }],
        )
        .unwrap();
    let mut request = builder.finish();
    request
        .append_tool_result_contents("c", &[], Presence::Missing)
        .unwrap();
    request.add_extension(key_b, value).unwrap();
    let mut output = [0; 1024];
    let report = request
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [0; 128],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    assert!(
        std::str::from_utf8(&output[..report.written])
            .unwrap()
            .contains("\"output\":[]")
    );
}

#[test]
fn replacing_a_nested_result_removes_its_old_provider_file_requirements() {
    let input = br#"{"model":"m","input":[{"type":"function_call","call_id":"c","name":"lookup","arguments":"{}"},{"type":"function_call_output","call_id":"c","output":[{"type":"input_file","file_id":"file-provider-owned"}]}]}"#;
    let mut records = RecordStorage::<32>::new();
    let mut tokens = TokenStorage::<256>::new();
    let mut request = Request::decode(
        Protocol::OpenAiResponses.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    assert!(request.requirements().features.contains(Feature::File));
    request
        .replace_content(
            1,
            0,
            Content::ToolResult {
                id: Presence::Value(Text::new("c")),
                name: Presence::Missing,
                result: Data::Text(Text::new("plain result")),
                is_error: Presence::Missing,
            },
        )
        .unwrap();
    assert!(!request.requirements().features.contains(Feature::File));
    let mut output = [0; 2048];
    let report = request
        .encode(
            Protocol::OpenAiChat.into(),
            &mut output,
            &mut [0; 256],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap();
    let body = std::str::from_utf8(&output[..report.written]).unwrap();
    assert!(body.contains("plain result"));
    assert!(!body.contains("file-provider-owned"));
}
