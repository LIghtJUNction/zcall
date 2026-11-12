use zcall::v1::*;

const CHAT: &str = concat!(
    "data: {\"id\":\"response_9\",\"model\":\"model-x\",\"created\":9,\"choices\":[{\"index\":0,\"delta\":{\"role\":\"assistant\",\"content\":\"Hello \\u4e2d🌍\"},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
    "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8}}\n\n",
    "data: [DONE]\n\n"
);
const TOOLS: &str = concat!(
    "data: {\"id\":\"response_9\",\"model\":\"model-x\",\"created\":9,\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":0,\"id\":\"call_a\",\"type\":\"function\",\"function\":{\"name\":\"lookup\",\"arguments\":\"{\\\"a\\\":\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"id\":\"call_b\",\"type\":\"function\",\"function\":{\"name\":\"save\",\"arguments\":\"{\\\"b\\\":\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{\"tool_calls\":[{\"index\":1,\"function\":{\"arguments\":\"2}\"}},{\"index\":0,\"function\":{\"arguments\":\"1}\"}}]},\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":4,\"total_tokens\":9}}\n\n",
    "data: [DONE]\n\n"
);

const ANTHROPIC: &str = concat!(
    "event: message_start\ndata: {\"type\":\"message_start\",\"message\":{\"id\":\"response_9\",\"model\":\"model-x\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":5,\"output_tokens\":0}}}\n\n",
    "event: content_block_start\ndata: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"\"}}\n\n",
    "event: content_block_delta\ndata: {\"type\":\"content_block_delta\",\"index\":0,\"delta\":{\"type\":\"text_delta\",\"text\":\"Hello \\u4e2d🌍\"}}\n\n",
    "event: content_block_stop\ndata: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
    "event: message_delta\ndata: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":3}}\n\n",
    "event: message_stop\ndata: {\"type\":\"message_stop\"}\n\n"
);
const RESPONSES: &str = concat!(
    "event: response.created\ndata: {\"type\":\"response.created\",\"response\":{\"id\":\"response_9\",\"model\":\"model-x\",\"created_at\":9}}\n\n",
    "event: response.output_item.added\ndata: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"item_original\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
    "event: response.content_part.added\ndata: {\"type\":\"response.content_part.added\",\"output_index\":0,\"content_index\":0,\"item_id\":\"item_original\",\"part\":{\"type\":\"output_text\",\"text\":\"\",\"annotations\":[]}}\n\n",
    "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"output_index\":0,\"content_index\":0,\"delta\":\"Hello \\u4e2d🌍\"}\n\n",
    "event: response.output_text.done\ndata: {\"type\":\"response.output_text.done\",\"output_index\":0,\"content_index\":0,\"text\":\"Hello \\u4e2d🌍\"}\n\n",
    "event: response.output_item.done\ndata: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"item_original\",\"type\":\"message\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello \\u4e2d🌍\"}]}}\n\n",
    "event: response.completed\ndata: {\"type\":\"response.completed\",\"response\":{\"usage\":{\"input_tokens\":5,\"output_tokens\":3,\"total_tokens\":8},\"output\":[{\"id\":\"item_original\",\"type\":\"message\",\"content\":[{\"type\":\"output_text\",\"text\":\"Hello \\u4e2d🌍\"}]}]}}\n\n"
);
const GEMINI: &str = concat!(
    "data: {\"responseId\":\"response_9\",\"modelVersion\":\"model-x\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"Hello \\u4e2d🌍\"}]}}]}\n\n",
    "data: {\"candidates\":[{\"index\":0,\"finishReason\":\"STOP\"}],\"usageMetadata\":{\"promptTokenCount\":5,\"candidatesTokenCount\":3,\"totalTokenCount\":8}}\n\n"
);
const INTERACTIONS: &str = concat!(
    "data: {\"event_type\":\"interaction.created\",\"interaction\":{\"id\":\"response_9\",\"model\":\"model-x\"}}\n\n",
    "data: {\"event_type\":\"step.start\",\"index\":0,\"step\":{\"type\":\"model_output\",\"content\":[]}}\n\n",
    "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"text\",\"text\":\"Hello \\u4e2d🌍\"}}\n\n",
    "data: {\"event_type\":\"step.stop\",\"index\":0}\n\n",
    "data: {\"event_type\":\"interaction.completed\",\"interaction\":{\"status\":\"completed\",\"usage\":{\"total_input_tokens\":5,\"total_output_tokens\":3,\"total_tokens\":8}}}\n\n"
);
const COMPLETIONS: &str = concat!(
    "data: {\"id\":\"response_9\",\"model\":\"model-x\",\"created\":9,\"choices\":[{\"index\":0,\"text\":\"Hello \\u4e2d🌍\",\"finish_reason\":null}]}\n\n",
    "data: {\"choices\":[{\"index\":0,\"text\":\"\",\"finish_reason\":\"stop\"}],\"usage\":{\"prompt_tokens\":5,\"completion_tokens\":3,\"total_tokens\":8}}\n\n",
    "data: [DONE]\n\n"
);

#[derive(Default, Debug)]
struct Observed {
    text: String,
    starts: usize,
    terminals: Vec<TurnOutcome>,
    tools: Vec<(EventKey, String, String, Vec<u8>)>,
    usage: StreamUsage,
    error: Option<Error>,
    snapshots: usize,
    losses: Vec<StreamLoss>,
}
fn text(value: Text<'_>) -> String {
    let mut data = vec![0; value.decoded_len()];
    value.decode_into(&mut data).unwrap();
    String::from_utf8(data).unwrap()
}
fn record(observed: &mut Observed, event: Event<'_>) {
    match event.data() {
        EventData::Started { .. } => observed.starts += 1,
        EventData::Delta {
            kind: BlockType::Text,
            text: value,
            ..
        } => observed.text.push_str(&text(value)),
        EventData::ToolReady(tool) => observed.tools.push((
            tool.key,
            tool.call_id.into(),
            tool.name.into(),
            tool.arguments.as_bytes().to_vec(),
        )),
        EventData::Usage(usage) => observed.usage = usage,
        EventData::Terminal(terminal) => {
            observed.terminals.push(terminal.outcome);
            observed.usage = terminal.usage;
        }
        EventData::SnapshotVerified { .. } => observed.snapshots += 1,
        EventData::Loss { field, .. } => observed.losses.push(field),
        _ => {}
    }
}
fn observe(config: SessionConfig<'_>, input: &[u8], split: usize, pause: bool) -> Observed {
    let mut framing = vec![0; 16384];
    let mut payload = vec![0; 16384];
    let mut aggregate = vec![0; 32768];
    let mut workspace = StreamWorkspace::<512, 96, 32>::new();
    let mut session = Session::new(
        config,
        &mut framing,
        &mut payload,
        &mut aggregate,
        &mut workspace,
    )
    .unwrap();
    session.prepare().unwrap();
    let mut result = Observed::default();
    let mut at = 0;
    let mut reject = pause;
    for _ in 0..input.len().saturating_mul(8) + 1000 {
        let end = (at + split).min(input.len());
        let step = session.step(&input[at..end], end == input.len(), |event| {
            if reject {
                reject = false;
                return Control::Pause;
            }
            record(&mut result, event);
            reject = pause;
            Control::Continue
        });
        at += step.consumed;
        match step.status {
            StepStatus::Done if at == input.len() => return result,
            StepStatus::Failed(error) => {
                result.error = Some(error);
                return result;
            }
            StepStatus::NeedBuffer(capacity) => panic!("unexpected capacity: {capacity:?}"),
            _ => {}
        }
    }
    panic!("no progress: {at}/{}", input.len())
}
fn config(protocol: Protocol) -> SessionConfig<'static> {
    SessionConfig::new(protocol.into())
}

#[test]
fn six_real_adapters_feed_identical_harness_code_under_every_byte_split() {
    for (source, input) in [
        (Protocol::OpenAiChat, CHAT),
        (Protocol::OpenAiResponses, RESPONSES),
        (Protocol::AnthropicMessages, ANTHROPIC),
        (Protocol::GeminiGenerateContent, GEMINI),
        (Protocol::GeminiInteractions, INTERACTIONS),
        (Protocol::OpenAiCompletions, COMPLETIONS),
    ] {
        for split in [1, 2, 3, 7, 31, 4096] {
            let result = observe(config(source), input.as_bytes(), split, true);
            assert!(result.error.is_none(), "{source:?}/{split}: {result:?}");
            assert_eq!(result.text, "Hello 中🌍", "{source:?}/{split}");
            assert_eq!(result.starts, 1);
            assert_eq!(result.terminals, [TurnOutcome::Completed]);
            assert_eq!(result.usage.input, Some(5));
            assert_eq!(result.usage.output, Some(3));
        }
    }
}

#[test]
fn crlf_and_utf8_escapes_can_split_at_any_byte_including_terminal_crlf() {
    let crlf = CHAT.replace('\n', "\r\n");
    for split in 1..=crlf.len() {
        let result = observe(config(Protocol::OpenAiChat), crlf.as_bytes(), split, false);
        assert!(result.error.is_none(), "split {split}: {result:?}");
        assert_eq!(result.text, "Hello 中🌍");
        assert_eq!(result.terminals.len(), 1);
    }
}

#[test]
fn zero_capacity_regions_recover_without_replaying_accepted_events() {
    let mut empty_f = [];
    let mut empty_p = [];
    let mut empty_a = [];
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = [0; 4096];
    let mut w0 = StreamWorkspace::<0, 0, 0>::new();
    let mut wt = StreamWorkspace::<128, 0, 0>::new();
    let mut we = StreamWorkspace::<128, 32, 0>::new();
    let mut wi = StreamWorkspace::<128, 32, 8>::new();
    let mut f = Some(&mut f[..]);
    let mut p = Some(&mut p[..]);
    let mut a = Some(&mut a[..]);
    let mut wt = Some(&mut wt);
    let mut we = Some(&mut we);
    let mut wi = Some(&mut wi);
    let mut session = Session::new(
        config(Protocol::OpenAiChat),
        &mut empty_f,
        &mut empty_p,
        &mut empty_a,
        &mut w0,
    )
    .unwrap();
    session.prepare().unwrap();
    let mut at = 0;
    let mut seen = Vec::new();
    let mut result = Observed::default();
    loop {
        let step = session.step(&CHAT.as_bytes()[at..], true, |event| {
            record(&mut result, event);
            Control::Continue
        });
        at += step.consumed;
        match step.status {
            StepStatus::NeedBuffer(c) => {
                assert_eq!(c.replay, Replay::Retained);
                seen.push(c.region);
                match c.region {
                    Resource::Framing => {
                        session.replace_buffer(c.region, f.take().unwrap()).unwrap();
                    }
                    Resource::Payload => {
                        session.replace_buffer(c.region, p.take().unwrap()).unwrap();
                    }
                    Resource::Aggregate => {
                        session.replace_buffer(c.region, a.take().unwrap()).unwrap();
                    }
                    Resource::Tokens => session.replace_workspace(wt.take().unwrap()).unwrap(),
                    Resource::Events => session.replace_workspace(we.take().unwrap()).unwrap(),
                    Resource::Items => session.replace_workspace(wi.take().unwrap()).unwrap(),
                    _ => panic!("{c:?}"),
                }
            }
            StepStatus::Done => break,
            other => panic!("{other:?}"),
        }
    }
    for region in [
        Resource::Framing,
        Resource::Payload,
        Resource::Aggregate,
        Resource::Tokens,
        Resource::Events,
        Resource::Items,
    ] {
        assert!(seen.contains(&region), "missing {region:?}");
    }
    assert_eq!(at, CHAT.len());
    assert_eq!(result.starts, 1);
    assert_eq!(result.text, "Hello 中🌍");
    assert_eq!(result.terminals, [TurnOutcome::Completed]);
}

#[test]
fn interleaved_tools_are_executable_once_and_results_are_linked_by_id() {
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = [0; 4096];
    let mut workspace = StreamWorkspace::<256, 64, 8>::new();
    let mut session = Session::new(
        config(Protocol::OpenAiChat),
        &mut f,
        &mut p,
        &mut a,
        &mut workspace,
    )
    .unwrap();
    session.prepare().unwrap();
    assert_eq!(
        session.submit_tool_result("call_a").unwrap_err().kind,
        ErrorKind::Lifecycle
    );
    let mut result = Observed::default();
    let step = session.step(TOOLS.as_bytes(), true, |event| {
        record(&mut result, event);
        Control::Continue
    });
    assert_eq!(step.status, StepStatus::Done, "{result:?}");
    assert_eq!(session.state(), CallState::WaitingTools);
    assert_eq!(session.pending_tool_count(), 2);
    assert_eq!(
        session.tool("call_a").unwrap().arguments.as_bytes(),
        br#"{"a":1}"#
    );
    assert_eq!(result.terminals, [TurnOutcome::WaitingTools]);
    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.tools[0].1, "call_a");
    assert_eq!(result.tools[0].2, "lookup");
    assert_eq!(result.tools[0].3, br#"{"a":1}"#);
    assert_eq!(result.tools[1].1, "call_b");
    assert_eq!(result.tools[1].3, br#"{"b":2}"#);
    assert_eq!(
        session.submit_tool_result("missing").unwrap_err().kind,
        ErrorKind::InvalidInput
    );
    session.submit_tool_result("call_a").unwrap();
    assert_eq!(
        session.submit_tool_result("call_a").unwrap_err().kind,
        ErrorKind::Lifecycle
    );
    session.submit_tool_result("call_b").unwrap();
    assert_eq!(session.state(), CallState::ToolsResolved);
    assert_eq!(session.pending_tool_count(), 0);
    let again = session.step(&[], true, |_| panic!("repeated terminal"));
    assert_eq!(again.status, StepStatus::Done);
    session.reset().unwrap();
    assert_eq!(session.state(), CallState::Idle);
}

#[test]
fn malformed_completed_arguments_never_become_ready_but_truncation_is_incomplete() {
    let invalid = TOOLS.replace("1}", "1,");
    let result = observe(config(Protocol::OpenAiChat), invalid.as_bytes(), 1, true);
    assert_eq!(result.error.unwrap().kind, ErrorKind::InvalidJson);
    assert!(result.tools.is_empty());
    assert_eq!(result.terminals, [TurnOutcome::LocalFailure]);
    let incomplete = invalid.replace(
        "\"finish_reason\":\"tool_calls\"",
        "\"finish_reason\":\"length\"",
    );
    let result = observe(config(Protocol::OpenAiChat), incomplete.as_bytes(), 3, true);
    assert!(result.error.is_none());
    assert!(result.tools.is_empty());
    assert_eq!(result.terminals, [TurnOutcome::Incomplete]);
}

#[test]
fn cumulative_cache_and_thinking_updates_never_add_prior_totals_twice() {
    let cached = ANTHROPIC.replace("\"input_tokens\":5,\"output_tokens\":0", "\"input_tokens\":5,\"output_tokens\":0,\"cache_read_input_tokens\":7,\"cache_creation_input_tokens\":2")
        .replace("\"output_tokens\":3", "\"output_tokens\":3,\"cache_read_input_tokens\":9");
    let result = observe(
        config(Protocol::AnthropicMessages),
        cached.as_bytes(),
        1,
        true,
    );
    assert!(result.error.is_none());
    assert_eq!(result.usage.input, Some(16));
    assert_eq!(result.usage.output, Some(3));
    assert_eq!(result.usage.total, Some(19));
    assert_eq!(result.usage.cached, Some(9));
    let thoughts = GEMINI
        .replace(
            "\"candidatesTokenCount\":3",
            "\"candidatesTokenCount\":3,\"thoughtsTokenCount\":4",
        )
        .replace("\"totalTokenCount\":8", "\"totalTokenCount\":12");
    let result = observe(
        config(Protocol::GeminiGenerateContent),
        thoughts.as_bytes(),
        1,
        true,
    );
    assert!(result.error.is_none());
    assert_eq!(result.usage.output, Some(7));
    assert_eq!(result.usage.reasoning, Some(4));
}

#[test]
fn final_snapshots_are_verified_without_appending_again() {
    let result = observe(
        config(Protocol::OpenAiResponses),
        RESPONSES.as_bytes(),
        1,
        true,
    );
    assert_eq!(result.text, "Hello 中🌍");
    assert_eq!(result.snapshots, 3);
    let wrong = RESPONSES.replacen(
        "\"text\":\"Hello \\u4e2d🌍\"",
        "\"text\":\"Jello \\u4e2d🌍\"",
        1,
    );
    let result = observe(config(Protocol::OpenAiResponses), wrong.as_bytes(), 1, true);
    assert_eq!(result.error.unwrap().field, "snapshot");
    assert_eq!(result.text, "Hello 中🌍");
}

#[test]
fn cancellation_intent_confirmation_and_transport_failure_are_distinct() {
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = [0; 4096];
    let mut w = StreamWorkspace::<256, 64, 8>::new();
    let mut session =
        Session::new(config(Protocol::OpenAiChat), &mut f, &mut p, &mut a, &mut w).unwrap();
    session.prepare().unwrap();
    session.request_cancel().unwrap();
    assert_eq!(session.state(), CallState::CancelRequested);
    let end = CHAT.find("\n\n").unwrap() + 2;
    let step = session.step(&CHAT.as_bytes()[..end], false, |_| Control::Continue);
    assert_eq!(step.status, StepStatus::NeedInput);
    assert_eq!(session.state(), CallState::CancelRequested);
    session.confirm_cancel().unwrap();
    let mut outcomes = Vec::new();
    assert_eq!(
        session
            .step(&[], false, |event| {
                if let EventData::Terminal(t) = event.data() {
                    outcomes.push(t.outcome);
                }
                Control::Continue
            })
            .status,
        StepStatus::Done
    );
    assert_eq!(outcomes, [TurnOutcome::Cancelled]);
    assert_eq!(
        session
            .step(&[], false, |_| panic!("duplicate terminal"))
            .status,
        StepStatus::Done
    );
    assert!(matches!(
        session.step(b"x", false, |_| panic!("late event")).status,
        StepStatus::Failed(Error {
            kind: ErrorKind::Lifecycle,
            ..
        })
    ));
    session.reset().unwrap();
    session.prepare().unwrap();
    session.transport_failed("connection reset").unwrap();
    let step = session.step(&[], false, |event| {
        if let EventData::Terminal(t) = event.data() {
            assert_eq!(t.error.unwrap().kind, ErrorKind::Transport);
        }
        Control::Continue
    });
    assert!(matches!(
        step.status,
        StepStatus::Failed(Error {
            kind: ErrorKind::Transport,
            ..
        })
    ));
}

#[test]
fn eof_duplicate_termination_and_invalid_event_names_have_stable_errors() {
    let no_done = CHAT.strip_suffix("data: [DONE]\n\n").unwrap();
    let result = observe(config(Protocol::OpenAiChat), no_done.as_bytes(), 1, false);
    assert_eq!(result.error.unwrap().kind, ErrorKind::Truncated);
    assert_eq!(result.terminals.len(), 1);
    let duplicate = format!("{CHAT}data: [DONE]\n\n");
    let result = observe(
        config(Protocol::OpenAiChat),
        duplicate.as_bytes(),
        duplicate.len(),
        false,
    );
    assert_eq!(result.error.unwrap().kind, ErrorKind::Lifecycle);
    assert_eq!(result.terminals, [TurnOutcome::Completed]);
    let mismatch = ANTHROPIC.replacen("event: message_start", "event: message_stop", 1);
    let result = observe(
        config(Protocol::AnthropicMessages),
        mismatch.as_bytes(),
        7,
        false,
    );
    assert_eq!(result.error.unwrap().kind, ErrorKind::InvalidInput);
}

#[test]
fn upstream_error_is_a_terminal_event_not_a_json_failure() {
    let input = b"data: {\"error\":{\"code\":\"overloaded\",\"message\":\"Try later\"}}\n\n";
    let result = observe(config(Protocol::OpenAiChat), input, 1, true);
    assert!(result.error.is_none());
    assert_eq!(result.terminals, [TurnOutcome::UpstreamFailed]);
    let invalid = observe(
        config(Protocol::OpenAiChat),
        b"data: {bad json}\n\n",
        1,
        true,
    );
    assert_eq!(invalid.error.unwrap().kind, ErrorKind::InvalidJson);
    assert_eq!(invalid.terminals, [TurnOutcome::LocalFailure]);
}

#[test]
fn unknown_metadata_requires_explicit_field_permission_and_reports_its_name() {
    let input = CHAT.replacen(
        "\"id\":\"response_9\"",
        "\"system_fingerprint\":\"fp-x\",\"id\":\"response_9\"",
        1,
    );
    let strict = observe(config(Protocol::OpenAiChat), input.as_bytes(), 9, false);
    assert_eq!(strict.error.unwrap().kind, ErrorKind::ForbiddenLoss);
    let mut permit = config(Protocol::OpenAiChat);
    permit.allowed_losses = &[StreamLoss::SystemFingerprint];
    let allowed = observe(permit, input.as_bytes(), 9, true);
    assert!(allowed.error.is_none());
    assert_eq!(allowed.losses, [StreamLoss::SystemFingerprint]);
}

fn encode(
    source: Protocol,
    target: Protocol,
    input: &[u8],
    allow_item_id_loss: bool,
) -> Result<(Vec<u8>, EventEncodingReport), Error> {
    encode_formats(source.into(), target.into(), input, allow_item_id_loss)
}

fn encode_formats(
    source: Format,
    target: Format,
    input: &[u8],
    allow_item_id_loss: bool,
) -> Result<(Vec<u8>, EventEncodingReport), Error> {
    let mut f = vec![0; 8192];
    let mut p = vec![0; 8192];
    let mut a = vec![0; 32768];
    let mut w = StreamWorkspace::<512, 96, 32>::new();
    let mut session =
        Session::new(SessionConfig::new(source), &mut f, &mut p, &mut a, &mut w).unwrap();
    session.prepare().unwrap();
    let mut pending = vec![0; 32768];
    let mut ea = vec![0; 32768];
    let mut scratch = vec![0; 8192];
    let mut ew = EncoderWorkspace::<512, 32>::new();
    let mut ec = EventEncoderConfig::new(source, target);
    ec.context.created = Some(9);
    ec.context.initial_usage.input = Some(5);
    ec.context.target_item_id_prefix = Some("caller_");
    ec.allow_timestamp_loss = true;
    ec.allow_item_id_loss = allow_item_id_loss;
    let mut encoder = EventEncoder::new(ec, &mut pending, &mut ea, &mut scratch, &mut ew).unwrap();
    let mut output = Vec::new();
    let mut failure = None;
    let step = session.step(input, true, |event| {
        if failure.is_some() {
            return Control::Pause;
        }
        match encoder.push(event) {
            StageStatus::Accepted(_) => {}
            StageStatus::Failed(error) => {
                failure = Some(error);
                return Control::Pause;
            }
            other => panic!("stage: {other:?}"),
        }
        let mut one = [0; 1];
        loop {
            let step = encoder.drain(&mut one);
            output.extend_from_slice(&one[..step.written]);
            match step.status {
                EncoderStatus::NeedOutput => {}
                EncoderStatus::Ready | EncoderStatus::Done => break,
                EncoderStatus::Failed(error) => {
                    failure = Some(error);
                    return Control::Pause;
                }
                other => panic!("drain: {other:?}"),
            }
        }
        Control::Continue
    });
    if let Some(error) = failure {
        return Err(error);
    }
    assert_eq!(step.status, StepStatus::Done);
    Ok((output, encoder.report()))
}

#[test]
fn uniform_events_encode_to_all_six_targets_with_one_byte_output() {
    for target in [
        Protocol::OpenAiChat,
        Protocol::OpenAiCompletions,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ] {
        let (output, report) =
            encode(Protocol::OpenAiChat, target, CHAT.as_bytes(), false).unwrap();
        let observed = observe(config(target), &output, 1, true);
        assert!(
            observed.error.is_none(),
            "{target:?}: {observed:?}\n{}",
            String::from_utf8_lossy(&output)
        );
        assert_eq!(observed.text, "Hello 中🌍", "{target:?}");
        assert_eq!(observed.terminals, [TurnOutcome::Completed]);
        assert!(report.accepted_events > 0);
    }
}

#[test]
fn exact_item_id_loss_is_explicit_in_reverse_event_encoding() {
    let error = encode(
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
        RESPONSES.as_bytes(),
        false,
    )
    .unwrap_err();
    assert_eq!(error.kind, ErrorKind::ForbiddenLoss);
    assert_eq!(error.field, "item.id");
    let (output, report) = encode(
        Protocol::OpenAiResponses,
        Protocol::OpenAiChat,
        RESPONSES.as_bytes(),
        true,
    )
    .unwrap();
    assert_eq!(report.dropped_item_ids, 1);
    let result = observe(config(Protocol::OpenAiChat), &output, 1, true);
    assert!(result.error.is_none());
    assert_eq!(result.text, "Hello 中🌍");
}

#[test]
fn tool_event_encoding_preserves_ids_and_complete_arguments() {
    for target in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ] {
        let (output, _) = encode(Protocol::OpenAiChat, target, TOOLS.as_bytes(), false).unwrap();
        let result = observe(config(target), &output, 1, true);
        assert!(
            result.error.is_none(),
            "{target:?}: {result:?}\n{}",
            String::from_utf8_lossy(&output)
        );
        assert_eq!(result.tools.len(), 2);
        assert_eq!(result.tools[0].1, "call_a");
        assert_eq!(result.tools[0].3, br#"{"a":1}"#);
        assert_eq!(result.tools[1].1, "call_b");
        assert_eq!(result.tools[1].3, br#"{"b":2}"#);
        assert_eq!(result.terminals, [TurnOutcome::WaitingTools]);
    }
}

#[test]
fn arbitrary_input_and_duplicate_fields_never_panic_or_spin() {
    let mut seed = 0x1234_5678_9abc_def0u64;
    for source in [
        Protocol::OpenAiChat,
        Protocol::OpenAiCompletions,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ] {
        for len in 0..96 {
            let mut input = Vec::from(&b"data: "[..]);
            for _ in 0..len {
                seed ^= seed << 13;
                seed ^= seed >> 7;
                seed ^= seed << 17;
                input.push(seed as u8);
            }
            input.extend_from_slice(b"\n\n");
            let result = observe(config(source), &input, 1, false);
            assert!(result.error.is_some() || !result.terminals.is_empty());
        }
    }
    let duplicate = b"data: {\"choices\":[],\"choices\":[]}\n\n";
    assert_eq!(
        observe(config(Protocol::OpenAiChat), duplicate, 1, false)
            .error
            .unwrap()
            .kind,
        ErrorKind::InvalidJson
    );
}

#[test]
fn same_gemini_part_index_in_later_frames_is_a_new_tool_with_its_own_id() {
    let input = concat!(
        "data: {\"responseId\":\"response_9\",\"modelVersion\":\"model-x\",\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"functionCall\":{\"id\":\"a\",\"name\":\"f\",\"args\":{\"x\":1}}}]}}]}\n\n",
        "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"functionCall\":{\"id\":\"b\",\"name\":\"g\",\"args\":{\"y\":2}}}]},\"finishReason\":\"STOP\"}]}\n\n"
    );
    let result = observe(
        config(Protocol::GeminiGenerateContent),
        input.as_bytes(),
        1,
        true,
    );
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.tools.len(), 2);
    assert_ne!(result.tools[0].0, result.tools[1].0);
    assert_eq!(result.tools[0].1, "a");
    assert_eq!(result.tools[1].1, "b");
    assert_eq!(result.terminals, [TurnOutcome::WaitingTools]);
}

#[test]
fn tool_call_context_is_required_and_never_invented() {
    let legacy = concat!(
        "data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"function_call\":{\"name\":\"f\",\"arguments\":\"{}\"}},\"finish_reason\":\"function_call\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let missing = observe(config(Protocol::OpenAiChat), legacy.as_bytes(), 1, false);
    assert_eq!(missing.error.unwrap().kind, ErrorKind::ContextRequired);
    assert!(missing.tools.is_empty());
    let ids = [ToolIdentity {
        key: EventKey {
            choice: 0,
            item: 0,
            part: 0,
        },
        call_id: "harness_call_4",
    }];
    let mut supplied = config(Protocol::OpenAiChat);
    supplied.tool_ids = &ids;
    let result = observe(supplied, legacy.as_bytes(), 1, true);
    assert!(result.error.is_none());
    assert_eq!(result.tools[0].1, "harness_call_4");
}

#[test]
fn active_choice_must_finish_and_tool_arguments_must_be_an_object() {
    let choices = concat!(
        "data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"a\"},\"finish_reason\":\"stop\"},{\"index\":1,\"delta\":{\"content\":\"b\"},\"finish_reason\":null}]}\n\n",
        "data: [DONE]\n\n"
    );
    let result = observe(config(Protocol::OpenAiChat), choices.as_bytes(), 1, true);
    assert_eq!(result.error.unwrap().kind, ErrorKind::Truncated);
    assert_eq!(result.text, "ab");
    let array = TOOLS.replace("{\\\"a\\\":", "[").replace("1}", "1]");
    let result = observe(config(Protocol::OpenAiChat), array.as_bytes(), 1, true);
    assert_eq!(result.error.unwrap().field, "tool arguments");
    assert!(result.tools.is_empty());
}

#[test]
fn aggregation_growth_at_tool_completion_retains_committed_fragments() {
    let cut = TOOLS
        .find("data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"tool_calls\"")
        .unwrap();
    let used = {
        let mut f = [0; 2048];
        let mut p = [0; 2048];
        let mut a = [0; 4096];
        let mut w = StreamWorkspace::<256, 64, 8>::new();
        let mut session =
            Session::new(config(Protocol::OpenAiChat), &mut f, &mut p, &mut a, &mut w).unwrap();
        session.prepare().unwrap();
        assert_eq!(
            session
                .step(&TOOLS.as_bytes()[..cut], false, |_| Control::Continue)
                .status,
            StepStatus::NeedInput
        );
        session.memory().aggregate_used
    };
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = vec![0; used];
    let mut larger = [0; 4096];
    let mut w = StreamWorkspace::<256, 64, 8>::new();
    let mut session =
        Session::new(config(Protocol::OpenAiChat), &mut f, &mut p, &mut a, &mut w).unwrap();
    session.prepare().unwrap();
    let mut result = Observed::default();
    let first = session.step(TOOLS.as_bytes(), true, |event| {
        record(&mut result, event);
        Control::Continue
    });
    assert!(matches!(
        first.status,
        StepStatus::NeedBuffer(Capacity {
            region: Resource::Aggregate,
            ..
        })
    ));
    assert!(result.tools.is_empty());
    session
        .replace_buffer(Resource::Aggregate, &mut larger)
        .unwrap();
    let second = session.step(&TOOLS.as_bytes()[first.consumed..], true, |event| {
        record(&mut result, event);
        Control::Continue
    });
    assert_eq!(second.status, StepStatus::Done);
    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.tools[0].3, br#"{"a":1}"#);
    assert_eq!(result.tools[1].3, br#"{"b":2}"#);
}

#[test]
fn encoder_zero_buffers_recover_atomically_and_zero_output_backpressures() {
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = [0; 4096];
    let mut w = StreamWorkspace::<256, 64, 8>::new();
    let mut session =
        Session::new(config(Protocol::OpenAiChat), &mut f, &mut p, &mut a, &mut w).unwrap();
    session.prepare().unwrap();
    let mut empty_o = [];
    let mut empty_a = [];
    let mut empty_s = [];
    let mut w0 = EncoderWorkspace::<0, 0>::new();
    let mut ew = EncoderWorkspace::<256, 8>::new();
    let mut output_store = [0; 8192];
    let mut aggregate_store = [0; 8192];
    let mut scratch_store = [0; 2048];
    let mut eo = Some(&mut output_store[..]);
    let mut ea = Some(&mut aggregate_store[..]);
    let mut es = Some(&mut scratch_store[..]);
    let mut ew = Some(&mut ew);
    let mut ec = EventEncoderConfig::new(
        Protocol::OpenAiChat.into(),
        Protocol::OpenAiResponses.into(),
    );
    ec.context.target_item_id_prefix = Some("target_");
    let mut encoder =
        EventEncoder::new(ec, &mut empty_o, &mut empty_a, &mut empty_s, &mut w0).unwrap();
    let mut output = Vec::new();
    let mut grew = Vec::new();
    let mut zero_output_seen = false;
    let step = session.step(TOOLS.as_bytes(), true, |event| {
        loop {
            match encoder.push(event) {
                StageStatus::Accepted(_) => break,
                StageStatus::NeedBuffer(c) => {
                    grew.push(c.region);
                    match c.region {
                        Resource::Output => {
                            encoder
                                .replace_buffer(c.region, eo.take().unwrap())
                                .unwrap();
                        }
                        Resource::Aggregate => {
                            encoder
                                .replace_buffer(c.region, ea.take().unwrap())
                                .unwrap();
                        }
                        Resource::Scratch => {
                            encoder
                                .replace_buffer(c.region, es.take().unwrap())
                                .unwrap();
                        }
                        Resource::Items | Resource::Tokens => {
                            encoder.replace_workspace(ew.take().unwrap()).unwrap()
                        }
                        _ => panic!("{c:?}"),
                    }
                }
                other => panic!("{other:?}"),
            }
        }
        let zero = encoder.drain(&mut []);
        assert_eq!(zero.written, 0);
        zero_output_seen |= zero.status == EncoderStatus::NeedOutput;
        let mut one = [0; 1];
        loop {
            let step = encoder.drain(&mut one);
            output.extend_from_slice(&one[..step.written]);
            match step.status {
                EncoderStatus::Ready | EncoderStatus::Done => break,
                EncoderStatus::NeedOutput => {}
                other => panic!("{other:?}"),
            }
        }
        Control::Continue
    });
    assert_eq!(step.status, StepStatus::Done);
    assert!(zero_output_seen);
    for region in [
        Resource::Output,
        Resource::Aggregate,
        Resource::Scratch,
        Resource::Items,
    ] {
        assert!(grew.contains(&region));
    }
    let result = observe(config(Protocol::OpenAiResponses), &output, 1, true);
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.tools.len(), 2);
}

#[test]
fn gemini_missing_ids_bind_source_positions_before_normalizing_item_indices() {
    let input = concat!(
        "data: {\"responseId\":\"r\",\"modelVersion\":\"m\",\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"text\":\"prefix\"}]}}]}\n\n",
        "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"f\",\"args\":{}}}]}}]}\n\n",
        "data: {\"candidates\":[{\"index\":0,\"content\":{\"parts\":[{\"functionCall\":{\"name\":\"g\",\"args\":{}}}]},\"finishReason\":\"STOP\"}]}\n\n"
    );
    let ids = [
        ToolIdentity {
            key: EventKey {
                choice: 0,
                item: 0,
                part: 0,
            },
            call_id: "provided_a",
        },
        ToolIdentity {
            key: EventKey {
                choice: 0,
                item: 0,
                part: 0,
            },
            call_id: "provided_b",
        },
    ];
    let mut configured = config(Protocol::GeminiGenerateContent);
    configured.tool_ids = &ids;
    let result = observe(configured, input.as_bytes(), 1, true);
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.tools.len(), 2);
    assert_eq!(result.tools[0].0.item, 1);
    assert_eq!(result.tools[1].0.item, 2);
    assert_eq!(result.tools[0].1, "provided_a");
    assert_eq!(result.tools[1].1, "provided_b");
}

#[test]
fn empty_responses_message_has_a_real_lifecycle_and_no_fabricated_delta() {
    let input = concat!(
        "data: {\"type\":\"response.created\",\"response\":{\"id\":\"r\",\"model\":\"m\",\"created_at\":1}}\n\n",
        "data: {\"type\":\"response.output_item.added\",\"output_index\":0,\"item\":{\"id\":\"empty_item\",\"type\":\"message\",\"role\":\"assistant\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.output_item.done\",\"output_index\":0,\"item\":{\"id\":\"empty_item\",\"type\":\"message\",\"role\":\"assistant\",\"status\":\"completed\",\"content\":[]}}\n\n",
        "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"id\":\"empty_item\",\"type\":\"message\",\"content\":[]}]}}\n\n"
    );
    let result = observe(config(Protocol::OpenAiResponses), input.as_bytes(), 1, true);
    assert!(result.error.is_none(), "{result:?}");
    assert_eq!(result.text, "");
    assert_eq!(result.terminals, [TurnOutcome::Completed]);
    let inconsistent = input.replacen("data: {\"type\":\"response.output_item.done\"", concat!(
        "data: {\"type\":\"response.content_part.added\",\"output_index\":0,\"content_index\":0,\"item_id\":\"empty_item\",\"part\":{\"type\":\"output_text\",\"text\":\"x\",\"annotations\":[]}}\n\n",
        "data: {\"type\":\"response.output_item.done\""
    ), 1);
    let result = observe(
        config(Protocol::OpenAiResponses),
        inconsistent.as_bytes(),
        7,
        false,
    );
    assert_eq!(result.error.unwrap().kind, ErrorKind::Lifecycle);
    assert_eq!(result.text, "x");
}

#[test]
fn one_extension_permission_does_not_authorize_cache_write_loss() {
    let input = ANTHROPIC.replace(
        "\"input_tokens\":5,\"output_tokens\":0",
        "\"input_tokens\":5,\"output_tokens\":0,\"cache_creation_input_tokens\":2",
    );
    for (allowed, expect_success) in [
        (&[StreamLoss::SystemFingerprint][..], false),
        (&[StreamLoss::CacheCreationTokens][..], true),
    ] {
        let mut f = [0; 2048];
        let mut p = [0; 2048];
        let mut a = [0; 4096];
        let mut w = StreamWorkspace::<256, 64, 8>::new();
        let mut session = Session::new(
            config(Protocol::AnthropicMessages),
            &mut f,
            &mut p,
            &mut a,
            &mut w,
        )
        .unwrap();
        session.prepare().unwrap();
        let mut o = [0; 8192];
        let mut ea = [0; 4096];
        let mut s = [0; 2048];
        let mut ew = EncoderWorkspace::<256, 8>::new();
        let mut ec = EventEncoderConfig::new(
            Protocol::AnthropicMessages.into(),
            Protocol::OpenAiChat.into(),
        );
        ec.context.created = Some(9);
        ec.allowed_losses = allowed;
        let mut encoder = EventEncoder::new(ec, &mut o, &mut ea, &mut s, &mut ew).unwrap();
        let mut failure = None;
        let mut losses = Vec::new();
        let mut encoded = Vec::new();
        session.step(input.as_bytes(), true, |event| {
            match encoder.push(event) {
                StageStatus::Accepted(loss) => {
                    if let Some(field) = loss.extension {
                        losses.push(field);
                    }
                }
                StageStatus::Failed(error) => {
                    failure = Some(error);
                    return Control::Pause;
                }
                other => panic!("{other:?}"),
            }
            let mut out = [0; 64];
            loop {
                let step = encoder.drain(&mut out);
                encoded.extend_from_slice(&out[..step.written]);
                match step.status {
                    EncoderStatus::Ready | EncoderStatus::Done => break,
                    EncoderStatus::NeedOutput => {}
                    other => panic!("{other:?}"),
                }
            }
            Control::Continue
        });
        if expect_success {
            assert!(failure.is_none());
            assert!(losses.contains(&StreamLoss::CacheCreationTokens));
            let result = observe(config(Protocol::OpenAiChat), &encoded, 1, false);
            assert!(result.error.is_none());
            assert_eq!(result.usage.input, Some(7));
            assert_eq!(result.usage.total, Some(10));
        } else {
            let error = failure.unwrap();
            assert_eq!(error.kind, ErrorKind::ForbiddenLoss);
            assert_eq!(error.field, "cache_creation_input_tokens");
            assert!(encoded.is_empty());
            assert!(losses.is_empty());
        }
    }
}

#[test]
fn late_real_identity_is_observable_and_encoder_rechecks_caller_context() {
    let input = concat!(
        "data: {\"choices\":[]}\n\n",
        "data: {\"id\":\"real_r\",\"model\":\"real_m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hi\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    for caller_id in ["real_r", "different_id"] {
        let mut f = [0; 2048];
        let mut p = [0; 2048];
        let mut a = [0; 4096];
        let mut w = StreamWorkspace::<256, 64, 8>::new();
        let mut session =
            Session::new(config(Protocol::OpenAiChat), &mut f, &mut p, &mut a, &mut w).unwrap();
        session.prepare().unwrap();
        let mut o = [0; 8192];
        let mut ea = [0; 4096];
        let mut s = [0; 2048];
        let mut ew = EncoderWorkspace::<256, 8>::new();
        let mut ec =
            EventEncoderConfig::new(Protocol::OpenAiChat.into(), Protocol::OpenAiChat.into());
        ec.context.response_id = Some(caller_id);
        ec.context.model = Some("real_m");
        ec.context.created = Some(1);
        let mut encoder = EventEncoder::new(ec, &mut o, &mut ea, &mut s, &mut ew).unwrap();
        let mut starts = 0;
        let mut updates = 0;
        let mut failure = None;
        session.step(input.as_bytes(), true, |event| {
            match event.data() {
                EventData::Started { response_id, .. } => {
                    starts += 1;
                    assert!(response_id.is_none());
                }
                EventData::IdentityUpdated(identity) => {
                    updates += 1;
                    assert!(identity.response_id.unwrap().eq_str("real_r"));
                }
                _ => {}
            }
            match encoder.push(event) {
                StageStatus::Accepted(_) => {}
                StageStatus::Failed(error) => {
                    failure = Some(error);
                    return Control::Pause;
                }
                other => panic!("{other:?}"),
            }
            loop {
                let step = encoder.drain(&mut [0; 256]);
                match step.status {
                    EncoderStatus::Ready | EncoderStatus::Done => break,
                    EncoderStatus::NeedOutput => {}
                    other => panic!("{other:?}"),
                }
            }
            Control::Continue
        });
        assert_eq!(starts, 1);
        assert_eq!(updates, 1);
        assert!(session.identity().response_id.unwrap().eq_str("real_r"));
        if caller_id == "real_r" {
            assert!(failure.is_none());
        } else {
            assert_eq!(failure.unwrap().kind, ErrorKind::InvalidInput);
        }
    }
}

#[test]
fn reasoning_summary_is_typed_and_never_translated_into_a_thought() {
    let input = concat!(
        "data: {\"event_type\":\"interaction.created\",\"interaction\":{\"id\":\"r\",\"model\":\"m\"}}\n\n",
        "data: {\"event_type\":\"step.start\",\"index\":0,\"step\":{\"type\":\"thought\"}}\n\n",
        "data: {\"event_type\":\"step.delta\",\"index\":0,\"delta\":{\"type\":\"thought_summary\",\"content\":{\"type\":\"text\",\"text\":\"A summary\"}}}\n\n"
    );
    let mut f = [0; 2048];
    let mut p = [0; 2048];
    let mut a = [0; 4096];
    let mut w = StreamWorkspace::<256, 64, 8>::new();
    let mut session = Session::new(
        config(Protocol::GeminiInteractions),
        &mut f,
        &mut p,
        &mut a,
        &mut w,
    )
    .unwrap();
    session.prepare().unwrap();
    let mut o = [0; 8192];
    let mut ea = [0; 4096];
    let mut s = [0; 2048];
    let mut ew = EncoderWorkspace::<256, 8>::new();
    let mut ec = EventEncoderConfig::new(
        Protocol::GeminiInteractions.into(),
        Protocol::AnthropicMessages.into(),
    );
    ec.context.initial_usage.input = Some(0);
    ec.allow_reasoning_loss = true;
    let mut encoder = EventEncoder::new(ec, &mut o, &mut ea, &mut s, &mut ew).unwrap();
    let mut failure = None;
    let mut saw_summary = false;
    session.step(input.as_bytes(), false, |event| {
        saw_summary |= event.reasoning_kind() == Some(StreamReasoningKind::Summary);
        match encoder.push(event) {
            StageStatus::Accepted(_) => {}
            StageStatus::Failed(error) => {
                failure = Some(error);
                return Control::Pause;
            }
            other => panic!("{other:?}"),
        }
        loop {
            let step = encoder.drain(&mut [0; 256]);
            match step.status {
                EncoderStatus::Ready => break,
                EncoderStatus::NeedOutput => {}
                other => panic!("{other:?}"),
            }
        }
        Control::Continue
    });
    assert!(saw_summary);
    let error = failure.unwrap();
    assert_eq!(error.kind, ErrorKind::Unsupported);
    assert_eq!(error.field, "stream reasoning thought/summary mapping");
}

fn visit_metadata(source: Protocol, input: &[u8], mut consume: impl FnMut(EventData<'_>)) {
    let mut framing = [0; 8192];
    let mut payload = [0; 8192];
    let mut aggregate = [0; 16384];
    let mut workspace = StreamWorkspace::<512, 64, 32>::new();
    let mut session = Session::new(
        config(source),
        &mut framing,
        &mut payload,
        &mut aggregate,
        &mut workspace,
    )
    .unwrap();
    session.prepare().unwrap();
    let step = session.step(input, true, |event| {
        consume(event.data());
        Control::Continue
    });
    assert_eq!(step.status, StepStatus::Done);
}

#[test]
fn images_and_usage_metadata_survive_public_events_and_same_format_encoding() {
    let input = concat!(
        "data: {\"id\":\"r\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"images\":[{\"type\":\"image_url\",\"image_url\":{\"url\":\"https://example.com/a.png\"},\"extra\":\"kept\"}]}}]}\n\n",
        "data: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\n",
        "data: {\"choices\":[],\"usage\":{\"cache_creation_input_tokens\":3,\"prompt_tokens_details\":{\"cache_write_tokens\":3,\"cache_write_tokens_details\":{\"ttl_30m_tokens\":1,\"ttl_12h_tokens\":2}},\"cost\":1.25e-7}}\n\n",
        "data: [DONE]\n\n"
    );
    let mut images = false;
    let mut usage = false;
    visit_metadata(
        Protocol::OpenAiChat,
        input.as_bytes(),
        |event| match event {
            EventData::Images { choice, json } => {
                assert_eq!(choice, 0);
                assert!(
                    std::str::from_utf8(json)
                        .unwrap()
                        .contains("\"extra\":\"kept\"")
                );
                images = true;
            }
            EventData::Usage(value) => {
                assert_eq!(value.input, None);
                assert_eq!(value.cache_write_30m, Some(1));
                assert_eq!(value.cache_write_12h, Some(2));
                assert_eq!(value.cost.unwrap().raw(), b"1.25e-7");
                assert!(value.openai_cache_details);
                usage = true;
            }
            _ => {}
        },
    );
    assert!(images && usage);
    let vendor = Format {
        profile: Profile::OpenAiCompatible("openrouter"),
        ..Protocol::OpenAiChat.into()
    };
    let (output, report) = encode_formats(vendor, vendor, input.as_bytes(), false).unwrap();
    let output = std::str::from_utf8(&output).unwrap();
    for retained in [
        "\"extra\":\"kept\"",
        "\"cost\":1.25e-7",
        "\"ttl_12h_tokens\":2",
    ] {
        assert!(output.contains(retained), "{output}");
    }
    assert_eq!(report.dropped_extensions, 0);
    assert_eq!(
        encode(
            Protocol::OpenAiChat,
            Protocol::OpenAiResponses,
            input.as_bytes(),
            false
        )
        .unwrap_err()
        .kind,
        ErrorKind::Unsupported
    );
}

#[test]
fn grounding_and_continuation_remain_distinct_from_text_and_stop() {
    let input = b"data: {\"responseId\":\"r\",\"modelVersion\":\"m\",\"usageMetadata\":{\"toolUsePromptTokenCount\":4},\"candidates\":[{\"index\":0,\"groundingMetadata\":{\"webSearchQueries\":[\"q\"]},\"content\":{\"role\":\"model\",\"parts\":[{\"text\":\"ok\"}]},\"finishReason\":\"CONTINUATION\",\"continuationToken\":\"next\"}]}\n\n";
    let mut seen = [false; 4];
    visit_metadata(
        Protocol::GeminiGenerateContent,
        input,
        |event| match event {
            EventData::Grounding { choice, json } => {
                assert_eq!(choice, 0);
                assert_eq!(json, br#"{"webSearchQueries":["q"]}"#);
                seen[0] = true;
            }
            EventData::Continuation { choice, token } => {
                assert_eq!(choice, 0);
                assert!(token.eq_str("next"));
                seen[1] = true;
            }
            EventData::ChoiceEnded { reason, .. } => {
                assert_eq!(reason, StopReason::Continuation);
                seen[2] = true;
            }
            EventData::Usage(value) => {
                assert_eq!(value.tool_use_prompt, Some(4));
                seen[3] = true;
            }
            _ => {}
        },
    );
    assert!(seen.into_iter().all(|value| value));
    let (output, _) = encode(
        Protocol::GeminiGenerateContent,
        Protocol::GeminiGenerateContent,
        input,
        false,
    )
    .unwrap();
    let output = std::str::from_utf8(&output).unwrap();
    for retained in [
        "webSearchQueries",
        "\"continuationToken\":\"next\"",
        "\"finishReason\":\"CONTINUATION\"",
        "\"toolUsePromptTokenCount\":4",
    ] {
        assert!(output.contains(retained), "{output}");
    }
}

#[test]
fn signed_atomic_tools_keep_provider_binding_through_ready_and_encoding() {
    let input = b"data: {\"responseId\":\"r\",\"modelVersion\":\"m\",\"candidates\":[{\"index\":0,\"content\":{\"role\":\"model\",\"parts\":[{\"functionCall\":{\"id\":\"c\",\"name\":\"f\",\"args\":{}},\"thoughtSignature\":\"signed\"}]},\"finishReason\":\"STOP\"}]}\n\n";
    let mut signed = false;
    visit_metadata(Protocol::GeminiGenerateContent, input, |event| {
        if let EventData::ToolReady(tool) = event {
            let signature = tool.signature.unwrap();
            assert_eq!(signature.source, Protocol::GeminiGenerateContent.into());
            assert!(signature.value.eq_str("signed"));
            signed = true;
        }
    });
    assert!(signed);
    let (output, _) = encode(
        Protocol::GeminiGenerateContent,
        Protocol::GeminiGenerateContent,
        input,
        false,
    )
    .unwrap();
    assert!(
        std::str::from_utf8(&output)
            .unwrap()
            .contains("\"thoughtSignature\":\"signed\"")
    );
    assert!(
        encode(
            Protocol::GeminiGenerateContent,
            Protocol::OpenAiChat,
            input,
            false
        )
        .is_err()
    );
}

#[test]
fn stop_sequence_is_observed_without_becoming_a_text_delta() {
    let input = ANTHROPIC.replace(
        "\"stop_reason\":\"end_turn\"",
        "\"stop_reason\":\"stop_sequence\",\"stop_sequence\":\"END\"",
    );
    let mut seen = false;
    visit_metadata(Protocol::AnthropicMessages, input.as_bytes(), |event| {
        if let EventData::StopSequence { sequence } = event {
            assert!(sequence.eq_str("END"));
            seen = true;
        }
    });
    assert!(seen);
}

#[test]
fn original_uncached_and_canonical_input_coexist_without_double_counting() {
    let input = ANTHROPIC.replace(
        "\"input_tokens\":5,\"output_tokens\":0",
        "\"input_tokens\":5,\"cache_read_input_tokens\":2,\"cache_creation_input_tokens\":3,\"output_tokens\":0",
    );
    let mut seen = false;
    visit_metadata(Protocol::AnthropicMessages, input.as_bytes(), |event| {
        if let EventData::Usage(usage) = event {
            assert_eq!(usage.uncached_input, Some(5));
            assert_eq!(usage.input, Some(10));
            seen = true;
        }
    });
    assert!(seen);
    let (output, _) = encode(
        Protocol::AnthropicMessages,
        Protocol::AnthropicMessages,
        input.as_bytes(),
        false,
    )
    .unwrap();
    let output = std::str::from_utf8(&output).unwrap();
    assert!(output.contains("\"input_tokens\":5"), "{output}");
    assert!(!output.contains("\"input_tokens\":10"), "{output}");
    assert!(
        output.contains("\"cache_creation_input_tokens\":3"),
        "{output}"
    );
}
