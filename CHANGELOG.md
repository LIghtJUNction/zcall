# Changelog

## Unreleased

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
