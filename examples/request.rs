//! The business message and response consumer stay unchanged for every backend.
use zcall::v1::*;

fn main() -> Result<(), Error> {
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<128>::new();
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("caller-selected-model")),
        &mut records,
    )?;
    builder.message(Role::User, &[Content::text("Explain a hash table.")])?;
    let mut request = builder.finish();
    // Backend selection belongs to the integration boundary. It does not change
    // the messages, business logic, token policy or response/event consumer.
    for backend in [
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ] {
        let mut output = [0; 2048];
        let report = request.encode(
            backend.into(),
            &mut output,
            &mut [],
            &mut tokens,
            EncodeContext::default().with_max_tokens(256),
            Policy::default(),
        )?;
        println!(
            "{backend:?}: {}",
            std::str::from_utf8(&output[..report.written]).unwrap()
        );
        if let Presence::Value(model) = report.transport_model {
            println!("model for caller's endpoint: {}", model.as_str().unwrap());
        }
    }
    Ok(())
}
