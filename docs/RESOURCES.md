# Resource ownership and recovery

The harness owns every growing region. `TokenStorage<N>`, `RecordStorage<N>`,
`StreamWorkspace<T,E,I>` and `EncoderWorkspace` expose capacity and byte costs.
The arrays have private, correctly typed elements; an arbitrary unaligned byte
buffer is never reinterpreted as Rust objects. There is no hidden heap, cache or
global mutable state.

Complete documents borrow their input. Request and response headers are fixed
size; record storage contains the actual zconv IR. An input can outlive the token
tape, because retained JSON/text values borrow only the original bytes. Reusing
records requires dropping their live views first.

For a live edited document, `replace_storage` grows its record store in place.
If the consuming `relocate` alternative fails, `RelocationError::original`
returns the original document together with its capacity error; prior edits and
the usable handle are retained. A failed growth attempt does not discard work.

| Region | Meaning | Needed value | Retry rule |
|---|---|---|---|
| Tokens | JSON token tape | Lower bound at the failing token | Retry the document, or replace stream workspace and keep the unconsumed suffix |
| Ir | Complete request/response records | Decoder lower bound; builder operations exact | Retry decode or grow existing typed storage; failed builders keep prior content |
| Output | Complete encoded bytes | Exact | Discard partial output, enlarge, repeat the encode; no input consumed |
| Scratch | Decoding a JSON string containing tool argument JSON | Exact | Enlarge scratch and repeat; request/response unchanged |
| Framing | Current SSE wire event | Lower bound | Replace session buffer; consumed input remains retained |
| Payload | Decoded/joined SSE data | Exact | Replace payload; original complete source event remains buffered |
| Events | Current frame's event IR | Lower bound | Replace workspace; committed earlier events remain committed |
| Items | Active choice/content/tool state | Lower bound | Replace workspace, keeping retained state |
| Aggregate | IDs, complete arguments and required snapshots | Lower bound unless explicitly marked exact | Replace aggregate; earlier accepted data is preserved |
| Reports | Capability diagnostics | Exact | Enlarge diagnostics and repeat preparation before sending |
| Context | Caller tool identity/name bindings | Exact | Enlarge typed binding storage before encoding |

`Error.capacity` is the authoritative value for a particular failure. A configured
parser/depth/input limit is a Limit error, not a request to silently allocate more
storage. Errors mark retry ownership as `Unconsumed` or `Retained`.

## Stream loop

Advance the transport cursor by `step.consumed` for every result, including
NeedBuffer, NeedEvents and Failed. Never resend that prefix. When a buffer needs
growth, replace the named buffer or workspace and retry using only the remaining
suffix. A callback returning Pause has not accepted the current event; resume
with a consumer that can accept it. `emitted` counts accepted events.

An exhausted output buffer in `EventEncoder` is drained independently of staging
the next event. Its pending bytes retain a cursor, so one-byte and zero-byte
outputs do not duplicate previously written bytes. An event rejected before
staging can be retried after capacity recovery.

## Aggregation cost

Ordinary text deltas do not create an unbounded conversation transcript. Tool
arguments must be retained through completion for validation. Default Responses
text snapshot checking also retains text for byte-for-byte comparison. These
costs scale with the retained content and appear in aggregate capacity/peak
statistics. Choosing a larger caller buffer is visible in the memory report.

Session item tables and checkpoints are separate caller-sized tapes. Checkpoints
allow a failed state transition to roll back without corrupting committed
items. They are included in workspace byte totals. Completed model turns may
retain tool identities until result association or explicit reset; the caller
chooses when to recycle the session.

See [PERFORMANCE.md](PERFORMANCE.md) for measured type sizes and benchmark
configuration. These are target/compiler measurements, not a Rust layout ABI.
