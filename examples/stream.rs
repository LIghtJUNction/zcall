//! The same event consumer runs with two independent source adapters.
use zcall::v1::*;

fn consume(event: Event<'_>) -> Control {
    match event.data() {
        EventData::Delta {
            kind: BlockType::Text,
            text,
            ..
        } => {
            let mut decoded = [0; 128];
            let len = text.decode_into(&mut decoded).expect("example-sized text");
            print!("{}", core::str::from_utf8(&decoded[..len]).unwrap());
        }
        EventData::ToolReady(tool) => {
            // Dispatch to the harness's executor after accepting this event.
            println!(
                "tool {}: {}",
                tool.call_id,
                tool.arguments.as_str().unwrap()
            );
        }
        EventData::Terminal(terminal) => println!(" [{:?}]", terminal.outcome),
        _ => {}
    }
    Control::Continue
}

fn drive(source: Protocol, input: &[u8]) -> Result<(), Error> {
    let mut framing = [0; 512];
    let mut payload = [0; 512];
    let mut aggregate = [0; 256];
    let mut workspace = StreamWorkspace::<96, 12, 4>::new();
    let mut session = Session::new(
        SessionConfig::new(source.into()),
        &mut framing,
        &mut payload,
        &mut aggregate,
        &mut workspace,
    )?;
    session.prepare()?;
    let mut offset = 0;
    while offset < input.len() {
        let end = (offset + 11).min(input.len());
        let step = session.step(&input[offset..end], end == input.len(), consume);
        offset += step.consumed;
        match step.status {
            StepStatus::NeedInput | StepStatus::Done => {}
            StepStatus::NeedEvents => continue,
            StepStatus::NeedBuffer(c) => {
                return Err(Error::capacity(c.region, c.needed, c.exact, c.replay));
            }
            StepStatus::Failed(error) => return Err(error),
        }
    }
    Ok(())
}

fn main() -> Result<(), Error> {
    let chat = concat!(
        "data: {\"id\":\"r\",\"model\":\"m\",\"created\":1,\"choices\":[{\"index\":0,\"delta\":{\"content\":\"Hello\"},\"finish_reason\":\"stop\"}]}\n\n",
        "data: [DONE]\n\n"
    );
    let anthropic = concat!(
        "data: {\"type\":\"message_start\",\"message\":{\"id\":\"r\",\"model\":\"m\",\"role\":\"assistant\",\"content\":[],\"usage\":{\"input_tokens\":1,\"output_tokens\":0}}}\n\n",
        "data: {\"type\":\"content_block_start\",\"index\":0,\"content_block\":{\"type\":\"text\",\"text\":\"Hello\"}}\n\n",
        "data: {\"type\":\"content_block_stop\",\"index\":0}\n\n",
        "data: {\"type\":\"message_delta\",\"delta\":{\"stop_reason\":\"end_turn\"},\"usage\":{\"output_tokens\":1}}\n\n",
        "data: {\"type\":\"message_stop\"}\n\n"
    );
    drive(Protocol::OpenAiChat, chat.as_bytes())?;
    drive(Protocol::AnthropicMessages, anthropic.as_bytes())
}
