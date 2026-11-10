# Release procedure

The Release workflow supports `none`, `patch`, `minor`, `major` and an explicit
stable `MAJOR.MINOR.PATCH`. An explicit version requires `bump=none`. The first
0.1.0 release also uses `none`. Manual releases must run from main. A matching
version tag can trigger the same gates.

## Gates

1. Resolve the exact required zconv version from crates.io. A development Git
   pin is removed only after the registry check, then the registry lockfile is
   generated with Rust 1.74 and the full tests run against it.
2. Validate version input without interpolating it into shell code. Reject
   malformed versions, decreases, conflicting bump/explicit inputs and tag
   mismatches. Preserve a v3 lockfile usable by the MSRV.
3. Run MSRV and stable tests, documentation examples, formatting, Clippy,
   dependency/source audit, release-script tests and verified Cargo packaging.
4. Check crates.io name ownership, repository identity and unpublished version.
   Repeat the registry check immediately before publication.
5. Verify the checkout SHA and current main head. An old manual run cannot
   publish source after main has advanced. An existing tag must point at the
   verified source with the prepared version changes included.
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

The corresponding zconv 0.2.0 changes are committed separately. Publish that
version with zconv's own Release workflow (`bump=none`, `publish=true`) before
publishing zcall 0.1.0. The zcall dependency gate verifies the actual registry
result and builds it; a Git tag alone does not satisfy the gate.

For local review:

```sh
python3 scripts/release.py dependency
BUMP=none python3 scripts/release.py prepare
python3 scripts/check_package.py
cargo +1.74.0 test --all-targets --locked
cargo package --locked --allow-dirty
cargo publish --dry-run --locked --allow-dirty
```

The first command deliberately fails while the required registry version is
unavailable. Development builds use the pinned source and remain testable.
