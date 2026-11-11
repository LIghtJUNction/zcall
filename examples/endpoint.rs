//! Build endpoints without making network requests.
use zcall::v1::*;

fn main() -> Result<(), EndpointError> {
    let mut url = [0; 512];
    for protocol in [
        Protocol::OpenAiCompletions,
        Protocol::OpenAiChat,
        Protocol::OpenAiResponses,
        Protocol::AnthropicMessages,
        Protocol::GeminiGenerateContent,
        Protocol::GeminiInteractions,
    ] {
        let endpoint = HttpEndpoint::new("https://gateway.example", protocol.into())?;
        println!(
            "{protocol:?}: {}",
            endpoint.url_into(Some("caller-selected-model"), false, &mut url)?
        );
    }

    let endpoint = HttpEndpoint::new(
        "https://gateway.example/proxy/v1beta?tenant=example",
        Protocol::GeminiGenerateContent.into(),
    )?;
    assert_eq!(
        endpoint.url_into(Some("models/caller-selected-model"), true, &mut url)?,
        "https://gateway.example/proxy/v1beta/models/caller-selected-model:streamGenerateContent?tenant=example&alt=sse"
    );
    Ok(())
}
