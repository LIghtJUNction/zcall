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
against the manifest and only runs for marker changes on `main`. Ordinary source
pushes do not publish. A protected `crates-io` environment may still require
approval. Do not reuse an already published version.

## Gates

1. Resolve the exact required zconv version from crates.io. A development Git
   pin is removed only after the registry check, then the registry lockfile is
   generated with Rust 1.99.1 and the full tests run against it. Existing standalone
   fuzz/comparison locks receive the verified zconv registry entry while keeping
   their other dependency versions and checksums.
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

zcall 0.3.0 requires exact zconv 0.4.0. Publish the required zconv version before
zcall, then resolve both the root and standalone fuzz lockfiles from crates.io.
The dependency gate verifies the actual registry result and builds it; a local
path patch or Git tag alone does not satisfy the gate.

Manual releases can bump the version and commit the verified registry manifest
and all affected lockfiles before recording the release tag. Explicit marker
releases must already have their version and registry lockfiles committed, so
the resulting tag identifies exactly the source that passed verification.

For local review:

```sh
python3 scripts/release.py dependency
BUMP=none python3 scripts/release.py prepare
python3 scripts/check_package.py
cargo +1.99.1 test --all-targets --locked
cargo package --locked --allow-dirty
cargo publish --dry-run --locked --allow-dirty
```

The first command fails if the required registry version is unavailable. Local
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
