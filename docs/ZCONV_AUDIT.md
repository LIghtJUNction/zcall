# zconv integration audit

## Baseline

The initialization audit read zconv 0.1.1 at commit
`ea4d44cdc8da10a58e83282fceb46b9043fe6625`. crates.io's official index reported
that version as published and unyanked, with no dependencies, an empty default
feature and Rust 1.74. The downloaded published archive matched index checksum
`34f66e432c0159637136b9ce91167a514e97b1c02fd4a470a475841a0ed7d306`.

On 2026-11-10, crates.io's API returned 404 with `crate zcall does not exist`.
The project therefore uses zcall consistently. The release gate repeats the
name/repository/owner/version checks immediately before publication.

## Required upstream integration

zcall depends on zconv 0.2.0 at commit
`b905a764b6e9fe719bf968b1a14e21fdcab77d00` during development. It includes these
independent upstream improvements, tested in zconv before zcall uses them:

- `decode_frame_format` shares actual SSE discriminator and terminal marker
  validation with StreamConverter.
- `validate_stream_format` exposes the narrower streaming version/profile check.
- `UsageAccumulator` is shared by event consumers and the original encoder;
  cumulative cache/thought normalization has one implementation.
- `Event.item_id` separates Responses output item identity from a tool call ID.
- `ScratchTooSmall` identifies embedded-JSON scratch growth separately from
  token tape growth.
- Typed finish edits and empty Responses message items preserve their semantics
  through the original encoders/decoders.

The new public Event field affects exhaustive zconv struct literals, so this
change uses 0.2.0. The old 0.1.1 remains an independent published version. The
zcall public API exposes none of those raw event/token/record layouts. Its private
adaptation layer translates them into the stable v1 interface.

## Reused boundaries

| Work | Actual owner/API |
|---|---|
| JSON lexical parsing and validation | zconv::json::parse |
| Complete request/response interpretation | zconv::protocol::decode_request/decode_response |
| Complete output encoding | zconv::protocol::encode_request/encode_response |
| SSE framing and data extraction | zconv::sse::Decoder |
| Provider event interpretation | zconv::stream::decode_frame_format |
| Stream protocol output | zconv::stream::StreamConverter::stage_events |
| Cache/thought usage accounting | zconv::stream::UsageAccumulator |
| Stable views, capabilities, local lifecycle, tool readiness and consumer backpressure | zcall |

Requests and complete responses retain one caller-provided record store, not a
copied second tree. No supplier JSON/SSE parser or writer is copied into zcall.
`scripts/check_package.py` audits the resolved dependency graph and production
sources, including dependency features and target-specific declarations.

## Publication state

The development Git pin is intentional until the required version can be
resolved from crates.io. `scripts/release.py dependency` requires that exact
unyanked registry version, removes the Git pin, resolves the registry lockfile
and verifies the dependency graph before release tests and packaging. A missing
dependency or unavailable registry stops publication; it never publishes a
Git-only package or silently substitutes 0.1.1.
