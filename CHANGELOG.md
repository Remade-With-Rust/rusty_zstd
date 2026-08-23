# Changelog

All notable changes to this project are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/); this project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.1.0] - 2026-08-23

The first public release. `rusty_zstd` has been developed against facebook/zstd
v1.5.7 as a pinned external oracle since M1; this is the point at which the
product surface (M1–M6) is complete and the performance campaign (M7) has taken
compression **ratio** to parity. It is published now because everything a caller
depends on — the format, the API and the interop — is settled, and the remaining
work is speed, which changes no output byte.

### The state of the thing

- **Format: all of RFC 8878.** Raw / RLE / Compressed blocks, Huffman literals
  (1-stream, 4-stream, treeless), FSE sequences in all four modes, repeat
  offsets, skippable frames, multi-frame concatenation, content size, dictionary
  IDs, XXH64 content checksum.
- **Compress and decompress**, levels **−7…22**, with all **nine** libzstd
  strategies implemented — fast, dfast, greedy, lazy, lazy2, btlazy2, btopt,
  btultra, btultra2.
- **The libzstd job list:** streaming, dictionaries with `fastcover` / `COVER` /
  legacy trainers, `--patch-from` prefixes, long-distance matching, `--rsyncable`,
  the seekable frame format, multi-threading with job size and overlap, frame
  inspection.
- **A `zstd`-shaped CLI** — `rzstd` plus the `unzstd` / `zstdcat` / `zstdmt`
  aliases, `-l` / `-b` / `-r`, and the `ZSTD_CLEVEL` / `ZSTD_NBTHREADS` env vars.
- **Zero dependencies** in the published library. Builds on `no_std + alloc` and
  `wasm32-unknown-unknown`.

### Measured

Level 1, 18 corpora, against facebook/zstd v1.5.7 as a pinned external binary,
best-of-N on both arms with a **6.83%** session null arm
([`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md), 2026-08-22 board):

| | vs facebook/zstd v1.5.7 |
| --- | --- |
| ratio, mean `us/c size` at L1 | **0.975** — we emit fewer bytes than C |
| ratio, mean `us/c size` at L3 | **1.012**, worst cell `nci` 1.100 |
| compress speed, mean | 1.83× behind C (we lead on 2 of 18) |
| decompress speed, mean | 1.49× behind C (we lead on 4 of 18) |

The L3 ratio column is identical cell-for-cell across four consecutive boards,
which doubles as an end-to-end identity check. **No speed claim is made for the
optimization campaign**: every brick in it shipped on strictly-less-work plus
byte-identity, never on a wall-clock delta.

### Added — release engineering

- `crates/rusty_zstd/README.md`, wired in as the crate's module documentation via
  `#![doc = include_str!]`, so all three of its examples are compiled and run by
  `cargo test --doc` and cannot go stale.
- `CHANGELOG.md`, this file.
- Crate metadata for crates.io and docs.rs: `documentation`, expanded
  `description`, `no-std` in keywords and categories.

### Changed — the public API is now the codec, not the campaign

Roughly 200 A/B arms and counters (`set_*_arm`, `take_*`, the `prof_*` re-exports,
`BT_SPEC_PAIRS`, `Xxh64Pub`, …) were re-exported at the crate root and would have
rendered on docs.rs beside `compress` and `decompress`. They are now
`#[doc(hidden)]` and documented as carrying **no semver promise**. Nothing moved
and nothing was removed, so the benchmark harness is unchanged; the rendered
public surface is the codec.

A `--features bench-arms` gate was tried first and rejected: it made the arms
unreachable in the default build, which stranded ~228 items as dead code and
would have required blanket-allowing `dead_code` in the shipping configuration.

### Changed — the CLI is four shims over one entry point

`rusty_zstd-cli` declared four `[[bin]]` targets sharing a single `src/main.rs`,
which Cargo warns about on every invocation and has deprecated. It is now a
library exposing `entry()` plus four `src/bin/*.rs` shims. The argv[0] dispatch
that gives `unzstd` / `zstdcat` / `zstdmt` their behaviour is unchanged, and each
shim installs the `#[global_allocator]` itself — house law is that the allocator
lives in the deliverable, never in a library.

### Changed — `rusty_alloc-api` 0.x → `=1.1.0`

The process-wide allocator behind the `rzstd-alloc` seam. It reaches the two
binaries only; the published library still has an empty dependency tree.

### Fixed — `no_std + alloc` did not build

`ctable_from_nbits` called `huff_pool::take_w()` unconditionally, but the pool is
`thread_local!` and therefore `std`-only. Every other pool site in `huffman.rs`
already had the `cfg` fallback; this one did not, so the configuration the README
advertises — and that CI checks for `wasm32` — failed to compile. It now
allocates fresh under `no_std`, as its siblings do.

### Fixed — a shadowed `#[inline(never)]` meant W18 never took effect

`seq_table` in `compressed.rs` carried a documented `#[inline(never)]` (the W18
brick: outline the per-block FSE table build so the sequence loop keeps its
register budget). It was written *below* an `#[inline(always)]` that already sat
on the item, so rustc took the first and discarded the second — silently, until
`unused_attributes` was promoted to a warning. The dead attribute was removed
rather than the live one, so the shipped binary is the one that was measured, and
a note now records that the W18 A/B must be re-run before `seq_table` is promoted
to `inline(never)`. Two more duplicated-attribute pairs (`x2_from_x1_into`,
`FseTable::from_norm_into`) were resolved the same way, both no-ops on codegen.

### Fixed — six duplicated `if COUNT` guards, and dead work in the DP loop

`find_fast`'s instrumented arms contained `if COUNT { if COUNT { … } }` at six
sites. The optimal-parse DP also carried three `o_skip_*` counters that only the
`profile` census reads, and their increments ran in every build; they are now
`cfg`-gated with their reader. An `active: Vec<usize>` work list in the Huffman
merge — one allocation per block — had gone unread since the two-queue rewrite
landed and is removed.

### Changed — the lint gates are real now

The workspace builds and lints clean: zero rustc warnings in the default,
`profile`, and `no_std + alloc` configurations, and
`cargo clippy --all-targets -- -D warnings` passes on all three shipping crates.
`too_many_arguments` and `type_complexity` are allowed at the workspace level with
a stated reason — an entropy or match-find kernel takes its state by argument
precisely so it does *not* reach through a struct in the hot loop.

CI now scopes its strict gates to the shipping crates (`rusty_zstd`,
`rusty_zstd-cli`, `rzstd-alloc`) and checks the campaign harness separately: the
harness carries 455 one-off measurement instruments that need a pinned C binary
and multi-gigabyte corpora CI does not have, and holding them to `-D warnings`
gates nothing a consumer can see.
