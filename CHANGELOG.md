# Changelog

## 0.2.0 (unreleased)

- Target zconv 0.3.0 while retaining the v1 request, stream and tool lifecycle semantics.
- Preserve schema dialects, provider-bound block/tool signatures, explicit ObjectOrText data, images, grounding, continuation and stop-sequence events.
- Preserve cache lifetime/miss counters, tool-use prompt counts and exact wire cost numbers in StreamUsage. Unknown counters remain absent.
- Retain raw uncached input alongside canonical inclusive input, and preserve explicit signature origin labels through public events and payload repacking.
- Rust source migration: handle the new Data/EventData/StopReason variants and initialize the new public signature, schema_dialect and StreamUsage fields. Existing constructors keep explicit defaults. Continuation finishes an incomplete turn rather than a completed answer.
- Accept repeated Responses identity envelopes only after checking existing identity/timestamp consistency.

## 0.1.3

- Add borrowed `HttpEndpoint` configuration for six protocols, caller-buffered URL
  construction, explicit API prefixes, complete endpoints, query preservation
  and Gemini model/stream routing.
- Add bounded Sans-I/O `Discovery` with configurable candidate order and token
  limits, protocol-specific response validation, inspectable attempt outcomes
  and reusable endpoint selection.
- Add endpoint examples, a runnable curl-backed discovery client, URL and
  discovery regression tests, and local HTTP integration coverage.

## 0.1.2

- Upgrade the sole runtime dependency from published `zconv =0.2.0` to
  `zconv =0.2.2`, preserving the stable `v1` public interface and `no_std` contract.
- Revalidate the exact crates.io dependency, caller-owned workspace paths,
  stream lifecycle, test suite, and standalone fuzz lockfile on Rust 1.99.1.
- Add a separately validated, explicit `main` release-request trigger with
  exact crate/version/dependency matching; prevent tag pushes from republishing.

## 0.1.1

- Upgrade the declared MSRV and pinned development toolchain to Rust 1.99.1,
  the latest stable release verified on 2026-11-11; adopt Rust Edition 2024.
- Update CI, release verification, version-bump locking and package audits to
  Rust 1.99.1 and Cargo lockfile v4. Keep the sole zconv dependency and no-std
  library contract intact.

## 0.1.0

- Add the `v1` request, response, capability and event contract over zconv.
- Add caller-owned IR builders, borrowed views, edits and detailed loss reports.
- Add resumable SSE sessions, checked tool lifecycles and event encoding.
- Add explicit cancellation, transport errors, terminal-once behavior and usage accounting.
- Add Rust 1.74, platform and no-std validation, benchmarks and guarded version-bump publication.
