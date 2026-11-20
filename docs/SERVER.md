# Optional HTTP and Unix gateway

The default build remains the allocation-free, safe Rust, `no_std` call kernel.
The `server` feature enables `std` and adds a bounded HTTP server, Unix socket
ownership, caller-key management and a gateway for the six registered
conversation protocols. Both feature graphs contain only zcall and zconv.

## Responsibilities

| Component | Library responsibility | Host responsibility |
| --- | --- | --- |
| `Server` | Fixed workers, bounded admission, cancellation and shutdown reports | Process/service lifecycle and optional native accept loop |
| `transport::Listener` | TCP accept; private, owned Unix socket binding and cleanup | Verified UID and optional native stale-socket probe |
| `Handler` | Extension point for any HTTP path or additional format | Route semantics and application policy |
| `gateway::Gateway` | Authentication, model-route callback, registered JSON/SSE conversion | Upstream transport, TLS and provider credentials |
| `server::auth` | Private key schema, metadata, insertion, rotation and revocation | Secure randomness, atomic persistence and synchronization |
| `Connection` / `PeerInspector` | Timeout, cancellation, half-close and peer-hook contracts | Native peer credentials and trusted locality |

Enable `features = ["server"]` on the versioned dependency. The feature adds
heap buffers and threads; the default kernel does not. It adds no HTTP/TLS or
serialization crate. OS networking and filesystem calls come from Rust's
standard library. This is not a claim that a native binary has no OS libraries.

## Run a real local example

```sh
cargo run --features server --example server -- --check
cargo run --release --features server --example server
```

The first command opens an ephemeral loopback listener, sends real HTTP
requests in all six registered formats and verifies the fake upstream's replies.
The second serves at `127.0.0.1:9416` until Enter is pressed. Its upstream is an
in-process fixture: it never contacts a provider or reads an account.

For an application, construct `Gateway::new(router, transport)`, pass it to
`Server::new(config, handler)`, then call `listen(Listener::bind_tcp(...))`.
`Router` chooses the upstream protocol/model and explicit provider headers.
`Transport::execute` returns an owned response reader and must honor the given
cancellation/deadline. `UpstreamBody::cancel` is called when the response is
dropped, including a downstream error; it must be nonblocking and idempotent.
A host can retain its existing TLS client or use Unix upstream transport.

A custom `Handler` can own additional paths or new protocols. The server does
not install health, management, model-list, plugin or CORS routes. `Gateway`
rejects unknown conversation endpoints instead of pretending to convert them.

## Protocol and HTTP boundaries

The registered conversation formats are OpenAI Completions, Chat Completions
and Responses, Anthropic Messages, Gemini GenerateContent and Gemini
Interactions. The gateway tests exercise all 36 source/target combinations for
common text and usage semantics over real local TCP. This does not mean every
provider-specific field has an equivalent in every target. A response message
identity with no target representation, for example, produces `ForbiddenLoss`;
it is not silently discarded. JSON and fragmented SSE use the same strict
zconv-backed conversion as the standalone kernel.

Inbound caller credentials, Cookie and arbitrary request headers are never
forwarded upstream. Provider credentials must be supplied explicitly in the
selected `Target`. Successful finite responses require JSON; streaming requests
require an SSE media type. Body and response limits apply during reads/writes,
and owned upstream bodies are cancelled on error.

`http::read_request` defaults to JSON API clients and rejects browser Origin
headers and encoded bodies. `read_request_with_policy` lets the host explicitly
permit Origin, opaque encoded bytes or another request media type. Duplicate
fields, ambiguous length/transfer framing, size limits and deadlines remain
strict. `Expect` negotiation and request trailers are unsupported.

A well-formed HTTP/1.1 GET WebSocket handshake receives `426 Upgrade Required`
with a message to use HTTP/SSE. This is a transport fallback signal, not a
WebSocket connection: no `101` response or WebSocket framing is implemented.
Malformed or other Upgrade requests remain `400`; Origin, Expect and duplicate
header checks retain their earlier priority. The decision does not depend on
an application path.

Response headers are validated before output. Early rejection writes the
response, closes the write half, then discards at most 64 KiB of input for at
most 200 ms. HEAD replies omit the body. A streaming callback that panics after
writing cannot append a second HTTP response to the partial stream.

`read_request_with_decoder` accepts an optional host `BodyDecoder`; a complete
`Server` accepts the same hook through `Server::with_hooks`. Without that hook,
encoded bodies are rejected with 415 unless the host explicitly requests opaque
encoded bytes. The decoder receives the normalized encoding name, bounded wire
bytes, decoded-byte limit and the original absolute read deadline. The library
checks cancellation and that deadline before and after decoding, validates the
returned size, removes transfer/content encoding and replaces Content-Length
with the decoded length. Both wire and decoded bodies have independent bounds.

A host decoder must also bound its codec window and temporary allocations, and
cooperate with cancellation and the supplied deadline. These checks cannot
preempt an arbitrary callback. zcall contains no native decompressor or unsafe
codec binding; a host can supply its existing implementation.

## Unix sockets and peer identity

`Listener::bind_unix(path, verified_uid)` requires an absolute path with real
directory ancestors and an existing private parent owned by that UID. It does
not trust an environment variable for identity or create parent directories.
The socket and stable lock use mode `0600`. Non-socket nodes, symlinks, an
untrusted owner or an unsafe lock are rejected without replacement.

The default bind operation refuses an existing socket. A host may use
`bind_unix_with_probe` with its bounded native liveness probe to confirm a stale
owned socket. Rust's standard library has no Unix connect timeout, so the
library does not hide an unbounded connection probe in the default path.

The ownership guard is shared by the listener and accepted stream clones.
Cleanup waits for the last guard and checks the socket's device/inode before
unlinking it. A replacement node is preserved. The stable lock file remains
for later binds, preventing old and new waiters from locking different inodes.

A local socket location is not an OS UID/PID assertion. `PeerInspector` can read
native peer credentials through a borrowed `SocketRef`. Hosts using their own
accept loop submit an already verified `Peer`; request headers never supply it.

## Admission and shutdown

`ServerConfig` fixes worker count, queue size, request/response limits and
read/write/request deadlines. The built-in acceptors block in the OS and idle
workers wait on a condition variable. There is no listener polling timer.
Queue saturation produces a bounded `503` response. A native host can omit
`listen` and use `submit(connection, peer)` or `submit_boxed` with any
`Connection + Send` implementation.

`shutdown(budget)` stops admission, cancels active work, discards queued work
on tracked workers and uses one shared deadline. It returns the counts of
remaining workers, acceptors and wake helpers. A user handler, upstream reader
or connection destructor that does not cooperate is reported rather than
forcibly killed. A later call can reap it after it finishes. The tests include
both an uncooperative handler and a blocked queued connection destructor.

The built-in `Handler` is shared and requires `Send + Sync`. It is not a
worker-local factory for application types that are `!Send`. zcalld therefore
retains its thread-local plugin/curl orchestration while sharing the HTTP,
protocol, auth and connection primitives.

## Caller keys

`CallerKeys::from_json` and `auth::load` retain the existing private JSON array:
`id`, `name`, `secret`, `lan` and `workspace`, including anonymous legacy items.
The parser rejects ambiguous identities/secrets, unknown fields, invalid types,
excessive entries and oversized files. On Unix, `load` requires a regular file
without group or other permission bits.

`entries()` exposes metadata only. `ApiKey`, `CallerKeys`, provider `Target`
and outbound request types do not implement secret-printing Debug output.
Authentication checks valid keys before loopback trust so a verified key keeps
its server-owned workspace. Remote peers require a key with `lan` permission.

Insertion and rotation validate before mutation. Revocation applies to later
authentication; it does not cancel accepted requests. `to_json` returns a
secret-bearing buffer for the host's private atomic writer. The library does
not generate weak random keys or force a file-format migration.
`KeyAuthenticator` can share `Arc<RwLock<CallerKeys>>` for in-process updates.

## Host integration and evidence

The HTTP/protocol extraction starts from MIT-licensed zcalld source
`226764bd980419bfd123bf297b891e09ccdc94c1`, retaining its protocol and staged-close
regressions. zcalld can re-export `server::http` and `server::protocol`, delegate
caller-key parsing to `server::auth`, and implement `Connection` on its existing
stream. Its cancellation flag and JSON error name remain host choices.

Signals, systemd, provider/plugin policy, privacy, migration and account storage
remain application responsibilities. Local tests use only buffers, private
temporary paths, loopback TCP and Unix sockets. The formal workflow runs the
server feature on five native targets and separately checks the default
no_std kernel on WebAssembly, ARM Linux and Cortex-M.
