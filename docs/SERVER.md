# Optional server primitives

The default build remains the allocation-free, safe Rust, `no_std` call kernel.
The `server` feature enables `std` and adds bounded HTTP buffers, conversation
protocol conversion and caller-key management without another crate dependency.

This first service-layer increment lets an existing host adopt the primitives
without changing its listeners, upstream transport or configuration files. A
standalone listener and worker scheduler are not part of this increment.

## Feature boundary

| Module | Library responsibility | Host responsibility |
| --- | --- | --- |
| `server::http` | HTTP/1 framing, limits, timeouts, errors, chunked writes | Route dispatch and response policy |
| `server::protocol` | Six registered conversation protocols and strict JSON/SSE conversion | Model selection and upstream transport |
| `server::auth` | Key parsing, verified metadata, insertion, rotation and revocation | Secure randomness, persistence and synchronization |
| `server::Connection` | Read/write, timeout, write half-close and cancellation | Accepted connection, lifecycle and OS peer metadata |

Enable `features = ["server"]` in a source checkout. The manifest remains at the
last published version until the normal release transaction bumps it; this
increment is not yet a new registry release.

## HTTP behavior and application policy

`http::read_request` uses safe JSON API defaults. It rejects browser Origin
headers, encoded request bodies, duplicate fields, ambiguous length and
transfer framing, oversized input and incomplete requests. POST bodies require
a length or chunked framing.

A custom route can use `read_request_with_policy` and `RequestPolicy` to allow
Origin, opaque encoded bytes or media types other than JSON. These application
choices do not relax framing or size validation. The host decides which
origins, methods and paths it accepts; the library installs no management
routes or CORS policy. Upgrade handshakes, trailers and `Expect` negotiation
are currently unsupported.

Response fields are checked before output starts. Newlines, duplicate extra
fields and extra framing fields are errors. Early rejection writes the
response and closes the write half, then discards at most 64 KiB of remaining
input for at most 200 ms. Existing TCP delayed-body regressions and a Unix
socket test exercise this behavior.

Implement `Connection` for a host stream to preserve timeout and cancellation
rules. Built-in implementations cover `TcpStream` and Unix `UnixStream`.
`cancelled()` is checked between bounded reads/writes; `error_type()` retains a
host's JSON error name. Peer UID, PID and ownership remain host inputs, never
request-header claims.

## Protocol conversion

`protocol::ingress` recognizes OpenAI Completions, Chat Completions and
Responses, Anthropic Messages, Gemini GenerateContent and Gemini Interactions.
`inspect`, `endpoint`, `prepare_request`, `convert_response` and
`with_stream_converter` preserve strict zconv-backed JSON and SSE semantics.

Unknown protocol names and paths return errors. New formats need an explicit
adapter or custom handler; they are not silently treated as known protocols.
Images, files and other resource codecs in `zcall::api` are separate from these
conversation routes.

The host supplies its upstream sender, including TLS and provider credentials.
It can retain its existing HTTP client or use a local mock. Upstream account
secrets never become inbound caller credentials.

## Caller keys and existing files

`CallerKeys::from_json` and `auth::load` read the existing private JSON array
with `id`, `name`, `secret`, `lan` and `workspace`. Legacy optional metadata
remains optional. Duplicate identities or secrets, unknown fields, invalid
types, excessive entries and oversized files are rejected. On Unix, `load`
requires a regular file with no group or other permission bits.

`ApiKey` and `CallerKeys` intentionally omit `Debug`. Public metadata and
`VerifiedCaller` omit secrets. Authentication checks configured keys before
loopback trust, retaining a valid key's server-owned workspace. A remote peer
can use only a key whose `lan` flag permits it. Accurate peer locality is the
host's responsibility.

Insertion and rotation validate before mutation. Revocation affects subsequent
authentication; it does not cancel accepted requests. `to_json` returns a
secret-bearing buffer for a host-owned private store. There is no automatic
file migration or weak random generator. A host should lock modifications and
atomically replace its private credential file.

## zcalld integration shims

The initial extraction comes from MIT-licensed zcalld source
`226764bd980419bfd123bf297b891e09ccdc94c1`; protocol and staged-close regression
tests were retained.

1. Enable `zcall/server`.
2. Re-export `server::protocol` from the old protocol module.
3. Implement `Connection` on the existing TCP/Unix wrapper using the host
   cancellation flag and existing error type.
4. Re-export or delegate HTTP entry points to `server::http`.
5. Delegate caller-key parsing and lookup to `server::auth`, retaining the
   existing private writer and secure random source.

The host keeps native accept/wait, signals, systemd, provider routing, plugins,
privacy, migration and its upstream client. These application policies are not
required by the reusable kernel.

## Local validation

```sh
cargo test --locked --no-default-features
cargo test --locked --features server --all-targets
cargo clippy --locked --features server --all-targets -- -D warnings
```

New tests cover request policy, ambiguous framing, response injection,
cancellation, Unix HTTP framing, compatible key files, rotation/revocation and
verified workspace metadata. They use local buffers and sockets, without real
provider calls or credentials.
