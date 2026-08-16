# rusty_zstd

[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/remade-with-rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#licence)

Pure-Rust [Zstandard](https://facebook.github.io/zstd/) (RFC 8878) — compress + decompress, CLI, optional C ABI. No C, no FFI on the core path.

**Status: M6.** Compress (levels −7…22; all 9 strategies) and decompress are RFC 8878 interop both ways against facebook/zstd v1.5.7. Dictionaries, `--long` / LDM, seekable frames, multi-thread (`-T#` / `--jobsize` / overlap), and CLI aliases (`unzstd` / `zstdcat` / `zstdmt`) plus `-l` / `-b` / `-r` / env vars are dual-gated against C. Plan: [`docs/plans/rusty-zstd-mission.md`](docs/plans/rusty-zstd-mission.md).

No speed claim yet. M2 ratio vs C at −7…3 is ledgered (`kind=m2_ratio` in [`bench/ledger.jsonl`](bench/ledger.jsonl)).

## Workspace

| Crate | Role |
|---|---|
| `rusty_zstd` | Library (compress + decompress, streaming, dicts, trainer) |
| `rusty_zstd-cli` | `rzstd` / `unzstd` / `zstdcat` / `zstdmt` (`-z` / `-d` / `-t` / `-l` / `-b` / `-T#` / `--train` / `--long` / `--seekable`) |
| `rusty_zstd-bench` | Shells out to pinned C `zstd`; never links libzstd |
| `rzstd-alloc` | `rusty_alloc-api =0.4.0` seam for binaries only |

## Pinned C oracle

facebook/zstd **v1.5.7**. Fetch (Windows):

```powershell
pwsh scripts/fetch-oracle.ps1
```

Or set `RUSTY_ZSTD_ORACLE` to a `zstd` binary whose `--version` contains `1.5.7`. Details: [`third_party/zstd/README.md`](third_party/zstd/README.md).

## Baselines

```powershell
cargo run -p rusty_zstd-bench --release -- --baseline-only
```

```powershell
cargo run -p rusty_zstd-bench --release -- --m7-speed
cargo run -p rusty_zstd-bench --release --features profile -- --m7-profile
```

M7 speed is **only** facebook/zstd v1.5.7 `zstd -1`, `zstd --fast=1`, and `zstd --fast=4` (ABBA, pinned, in-process us vs C `-b -T1`). Not a §7 exit claim until the ledger says so. `--m7-speed --smoke` is one loop on 1 MiB zeros.

Ratio lines are `kind=m2_ratio` (not a speed claim). M3 adds Huffman literals and FSE sequence modes; standing dual-gate is still generated train/holdout.

Corpora: deterministic generated files (train/holdout) plus optional Silesia — [`corpora/README.md`](corpora/README.md).

## Licence

MIT OR Apache-2.0. See `LICENSE-MIT` and `LICENSE-APACHE`.
