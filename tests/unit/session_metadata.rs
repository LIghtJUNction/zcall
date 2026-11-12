extern crate std;
use super::*;
use std::vec::Vec;

fn span(offset: usize, length: usize) -> wire::EventText {
    wire::EventText {
        offset,
        length,
        quoted: false,
        present: true,
    }
}

#[test]
fn signature_labels_are_explicit_and_only_absence_inherits_source() {
    let fallback = Format {
        profile: crate::common::Profile::OpenAiCompatible("source"),
        ..Protocol::OpenAiChat.into()
    };
    for (label, protocol) in [
        ("anthropic", Protocol::AnthropicMessages),
        ("gemini", Protocol::GeminiGenerateContent),
        ("chat", Protocol::OpenAiChat),
        ("responses", Protocol::OpenAiResponses),
    ] {
        assert_eq!(
            signature_origin(label.as_bytes(), span(0, label.len()), fallback).unwrap(),
            protocol.into()
        );
    }
    assert_eq!(
        signature_origin(b"", wire::EventText::default(), fallback).unwrap(),
        fallback
    );
    assert!(signature_origin(b"unknown", span(0, 7), fallback).is_err());
}

#[test]
fn signature_origin_survives_event_encoder_payload_repacking() {
    let source = Format {
        profile: crate::common::Profile::OpenAiCompatible("metadata"),
        ..Protocol::OpenAiChat.into()
    };
    let mut pending = [0; 4096];
    let mut aggregate = [0; 4096];
    let mut scratch = [0; 256];
    let mut workspace = EncoderWorkspace::<64, 8>::new();
    let mut config = EventEncoderConfig::new(source, source);
    config.context.response_id = Some("r");
    config.context.model = Some("m");
    config.context.created = Some(7);
    let mut encoder = EventEncoder::new(
        config,
        &mut pending,
        &mut aggregate,
        &mut scratch,
        &mut workspace,
    )
    .unwrap();
    let key = EventKey::default();
    let raw_events = [
        wire::Event::EMPTY,
        wire::Event {
            kind: wire::EventKind::BlockStart,
            ..wire::Event::EMPTY
        },
        wire::Event {
            kind: wire::EventKind::Signature,
            a: span(6, 6),
            b: span(0, 6),
            ..wire::Event::EMPTY
        },
    ];
    let data = [
        EventData::Started {
            response_id: None,
            model: None,
            created: None,
        },
        EventData::BlockStarted {
            key,
            kind: BlockType::Text,
            call_id: None,
            name: None,
            item_id: None,
        },
        EventData::Signature {
            key,
            signature: Text::new("signed"),
            origin: Protocol::GeminiGenerateContent.into(),
        },
    ];
    let mut output = Vec::new();
    for (raw, data) in raw_events.iter().zip(data) {
        let status = encoder.push(Event {
            source,
            data,
            replay: Some((raw, b"geminisigned")),
        });
        assert!(
            matches!(status, StageStatus::Accepted(_)),
            "{data:?}: {status:?}"
        );
        let mut bytes = [0; 512];
        loop {
            let step = encoder.drain(&mut bytes);
            output.extend_from_slice(&bytes[..step.written]);
            match step.status {
                EncoderStatus::NeedOutput => {}
                EncoderStatus::Ready => break,
                other => panic!("{other:?}"),
            }
        }
    }
    let output = std::str::from_utf8(&output).unwrap();
    assert!(output.contains("\"protocol\":\"gemini\""), "{output}");
    assert!(output.contains("\"signature\":\"signed\""), "{output}");
}
