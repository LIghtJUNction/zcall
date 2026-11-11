//! The only unsafe code is the std test allocator hook, outside the library.
use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use zcall::v1::*;

thread_local! {
    static TRACK: Cell<bool> = const { Cell::new(false) };
    static ALLOCATIONS: Cell<usize> = const { Cell::new(0) };
}
struct Counter;
unsafe impl GlobalAlloc for Counter {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        if TRACK.try_with(Cell::get).unwrap_or(false) {
            ALLOCATIONS.with(|n| n.set(n.get() + 1));
        }
        // SAFETY: Forward the same valid layout to the system allocator.
        unsafe { System.alloc(layout) }
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: Forward exactly the pointer/layout returned by System.
        unsafe { System.dealloc(ptr, layout) }
    }
}
#[global_allocator]
static ALLOCATOR: Counter = Counter;

#[test]
fn request_edit_encode_and_stream_do_not_allocate() {
    let mut records = RecordStorage::<8>::new();
    let mut tokens = TokenStorage::<64>::new();
    let mut output = [0; 512];
    let mut framing = [0; 256];
    let mut payload = [0; 256];
    let mut aggregate = [0; 128];
    let mut workspace = StreamWorkspace::<64, 8, 4>::new();
    // Initialize thread-local accounting before observing the library call.
    ALLOCATIONS.with(|n| n.set(0));
    TRACK.with(|n| n.set(true));
    let result = (|| -> Result<usize, Error> {
        let mut request = Request::decode(
            Protocol::OpenAiChat.into(),
            br#"{"model":"m","messages":[{"role":"user","content":"hello"}]}"#,
            &mut tokens,
            &mut records,
            Limits::default(),
        )?;
        request.replace_content(0, 0, Content::text("edited"))?;
        let report = request.encode(
            Protocol::OpenAiResponses.into(),
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default(),
            Policy::default(),
        )?;
        let mut session = Session::new(
            SessionConfig::new(Protocol::OpenAiChat.into()),
            &mut framing,
            &mut payload,
            &mut aggregate,
            &mut workspace,
        )?;
        session.prepare()?;
        let stream=b"data: {\"id\":\"r\",\"model\":\"m\",\"choices\":[{\"index\":0,\"delta\":{\"content\":\"ok\"},\"finish_reason\":null}]}\n\ndata: {\"choices\":[{\"index\":0,\"delta\":{},\"finish_reason\":\"stop\"}]}\n\ndata: [DONE]\n\n";
        let step = session.step(stream, true, |_| Control::Continue);
        if let StepStatus::Failed(e) = step.status {
            return Err(e);
        }
        if step.status != StepStatus::Done {
            return Err(Error::invalid("allocation test", "stream did not finish"));
        }
        Ok(report.written)
    })();
    TRACK.with(|n| n.set(false));
    assert!(result.unwrap() > 0);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}

#[test]
fn endpoint_and_discovery_do_not_allocate() {
    let mut url = [0; 512];
    let mut body = [0; 512];
    let mut tokens = TokenStorage::<128>::new();
    let response = br#"{"id":"r","object":"chat.completion","choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]}"#;
    ALLOCATIONS.with(|n| n.set(0));
    TRACK.with(|n| n.set(true));
    let result = (|| -> Result<usize, DiscoveryError> {
        let mut total = 0;
        for format in DEFAULT_DISCOVERY_FORMATS {
            let endpoint = HttpEndpoint::new("https://example.test/proxy?api-version=1", format)?;
            total += endpoint.url_into(Some("model-中文"), true, &mut url)?.len();
        }
        let mut discovery = Discovery::new("https://example.test", "model")?;
        for _ in DEFAULT_DISCOVERY_FORMATS {
            let probe = discovery.next_probe(&mut url, &mut body)?.unwrap();
            total += probe.url.len() + probe.body.len();
            discovery.observe(404, None, &[], &mut tokens, Limits::default())?;
        }
        assert!(discovery.is_finished());
        let mut discovery = Discovery::new("https://example.test", "model")?;
        discovery.next_probe(&mut url, &mut body)?;
        let outcome = discovery.observe(
            200,
            Some("application/json"),
            response,
            &mut tokens,
            Limits::default(),
        )?;
        assert_eq!(outcome, ProbeOutcome::Supported);
        assert!(discovery.selected().is_some());
        Ok(total)
    })();
    TRACK.with(|n| n.set(false));
    assert!(result.unwrap() > 0);
    assert_eq!(ALLOCATIONS.with(Cell::get), 0);
}
