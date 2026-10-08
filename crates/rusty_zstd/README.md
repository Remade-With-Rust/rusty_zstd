> **In the wild** — [RAG Converter](https://ragconverter.com) uses `rusty_zstd` for compression.
> It makes personal and work files AI-readable without them leaving the machine:
> the whole conversion runs as WebAssembly in the browser tab, with nothing
> uploaded and nothing to install.

# rusty_zstd

[![crates.io](https://img.shields.io/crates/v/rusty_zstd?logo=rust)](https://crates.io/crates/rusty_zstd)
[![docs.rs](https://img.shields.io/docsrs/rusty_zstd?logo=docsdotrs)](https://docs.rs/rusty_zstd)
[![CI](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml/badge.svg)](https://github.com/Remade-With-Rust/rusty_zstd/actions/workflows/ci.yml)
[![License: MIT OR Apache-2.0](https://img.shields.io/badge/license-MIT%20OR%20Apache--2.0-blue)](https://github.com/Remade-With-Rust/rusty_zstd#license)
[![Remade With Rust](https://img.shields.io/badge/Remade%20With-Rust-000?logo=rust&logoColor=fff)](https://github.com/Remade-With-Rust)
[![By Mata Network](https://img.shields.io/badge/by-Mata%20Network-5b2be0)](https://www.mata.network/)

> **A ground-up, pure-Rust [Zstandard](https://facebook.github.io/zstd/)
> ([RFC 8878](https://datatracker.ietf.org/doc/html/rfc8878)) compressor and
> decompressor.** `#![deny(unsafe_code)]` everywhere but one audited SIMD island,
> **zero dependencies**, no C, no `*-sys` crate, no FFI. Every frame it emits
> decompresses in facebook/zstd v1.5.7 and every frame that emits decompresses
> here — dual-gated on the Silesia corpus every commit.

Part of **[Remade With Rust](https://github.com/Remade-With-Rust)** by
**[Mata Network](https://www.mata.network/)**.
Full README, benchmark boards and methodology:
**[the repository](https://github.com/Remade-With-Rust/rusty_zstd)**.

---

## Install

```sh
cargo add rusty_zstd
```

```toml
[dependencies]
rusty_zstd = "0.3"

# …or for embedded / wasm targets with no `std`:
rusty_zstd = { version = "0.3", default-features = false, features = ["alloc"] }
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
# Ok::<(), rusty_zstd::Error>(())
```

Levels run **−7…22** with all nine libzstd strategies behind them.
`compress_with` takes a `CompressOptions` for the checksum flag, content size and
dictionary ID; `compress_with_advanced` exposes the full `AdvancedOptions` —
window log, strategy, LDM, workers, job size, overlap.

Streaming, for data that does not fit in memory. The pump is libzstd-shaped: hand
it an input slice and an output buffer, and it reports what it consumed and
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
# Ok::<(), rusty_zstd::Error>(())
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
# Ok::<(), rusty_zstd::Error>(())
```

## What is in here

| Area | Surface |
|---|---|
| **One-shot** | `compress`, `decompress`, `decompress_into`, `compress_bound`, `content_size`, `find_frame_compressed_size` |
| **Options** | `CompressOptions`, `AdvancedOptions`, `DecompressOptions`, `CompressionParameters`, `Strategy` |
| **Streaming** | `Compressor`, `Decompressor`, `Flush`, `StreamStatus`, and the four recommended-buffer-size helpers |
| **Dictionaries** | `Dictionary`, `compress_using_dict`, `decompress_using_dict`, `compress_using_prefix` (patch-from), `train` + `TrainOptions` / `TrainAlgo` |
| **Long-range** | `LdmParams`, `DEFAULT_LONG_WINDOW_LOG` |
| **Seekable** | `compress_seekable`, `decompress_frame_at`, `parse_seek_table`, `SeekTable`, `SeekEntry` |
| **Multi-thread** | `compress_mt`, `default_nb_workers`, `resolve_job_size`, `overlap_size` |
| **Inspection** | `get_frame_header`, `FrameHeader`, `FrameKind`, `inspect_frames`, `ListedFrame` |
| **Checksum** | `xxh64` — the frame content hash, usable on its own |

Items marked `#[doc(hidden)]` are campaign instrumentation for the repository's
own benchmark harness. They carry **no semver promise** and may be renamed or
removed in any release.

## Performance

Both CLIs at their real defaults — `zstd -<lvl> <files>` against
`rzstd -<lvl> <files>`, no flags beyond the level, one invocation per run over
19 corpus files capped at 8 MiB each (143.9 MB), 10 alternating runs per arm,
pinned to one core, each arm decoding the other's output as the cross-check.
Ratios are C's time over ours: **above 1× we are faster.**

| level | encode vs C | decode vs C | size vs C |
|---|---:|---:|---:|
| L1 | **1.31–1.58×** | **1.42–1.78×** | +2.00% |
| L3 (default) | 0.90–1.35× | **1.25–1.69×** | +1.76% |
| L9 | **1.06–1.37×** | **1.07–4.44×** | +0.29% |
| L19 | **1.18–1.43×** | **1.41–1.90×** | +3.82% |

Measured for 0.3.0 against the official zstd 1.5.7 binary. Speed cells are
min–max over paired samples on a non-quiescent host. **Size** is the full,
uncapped corpus (355,593,492 bytes) and is deterministic.

**These are whole-program numbers (read + codec + write), and C's CLI is slow
at writing files on this host — the decode column is mostly that.** In the
codec alone (in-process, no checksum, no I/O, against `zstd -b
--single-thread` on six silesia files) our encode runs at 0.91–1.27× of C's
speed at L1, 0.74–0.99× at L3, 0.91–1.08× at L9 and 1.20–1.49× at L19. Judge
end-user experience from the table; judge codec work from
[`docs/plans/m7-anatomy.md`](https://github.com/Remade-With-Rust/rusty_zstd/blob/main/docs/plans/m7-anatomy.md).

**0.3.0 made encode 1.8× faster than 0.2.5** (geometric mean over six silesia
files at L1/3/5/7/9/12: 0.61× / 0.72× / 0.68× / 0.61× / 0.54× / 0.31× of the
0.2.5 time). L1–L3 output is byte-identical to 0.2.5; L4–L12 move to
libzstd's row match finder and emit smaller output (−1.8% to −2.9% over 18
corpora), with one corpus (`smallmsg-8m`) growing 2–3.6%. Details in the
[changelog](https://github.com/Remade-With-Rust/rusty_zstd/blob/main/crates/rusty_zstd/CHANGELOG.md).

## Correctness

Gated against facebook/zstd **v1.5.7** as an external process, in both
directions, every commit: C compresses → this decompresses bit-exact, and this
compresses → C's `zstd -t` and `zstd -d` accept it. The XXH64 checksum is gated
against the published vectors, and every SIMD kernel against its scalar twin.

## License

**MIT OR Apache-2.0**, at your option. No GPL/LGPL and no C anywhere in the
dependency tree — CI-enforced with `cargo-deny`.
