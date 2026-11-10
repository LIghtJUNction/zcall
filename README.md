# zcall

A protocol-independent, caller-buffered AI call kernel for harness software.
Rust 1.74 · `no_std` · `core` + `zconv` · no allocation · safe Rust · Sans-I/O.

`zcall::v1` gives a harness ordered requests, borrowed complete responses, unified
stream events, explicit capabilities and a model-turn state machine. `zconv`
owns protocol parsing and encoding. The harness owns transport, authentication,
retries, scheduling, conversation history, tool execution and agent decisions.

## Use

```toml
[dependencies]
zcall = { git = "https://github.com/LIghtJUNction/zcall", branch = "main", default-features = false }
```

The current development dependency is the exact zconv commit recorded in
`Cargo.toml`. Registry publication checks that the required zconv version is
available on crates.io, replaces the development Git pin with the exact registry
version, and tests the resulting package before publishing.

```rust
use zcall::v1::*;

fn main() -> Result<(), Error> {
    let mut records = RecordStorage::<16>::new();
    let mut tokens = TokenStorage::<128>::new();
    // An explicit authoring contract governs defaults such as tool strictness.
    // Backend selection below does not change the message or business logic.
    let mut builder = RequestBuilder::new(
        Protocol::OpenAiChat.into(),
        Presence::Value(Text::new("caller-selected-model")),
        &mut records,
    )?;
    builder.message(Role::User, &[Content::text("Explain a hash table.")])?;
    let mut request = builder.finish();
    let mut output = [0; 1024];
    let report = request.encode(
        Protocol::AnthropicMessages.into(),
        &mut output,
        &mut [],
        &mut tokens,
        EncodeContext::default().with_max_tokens(256),
        Policy::default(),
    )?;
    // The harness sends output[..report.written] using its own transport.
    assert!(report.written > 0);
    Ok(())
}
```

Use `Request::prepare` when sending a model call: it derives requirements from
the actual request, checks caller model/deployment evidence, and runs the target
encoder. `Request::encode` is the lower-level conversion operation and makes no
model-capability promise. Unknown model features or deployment limits remain
unknown unless explicitly accepted, with a per-layer diagnostic report.

## Contract

- Requests, complete responses and events have separate types. `Presence`
  preserves missing, null and values; zero and empty remain values.
- `RecordStorage` holds the actual zconv IR. Views borrow it; edits and appends
  write directly into it. There is no second copied request tree.
- `Session::step` accepts arbitrary byte fragments and a backpressured event
  consumer. `Control::Pause` rejects the current event; retry sees that event
  again. Perform consumer side effects only when returning `Continue`.
- Tools may interleave. `ToolReady` contains a complete validated JSON object;
  execution belongs to the harness. `WaitingTools` ends a model turn and still
  requires harness work.
- Local cancel intent, confirmed cancellation, transport failure, upstream
  failure, malformed input and truncated input have distinct behavior.
- Counters remain integers. Cumulative usage updates replace prior fields;
  cache and thought accounting use the shared zconv accumulator.
- A capacity error names its storage region, required size, exactness and retry
  rule. Always advance stream input by `SessionStep::consumed`.
- The v1 semantic contract is independent of zconv's public record layout and
  token types. Rust type layouts and cross-compiler binary ABI are not promised.

## Implemented adapters

OpenAI Completions, OpenAI Chat Completions, OpenAI Responses, Anthropic Messages,
Gemini GenerateContent and Gemini Interactions have real request, response and
SSE event adapters. Support is limited to representable semantics and tested
versions/profiles. Unknown protocols require an implemented adapter in zconv and
an explicit zcall integration with conformance tests.

Strict conversion is the default. Provider context references, file IDs, opaque
signatures and extension values retain their source namespace. When a target
requires absent facts, the caller supplies them through an explicit context.
Loss permissions are narrow and accompanied by inspectable field-level reports.

## Build and verify

```sh
cargo test --all-targets --locked
cargo +1.74.0 test --all-targets --locked
cargo clippy --all-targets --locked -- -D warnings
python3 scripts/check_package.py
python3 -m unittest discover -s scripts -p 'test_*.py'
```

See [examples/request.rs](examples/request.rs) for backend switching and
[examples/stream.rs](examples/stream.rs) for the shared event consumer. The
[semantic contract](docs/CONTRACT.md), [support matrix](docs/SUPPORT.md),
[resource recovery](docs/RESOURCES.md), [dependency audit](docs/ZCONV_AUDIT.md),
[measurements](docs/PERFORMANCE.md), [validation evidence](docs/VALIDATION.md)
and [release procedure](docs/RELEASING.md) describe the implemented boundaries.

License: MIT OR Apache-2.0.
