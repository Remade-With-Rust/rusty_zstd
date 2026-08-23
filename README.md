# rusty_zstd

[![crates.io](https://img.shields.io/crates/v/rusty_zstd?logo=rust)](https://crates.io/crates/rusty_zstd)
[![docs.rs](https://img.shields.io/docsrs/rusty_zstd?logo=docsdotrs)](https://docs.rs/rusty_zstd)
[![CI](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml/badge.svg)](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](#license)
[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/Remade-With-Rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network)

> **rusty_zstd** is a ground-up, pure-**Rust** [Zstandard](https://facebook.github.io/zstd/)
> ([RFC 8878](https://datatracker.ietf.org/doc/html/rfc8878)) **compressor and
> decompressor**: `#![deny(unsafe_code)]` everywhere but one audited SIMD island,
> **zero dependencies**, no C, no `*-sys` crate, no FFI. Every frame we emit
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

## ⚡ The headline

A pure-Rust Zstandard codec that is **interoperable in both directions with the C
reference**, ships the whole product surface rather than a decoder, and is
**at parity on ratio at matched settings**:

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
- **`#![deny(unsafe_code)]`, with one island.** All `unsafe` in the crate lives
  in `simd.rs`, the runtime-dispatched AVX2/NEON kernels. Every kernel keeps its
  scalar twin as the oracle and the fallback, and the twins are gated equal in
  the test suite.
- **Zero dependencies.** Not "few" — the published library's dependency tree is
  empty. It builds on `wasm32-unknown-unknown` and on `no_std + alloc`.

| | facebook/zstd (C) | **rusty_zstd (Rust)** |
|---|---|---|
| C/C++ in the dependency tree | all of it | **none** — the library has zero dependencies |
| `unsafe` in the codec | pervasive | **one audited module** (`simd.rs`), scalar twins kept as oracles |
| License | BSD-2 **or** GPLv2 | **MIT OR Apache-2.0** |
| Interop with the other implementation | — | **both directions**, dual-gated per commit |
| `no_std` / `wasm32` | needs a porting layer | **builds as-is** |
| Levels / strategies | −7…22, 9 strategies | **−7…22, all 9** |

### Performance — measured, and honestly reported

The board below is the **2026-08-22 third pass** of
[`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md): level 1, all 18 corpora,
against **facebook/zstd v1.5.7** driven as a pinned external binary. Best-of-N on
both arms, phases timed separately as C's `-b` does, N ≥ 25 per phase, warmup
discarded. `C/us` **> 1 means C is faster**; `us/c size` **> 1 means we emit more
bytes**. The session's null arm — the worst same-arm spread — was **6.83%**, the
lowest this instrument has recorded, which is what makes these cells readable at
all.

| | compress speed | decompress speed | **ratio (`us/c size`)** |
|---|---|---|---|
| mean over 18 corpora | **1.83× behind C** | **1.49× behind C** | **0.975 — we emit fewer bytes than C** |
| we match or beat C on | 2 corpora | 4 corpora | 2 corpora |
| worst cell | `ooffice` 2.61× | `reymont` 2.21× | `text-32m` 1.126 |

Selected rows, sorted by ratio (throughput in MB/s):

| corpus | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | **us/c size** |
|---|---:|---:|---:|---:|---:|---:|---:|
| `versions-16m` | 1641.7 | 7057.8 | **0.23** | 3936.3 | 20481.3 | **0.19** | **0.088** |
| `zeros-32m` | 12579.3 | 31961.6 | **0.39** | 38223.5 | 41710.1 | **0.92** | **0.967** |
| `incomp-32m` | 5921.6 | 3324.1 | 1.78 | 22293.3 | 24081.9 | **0.93** | 1.000 |
| `x-ray` | 906.4 | 363.5 | 2.49 | 1322.3 | 1109.4 | 1.19 | 1.000 |
| `mr` | 481.2 | 205.5 | 2.34 | 1668.0 | 1046.3 | 1.59 | 1.010 |
| `dickens` | 345.9 | 188.3 | 1.84 | 1648.8 | 867.9 | 1.90 | 1.015 |
| `mozilla` | 716.8 | 348.8 | 2.06 | 2120.8 | 1270.2 | 1.67 | 1.034 |
| `nci` | 954.9 | 487.5 | 1.96 | 2689.7 | 1375.8 | 1.95 | 1.106 |
| `text-32m` | 19454.1 | 12777.5 | 1.52 | 9648.1 | 34379.0 | **0.28** | 1.126 |

<sub>**Read this as: ratio is at parity; speed is not, and that is the open
work.** At matched settings the sizes we emit are within ~1.6% of C on eleven of
eighteen corpora and *smaller* on average — mean `us/c size` is **0.975** at L1
and **1.012** at L3, and the L3 ratio column has now been identical
cell-for-cell across **four consecutive boards**, which doubles as an end-to-end
identity check on the campaign. On speed, C is ahead by ~1.8× compressing and
~1.5× decompressing in the mean, and we are ahead on the degenerate and
highly-repetitive corpora where the repcode and match-find work pays off.
**No claim is made that the optimization campaign made this faster:** every brick
in it shipped on strictly-less-work plus byte-identity, never on a wall-clock
delta, and the boards say so explicitly. Method, per-corpus stage shares, and the
instrument's own repair history:
[`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md) and
[`docs/plans/m7-benchmark-repair.md`](docs/plans/m7-benchmark-repair.md).
**Never average these files** — the per-file spread is the story.</sub>

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

## Where `unsafe` is allowed

Performance is a requirement here, so `unsafe` is **permitted where it is
justified** — but deliberately, and in one place rather than scattered:

- **`simd.rs` is the designated island.** It holds the runtime-dispatched AVX2
  and NEON kernels, and it is the only module that opts out of the crate's
  `#![deny(unsafe_code)]`. Callers stay safe; dispatch is a cached CPU-feature
  probe, so there is no ISA baseline to opt into and no build script.
- **Every kernel keeps its scalar twin as the oracle and the fallback.** The
  twins are gated equal in the test suite (`eq_oracle_exhaustive`,
  `count_eq_len_matches_byte_and_words`) and the scalar path is reachable on any
  CPU without the ISA.
- **Elsewhere, each `get_unchecked` site carries a `SAFETY:` note naming the
  invariant and a `debug_assert!` that checks it** — so the debug test suite is a
  standing audit of every one of them.
- **A kernel earns its place by measurement.** Bricks that do not measure better
  are reverted, and the ones reverted for a *reason* keep their measurement
  recorded in [`docs/plans/`](docs/plans/) so the idea is not re-litigated.

## Install

```sh
cargo add rusty_zstd
```

```toml
[dependencies]
rusty_zstd = "0.1"

# …or for embedded / wasm targets with no `std`:
rusty_zstd = { version = "0.1", default-features = false, features = ["alloc"] }
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
- **`no_std + alloc` and `wasm32-unknown-unknown` build in CI**, so the portable
  configuration cannot rot.

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
- [x] **M7 (in progress) — the performance campaign.** Ratio has reached parity
      (mean `us/c size` **0.975** at L1, **1.012** at L3); the speed gap is
      ~1.8× compress / ~1.5× decompress and is the open work
- [ ] Mission §7's exit bars: compress within **1.25×** of C at L1/L3 and
      decompress within **1.11×**, which the current board does not yet clear
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
