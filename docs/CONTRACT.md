# v1 semantic contract

## Responsibility and stability

`zcall::v1` describes one model invocation and its result. It never performs
network I/O, authenticates, retries, chooses a model, executes a tool, persists a
conversation, reads a clock or decides whether an agent has finished its task.
Those decisions stay with the harness. A response that requests tools ends the
model turn in `WaitingTools` and leaves the agent's task open.

The contract covers field meanings, presence, event ordering, terminal behavior,
error classes, usage accounting, capability evidence and explicit degradation.
The implementation can change private zconv storage without changing these
meanings. Rust source compatibility is governed by crate SemVer. No promise is
made about Rust type layout, a C ABI, dynamic linking or cross-compiler ABI.

## Requests and provenance

`RequestBuilder` writes directly into caller-owned `RecordStorage`. Importing a
wire request uses zconv's JSON parser and protocol decoder. Both paths produce
the same editable request representation. Views borrow the request, so mutable
edits cannot occur while a view is live. There is one arena of message, block,
tool and extension records, plus a fixed-size header.

A builder's explicit authoring format governs defaults whose meaning differs
between protocols, such as function strictness. It is not a model capability
declaration. The format can be selected once at the integration boundary; the
ordered messages, tools and business logic remain unchanged when the target
backend changes. Imported requests always retain the actual source identity.

`Presence::Missing` means the source did not specify a field. `Null` means an
explicit JSON null. `Value` includes empty strings, empty containers, false and
zero. Setters preserve that distinction and encoders reject unrepresentable
target semantics. No unknown count or capacity is silently replaced by zero.

`Text` borrows either UTF-8 or a validated JSON string. An unescaped string is
available through `as_str`; an escaped one can be compared without allocation
or decoded into a caller buffer. `Json` preserves exact validated bytes. Integer
usage values remain integers or borrowed decimal JSON; no floating-point cast
is used. A value outside u64 can remain inspectable in a complete response.

Messages and their blocks preserve order. Roles, media reference kinds,
function names, arguments, results, refusal and reasoning are explicit types.
Thought text, public summaries and opaque signatures have different meanings.
Tool argument JSON and ordinary text are separate `Data` variants.

## Identities and context

An overall response ID, an output item ID and a tool call ID have distinct
namespaces. They are exposed separately. zcall never substitutes a call ID for
an output item ID or fabricates a response ID, timestamp or token count.

`EncodeContext` carries facts missing from a source: transport model, required
maximum output tokens, response timestamps and tool-name/ID mappings. Gemini
model and streaming selection can belong in the transport URL; the encode
report returns them for the harness instead of hiding a network operation.

Stored server context, provider file IDs and opaque extension values retain
their source format/version/profile. Changing providers does not rename these
references into target references. The harness must materialize history or
explicitly permit dropping a reference, receiving the resulting loss report.

## Capabilities and preparation

`Request::prepare` is the sending boundary. It derives requirements from the
actual IR, checks the selected adapter operation, the protocol's expressible
subset, caller-provided model evidence and deployment restrictions, then runs
the target encoder against the current input. Its output is uncommitted until
the entire operation succeeds.

`Supported`, `Unsupported` and `Unknown` remain distinct. An image field in a
protocol supplies no evidence that a particular model supports images. Model
and deployment declarations contain their source and optional revision.
Context windows, output limits and named reasoning levels remain unknown when
the caller has not supplied evidence. Token counts must come from the caller's
tokenizer or another explicit measurement; byte counts are not token counts.

Strict policy rejects required unknown capabilities, incompatible features and
unverified conditions. Narrow options permit specified uncertainty, with a
diagnostic for each affected requirement and layer. A condition validated by
the actual encoder is marked separately from a condition merely accepted by
policy. Report capacity is caller-owned and recoverable from zero.

`Request::encode` is a lower-level semantic conversion operation. It checks
representability, context and losses but does not promise a model/deployment
capability check. This distinction lets offline converters use the crate
without inventing model evidence.

## Stream submission and lifetimes

Construct a `Session`, mark its outgoing request prepared, then call `step` with
bytes and a synchronous consumer. `SessionStep` reports consumed input, accepted
events and progress. The consumer returns `Continue` to accept an event or
`Pause` to reject it. Pause leaves that exact event pending. Consumer side
effects belong only to the Continue branch.

An event's strings and JSON borrow the session only for the consumer call.
The higher-ranked callback lifetime prevents storing those borrowed views in a
longer-lived queue. A harness that needs an asynchronous work item must copy the
necessary data into its own bounded storage. Calling `step` again cannot leave
an exposed borrowed pointer dangling because the earlier callback borrow ended.

SSE framing, CR/LF handling, UTF-8 replacement and protocol JSON parsing belong
to zconv. Fragments can split an escape, a UTF-8 sequence, a CRLF pair or any
other byte boundary. One call can contain many events and a single event can
span many calls. Committed events and consumed bytes are never replayed to
recover a capacity failure.

Repeated wire begin metadata is checked against the current identity and
normalized into one logical Started event. Usage can arrive before Started;
consumers must not assume the first event contains the response ID. Text delta
events need not have an explicit preceding block-start event in a source that
does not provide one. Keys expose choice, item and part indices.

When real identity fields arrive later, `IdentityUpdated` exposes the newly
known identity and `Session::identity` returns the current view. Previously
known fields must remain consistent. An event encoder checks these updates
against explicit caller context before writing target identity fields.

`Event::reasoning_kind` distinguishes `Thought`, `Summary` and `Unspecified`.
An unspecified compatible-protocol field is not evidence of either meaning.
An encoder rejects translation between these meanings unless an exact allowed
loss applies; it does not present a public summary as private thought text.

`SnapshotVerified` is validation of a completed snapshot, not content to append.
Tools always retain complete argument bytes for exact validation. Responses text
snapshots are checked byte for byte by default. Explicitly disabling text
snapshot retention selects a documented length-only check, reducing aggregation
storage without changing delta-authoritative output.

## Tool and call lifecycle

The session distinguishes idle, prepared, receiving, tool formation, waiting
for results, resolved tools, completed, incomplete, upstream failure, local
cancel intent, confirmed cancellation, truncation and local failure.

Tool argument fragments may be incomplete JSON. Before ToolReady, the aggregate
must be a complete JSON object under the configured parser limits. This is a
JSON-object validity contract; it does not claim full JSON Schema validation of
a tool's parameters. The harness may apply its domain/schema validator before
executing a tool.

Interleaved tool calls have separate identities and ordered argument aggregates.
ToolReady is submitted once per completed tool. `submit_tool_result` records the
association with a known ready call; an unknown ID, duplicate result or illegal
state yields a lifecycle error. Append the actual result to a caller-managed
follow-up `Request`; the kernel does not retain an unbounded transcript.

`Session::tool(call_id)` borrows the retained name and complete arguments after
ToolReady, allowing the harness to inspect or copy them outside the callback.
`pending_tool_count` reports ready calls whose results have not been submitted.
The borrowed tool view prevents mutable session reuse while it is still live.

`request_cancel` records local intent. The transport can still deliver a racing
normal completion. `confirm_cancel` records a transport-confirmed cancellation.
`transport_failed` is a caller-reported transport failure, distinct from an
upstream error event and a parser/converter failure. A single terminal event is
accepted locally. Repeating terminal input or sending new bytes after terminal
state is a lifecycle error; terminal events are not emitted twice.

Explicit EOF must follow the source's termination rule. Missing protocol
termination produces Truncated. GenerateContent SSE ends after valid candidate
finish reasons and caller EOF. A terminal model turn that needs tool results
does not claim completion of the outer agent task. These local rules do not
promise distributed exactly-once execution or delivery.

`StepStatus::Done` means the turn's bytes and events have ended. The terminal
outcome and `CallState` distinguish success, waiting for tools, incomplete output,
upstream failure and confirmed cancellation; Done alone is not a success flag.

## Usage, losses and recovery

Stream usage is cumulative. A present update replaces that source field, while
an absent update preserves the previous known field. zconv's shared accumulator
normalizes Anthropic cache reads/writes and Gemini thought counts, using checked
arithmetic. It retains underlying source bases so partial cumulative updates
cannot add the same usage twice. Overflow is a semantic error.

Nonstream loss policies separately permit extensions, reasoning, stored context
and metadata. `losses` enumerates actual affected fields with source identity and
semantic location. Stream loss permissions list exact field classes; loss
events identify the source and affected field. A permission does not justify
discarding other unlisted data.

Errors have stable classes for JSON, protocol input, unsupported adapter/version/
profile/feature, unknown or mismatched capability, missing context, forbidden
loss, capacity, configured limits, truncation, lifecycle, upstream failure and
transport failure. Capacity failures additionally name the region, needed
capacity, exactness and retry ownership. See [RESOURCES.md](RESOURCES.md).
