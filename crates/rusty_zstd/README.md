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
`rzstd -<lvl> <files>`, no flags beyond the level, each arm decoding the
other's output (cross-checked every run).

| level | encode vs C | decode vs C | size vs C |
|---|---:|---:|---:|
| L1 | 0.56–0.61× | **2.58–2.82×** | +2.00% |
| L3 (default) | 0.73–0.75× | **2.20–3.12×** | +2.11% |
| L9 | 0.34–0.38× | **2.32–2.85×** | +2.45% |
| L19 | **1.22–1.40×** | **2.64–2.76×** | +3.82% |

The two columns come from different runs. **Size** was re-measured for 0.2.3
over the full 19-file corpus, 355,593,492 bytes — deterministic, reproduces
bit-for-bit. **Speed** is carried over from the 0.2.0 run, which used the same
files in a capped 143.9 MB staging, and was not re-measured: this host has been
under sustained load, and a fresh number would be a worse estimate than the one
it replaced.

**Read the decode column carefully — it is not a codec claim.** These are
whole-program numbers (read + codec + write), and decode is dominated by each
CLI's file-*writing* strategy. Decoding 24 MiB to files: C 398 ms, us 93 ms; the
same decode to *stdout*, with the write removed: C 171 ms, us 194 ms. Measured
in-process on the codec alone, C leads decode. Judge end-user experience from
this table; judge codec work from
[`docs/plans/m7-anatomy.md`](https://github.com/Remade-With-Rust/rusty_zstd/blob/main/docs/plans/m7-anatomy.md).

Speed cells are min–max over independent samples (N=10 per arm per phase) on a
non-quiescent host, so they are ranges rather than point estimates. The size
column is deterministic and exact. Encode trails C at L1–L9 and leads at L19;
closing the mid-level gap is the open work.

**0.2.3 moved the size column deliberately, in both directions.** DFast back
extension took L3 −1.28% and L4 −1.15%, a pure win on the default level with no
corpus regressing. Tightening the chain walk's first-find bar traded +0.35% to
+1.39% size for +5.2% to +38.1% encode throughput at L5–L12 — the one
deliberate size-for-speed trade in this encoder. Setting
`RZSTD_WALK_FIRST_MAX=0.70` restores the previous bitstream exactly at L7/L9.

## Correctness

Gated against facebook/zstd **v1.5.7** as an external process, in both
directions, every commit: C compresses → this decompresses bit-exact, and this
compresses → C's `zstd -t` and `zstd -d` accept it. The XXH64 checksum is gated
against the published vectors, and every SIMD kernel against its scalar twin.

## License

**MIT OR Apache-2.0**, at your option. No GPL/LGPL and no C anywhere in the
dependency tree — CI-enforced with `cargo-deny`.
