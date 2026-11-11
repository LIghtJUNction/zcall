# Performance and memory measurements

## Endpoint and discovery run (2026-11-11)

The endpoint/discovery source at commit
[`413ca14`](https://github.com/LIghtJUNction/zcall/commit/413ca14ecfc27a7e060f7c606cf849fdf04ccb81)
was measured with Rust 1.99.1, zconv 0.2.2 and an AMD EPYC 9V74 host exposing
nine logical CPUs. Each workload warms up for 200 iterations and reports the
median of seven samples of 20,000 iterations. The optimized profile uses thin
LTO and one codegen unit. Network access and model execution are outside these
measurements; host contention and CPU frequency are uncontrolled.

| Workload | Median µs | Sample mean range µs |
| --- | ---: | ---: |
| Validate a base URL and build a Gemini streaming URL | 0.480 | 0.452–0.505 |
| Initialize discovery and build the first probe URL/body | 1.952 | 1.927–2.015 |
| Initialize, build a probe, parse and confirm a 229-byte Chat response | 6.833 | 5.705–10.855 |

The last row includes the whole local discovery setup and confirmation path.
It does not measure a response parser alone or end-to-end network latency.
The allocation-counting integration test observes zero heap allocations for
all six URL/probe paths and successful response confirmation. `HttpEndpoint`
occupies 56 bytes and `Discovery` 480 bytes on this target, excluding borrowed
strings, caller buffers, token storage and temporary stack frames.

The [CSV](benchmarks/2026-11-11-endpoints.csv) includes the new workloads and
the existing kernel measurements from the same run. The
[environment record](benchmarks/2026-11-11-endpoints-environment.json) identifies
the source, compiler and method. These absolute measurements make no speedup
claim against another library or a different host.

## Rust 1.99.1 / Edition 2024 run (2026-11-11)

The Edition 2024 migration was benchmarked on production-source commit
[`8288cb9`](https://github.com/LIghtJUNction/zcall/commit/8288cb9b80e59ca112d717ba68fa9ec70382c6a4),
using Rust 1.99.1 and the published zconv 0.2.0 dependency. This run used an
**AMD EPYC 9V74** host with nine logical CPUs exposed. The older run below used
an **Intel Xeon Platinum 8573C** host. Consequently, the differences between
dates cannot be attributed to a Rust edition change or a code optimization.

Both paired request workloads inspect their actual parsed IR before encoding.
The zconv baseline checks the same message count and produces byte-identical
output. Each row reports the median of seven per-sample means, with 20,000
iterations per document sample and 5,001 per stream sample, after warmup.
The input throughput figures include parsing, encoding, and the stated harness
work; they are not model inference throughput.

| Workload | Input bytes | Median µs | Sample mean range µs | Input MiB/s |
|---|---:|---:|---:|---:|
| zcall: small request, inspect IR, encode | 111 | 3.07 | 2.46–3.93 | 34.47 |
| zconv: same small IR conversion | 111 | 2.90 | 2.42–4.79 | 36.48 |
| zcall: Unicode request, inspect IR, encode | 1,783 | 3.87 | 3.74–5.16 | 439.79 |
| zconv: same Unicode IR conversion | 1,783 | 3.88 | 3.79–3.97 | 438.60 |
| zcall: tool request, object validation, encode | 513 | 4.85 | 4.70–5.26 | 100.81 |
| zcall: complete response decode and content view | 229 | 2.17 | 2.08–2.24 | 100.61 |
| zcall: stream supplied in one 309-byte fragment | 309 | 5.35 | 5.16–6.37 | 55.03 |
| zcall: same stream in 16-byte fragments | 309 | 5.45 | 5.29–5.65 | 54.10 |
| zcall: same stream in one-byte fragments | 309 | 8.04 | 7.79–11.25 | 36.66 |

The small-request median is approximately 5.8% higher than its direct zconv
baseline in this run; the sample ranges overlap substantially. The Unicode
medians are within 0.3% of each other. Neither observation establishes a
statistically reliable relative speed difference. In particular, zcall does
not omit an IR inspection for the comparison.

The [2026-11-11 CSV](benchmarks/2026-11-11.csv) records the full precision of
all nine measurements. Its [environment record](benchmarks/2026-11-11-environment.json)
captures the compiler, edition, commit, CPU, flags, dependency checksum, source
hashes and caller-owned type sizes. The measured memory sizes match the older
table below on this x86_64 target; sizes remain architecture and compiler
dependent.

## Historical Rust 2021 baseline (2026-11-10)

This is the **2026-11-10 baseline**, measured from source commit `be9c04f`
with Rust 1.99.1 and Edition 2021. On 2026-11-11, the crate adopted Edition
2024 with Rust 1.99.1 as its declared minimum; that migration must be evaluated
on its own commit. The archived numbers below retain their original source
hashes and are not silently relabeled as post-migration results.

The recorded workloads complete in single-digit microseconds for small document
operations and in 7.92–12.26 microseconds for the same 309-byte streamed model
turn, depending on fragmentation. These are local kernel measurements, not
network latency, model token throughput or a claim about all protocol payloads.

The [CSV](benchmarks/2026-11-10.csv) contains the full run. The
[environment record](benchmarks/2026-11-10-environment.json) includes compiler,
CPU, flags, dependency checksum, source hashes and measured type sizes. The
implementation and workload are in [benches/kernel.rs](../benches/kernel.rs).

### Method

The run used stable Rust 1.99.1 on x86_64 Linux, with the published zconv 0.2.0
registry dependency. The managed environment exposed an Intel Xeon Platinum
8573C and nine logical CPUs. CPU frequency, host contention and scheduling were
not controlled; no CPU affinity or `target-cpu=native` setting was used. Task-local
Miri, fuzz and other compilation work had finished before the timed run.

`cargo bench` uses optimization level 3, thin LTO and one codegen unit. Each
workload warms up for 200 iterations, then runs seven samples of 20,000
iterations; streaming samples use 5,001 iterations. `Instant` measures elapsed
time, divided by iterations. Inputs and outputs pass through `black_box`.
The median, minimum and maximum are per-sample means. The min/max range is not a
confidence interval. `--quick` and debug-mode runs are smoke checks and are not
the measurements reported here.

The paired text-request baseline parses into zconv IR, inspects the message
count and encodes Chat to Responses. Both implementations use the same input,
target, token/record capacities and semantically equivalent work. Output byte
equality is asserted before timing. zcall does not bypass IR inspection for this
comparison. Tool validation, complete responses and stateful streams are reported
as absolute costs because a codec-only baseline would omit their contract work.

### Results

| Workload | Input bytes | Median µs | Sample mean range µs | Input MiB/s |
|---|---:|---:|---:|---:|
| zcall: small request, inspect IR, encode | 111 | 4.44 | 4.04–4.81 | 23.84 |
| zconv: same small IR conversion | 111 | 4.31 | 3.86–4.58 | 24.53 |
| zcall: Unicode request, inspect IR, encode | 1,783 | 6.09 | 5.87–6.21 | 279.08 |
| zconv: same Unicode IR conversion | 1,783 | 6.16 | 5.70–6.43 | 276.17 |
| zcall: tool request, object validation, encode | 513 | 7.85 | 7.71–8.42 | 62.34 |
| zcall: complete response decode and content view | 229 | 3.76 | 3.59–3.81 | 58.14 |
| zcall: stream supplied in one 309-byte fragment | 309 | 7.92 | 7.73–9.35 | 37.21 |
| zcall: same stream in 16-byte fragments | 309 | 8.97 | 8.18–9.26 | 32.87 |
| zcall: same stream in one-byte fragments | 309 | 12.26 | 11.61–12.86 | 24.04 |

The small paired workload's medians differ by about 2.9%; the Unicode paired
workload's medians differ by about 1.0% in the opposite direction. The overlapping
sample ranges do not establish a performance advantage for either library.
They show the cost of the stable wrapper at these tested sizes, under this
environment's noise. A broader claim needs larger corpora and controlled runs.

Fragmentation increases the number of calls, framing checks and resumptions.
The one-byte case is a useful recovery stress case; the same text and one
terminal event are checked in every case. Each streaming iteration includes
session construction, preparation, parsing, lifecycle handling and callback
submission. Buffers are reused between iterations without hidden allocation.

## Explicit memory ownership

These are `size_of` / `memory_bytes()` measurements on the recorded target.
They are not a layout ABI and may differ on another compiler or architecture.

| Caller-owned type or header | Bytes |
|---|---:|
| `TokenStorage<128>` | 2,560 |
| `RecordStorage<32>` | 6,912 |
| `Request` header, excluding records and input | 664 |
| `Response` header, excluding records and input | 448 |
| `StreamWorkspace<64, 8, 4>`, including rollback checkpoints | 4,352 |
| `Session` state, excluding caller buffers/workspace | 640 |
| `EncoderWorkspace<64, 4>`, including rollback checkpoints | 1,984 |
| `EventEncoder` state, excluding caller buffers/workspace | 1,200 |

The stream benchmark supplies three 256-byte buffers for framing, payload and
aggregation. Its persistent session state plus workspace and these buffers total
**5,760 bytes**. This excludes the input slice, consumer-owned storage and
temporary call-stack frames; it is not a measured peak stack usage claim.
The library does not impose these example capacities. Zero-capacity tests grow
each required region explicitly and resume without replaying committed work.

Requests write into the actual zconv record storage. Stable views add no second
message tree. Session events borrow their replay record during the callback
instead of copying the complete raw event on every submission. Text that does
not need a final exact snapshot remains incremental. Tool arguments and exact
Responses snapshots need retained bytes; their growth is separately visible in
`Session::memory().aggregate_used` and `aggregate_peak`. The
[resource contract](RESOURCES.md) specifies each retry and growth rule.

Both production crates are `no_std`, forbid unsafe code, have no `alloc` access
and have no other dependencies. The resolved-graph/source audit and successful
bare-metal build check that property. A counting-allocator integration test also
observes zero allocations during request decode/edit/encode and session input.
The test harness and benchmark may allocate their fixtures before kernel work;
those allocations are not library behavior.

## Reproduce

```sh
cargo bench --bench kernel --locked
cargo test --test allocation --locked
python3 scripts/check_package.py
```

Run on an otherwise idle machine when comparing changes. Keep the workload,
compiler, flags, dependency version, fragmentation and semantic checks identical.
The current measurements do not cover long histories, huge tool schemas,
many simultaneously active items, all media forms, cancellation-heavy workloads
or exhaustive capacity-growth patterns. Their correctness tests are separate
from these timing measurements. No vendor HTTP endpoint was called.
