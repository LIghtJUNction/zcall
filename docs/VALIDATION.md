# Validation and acceptance evidence

## Current dependency upgrade (2026-11-11)

The zcall 0.1.2 patch release updates its sole runtime dependency to published
`zconv = "=0.2.2"`, with default features disabled. The locked registry archive
checksum is `48036db39f0148be3536614d546448cc9a386ce30b43df901a2c779b9e1c1421`;
the upstream release tag points to commit
`5c35f4d98ac61fcf331ef010defd40403f274acc`. Both crates declare Rust
1.99.1 and Edition 2024; neither introduces `std`, `alloc`, a build script, or
third-party runtime dependencies beyond zcall's direct zconv dependency.

The upgraded dependency passed the following checks on Rust 1.99.1:

| Check | Observed result |
|---|---|
| `python3 scripts/release.py dependency` | Registry resolution, exact version, two-package graph, and standalone fuzz lock checksum verified. |
| `cargo +1.99.1 test --all-targets --locked` | 73 integration tests passed; examples and benchmark target compiled and ran. |
| `cargo +1.99.1 test --release --locked` | 73 integration tests and one doctest passed in release mode. |
| `cargo +1.99.1 test --doc --locked` | One doctest passed. |
| `cargo +1.99.1 clippy --all-targets --locked -- -D warnings` | Passed with warnings denied. |
| `python3 scripts/check_package.py` | Exactly zcall → zconv; source audit found no `std`, `alloc`, `unsafe` or build script. |
| `python3 -m unittest discover -s scripts -p 'test_*.py'` | 29 release-script tests passed, including explicit push-request validation. |
| `cargo +1.99.1 fmt --all -- --check` | Passed. |
| `cargo +1.99.1 package --locked --allow-dirty` | Package built and verified against the registry dependency. |

These checks establish source compatibility and the tested behavior of this
patch release; they are not a new throughput measurement or a guarantee about
untested model/endpoint capabilities. The historical 0.2.0 acceptance evidence
below remains available for comparison.

The original 2026-11-10 acceptance passed 73 integration test functions and one
README doctest on stable Rust 1.99.1 and the **then-declared** MSRV, Rust 1.74.0.
On 2026-11-11, zcall moved to Edition 2024 and declared Rust 1.99.1 as its new
minimum compiler. The original observations below remain historical evidence;
current release requirements and regression checks are recorded separately.

The counts describe test functions. A function may loop over protocols,
fragments, capacities, or invalid inputs; these iterations are correlated
checks, not independent samples or a statistical confidence estimate. The
acceptance mapping below states what the fixtures actually exercise.

## 1. Executed evidence and source identity

The completed development runs used zconv 0.2.0 at the reviewed commit
`b905a764b6e9fe719bf968b1a14e21fdcab77d00`. The published zconv 0.2.0 archive has
since been retrieved from crates.io. Its 13 Rust source files match that reviewed
commit byte for byte. The archive identifies the later development-toolchain commit
`0eaad6111560d7823511bb3effcd5f7d0354b32e` in `.cargo_vcs_info.json`.

The registry archive SHA-256 is:

```text
b2df4342b60d1f96b26342fa0433fa91dee464df9105b2c46c19c2fc39528dc6
```

The exact registry dependency
`zconv = { version = "=0.2.0", default-features = false }` has now been resolved
and verified by the release dependency script. The root and fuzz lockfiles use
the actual registry source and checksum. Registry-based MSRV and stable release
tests also passed, as distinguished below. Source equality supports the
provenance check; it does not replace dependency resolution or packaging. See
[ZCONV_AUDIT.md](ZCONV_AUDIT.md) and the [release gates](RELEASING.md).

### Test inventory

| Source | Test functions | Main purpose |
|---|---:|---|
| [tests/allocation.rs](../tests/allocation.rs) | 1 | Observe allocations during request import, edit, encoding, and a complete stream. |
| [tests/capability.rs](../tests/capability.rs) | 22 | Separate adapter, format, model, deployment, and input evidence; enforce budgets and explicit uncertainty. |
| [tests/document.rs](../tests/document.rs) | 25 | Complete request/response conversion, edits, identities, losses, and bounded storage recovery. |
| [tests/session.rs](../tests/session.rs) | 25 | Incremental input, backpressure, tools, usage, terminal states, and event encoding. |
| **Integration total** | **73** | The sum of the four test binaries above. |
| [README.md](../README.md), included by [src/lib.rs](../src/lib.rs) | 1 doctest | Compile and run the public request construction/encoding example. |
| [scripts/test_release.py](../scripts/test_release.py) | 26 Python tests | Version, registry, source, branch/tag, and independent lockfile publication rules. |

### Completed runs

| Check | Observed result | Scope of the result |
|---|---|---|
| Local Linux, Rust 1.99.1 | Debug and release integration tests passed; README doctest passed. | The tested development source and fixtures. |
| Local Linux, Rust 1.74.0 | Debug and release integration tests passed; README doctest passed. | Actual execution on the minimum declared on 2026-11-10, before the Edition 2024 migration. |
| Registry dependency, Rust 1.74.0 | All-targets tests and doctests passed: 73 integration functions and one README doctest. | A new run using the resolved crates.io dependency. |
| Registry dependency, Rust 1.99.1 | Release tests passed: 73 integration functions and one README doctest. | A new optimized run using the resolved crates.io dependency. |
| `wasm32-unknown-unknown` | Library cross-check passed. | Compilation; no browser or WASM runtime execution. |
| `aarch64-unknown-linux-gnu` | Library cross-check passed. | Compilation; no native aarch64 workload execution. |
| `thumbv7em-none-eabi` | Library cross-check passed. | Compilation for a target without a hosted standard library. |
| Registry quality/cross-check | Clippy with warnings denied and the `thumbv7em-none-eabi` library check passed. | These checks were repeated with the registry dependency. |
| Release scripts | All 26 Python tests passed. | Registry responses are mocked in unit tests; Git commit/tag checks also run against real temporary local repositories. |
| Local Miri | All 72 selected functions passed: capability 22, document 25, session 25. The session suite took 479.16 seconds. | Used Git commit `b905a764`; its 13 production Rust files match the registry archive. The allocator-hook test is outside the selected Miri suites. |
| Local libFuzzer smoke | 5,000 executions completed without a reported crash. | The bounded, instrumented run described in section 3. |
| Published zconv archive | SHA-256 and all 13 Rust source files verified. | Registry provenance and source equality, separate from the new registry test runs above. |
| Cargo packaging | `cargo +1.74.0 package --locked --allow-dirty` passed its extracted-package compilation. | 2026-11-10 artifact, before the compiler/edition upgrade; independent tooling excluded. |
| Publication dry-run | `cargo publish --dry-run --locked --allow-dirty` passed packaging and verification, then stopped before upload. | 2026-11-10 artifact: only exact registry zconv 0.2.0 and a v3 two-package lockfile. |

### Rust 1.99.1 / Edition 2024 migration (2026-11-11)

The required compiler is now Rust 1.99.1, which was the newest stable point
release on this date. The root `Cargo.toml` uses Edition 2024, the committed
`rust-toolchain.toml` selects 1.99.1, and the root lockfile uses Cargo format
v4. The isolated fuzz harness also uses Edition 2024. CI checks both the pinned
minimum and the moving stable channel on each supported desktop OS. The
release workflow resolves the registry dependency and regenerates lockfiles
using the declared minimum compiler.

The 2024 match-ergonomics rule required a targeted pattern change in the
document editing path. That change preserves the same mutable borrow and
removes an explicit `ref mut` now rejected by Rust 2024. All six adapters and
the public `v1` contract remain in place; the resolved production dependency
still consists of zcall and zconv only.

The following checks were executed locally against the new Edition 2024
source committed as `8288cb9` (and this documentation update):

| Gate | Result | Scope and qualification |
|---|---|---|
| `cargo +1.99.1 test --all-targets --locked` | 73 integration functions passed. | Linux, pinned registry dependency, Edition 2024. |
| `cargo +1.99.1 test --release --locked` | 73 integration functions and one doctest passed. | Optimized build on Linux. |
| `cargo +1.99.1 test --doc --locked` | One README doctest passed. | The public builder/encoder example compiles and runs. |
| `cargo +1.99.1 clippy --all-targets --locked -- -D warnings` | Passed. | 2024 match bindings and current Clippy diagnostics addressed. |
| `cargo +1.99.1 fmt --all -- --check` | Passed. | All library, test, example and bench source. |
| `python3 scripts/check_package.py` | Passed. | Exactly zcall → zconv; no production `std`, `alloc`, unsafe or build script. |
| `python3 -m unittest discover -s scripts -p 'test_*.py'` | 26 tests passed. | Release, registry and lockfile safeguards. |
| WASM, aarch64 and `thumbv7em-none-eabi` | All three cross-target library checks passed. | Compilation on installed targets; runtime behavior was not exercised there. |
| `cargo +1.99.1 package --locked --allow-dirty` | Passed. | 33 files, package verification compiled after extraction. |
| `cargo +1.99.1 publish --dry-run --locked --allow-dirty` | Passed. | Verified release archive and dependency; no crates.io upload. |
| Registry name availability | `zcall` still unoccupied when checked. | Remote crates.io check is time-dependent and repeated by release workflow. |
| `cargo +1.99.1 bench --bench kernel --locked` | Nine workload measurements completed. | [New CSV](benchmarks/2026-11-11.csv) and [environment](benchmarks/2026-11-11-environment.json); the host CPU differs from the old benchmark. |

The new compiler floor was exercised, rather than only placed in the manifest.
Rust 1.74.0 remains historical compatibility evidence for the *previous*
Edition 2021 crate; it is not an MSRV promise for the new source. The prior
Miri and libFuzzer records also belong to the original source. New remote CI
provides the migration's independent Miri/fuzz/platform gates once its final
GitHub workflow run has completed.

Machine-readable [dependency audit](verification/zconv-0.2.0-audit.json),
[Miri summary](verification/miri-summary.txt) and
[local fuzz summary](verification/fuzz-smoke.txt) accompany this record.

The allocation test counts zero allocator calls inside its observed library
path. Its allocator hook uses `std` and `unsafe` in the integration test binary;
it is outside the production library. The result is a direct check of that
path. The production `#![no_std]`/`#![forbid(unsafe_code)]` attributes, resolved
dependency/source audit, and bare-target compilation provide separate evidence
for the structural constraints; the allocation test alone does not establish
every possible execution path. See [scripts/check_package.py](../scripts/check_package.py).

## 2. Contract-to-test mapping

The following tables name test functions as entry points into the linked source
files. A passing fixture supports the stated behavior within its inputs and
declared context. The full supported subset and excluded protocol features are
listed in [SUPPORT.md](SUPPORT.md).

### Complete requests and responses

All function names in this table refer to
[tests/document.rs](../tests/document.rs).

| Acceptance claim | Executable evidence | Interpretation |
|---|---|---|
| A common text view survives import, modification, and backend conversion. | `all_36_request_directions_preserve_uniform_business_view`; `all_36_response_directions_preserve_ordered_contents` | Each test traverses the 6 × 6 source/target matrix. These are 36 text request directions and 36 text response directions, including identity conversion. |
| The connected tool subset works across the five tool-capable adapters. | `all_25_modern_tool_request_directions_are_connected`; `all_25_modern_tool_response_directions_preserve_action_requirement` | Each traverses a 5 × 5 matrix: 25 request directions and 25 response directions. Definitions, calls/results, and action-required completion are checked. Legacy Completions is excluded. |
| Builders and edits use the actual bounded document storage. | `building_requests_uses_actual_storage_and_can_switch_all_backends`; `building_responses_uses_real_choices_and_caller_facts`; `edits_keep_message_markers_coherent_and_reject_bad_types` | Tests inspect used records and decode encoded output after edits; they exercise real adapters. |
| Missing, null, empty, and zero remain distinct; token counts remain exact. | `missing_null_empty_and_zero_stay_distinct`; `usage_integers_and_cache_arithmetic_are_preserved` | Explicit presence and large integer usage values are checked, including cache normalization. |
| Response, output item, and tool call identities remain separate. | `multiple_candidates_and_separate_identity_namespaces`; `tool_result_linkage_is_checked_before_mutation` | Multiple candidates, unknown call IDs, duplicate results, and association before mutation are covered. |
| Capacity failures preserve edited state and can be retried. | `capacity_failures_are_precise_retryable_and_transactional`; `builder_rejects_invalid_edits_and_rolls_back_capacity_failures`; `replacing_storage_in_place_preserves_edits_after_failure_and_growth` | Tiny output/IR storage, exact versus lower-bound requirements, failed growth, and successful growth are exercised. |
| Provider-owned semantics require an explicit compatible mapping or loss policy. | `provider_owned_extensions_context_and_signatures_do_not_leak`; `new_extensions_preserve_existing_records_and_are_reported_individually`; `structured_output_and_media_keep_source_namespaces`; `opaque_signatures_are_visible_and_cross_provider_transfer_is_strict` | These are targeted subset tests, not a 36-direction promise for every media, schema, or signature form. |
| Edited finish/safety states do not silently become successful ordinary text. | `explicit_finish_edits_change_encoded_status_and_keep_cancellation_distinct`; `refusal_history_cannot_silently_become_ordinary_text`; `changing_incomplete_choice_clears_stale_provider_snapshot` | Checks cancellation distinctions, forbidden refusal conversion, and stale completion metadata after edits. |
| Tool defaults and complete arguments are checked. | `strict_tool_defaults_are_inherited_and_can_be_explicitly_edited`; `complete_tool_arguments_are_validated_even_for_same_format` | Source/authoring strict defaults are preserved, and same-format encoding cannot bypass complete argument syntax validation. |

The matrix fixtures supply required external facts through `EncodeContext` and
tool bindings. Response fixtures explicitly allow selected metadata loss and
check the reported counts. The matrix results therefore do not promise
unconditional, lossless conversion of every vendor field.

### Incremental input, backpressure, and lifecycle

All function names in this table refer to [tests/session.rs](../tests/session.rs).

| Acceptance claim | Executable evidence | Interpretation |
|---|---|---|
| The same event consumer handles all six sources. | `six_real_adapters_feed_identical_harness_code_under_every_byte_split` | Each source is fed with chunk widths 1, 2, 3, 7, 31, and 4096. The consumer pauses before accepting each event, then resumes. Text, start count, terminal count, and usage are checked. |
| Fragmentation can cross UTF-8, JSON escapes, SSE boundaries, and terminal CRLF. | `crlf_and_utf8_escapes_can_split_at_any_byte_including_terminal_crlf` | The Chat fixture is tested at every fixed chunk width from 1 through its byte length. Every byte boundary is exercised as a cut; all possible combinations of cuts are not enumerated. |
| Zero-capacity session regions can grow without replaying committed events. | `zero_capacity_regions_recover_without_replaying_accepted_events` | Starts framing, payload, aggregate, token, event, and item storage at zero. Each requested region is replaced; consumed input, text, and the single terminal are checked. |
| Output backpressure preserves staged bytes and event order. | `uniform_events_encode_to_all_six_targets_with_one_byte_output`; `encoder_zero_buffers_recover_atomically_and_zero_output_backpressures` | Chat-derived uniform events are encoded to all six targets, drained one byte at a time, and decoded again. A separate test checks zero output and zero encoder storage recovery. This is not a full 36-direction stream matrix. |
| Interleaved tools become executable once, and results remain linked. | `interleaved_tools_are_executable_once_and_results_are_linked_by_id`; `tool_event_encoding_preserves_ids_and_complete_arguments`; `aggregation_growth_at_tool_completion_retains_committed_fragments` | Two interleaved calls preserve their argument order and IDs; five target encoders preserve the resulting calls. Unknown/duplicate results and growth at tool completion are covered. |
| Tool readiness requires complete valid arguments. | `malformed_completed_arguments_never_become_ready_but_truncation_is_incomplete`; `active_choice_must_finish_and_tool_arguments_must_be_an_object` | Malformed completed JSON and a completed array cannot produce `ToolReady`; truncated generation remains incomplete. The required root is a JSON object. |
| Cumulative usage and final snapshots are not appended twice. | `cumulative_cache_and_thinking_updates_never_add_prior_totals_twice`; `final_snapshots_are_verified_without_appending_again`; `empty_responses_message_has_a_real_lifecycle_and_no_fabricated_delta` | Cache/thought updates replace cumulative observations, final text is verified, and a genuine empty message has a lifecycle without a fabricated delta. Contradictory snapshots fail. |
| Cancellation, transport errors, upstream failures, and invalid input stay distinct. | `cancellation_intent_confirmation_and_transport_failure_are_distinct`; `eof_duplicate_termination_and_invalid_event_names_have_stable_errors`; `upstream_error_is_a_terminal_event_not_a_json_failure` | Local cancellation intent does not confirm remote cancellation. EOF, duplicate termination, and input after completion have explicit behavior. |
| Caller facts and loss permissions are narrow and observable. | `tool_call_context_is_required_and_never_invented`; `gemini_missing_ids_bind_source_positions_before_normalizing_item_indices`; `late_real_identity_is_observable_and_encoder_rechecks_caller_context`; `one_extension_permission_does_not_authorize_cache_write_loss` | Missing identities require context; later real identities must agree. Permission for one extension does not authorize losing unrelated cache usage. |
| Reasoning summaries do not silently become thoughts. | `reasoning_summary_is_typed_and_never_translated_into_a_thought` | A tested incompatible mapping returns `Unsupported`, even when a broader reasoning-loss option is present. |

The tool argument contract is JSON syntax plus an object root, with the
documented parsing/resource limits. It is not full validation against the tool's
JSON Schema. The caller performs schema validation, authorization, and tool
execution. Likewise, a locally committed `ToolReady` or terminal event does not
provide distributed exactly-once execution across network retries.

### Capability and release decisions

The capability tests use attributed synthetic declarations, not a live model
catalog. They demonstrate that the decision mechanism respects its evidence;
they do not establish the capabilities of a particular deployed model.

| Acceptance claim | Source and representative checks |
|---|---|
| Unknown model/deployment support is never silently promoted to support. | [tests/capability.rs](../tests/capability.rs): `unknown_is_never_implicit_support`, `explicit_uncertainty_is_reported_per_layer`, `accepting_a_condition_does_not_override_unknown_or_unsupported_evidence`. |
| Adapter operation, version/profile, and actual input are checked separately. | `request_and_response_adapter_operations_are_independent`, `protocol_expression_does_not_claim_an_unwired_stream_adapter`, `standalone_adapter_conditions_are_explicit_then_prepare_verifies_the_shape`, `prepare_checks_actual_request_before_send`. |
| Budgets and evidence retain exact meaning. | `explicit_zero_and_large_integer_limits_are_compared_exactly`, `missing_output_count_is_unknown_and_never_assumed_zero`, `context_supplied_output_budget_cannot_bypass_model_limits`, `limits_and_named_levels_require_attributed_evidence`. |
| Diagnostic capacity and loss permissions remain recoverable and specific. | `zero_report_recovers_and_needed_is_exact`, `preparation_reserves_an_input_report_before_encoding`, `a_context_loss_permission_does_not_remove_a_retained_context_requirement`. |
| Publication cannot silently substitute an unresolved Git dependency or omit independent lockfiles. | [scripts/test_release.py](../scripts/test_release.py): `DependencyTests`, `DependencyLockTests`, `PrepareTests`, `GitPreflightTests`, `RegistryTests`, and `PublishedSourceTests`. These check exact registry resolution rules, version input, ownership responses, main/tag consistency, and the workflow's actual commit script in temporary Git repositories. |

## 3. Adversarial input and fuzz smoke

Deterministic malformed-input tests complement the valid protocol fixtures.
`malformed_inputs_are_not_upstream_failures_or_panics` in
[tests/document.rs](../tests/document.rs) checks malformed examples and generated
byte sequences of lengths 0 through 95 across six adapters.
`arbitrary_input_and_duplicate_fields_never_panic_or_spin` in
[tests/session.rs](../tests/session.rs) exercises generated SSE payloads at the
same lengths and rejects duplicate JSON fields. The harness bounds progress
iterations so a failure to advance is observable.

The independent [fuzz project](../fuzz/fuzz_targets/kernel.rs) exercises complete
request and response parsing, all target encoders after successful parses, and
fragmented sessions for each of the six protocols. Its inputs are bounded to
4096 bytes and its workspaces are fixed for the run. It checks consumed-byte
bounds and at-most-one terminal observation; expected parse, unsupported, and
capacity errors are allowed outcomes.

The completed local smoke run reported:

| Measurement | Observed value | Meaning |
|---|---:|---|
| Executions | 5,000 | libFuzzer executions; mutations and repeated inputs are not independent samples. |
| `cov` | 1,844 | libFuzzer's coverage counter for this instrumented build, not a percentage or a Rust line count. |
| Peak RSS | 55 MiB | Whole fuzz process, including runtime and sanitizer overhead; not per-call zcall memory. |
| AddressSanitizer | Enabled | Memory instrumentation used for the smoke run. |
| LeakSanitizer | Disabled locally | The platform's ptrace restriction prevented LSan operation; this run supplies no leak-detection result. |

The run used `-runs=5000 -max_total_time=30 -rss_limit_mb=1536
-print_final_stats=1` and reached the execution limit. This is a smoke result,
not an exhaustive search, a proof of panic freedom for arbitrary bytes, or a
complete protocol-conformance campaign. External fuzz dependencies remain in
their own workspace and are excluded from the formal dependency graph and
published package.

## 4. CI gates and reproduction

The [CI workflow](../.github/workflows/ci.yml) defines the following ongoing
gates. A configured job becomes executed evidence only when its result is
recorded against the relevant commit.

| Gate | Configuration |
|---|---|
| Platforms and compilers | Linux, macOS, and Windows; latest stable and pinned Rust 1.99.1; debug/all-targets, release, and doctest runs. |
| Quality and dependency boundaries | Formatting, Clippy with warnings denied, resolved dependency/source audit, all Python release tests, and the quick kernel benchmark. |
| Cross compilation | Library checks for WASM, aarch64 Linux, and `thumbv7em-none-eabi`. |
| Miri | Capability, document, and session integration suites, with a 30-minute job limit. |
| Fuzz | Independent cargo-fuzz project, up to 10,000 executions or 30 seconds, with a memory limit. Its configured budget differs from the completed local 5,000-execution run. |
| Release | Exact registry dependency resolution, MSRV/stable validation, verified Cargo packaging, name/owner/version checks, source/tag checks, and publication credentials confined to the publish step. |

[CI run 42841282580](https://github.com/LIghtJUNction/zcall/actions/runs/42841282580)
for commit `c8fa349` completed successfully: all six desktop test jobs, all three
cross-check jobs, quality, fuzz and Miri passed. This first complete remote run
used the development dependency. The separate completed local registry results
are listed in section 1. Later commits receive their own runs on the
[repository Actions page](https://github.com/LIghtJUNction/zcall/actions/workflows/ci.yml);
a run's commit and conclusion are the authoritative remote result for that revision.

The core reproduction commands, from the repository root with the stated
toolchains installed, are:

```sh
cargo +1.99.1 test --all-targets --locked
cargo +1.99.1 test --release --locked
cargo +1.99.1 test --doc --locked
cargo +stable test --all-targets --locked
cargo +stable test --release --locked
cargo +stable test --doc --locked
python3 scripts/check_package.py
python3 -m unittest discover -s scripts -p 'test_*.py'
cargo +nightly miri test --test document --test session --test capability
```

Record `rustc -Vv`, the source commit, the dependency source/checksum, and the
target when repeating a run. In this record, `stable` resolved to Rust 1.99.1;
the name may resolve to a different compiler later. Cross-check commands are
listed exactly in the CI workflow. Publication requires the separate
[release procedure](RELEASING.md); passing these commands does not publish a
crate or establish that publication occurred.

## 5. Outside the acceptance claim

- No provider network request or live model inference is part of these tests.
  Authentication, rate limits, endpoint availability, real-model behavior, and
  model capability discovery remain unmeasured here.
- Fixture conversion does not establish support for every vendor field,
  undocumented profile, media stream, or future protocol version. Unknown
  protocols require an implemented adapter and new conformance evidence.
- Tool arguments receive JSON/object validation, not full JSON Schema
  validation, tool execution, or authorization.
- A terminal-once local state machine does not promise distributed exactly-once
  transport or tool execution. Retries, transcript persistence, clocks, and
  scheduling are caller responsibilities.
- Cross compilation does not establish runtime behavior on every target.
  Miri, sanitizers, generated inputs, and finite tests do not prove all inputs
  safe or all semantic mappings correct.
- This document makes no comparative throughput or latency claim and no stable
  Rust layout or binary ABI claim. Resource costs depend on caller capacities;
  see [RESOURCES.md](RESOURCES.md).

## 6. Five-dimension self-review

| Dimension | Question and assessment |
|---|---|
| Contribution | Does the evidence support a usable uniform kernel? **Supported within the tested subset:** import/build/edit/encode paths and resumable sessions are connected to real adapters. No scientific novelty or universal protocol claim is made. |
| Writing clarity | Can a reader distinguish functions, directions, iterations, and completed runs? **Checked:** the inventory, matrix interpretation, and run statuses define them separately and link their sources. |
| Experimental strength | Do the results justify performance or universal robustness claims? **Limited:** targeted regressions, an allocator observation, and a fuzz smoke justify their stated observations. They do not establish superiority over another implementation. |
| Evaluation completeness | Are executed and absent results visible? **Checked within the listed scope:** the first remote CI, local Miri and the stated registry revalidation passed. Later revisions have distinct CI runs. Live-provider and exhaustive conformance testing are absent. |
| Method design soundness | Are hidden assumptions exposed? **Checked within scope:** caller facts, bounded storage, explicit loss/uncertainty, JSON-object tool validation, and local lifecycle limits are stated alongside their tests. |

