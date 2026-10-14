# Small-frame decode: recommendations (2026-10-14)

**Status: recommendations, measured on a scratch copy; nothing here is applied
to `crates/`.** Every number below is an exact instruction count (callgrind
Ir) with byte-identical output, measured outside this repository. This page
proposes changes and records the evidence for them; each still needs this
repository's own gates (the corpus boards, `cargo test`, the wall-clock
instrument in `m7-benchmark-repair.md`) before it lands.

## Why small frames

The campaign record here measures large corpora (Silesia, the 18 m7 corpora).
A different workload is now the biggest consumer by call count: **SpaceDB**
(`spacedb-store`, in mata-master) compresses every stored row on its own, at
level 3, above a 96-byte floor, keeping the frame only if it shrank. Typical
rows are 100-600 bytes of JSON: DID documents, audit rows, CRM leads, form
submissions, device records. On that shape rusty_zstd spends almost all of
its decode time building tables it then barely uses.

Found while measuring mata-master's DID registry: its snapshot read every
stored row back, and **92% of the snapshot's cost was decompressing those
rows**, about 80K instructions for a ~300-byte row.

## The instrument

A standalone driver (outside this repo, at `E:/ir-midkms/zstd-drv`) built
against a copy of this crate at 0.3.1, in mata-master's shipping profile
(`opt-level = "s"`, thin LTO). Five row shapes from the MATA stores, each
built at target sizes from 96 B to 64 KiB, eight rows per shape and size;
every row compressed at level 3 and kept only if it shrank (exactly SpaceDB's
rule), every kept frame decompressed 20 times. Anchors: rows, kept frames,
bytes in and out, and an FNV over every decompressed byte. Each size can run
alone, and a compress-only mode isolates decode cost by subtraction.

Exactness: same binary re-run is identical to the instruction; separate WSL
invocations differ by a fixed ~400-instruction pedestal, so every comparison
below ran all arms in one invocation.

## Finding 1 (the big one): the X2 table is built for sections that never use it

`read_table` always calls `table_from_weights(.., want_x2 = true)`, so every
Huffman literals section pays for:

- `upsample_dtable` stretching the X1 table to `FAST_TABLELOG` (2,048 entries),
- zero-filling a 2,048-entry `u32` buffer (`Vec::extend_with`),
- `x2_from_x1_into`: the 2,048-entry data-dependent gather.

But `select_x2` returns **false for every section regenerating under 256
bytes**, so on small frames the X2 table is built and never read. The
`N2` instrument in `huffman.rs` (`X2_STATS`: built vs used) was written to
ask exactly this question; on small frames the answer is "built every time,
used never".

Census of the 96-byte bucket (all rows ~183 B), baseline:

| function (inclusive) | Ir | share |
|---|---:|---:|
| `decompress_with_history` | 94,821,026 | 94.2% |
| `huffman::read_table` | 87,894,500 | 87.3% |
| `huffman::table_from_weights` | 81,802,684 | 81.3% |
| **`huffman::x2_from_x1_into`** | **71,143,115** | **70.7%** |
| `fse::decompress_weights_into` | 6,609,351 | 6.6% |
| `huffman::upsample_dtable` | 6,566,120 | 6.5% |
| `HuffmanTable::decode_into_x1` (the actual decoding) | 2,663,120 | 2.6% |

The literals themselves cost 2.6%. The table nobody reads costs 70.7%.

### The change

Decide `want_x2` before the table is built. `decode_literals` already knows
the section's regenerated size from its header when it calls `read_table`:

```rust
// compressed.rs, the Huffman (type 2) arm of decode_literals
let (table, tree_size) =
    huffman::read_table(huff_recycle, section, regen as usize >= X2_BUILD_MIN)?;
```

with `read_table` passing the flag through to `table_from_weights` (which
already supports `want_x2 = false`: it skips the upsample and hands the X2
buffer back empty). `use_x2` already checks `table_x2.len() == table.len()`,
so a table built without X2 simply decodes on X1. Output is identical by
construction: X1 and X2 decode the same symbols.

Two call sites keep today's behaviour: `dict.rs` (a dictionary's table serves
many frames, so building X2 once there is cheap) passes `true`, and tests that
assert on the X2 arm keep it through a `#[cfg(test)]` wrapper.

Treeless sections (type 3) reuse the previous table. If that table was built
without X2, a large treeless section decodes on X1; still correct, possibly
slower. Rare on small frames, and worth counting with `X2_STATS` on the
corpus boards before choosing the threshold below.

### The threshold sweep

Same driver, five builds, one invocation. `z0` = 0.3.1 as shipped;
`X2_BUILD_MIN = N` builds X2 only for sections of at least N regenerated
bytes. Whole driver Ir (compression included, so these understate the decode
change):

| row size | today | N = 256 | N = 1024 | N = 2048 | N = 4096 |
|---:|---:|---:|---:|---:|---:|
| 96 | 100,644,242 | 36,264,616 | 23,146,158 | 23,146,158 | 23,146,158 |
| 256 | 128,147,101 | 52,120,921 | 37,003,060 | 37,003,060 | 37,003,060 |
| 384 | 130,807,018 | 114,443,227 | 42,850,331 | 42,850,331 | 42,850,331 |
| 512 | 135,482,471 | 119,092,613 | 48,663,862 | 48,663,862 | 48,663,862 |
| 1,024 | 150,742,079 | 141,401,125 | 66,508,192 | 66,508,192 | 66,508,192 |
| 2,048 | 179,037,870 | 174,360,326 | 129,019,902 | 100,902,282 | 100,902,282 |
| 4,096 | 233,282,521 | 230,951,051 | 198,623,617 | 185,445,357 | 166,320,297 |
| 16,384 | 528,080,305 | 528,045,829 | 510,172,833 | 510,172,833 | 502,245,613 |
| 65,536 | 1,678,418,197 | 1,678,385,551 | 1,660,459,763 | 1,660,459,763 | 1,660,459,763 |
| **all sizes 96-4,096** | **1,295,510,305** | **957,747,587** | **604,658,239** | **562,652,538** | **542,307,828** |
| | | **-26.1%** | **-53.3%** | **-56.6%** | **-58.1%** |

Every column's FNV anchor is identical to `today`'s, at every size.

Decode alone (driver minus its compress-only twin), per decompressed frame:

| row size | today | N = 4096 | change |
|---:|---:|---:|---:|
| 96 (frames ~183 B) | 140.9K | 26.9K | **-80.9%** |
| 256 | 153.0K | 39.1K | **-74.5%** |
| 384 | 155.0K | 45.0K | **-71.0%** |
| 512 | 159.6K | 51.1K | **-68.0%** |
| 1,024 | 175.5K | 70.2K | **-60.0%** |
| 2,048 | 204.9K | 107.3K | **-47.7%** |
| 4,096 | 261.7K | 178.0K | **-32.0%** |

### Recommendation, in two steps

1. **Land `X2_BUILD_MIN = 256` first.** It is no-regret: below 256 `select_x2`
   already forbids the X2 arm, so no section changes decoder; only a table
   nobody reads stops being built. -26.1% on this corpus, -64% on 96-byte rows.
2. **Then restore libzstd's real decoder choice, table time included.**
   `select_x2` reads `ALGO_TIME` as `(_, d0, _, d1)` and drops the
   table-build terms, which was correct while X2 was always built (its cost
   was sunk). Once the build is lazy that cost is real again, and libzstd's
   `HUF_selectDecoder` counts it: `DTime = tableTime + decode256Time * D256`,
   with `DTime1 += DTime1 >> 5`. Use that full model as the `want_x2`
   decision in `decode_literals`. For mid-range Q it moves the X1/X2
   crossover to roughly 3 KiB, which is where this sweep's Ir curve says it
   belongs (N = 4096 still won at 4 KiB rows and at 16 KiB).

   **Gate this one on wall clock, not only Ir.** Ir prices X2's per-symbol
   win and its build cost at 1 per instruction; the gather is cache-bound and
   the two-symbol decode is ILP-friendly. The sweep above stops at N = 4096
   and never found the point where building X2 pays in instructions; the
   wall-clock crossover will sit lower. Measure it on the m7 boards and on a
   small-row corpus, and keep whichever threshold model wins both.

## Finding 2: `read_ctable` builds a decode table only to measure it

```rust
pub(crate) fn read_ctable(src: &[u8]) -> Result<(HuffCTable, usize), Error> {
    let (table, used) = read_table(None, src)?;
    let _ = table;
```

The dictionary-load path parses the Huffman tree as a decode table (X1, the
upsample and the full X2 gather) to learn how many bytes the description
used, then drops it. Passing `want_x2 = false` there removes the upsample and
the X2 build; a header-length parse that never builds a table would remove the
rest. Once per dictionary load, so small in total, but it is a full X2 build
per load with no reader.

## Finding 3: what small decodes cost once Finding 1 lands

Census of the 96-byte bucket at N = 256 (before the threshold is raised):

| function (inclusive) | Ir | share |
|---|---:|---:|
| `huffman::read_table` | 23,571,824 | 65.0% |
| `huffman::x2_from_x1_into` (sections just over 256 B) | 12,141,554 | 33.5% |
| `fse::decompress_weights_into` (the tree's FSE-coded weights) | 6,609,351 | 18.2% |
| `compressed::decode_huff_streams` | 3,560,688 | 9.8% |
| `fse::FseTable::from_norm_buf` | 2,235,243 | 6.2% |
| `compressed::decode_sequences_avx2` | 2,196,944 | 6.1% |
| `fse::read_ncount_into` | 1,743,952 | 4.8% |

With the threshold raised, the next costs are the weights' FSE decode and the
sequence tables: small absolute numbers, but per frame they are now a
large share. Two candidates for a later pass:

- **Weights decode** (`decompress_weights_into`, ~9.7K per table): a table of
  at most 255 4-bit weights decoded through a freshly built FSE table each
  time.
- **A reusable decoder context.** One-shot `decompress` sets up per frame what
  a long-lived context could recycle (`huff_pool` already does this for the
  encoder). A `Decoder` handle that keeps its table buffers across frames
  would suit SpaceDB exactly: one per collection, thousands of tiny frames.

## Finding 4: the encoder's fixed cost on small rows (changes output)

Compressing a ~183-byte row costs about 108K instructions, and 72% of that is
`write_literals`: building the Huffman code (`huffman_nbits`,
`ctable_from_nbits`), writing its FSE-compressed tree (`write_tree_fse`), and
decoding the weights back (`decompress_weights_into` inside the encoder). A
Huffman section on ~100 literals carries a 20-40 byte tree and saves little.
libzstd raises the bar for small literal sections (`ZSTD_minGain`, and raw
literals under `COMPRESS_LITERALS_SIZE_MIN`); a similar minimum-gain rule
here would make small frames cheaper to write AND to read (no tree to
parse). **This changes compressed bytes** (format-compatible, ratio slightly
different), so it belongs behind the corpus ratio boards, not in the same
change as Finding 1.

## Reproduce

Scratch copy of this repo at `E:/ir-midkms/zstd-probe`, driver at
`E:/ir-midkms/zstd-drv`, the probe patch at
`E:/ir-midkms/patches/z1_lazy_x2.py <threshold>`, build with
`tools/zbuild.sh <label>`, measure with
`tools/zir.sh <label,label,...> all 96 256 ... 65536`. Toolchain and valgrind
in WSL (Rust 1.99, valgrind 3.26); the driver needs `--vgdb=no` on a Windows
mount.

---

## Outcome (applied 2026-10-14, branch `perf/small-frame-decode`)

Six bricks landed, each its own commit with its numbers; two probes and two
findings were refuted or pruned and are recorded below so they are not
re-tried. Decoded output is byte-identical throughout, compressed output is
unchanged, and every libzstd 1.5.7 frame in the conformance corpus (1,552
frames, 15 levels, 1 B-3 MB inputs) still decodes.

### Method

- **Instrument:** `sfd`, a callgrind driver modelled on SpaceDB (one row per
  frame, level 3, kept only if it shrank, `rusty_zstd::decompress`): five row
  shapes x 13 sizes (96 B-64 KiB) x 16 rows, plus a bulk instrument (6
  silesia slices x L1/L3/L19, 512 KiB) and dictionary load (16 KiB dictionary
  trained by libzstd on the rows). Anchors (frames, bytes, FNV of the output)
  printed with every count.
- **Allocator:** every number is also taken with **rusty_alloc 2.2.6** linked
  as the global allocator (as `mata-alloc` ships it) and with its `secure`
  feature; opt-level "s" + thin LTO as in mata-master.
- **Pedestal correction:** the per-frame figures subtract a zero-rep run with
  the SAME bucket filter. The first five commit messages used a whole-file
  pedestal, which over-subtracted: their absolute deltas are right, their
  percentages flattering -- mildly at most sizes (brick 4 at 256 B said
  -23.6%, corrected -21.1%) and badly in the 96 B bucket, which has only 18
  frames (brick 4 there said -24.7%, corrected -4.3%). Every figure on this
  page is the corrected one.
- **Wall clock:** one program compiled per arm, pinned to one core at High
  priority, rotated rounds, global minimum; null arm 0.94-1.02.

### What landed

| commit | brick | effect (rusty_alloc, Ir per frame) |
|---|---|---|
| `adcae3e` | Build X2 only for sections `select_x2` can use (>= 256 B); `ensure_x2` fills it lazily for a treeless section reusing an X1-only table | rows 96-384 B -75% to -86% |
| `e577fbf` | Build X2 only from 4 KiB, and only when `select_x2` will use it | rows 512 B-4 KiB -35% to -65% |
| `aedddcc` | Dictionary load reads the Huffman tree once (`read_tree_tables`) | dictionary load -34% |
| `a0da634` | Predefined sequence FSE tables built at compile time (decoder side of `default_ctable_cached`) | rows 128-512 B -15% to -22% (96 B -4%) |
| `264d653` | `rewind_to_start` returns early at `ptr == 0` (it was a no-op rebuild per reload) | rows -1% to -6% |
| `24c0b43` | Frame reserve includes `COPY_PAD` (no per-frame output realloc) | small rows ~-1%, 16-64 KiB -4% to -5%, bulk -3.1% |
| `56acdcb` | Four tree weights per reload (C's FSE fast loop) | rows 96-384 B -3% to -4% |

Cumulative, 0.3.1 -> `56acdcb`, rusty_alloc 2.2.6 (the `secure` build and
glibc agree to within 0.2 points):

| row size | Ir per frame | wall clock |
|---:|---:|---:|
| 96 B | 136,590 -> 16,628 (-87.8%) | 6.6x faster |
| 128 B | 142,424 -> 19,009 (-86.7%) | 5.4x |
| 256 B | 150,746 -> 23,984 (-84.1%) | 4.8x |
| 512 B | 157,817 -> 32,508 (-79.4%) | 4.1x |
| 1 KiB | 167,534 -> 49,746 (-70.3%) | 3.0x |
| 4 KiB | 238,788 -> 132,990 (-44.3%) | 1.8x |
| 16 KiB | -13.4% | 1.15x |
| 64 KiB | -5.8% | within the null arm |
| bulk / libzstd silesia | -3.5% | within the null arm |

Dictionary load -35.0%; encode -0.4% (it decodes its own trees).

### Refuted or pruned (with the numbers)

- **C's whole `HUF_selectDecoder` as the X2 build rule** (step 2 as written
  here): same as a 4 KiB threshold on rows, +2.9% Ir on bulk, no wall-clock
  gain anywhere. A size threshold won; the sweep (1 KiB-32 KiB, Ir and wall)
  is in `X2_BUILD_MIN`'s comment. The wall-clock crossover did NOT sit lower
  than the Ir one: 1 KiB and 2 KiB thresholds lost 14-41% in time.
- **A reusable decoder context / buffer pooling** (Finding 3b): with
  rusty_alloc every allocation of a small-row decode costs 246 Ir per frame
  (~1%; 292 with `secure`), and one of those is the output the caller keeps.
  Pruned on arithmetic. Under glibc it read ~6% -- the allocator matters for
  this question, which is why it was re-measured with the shipping one.
- **Finding 4, libzstd's minimum-literals rule:** every one of 978 row frames
  has a Huffman literals section of >= 64 bytes, so `ZSTD_minLiteralsToCompress`
  never fires; a probe produced byte-identical frames. The real cost it points
  at (Huffman construction in the encoder) is encoder work, not done here.
- `reload` testing `ptr != 0` inline (+0.3% to +0.7%), and growing the X1
  table by `resize` instead of a zero-fill (small rows +0.7% to +2.5%):
  recorded at their sites.

### What is left on a 256-byte row (~24K Ir)

The tree's weight decode with its NCount header (~28%), the X1 table build
(~21%), the literals themselves (~19%), and the sequences (~14%) -- each a
per-frame cost of the frame's own content, with no further constant to hoist.
