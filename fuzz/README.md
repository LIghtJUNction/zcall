# Independent fuzz harness

This project is excluded from the zcall workspace and published crate. Its
libFuzzer dependencies never enter the formal library dependency graph.

The target feeds arbitrary bytes to all six request/response decoders, attempts
cross-format encoding of valid documents, and exercises fragmented session input
and explicit EOF. Assertions bound consumed input and terminal submissions. The
input cap is 4,096 bytes and every kernel region has an explicit caller capacity.
Capacity failures are valid outcomes, not requests for hidden allocation.

```sh
cargo install cargo-fuzz --locked
mkdir -p fuzz/corpus/kernel
cp fuzz/seeds/* fuzz/corpus/kernel/
CARGO_BUILD_JOBS=1 CARGO_PROFILE_RELEASE_OPT_LEVEL=1 \
  cargo +nightly fuzz run kernel fuzz/corpus/kernel -- \
  -max_total_time=60 -rss_limit_mb=1536 -print_final_stats=1
```

Use `fuzz/corpus/kernel` for mutations; the three files in `fuzz/seeds` are the
reviewable starting corpus. Generated corpus files and crash artifacts are
ignored. On a sandbox that prohibits LeakSanitizer's process tracing, set
`ASAN_OPTIONS=detect_odr_violation=0:detect_leaks=0`; AddressSanitizer remains
enabled, but that run makes no LeakSanitizer claim. See the validation report for
the exact local run and its limitations.
