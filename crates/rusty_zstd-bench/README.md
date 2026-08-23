# rusty_zstd-bench

The campaign harness. **Not published**, and not a product surface — it exists to
produce admissible numbers, and every public claim in the repository is a row it
wrote to [`bench/ledger.jsonl`](../../bench/ledger.jsonl).

It shells out to a **pinned facebook/zstd v1.5.7 binary** and never links
libzstd, so the C implementation cannot reach the dependency graph of anything we
ship.

## Build it with `--features profile`

```sh
cargo check -p rusty_zstd-bench --all-targets --features profile
```

Over half of the 455 instruments in [`examples/`](examples/) read counters that
only exist behind `rusty_zstd/profile`, so **that is the configuration the
examples are written for** and the one CI checks. Building the harness's examples
without it will fail on those files — that is expected, not a break.

Two exceptions, both deliberate:

- **`--features dupladder`** for `dsloop`, declared as its `required-features`.
  The ladder adds a per-sequence arm dispatch to the very loop it measures, so it
  is opt-in rather than part of `profile`.
- **No features at all** for a speed run. The profiler has a real timing tax; a
  `--m7-speed` board taken with it compiled in is not admissible. This is why
  `profile` is *not* a default feature here, and why it is worth the friction of
  the paragraph above.

## The two things it does

```sh
# Ratio and correctness against the pinned oracle -- no timing involved.
cargo run -p rusty_zstd-bench --release -- --baseline-only

# The speed board. Profiler OFF, best-of-N both arms, null arm reported.
cargo run -p rusty_zstd-bench --release -- --m7-speed

# Stage shares. Separate build; comparable only as shares within one run.
cargo run -p rusty_zstd-bench --release --features profile -- --m7-profile
```

`--m7-speed --smoke` is one loop on 1 MiB of zeros, for checking the harness
itself rather than the codec.

## Getting the oracle

`pwsh ../../scripts/fetch-oracle.ps1`, or set `RUSTY_ZSTD_ORACLE` to any `zstd`
binary whose `--version` contains 1.5.7. See
[`third_party/zstd/README.md`](../../third_party/zstd/README.md).

## Reading a board

`C/us > 1` means C is faster. `us/c size > 1` means we emit more bytes. The
**null arm** — the worst same-arm spread in the session — is printed with every
board and is the noise floor: a cell inside it is not a result. **Never average
the corpora**; the per-file spread is the story. The method, and the history of
this instrument's own repair, are in
[`docs/plans/m7-benchmark-repair.md`](../../docs/plans/m7-benchmark-repair.md)
and [`docs/plans/m7-anatomy.md`](../../docs/plans/m7-anatomy.md).
