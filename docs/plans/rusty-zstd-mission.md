# rusty_zstd — deployment plan: a 1:1 competitor to facebook/zstd

**Component:** rusty_zstd — pure-Rust Zstandard codec, library, CLI, and optional C ABI
**Bar:** [facebook/zstd](https://github.com/facebook/zstd) (libzstd + `zstd` CLI), format [RFC 8878](https://datatracker.ietf.org/doc/html/rfc8878)
**Status:** M2 complete -- compressor core (−7…3 fast/dfast/greedy; 4…22 greedy stand-in), dual-gate round-trip, streaming APIs, CLI `-z/-d/-t/-c/-o`. Huffman/FSE encode parity and lazy/bt are M3.
**License:** Apache-2.0 / MIT dual
**Prime directive:** Pure Rust end-to-end. Zero C/FFI on the core path. Every public claim is a ledger line against a pinned C libzstd. No claim without a number.

This is a **deployment plan**, not a sketch. The product is a drop-in competitor: same frames, same CLI habits, same embedder jobs, same (or better) speed/ratio, under a permissive license you can actually ship.

---

## 0. Competitive thesis

facebook/zstd is the world standard for fast lossless compression. It is C, dual BSD/GPLv2, and the definition of the format. rusty_zstd exists so Remade-With-Rust — and anyone who cannot or will not take a C toolchain / copyleft dual-license into the tree — has a **production replacement**, not a hobby decoder.

We win if a user who today types `zstd -T0 --long -19` or calls `ZSTD_compressStream2` can switch to rusty_zstd and keep:

1. **Interoperability** — every RFC 8878 frame we emit decompresses in C libzstd; every RFC 8878 frame C libzstd emits decompresses in us.
2. **Behavior** — gzip-like CLI, same flags, same env vars, same aliases (`zstdmt`, `unzstd`, `zstdcat`).
3. **Jobs** — one-shot, streaming, dictionaries, long-distance matching, multi-thread, rsyncable, patch-from, seekable, list/test/bench/train.
4. **Numbers** — decompression at or near C; compression within a small factor of C at matched ratio (or better ratio at matched speed) on representative corpora.
5. **Posture** — memory-safe, `wasm32`-buildable core, no `*-sys`, no libzstd in the dependency tree.

We do **not** compete by emitting bit-identical compressed bytes. Match finders and entropy tables are allowed to differ. The contract is **format-legal + competitive Pareto**, not clone-the-bitstream.

Peers (`ruzstd`, `zstd-safe`/`zstd-sys`, `compression-zstd`, etc.) are comparison arms, never the bar. The bar is C libzstd.

---

## 1. What “1:1 competitor” means

Four surfaces. A skipped surface is not a pass.

| Surface     | 1:1 means                                                                                                                                                  | Explicitly not required                                     |
| ----------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------- |
| **Format**  | Full RFC 8878 read + write. Cross-decode both directions. Skippable frames, multi-frame concat, dict ID, content size, XXH64 checksum.                     | Bit-identical compressed output vs C                        |
| **Library** | Rust API covering the libzstd jobs: simple, explicit context, streaming, dict, prefix/patch, advanced params, MT, sequences, frame inspect, size estimates | C header layout, `size_t` error codes in the Rust API       |
| **CLI**     | Drop-in for the `zstd` man page: modes, modifiers, env (`ZSTD_CLEVEL`, `ZSTD_NBTHREADS`), aliases                                                          | Shipping `gzip`/`xz`/`lz4` as C-linked extra formats        |
| **C ABI**   | Optional `cdylib` that exports the stable libzstd simple + streaming + dict symbols so existing C callers can relink                                       | 100% of experimental `ZSTD_c_experimentalParam*` on day one |

**Decoder is bit-exact on reconstructed bytes.** Encoder is ratio/speed gated, not byte-gated, except where a brick is *supposed* to be identical (entropy primitives, Huffman/FSE table decode, checksum).

---

## 2. Format contract (RFC 8878)

Must implement, no shortcuts:

| Element           | Contract                                                                                                                                             |
| ----------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------- |
| Frame magic       | `0xFD2FB528` (Zstd), skippable `0x184D2A5X`                                                                                                          |
| Frame header      | Descriptor, Window_Descriptor, Dictionary_ID (0/1/2/4 bytes), Frame_Content_Size (0/1/2/4/8), Single_Segment                                         |
| Blocks            | Raw / RLE / Compressed; max block 128 KiB (`ZSTD_BLOCKSIZE_MAX`); last-block flag                                                                    |
| Literals          | Raw, RLE, Compressed (Huffman 1-stream + 4-stream), Treeless; Jump table                                                                             |
| Sequences         | FSE / Repeat / Predefined / RLE tables; `litLength`, `matchLength`, `offset`; Repeat_Offsets (rep[3])                                                |
| Content checksum  | XXH64 of uncompressed data, low 32 bits, little-endian (when flag set)                                                                               |
| Multi-frame       | Concatenated `.zst` decompresses as one stream (CLI and library)                                                                                     |
| Dictionaries      | Raw-content dicts + trained dict format (magic `0xEC30A437`, dict ID, entropy tables, content)                                                       |
| Window            | Default CLI decompress cap **128 MiB**; spec max windowLog 31 (2 GiB) on 64-bit. Frames above the cap **reject** unless `-M` / `windowLogMax` raised |
| Reserved dict IDs | RFC 8878: IDs `< 32768` and `>= 2^31` not for public use — trainer must not emit them unless `--dictID` forces                                       |

**Legacy frames (pre-v0.8, v0.1–v0.7):** decode-only, behind Cargo feature `legacy`, default **on in the CLI** to match facebook/zstd (`ZSTD_LEGACY_SUPPORT=4` ⇒ v0.4+). Encode never emits legacy. Library default: legacy **off** (opt-in), so embedders do not pay for it.

**Magicless format (`ZSTD_f_zstd1_magicless`):** library-only, for containers that already framed the payload. Required for ABI parity; not a CLI default.

**Seekable format** ([contrib spec](https://github.com/facebook/zstd/blob/dev/contrib/seekable_format/zstd_seekable_compression_format.md)): independent frames + skippable seek table. First-class library + CLI (`--seekable` / random-access decompress). Not RFC 8878, but it is how production zstd does range GETs. Ship it.

---

## 3. Product surfaces

### 3.1 Library (Rust) — the core deliverable

Crate `rusty_zstd`. Safe public API. Stages are independently callable; `Compress` / `Decompress` compose them.

**Simple**

```rust
fn compress(src: &[u8], level: i32) -> Result<Vec<u8>, Error>;
fn decompress(src: &[u8]) -> Result<Vec<u8>, Error>;
fn decompress_to(src: &[u8], dst: &mut [u8]) -> Result<usize, Error>;
fn compress_bound(src_len: usize) -> usize;
fn content_size(src: &[u8]) -> Result<Option<u64>, Error>;
fn find_frame_compressed_size(src: &[u8]) -> Result<usize, Error>;
```

Levels: **−7…22** (`--fast=7` … `--ultra` 22). Default **3**. `min_clevel` / `max_clevel` / `default_clevel` match libzstd.

**Explicit context** (maps to `ZSTD_CCtx` / `ZSTD_DCtx`)

- Reusable `Compressor` / `Decompressor`
- `set_parameter(CParameter, i32)` / `DParameter`
- `set_pledged_src_size`
- `reset(session | parameters | session_and_parameters)`
- Dictionary load / `CDict` / `DDict` (pre-digested)
- `ref_prefix` (patch-from / diff engine)
- `compress2` one-shot on a configured context

**Streaming** (maps to `ZSTD_compressStream2` / `ZSTD_decompressStream`)

```rust
enum Flush { Continue, Flush, End }
impl Compressor {
    fn stream(&mut self, input: &[u8], output: &mut [u8], flush: Flush) -> Result<StreamStatus, Error>;
}
impl Decompressor {
    fn stream(&mut self, input: &[u8], output: &mut [u8]) -> Result<StreamStatus, Error>;
}
```

Bounded memory. Recommended in/out buffer sizes exported (`in_size()`, `out_size()`). Multi-frame. Error on window > cap.

**Frame inspection**

- `get_frame_header` (window, dict ID, content size, checksum flag, block size max)
- `list` metadata (CLI `-l`)
- `decompression_margin` / in-place decompress sizing
- `sizeof_*` workspace estimates for embedders

**Sequences API** (libzstd 1.5+ job)

- `generate_sequences` / `compress_sequences`
- External sequence producer hook (optional; needed for some ABI callers)
- Validate-sequences flag

**Advanced compression parameters** — every stable `ZSTD_c_*` is a first-class `CParameter`:

| Param                                                                                         | Role                                 |
| --------------------------------------------------------------------------------------------- | ------------------------------------ |
| `compression_level`                                                                           | −7…22 preset                         |
| `window_log`                                                                                  | 10…31 (64-bit)                       |
| `hash_log`, `chain_log`, `search_log`                                                         | match structures                     |
| `min_match`                                                                                   | 3…7                                  |
| `target_length`                                                                               | strategy-dependent                   |
| `strategy`                                                                                    | 1=`fast` … 9=`btultra2` (see §3.1.1) |
| `target_cblock_size`                                                                          | latency-oriented block split         |
| `enable_ldm` + `ldm_hash_log` / `ldm_min_match` / `ldm_bucket_size_log` / `ldm_hash_rate_log` | `--long`                             |
| `content_size_flag`, `checksum_flag`, `dict_id_flag`                                          | frame header                         |
| `nb_workers`, `job_size`, `overlap_log`                                                       | multi-thread                         |
| `rsyncable`                                                                                   | periodic state sync                  |
| `src_size_hint`                                                                               | streaming param pick                 |
| `literal_compression_mode`                                                                    | force Huffman on/off/auto            |
| `force_max_window`, `force_attach_dict`                                                       | dict attach policy                   |
| `stable_in_buffer` / `stable_out_buffer`                                                      | zero-copy promises                   |
| `max_block_size`                                                                              | cap < 128 KiB                        |

Experimental params are tracked in an appendix and landed when a real caller needs them — not a launch blocker, not silently advertised as done.

#### 3.1.1 Strategies (must all exist)

| #   | Name       | Match finder                              |
| --- | ---------- | ----------------------------------------- |
| 1   | `fast`     | hash table, skip (`target_length`)        |
| 2   | `dfast`    | double fast                               |
| 3   | `greedy`   | hash chain, greedy                        |
| 4   | `lazy`     | hash chain, lazy                          |
| 5   | `lazy2`    | hash chain, lazy²                         |
| 6   | `btlazy2`  | binary tree, lazy²                        |
| 7   | `btopt`    | binary tree, optimal parse                |
| 8   | `btultra`  | binary tree, stronger optimal             |
| 9   | `btultra2` | binary tree, ultra optimal (levels 20–22) |

Level table (−7…22) is a **pinned translation** into these params, checked against C `--show-default-cparams` on a matrix of sizes. Divergence from C’s table is a product decision and must be ledgered; default is **match C’s table**.

### 3.2 CLI — drop-in `zstd`

The CLI is **not** a lagging extra. A 1:1 competitor is what people type.

**Binaries / aliases** (same as facebook/zstd):

| Name                    | Meaning                             |
| ----------------------- | ----------------------------------- |
| `zstd`                  | default                             |
| `zstdmt`                | `zstd -T0`                          |
| `unzstd`                | `zstd -d`                           |
| `zstdcat`               | `zstd -dcf`                         |
| `zstdgrep` / `zstdless` | thin wrappers, later but on the map |

Crate/binary naming: package `rusty_zstd`; installed CLI names above. A `rzstd` alias is fine; `zstd` is the product name on PATH.

**Operation modes**

| Flag                                                                 | Job                                 |
| -------------------------------------------------------------------- | ----------------------------------- |
| `-z` / `--compress`                                                  | compress (default)                  |
| `-d` / `--decompress`                                                | decompress                          |
| `-t` / `--test`                                                      | integrity test (decompress to sink) |
| `-b#` / `-e#` / `-i#`                                                | in-process benchmark (see §6)       |
| `--train` / `--train-cover` / `--train-fastcover` / `--train-legacy` | dictionary builder                  |
| `-l` / `--list`                                                      | frame info                          |

**Modifiers (launch-complete)**

`-#`, `--ultra`, `--max`, `--fast[=#]`, `-T#` / `--threads`, `--single-thread`, `--auto-threads={physical,logical}`, `--adapt[=min,max]`, `--long[=#]`, `-D`, `--patch-from`, `--rsyncable`, `-C` / `--[no-]check`, `--[no-]content-size`, `--no-dictID`, `-M` / `--memory`, `--stream-size`, `--size-hint`, `--target-compressed-block-size`, `-f`, `-c` / `--stdout`, `-o`, `--[no-]sparse`, `--[no-]pass-through`, `--rm`, `-k` / `--keep`, `-r`, `--filelist`, `--output-dir-flat`, `--output-dir-mirror`, `--exclude-compressed`, `--no-progress`, `--show-default-cparams`, `-v` / `-q`, `-h` / `-H`, `-V`, `--zstd=wlog=…`, `--jobsize`, `--seekable`.

**Environment:** `ZSTD_CLEVEL` (1–19 default override), `ZSTD_NBTHREADS`. CLI flags win.

**gzip-like defaults:** preserve sources unless `--rm`; refuse compressed I/O on a terminal unless `-f`/`-c`; `.zst` suffix; progress on tty unless `-q`.

**`--format=`:** facebook/zstd optionally links zlib/lzma/lz4. We do **not** pull those C libs. Adapters are allowed only onto **house codecs** if/when they exist. Until then `--format=zstd` is the only supported value; other names error with a clear “not in this build”. This is a documented delta, not a silent gap.

**Glyphs:** all CLI/status/progress characters go through [`thoth`](https://github.com/Remade-With-Rust/thoth). No raw Unicode in `.rs`.

### 3.3 Dictionary trainer

Launch-complete, because small-message compression is why zstd displaced gzip in RPC:

- `--train` = `--train-fastcover=d=8,steps=4` (match C default)
- `--train-cover` with `k,d,steps,split,shrink`
- `--train-fastcover` with `k,d,f,steps,split,accel,shrink`
- `--train-legacy` (selectivity) — keep for C dictionary compatibility
- `--maxdict` default 112640, `--dictID`, `-r`, `-M` sample cap (default/max 2 GiB), deterministic subsample when over cap
- Train-time `-#` to bias stats toward a level

Gate: dictionaries we train must **round-trip in C** (C compress-with-our-dict + C decompress; our compress-with-C-trained-dict + C decompress). Ratio vs C-trained dicts on a held-out small-file corpus is a ledger metric, not a vanity number.

### 3.4 Multi-thread

Maps to `ZSTD_c_nbWorkers`:

- `-T0` = physical cores (or logical if `--auto-threads=logical`), cap 256 (64-bit)
- `-T1` = one compression thread **plus** I/O thread (C’s distinction)
- `--single-thread` = serialized I/O+compress, lower RSS, **different bitstream** than `-T1` (document and test both)
- `job_size` default ~`4 * windowSize`; min max(512 KiB, overlap)
- `overlap_log` 0=default, 1=independent jobs, 9=full window overlap
- Threaded decompress is **not** a C default; do not invent it as a silent behavior change. Optional later, flag-gated.

`--adapt` is launch-complete but **non-reproducible by design** (match C). Tests pin `--adapt` off.

### 3.5 Optional C ABI (`rusty_zstd-capi`)

A `cdylib` / `staticlib` exporting the **stable** libzstd symbols so a C program can relink:

`ZSTD_compress`, `ZSTD_decompress`, `ZSTD_compressBound`, `ZSTD_isError`, `ZSTD_getErrorName`, `ZSTD_CCtx_*`, `ZSTD_DCtx_*`, `ZSTD_compress2`, `ZSTD_compressStream2`, `ZSTD_decompressStream`, `ZSTD_CDict` / `ZSTD_DDict` family, `ZSTD_getFrameContentSize`, `ZSTD_findFrameCompressedSize`, version helpers.

- Error model: C `size_t` error codes, same `ZSTD_ErrorCode` numbering where it is documented as stable
- Soname / crate feature `capi`; **off by default** in the Rust crate
- `#[global_allocator]` is **never** in the library (house law). The consuming binary chooses. Our CLI uses the `mata-alloc` / `rusty_alloc` seam
- ABI is a **launch-minus-one** milestone: the Rust API ships first; C ABI is the embedder-replacement story and must exist before we call this a libzstd competitor in C ecosystems

### 3.6 no_std / wasm

Core decode + one-shot encode: `no_std` + `alloc`. Streaming I/O and CLI need `std`. `wasm32-unknown-unknown` is a standing `cargo check` target for the library. Threads / LDM-huge-windows are `std` features.

---

## 4. Architecture

Independent stages. Embedders call any stage; the engine composes them. No stage knows its neighbors.

```
rusty_zstd/
├── crates/
│   ├── rusty_zstd/          # public Rust API (lib)
│   ├── rusty_zstd-cli/      # zstd / unzstd / zstdcat / zstdmt
│   ├── rusty_zstd-capi/     # optional C ABI cdylib
│   └── rusty_zstd-bench/    # harness; shells out to pinned C zstd; never links libzstd
├── bench/
│   └── ledger.jsonl         # append-only
├── corpora/                 # hashes + fetch script; not necessarily the blobs
└── docs/plans/              # this file
```

Internal modules of `rusty_zstd` (may later split crates if compile times demand it):

```
src/
├── frame/        header parse/write, multi-frame, skippable, checksum
├── block/        raw / RLE / compressed dispatch
├── literals/     Huffman encode + decode (1-stream + 4-stream)
├── sequences/    FSE tables, sequence (de)code, repeat offsets
├── match/        strategies 1–9 (hash, chain, binary tree, optimal)
├── ldm/          long distance matching
├── entropy/      shared FSE + Huffman primitives
├── dict/         load raw + trained; CDict/DDict
├── train/        COVER / fastcover / legacy
├── stream/       streaming compress + decompress
├── mt/           job split, overlap, worker pool
├── seekable/     seek table in skippable frame
├── params/       level table + CParameter
└── engine.rs     Compress / Decompress
```

**Contracts**

- Stages consume/produce byte slices, frames, or small internal structs
- Every stage has oracle tests against C (or bit-exact fixtures derived from C traces)
- Dictionary, streaming, MT, seekable, legacy are **additive layers**, not entangled with the block codec
- Scalar path is the oracle; SIMD is `#[target_feature]` + runtime detect + scalar tail (`codec-vectorize-kernel`)

**Dependencies (library)**

- Pure Rust only. XXH64: a pure-Rust implementation (no `xxhash-sys`)
- No `zstd-sys`, no `libzstd-rs`, no `ruzstd` in the core path (ruzstd may be a *comparison arm* in the bench crate)
- `cargo deny`: no copyleft, no `*-sys`, license allowlist
- CLI may depend on `thoth`, `rusty_alloc` **only in the bin crate** via the allocator seam

---

## 5. Oracles and references

Pinned in the ledger; never “whatever is on PATH”.

| Role               | Stack                                                                                  | Why                                    |
| ------------------ | -------------------------------------------------------------------------------------- | -------------------------------------- |
| **Primary bar**    | facebook/zstd CLI + libzstd, **pinned tag** (start: latest stable 1.5.x, record exact) | Ratio, speed, correctness, CLI goldens |
| **Format**         | RFC 8878                                                                               | Legal frames                           |
| **Cross-decode**   | our compress → C decompress; C compress → our decompress                               | Interop                                |
| **Peer (not bar)** | ruzstd / other pure-Rust                                                               | Frontier check; never the gate         |
| **House**          | remade_ffmpeg_rs consumers, SpaceDB                                                    | Integration                            |

**How we invoke C:** the bench crate **shells out** to a pinned `zstd` binary (and optionally a pinned `unzstd`). We do **not** link libzstd. That keeps the “zero C in the tree” claim honest. Same pattern as other Remade-With-Rust codecs vs FFmpeg.

**Decoder bring-up** follows `codec-bringup-decoder`: instrument C (or dump Huffman/FSE state from a debug C build) so a divergence is a *symbol*, not a “wrong output somewhere”. For zstd the entropy state is FSE + Huffman, not `dif/rng/cnt` — same discipline, different registers. Fixtures live in-tree; the C binary is ephemeral.

**Encoder bring-up** follows `codec-bringup-encoder`: `decode(encode(x))` through **our** decoder is a standing gate, **and** C libzstd must decode to the same bytes. Self-consistent is not legal.

---

## 6. Analyzer, harness, ledger

`codec-measurement` is in force for every number. `codec-analyzer` is how we instrument. A number without a method line is not evidence.

### 6.1 Corpora

Pinned hashes, train/holdout split. Holdout is the only exit-gate data.

| Corpus                                                                              | Why                                                                          |
| ----------------------------------------------------------------------------------- | ---------------------------------------------------------------------------- |
| Silesia                                                                             | Standard lossless bar                                                        |
| Calgary / Canterbury                                                                | Small, classic                                                               |
| enwik8 / enwik9 excerpts                                                            | Text, large-window                                                           |
| Synthetic: incompressible (CSPRNG), highly compressible (zeros, repeats), JSON/logs | Path coverage (raw vs RLE vs compressed; Huffman vs incompressible literals) |
| Small-file set (many 1–16 KiB messages)                                             | Dictionary payoff                                                            |
| Binary / object code / tar-of-versions                                              | `--long` / LDM                                                               |
| SpaceDB-shaped: CRDT deltas, snapshots, CBOR/JSON logs                              | Product corpus                                                               |
| Media containers / model blobs                                                      | remade_ffmpeg_rs / FFAI                                                      |

### 6.2 Metrics (every milestone)

| Gate            | Pass means                                                                                                           |
| --------------- | -------------------------------------------------------------------------------------------------------------------- |
| **Correctness** | Bit-exact reconstructed bytes; C↔us both directions; fuzz (untrusted frames must not panic / OOB); window-cap reject |
| **Ratio**       | Compressed size vs original; vs C at **matched level** and vs C at **matched speed** (two curves)                    |
| **Speed**       | MB/s compress + decompress; CPU-time and wall; cores-busy printed; warm and end-to-end                               |
| **Footprint**   | Peak RSS, workspace estimates, binary size, `cargo tree` / deny                                                      |

Harness shape (port from rusty_h264 `pinvs`/`pinmt`): pinned process, High priority, ABBA arms, null arm per session, N ≥ 20 same-binary / N ≥ 31 cross-binary, refuse sub-timer-resolution, pair-1 standing-floor abort. Method line printed every run.

CLI `-b` uses the **same in-process path** as the harness (not a second timer). One compliant timer; everything else calls it.

### 6.3 Ledger

`bench/ledger.jsonl`, append-only. Fields: git SHA, corpus fingerprint, C zstd version, our version, env (CPU, OS), method line, work counts, gates (correctness / ratio / speed / footprint), verdict, notes. A skipped gate is never a pass.

Public README claims must cite a ledger line.

---

## 7. Performance targets

Standing targets. Revisit only with a ledger argument.

| Regime                                  | Target vs pinned C libzstd                                                                             |
| --------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| **Decompress, default window, Silesia** | ≥ 0.90× C (M1 exit may be 0.33–0.50×; M5 campaign to ≥ 0.90×; stretch 1.00×)                           |
| **Compress level 1, matched ratio**     | ≤ 2× slower at M2; ≤ 1.25× at M5; stretch 1.00×                                                        |
| **Compress level 3 (default)**          | ≤ 2× at M3; ≤ 1.25× at M5                                                                              |
| **Compress levels 19 / 22**             | Ratio within 3% of C at that level; speed is stretch (optimal parse is the C fortress)                 |
| **`--fast` / negative levels**          | Ratio within 5% of C; speed within 1.5× at M5                                                          |
| **`-T0` on large files**                | Scale similarly to C (cores-busy, not just wall); no nested-thread thrash vs `--single-thread`         |
| **RSS**                                 | Within 1.2× of C at the same windowLog / nbWorkers                                                     |
| **CLI binary**                          | No C runtime; size competitive with a mimalloc-linked C zstd is *not* the gate — correctness + deny is |

Lossless ⇒ **no** `codec-tune-quality` / BD-rate. Ratio is a Pareto metric against C, not a perceptual score. `codec-experimental` only if we deliberately change *our* match/entropy math while staying RFC-legal; still C-cross-decode gated.

---

## 8. Milestones and exit gates

Every milestone starts with `--baseline-only` against pinned C (or a stored baseline) and ends with a full comparison on **holdout**. A skipped gate blocks exit.

| #      | Deliverable                                                             | Exit gate                                                                                                                                                                                                                                      |
| ------ | ----------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **M0** | Repo + harness + pinned C + corpora hashes                              | C numbers on the board; ledger exists; `cargo deny` wired; no codec yet required                                                                                                                                                               |
| **M1** | Decompressor core                                                       | All RFC 8878 block types; skippable; multi-frame; checksum; C compress → us decompress bit-exact on holdout; fuzz 24h no crash; speed within ~2–3× of C                                                                                        |
| **M2** | Compressor core, fast levels (−7…3), strategies `fast`/`dfast`/`greedy` | Round-trip us+C; ratio vs C at −7…3 quantified; simple + streaming APIs; CLI `-z/-d/-t/-c/-o`                                                                                                                                                  |
| **M3** | Full level range 4…22, all 9 strategies, Huffman/FSE encode parity      | Ratio within small band of C at matched level on Silesia; `--show-default-cparams` matches C table (or ledgered deltas); CLI `-# --ultra --fast --zstd=`                                                                                       |
| **M4** | Dictionaries + trainer + prefix                                         | Raw + trained dict compress/decompress both directions with C; COVER/fastcover; `--patch-from`; small-file corpus ratio vs C                                                                                                                   |
| **M5** | Long / LDM / rsyncable / target block size / seekable                   | `--long`, `--rsyncable`, `--target-compressed-block-size`, seekable round-trip; C interop                                                                                                                                                      |
| **M6** | Multi-thread + CLI completeness                                         | `-T#`, `--single-thread`, `--jobsize`, overlap; aliases; env vars; `-l --train -b -r --rm` file semantics; goldens vs C CLI on a flag matrix                                                                                                   |
| **M7** | Performance campaign                                                    | Profile-driven bricks (`codec-optimize`: SIMD/NEON → Wiring Completness → Silent Zeros unknown failings → redundancy → memory-copies → vectorize → Great Gate functions). Hit §7 decompress ≥0.90× and compress L1/L3 ≤1.25× or ledger why not |
| **M8** | C ABI + integrations + stable                                           | `capi` symbols; SpaceDB + remade_ffmpeg_rs optional feature; docs; every README claim maps to a ledger line; crates.io                                                                                                                         |

**Sequencing:** decompress first (M1). Compression overlaps once the block format is solid. Trainer can prototype against M1 using C as the compressor. MT never before single-thread bit-correct.

**M1 brick 1 (landed):** frame header, raw + RLE blocks, skippable, multi-frame, XXH64 checksum.
**M3 (landed):** Full level range −7…22 and all 9 strategies; Huffman literals (1/4-stream, raw+FSE tree, treeless) and FSE sequence modes (predefined / RLE / compressed / repeat); CLI `--ultra --fast --zstd= --show-default-cparams`. Dual gate (us decode + C `zstd -d`).
**M4 (landed):** Raw + trained dictionaries both directions with C; COVER / fastcover / legacy trainer; CLI `-D --patch-from --train --maxdict --dictID --no-dictID`. Dual gate (us compress-with-dict → C decompress-with-dict and the reverse).
**M5 (landed):** `--long` / LDM, `--rsyncable`, `--target-compressed-block-size`, seekable independent frames + skippable seek table. Dual gate (us compress → us decode + C `zstd -d`; C `--long` / `--rsyncable` → us decode).
**M6 (landed):** `-T#` / `--threads` / `--single-thread` / `--jobsize` / `--overlap-log`; aliases `unzstd` `zstdcat` `zstdmt`; env `ZSTD_CLEVEL` `ZSTD_NBTHREADS`; `-l` `-b` `-r` `--rm`. Dual gate (us MT → C `-d`; C `-T2` → us decode).
**M7 (spine):** one in-process timer (`rusty_zstd::bench_roundtrip` / CLI `-b`); feature-gated stage profiler; `rzstd-bench --m7-speed` vs C 1.5.7 **only** at `zstd -1`, `zstd --fast=1`, `zstd --fast=4` (ABBA, pin affinity=4 High, dual-gate). Windows C rejects glued `-b-1`; harness uses `--fast=N -b`. Not a §7 exit. `--m7-profile` dumps stages + `hash_fills` (`--features profile`).
**M7 brick 1 (landed):** `find_fast` no longer hashes every byte of a found match. C `zstd_fast.c` inserts `hash(start+2)` and `hash(end-2)` only. Profile on text-32m L1: `EncodeMatchFind` 97% / `hash_fills` 33,552,327 → 85% / 256. Standing compress text-32m `-1`: 275 → 1175 MB/s (C/us 36.7 → 8.86). Dual-gate held. Ratio us/c 1.005 → 1.009.
**M7 bricks 2–6 (landed):** `count_match` u64 words (C `ZSTD_count`); decode `copy_match` via `extend_from_within` (C wildcopy); hash slot 0 is empty (`pos+1` sentinel); `find_dfast` 2-slot fill; `find_fast` stepSize = C `targetLength + !targetLength + 1`; streaming emit: no per-block scratch/reset, `in_acc` cursor instead of `drain(..take).collect()`. Dual-gate held.
**M7 bricks 7–9 (landed):** skip Huffman encode+verify-decode on incompressible literals (sample `max*8 < n`); empty-seq blocks that fail the sample emit raw with no entropy clone; RLE decode `Vec::resize`; word-sized `rle_byte`. Dual-gate held.

**M7 brick 10 (landed):** MatchFind `count_match` slices both windows once, CRT-memcmp's a full equal run, then 4-wide u64 + byte tail on the first difference (C `ZSTD_count` + ILP). `find_fast` probes 4 bytes (`MEM_read32`) and counts from +4; `hash_mls` loads u32. Profile text-32m L1: `EncodeMatchFind` 6.86 ms → 2.96 ms (`hash_fills` still 256). Tried C `step++` every 128 without the ip0/ip1 pair pipeline: `--fast=4` ratio 0.845 → 1.272, reverted. Dual-gate held. Ratio us/c 1.022 / 0.845.

**M7 brick 11 (landed):** `find_fast` extends matches backwards (C `_offset`) and pair-searches `ip+1` only when `step > 2` (`--fast=4`). A full C 4-pointer `step++` pipeline doubled incomp probes (C/us 1.30 → 1.70). Dual-gate held. `--fast=4` ratio 0.845 → **0.818**. `-1` ratio still 1.022.

**M7 brick 12 (landed):** xxh64 oneshot walks `chunks_exact(128)` so the inner loop is `movq/imul/rol` with no per-load bounds checks (the indexed unroll emitted 16 `cmp+ja` per stripe in the `.s`). Byte-identical vs published XXH64 vectors + hasher. No packed ops — 64-bit `imul` does not auto-vec; SIMD would need a different hash (not C's content checksum). Dual-gate held.

**M7 brick 13 (landed):** AVX2 `count_eq_len` (`pcmpeqb` + movemask + `trailing_ones`, 64-byte ILP) with u64-word scalar fallback and NEON mirror. Named reason auto-vec fails: early-exit first-difference; CRT `memcmp` of a whole run then a second scan to find the index. `count_eq_len_matches_byte_and_words` gates SIMD == words == byte loop. Offset-1 `copy_from_decoded` is a byte splat (`Vec::resize`) instead of doubling 1-byte `memcpy`. Dual-gate held. Ratio 1.022 / 0.818.

Standing 32 MiB after brick 13 (`kind=m7_speed`, `i=3`, null-arm 1.014, cores-busy 0.96–0.99, all `rt` + C `-d` true). C/us > 1 means C is faster:

| corpus     | flag       | C compress | us compress |   C/us c | C decomp | us decomp |   C/us d |
| ---------- | ---------- | ---------: | ----------: | -------: | -------: | --------: | -------: |
| zeros-32m  | `-1`       |       4872 |        4403 | **1.11** |    16566 |      5166 |     3.21 |
| zeros-32m  | `--fast=1` |       5147 |        4036 |     1.28 |    14403 |      5005 |     2.88 |
| zeros-32m  | `--fast=4` |       5234 |        3617 |     1.45 |    16185 |      4755 |     3.40 |
| text-32m   | `-1`       |       6765 |    **4453** | **1.52** |     6320 |      4891 | **1.29** |
| text-32m   | `--fast=1` |       7091 |        4160 | **1.70** |     6418 |      4728 |     1.36 |
| text-32m   | `--fast=4` |       7288 |        4334 | **1.68** |     7931 |      4869 |     1.63 |
| incomp-32m | `-1`       |       2661 |        2466 | **1.08** |     6549 |      3953 |     1.66 |
| incomp-32m | `--fast=1` |       3201 |        2339 |     1.37 |     6616 |      3939 |     1.68 |
| incomp-32m | `--fast=4` |       3182 |        2000 |     1.59 |     6541 |      3848 |     1.70 |

**M7 brick 14 (landed):** Oneshot checksum is incremental `Xxh64::update` per block (same walk as RLE/match/raw), not a second cold 32 MiB `xxh64(src)` after MatchFind. Byte-identical vs oneshot XXH64 (`frame_checksum_matches_oneshot_xxh64`). Dual-gate held on generated. Counts: text match_frac=1.0 / 256 vs 256 C blocks; unused Fast tables 98 KiB and 768 scratch allocs pruned on arithmetic. Silesia dual-gate: x-ray OK; other 11 files fail C `-t` (pre-existing entropy tail, not this brick). Descent: `docs/plans/m7-encoder-whys.md`.

Standing 32 MiB after brick 14 (`kind=m7_speed`, `i=3`, **null-arm 1.125**, cores-busy 0.96–0.99). Headline **us-vs-us** (C drifted). C/us > 1 means C is faster:

| corpus     | flag       | us c before | us c after |   C/us c | us/c size |
| ---------- | ---------- | ----------: | ---------: | -------: | --------: |
| zeros-32m  | `-1`       |        4403 |   **5664** |     1.28 |     0.901 |
| zeros-32m  | `--fast=4` |        3617 |   **6021** |     1.07 |     0.899 |
| text-32m   | `-1`       |        4453 |   **5085** |     1.61 |     1.022 |
| text-32m   | `--fast=4` |        4334 |   **5530** |     1.46 |     0.818 |
| incomp-32m | `-1`       |        2466 |       2748 | **0.99** |     1.000 |
| incomp-32m | `--fast=4` |        2000 |       2014 |     1.62 |     1.000 |

No content class lost. Silesia entropy Repeat fix landed after this brick (see below). Generated text is not the Silesia board.

**Silesia re-board (after Repeat FSE zero-prob reject + last-symbol decrement).** Dual-gate **12/12 × 3 flags** (`rt` + C `-d`). `--m7-speed` now ABBA-benches Silesia per file. Session **null-arm 1.158**, cores-busy 0.96–0.99. C/us > 1 means C is faster. **Never average these files.**

Generated continuity (profiler OFF, vs brick 14 us; C drifted again — headline us-vs-us):

| corpus     | flag | us c |   C/us c |   C/us d | us/c size |
| ---------- | ---- | ---: | -------: | -------: | --------: |
| zeros-32m  | `-1` | 5346 |     1.19 |     2.51 |     0.901 |
| text-32m   | `-1` | 5772 | **1.40** | **1.30** |     1.022 |
| incomp-32m | `-1` | 2819 |     1.18 |     1.77 |     1.000 |

Silesia `-1` standing (the §7 corpus). Split is holdout/train as in `list_silesia`:

| file    | split |   C/us c |   C/us d | us/c size | EncodeEntropy % | EncodeMatchFind % |
| ------- | ----- | -------: | -------: | --------: | --------------: | ----------------: |
| mr      | H     | **3.71** |     6.71 |     1.114 |        **62.7** |              35.1 |
| ooffice | H     |     2.48 |     2.53 |     1.262 |            40.2 |          **57.9** |
| osdb    | H     |     3.11 |     4.83 |     1.144 |        **60.0** |              38.4 |
| reymont | H     |     2.86 |     5.60 | **1.437** |            48.3 |              50.3 |
| sao     | H     | **1.08** | **0.49** |     1.143 |            11.5 |          **84.6** |
| webster | H     |     3.02 |     4.74 | **1.436** |            41.9 |              56.6 |
| dickens | T     |     3.18 |     8.43 |     1.215 |            47.6 |              51.2 |
| mozilla | T     | **4.70** |     5.26 |     1.222 |        **56.9** |              40.9 |
| nci     | T     |     3.78 |     4.63 |     1.303 |        **55.5** |              41.3 |
| samba   | T     |     2.91 |     4.55 |     1.255 |            47.2 |              51.0 |
| xml     | T     |     3.65 |     4.77 |     1.308 |        **52.8** |              45.0 |
| x-ray   | T     | **0.90** |     1.12 |     1.221 |        **64.3** |              21.6 |

`--fast=4` ratio often **beats** C (nci 0.720, dickens 0.766, mr 0.776) while `-1` loses 11–44%. x-ray `--fast=1`: C dumps 63 raw + 2 comp (2852 MB/s); we still Huffman 34 blocks (C/us **9.81**). Generated text entropy was **4.6%** — it hid the Silesia machine.

That per-file sign-flip is a **dispatch**, not a mean. Local Great Gate tools (gitignored): `_greatgate/` — calculator + `zstd-great-gate.md` census. Harvest: `rzstd-bench --features profile -- --m7-harvest` (per-block CSV; Silesia only). Do not average Silesia; do not force-on Raw.

**M7 brick 15 (landed):** Early noCompress on `--fast` only. After `find_sequences`, if `match_bytes < ZSTD_minGain` (`(src>>6)+2`), write Raw and skip Huffman/FSE. Default on iff strategy Fast and `target_length` in 1..=7 (`--fast=N`). `RZSTD_INCOMP_SKIP=0` is the old `payload >= len` path (`skip_off_l1_bytes_match_unset`). Dual-gate **12/12 × 3 flags**. Session **null-arm 1.125**.

Canaries (profiler OFF, us-vs-us vs the Huffman-then-dump board):

| file  | flag       | before C/us c | after C/us c | after us c |                       us/c size |
| ----- | ---------- | ------------: | -----------: | ---------: | ------------------------------: |
| x-ray | `--fast=1` |      **9.81** |     **1.35** |       1563 |               1.001 (was 0.939) |
| x-ray | `--fast=4` |     **15.85** |     **2.34** |       1151 |               1.000 (was 0.923) |
| x-ray | `-1`       |          0.90 |         0.98 |        545 | **1.221** (unchanged; skip off) |
| nci   | `-1`       |          3.78 |         3.72 |        176 |                       **1.303** |
| mr    | `-1`       |          3.71 |         4.53 |         67 |                       **1.114** |

x-ray `--fast=1`: 65 raw / 0 Huffman (`early_raw=46`); C still 63 raw + 2 comp. Entropy stage **0%**. nci/mr `-1` size did not move.

**Wave 0 instruments.** `EncodeHuff` / `EncodeTableSelect` / `EncodeFseSeq` split `EncodeEntropy`. `--m7-profile` dumps after decompress (decode stages were 0%). Per-block harvest: 4863 rows; `gain` = seqs (entropy work if skipped); no `c_raw_frac`. Calculator **HYPOTHESES ONLY**.

**Wave 2 (after skip), `-1` encode-only share** (`stage_ms / EncodeTotal`; dump % includes decode so do not compare to the old 62% column):

| file    | EncodeHuff | EncodeFseSeq | EncodeTableSelect | DecodeLiterals / DecodeTotal | DecodeSeq / DecodeTotal |
| ------- | ---------: | -----------: | ----------------: | ---------------------------: | ----------------------: |
| mr      |    **67%** |           1% |              0.6% |                      **92%** |                      6% |
| mozilla |    **41%** |           8% |                1% |                      **61%** |                     37% |
| nci     |        13% |      **20%** |                2% |                          20% |                 **76%** |
| webster |        12% |          12% |                1% |                          29% |                 **69%** |

Huffman emit is the mr/mozilla kernel. FSE seq emit+decode is nci/webster. Table-select clones were not hot; still moved into entropy after emit (no clone). BitCStream stores the container as words; Huffman `with_capacity`; seq flush already batched.

**Brick 16 (Huffman 1X, Rust not asm):** packed `entry[256]` (code+nbits one load), `add_bits_huff` (no mask / no overflow check), flush-then-4-symbols (`MAX_BITS=11` so 4*11+7 leftover fits in 64). Scalar twin `encode_stream_scalar` is the oracle (`encode_stream_unrolled_matches_scalar`). 4-stream lockstep was tried and **reverted** (xml us dropped with C — cache thrash, not ILP). No `std::arch` on this kernel: the pack is a serial bit-container; auto-vec cannot help; the C BMI2 asm equivalent here is the 4-unroll.

Standing `-1` after 16 (null-arm **1.147**, sizes unchanged). Headline us-vs-us vs brick 15; C drifted (xml C 540→440). Canaries: mr **66.6 → 91.6**; mozilla 78→82; nci 176→179. x-ray `-1` **545 → 740** (still Huffmans 11 blocks). Dual-gate 12/12 × 3.

**Wave 4 L3 baseline only** (no greedy rewrite). Same session as brick 15. Compress C/us **2.5–4.0** on every file — x-ray is **2.86** here (not the `-1` win). Different machine.

| file    | split | C/us c | C/us d | us/c size |
| ------- | ----- | -----: | -----: | --------: |
| mr      | H     |   4.00 |   6.76 |     1.052 |
| ooffice | H     |   3.98 |   4.54 |     1.136 |
| osdb    | H     |   2.72 |   3.77 |     1.104 |
| reymont | H     |   2.88 |   5.96 |     1.092 |
| sao     | H     |   2.55 |   2.16 |     1.056 |
| webster | H     |   2.76 |   5.17 |     1.141 |
| dickens | T     |   2.88 |   6.14 |     1.129 |
| mozilla | T     |   2.90 |   3.78 |     1.105 |
| nci     | T     |   2.79 |   3.22 |     1.221 |
| samba   | T     |   2.75 |   4.52 |     1.111 |
| xml     | T     |   2.89 |   3.69 |     1.185 |
| x-ray   | T     |   2.86 |   3.64 |     1.082 |

**Wave 5 decode ranking** (not sao — we already win `-1` decode there). Huffman 4-stream is the mr/mozilla/dickens kernel; FSE seq is nci/webster.

**Bricks 17–19 (FSE emit + Huffman decode + FSE decode, Rust not asm).** Same serial-container rule as brick 16: packed LUTs and unrolls, no NASM / no `std::arch`. 4-stream decode stays sequential (encode lockstep already failed on xml).

- **17 FSE seq emit:** `FseCDelta { nb, find }` AoS (one load per symbol). Repeat 0-prob still uses `+ 0xFFFF` before `>> 16`.
- **18 Huffman decode:** DTable `Vec<u16>` (`sym | nbits<<8`), `look_bits_fast` + `skip_bits` (no second look), reload then 4 symbols. Scalar twin `decode_stream_scalar` (`decode_stream_unrolled_matches_scalar`).
- **19 FSE seq decode:** stash `FseEntry` once; extra bits still between peek and `advance` (C order). Power-of-two mask, no Option on the hot walk.

Lib tests **106/106**. Dual-gate **12/12 × 3**. Sizes unchanged. Standing `--m7-speed --levels 1` (C flag `-1`), profiler OFF, null-arm **1.027**. Headline us-vs-us vs brick 16 (that session's null-arm was 1.147, so encode ±15% is noise). Encode is flat (nci **179 → 186**). Decode is the keep:

| file    | us d before | us d after |    C/us d after | us/c size |
| ------- | ----------: | ---------: | --------------: | --------: |
| mr      |         178 |    **282** | 3.45 (was ~6.7) |     1.114 |
| mozilla |         185 |    **289** |            3.05 |     1.222 |
| nci     |         385 |    **485** |            3.29 |     1.303 |
| webster |         222 |    **288** |            3.52 |     1.436 |
| dickens |         124 |    **204** |            4.52 |     1.215 |
| x-ray   |         877 |   **1326** |        **0.67** |     1.221 |

**Bricks 20–21 (Huffman 5-unroll + 4X decode lockstep).** After 17–19, encode-only share: mr Huff still **56%** of encode; nci/webster DecodeSeq still **80%** of decode; mr DecodeLiterals **86%**. `MAX_BITS=11` so 5×11+7 leftover = 62 < 64 (encode) and 55 < 57 after reload (decode). 4-stream **decode** lockstep is C `HUF_decompress4X1` (four readers, one DTable) — encode lockstep already failed on xml (four writers); xml decode C/us d **3.19 → 3.14** (no sign-flip). Scalar twins unchanged. `decode_4x_matches_sequential`.

Lib tests **107/107**. Dual-gate **12/12 × 3**. Sizes unchanged. Standing `--levels 1`, null-arm **1.016**. C also rose this session (box); same-session C/us d is the cleaner cross-brick signal. Decode keep; encode 5-unroll is a mild mozilla keep.

| file    | us d before | us d after | C/us d before | C/us d after | us/c size |
| ------- | ----------: | ---------: | ------------: | -----------: | --------: |
| mr      |         282 |    **417** |          3.45 |     **2.65** |     1.114 |
| mozilla |         289 |    **486** |          3.05 |     **2.37** |     1.222 |
| nci     |         485 |    **768** |          3.29 |         2.97 |     1.303 |
| webster |         288 |    **357** |          3.52 |         3.01 |     1.436 |
| xml     |         451 |        697 |          3.19 |         3.14 |     1.308 |
| x-ray   |        1326 |   **2455** |      **0.67** |     **0.48** |     1.221 |

**Brick 22 (`find_fast`).** After Huffman, encode-only MatchFind is #1 on ooffice **60%** / webster **57%** / dickens **53%**. `min_match` is clamped 3..=7 so `hash_mls`'s `hash8` arm is dead on Fast. Hoist `hash_shift`, always `hash4`, unaligned LE load in `simd` (`load_u32_le` / `load_u64_le`; `from_le_bytes` oracle). Hash slot stored as `pos+1` (no `Option`). Sizes unchanged.

Lib tests **108/108**. Dual-gate **12/12 × 3**. Standing `--levels 1`, null-arm **1.014**. Same-session C/us c (box still moving):

| file    | us c before | us c after | C/us c before | C/us c after | us/c size |
| ------- | ----------: | ---------: | ------------: | -----------: | --------: |
| ooffice |         108 |    **144** |          2.89 |     **2.34** |     1.262 |
| dickens |          82 |     **94** |          3.12 |     **2.84** |     1.215 |
| webster |          97 |        112 |          2.77 |         2.69 |     1.436 |
| sao     |         269 |    **292** |          1.18 |     **0.94** |     1.143 |
| nci     |         274 |        275 |          3.07 |         3.01 |     1.303 |

incomp C/us c 1.11 → 1.13 (probe-only; no loss). Next residue: seq copy / FSE extras on nci decode; remaining MatchFind is the count/hash table walk.

**Brick 23 (reverted — three residue hammers, none kept).** DecodeSeq ~80% of nci/webster decode is not another DTable walk. Tried in this session (null-arms 1.063 / 0.931 / 0.994 / 1.027; box slower than brick 22's C nci d 2259):

1. **Seq-loop restructure** (`copy_match_in_frame`, Repeat `take` not clone, skip zero-width extras, then a capturing closure / `decode_seq_loop<const IN_FRAME>`). Dual-gate held. nci C/us d **2.96 → 3.30** (closure) → **3.63** (const-generic). The restore-tables wrapper stopped the seq loop from inlining. Reverted.
2. **Overlapping period splat** (offset 2/4/8 fill vs doubling `extend_from_within`). Oracle `copy_from_decoded_matches_byte_push` held. nci C/us d **3.16** — no keep vs brick 22. Reverted (doubling stays; test lens widened).
3. **Reuse hashed `u32` in `fast_probe`** (don't reload `src[ip..ip+4]`). ooffice C/us c **2.34 → 2.71**; sao **0.94 → 1.06** (lost the encode win). Extra arg looks like register spill on the probe. Reverted.

**Brick 24 (SIMD/BMI2 campaign — two small keeps, the hammers reverted).** User authorized AVX2/NEON/BMI2 intrinsics, not NASM. Dual-gate **12/12** Silesia L1 on every attempt. Sizes unchanged. Lib tests **109/109** (`look_n_bits_bmi2_matches_shift` added; wildcopy oracle removed with the revert).

Reverted (do not retry these shapes):

1. **Huffman BMI2 peek.** C `BIT_lookBitsFast` is `_bextr_u64`. Per-symbol `has_bmi2()` dispatch: mr C/us d **2.65 → 4.50** (us d ~417 → 234). Stream-level `#[target_feature(enable = "bmi2")]` loop so bextr can inline: recovered to **3.49** (us d 276), still a loss. Inlined bextr via macro (rustc forbids `#[inline(always)]` + `#[target_feature]`): still a loss. Named reason: C compiles the whole HUF object with BMI2; our BMI2 loop is an ISA island called from SSE2-baseline decompress. Shift wins on this CPU.
2. **AVX2/NEON `overlap_wildcopy`.** Offset ≥8 unaligned 8/32-byte copies vs doubling `extend_from_within`. Oracle held. nci DecodeSeq canary did not move (C/us d ~3.00 vs brick 22 **2.97**). Reverted.

Kept:

1. **C-formula `look_n_bits_shift`:** `(container << consumed) >> (64-n)` — no extra mask. This is C's non-BMI2 `BIT_lookBitsFast`. Huffman/FSE both use it via `BitRev::look_bits_fast`.
2. **AVX2/NEON `count_eq_len` u64 remainder** after the 32/16-byte packed loop (the scalar twin already did this). Completes the kernel for 8–31 byte matches.

Standing `--levels 1` after the keeps (null-arm **0.976**, cores-busy 0.97–0.99). Headline us-vs-us vs brick 22 ledger (C drifted; nci C d 1621 → 2152 this session):

| file    | us d (b22 ledger) | us d now | C/us d now | us/c size |
| ------- | ----------------: | -------: | ---------: | --------: |
| mr      |               536 |      377 |       2.67 |     1.114 |
| mozilla |               339 |  **391** |   **2.43** |     1.222 |
| nci     |               532 |  **684** |       3.15 |     1.303 |
| xml     |               634 |      611 |       3.21 |     1.308 |
| ooffice |               536 |      508 |   **1.48** |     1.262 |
| dickens |               233 |      244 |       4.03 |     1.215 |

No breakthrough vs C on Huffman-heavy decode (mr/dickens). Remaining gap is still C's BMI2-compiled HUF + DecodeSeq extras/copy, not another DTable walk or a runtime-dispatched bextr. Do not wrap `decode_sequences` in a closure. Do not add `fast_probe` args. Do not runtime-dispatch BMI2 per symbol or per stream.

Lib tests **109/109**. Dual-gate **12/12** Silesia L1. Sizes unchanged throughout.

**Brick 25 (Huffman X2 — C `HUF_decompress4X2` + `HUF_selectDecoder`).** C is not faster because it is C. One peek can emit two symbols (`HUF_DEltX2`: `seq16 | nbits<<16 | length<<24`). `codec-eliminate-redundancy` move 2: a bigger table, byte-identical decode. X1 `decode_stream_scalar` stays the oracle (`decode_stream_unrolled_matches_scalar`). 4-stream X2 uses independent cursors (`decode_4x_matches_sequential`). Compose X2 from X1 (leftover bits of a `table_log` peek index the second symbol when `n1+n2 <= table_log`). Last byte of an X2 stream uses X1 (`decode_one`) so a length-2 entry cannot skip the second symbol's bits.

C `HUF_selectDecoder` (`algoTime[Q][X1,X2]`, Q = compressed/raw in 16ths). We already built both tables, so **tableTime is sunk** — scoring decode256 only, plus C's 1/32 X2 cache penalty. `dst < 256` stays X1 (1-stream). Q=15 (incompressible) stays X1.

Tried and not kept as the default (do not retry these shapes):

1. **Always X2** (no select). Clean session null-arm **1.001**: mr us d 377→433, xml 611→708, dickens 244→283; **nci 684→594** (C/us d **3.15→3.59**). Sign-flip on the seq-heavy file. 1-stream X2 on long codes is a slower X1 plus an extra store.
2. **4-stream X2, 1-stream always X1, no select.** Session null-arm **1.16** (inadmissible). Not the keep.
3. **Select including tableTime** (verbatim C). Picks X1 too often because C builds only one table. xml C/us d 3.21→3.89 in a wide session.

Standing `--levels 1` after the keep (null-arm **1.018**, cores-busy 0.95–1.00 except osdb 0.83 — ignore osdb). Same-session C/us d vs brick 24. Sizes unchanged. Dual-gate **12/12**. Lib tests **110/110** (`select_x2_follows_c_breakpoints`).

| file    | us d (b24) | us d now | C/us d (b24) | C/us d now | us/c size |
| ------- | ---------: | -------: | -----------: | ---------: | --------: |
| mr      |        377 |  **413** |         2.67 |   **2.45** |     1.114 |
| mozilla |        391 |      340 |         2.43 |       2.53 |     1.222 |
| nci     |        684 |      549 |         3.15 |   **3.04** |     1.303 |
| xml     |        611 |      481 |         3.21 |   **3.01** |     1.308 |
| ooffice |        508 |      547 |         1.48 |       1.51 |     1.262 |
| dickens |        244 |  **280** |         4.03 |   **3.27** |     1.215 |

mr C d this session **1013** (b24 implied ~1006) — C did not drift on the Huffman canary; the 2.67→2.45 is a real X2 keep. nci C d 2152→1670 (box cooler); C/us still improved and did not sign-flip. xml did not sign-flip. Remaining Huffman gap vs C is still C's BMI2-compiled HUF object, not another DTable shape. Do not always-X2. Do not put tableTime back into the select.

**Brick 26 (C fast 4X2 loop + DTable upsample to 11).** Opened the rest of HUF, not another peek tweak. C's remaining decode edge is `HUF_decompress4X2_usingDTable_internal_fast_c_loop`: bits stay left-justified, peek is `bits >> 53` (one shift), reload is `trailing_zeros` + unaligned load. Requires `tableLog == 11`. We now stretch every X1 DTable to 11 (nbits in each entry stay native; encode codes unchanged) then compose X2, so the fast loop is always eligible on 4-stream ≥8-byte bodies. Remainder still X2 via `BitRev::from_window` (CTZ → `bits_consumed`). Not BMI2 (brick 24 already lost that island). Not NASM.

Oracle: `decode_4x_matches_sequential` (fast vs four X1 `decode_stream`). Dual-gate **12/12**. Lib tests **110/110**. Sizes unchanged.

Standing `--levels 1` (null-arm **1.034**, cores-busy 0.97–1.00 except text-32m 0.71 / samba 0.84 — ignore those encode lines). Same-session C/us d vs brick 25. mr C d **921** (b25 **1013**) — box a bit cooler, us still jumped.

| file    | us d (b25) | us d now | C/us d (b25) | C/us d now | us/c size |
| ------- | ---------: | -------: | -----------: | ---------: | --------: |
| mr      |        413 |  **617** |         2.45 |   **1.49** |     1.114 |
| mozilla |        340 |      372 |         2.53 |      1.20* |     1.222 |
| nci     |        549 |      528 |         3.04 |       2.85 |     1.303 |
| xml     |        481 |      488 |         3.01 |       2.80 |     1.308 |
| ooffice |        547 |      547 |         1.51 |       1.27 |     1.262 |
| dickens |        280 |      267 |         3.27 |       3.27 |     1.215 |
| x-ray   |          — | **2038** |            — |   **0.44** |     1.221 |

\*mozilla C d this session **448** (typical ~900–1300) — C-side stall; do not headline that 1.20. mr us-vs-us **+49%** is the keep. nci/xml did not sign-flip. dickens did not move (likely 1-stream / short 4-stream, never enters the fast loop). Still not 15% faster than C (need C/us d ≤ 0.87); mr is 49% behind, down from 145%. Next: left-justify `BitRev` itself (same peek as the fast loop, for 1-stream + FSE + remainder). Do not retry BMI2 HUF. Do not NASM.

**Brick 27 (left-justify `BitRev`).** The fast 4X2 loop already peeks with `bits >> 53`. The rest of Huffman (1-stream, 4X2 remainder) and FSE still did C's two-shift `BIT_lookBitsFast`: `(container << consumed) >> (64-n)` every peek. Hoist the left-shift into `new` / `reload` / `skip_bits` so `look_bits_fast` is one shift. `look_n_bits_shift` stays as the formula oracle (`left_justified_look_matches_c_shift`). Not BMI2.

Oracle: Huffman scalar + 4X2 sequential, FSE table roundtrips, `look_bits_fast_zero_pads_at_start`. Dual-gate **12/12**. Lib tests **111/111**. Sizes unchanged.

Standing `--levels 1` (null-arm **1.075** — treat ≲8% as noise). Canary is **dickens** (1-stream; brick 26 left it flat). mr should hold (already in the fast loop).

| file    | us d (b26) | us d now | C/us d (b26) | C/us d now | us/c size |
| ------- | ---------: | -------: | -----------: | ---------: | --------: |
| mr      |        617 |      628 |         1.49 |       1.53 |     1.114 |
| dickens |        267 |  **302** |         3.27 |   **2.95** |     1.215 |
| nci     |        528 |      479 |         2.85 |       3.11 |     1.303 |
| xml     |        488 |      470 |         2.80 |       3.01 |     1.308 |
| mozilla |       372* |      331 |        1.20* |       2.42 |     1.222 |
| ooffice |        547 |      524 |         1.27 |       1.36 |     1.262 |
| x-ray   |       2038 | **2133** |         0.44 |   **0.41** |     1.221 |

\*b26 mozilla C was stalled. This session mozilla C d **802** is the real one; C/us 2.42 vs b25 **2.53**. dickens us-vs-us **+13%** is the keep. mr held. nci −9% is at the session floor — not a sign-flip vs C (already losing). xml no sign-flip. Do not retry two-shift peek. Do not BMI2.

**Brick 28 (reverted — 1-stream CTZ `fast_1x1`/`fast_1x2`).** Ported the 4X2 `bits>>53` + CTZ reload onto `decode_stream` (regen < 256 is X1). Clean session null-arm **0.988**. dickens us d **302 → 284** (C/us d 2.95 → 2.91); mr held 1.53 → 1.52. No unique canary. Named reason: C has no 1X1 fast loop; 1-stream bodies are short (`src` often just ≥8) so `ip/7` iters is 0 and the fast-init is a tax on the BitRev remainder. Do not retry a tighter `src`/`dst` gate of the same loop without a count of 1-stream blocks that would actually enter. Sizes unchanged. Dual-gate 12/12. Decode board stays brick 27.

**Brick 29 (Huffman emit: `covers` once, no per-symbol `Result`).** `nbits == 0` is treeless reuse of a previous CTable that does not contain a byte in this block — a fallback, not a bug (`silesia_mr_prefix` hits symbol 239). Naive `debug_assert` panics. C `HUF_validateCTable` checks coverage before emit. We scan `prev` once (`HuffCTable::covers`); new tables from `build_ctable(lits)` already cover. Hot `huff_sym` / `encode_rev_into` drop `Result`. Scalar `encode_stream_scalar` keeps the check as the oracle (`encode_stream_unrolled_matches_scalar`, `covers_rejects_unseen_symbol`).

Lib tests **112/112**. Dual-gate **12/12**. Sizes unchanged. Standing `--levels 1` (null-arm **1.013**). Encode canaries vs the brick-27 session (1-stream decode try; encode was unchanged). mozilla that session was cores-busy **0.76** — ignore 67.9. Decode not touched; mr C/us d **1.58** holds vs b27 **1.53**. xml encode flat (no sign-flip).

| file    | us c (b27 sess) |  us c now | C/us c now | us/c size |
| ------- | --------------: | --------: | ---------: | --------: |
| mr      |            83.8 |  **85.8** |       3.12 |     1.114 |
| nci     |           175.1 | **184.3** |       3.04 |     1.303 |
| mozilla |           67.9* |  **86.4** |       3.27 |     1.222 |
| xml     |           157.0 |     155.1 |       2.77 |     1.308 |
| x-ray   |           630.7 | **746.8** |   **0.66** |     1.221 |
| webster |            83.0 |  **90.7** |       2.36 |     1.436 |

Modest encode keep (mr Huff was ~67% of encode; Result ABI + 5× `?` in the unroll). Not 15% vs C (mr C/us c still ~3). Do not drop `Result` on decode `decode_one` the same way — nbits==0 there is corrupt input. Do not skip `covers` on treeless.

**Brick 30 (FSE DTable = C `FSE_decode_t` 4 bytes + `read_bits(0)` no-op).** nci/xml/mozilla lose on `decode_sequences` (~85% of decode, C/us d ~3.1). Packed `FseEntry` as `{baseline:u16, symbol:u8, num_bits:u8}` (`#[repr(C)]`, `size_of==4`). `read_bits(0)` returns without `skip_bits` (LL/ML extras are often 0 bits; FSE `advance` nbits can be 0). Did **not** wrap the seq loop (brick 23). Oracle: RFC LL table + FSE roundtrips + `silesia_mr_prefix_entropy_oracle`.

Lib tests **112/112**. Dual-gate **12/12**. Sizes unchanged. Standing `--levels 1` (null-arm **1.052** — box turbo vs b29; C also jumped ~40–50%). Unique signal is seq-heavy decode vs Huffman-heavy mr: nci/xml us d **+55–60%**, mr d only **+19%**. Same-session C/us d: nci **3.11→2.81**, xml **3.16→2.84**, mozilla **2.27→2.10**. mr C/us d 1.58→1.95 is C Huffman turbo, not a seq regression.

| file    | us d (b29) | us d now | C/us d (b29) | C/us d now | us/c size |
| ------- | ---------: | -------: | -----------: | ---------: | --------: |
| nci     |        479 |  **769** |         3.11 |   **2.81** |     1.303 |
| xml     |        433 |  **673** |         3.16 |   **2.84** |     1.308 |
| mozilla |        351 |  **570** |         2.27 |   **2.10** |     1.222 |
| mr      |        612 |      727 |         1.58 |       1.95 |     1.114 |

Still not 15% vs C. Next: Huffman emit on mr/mozilla (encode C/us ~3–4); FSE encode `.get()` on nci. Do not wrap `decode_sequences`. Do not BMI2. Do not retry 1-stream CTZ.

**Brick 31 (reverted — Huffman emit 16/8/fill dispatch).** Analog of 4×4 / 8×8 / 16×16: unroll width from CTable `max_nbits` (16 if ≤3, 8 if ≤7) and `emit_fill` (pack actual nbits into the 64-bit word) for tableLog 11. Byte-identical vs scalar (`encode_stream_unrolled_matches_scalar`). Dual-gate 12/12. Sizes unchanged.

1. **Always `emit_fill` on max≥8** (null-arm **1.034**): mr us c **101→115** (C/us c **4.04→3.72**). **sao C/us c 0.99→1.20** — sign-flip on the long-code holdout. Per-symbol `huff_fits` is a tax when codes are already 11-bit (same 5-wide as before plus a failed check). The sign-flip IS the dispatch trigger (`codec-content-adaptive-dispatch`).
2. **Dispatch `mean_nbits <= 6` → fill, else 5-unroll** (null-arm **1.070**, inadmissible): board-wide us drop including decode (untouched). Cannot keep.

Do not retry always-fill. Brick 32 is the harvested dispatch.

**Brick 32 (kept — Huffman emit K-from-max + fill, gated on census).** `codec-content-adaptive-dispatch`: the brick-31 sao sign-flip is the trigger, not a revert of the idea. Counted first (`silesia_huff_nbits_census`, ignored): per-CTable `max_nbits` / freq-weighted `mean_nbits_x10` on real `-1` literals.

| file    | tables |    mean |   max≤7 | mean≤5.5 | mean≤7.0 |
| ------- | -----: | ------: | ------: | -------: | -------: |
| mr      |     77 | **4.7** |      3% | **100%** |     100% |
| mozilla |    285 |     6.3 |      0% |      12% | **100%** |
| nci     |    252 | **4.1** | **52%** |     100% |     100% |
| xml     |     17 |     5.0 |      0% |     100% |     100% |
| sao     |      1 | **7.5** |      0% |       0% |   **0%** |
| x-ray   |      7 |     6.2 |      0% |       0% |     100% |

max≤3 never fires on Silesia (K16 stays for peaked synthetics). Menu: K from `max_nbits` so `K*max+7<64` (16/14/11/9/8/7/6/5 — the 4×4/8×8/16×16 analog). Fill only when `mean_x10≤70` AND expected pack beats K by >2; fill is **K-then-extras** (tax-free prefix). Sao's one table (mean 7.5, max 9) stays K6, no fill.

Oracle: `encode_stream_unrolled_matches_scalar` (fox + noise + peaked), `huff_pack_dispatch_separates_peaked_from_flat`. Lib **113/113** (1 ignored census). Dual-gate **12/12**. Sizes unchanged.

Standing `--levels 1` (null-arm **0.974**). Unique signal is Huffman-heavy encode vs the sao holdout:

| file    | us c (b30) | us c now | C/us c (b30) | C/us c now | us/c size |
| ------- | ---------: | -------: | -----------: | ---------: | --------: |
| mr      |        101 |  **144** |         4.04 |   **3.07** |     1.114 |
| mozilla |        133 |      142 |         3.36 |       3.38 |     1.222 |
| nci     |        268 |      309 |         3.05 |   **2.99** |     1.303 |
| xml     |        224 |      259 |         2.61 |       2.81 |     1.308 |
| sao     |        366 |      407 |         0.99 |   **0.97** |     1.143 |
| x-ray   |       1052 |     1146 |         0.74 |       0.74 |     1.221 |

mr encode **+42% us**, C/us **4.04→3.07**. Sao held (we still win). mozilla C/us flat — Huff is 33% of encode and mean 6.3 is the knife-edge; K-then-fill did not sign-flip it. Decode not this brick.

Still not 15% vs C. Next: FSE encode `.get()` / `delta_at` on nci/xml; mozilla encode is still MatchFind-heavy. Do not wrap `decode_sequences`. Do not BMI2. Do not retry always-fill. Do not retry 1-stream CTZ.

**Brick 33 (reverted — FSE CTable arrays / encode_fast).** nci encode is FSE seq (~20%) after brick 32 Huff. Three shapes, none a keep:

1. **Inline `[u16;512]` + `[FseCDelta;64]`**, drop `Vec::get` (null-arm **1.002**): nci C/us c **2.99→3.03**. Larger CTable memcpy on `entropy.clone()` per block; unused slots needed the zero-prob `deltaNbBits` or `bit_cost` lied. Reverted.
2. **Always flush + `encode_fast`/`add_bits_fse`** (C `ZSTD_encodeSequences` shape; one session null-arm **1.125** inadmissible, rerun **1.010**): nci C/us c **3.20**. Extra `extend_from_slice` per seq (~1e6 on nci) is a tax vs accumulating until 64.
3. **Opportunistic flush when `!fits(27)`** (null-arm **0.995**): nci C/us c **2.95** (~1%, noise). sao C/us c **0.97→1.26** looks like a holdout slip (MatchFind file; not this kernel). Not a keep.

Do not retry FSE CTable-as-arrays or per-seq flush. Remaining nci encode lever is MatchFind / extras-between-FSE, not another bit-container check. Next: mozilla/ooffice MatchFind. Standing board stays brick 32.

**Brick 34 (reverted — Fast repcode1 / prefetch / hash `get_unchecked`).** C `ZSTD_compressBlock_fast` checks repcode1 at `ip+1` before the hash probe. We never did. Four shapes, none a speed keep (sizes moved on the bitstream-changing arms; dual-gate still 12/12):

1. **Always-on repcode1** (null-arm **0.995**): nci/mozilla **us/c size 1.303→1.145 / 1.222→1.162** (more matches). **sao C/us c 0.97→1.12** — sign-flip (MatchFind 74%, extra u32-pair per step, almost no hits). mozilla us **142→124** (more seqs → more FSE). Dispatch trigger, not a mean.
2. **Dispatch `ip == anchor` only** (the 1-literal gap; null-arm **0.990**): sao still **1.07**; mozilla C/us c **3.38→3.67**. The ratio win is extra entropy work, not a faster kernel.
3. **Byte-identical prefetch of the hash candidate** + reuse the `ip` u32 (null-arm **0.985**, sizes = brick 32): mozilla C/us c **3.45** (flat). Not enough independent work between prefetch and the compare.
4. **`hash4_swap` `get_unchecked`** (null-arm **0.971**): mozilla C/us c **3.72**. LLVM already had the masked index; the helper was a tax.

Do not retry always-on Fast repcode1 (sao). Do not retry prefetch-without-pipeline. Do not retry hash-table `get_unchecked` without a `.s` that still shows `panic_bounds_check`. Next MatchFind lever needs a **count** (probe hit rate / hash-miss vs compare-miss) before another kernel. Decode extras-between-FSE still open. Standing board stays brick 32.

Descent: `docs/plans/m7-encoder-whys.md`.

---

### INSTRUMENT DEFECT (2026-08-14) — bricks 16-34 speed verdicts are NOT admissible

A full re-analysis of the 829 `m7_speed` rows / 36 full Silesia L1 sessions found the
harness, not the codec, was producing the deltas. Repair plan and evidence:
[`m7-benchmark-repair.md`](m7-benchmark-repair.md).

1. **Estimator mismatch.** facebook/zstd `-b` reports its **fastest** round (measured:
   360.6 -> 413.0 MB/s as `-i` rises 1 -> 3, spread collapsing 45 -> 0.5 MB/s). Our
   `time_loops` reported a **mean**. Quoting our mean against C's best understated us by
   a median **+9.5% compress / +11.4% decompress**, and content-dependently (+0.8%
   webster, +29.6% reymont) -- so it distorted the per-file axis every Great Gate
   dispatch decision was made on.
2. **The box swing is thermal decay, mid-session.** C's own unchanged binary read
   442 -> 201 MB/s across ONE 5-minute L1 session. The null arm for that session read
   **1.0012**, because it is taken once at session start. `r(null_arm, box scale) = -0.16`
   over 36 sessions: it is structurally blind to this.
3. **Campaign trend, box-normalised (first 5 full sessions vs last 5).** Decode is a real
   win: mr **-63%**, x-ray -48%, dickens -44%, ooffice/mozilla -33%. **Compress is flat on
   every file: mr -3.0%, mozilla -0.3%, nci -1.5%, xml -0.1%, ooffice -0.6%, sao +2.3%,
   x-ray +6.7%** -- all inside a measured zero-code-change band of +/-12% to +/-32%.
4. **Brick 32's citation checked against the run:** mr compress C/us is mean 3.22, sd 0.30
   over 36 sessions. The cited `4.04` is the **maximum of the entire run (z = +2.73)** and
   the `3.07` is the mean (z = -0.49). It measured regression to the mean.

**Consequences, in force now:**

- Every `C/us` figure in the tables above carries `instrument=v1 (wall, us-mean vs
  C-best, n=1 pair)`. They are history, not evidence.
- The "do not retry" list (bricks 23, 24 BMI2, 28, 31, 33, 34) is downgraded to
  **unproven on instrument v1**. `codec-measurement` 11: a refutation expires when its
  baseline moves.
- **Cycles per byte** (`QueryThreadCycleTime`, frequency-invariant) is now the
  cross-session progress metric. `C/us` MB/s stays as the standing cross-implementation
  number, ABBA-adjacent, quoted only with its session.
- The kept encode bricks (16, 17, 22, 29, 32) must each re-clear the gate on the repaired
  instrument or be reverted for simplicity.

### Standing board on the REPAIRED instrument (2026-08-14, `--levels 1`, bricks 35-37)

One session, all **18** corpora. Estimator best-of-N both arms, N>=25 per phase, phases
timed separately as C does, cycles/byte alongside MB/s, null-arm **1.0122**, dual-gate
**18/18**, every size unchanged from the v1 board. This is the new zero; do not compare
it to any v1 row above. **Never average these.**

| corpus           | split |   C/us c |   C/us d | cyc/B c | cyc/B d | us/c size |
| ---------------- | ----- | -------: | -------: | ------: | ------: | --------: |
| zeros-32m        | T     | **1.03** |     2.32 |   0.291 |   0.291 | **0.901** |
| text-32m         | T     |     1.46 | **1.03** |   0.290 |   0.299 |     1.022 |
| incomp-32m       | H     | **1.11** |     1.71 |   0.612 |   0.372 |     1.000 |
| **jsonlog-16m**  | H     |     2.28 |     1.69 |   9.462 |   2.597 | **1.527** |
| **smallmsg-8m**  | T     |     1.79 |     2.30 |   9.149 |   2.671 | **1.615** |
| **versions-16m** | T     | **0.97** | **1.00** |   2.423 |   0.965 | **0.755** |
| mr               | H     |     2.92 |     1.40 |  15.796 |   2.108 |     1.114 |
| ooffice          | H     |     2.13 | **0.95** |  12.510 |   2.028 |     1.262 |
| osdb             | H     |     2.56 |     1.77 |  12.525 |   2.591 |     1.144 |
| reymont          | H     | **1.86** |     1.98 |  14.371 |   3.110 |     1.437 |
| sao              | H     | **0.89** | **0.24** |   5.379 |   0.532 |     1.143 |
| webster          | H     |     2.16 |     1.79 |  14.041 |   2.751 |     1.436 |
| dickens          | T     |     2.24 |     2.07 |  17.171 |   3.341 |     1.215 |
| mozilla          | T     |     3.09 |     1.69 |  15.396 |   3.280 |     1.222 |
| nci              | T     |     2.41 |     2.27 |   6.357 |   2.218 |     1.303 |
| samba            | T     |     2.17 |     1.94 |   9.696 |   2.403 |     1.255 |
| xml              | T     |     2.18 |     2.04 |   7.262 |   2.120 |     1.308 |
| x-ray            | T     | **0.68** | **0.28** |   1.995 |   0.559 |     1.221 |

**Sec.7 target status** (decompress C/us <= 1.11, compress <= 1.25):

- **Decompress PASSES on 5 corpora** -- `sao` 0.24, `x-ray` 0.28, `ooffice` **0.95**
  (we are now *faster than C*), `versions-16m` 1.00, `text-32m` 1.03. `mr` 1.40 is the
  nearest miss. Worst is now `zeros-32m` 2.32 (an RLE-path artefact, not a kernel).
- **Compress PASSES on 5 corpora** -- `x-ray` 0.68, `sao` 0.89, `versions-16m` 0.97,
  `zeros-32m` 1.03, `incomp-32m` 1.11. `mozilla` 3.09 is the worst; `EncodeMatchFind`
  owns that gap.

Decompress movement across bricks 36+37 alone (same instrument, controls flat):
webster **4.318 -> 2.751 (-36%)**, samba **2.905 -> 2.403 (-17%)**, xml **2.461 -> 2.120
(-14%)**, ooffice **2.402 -> 2.028 (-16%)**, jsonlog **3.184 -> 2.597 (-18%)**.

**The ratio front is now the bigger story.** The three new product-corpus classes expose
`smallmsg-8m` **1.615** and `jsonlog-16m` **1.527** -- worse than any Silesia file -- and
that is the content MATA actually ships. `versions-16m` at **0.755** is a class we beat C
on outright. Neither was visible from Silesia. Ratio is a `codec-experimental` / block-
splitting question (C emits 138 compressed blocks on mr where we emit 77), not a speed
brick.

Combined movement of the instrument repair plus bricks 35-36 vs the v1 last-5 mean:
compress xml **-24%**, nci **-26%**, webster **-20%**, dickens **-20%**, ooffice -16%,
sao -19%, x-ray -16%, mr -16%; decompress webster **-23%**, xml **-22%**, samba **-21%**,
nci **-20%**, ooffice -17%.

### Phase B / C results (2026-08-14) -- see [`m7-benchmark-repair.md`](m7-benchmark-repair.md)

**Phase C, C1 CLOSED.** The Huffman emit batch (bricks **16 + 29 + 32**) was
re-adjudicated on the repaired instrument via an interleaved ABAB behind
`RZSTD_HUFF_FAST`, with `encode_stream_scalar` (already the byte-identity oracle) as the
second arm. Removing them costs **mr +30.5%**, **mozilla +17.5%**, **osdb +14.0%**,
xml (control, Huff 9.9%) +6.6% -- effect tracks Huffman share, same-arm spread 0.1-6.7%.
**They were real all along**; instrument v1 simply could not resolve them. Bricks 17 and
22 still need toggles; refutations 23/24/28/31/33/34 stay *unproven*, and the 30% figure
above proves v1 was capable of hiding an effect that size.

**Phase B, level board LANDED -- and it found two defects.** Strategies 3-9 had never
been speed-measured. `--levels -1,1,3,4,5,7,9,11,13,16`:

- **B1: CONFIRMED and FIXED.** `find_lazy` / `find_bt_lazy` never back-filled the hash
  chain over the span a match covers -- `find_greedy` always did. On matchy content that
  is most of the file, so later searches saw a nearly empty chain. C does this via
  `nextToUpdate`. Fixed behind `RZSTD_LAZY_FILL` (default ON): **ratio improves on 6 of
  7 (file, level) points** (mr L7 **-6.81%**, webster L7 **-5.23%**, webster L9 -2.75%)
  **and speed improves on 5 of 7, by up to -36.8%** -- a populated chain finds long
  matches sooner, so `ip` advances further. Dual-gate OK on all 7. Open: `xml`
  (match_frac 0.886) is 2x slower with the back-fill while its ratio still improves --
  a dispatch trigger, not a revert.
- **B2: NOT A DEFECT -- my error.** The level -> strategy table is **source-size
  dependent** and I read it against `/dev/null`. For a real file L13 is btlazy2
  (searchLog 4), not btultra, and L11 is lazy2 (searchLog 6) -- so a cheaper higher
  level is legitimate. **Our level table was then verified against C v1.5.7 directly:
  all 7 cparams MATCH on levels 1/3/5/7/9/11/13/16/19** (mission 3.1.1 satisfied).
  Standing law: never read the level table without a real `src_hint`.

**Instrument additions:** discarded warmup pass (the first-measured file swung 63% for
identical code); per-row **same-arm spread** replacing the null arm (it flagged a bad
sample at 39.4% on its first run, where the null arm read normal); in-process peak RSS.

`unsafe` is authorised in `rusty_zstd` where a measurement justifies it (2026-08-14, user).
Shape is unchanged: `deny(unsafe_code)` at the root, per-module `#[allow(unsafe_code)]`
islands with a stated SAFETY invariant, scalar twin retained as the oracle.

---

## 9. Engineering discipline

House skills, non-negotiable:

1. **`codec-measurement` before any number.** Pin, CPU-time, ABBA, null arm, work-count parity, cores-busy, fresh binary, three-probe refutations.
2. **One brick per commit.** Revert if the relevant metric does not improve or hold. Independently revertible.
3. **Profile before optimizing.** Scalar-clear first. SIMD only where the profiler points and auto-vec failed (`--emit asm`, count packed ops).
4. **Stage oracles before end-to-end.** A ratio/speed regression must be attributable to one stage in minutes.
5. **No self-grading.** Correctness vs C libzstd (or fixtures derived from it).
6. **Holdout discipline.** Tuning data stays in train splits.
7. **`rusty-coding-requirements`.** Pure Rust; no `*-sys`; thoth for glyphs; `rusty_alloc` only in the CLI/capi *binary* via the allocator seam, never in the library; `forbid(unsafe_code)` on the library except a tiny, documented `unsafe` module for SIMD/profile/`get_unchecked` behind safe APIs.
8. **Unsafe last.** `rusty-unsafe-optimizations`: read the `.s` first; most “bounds-check tax” is already gone. Bounds-check ceiling probe once (`codec-analyzer` #4) and do not relitigate.
9. **One compliant timer.** CLI `-b`, bench crate, and unit benches call the same harness.

---

## 10. Security and untrusted input

zstd is an untrusted-input parser. Launch blockers:

- Window size cap default 128 MiB; explicit raise required (zip-bomb / RSS)
- Block size cap 128 KiB; reject oversize
- All FSE/Huffman tables validated before use (no OOB from corrupt NCount)
- Sequence codes in-range; offsets ≤ window; repcodes legal
- No panic / unwrap on malformed frames; `Error` with stable kinds
- Fuzz: `cargo fuzz` on decompress (and compress with random params) in CI
- Checksum verified by default; `--no-check` is opt-out
- `ZSTD_CLEVEL` / `ZSTD_NBTHREADS` ignored if malformed (warn, match C)
- Legacy decoder isolated behind a feature; fuzzed separately
- C ABI: no `unwrap` across FFI; null CCtx handled as C does

---

## 11. Packaging, distribution, install

| Channel                                | What ships                                                                                                                                                        |
| -------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **crates.io** `rusty_zstd`             | Library. Features: `std` (default), `legacy`, `seekable`, `train`, `mt`                                                                                           |
| **crates.io** `rusty_zstd-cli`         | Binaries `zstd` / `unzstd` / `zstdcat` / `zstdmt` (feature-gated names if crates.io binary clashes demand `rzstd` + symlink docs)                                 |
| **crates.io** `rusty_zstd-capi`        | `cdylib`/`staticlib`; not a default dep of the lib                                                                                                                |
| **GitHub** Remade-With-Rust/rusty_zstd | Source, corpora scripts, ledger, this plan                                                                                                                        |
| **cargo deny** + license files         | Apache-2.0 AND MIT; NOTICE for any vendored *spec* text                                                                                                           |
| **CI**                                 | `cargo test`, fuzz smoke, `cargo check --target wasm32-unknown-unknown -p rusty_zstd --no-default-features --features std`, deny, clippy, fmt, MSRV (document it) |

Windows, Linux, macOS, aarch64, wasm32 for the library. CLI is std OS. No C toolchain required to `cargo install`.

---

## 12. Integrations (portfolio)

Ship the library so these can take a feature flag. Do not block M1–M7 on them.

### Highest value — distributed / local-first

| Component                                                              | How zstd helps                                                                 | Why it matters                                                   |
| ---------------------------------------------------------------------- | ------------------------------------------------------------------------------ | ---------------------------------------------------------------- |
| **SpaceDB** (`spacedb-store`, `spacedb-replica`, `spacedb-durability`) | Compress CRDT deltas, state vectors, snapshots, WAL-style logs, erasure shards | Strongest fit. Mesh anti-entropy and repair are bandwidth-bound. |
| **MATA transport** (iroh + relay)                                      | Compress payloads before they leave the device                                 | Offline-first sync critical path                                 |
| **spacedb-replica anti-entropy**                                       | Smaller deltas → faster partition heal                                         | Direct multiplier on convergence                                 |

### Strong secondary

| Component                             | Integration                                                             |
| ------------------------------------- | ----------------------------------------------------------------------- |
| **remade_ffmpeg_rs**                  | Optional compression for side data, packaged assets, certain containers |
| **FFAI / Mercury / Carmenta / Diana** | Voice packs, model weights, cached features, offline bundles            |
| **deputy**                            | Dependency caches / offline package bundles                             |
| **starfire / comet**                  | GameStream payload / capture buffers if useful                          |

API-first: SpaceDB and ffmpeg consume the **library**, never the CLI.

---

## 13. Risks

| Risk                                       | Mitigation                                                                                                                          |
| ------------------------------------------ | ----------------------------------------------------------------------------------------------------------------------------------- |
| Speed gap to highly-tuned C remains large  | Correct+usable through M6; dedicated M7 campaign; measure vs C *and* best pure-Rust peers; SIMD only after redundancy               |
| Ratio falls behind at levels 19–22         | Treat ultra as stretch; own −7…3 and default 3 first (real traffic)                                                                 |
| Dictionary trainer quality                 | Ship raw-content dicts in M4 day one; COVER gated vs C-trained dicts                                                                |
| CLI flag matrix is huge                    | Goldens: parse C `--help` / man page into a table; implement by rows; unknown flags error like C                                    |
| C ABI drift / experimental params          | Export stable set; document experimental as unsupported; add on demand                                                              |
| `--format=gzip/xz/lz4` expected by scripts | Explicit unsupported error; adapters only onto house codecs later                                                                   |
| Existing pure-Rust crates “good enough”    | Differentiate on full CLI+ABI+trainer+MT+measurement rigor+house maintenance                                                        |
| Scope creep                                | Library-correctness > CLI goldens > ABI > format adapters. Ultra ratio is stretch, not a blocker for “competitor” on default levels |
| Oracle contamination (linking C)           | Never link libzstd; shell out to a pinned binary                                                                                    |

---

## 14. Launch checklist (call it 1.0)

A GitHub release / crates.io `1.0.0` only when **all** are true:

- [ ] RFC 8878 decode + encode; C cross-decode both directions on holdout
- [ ] Levels −7…22; all 9 strategies; level table vs C recorded
- [ ] Streaming, dicts, trainer (fastcover default), `--long`, `--rsyncable`, seekable, MT
- [ ] CLI drop-in for §3.2 launch-complete flags; aliases; env vars
- [ ] Default window cap 128 MiB; fuzz; no panic on malformed input
- [ ] Ledger: decompress ≥ 0.90× C on Silesia **or** a dated exception with a named remaining gap
- [ ] Ledger: compress L1 and L3 ≤ 1.25× C at matched ratio **or** a dated exception
- [ ] `cargo deny` clean; wasm check; alloc not in the library
- [ ] C ABI crate published **or** explicitly marked 1.1 with a public issue (prefer published)
- [ ] README claims each cite a ledger line
- [ ] SpaceDB and/or remade_ffmpeg_rs can depend on us behind a feature (even if not flipped default)

Until then the crate stays `0.x`.

---

## 15. Immediate next actions (M0)

**Done locally** (this tree). GitHub remote `Remade-With-Rust/rusty_zstd` is not created yet.

1. Workspace, licenses, `cargo deny`, README pointing at this plan.
2. C oracle pinned: facebook/zstd **v1.5.7** (`third_party/zstd/README.md`, `scripts/fetch-oracle.ps1`). Bench never links libzstd.
3. Generated corpora recipes + hashes on ledger lines; train/holdout split; optional Silesia fetch. Blobs gitignored.
4. `rzstd-bench`: pinned C CLI, ratio, in-memory `-b` MB/s, oneshot CPU+wall+RSS, method line → `bench/ledger.jsonl`.
5. `--baseline-only` first ledger: C numbers on the board (levels 1 and 3 × zeros/text/incomp 32 MiB).
6. Skeleton `compress`/`decompress` → `Error::Unimplemented`; CLI `rzstd --version`.

Then M1: decompressor vs C, brick by brick, entropy state first.

**M1 done:** `decompress` handles raw, RLE, compressed (Huffman + FSE), skippable, concatenated frames, and checksums. Holdout gate: C v1.5.7 compress of `incomp-32m` at levels 1 and 3, rusty_zstd decompress, reconstructed bytes identical. Fuzz 24h and speed vs C are standing M1+ campaigns, not blockers for the holdout close.

---

## 16. Skill routing (for agents on this repo)

| Mission                    | Skill                                                                                                                                     |
| -------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------- |
| Any number                 | `codec-measurement`                                                                                                                       |
| M0 harness / stage %       | `codec-analyzer`                                                                                                                          |
| M1 decode                  | `codec-bringup-decoder`                                                                                                                   |
| M2–M5 encode               | `codec-bringup-encoder`                                                                                                                   |
| M7 speed                   | `codec-optimize` → `codec-eliminate-redundancy` first, then `codec-memory-copies`, then `codec-vectorize-kernel`, `codec-asm-kernel` last |
| Stubborn gap               | `codec-six-whys-unknowns`                                                                                                                 |
| House deps / alloc / thoth | `rusty-coding-requirements`                                                                                                               |
| `unsafe`                   | `rusty-unsafe-optimizations` (after `.s`)                                                                                                 |

Do not use `codec-tune-quality` for this codec. It is lossless.
