### In The Wild with 1,842 Active Installs
> [MATA Network](https://mata.network) uses `rusty_zstd` as their unified compression tool
> deploying across Windows, PC, MAC, Linux, Android, iPhone, WASM, and ESP-32 for Home devices.
> This diverse ecosystem is battle testing our code base in every environment for performance,
> and security.
>
> [SpaceDB](https://mata.network/remade-with-rust) uses `rusty_zstd` as their core compression
> for distributed cloud applications, across many device types.
> 
> [RAG Converter](https://ragconverter.com) uses `rusty_zstd` for compression.
> It makes personal and work files AI-readable without them leaving the machine:
> the whole conversion runs as WebAssembly in the browser tab, with nothing
> uploaded and nothing to install.
> 

# rusty_zstd

[![crates.io](https://img.shields.io/crates/v/rusty_zstd?logo=rust)](https://crates.io/crates/rusty_zstd)
[![docs.rs](https://img.shields.io/docsrs/rusty_zstd?logo=docsdotrs)](https://docs.rs/rusty_zstd)
[![CI](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml/badge.svg)](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/Remade-With-Rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)

> **rusty_zstd** is a ground-up, pure-**Rust** [Zstandard](https://facebook.github.io/zstd/)
> ([RFC 8878](https://datatracker.ietf.org/doc/html/rfc8878)) **compressor and
> decompressor**: `#![deny(unsafe_code)]` at the root, with `unsafe` opted into
> item by item and every site counted per module, **zero dependencies**, no C, no `*-sys` crate, no FFI. Every frame we emit
> decompresses in facebook/zstd v1.5.7, and every frame it emits decompresses in
> us — dual-gated on the Silesia corpus plus generated train/holdout sets, every
> commit. Dual MIT / Apache-2.0, so there is no BSD-or-GPLv2 choice to make and
> nothing to vendor.

Part of **[Remade With Rust](https://github.com/Remade-With-Rust)** by
**[Mata Network](https://www.mata.network/)** — the compression layer under
**[SpaceDB](https://github.com/Remade-With-Rust/spacedb)**,
**[remade_ffmpeg_rs](https://github.com/Remade-With-Rust/remade_ffmpeg_rs)** and
the rest of the stack.
[Jump to the ecosystem ↓](#the-remade-with-rust-ecosystem)

---

## The headline

A pure-Rust Zstandard codec that is **interoperable in both directions with the C
reference**, ships the whole product surface rather than a decoder, and lands
**within ~2-4% of C on compressed size across the level range**:

- **Format: all of RFC 8878.** Raw / RLE / Compressed blocks, Huffman literals
  (1-stream and 4-stream, treeless), FSE sequences in all four modes, repeat
  offsets, skippable frames, multi-frame concatenation, content size, dictionary
  IDs, and the XXH64 content checksum.
- **Compress *and* decompress.** Levels **−7…22** with all **nine** libzstd
  strategies — fast, dfast, greedy, lazy, lazy2, btlazy2, btopt, btultra,
  btultra2 — not a fast-path stand-in for the high levels.
- **The jobs libzstd does.** Streaming, dictionaries (with `fastcover` / `COVER`
  / legacy **trainers**), `--patch-from` prefixes, long-distance matching, the
  seekable format, multi-threading with job size and overlap, frame inspection,
  and a `zstd`-shaped CLI with the real aliases.
- **`#![deny(unsafe_code)]`, opted out item by item.** `unsafe` is used for the
  runtime-dispatched AVX2/NEON kernels in `simd.rs` and, more widely, in the hot
  decode and match-finding loops (bounds-check elision, unaligned loads, buffer
  growth): 212 blocks in 16 modules, listed in [Where `unsafe` is
  used](#where-unsafe-is-used) and kept current by CI. Every SIMD kernel keeps
  its scalar twin as the oracle and the fallback, gated equal in the test suite.
- **Zero dependencies.** Not "few" — the published library's dependency tree is
  empty. It builds on `wasm32-unknown-unknown` and on `no_std + alloc`.

| | facebook/zstd (C) | **rusty_zstd (Rust)** |
|---|---|---|
| C/C++ in the dependency tree | all of it | **none** — the library has zero dependencies |
| `unsafe` in the codec | pervasive | **opted into per item**, every site counted per module in CI; SIMD kernels keep scalar twins as oracles |
| License | BSD-2 **or** GPLv2 | **MIT OR Apache-2.0** |
| Interop with the other implementation | — | **both directions**, dual-gated per commit |
| `no_std` / `wasm32` | needs a porting layer | **builds as-is** |
| Levels / strategies | −7…22, 9 strategies | **−7…22, all 9** |

### Performance — measured and honestly reported

**0.4.0: small frames decode 4–6× faster.** Databases that compress each row
on its own — SpaceDB stores every row above 96 bytes as its own level-3 frame —
were paying for tables the decoder built and never read. Decode time per row,
SpaceDB-shaped JSON rows (DID documents, audit rows, CRM leads, form
submissions, device records), every arm decoding the identical frames to
identical bytes:

| row | **rusty_zstd 0.4.0** | 0.3.1 | libzstd `ZSTD_decompress` | libzstd, reused `DCtx` |
|---:|---:|---:|---:|---:|
| 96 B | **1.00 µs** | 6.56 µs | 1.69 µs | 0.83 µs |
| 128 B | **1.32 µs** | 7.14 µs | 1.80 µs | 0.92 µs |
| 256 B | **1.61 µs** | 7.66 µs | 1.95 µs | 1.07 µs |
| 512 B | **2.02 µs** | 8.17 µs | 2.14 µs | 1.22 µs |
| 1 KiB | **2.91 µs** | 8.91 µs | 2.42 µs | 1.48 µs |
| 4 KiB | **7.08 µs** | 12.41 µs | 4.50 µs | 3.48 µs |
| 16 KiB | **17.4 µs** | 20.8 µs | 10.6 µs | 9.5 µs |

So for rows up to 512 B, `rusty_zstd::decompress` now beats libzstd's own
one-shot API; libzstd with a long-lived, reused decompression context is still
1.2–2× faster, and from 1 KiB up it leads either way. Large files are
unchanged by this release (instructions −3.5%, wall clock within noise).

<sub>Measured for **0.4.0** on 2026-10-14. One program per arm, pinned to one
core at high priority, global minimum over 10 rotated rounds; a null arm (0.4.0
against a byte-copy of itself) read 0.96–1.02. Our arms link **rusty_alloc
2.2.6** as the global allocator, as the MATA deliverables do. libzstd 1.5.7 is
built from the release source with clang 22 `-O3` (assembly off), writing into
a caller-provided buffer. In instructions retired (callgrind) a 256-byte row
went from 150,746 to 23,984 (−84%). The bricks, the refuted alternatives and the
method: [`docs/plans/small-frame-decode.md`](docs/plans/small-frame-decode.md).</sub>

**Both CLIs at their real defaults.** `zstd -<lvl> <files>` against
`rzstd -<lvl> <files>`, no flags beyond the level (both default to one thread
and a content checksum), one invocation per run over the 19-file corpus capped
at 8 MiB per file — 143,865,706 bytes. Each arm has its own staged copy;
encode and decode are each timed 10 times per level with the arms alternating,
every process pinned to one core at high priority, and each CLI decodes the
other's output against the original as the work-parity check (0 mismatches at
every level). Ratios are C's time over ours: **above 1× we are faster.**

| level | encode vs C | decode vs C | size vs C |
|---|---:|---:|---:|
| **L1** | **1.34×** | **2.14×** | +2.00% |
| **L3** (default) | **1.08×** | **1.81×** | +1.76% |
| **L9** | **1.09×** | **2.04×** | **+0.29%** |
| **L19** | **1.30×** | **2.48×** | +3.82% |

<sub>Measured for **0.4.0** on 2026-10-14 against the official zstd 1.5.7
Windows binary with [`bench/cli-board.ps1`](bench/cli-board.ps1). Each speed
cell is C's fastest run over ours — the floor is what survives a host that is
not quiescent (this one had another workload running and real-time antivirus
scanning every written file). The median of the 10 paired ratios agrees within
0.13 everywhere (encode 1.35 / 1.08 / 1.09 / 1.36, decode 2.19 / 1.84 / 2.09 /
2.35); the full min–max is in the script's output. **Size** is the full,
uncapped 19-file corpus (355,593,492 bytes), both CLIs at their defaults: at L1
we emit 116,548,162 bytes against C's 114,259,341. Our encoder's output is
byte-identical to 0.3.0's, re-verified for this release, so the size column is
unchanged.</sub>

**The CLI table is whole-program — read, codec and write — and that flatters
us.** On this host C's CLI is slow at writing files, and the decode column in
particular is mostly that. The codec alone, both sides in-process with no
checksum and no I/O (our `rzstd-bench` encode/decode loop against the official
binary's own `zstd -b -T1`, six silesia files capped at 8 MiB, pinned, best of
alternating runs, [`bench/codec-board.ps1`](bench/codec-board.ps1)), our speed
relative to C's:

| encode | dickens | samba | x-ray | mozilla | nci | xml |
|---|---:|---:|---:|---:|---:|---:|
| L1 | 0.91× | 0.93× | **1.30×** | 0.89× | 0.94× | 0.93× |
| L3 | 0.87× | 0.94× | 0.75× | 0.94× | 0.89× | 0.91× |
| L9 | **1.02×** | 0.95× | 0.95× | 0.90× | **1.09×** | **1.03×** |
| L19 | **1.49×** | **1.17×** | **1.29×** | **1.45×** | **1.46×** | **1.27×** |

| decode | dickens | samba | x-ray | mozilla | nci | xml |
|---|---:|---:|---:|---:|---:|---:|
| L1 | 0.61× | 0.58× | 0.86× | 0.62× | 0.62× | 0.60× |
| L3 | 0.47× | 0.55× | 0.54× | 0.67× | 0.62× | 0.59× |
| L9 | 0.49× | 0.55× | 0.42× | 0.63× | 0.57× | 0.54× |
| L19 | 0.56× | 0.65× | 0.52× | 0.84× | 0.77× | 0.63× |

So in the codec, C's encoder leads at L1 (by 6–11%, except x-ray) and L3 (by
6–25%); the two are level at L9; we lead at L19. **On large inputs C's decoder
is 1.2–2.4× faster than ours** — the CLI's decode lead above is C's file
writing, not its codec. That is the next campaign. **Judge end-user experience
from the first two tables, codec work from these two.**

**0.3.0 is where the encoder caught up.** Against 0.2.5 on the standing board
(the six files above at L1/3/5/7/9/12, whole files, one core, best of five
alternating rounds), encode time is 0.61× at L1, 0.72× at L3, 0.68× at L5,
0.61× at L7, 0.54× at L9 and 0.31× at L12 — **0.56× by geometric mean, 1.8×
faster.** L1–L3 output is byte-identical to 0.2.5. L4–L12 move to libzstd's
row match finder and its lazy parse, and the output there gets **smaller**
(−1.8% at L5, −2.8% to −2.9% at L7–L12 over 18 corpora at 4 MiB), with one
corpus growing (`smallmsg-8m`, +2–3.6%). See
[`crates/rusty_zstd/CHANGELOG.md`](crates/rusty_zstd/CHANGELOG.md).

<sub>The 0.3.0 CLI table read encode 1.31–1.58× / 0.90–1.35× / 1.06–1.37× /
1.18–1.43× and decode 1.42–1.78× / 1.25–1.69× / 1.07–4.44× / 1.41–1.90× at
L1/3/9/19 as min–max ranges; this release reports C's best run over ours
instead (see above). Method history and the per-corpus stage shares:
[`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md) and
[`docs/plans/m7-benchmark-repair.md`](docs/plans/m7-benchmark-repair.md).</sub>

## What is this?

`rusty_zstd` compresses and decompresses Zstandard in pure Rust. Unlike the
[`zstd`](https://crates.io/crates/zstd) crate — which vendors facebook's C source
and calls it through [`zstd-sys`](https://crates.io/crates/zstd-sys) — there is
**no C in the dependency tree** here and nothing to build. Unlike the
decoder-only pure-Rust crates, the compressor is the whole level range with all
nine strategies behind it, plus dictionaries, LDM, seekable frames and
multi-threading.

It is a reimplementation of the format, not a wrapper around the original
implementation: match finders and entropy tables are free to differ from C's. The
contract is **format-legal and competitive**, not clone-the-bitstream. What *is*
gated byte-for-byte is everything that must be — the XXH64 checksum, the Huffman
and FSE table decoders, and every SIMD kernel against its scalar twin.

`cargo-deny` enforces the promise in CI: no `*-sys` crate, no copyleft, and no
`zstd-sys` / `libz-sys` / `lz4-sys` / `xxhash-sys` anywhere in the graph.

## The Remade With Rust ecosystem

<!-- ORG BOILERPLATE — keep identical across repos -->

**Remade With Rust** is an initiative by **[Mata Network](https://www.mata.network/)**
to rebuild essential C and C++ tools in Rust — for the memory safety, the
predictable performance, and the freedom of a permissive license. Each project
is a reimplementation, not a fork: same wire protocols and file formats, new
code you can actually depend on.

We build the core to production grade and open-source it so the community can
extend it. No copyleft. No surprises. Just the tools we rely on, made faster and
safer.

| Project | What it is |
|---|---|
| 🎬 **[remade_ffmpeg_rs](https://github.com/Remade-With-Rust/remade_ffmpeg_rs)** | **Our FFmpeg alternative.** Drop-in `ffmpeg` and `ffprobe` binaries — demux → decode → filter → encode → mux, rebuilt as composable Rust crates with **zero GPL/LGPL**. Apache-2.0. |
| 🧠 **[FFAI](https://github.com/Remade-With-Rust/FFAI)** | **Our sister project: media *for* AI.** "The AI media toolkit, remade with rust." Embedded ASR + TTS (**Mercury**), OCR (**Carmenta**) and vision-language captioning (**Argus**) behind an ffmpeg-style, swap-by-name architecture — no Python, no CUDA. MIT OR Apache-2.0. |
| 🌐 **[Mata Network](https://www.mata.network/)** | **The home page.** *"Stop sacrificing your privacy for convenience."* Sovereign, self-hostable privacy infrastructure — wallet & identity, password manager, contact manager, and a browser extension that stops information leaking as you browse. Remade With Rust is its open-source arm. |

→ All projects: **[github.com/Remade-With-Rust](https://github.com/Remade-With-Rust)**

<!-- /ORG BOILERPLATE -->

## Where `unsafe` is used

Performance is a requirement here, so `unsafe` is **permitted where it is
measured to pay**. The crate root is `#![deny(unsafe_code)]`, and each use opts
out with an item-level `#[allow(unsafe_code)]`. This README used to say all of
it lived in `simd.rs`; that was wrong — most of it is in the decoder's and the
match finders' hot loops. The full count, per module, over non-test code:

<!-- unsafe-census:begin (generated by tools/unsafe_census.py; do not edit) -->
| module | unsafe blocks | unsafe fns | `get_unchecked` | raw copies | `set_len` | SIMD intrinsics | blocks without SAFETY note |
|---|---:|---:|---:|---:|---:|---:|---:|
| `compressed.rs` | 33 | 3 | 9 | 19 | 11 | 2 | 11 |
| `huffman.rs` | 31 | 5 | 32 | 0 | 2 | 0 | 12 |
| `encode/tables.rs` | 27 | 0 | 29 | 0 | 0 | 0 | 27 |
| `simd.rs` | 24 | 12 | 0 | 0 | 0 | 34 | 0 |
| `fse.rs` | 22 | 2 | 19 | 0 | 0 | 0 | 16 |
| `encode.rs` | 15 | 3 | 5 | 0 | 0 | 0 | 2 |
| `rowfind.rs` | 13 | 2 | 4 | 0 | 0 | 9 | 0 |
| `encode/knobs.rs` | 12 | 1 | 8 | 3 | 3 | 0 | 7 |
| `encode/tree.rs` | 10 | 0 | 11 | 0 | 1 | 0 | 9 |
| `encode/rowgreedy.rs` | 6 | 0 | 0 | 0 | 0 | 0 | 1 |
| `xxh64.rs` | 6 | 2 | 0 | 8 | 0 | 16 | 2 |
| `encode/fast.rs` | 4 | 1 | 7 | 0 | 0 | 0 | 0 |
| `encode/dfast.rs` | 3 | 0 | 2 | 0 | 0 | 0 | 0 |
| `ldm.rs` | 3 | 0 | 3 | 0 | 0 | 0 | 3 |
| `profclock.rs` | 2 | 0 | 0 | 0 | 0 | 0 | 0 |
| `bit.rs` | 1 | 0 | 0 | 1 | 1 | 0 | 0 |
| **16 modules** | **212** | **31** | **129** | **31** | **18** | **61** | **90** |
<!-- unsafe-census:end -->

<sub>Generated by [`tools/unsafe_census.py`](tools/unsafe_census.py) and checked
by CI, which fails when the table no longer matches the source. "Without SAFETY
note" counts blocks with no `SAFETY` comment on or in the six lines above them —
a heuristic, and the backlog of sites still to document.</sub>

- **`simd.rs` holds the runtime-dispatched AVX2 and NEON kernels.** Callers stay
  safe; dispatch is a cached CPU-feature probe, so there is no ISA baseline to
  opt into and no build script. Every kernel keeps its scalar twin as the oracle
  and the fallback; the twins are gated equal in the test suite
  (`eq_oracle_exhaustive`, `count_eq_len_matches_byte_and_words`) and the scalar
  path is reachable on any CPU without the ISA.
- **Elsewhere, `unsafe` removes bounds checks and copies in hot loops** —
  `get_unchecked` in the Huffman, FSE and match-table lookups, unaligned word
  loads, `copy_nonoverlapping` for literal and match copies, and `set_len` on
  output buffers. Where a site has a `SAFETY:` note, it names the invariant;
  about half the blocks also sit beside a `debug_assert!` that checks it, so
  the debug test suite audits those. The table counts the blocks with no note
  yet.
- **What checks them, and what does not yet.** Hostile-input regression tests
  cover each decoder finding so far (`tests/strict_bitstream.rs` and the
  decoder's unit tests), and 0.3.1's big-endian fixes were verified once under
  Miri on s390x. There is no fuzzing and no Miri run in CI yet.
- **A kernel earns its place by measurement.** Bricks that do not measure better
  are reverted, and the ones reverted for a *reason* keep their measurement
  recorded in [`docs/plans/`](docs/plans/) so the idea is not re-litigated.

## Install

```sh
cargo add rusty_zstd
```

```toml
[dependencies]
rusty_zstd = "0.4"

# …or for embedded / wasm targets with no `std`:
rusty_zstd = { version = "0.4", default-features = false, features = ["alloc"] }
```

The minimum supported configuration is `no_std + alloc` — every entry point
returns or fills a `Vec`, so `alloc` is required. MSRV is **1.85**.

| Feature | Default | What it adds |
|---|:--:|---|
| `std` | ✅ | `std::io`-shaped streaming, multi-threading, the trainer, runtime ISA dispatch |
| `alloc` | ✅ | implied by `std`; the minimum supported configuration |
| `profile` | | the in-process stage profiler and its counters (off = zero overhead) |

## Quick start

```rust
use rusty_zstd::{compress, decompress, DEFAULT_CLEVEL};

let data = b"the quick brown fox jumps over the lazy dog ".repeat(64);

let packed = compress(&data, DEFAULT_CLEVEL)?;   // level 3, as libzstd defaults
assert!(packed.len() < data.len());
assert_eq!(decompress(&packed)?, data);          // lossless, always
```

Levels run **−7…22**. `compress_with` takes a `CompressOptions` for the checksum
flag, content size and dictionary ID; `compress_with_advanced` exposes the full
`AdvancedOptions` — window log, strategy, LDM, workers, job size, overlap.

Streaming, for data that does not fit in memory. The pump is libzstd-shaped: you
hand it an input slice and an output buffer, and it reports what it consumed and
produced.

```rust
use rusty_zstd::{compress_stream_out_size, Compressor, Flush};

let mut enc = Compressor::new(3)?;
let mut out = Vec::new();
let mut buf = vec![0u8; compress_stream_out_size()];

for chunk in [b"first chunk ".as_slice(), b"second chunk".as_slice()] {
    let mut fed = 0;
    while fed < chunk.len() {
        let st = enc.stream(&chunk[fed..], &mut buf, Flush::Continue)?;
        fed += st.input_consumed;
        out.extend_from_slice(&buf[..st.output_produced]);
    }
}
loop {
    let st = enc.stream(&[], &mut buf, Flush::End)?;
    out.extend_from_slice(&buf[..st.output_produced]);
    if st.done {
        break;
    }
}
assert_eq!(rusty_zstd::decompress(&out)?, b"first chunk second chunk");
```

Dictionaries, trained from your own samples — the win that matters for many small
records:

```rust
use rusty_zstd::{compress_using_dict, decompress_using_dict, train, Dictionary, TrainOptions};

let samples: Vec<Vec<u8>> = (0..256)
    .map(|i| format!("{{\"event\":\"click\",\"id\":{i},\"session\":\"abc123\"}}").into_bytes())
    .collect();
let refs: Vec<&[u8]> = samples.iter().map(|s| s.as_slice()).collect();

let raw = train(&refs, TrainOptions::default())?;    // fastcover, d=8 steps=4
let dict = Dictionary::from_bytes(&raw)?;

let packed = compress_using_dict(&samples[0], &dict, 3)?;
assert_eq!(decompress_using_dict(&packed, &dict)?, samples[0]);
```

## Command line

The CLI installs four names, exactly as facebook/zstd does. The primary is called
`rzstd` rather than `zstd` so it never shadows the C binary on your `PATH`.

```sh
cargo install --git https://github.com/Remade-With-Rust/rusty_zstd rusty_zstd-cli
```

```sh
rzstd -19 book.txt -o book.txt.zst      # -1…-19; --ultra for 20-22, --fast=N for negatives
unzstd book.txt.zst                     # alias for `rzstd -d`
zstdcat book.txt.zst | head             # alias for `rzstd -dcf`
zstdmt -T0 -19 huge.tar                 # alias for `rzstd -T0`

rzstd -l book.txt.zst                   # list frames
rzstd -t book.txt.zst                   # test integrity
rzstd -b3 book.txt                      # in-process benchmark
rzstd --train -o dict.bin samples/*     # fastcover trainer
rzstd --long=27 -19 huge.tar            # long-distance matching
rzstd --seekable --max-frame-size=1048576 archive.tar
rzstd --patch-from=v1.bin v2.bin -o v2.patch
```

`ZSTD_CLEVEL` and `ZSTD_NBTHREADS` are honoured. `rzstd --help` lists everything.

## Architecture

```
crates/
  rusty_zstd        the library -- compress, decompress, streaming, dicts,
                    trainer, LDM, seekable, MT. Zero dependencies. PUBLISHED.
  rusty_zstd-cli    rzstd / unzstd / zstdcat / zstdmt, four shims over one entry()
  rusty_zstd-bench  the campaign harness: shells out to a pinned C `zstd`,
                    never links libzstd. Not published.
  rzstd-alloc       the `rusty_alloc` seam -- a #[global_allocator] for the
                    binaries only, never for the library. Not published.
bench/ledger.jsonl  append-only measurement ledger; every public number is a row
docs/plans/         the mission, the anatomy boards, and the whys descents
```

Inside the library, the format is one module per layer: `frame` (headers, magic),
`block`, `huffman`, `fse`, `compressed` (the sequence decoder), `encode` (all
nine match finders), `decode`, `stream`, `dict`, `ldm`, `seekable`, `mt`,
`xxh64`, and the `simd` island.

## Correctness

`rusty_zstd` is gated against facebook/zstd **v1.5.7** as an external process —
the bench crate shells out to a pinned binary and never links libzstd, so the C
implementation cannot leak into the dependency graph.

- **Both directions, every commit.** C compresses → we decompress bit-exact; we
  compress → C's `zstd -t` and `zstd -d` accept it. The standing gate is 12
  Silesia files × 3 flag sets, plus generated train/holdout corpora.
- **Byte-identity where it is owed.** The XXH64 checksum is gated against the
  published XXH64 vectors and a GOLD oracle; every SIMD kernel is gated against
  its scalar twin; the Huffman and FSE table decoders are gated against C's.
- **`no_std + alloc`, `wasm32-unknown-unknown` and two bare-metal targets build
  in CI**, so the portable configuration cannot rot. The bare-metal rungs are
  Cortex-M4F and RV32IMAC, neither of which has 64-bit atomics: a `no-std`
  category is not a claim, a target that compiles is. Note that the measurement
  counters need `AtomicU64`, so on a part without it they compile to a
  zero-sized stub and `census64::CENSUS_LIVE` reads `false` -- a zero from a
  counter there means "not measurable on this target", never "measured zero".

Fetch the oracle with `pwsh scripts/fetch-oracle.ps1`, or point
`RUSTY_ZSTD_ORACLE` at any `zstd` binary whose `--version` says 1.5.7. Details:
[`third_party/zstd/README.md`](third_party/zstd/README.md).

## Platform support

| Platform | Status |
|---|---|
| Windows (x86-64) | ✅ builds + tests |
| Linux (x86-64) | ✅ builds + tests |
| macOS (x86-64 / aarch64) | ✅ builds + tests |
| `wasm32-unknown-unknown` | ✅ builds (`std` and `alloc`) |
| `no_std + alloc` | ✅ builds |
| `thumbv7em-none-eabihf` (Cortex-M4F) | ✅ builds (`no_std + alloc`) |
| `riscv32imac-unknown-none-elf` | ✅ builds (`no_std + alloc`) |
| ESP32-S3 (Xtensa LX7) | ✅ **round-trips on the board** ([proof](bare-metal/esp32s3/README.md)) |

AVX2 and NEON kernels are selected at **runtime**. On a CPU without them, the
scalar twins run and the output is identical.

## Roadmap

- [x] **M1 — decompressor core.** All RFC 8878 block types, skippable frames,
      multi-frame concatenation, checksums; C-compressed holdout decodes bit-exact
- [x] **M2 — compressor core.** Levels −7…3, dual-gated round-trip, streaming
      APIs, CLI `-z / -d / -t / -c / -o`
- [x] **M3 — entropy parity.** Huffman literals (1- and 4-stream, treeless) and
      all four FSE sequence modes on encode; the full −7…22 range with all nine
      strategies
- [x] **M4 — dictionaries.** Raw-content and trained dictionary format, the
      `fastcover` / `COVER` / legacy trainers, `--patch-from` prefixes
- [x] **M5 — LDM and seekable.** Long-distance matching, `--rsyncable`, the
      seekable frame format with random-access decompress
- [x] **M6 — CLI completeness.** Multi-threading (`-T#`, `--jobsize`,
      `--overlap-log`), `-l` / `-b` / `-r`, env vars, and the `unzstd` /
      `zstdcat` / `zstdmt` aliases — all dual-gated against C
- [x] **M7 (in progress) — the performance campaign.** 0.3.0 made encode
      1.8× faster than 0.2.5; 0.4.0 made small frames decode 4–6× faster and
      beat libzstd's one-shot API on rows up to 512 B. In the codec alone our
      encoder is level with C at L9 and ahead at L19; C still leads at L1
      (6–11%) and L3 (6–25%). At default settings we emit +0.3% (L9) to +3.8%
      (L19) more than C. The CLI beats C's end to end at every level measured
      (see Performance above), partly on its write path
- [ ] Mission §7's exit bars: compress within **1.25×** of C at L1/L3 (met in
      the codec except x-ray at L3, 1.33×) and decompress within **1.11×** in
      the codec — **not met**: measured for 0.4.0, C's decoder is 1.2–2.4×
      faster on large inputs
- [ ] Legacy frame decode (v0.1–v0.7), behind the `legacy` feature
- [ ] The optional C ABI `cdylib`, so existing C callers can relink

## License

**MIT OR Apache-2.0**, at your option — see [LICENSE-MIT](LICENSE-MIT) and
[LICENSE-APACHE](LICENSE-APACHE). No GPL/LGPL and no C anywhere in the dependency
tree, CI-enforced with `cargo-deny`. The C `zstd` binary used as a measurement
oracle is neither distributed here nor linked; see [NOTICE.md](NOTICE.md).

## About Mata Network

<!-- ORG BOILERPLATE — keep identical across repos -->

**[Mata Network](https://www.mata.network/)** builds sovereign, self-hostable
privacy infrastructure — *"stop sacrificing your privacy for convenience"*:
wallet & identity, a password manager, a contact manager, and a browser
extension that stops your information leaking as you browse.

**Remade With Rust** is our open-source home for the permissively-licensed
building blocks that work depends on — including
[remade_ffmpeg_rs](https://github.com/Remade-With-Rust/remade_ffmpeg_rs) (the
FFmpeg alternative) and [FFAI](https://github.com/Remade-With-Rust/FFAI) (the
AI media toolkit).

→ **[www.mata.network](https://www.mata.network/)**

<!-- /ORG BOILERPLATE -->
