#![no_main]
use libfuzzer_sys::fuzz_target;
use zcall::v1::*;

fuzz_target!(|input: &[u8]| {
    if input.len() > 4096 {
        return;
    }
    let protocols = [
        Protocol::OpenAiCompletions,
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ];
    for protocol in protocols {
        let mut records = RecordStorage::<32>::new();
        let mut tokens = TokenStorage::<128>::new();
        let mut output = [0; 2048];
        let mut scratch = [0; 512];
        if let Ok(mut request) = Request::decode(
            protocol.into(),
            input,
            &mut tokens,
            &mut records,
            Limits::default(),
        ) {
            let _ = request.messages().flat_map(|m| m.contents()).count();
            for target in protocols {
                let _ = request.encode(
                    target.into(),
                    &mut output,
                    &mut scratch,
                    &mut tokens,
                    EncodeContext::default(),
                    Policy::default(),
                );
            }
        }
        let mut records = RecordStorage::<32>::new();
        if let Ok(mut response) = Response::decode(
            protocol.into(),
            input,
            &mut tokens,
            &mut records,
            Limits::default(),
        ) {
            let _ = response.choices().flat_map(|c| c.contents()).count();
            for target in protocols {
                let _ = response.encode(
                    target.into(),
                    &mut output,
                    &mut scratch,
                    &mut tokens,
                    EncodeContext::default(),
                    Policy::default(),
                );
            }
        }
        let mut workspace = StreamWorkspace::<128, 16, 8>::new();
        let mut framing = [0; 1024];
        let mut payload = [0; 1024];
        let mut aggregate = [0; 2048];
        if let Ok(mut session) = Session::new(
            SessionConfig::new(protocol.into()),
            &mut framing,
            &mut payload,
            &mut aggregate,
            &mut workspace,
        ) {
            session.prepare().unwrap();
            let mut terminals = 0;
            let fragment = input.first().map_or(1, |v| usize::from(*v) + 1);
            let mut at = 0;
            while at < input.len() {
                let end = (at + fragment).min(input.len());
                let step = session.step(&input[at..end], end == input.len(), |event| {
                    if matches!(event.data(), EventData::Terminal(_)) {
                        terminals += 1;
                    }
                    Control::Continue
                });
                assert!(step.consumed <= end - at);
                assert!(terminals <= 1);
                at += step.consumed;
                if !matches!(step.status, StepStatus::NeedInput) || step.consumed == 0 {
                    break;
                }
            }
            let _ = session.step(&[], true, |event| {
                if matches!(event.data(), EventData::Terminal(_)) {
                    terminals += 1;
                }
                Control::Continue
            });
            assert!(terminals <= 1);
        }
    }
});
