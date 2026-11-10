# Supported operations and boundaries

Support means the operation is connected to a real zconv adapter and exercised
through zcall. It does not imply every vendor option or every model is supported.

| Adapter | Complete requests/responses | SSE source | SSE target | Version/profile boundary |
|---|---|---|---|---|
| OpenAI Completions | Yes, text prompt/choices | Yes | Yes | v1; no tools or flattening arbitrary chat history |
| OpenAI Chat | Yes, text/tools/media subset | Yes | Yes | v1; explicit compatible profile for declared reasoning extensions |
| OpenAI Responses | Yes, ordered output/input items | Yes | Yes | v1; context and distinct output item identities remain explicit |
| Anthropic Messages | Yes | Yes | Yes | Complete: 2023-06-01 and 2026-10-21; streaming: 2023-06-01 |
| Gemini GenerateContent | Yes | Yes | Yes | Complete: v1/v1beta; streaming: v1beta with SSE framing |
| Gemini Interactions | Yes | Yes | Yes | Independent v1 interactions/steps adapter |

The complete-document test matrix includes all 36 source/target text directions.
Tool and media operations are tested against their actual representable subsets.
The streaming tests use the same consumer for all six sources and encode unified
events to all six targets. Different targets can require caller IDs, timestamps,
usage, output budgets or explicit loss permissions. These are conversion
conditions, not implicit defaults.

## Semantics

| Semantic category | zcall interface | Condition / limit |
|---|---|---|
| Text and ordering | Borrowed Text, messages/blocks/choices | Source order retained; escaping decoded on demand |
| Missing/null/empty/zero | Presence | Target must have an equivalent supported meaning |
| Image/audio/file/video references | Typed media kind and source | Cross-provider file IDs remain source-bound; unsupported target forms fail |
| Function tools and results | Typed definitions, calls, results, ToolReady | Arguments complete to a valid JSON object; tool execution/schema checking are caller-owned |
| Strict function defaults | Authoring/source contract and explicit strict field | Responses unresolved strict defaults require explicit resolution for incompatible targets |
| Structured output | OutputConstraint / validated schema JSON | Verified dialect mappings only, including Chat/Responses portable forms |
| Reasoning and signatures | Thought/summary and opaque signature | Namespace and target semantics must match or be explicitly dropped |
| Refusal and safety | Refusal content and completion reasons | An unsupported target fails instead of turning refusal into a successful answer |
| Usage | Exact integer complete counts, cumulative StreamUsage | Cache/thought normalization is inherited from the shared tested accumulator |
| Context references/extensions | Source format/version/profile | No implicit provider reference substitution |
| Item IDs versus call IDs | Separate view/event fields | Cross-target stream item identity changes require explicit policy and a report |
| Complete output snapshots | SnapshotVerified | Validated; never appended a second time as delta content |

## Explicitly outside the implemented adapter subset

- Unknown protocols, unknown schema versions and unsupported profiles.
- Raw Gemini streamed JSON arrays, NDJSON, WebSockets and network transports.
- Streaming generation of image/audio/video blocks, grounding-specific event
  families and server-executed tool event families without a verified mapping.
- A vendor's entire JSON surface or undocumented compatibility fields.
- Full JSON Schema validation, tool execution, tokenizer/model discovery and
  automatic context-window negotiation.
- Network/tool exactly-once semantics, retries, clocks, randomness and transcript
  persistence.

These operations produce explicit errors when their bytes enter an implemented
adapter. A string identifying a new protocol is not an adapter registration.

## Adding an adapter

1. Add parsing, conversion and exact protocol semantics to zconv first. Keep the
   codec independent of network I/O and supply malformed/capacity tests.
2. Register a distinct typed `Protocol` mapping in zcall's private adaptation
   layer; validate its version and profile at construction.
3. Extend operation-specific adapter capability evidence without declaring model
   support. A format's ability to contain a field remains separate from model
   and deployment evidence.
4. Run the existing stable request/response/session conformance flow with the
   new source and target. Include all byte splits, backpressure, zero capacity,
   terminal-once behavior, IDs, usage and forbidden-loss tests.
5. Publish the dependency version, pin it, and pass the registry packaging gate.
   The release workflow never publishes a Git-only dependency.

This is a statically compiled adapter mechanism: no runtime plugin loader,
dynamic ABI or unchecked JSON passthrough is used.
