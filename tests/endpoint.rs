use zcall::v1::*;

fn url(base: &str, protocol: Protocol, model: Option<&str>, stream: bool) -> String {
    let endpoint = HttpEndpoint::new(base, protocol.into()).unwrap();
    let mut output = [0; 1024];
    endpoint
        .url_into(model, stream, &mut output)
        .unwrap()
        .to_owned()
}

#[test]
fn all_protocols_build_from_an_origin() {
    for (protocol, suffix) in [
        (Protocol::OpenAiCompletions, "/v1/completions"),
        (Protocol::OpenAiChat, "/v1/chat/completions"),
        (Protocol::OpenAiResponses, "/v1/responses"),
        (Protocol::AnthropicMessages, "/v1/messages"),
        (
            Protocol::GeminiGenerateContent,
            "/v1beta/models/gemini:generateContent",
        ),
        (Protocol::GeminiInteractions, "/v1beta/interactions"),
    ] {
        for origin in ["https://example.test", "https://example.test/"] {
            assert_eq!(
                url(origin, protocol, Some("gemini"), false),
                format!("https://example.test{suffix}")
            );
        }
    }
}

#[test]
fn custom_prefix_versions_and_complete_endpoints_are_preserved() {
    assert_eq!(
        url(
            "https://example.test/v1/",
            Protocol::OpenAiChat,
            None,
            false
        ),
        "https://example.test/v1/chat/completions"
    );
    assert_eq!(
        url(
            "https://example.test/proxy/team/",
            Protocol::OpenAiResponses,
            None,
            false
        ),
        "https://example.test/proxy/team/responses"
    );
    assert_eq!(
        url(
            "https://example.test/openai/deployments/demo/chat/completions?api-version=2026-01-01",
            Protocol::OpenAiChat,
            None,
            false
        ),
        "https://example.test/openai/deployments/demo/chat/completions?api-version=2026-01-01"
    );
    for (protocol, path) in [
        (Protocol::OpenAiCompletions, "/custom/completions"),
        (Protocol::OpenAiChat, "/custom/chat/completions"),
        (Protocol::OpenAiResponses, "/custom/responses"),
        (Protocol::AnthropicMessages, "/custom/messages"),
        (Protocol::GeminiInteractions, "/custom/interactions"),
    ] {
        let base = format!("http://localhost:8000{path}/");
        assert_eq!(
            url(&base, protocol, None, true),
            format!("http://localhost:8000{path}")
        );
    }
    assert_eq!(
        url(
            "https://example.test/v1",
            Protocol::GeminiGenerateContent,
            Some("m"),
            false
        ),
        "https://example.test/v1/models/m:generateContent"
    );
    assert_eq!(
        url(
            "https://example.test/v1beta/models/",
            Protocol::GeminiGenerateContent,
            Some("m"),
            false
        ),
        "https://example.test/v1beta/models/m:generateContent"
    );
}

#[test]
fn a_complete_endpoint_must_match_the_configured_protocol() {
    for (base, found) in [
        (
            "https://example.test/v1/chat/completions",
            Protocol::OpenAiChat,
        ),
        (
            "https://example.test/v1/responses",
            Protocol::OpenAiResponses,
        ),
        (
            "https://example.test/v1/messages",
            Protocol::AnthropicMessages,
        ),
        (
            "https://example.test/v1beta/models/m:generateContent",
            Protocol::GeminiGenerateContent,
        ),
        (
            "https://example.test/v1beta/models/m:streamGenerateContent?alt=sse",
            Protocol::GeminiGenerateContent,
        ),
        (
            "https://example.test/v1beta/interactions",
            Protocol::GeminiInteractions,
        ),
    ] {
        assert_eq!(
            HttpEndpoint::new(base, Protocol::OpenAiCompletions.into()),
            Err(EndpointError::ProtocolMismatch {
                expected: Protocol::OpenAiCompletions,
                found
            })
        );
    }
}

#[test]
fn malformed_complete_gemini_paths_are_rejected_instead_of_used_as_prefixes() {
    for base in [
        "https://example.test/v1beta/models/:generateContent",
        "https://example.test/v1beta/models/m/n:generateContent",
        "https://example.test/v1beta/m:generateContent",
        "https://example.test/v1beta/models/:streamGenerateContent/?alt=sse",
        "https://example.test/v1beta/models/m/n:streamGenerateContent",
        "https://example.test/v1beta/models/.:generateContent",
        "https://example.test/v1beta/models/..:generateContent",
        "https://example.test/v1beta/models/%2e%2E:generateContent",
    ] {
        for protocol in [Protocol::GeminiGenerateContent, Protocol::OpenAiChat] {
            assert!(
                matches!(
                    HttpEndpoint::new(base, protocol.into()),
                    Err(EndpointError::InvalidUrl(_))
                ),
                "{base} with {protocol:?}"
            );
        }
    }
}

#[test]
fn gemini_action_model_and_query_follow_each_request() {
    let base = "https://example.test/proxy/v1beta/models/old:generateContent?api-version=1&alt=json&key=a%2Bb";
    assert_eq!(
        url(
            base,
            Protocol::GeminiGenerateContent,
            Some("models/new"),
            true
        ),
        "https://example.test/proxy/v1beta/models/new:streamGenerateContent?api-version=1&key=a%2Bb&alt=sse"
    );
    assert_eq!(
        url(base, Protocol::GeminiGenerateContent, None, false),
        base
    );
    let stream =
        "https://example.test/v1beta/models/existing%2Fmodel:streamGenerateContent?key=x&alt=sse";
    assert_eq!(
        url(stream, Protocol::GeminiGenerateContent, None, false),
        "https://example.test/v1beta/models/existing%2Fmodel:generateContent?key=x"
    );
    assert_eq!(
        url(
            "https://example.test?%61lt=sse&x=1&alt=json",
            Protocol::GeminiGenerateContent,
            Some("m"),
            true
        ),
        "https://example.test/v1beta/models/m:streamGenerateContent?x=1&alt=sse"
    );
    assert_eq!(
        url(
            "https://example.test?%61lt=%73se&x=1",
            Protocol::GeminiGenerateContent,
            Some("m"),
            false
        ),
        "https://example.test/v1beta/models/m:generateContent?x=1"
    );
}

#[test]
fn raw_model_names_are_encoded_as_one_segment() {
    assert_eq!(
        url(
            "https://example.test",
            Protocol::GeminiGenerateContent,
            Some("models/a/b?x=1#片%20"),
            true
        ),
        "https://example.test/v1beta/models/a%2Fb%3Fx%3D1%23%E7%89%87%2520:streamGenerateContent?alt=sse"
    );
    let endpoint = HttpEndpoint::new(
        "https://example.test",
        Protocol::GeminiGenerateContent.into(),
    )
    .unwrap();
    let mut output = [b'!'; 128];
    assert_eq!(
        endpoint.url_into(None, false, &mut output),
        Err(EndpointError::MissingModel)
    );
    for model in ["", "models/", ".", "..", "a\nb", "a\0b", "a\u{85}b"] {
        assert!(
            matches!(
                endpoint.url_into(Some(model), false, &mut output),
                Err(EndpointError::InvalidModel(_))
            ),
            "{model:?}"
        );
        assert_eq!(output, [b'!'; 128]);
    }
}

#[test]
fn insufficient_output_has_an_exact_size_and_no_partial_writes() {
    for (base, protocol, model, stream) in [
        ("https://example.test", Protocol::OpenAiChat, None, false),
        (
            "https://example.test/custom?api-version=x",
            Protocol::OpenAiResponses,
            None,
            false,
        ),
        (
            "https://example.test?key=x&alt=json",
            Protocol::GeminiGenerateContent,
            Some("models/a?片"),
            true,
        ),
    ] {
        let expected = url(base, protocol, model, stream);
        let endpoint = HttpEndpoint::new(base, protocol.into()).unwrap();
        for capacity in 0..expected.len() {
            let mut storage = vec![0xA5; capacity + 8];
            assert_eq!(
                endpoint.url_into(model, stream, &mut storage[..capacity]),
                Err(EndpointError::BufferTooSmall {
                    needed: expected.len()
                })
            );
            assert!(storage.iter().all(|byte| *byte == 0xA5));
        }
        let mut exact = vec![0; expected.len()];
        assert_eq!(
            endpoint.url_into(model, stream, &mut exact).unwrap(),
            expected
        );
    }
}

#[test]
fn invalid_urls_are_rejected_before_rendering() {
    for base in [
        "",
        "example.test",
        "/v1",
        "ftp://example.test",
        "https://",
        "https:///v1",
        "https://?x=1",
        "https://user:password@example.test",
        "https://example.test/#fragment",
        "https://example.test/a b",
        "https://example.test/\r\nX:injected",
        "https://example.test\\@evil.test",
        "https://example.test:bad",
        "https://example.test:",
        "https://example.test:65536",
        "https://example.test:-1",
        "https://[not-ipv6]/",
        "https://[::1",
        "https://[::1]evil",
        "https://::1/",
        "https://999.999.999.999/",
        "https://-example.test",
        "https://example-.test",
        "https://example..test",
        "https://.example.test",
        "https://片.test",
        "https://example.test/片",
        "https://example.test/%",
        "https://example.test/%xz",
        "https://example.test?x=%1",
        "https://example.test/<path>",
        "https://example.test?x=\"quoted\"",
        "https://example.test/../v1",
        "https://example.test/%2e%2E/v1",
        "https://example.test/v1/./",
        "https://example.test/%2e/v1",
    ] {
        assert!(
            matches!(
                HttpEndpoint::new(base, Protocol::OpenAiChat.into()),
                Err(EndpointError::InvalidUrl(_))
            ),
            "{base:?}"
        );
    }
}

#[test]
fn local_ipv4_ipv6_and_encoded_uri_components_are_supported() {
    for (base, expected) in [
        (
            "http://127.0.0.1:8000",
            "http://127.0.0.1:8000/v1/responses",
        ),
        ("http://[::1]:8000/v1", "http://[::1]:8000/v1/responses"),
        (
            "HTTPS://example.test./proxy%20path?x=a%2Bb&url=https://x.test/a?b=1",
            "HTTPS://example.test./proxy%20path/responses?x=a%2Bb&url=https://x.test/a?b=1",
        ),
    ] {
        assert_eq!(url(base, Protocol::OpenAiResponses, None, false), expected);
    }
}

#[test]
fn schema_version_is_validated_separately_from_http_path() {
    let mut format = Format::new(Protocol::GeminiGenerateContent);
    format.version = "v1";
    let endpoint = HttpEndpoint::new("https://example.test/v1beta", format).unwrap();
    assert_eq!(endpoint.base_url(), "https://example.test/v1beta");
    assert_eq!(endpoint.format(), format);
    let mut output = [0; 128];
    assert_eq!(
        endpoint.url_into(Some("m"), false, &mut output).unwrap(),
        "https://example.test/v1beta/models/m:generateContent"
    );
    format.version = "unknown-version";
    assert!(matches!(
        HttpEndpoint::new("https://example.test", format),
        Err(EndpointError::InvalidFormat(_))
    ));
}

#[test]
fn endpoint_error_implements_error_without_exposing_the_url() {
    fn require_error<T: std::error::Error>() {}
    require_error::<EndpointError>();
    let error =
        HttpEndpoint::new("https://secret@example.test", Protocol::OpenAiChat.into()).unwrap_err();
    assert!(!error.to_string().contains("secret"));
}
