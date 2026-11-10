//! Fair IR baselines share input, token/record capacities and target semantics.
//! Stateful stream timings include lifecycle/usage validation and are absolute.
use std::hint::black_box;
use std::time::Instant;
use zcall::v1::*;

const CHAT: &[u8] = br#"{"model":"m","messages":[{"role":"user","content":"Explain a hash table."}],"temperature":0.7,"max_tokens":256}"#;
const RESPONSE: &[u8] = br#"{"id":"r","model":"m","created":1,"choices":[{"index":0,"message":{"role":"assistant","content":"A hash table maps keys to buckets."},"finish_reason":"stop"}],"usage":{"prompt_tokens":10,"completion_tokens":12,"total_tokens":22}}"#;
const TOOL_REQUEST: &[u8] = br#"{"model":"m","messages":[{"role":"assistant","content":null,"tool_calls":[{"id":"call-1","type":"function","function":{"name":"lookup","arguments":"{\"city\":\"Taipei\",\"unit\":\"c\"}"}}]},{"role":"tool","tool_call_id":"call-1","content":"{\"temperature\":24}"}],"tools":[{"type":"function","function":{"name":"lookup","description":"Weather","parameters":{"type":"object","properties":{"city":{"type":"string"},"unit":{"type":"string"}},"required":["city","unit"],"additionalProperties":false},"strict":true}}]}"#;
const STREAM: &[u8] = b"data: {\"id\":\"r\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"hello \"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"r\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"world\"},\"finish_reason\":null}]}\n\ndata: {\"id\":\"r\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";

fn measure(name: &str, bytes: usize, iters: usize, mut work: impl FnMut() -> usize) {
    for _ in 0..iters.min(200) {
        black_box(work());
    }
    let mut samples = [0f64; 7];
    for sample in &mut samples {
        let start = Instant::now();
        let mut check = 0;
        for _ in 0..iters {
            check ^= black_box(work());
        }
        black_box(check);
        *sample = start.elapsed().as_nanos() as f64 / iters as f64;
    }
    samples.sort_by(f64::total_cmp);
    let median = samples[3];
    let mib_s = bytes as f64 / median * 1e9 / (1024.0 * 1024.0);
    println!(
        "{name},{bytes},{iters},{median:.2},{:.2},{:.2},{mib_s:.2}",
        samples[0], samples[6]
    );
}

fn requests(input: &[u8], name: &str, iters: usize) {
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<32>::new();
    let mut output = [0; 8192];
    let mut raw_tokens = [zconv::json::Token::EMPTY; 128];
    let mut raw_records = [zconv::ir::Record::EMPTY; 32];
    let mut raw_output = [0; 8192];
    let mut scratch = [0; 64];
    let mut nested = [zconv::json::Token::EMPTY; 128];
    // Equality is checked before timing, so an omitted semantic operation fails.
    let mut request = Request::decode(
        Protocol::OpenAiChat.into(),
        input,
        &mut tokens,
        &mut records,
        Limits::default(),
    )
    .unwrap();
    let wrapped = request
        .encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )
        .unwrap()
        .written;
    let doc = zconv::json::parse(input, &mut raw_tokens, zconv::json::Limits::default()).unwrap();
    let request = zconv::protocol::decode_request(
        zconv::Protocol::OpenAiChat.into(),
        doc.root(),
        &mut raw_records,
    )
    .unwrap();
    let direct = zconv::protocol::encode_request(
        &request,
        zconv::Protocol::OpenAiResponses.into(),
        &zconv::ir::Context::default(),
        zconv::ir::Policy::default(),
        &mut raw_output,
        &mut scratch,
        &mut nested,
    )
    .unwrap()
    .written;
    assert_eq!(&output[..wrapped], &raw_output[..direct]);
    measure(&format!("zcall_ir_{name}"), input.len(), iters, || {
        let mut request = Request::decode(
            Protocol::OpenAiChat.into(),
            black_box(input),
            &mut tokens,
            &mut records,
            Limits::default(),
        )
        .unwrap();
        let count = request.messages().count();
        let written = request
            .encode(
                Protocol::OpenAiResponses.into(),
                &mut output,
                &mut [],
                &mut tokens,
                EncodeContext::default(),
                Policy::default(),
            )
            .unwrap()
            .written;
        black_box(&output[..written]);
        count + written
    });
    measure(&format!("zconv_ir_{name}"), input.len(), iters, || {
        let doc = zconv::json::parse(
            black_box(input),
            &mut raw_tokens,
            zconv::json::Limits::default(),
        )
        .unwrap();
        let request = zconv::protocol::decode_request(
            zconv::Protocol::OpenAiChat.into(),
            doc.root(),
            &mut raw_records,
        )
        .unwrap();
        let count = request
            .records
            .iter()
            .filter(|r| matches!(r, zconv::ir::Record::Message(_)))
            .count();
        let written = zconv::protocol::encode_request(
            &request,
            zconv::Protocol::OpenAiResponses.into(),
            &zconv::ir::Context::default(),
            zconv::ir::Policy::default(),
            &mut raw_output,
            &mut scratch,
            &mut nested,
        )
        .unwrap()
        .written;
        black_box(&raw_output[..written]);
        count + written
    });
}

fn main() {
    let quick = cfg!(debug_assertions) || std::env::args().any(|a| a == "--quick");
    let iters = if quick { 100 } else { 20_000 };
    println!("workload,input_bytes,iterations,median_ns,min_ns,max_ns,mib_per_s");
    requests(CHAT, "small_request", iters);
    let unicode = format!(
        "{{\"model\":\"m\",\"messages\":[{{\"role\":\"user\",\"content\":\"{}\"}}]}}",
        "中文🙂αβγ\n".replace('\n', "\\n").repeat(96)
    );
    requests(unicode.as_bytes(), "unicode_request", iters);
    let mut tool_tokens = TokenStorage::<256>::new();
    let mut tool_records = RecordStorage::<32>::new();
    let mut tool_output = [0; 4096];
    let mut tool_scratch = [0; 512];
    // Absolute cost includes zcall's completed-argument object validation.
    // A raw conversion without this lifecycle check is not an equivalent task.
    measure("zcall_tool_request", TOOL_REQUEST.len(), iters, || {
        let mut request = Request::decode(
            Protocol::OpenAiChat.into(),
            black_box(TOOL_REQUEST),
            &mut tool_tokens,
            &mut tool_records,
            Limits::default(),
        )
        .unwrap();
        let written = request
            .encode(
                Protocol::OpenAiResponses.into(),
                &mut tool_output,
                &mut tool_scratch,
                &mut tool_tokens,
                EncodeContext::default(),
                Policy::default(),
            )
            .unwrap()
            .written;
        black_box(&tool_output[..written]);
        written
    });
    let mut tokens = TokenStorage::<128>::new();
    let mut records = RecordStorage::<16>::new();
    measure("zcall_response_view", RESPONSE.len(), iters, || {
        let response = Response::decode(
            Protocol::OpenAiChat.into(),
            black_box(RESPONSE),
            &mut tokens,
            &mut records,
            Limits::default(),
        )
        .unwrap();
        response
            .choices()
            .flat_map(|c| c.contents())
            .filter_map(|b| {
                if let Content::Text(t) = b.content() {
                    Some(t.decoded_len())
                } else {
                    None
                }
            })
            .sum()
    });
    for fragment in [STREAM.len(), 16, 1] {
        let mut workspace = StreamWorkspace::<64, 8, 4>::new();
        let mut framing = [0; 256];
        let mut payload = [0; 256];
        let mut aggregate = [0; 256];
        measure(
            &format!("zcall_session_fragment_{fragment}"),
            STREAM.len(),
            iters / 4 + 1,
            || {
                let mut session = Session::new(
                    SessionConfig::new(Protocol::OpenAiChat.into()),
                    &mut framing,
                    &mut payload,
                    &mut aggregate,
                    &mut workspace,
                )
                .unwrap();
                session.prepare().unwrap();
                let mut total = 0;
                let mut at = 0;
                while at < STREAM.len() {
                    let end = (at + fragment).min(STREAM.len());
                    let step =
                        session.step(black_box(&STREAM[at..end]), end == STREAM.len(), |event| {
                            if let EventData::Delta { text, .. } = event.data() {
                                total += text.decoded_len();
                            }
                            Control::Continue
                        });
                    at += step.consumed;
                    assert!(matches!(
                        step.status,
                        StepStatus::NeedInput | StepStatus::Done
                    ));
                    assert!(step.consumed > 0);
                }
                assert_eq!(session.state(), CallState::Completed);
                assert_eq!(total, 11);
                total
            },
        );
    }
    println!(
        "# token_128_bytes={}",
        TokenStorage::<128>::new().memory_bytes()
    );
    println!(
        "# record_32_bytes={}",
        RecordStorage::<32>::new().memory_bytes()
    );
    println!(
        "# stream_workspace_64_8_4_bytes={}",
        StreamWorkspace::<64, 8, 4>::new().memory_bytes()
    );
    println!(
        "# session_state_bytes={}",
        core::mem::size_of::<Session<'_, '_>>()
    );
    println!(
        "# encoder_workspace_64_4_bytes={}",
        EncoderWorkspace::<64, 4>::new().memory_bytes()
    );
    println!(
        "# encoder_state_bytes={}",
        core::mem::size_of::<EventEncoder<'_, '_>>()
    );
    println!(
        "# request_header_bytes={}",
        core::mem::size_of::<Request<'_, '_>>()
    );
    println!(
        "# response_header_bytes={}",
        core::mem::size_of::<Response<'_, '_>>()
    );
}
