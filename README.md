> **In the wild** — [RAG Converter](https://ragconverter.com) uses `rusty_zstd` for compression.
> It makes personal and work files AI-readable without them leaving the machine:
> the whole conversion runs as WebAssembly in the browser tab, with nothing
> uploaded and nothing to install.

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

### Performance — default deployment, measured and honestly reported

**Both CLIs at their real defaults.** `zstd -<lvl> <files>` against
`rzstd -<lvl> <files>`, no flags beyond the level, one invocation each so
startup is paid once. 19 files, 143.9 MB. This is the only true
default-vs-default comparison available: `zstd -b` **cannot** include a
checksum — `-b --check` is accepted and ignored — yet both projects default to
checksum **ON**, so a `-b` table compares two configurations neither project
ships. Each arm gets its own staged copy of the inputs, and each decodes the
other's output; the cross-check passed on every run below.

| level | encode vs C | decode vs C | size vs C |
|---|---:|---:|---:|
| **L1** | 0.56–0.61× | **2.58–2.82×** | **+2.00%** |
| **L3** (default) | 0.73–0.75× | **2.20–3.12×** | **+2.11%** |
| **L9** | 0.34–0.38× | **2.32–2.85×** | **+2.45%** |
| **L19** | **1.22–1.40×** | **2.64–2.76×** | **+3.82%** |

<sub>**The two columns come from different runs, and mixing them would be
sloppy, so here is exactly which is which.** The **size** column was
re-measured for 0.2.3 over the **full 19-file corpus, 355,593,492 bytes**,
both CLIs at their defaults: at L1 we emit 116,548,162 bytes against C's
114,259,341. It is deterministic and reproduces bit-for-bit. The **speed**
ranges are carried over from the 0.2.0 measurement, which ran the same 19
files in a *capped* staging totalling 143.9 MB, and were **not** re-measured
here — this host has been under sustained load all campaign, and a speed
number taken today would be a worse estimate than the one it replaced, not a
better one.</sub>

<sub>**Speed cells are ranges, not point estimates, and that is deliberate.**
Min–max over independent samples at N=10 per arm per phase, on a host that was
not quiescent. The same measurement at N=3 read L1 decode as 2.66× on one
sample and 1.57× on the next, and even at N=10 the L3 encode cell moved 0.63×
to 0.93× *between batches*. Anything quoted here as a single number would be
one draw from that spread.</sub>

<sub>**0.2.3 moved the size column on purpose, in both directions.** DFast
back-extension took **L3 −1.28%** and L4 −1.15% — a pure win on the default
level's ladder, no corpus regressing. Tightening the chain walk's first-find
bar traded **+0.35% to +1.39% size for +5.2% to +38.1% encode throughput** at
L5–L12; it is the one deliberate size-for-speed trade in this encoder, and
`RZSTD_WALK_FIRST_MAX=0.70` restores the previous bitstream exactly at L7/L9.
The L1 and L19 rows are bit-identical to 0.2.2 — neither change touches those
ladders, so their movement above is corpus, not code.</sub>

<sub>**READ THIS BEFORE OPTIMISING FROM IT — the decode figure is not a codec
claim.** These are whole-program numbers: read + codec + write. On this host
decode is dominated by each CLI's file-**writing** strategy, not by the codec.
Decoding the same 24 MiB to files took C 398 ms and us 93 ms; the same decode
to **stdout**, with the write removed, took C 171 ms and us 194 ms. The
advantage is the write path, and with it removed C is ahead. Measured
in-process on the codec alone — checksum off on both arms, as `zstd -b` runs —
C leads **both** phases: by ~1.6× at L1/L3 and by ~2.1–2.8× at L9. The
in-process encode figures (0.54× / 0.63× / 0.36× at L1/L3/L9) agree closely
with the CLI encode column above, and two instruments that disagree about
decode while agreeing about encode is what establishes the encode gap as a
real codec property rather than an artifact of either harness.
**Judge end-user experience from this table; judge codec work from**
[`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md).</sub>

<sub>**The speed columns moved between releases and that is NOT attributable to
this project's changes.** Every level's throughput read higher in the 0.2.0
measurement than in 0.1.0's — including L9, whose compressed bytes are
*bit-identical* between the two, so its code path did not change. When an
unchanged path speeds up, the lift is the host, not the codec. The SIZE column
carries no such caveat: it is deterministic, and its 0.1.0 → 0.2.0 movement at
L1 and L3 is exactly the fill-density change described in the changelog.</sub>

<sub>**No claim is made that the optimization campaign made this faster.**
Every brick in it shipped on strictly-less-work plus byte-identity, never on a
wall-clock delta, and the boards say so explicitly. The per-file spread is the
story and these files differ enormously — **never average them**. Method,
per-corpus stage shares, and the instrument's own repair history:
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
- [x] **M7 (in progress) — the performance campaign.** At default settings we
      emit **+0.9%** (L1) to **+2.2%** (L3) more than C, and the CLI decodes
      **faster** than C end-to-end — but that is the write path, not the codec
      (see Performance above). The codec speed gap, ~1.6× on both phases, is
      the open work, and it is concentrated in the **mid-level match finders**
      (L5–L12): the opt-class finders at L19 already reach C's encode rate
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
