# zconv integration audit

## Current dependency: zconv 0.2.2 (2026-11-11)

zcall 0.1.2 pins the published `zconv = "=0.2.2"` with all features disabled.
The upstream annotated tag `v0.2.2` points to commit
`5c35f4d98ac61fcf331ef010defd40403f274acc`. The registry lockfile records
SHA-256 `48036db39f0148be3536614d546448cc9a386ce30b43df901a2c779b9e1c1421`.
Its published manifest declares Rust 1.99.1, Edition 2024, no dependencies and
an empty default feature set. The audit below documents the earlier 0.2.0
integration; those historical source and checksum claims do not describe 0.2.2.

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

Development used zconv 0.2.0 at commit
`b905a764b6e9fe719bf968b1a14e21fdcab77d00`. The final manifest pins the actual
published registry version `=0.2.0`. It includes these independent upstream
improvements, tested in zconv before zcall uses them:

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

The official API was rechecked at 2026-11-10 15:10 UTC. zcall remained available;
zconv 0.2.0 had been published at 15:01:36 UTC and was unyanked. Its downloaded
archive matched registry SHA-256
`b2df4342b60d1f96b26342fa0433fa91dee464df9105b2c46c19c2fc39528dc6`.

The archive records source commit
`0eaad6111560d7823511bb3effcd5f7d0354b32e`. That later commit only adds the
upstream repository's development toolchain file, which is outside its published
include list. All 13 published Rust source files and `Cargo.toml.orig` are
byte-for-byte identical to the audited b905a764 commit. The published manifest
declares Rust 1.74, no dependencies and an empty default feature. Both production
source roots remain `no_std` and forbid unsafe code.

`scripts/release.py dependency` has successfully resolved that registry version,
removed the development Git pin, synchronized the independent fuzz lock and
verified the two-package dependency graph. The complete MSRV test suite and
doctest were then rerun against the registry package. A missing dependency or
unavailable registry still stops future releases; it never silently substitutes
another version.
