# Release procedure

The Release workflow supports `none`, `patch`, `minor`, `major` and an explicit
stable `MAJOR.MINOR.PATCH`. An explicit version requires `bump=none`. Manual
releases must run from main. Version preparation updates the manifest, root and
standalone lockfiles, README dependency example and changelog. If local lockfile
preparation fails, the script restores the files it changed.
Alternatively, a push to `main` modifying the release-request marker publishes
an already-bumped version. Tag pushes do not publish again; successful
publication creates the tag and GitHub release.

For an explicit push-triggered release, first update the manifest and its
lockfiles to the intended version, and commit a matching
`.github/release-request.json` with `crate`, `version`, `zconv` (including the
exact `=` prefix) and `publish: true`. The workflow validates all four fields
against the manifest and the committed Git dependency in `Cargo.lock`, and only runs for marker changes on `main`. Ordinary source
pushes do not publish. A protected `crates-io` environment may still require
approval. Do not reuse an already published version.

## Gates

1. Read the unique zconv Git package from committed `Cargo.lock`. Its repository,
   Git selector and full commit must match the development declaration; the
   working lock's zconv entry must be unchanged. A same-name registry package,
   missing or ambiguous entry, or mismatched legacy exact version is rejected.
   Check that locked package version on crates.io, then create a detached
   `target/release-stage` worktree. Only that stage receives an exact registry
   selector and a registry lock generated with Rust 1.99.1. Verify the resolved
   package's source, version, checksum and empty dependency closure. Existing
   standalone fuzz/comparison locks in the stage receive only that verified
   registry entry; their other pins and checksums remain intact.
2. Validate version input without interpolating it into shell code. Reject
   malformed versions, decreases, conflicting bump/explicit inputs and tag
   mismatches. Preserve a v4 lockfile usable by the declared MSRV.
3. Call the read-only Check workflow on Rust 1.99.1 for Linux x86_64/arm64,
   macOS x86_64/arm64 and Windows x86_64. Each native job runs formatting, the
   default no_std tests, all-feature unit/integration and documentation tests,
   default/all-feature Clippy, release-mode tests and the local TCP example.
   Separately check the default no_std library for WebAssembly, aarch64 Linux
   and Cortex-M, audit both dependency graphs, test release preparation and
   verify Cargo packaging.
4. Check crates.io name ownership, repository identity and unpublished version.
   Repeat the registry check immediately before publication.
5. Verify the checkout SHA and current main head. An old manual run cannot
   publish source after main has advanced. An existing tag must point at the
   verified source with the prepared version changes included and must be part
   of main's history.
6. Publish with `CARGO_REGISTRY_TOKEN`, then record the version, tag and GitHub
   release. No token is used in tests, exposed in logs or passed to an untrusted
   pull-request workflow.

`publish=false` runs preparation and verification without registry publication.
`publish=true` requires the `crates-io` environment's existing rules and the
repository's `CARGO_REGISTRY_TOKEN` secret. This workflow does not alter
environment protection, secret scope or branch protection.

The token variable has exactly the requested Cargo name. No alternative secret
name, personal token, npm tool, build script or release crate dependency is
required. Third-party Actions are pinned to commit SHAs; repository permissions
are read-only outside the publication job. Publication jobs are serialized and
never cancelled by a newer release.

## Required dependency release

Development keeps zconv as a Git-only dependency. For the current committed lock,
the external dependency is exactly zconv 0.4.1; publish that version before zcall.
The version comes from the matched Git package in the committed lock, not from
an optional development `version` field or a registry package with the same name.
Neither a local path patch nor a Git tag proves registry availability.

Version preparation still updates the development crate version, local root
package entries and versioned documentation. Its Git dependency and Git lock
entry stay intact. After successful publication the workflow records only those
development changes on main, preserving the next automatic bump. The release tag
identifies this Git development source; the uploaded crate and staged manifests
record the external registry projection. Registry staging is never committed to
main. Explicit marker releases must already commit their development version
and Git locks.

The five native jobs continue to test the locked development graph. Dependency
graph auditing, Cargo packaging and publication use the isolated registry stage;
both sets of gates are required. A staging directory is single-use: an existing
directory is rejected instead of silently reusing an earlier projection.

For local review:

```sh
BUMP=none python3 scripts/release.py prepare
python3 scripts/release.py dependency
cd target/release-stage
python3 scripts/check_package.py
cargo +1.99.1 test --all-targets --locked
cargo package --locked --allow-dirty
cargo publish --dry-run --locked --allow-dirty
```

The dependency command fails if the required registry version is unavailable. Local
path-patch checks are development evidence only; final publication must pass
the commands above against the published dependency.

## Maintainer commands and local iteration

Ordinary source pushes and pull requests do not start the native matrix. Run
local checks while iterating, and request the complete matrix for a release or
an explicit validation need. These workflows do not create a schedule.

```sh
# Local checks; all network traffic in the server tests stays on local fixtures.
cargo fmt --all -- --check
cargo test --locked --no-default-features
cargo test --locked --all-features --all-targets
cargo clippy --locked --all-features --all-targets -- -D warnings
python3 scripts/check_package.py
python3 -m unittest discover -s scripts -p 'test_*.py'

# Manual validation without publication.
gh workflow run ci.yml --repo LIghtJUNction/zcall --ref main -f extended=false
# Include the existing Miri and bounded fuzz jobs when needed.
gh workflow run ci.yml --repo LIghtJUNction/zcall --ref main -f extended=true

# Release from main; choose patch, minor or major as appropriate.
gh workflow run release.yml --repo LIghtJUNction/zcall --ref main \
  -f bump=minor -f publish=true
```

The formal release invokes the five native jobs explicitly through
`workflow_call`; it does not rely on a token-created tag starting a second
workflow. Registry publication still needs the existing `CARGO_REGISTRY_TOKEN`
and any `crates-io` environment approval. A workflow file alone does not prove
those credentials are configured. Failed publication must be inspected before
retrying: a published crate version is immutable, and no existing release tag
should be moved to recover a later step.
