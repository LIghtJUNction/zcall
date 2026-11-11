# zcall base URL and protocol discovery validation

Date: 2026-11-11. Feature source:
[`413ca14ecfc27a7e060f7c606cf849fdf04ccb81`](https://github.com/LIghtJUNction/zcall/commit/413ca14ecfc27a7e060f7c606cf849fdf04ccb81).
The initial endpoint change is
[`792873c636e9a87ca8acdb5ab816d125a8c78896`](https://github.com/LIghtJUNction/zcall/commit/792873c636e9a87ca8acdb5ab816d125a8c78896).

## Delivered behavior

`HttpEndpoint::new(base_url, format)` validates a borrowed URL and binds it to
an explicit protocol. `url_into` writes the final request URL into caller-owned
storage, preserving API prefixes and query parameters. Complete endpoints are
reused, protocol/path conflicts are rejected, and Gemini model names and SSE
actions are handled explicitly. The existing capability `Endpoint` type keeps
its name and public fields.

`Discovery::new(base_url, model)` provides automatic protocol selection as a
serial Sans-I/O state machine. The caller sends each `next_probe` request with
its HTTP client and passes the complete response to `observe`. A usable
response must have a successful HTTP status, an unambiguous protocol-specific
JSON shape and a valid zconv response schema. `selected()` returns a reusable
`HttpEndpoint`; `attempts()` retains the distinct outcomes of failed attempts.

Both APIs support the existing six adapters: OpenAI Completions, Chat and
Responses; Anthropic Messages; Gemini GenerateContent and Interactions. The
default discovery order is Chat, Responses, Anthropic, GenerateContent,
Interactions, then Completions. The application can supply its own ordered
subset, output-token limit and Chat token-limit field. A complete endpoint
restricts the candidates to its path's protocol and still requires response
verification.

The library stays `no_std`, forbids unsafe code, allocates no heap memory and
depends only on `zconv =0.2.2` with default features disabled. The working
examples include a complete external curl transport; no HTTP crate or runtime
was added to the library.

## Verification

| Check | Observed result |
| --- | --- |
| Integration tests in optimized mode | 109 test functions passed; none failed. |
| README doctests | 2 passed in debug and optimized modes. |
| Endpoint tests | 11 functions cover all six routes, prefixes, queries, full URLs, invalid URLs/models and exact buffer recovery. |
| Discovery tests | 22 functions include 36 cross-protocol response combinations, HTTP errors, mixed envelopes, incomplete/background responses, malformed JSON, resource recovery and state transitions. |
| Loopback HTTP tests | 2 passed: Chat 404 followed by Responses 200; authentication failure across all configured candidates without a selection. |
| Allocation counter | Zero allocations for all six URL/probe paths, probe exhaustion and successful confirmation. |
| Clippy and rustfmt | All targets pass with warnings denied; formatting passes. |
| Dependency/source audit | Exactly zcall and zconv; no production `std`, `alloc`, unsafe code or build script. |
| Release-script regression tests | 29 passed. |
| Cargo package verification | 46 files packaged; the extracted crate compiled successfully against the locked registry dependency. |
| Local cross compilation | `wasm32-unknown-unknown`, library only, default features disabled: passed. |
| External transport example | Local HTTP checks passed for successful fallback, redirect refusal and a response exceeding the 64 KiB limit. |

Test-function counts are not independent statistical samples. Protocol and
capacity combinations are exercised within those functions. The
[feature CI run](https://github.com/LIghtJUNction/zcall/actions/runs/43005752251)
records independent Linux, macOS, Windows, cross-target, quality, Miri and fuzz
jobs; consult its live status for completion. The existing Miri/fuzz jobs retain
their original selected suites, while endpoint/discovery tests run in the
regular platform matrix.

At report creation, all six Linux/macOS/Windows matrix jobs, all three cross
targets, the quality job and fuzz job had passed. The Miri job was still running.

## Performance and memory

The full benchmark uses the source commit above, Rust 1.99.1, zconv 0.2.2 and
an AMD EPYC 9V74 shared host. Each row is the median of seven per-sample means,
20,000 iterations each, following a 200-iteration warmup. The profile enables
optimization, thin LTO and one codegen unit. No network or model generation
runs inside the timed region.

| Local operation | Median | Sample mean range |
| --- | ---: | ---: |
| Validate and build a Gemini SSE URL | 0.480 µs | 0.452–0.505 µs |
| Initialize discovery and construct the first probe | 1.952 µs | 1.927–2.015 µs |
| Initialize, construct and confirm a 229-byte Chat response | 6.833 µs | 5.705–10.855 µs |

On this x86_64 build, `HttpEndpoint` is 56 bytes and `Discovery` is 480 bytes.
These sizes exclude borrowed data, output buffers, token storage and temporary
stack space. They are not an ABI or peak-memory promise. Host contention is
uncontrolled, so the recorded ranges are descriptive rather than confidence
intervals. [Raw measurements](https://github.com/LIghtJUNction/zcall/blob/main/docs/benchmarks/2026-11-11-endpoints.csv) and
[environment details](https://github.com/LIghtJUNction/zcall/blob/main/docs/benchmarks/2026-11-11-endpoints-environment.json) are
included for reproduction.

## Integration and release state

The feature is committed on `main` under `Unreleased`; the manifest remains
0.1.2 and this task does not upload a new registry version. To use the verified
source immediately:

```toml
zcall = { git = "https://github.com/LIghtJUNction/zcall", rev = "413ca14ecfc27a7e060f7c606cf849fdf04ccb81", default-features = false }
```

```sh
cargo run --example discover -- https://your-gateway.example/v1 your-model auto
cargo run --example discover -- https://your-gateway.example/v1 your-model responses
```

The auto command sends small generation probes, which a provider may bill. The
caller supplies a valid model and credential; HTTP authentication, deadlines,
redirect policy and retries remain with the transport. The default output limit
is 16 tokens, and Responses/Interactions request `store:false`. Discovery stops
at the first verified protocol and does not claim that it is the service's only
protocol. Unknown protocols, rejected models and unsupported schema versions
remain explicit unsuccessful outcomes.

The implementation was verified against fixtures and local HTTP servers. It
did not send requests to a paid model service. See the
[endpoint guide](https://github.com/LIghtJUNction/zcall/blob/main/docs/ENDPOINTS.md) for complete use and recovery rules.
