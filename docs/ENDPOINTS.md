# Endpoints and protocol discovery

`HttpEndpoint` binds a base URL to a wire format. `Discovery` finds a usable format
by producing small generation requests and checking the HTTP responses supplied
by the application. Both APIs borrow their configuration and use caller-owned
buffers. HTTP, credentials, TLS, deadlines and retries belong to the transport.

## Configure a known protocol

```rust
use zcall::v1::*;

let endpoint = HttpEndpoint::new(
    "https://gateway.example/proxy/v1",
    Protocol::OpenAiChat.into(),
)?;
let mut output = [0; 512];
let url = endpoint.url_into(Some("my-model"), false, &mut output)?;
assert_eq!(url, "https://gateway.example/proxy/v1/chat/completions");
# Ok::<(), EndpointError>(())
```

Use `endpoint.format()` as the target format for `Request::prepare` or
`Request::encode`. The resulting request bytes go to the URL returned by
`url_into`. For GenerateContent, the request encoder reports the model and
stream setting through `EncodeReport::transport_model` and
`EncodeReport::transport_stream`; supply the same values to URL construction.

| Protocol | URL from `https://gateway.example` | Model in URL |
| --- | --- | --- |
| OpenAI Completions | `/v1/completions` | No |
| OpenAI Chat | `/v1/chat/completions` | No |
| OpenAI Responses | `/v1/responses` | No |
| Anthropic Messages | `/v1/messages` | No |
| Gemini GenerateContent | `/v1beta/models/{model}:generateContent` | Yes |
| Gemini Interactions | `/v1beta/interactions` | No |

An origin-only URL receives the default prefix above. An explicit path prefix
is retained verbatim: `https://gateway.example/api` with Chat becomes
`https://gateway.example/api/chat/completions`. Include `/api/v1` when your
gateway requires that version segment. Trailing slashes are normalized. A
complete recognized endpoint is retained without appending it twice; selecting
a conflicting protocol returns `EndpointError::ProtocolMismatch`.

The HTTP route and adapter schema version are separate settings. For example,
Gemini Interactions uses the current `Format` schema identifier `v1`, while an
origin-only URL defaults to the `/v1beta/interactions` HTTP route. Pass an
explicit `/v1` base URL for a deployment exposing that route. Anthropic's
`Format.version` supplies the `anthropic-version` probe header.

GenerateContent URL construction takes a model name, accepts a leading
`models/`, and encodes the model as a path segment. When the base URL already
contains a complete GenerateContent endpoint, `None` keeps its model and
`Some(model)` replaces it. `stream=true` selects `:streamGenerateContent` and
sets `alt=sse`; `stream=false` selects `:generateContent` and removes the SSE
query selector. Other query parameters, including routing parameters such as
`api-version`, are preserved.

Base URLs use ASCII URI syntax; supply punycode hosts and percent-encoded
non-ASCII path/query text. URL validation rejects unsupported schemes, missing
hosts, URL userinfo, fragments, control characters, malformed escapes and dot
path segments. The application decides which hosts it may contact. `url_into`
reports the exact required output capacity before changing an undersized
buffer, so the caller can enlarge the buffer and retry.

## Discover an unknown protocol

Create one discovery session for the base URL and a caller-selected model.
`next_probe` produces a `POST` URL, JSON body, content type, accept header and
optional protocol-version header. Add the deployment's authentication, send
the request, and supply the response to `observe`. A successful selection is
an `HttpEndpoint` that can be cached and reused for subsequent calls.

```rust
use zcall::v1::*;

let mut discovery = Discovery::new("https://gateway.example/v1", "my-model")?;
let mut url = [0; 512];
let mut body = [0; 512];
let probe = discovery.next_probe(&mut url, &mut body)?.unwrap();
assert_eq!(probe.format.protocol, Protocol::OpenAiChat);
assert_eq!(probe.method, "POST");
assert_eq!(probe.url, "https://gateway.example/v1/chat/completions");
// Your transport sends probe.body with probe's headers and authentication.

let response = br#"{"id":"r","object":"chat.completion","model":"my-model","choices":[{"index":0,"message":{"role":"assistant","content":"OK"},"finish_reason":"stop"}]}"#;
let mut tokens = TokenStorage::<128>::new();
assert_eq!(
    discovery.observe(200, Some("application/json"), response,
                      &mut tokens, Limits::default())?,
    ProbeOutcome::Supported,
);
assert_eq!(discovery.selected().unwrap().format().protocol, Protocol::OpenAiChat);
# Ok::<(), DiscoveryError>(())
```

The default candidate order is Chat, Responses, Anthropic Messages, Gemini
GenerateContent, Gemini Interactions and Completions. Discovery stops at the
first verified candidate. A service may support several protocols; customize
the order when the harness prefers one. A complete endpoint URL narrows the
candidates to that endpoint's protocol, while its response must still pass
verification. Provider hostnames and model names are never used as proof of
protocol support.

```rust
use zcall::v1::*;

let formats = [Protocol::OpenAiResponses.into(), Protocol::OpenAiChat.into()];
let mut config = DiscoveryConfig::new("my-model");
config.formats = &formats;
config.max_output_tokens = 32;
config.chat_token_limit = ChatTokenLimit::MaxCompletionTokens;
let discovery = Discovery::with_config("https://gateway.example/v1", config)?;
assert!(discovery.selected().is_none());
# Ok::<(), DiscoveryError>(())
```

Each candidate gets a separate, nonstreaming `Reply OK.` generation probe with
a configurable output-token limit, defaulting to 16. The probe contains no
tools or conversation history. Responses and Interactions include
`store:false`. Providers can bill these requests. Supply a model and credential
that are valid for the deployment; an unknown model or rejected generation
setting remains an unsuccessful attempt. Chat uses `max_tokens` by default;
select `ChatTokenLimit::MaxCompletionTokens` for models requiring that field.

## Interpret outcomes and retry ownership

Protocol selection requires a successful HTTP response, a recognizable JSON
response shape and validation by the corresponding zconv adapter. An HTML
success page, a generic models list, ambiguous response data or an error
envelope does not produce a selection. A selected endpoint establishes that
the probe's request/response path worked; tool, modality, context and streaming
capabilities still use the existing capability checks and session APIs.

| Observed result | Discovery outcome |
| --- | --- |
| Recognized and valid successful response | `Supported` |
| HTTP 401 | `AuthenticationRequired` |
| HTTP 403 | `Forbidden` |
| HTTP 429 | `RateLimited` |
| HTTP 5xx | `ServerError(status)` |
| HTTP redirect | `Redirect(status)` |
| HTTP 404 / 405 / 410, including model-routing failures | `NotFound` |
| Other request rejection | `Rejected(status)` |
| Unrecognized successful response | `UnrecognizedResponse` |
| Transport reports connection/timeout failure | `TransportFailed` |

`attempts()` retains each completed candidate's format and outcome. Failed
candidates can be followed by `next_probe`; success or exhaustion makes it
return `None`. `selected()` is `None` after exhaustion. Never relabel an
authentication error, quota error or server failure as evidence that a protocol
is unsupported.

Call `observe` or `transport_failed` for the outstanding probe before requesting
the next one. Buffer and resource errors preserve the pending operation for
retry; no candidate is silently skipped. Read `DiscoveryError` before deciding
whether to enlarge a buffer, adjust a limit or stop. The caller must bound HTTP
response size before buffering it, and may stop probing on authentication,
rate-limit or server errors according to its own policy. The library performs
no hidden network retry, redirect or timeout.

Cache a selected `HttpEndpoint` under the base URL, deployment and credential scope
used for discovery. Reuse it with `Request::prepare` for normal calls. Run a new
discovery when that routing or credential context changes, or when the
application's expiry policy requires it. There is no process-wide cache or
cross-tenant mutable state in the library.

## Run the external transport example

The [`discover` example](../examples/discover.rs) uses `std` and an installed
`curl` executable. It adds no Cargo dependencies and is separate from the
`no_std` library. Set `API_KEY` through your environment, then run:

```sh
cargo run --example discover -- https://gateway.example/v1 my-model auto
cargo run --example discover -- https://gateway.example/v1 my-model responses
```

The second command constructs an explicit endpoint without network access.
Auto mode supplies `Authorization: Bearer` for OpenAI-style protocols,
`x-api-key` for Anthropic and `x-goog-api-key` for Gemini. Gateways with different
authentication rules should adapt the `send` function to their existing
transport. Set `CHAT_TOKEN_LIMIT=max_completion_tokens` when required by the
model. API keys are passed to curl through stdin; the example omits URL query
values from its diagnostic output.

The example disables curl's user configuration and automatic redirects, uses a
5-second connection timeout and a 20-second overall timeout, and caps response
bodies at 64 KiB. TLS verification and the environment's proxy settings remain
enabled. Tests use local HTTP fixtures, so running the test suite does not
contact a model provider or spend credits.

## Protocol references

- [OpenAI Chat Completions](https://platform.openai.com/docs/api-reference/chat/create)
- [OpenAI Responses](https://platform.openai.com/docs/api-reference/responses/create)
- [Anthropic Messages](https://platform.claude.com/docs/en/api/messages/create)
- [Gemini GenerateContent](https://ai.google.dev/api/generate-content)
- [Gemini Interactions](https://ai.google.dev/api/interactions-api)

The URL and response contracts above are checked against these references and
the zconv 0.2.2 adapters. The repository's endpoint, discovery and local-HTTP
tests cover the implemented behavior; live availability depends on the selected
deployment, model and credential.
