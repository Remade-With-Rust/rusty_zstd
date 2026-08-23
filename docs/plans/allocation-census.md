# allocation-census — the encoder's allocation campaign

**Opened 2026-08-22**, out of `inline-execution.md` §12.2, which named allocations "the
richest remaining vein" without measuring them. This document measures them first.

---

## 0. The headline, before the catalogue

**The allocation problem is the ENCODER's. The decoder does not have one.**

Measured with a counting global allocator over 12 Silesia corpora, 88 MiB, encode and
decode counted separately (`examples/alloccensus.rs`):

| level | ENCODE allocs | per MiB | bytes requested | DECODE allocs | per MiB |
| --- | ---: | ---: | ---: | ---: | ---: |
| L1 | 52,599 | **596** | 252.6 MiB | 176 | **2.0** |
| L3 | 63,572 | **721** | 241.3 MiB | 127 | **1.4** |
| L9 | 59,321 | **673** | 355.0 MiB | 132 | **1.5** |
| L19 | 65,857 | **747** | **1,187.3 MiB** | 142 | **1.6** |

**Encode allocates 400–500x more often than decode, per byte of data.** That ratio is
this document's equivalent of `inline-execution.md`'s "491 ymm against 27,862 xmm": it
sets the direction, and it says the decode side is *already solved*.

**Why decode is solved and encode is not** — decode has the W25/W26 recycling machinery
(tables handed back and reused). The encoder has `MatchTables` scratch recycling for a
few big buffers and nothing at all for the per-block entropy work. 97.3% of decode's
requested bytes is the 12 output buffers, i.e. one unavoidable allocation per frame.

**Do not "fix allocations" as one problem. There are two, and they take opposite cures:**

| | the COUNT problem | the BYTES problem |
| --- | --- | --- |
| shape | ~38,000 allocations under 256 B per 88 MiB — **60% of all encode allocations** | **48 allocations requesting 1,112 MiB** at L19 — 89.3% of bytes in 0.1% of calls |
| cost | allocator call overhead, ~40k x per-call cost | page-fault + memset + cache eviction on ~23 MiB blocks |
| cure | pool / reuse / stack-allocate; or delete the allocation | size once and recycle; stop re-growing |
| where | per-block entropy plumbing | match-table / opt-parse machinery on the Bt ladder |

## 0b. Encode allocation profile by size class (L3 and L19)

| size class | L3 count | L3 count% | L3 bytes% | L19 count | L19 bytes% |
| --- | ---: | ---: | ---: | ---: | ---: |
| <64 | 20,539 | 32.3% | 0.2% | 21,745 | 0.0% |
| 64..255 | 17,508 | 27.5% | 1.0% | 18,285 | 0.2% |
| 256..1K | 6,895 | 10.8% | 1.2% | 7,625 | 0.3% |
| 1K..4K | 8,555 | 13.5% | 5.0% | 9,079 | 1.1% |
| 4K..16K | 8,337 | 13.1% | **22.5%** | 7,765 | 4.1% |
| 16K..64K | 1,535 | 2.4% | 17.9% | 1,133 | 2.3% |
| 64K..256K | 156 | 0.2% | 9.2% | 151 | 1.6% |
| 256K..1M | 35 | 0.1% | 6.3% | 26 | 1.1% |
| **>=1M** | **12** | 0.0% | **36.7%** | **48** | **89.3%** |

**Read the L19 row.** Twelve huge allocations at L3 become forty-eight at L19, and the
bytes go 92.8 MiB -> 1,112 MiB. **L19 requests 13.5x the input size in allocation
churn.** That is not per-block plumbing; that is the Bt/opt machinery being re-grown.

---

## 1. Evidence this vein is real, already collected

Not a hypothesis. Two things landed in `inline-execution.md` before this document
existed, both allocation defects found while looking for something else:

- **N9** — `select_seq_table` rebuilt an RFC-constant FSE ctable on every call: **three
  heap allocations each, 1,949–2,628 times per 88 MiB**. Removing it deleted ~7,500
  allocations as a *side effect of a redundancy fix*. Byte-identical, shipped.
- **N4** — `upsample_dtable` zero-filled 4 KiB it immediately overwrote, ~1,400 times
  per run. Byte-identical, shipped.

Both were found by reading, not by an allocation instrument. The instrument now exists.

## 2. Known sites, inherited from `inline-execution.md` §11

These were catalogued there and rehomed here because they are allocation items, not
inline/SIMD items. **None has been priced against the census above** — that is step 1.

| id | site | shape | first guess at class |
| --- | --- | --- | --- |
| **N11** | `FseCTable::clone()` on the repeat-table path | 2 allocations per accepted repeat, x3 tables, per block | COUNT |
| **N16** | `normalize_count` | allocates per call | COUNT |
| **N17** | `write_ncount` / `ncount_seq_table` | a `Vec<u8>` per call | COUNT |
| **N18** | `segment_histograms` | heap-allocates its result | COUNT |
| **N19** | `encode_4_streams` | builds four `Vec<u8>` then concatenates | COUNT + copy |
| **N3** | `stream.rs` decode-side `drain` x6 | O(n) memmove per call, not an allocation but the same plumbing | COPY |
| **N14** | `stream.rs` encode-side `drain` x4 | same; the encode side already *has* `in_off` and drains anyway | COPY |

## 2b. STEP 1a ANSWERED — ~78 allocations per block, and it is level-independent

Measured by differencing input sizes, which cancels the fixed per-frame cost entirely
(`examples/allocperblock.rs`, dickens 1/2/4/8 MiB):

| level | marginal allocations, 1→8 MiB | **per 128 KiB block** | per MiB (flat across sizes) |
| --- | ---: | ---: | ---: |
| L1 | 4,397 | **78.5** | 628–664 |
| L3 | 4,371 | **78.1** | 627–642 |
| L9 | 4,261 | **76.1** | 613–646 |
| L19 | 4,475 | **79.9** | 640–650 |

**Three things fall out, and they redirect the campaign:**

1. **It is per-BLOCK, not per-frame.** The per-MiB rate is flat from 1 MiB to 8 MiB, so
   nothing is amortising. Every block pays ~78 allocations.
2. **It is level-INDEPENDENT.** 76–80 at every level from L1 to L19. The finder ladder
   changes completely across that range (Fast → DFast → Lazy → BtUltra2) and the
   allocation count does not move — so **the COUNT problem is not in match-finding at
   all.** It is in the shared per-block entropy/emit path, which is the same code at
   every level.
3. **N11, N16, N17, N18 are therefore very likely ONE fix, not four.** They are all
   per-block entropy-path allocations, and the flat level-independence says they share
   an owner. **A single per-block scratch struct on `MatchTables` — the pattern the
   codec already uses for `coded_scratch` / `payload_scratch` / `bits_scratch` — is the
   candidate, and it should be priced as one brick before any of the four is built
   separately.**

This also reprices the L19 `>=1M` byte problem as **fully independent**: it is 48
allocations that do *not* scale with block count, sitting in a different mechanism, and
it should stay a separate item rather than being bundled into the scratch struct.

## 3. What is NOT yet known, and must be measured before any of it is built

**The census says how many and how big. It does not say WHERE.** Attribution is the
missing half and it is step 1 of this campaign.

Cheapest attribution first:

1. ~~Allocations per BLOCK~~ — **DONE, see §2b: ~78/block, level-independent.**
2. **Tag the counting allocator with the active `prof::Stage`.** The stage profiler
   already exists; a thread-local "current stage" read inside `alloc` attributes every
   allocation to a stage for free. That turns the size table into a site table.
3. **Only then** rank N11/N16–N19 against the measured shares.

## 4. The discipline (inherited, non-negotiable)

Everything `inline-execution.md` learned applies here unchanged:

- **Gate byte-identical.** `examples/bytegate.rs` — 18 corpora x 9 levels folded to one
  number, currently `GOLD BE0071FB0CB0CED9`. An allocation fix that changes the
  bitstream is a bug, with no exceptions.
- **Counter first, clock never.** This box runs at ~70% CPU from a neighbouring job and
  its pinned CPU-time instrument is quantised to the 15.6 ms Windows tick. Every verdict
  in the parent document was a count. `alloccensus.rs` is deterministic — same numbers
  on any machine at any load.
- **Prune on arithmetic before building.** The parent campaign refused eight of fourteen
  scheduled items on ceiling probes, including its own highest-rated one. Expect the same
  rate here: a site that is 0.1% of allocations cannot repay a restructure.
- **Eligibility is not volume** (D6: 99.6% eligible, negligible entries). Ask how many,
  not how often it *could*.
- **Provenance is content** (N21, N1 both inverted on C-zstd frames). Less likely to bite
  on the encode side — we produce those streams — but the decode-side items (N3) must be
  harvested on foreign frames.
- **Verify the instrument before believing a zero.** Three separate probes in the parent
  campaign read zero for instrument reasons: two were `profile`-gated and run without the
  feature, one dispatched on `ptr::eq` against a `const` (which is inlined per use site,
  so the addresses never match). A zero is a trigger to check the probe.

## 5. Execution order (provisional — step 1 may reorder everything)

| # | item | why here |
| --- | --- | --- |
| 1 | **Stage-tagged allocation attribution** | the census says how many, not where; nothing else is rankable until this exists |
| 2 | **The L19 `>=1M` class** | 48 allocations, 1,112 MiB, 89.3% of bytes. Highest bytes-per-fix in the codebase; likely one or two sites |
| 3 | **The per-block scratch struct** | §2b confirms ~78/block and level-independent, so N11/N16/N17/N18 share an owner — price as ONE brick |
| 4 | N19 `encode_4_streams` | four Vecs + a concatenate is also a COPY item |
| 5 | N3 / N14 front-drains | `codec-memory-copies`' opening pattern; the encode side already has the cursor it declines to use |

**Not scheduled:** anything sub-0.5% of either count or bytes, until steps 1–3 land and
the profile is re-taken. The parent campaign's clearest lesson is that the bottleneck
moves after every win.

## 6. Instruments

| instrument | what it answers | status |
| --- | --- | --- |
| `examples/alloccensus.rs` | count + bytes, bucketed by size, encode vs decode, per level | **built, §0 is its output** |
| `examples/allocperblock.rs` | allocations per block, by differencing input sizes | **built, §2b is its output** |
| `examples/allocs.rs` | decode FSE table allocations specifically (W26 recycling check) | pre-existing |
| `examples/allocost.rs` | what ONE fresh per-block buffer costs, as a pure microbenchmark | pre-existing |
| `examples/cpmem.rs` | bytes per frame as arithmetic rather than a clock | pre-existing |
| stage-tagged allocator | **which site** — the missing half | **step 1** |

---

## 7. DEPLOYED — 2026-08-22. Encoder allocations cut ~83%.

Eighteen bricks, all byte-identical, all gated on `GOLD BE0071FB0CB0CED9`.

### Result

| | before | after | cut |
| --- | ---: | ---: | ---: |
| **allocations per 128 KiB block** (L1) | 78.5 | **13.4** | **−83%** |
| per block (L3 / L9 / L19) | 78.1 / 76.1 / 79.9 | **17.2 / 16.7 / 18.8** | −78% / −78% / −76% |
| **encode allocations per MiB** (L1) | 596 | **126** | **−79%** |
| per MiB (L3 / L9 / L19) | 721 / 673 / 747 | **155 / 142 / 170** | −79% / −79% / −77% |
| total, 88 MiB @ L3 | 63,572 | **13,602** | **−49,970** |
| total @ L1 / L9 / L19 | 52,599 / 59,321 / 65,857 | **11,071 / 12,502 / 14,998** | −79% / −79% / −77% |
| bytes requested @ L1 | 252.6 MiB | **120.6 MiB** | −52% |
| **encode : decode ratio** | 400–500x | **64–110x** | — |

### The bricks

| id | site | what it was | fix |
| --- | --- | --- | --- |
| **ALLOC-1** | `FseCTable::from_norm` | `table_symbol` + `cumul`, pure scratch, per candidate table per block | leased |
| **ALLOC-2** | `encode_4_streams` + `encode_stream` | six allocations per call: the `streams` Vec and four `BitCStream::with_capacity` buffers | pooled; wired to the **already-existing** `BitCStream::from_vec` |
| **ALLOC-3** | `huffman_nbits` | four pure-scratch Vecs (`present`, `nodes`, `leaves`, `internal`) | leased; `Node` hoisted to module scope so its arena could be |
| **ALLOC-4** | `fse::write_ncount` | `Vec::new()` grown by `push` — a realloc per doubling | sized once |
| **ALLOC-5** | `select_seq_table` (N11) | on Repeat mode, cloned `entropy.<x>` and assigned the clone back onto itself | borrow via `SeqTable::Ref`; write back only a genuinely new table |
| **ALLOC-6** | `segment_histograms` | returned a fresh `Vec<[u32; 256]>` per block | `_into` form + leased at the one caller |
| **ALLOC-7** | Predefined retained table | cloned the process-constant table to retain it | `RetainedTable::Static` — retain by reference. **Below measurement** on this corpus (see below) |
| **ALLOC-8** | `EntropyState` speculative save | **8–12 allocations per block**, deep-copying 3 FSE tables + a Huffman table for a rollback that **fires 0 times in 707–876 saves** | tables behind `Arc` — the save is four refcount bumps |
| **ALLOC-9** | `FseCTable`'s `state_table` + `delta` | owned BY the table, so unleasable; built per candidate per seq table per block and dropped when the candidate loses | bounded `Drop` free list (8 entries, ~24 KiB/thread) |
| **ALLOC-10** | Huffman `table` + `table_x2` (4 KiB + 8 KiB) | `table_from_weights` already took a `recycle` parameter — the decoder passed one, **both encoder sites passed `None`** | `Drop` free list on `HuffCTable` (the inner type can't carry it — see below) |
| **ALLOC-11** | `HuffUpdate::New(ct.clone())` | cloned a 12 KiB table into a branch whose only sibling *moves* it | move |
| **ALLOC-12** | literal-section `body` | built per candidate, copied into the section, dropped | pooled; `encode_4_streams_into` |
| **ALLOC-13** | literal-section candidates + header | every candidate Vec, INCLUDING the winner — which `write_literals_inner` copies into `dst` and drops | pooled end-to-end, header built on the pooled buffer |
| **ALLOC-14** | `normalize_count`, `write_ncount` | both escape to `ncount_and_ctable` / `select_seq_table` and die after being copied out | pooled, with give-backs at the death sites |
| **ALLOC-15** | `write_tree` | builds BOTH a raw and an FSE encoding and drops the loser; the winner dies at the end of `encode_literals_section` | pooled at both death sites |
| **ALLOC-16** | the LOSING ncount header | dropped in `select_seq_table`'s `else`, so it never reached ALLOC-14's pool — which is why `write_ncount` stayed the top site afterwards | give-back on the losing branch |
| **ALLOC-17** | `HuffCTable::weights_wo_last` | the table's third owned buffer, moved in by `finish_ctable` | added to the ALLOC-10 `Drop` free list |
| **ALLOC-18** | the FSE tree-encoding verification | round-tripped the encoding through the ALLOCATING `decompress_weights` and threw the `Vec` away one line later | `decompress_weights_into` + a 256-byte stack buffer |

**ALLOC-2 is another V1 recurrence.** `BitCStream::from_vec` already existed, documented
as *"Frame-scratch constructor: reuse a caller-kept buffer so the per-block bitstream
costs no allocation after warm-up"* — with exactly **one** caller. The two Huffman
literal-stream sites called `with_capacity` and allocated fresh, four times per block.
Third time this campaign that the right helper existed and the hot path did not call it.

### The mechanism: `src/scratch.rs`

A `Lease` takes a `Vec` from a thread-local slot and returns it on drop — including on
the `?` exits that make hand-written take/restore pairs leak the buffer. Two lines per
site, no signature churn, independently revertible, and MT-safe for free (per-thread
slots, no sharing, no lock). A `BlockScratch` threaded through `fse.rs`/`huffman.rs`/
`encode.rs` is the tidier end state; these leases collapse into it unchanged if it is
ever built.

### Two bugs the gates caught, both worth keeping

1. **`lease()` clears — which destroys a POOL.** `Vec<Vec<u8>>` needs its inner buffers
   kept, and `clear()` drops every one. The first `encode_4_streams` attempt recycled
   nothing and measured ~1 allocation/block instead of ~5. The drop rule was wrong the
   same way: it compared `capacity()`, which for a pool is the *slot count* — equal on
   both sides, so the pool was discarded every time. Fixed with `lease_pool` (no clear)
   and `cur.is_empty() || …` in Drop.
2. **`bytegate` caught a real bitstream change in ALLOC-5.** Returning
   `SeqTable::Ref(cached)` for **Predefined** mode looked free and was not: the caller
   writes the returned table into `entropy.<x>`, which becomes the NEXT block's `prev`.
   Skipping that write-back left the previous compressed table in place, the next
   block's Repeat test saw a different `prev`, and mozilla diverged. `SeqTable::Ref` now
   means exactly one thing — *"this IS the table `entropy` already holds"* — and is
   produced only on the Repeat path.

**And one wrong assumption the compiler caught**: `normalize_count`'s `n2` is *returned*,
not scratch. Reverted before it could ship.

### Gates

`bytegate` GOLD `BE0071FB0CB0CED9` unmoved · **137/137 tests release + debug** ·
**49 external frames** through C zstd v1.5.7, 0 failures · aarch64 cross-check clean ·
**determinism: 0 non-deterministic of 216** (each corpus preceded by all 17 others) and
0/12 across repeated calls — the gate that matters most, because thread-local buffers
carry state across calls and a partially-written lease would make output depend on
history.

### ALLOC-8 is the sharpest measurement of the campaign

`encode_block` snapshots `EntropyState` before trying the compressed encoding so it can
roll back if Raw or RLE wins. The probe:

| level | saves | rollbacks | used |
| --- | ---: | ---: | ---: |
| L1 / L3 / L9 / L19 | 707 / 837 / 796 / 876 | **0 / 0 / 0 / 0** | **0.00%** |

Every snapshot was a full deep copy of four entropy tables, and **every single one was
discarded**. `Arc` is sound because nothing mutates a table in place — every use is a
read (`as_deref`) or a whole-value assignment — so there is no writer to share with.
`Arc` rather than `Rc` keeps `Compressor` `Send`.

### ALLOC-7 landed and moved nothing — recorded, not hidden

Retaining the Predefined table by reference is strictly less work and is byte-identical
and determinism-gated, but the per-block count did **not** change. That is consistent
with N21's independent measurement: our encoder selects Predefined mode rarely, so the
clone it removes is rare *on this corpus*. Kept as below-instrument-resolution rather
than claimed as a win.

### The BYTES problem: measured, and NOT a defect

L19 requests ~1,157 MiB for 88 MiB of input, 89.3% of it in ~48 allocations. Attribution
puts it in `MatchTables::new` — **83.9 MB in 2 allocations** for one 8 MiB input, i.e.
the hash and chain tables, sized from `hash_log`/`chain_log`.

The obvious fix — pool `MatchTables` across `compress()` calls — was probed and refused:

- Reuse requires `MatchTables::reset()`, which is `.fill(0)` over the **whole** table.
- Fresh `vec![0; n]` gets lazily-zeroed pages; the OS materialises only what is touched.
- **The probe refuted the easy version of that argument**: positions (8.4M for an 8 MiB
  input) exceed hash slots (4.2M) at every level, so the hash table *is* fully touched
  and lazy zeroing buys nothing there. Only L19's chain (2^24 slots) is under-touched.
- So pooling trades page-faults for an equivalent memset, saving only the mapping
  syscalls — while retaining ~80 MiB per thread indefinitely, which is a bad default for
  a library.

**The streaming path already does the right thing**: `Compressor` keeps its `MatchTables`
across blocks and calls `reset()` (`stream.rs:301`). A caller compressing many buffers
should reuse a `Compressor`, exactly as libzstd expects a reused `ZSTD_CCtx`. The
one-shot `compress()` allocating per call is correct behaviour for a one-shot API, not a
leak. **Closed as not-a-defect.**

### The escaping-buffer problem, and why the `Drop` sat where it did

ALLOC-9/10 are the two the earlier pass called "buffers that genuinely escape their
constructor". They cannot be leased — they outlive the function that builds them — but
their lifetime is still short and bursty, so closing the loop at `Drop` recycles them
with no signature change anywhere.

**`HuffmanTable` could not carry the `Drop`.** `table_from_weights` does
`Some(h) => (h.table, h.table_x2)`, moving both fields out, and Rust forbids moving out
of a type with `Drop`. `HuffCTable` owns it and is never destructured, so the impl goes
there and reaches the buffers with `mem::take` — a mutation, not a move-out. That one
constraint decided the whole shape.

**ALLOC-10 is the fourth V1 recurrence in this campaign.** The recycling parameter
existed, was documented, was used by the decoder, and both encoder call sites passed
`None`. After V1 (xxh64 kernel), N9 (default ctables), and ALLOC-2 (`BitCStream::from_vec`),
this is a pattern worth stating plainly: **in a mature codec, assume the mechanism you
need already exists and check who calls it, before building a new one.**

### ALLOC-13: a pool that starves measures nothing

Pooling the section candidates alone moved the number **not at all** — 24.0/block before
and after. The winner escapes `encode_literals_section` to `write_literals_inner`, which
`extend_from_slice`s it into `dst` and drops it; nothing ever returned to the pool, so
every candidate drew from an empty one. One line in `encode.rs` — `sec_pool_give(sec)`
after the copy — closed the loop and took it to **21.0**.

**A recycling pool is only as good as its return path, and the return path is easy to
leave open when the value crosses a module boundary.** The measurement is what caught
it; the code looked correct either way.

### FIVE times the mechanism already existed

ALLOC-18 is the fifth. Each time, a recycling or in-place variant was already written,
documented and used by one caller, while another caller allocated:

| # | the mechanism | who used it | who did not |
| --- | --- | --- | --- |
| V1 | `stripes_hybrid` (AVX2 xxh64) | `xxh64_seed` (a test + a bench) | the encoder, decoder and streaming API |
| N9 | `default_ll_ctable()` and siblings | tests only | `select_seq_table`, 22–30x per MiB |
| ALLOC-2 | `BitCStream::from_vec` | the sequence bitstream | both Huffman literal-stream sites |
| ALLOC-10 | `table_from_weights`'s `recycle` | the decoder | both encoder call sites |
| ALLOC-18 | `decompress_weights_into` | the decoder ("W39: no Vec, no copy") | the encoder's tree verification |

**This is the single most productive question in the campaign: before writing a
mechanism, grep for it and check who calls it.** It found more than every other
technique combined.

### Three wrong assumptions, all caught before they shipped

The compiler and the gates refused three things I was confident about:

1. **`n2` in `normalize_count` is returned, not scratch** — caught by the type checker.
2. **`weights` in `ctable_from_nbits` is moved into `finish_ctable`** — same.
3. **`SeqTable::Ref(cached)` for Predefined changed the bitstream** — caught by
   `bytegate` on mozilla, because the returned table becomes the NEXT block's `prev`.

Two more were caught by measurement rather than by a compiler: the section pool that
starved (ALLOC-13) and the losing-header path that never returned (ALLOC-16). In both
the code looked correct and the number did not move.

### What is left

**13.4 allocations per block remain, down from 78.** Every class the census opened with
is now closed:

| class | status |
| --- | --- |
| pure scratch (leasable) | **closed** — ALLOC-1/3/6 |
| grown-by-push buffers | **closed** — ALLOC-4 |
| clones of things already owned or borrowable | **closed** — ALLOC-5/7/8/11 |
| buffers owned by a returned value | **closed** — ALLOC-9/10 (`Drop` free lists) |
| build-then-concatenate output sections | **closed** — ALLOC-2/12/13 (pooled end-to-end) |
| unwired recycling mechanisms that already existed | **closed** — ALLOC-2/10, and V1/N9 before them |

**Every site this campaign named is now closed**, including the last two
(`ctable_from_nbits`'s `weights` -> ALLOC-17, the FSE tree verification -> ALLOC-18).

The final attribution over one 8 MiB corpus at L3 (32,001 allocations before the
campaign, **4,569 after**) shows no site above 10 sampled hits, and the largest are:

| site | est. per block | what it is |
| --- | --- | --- |
| `select_seq_table` | < 1 | `FseCTable::rle`'s single-byte header, and `basic_owned` when the N9 cache declines |
| `write_literals_inner` | < 1 | the Raw/RLE fallback section |
| `build_ctable_from_freq` | < 1 | residue inside `finish_ctable` |

**These are the floor, not a backlog.** Each is a buffer that is the output, or a
rarely-taken fallback. There is no remaining clone, no redundant pass, no
build-then-concatenate, and no unwired mechanism.

**Before any further work here, re-run `alloccensus` and `allocsites`.** The bottleneck
moved eight times during this campaign and every ranking written down went stale within
three bricks.

The **BYTES** side is closed as not-a-defect (see above): the L19 volume is
`MatchTables`, it is genuinely needed, and the reusable `Compressor` already avoids
paying it per call.

---

## 8. HUFF-1 / HUFF-2 — the encoder was building a DECODER, 2026-08-22

Asked whether more was available from `HuffCTable`, and the field declaration answers it:

```rust
/// Decode twin kept as the test oracle (`ct.table.decode_stream`).
#[allow(dead_code)]
table: HuffmanTable,
```

`HuffmanTable` owns a 4 KiB X1 table and an 8 KiB X2 table. Release **encode** code reads
only the X1 half, once, at build time, to derive `entry[]` and `code[]`. The X2 half is
read solely by `HuffmanTable`'s DECODE methods — which the encoder never calls. The
tests-module boundary confirms it: every other read of `ct.table` sits after
`#[cfg(test)]`.

**Measured on encode only, no decompress in the loop:**

| level | X2 tables built | X2 tables used |
| --- | ---: | ---: |
| L1 / L3 / L9 / L19 | 665 / 754 / 712 / 750 | **0 / 0 / 0 / 0** |

Every one wasted, and each is a **2048-entry data-dependent gather**.

| brick | what it removes | per 88 MiB encoded |
| --- | --- | ---: |
| **HUFF-1** | `x2_from_x1_into` on the encoder path | ~**1.4–1.5M gather + store ops** |
| **HUFF-2** | `upsample_dtable`'s 2048-entry replication, same path | ~**1.4–1.5M u16 writes** |

HUFF-2 was the riskier of the two and the source had already answered it: the upsample
exists so one decoder peek can pair more symbols, and the comment beside it notes the
encode codes (`idx >> (max - nb)`) are *unchanged* across it. `max_nbits` is derived from
the `nbits` array, not `table.max_bits`, so the unroll-width dispatch does not move
either. `bytegate` confirmed: GOLD unmoved.

**The oracle survives.** Both call sites pass `cfg!(test)` rather than `false`, so test
builds still construct the full decode twin and
`ct.table.decode_stream` is still asserted against — the safety property that made the
field worth keeping is intact, and release pays nothing for it.

**This is not an allocation win** — ALLOC-10 had already pooled both buffers, and the
per-block allocation count is unchanged at 13.5. It is a WORK win, and it would have been
invisible to the allocation census that found everything else in this document. The
counter that found it was `take_x2_stats()`, built for N2 and reused.

**Decoder unaffected:** its call site still passes `true`, and C-zstd frames still build
1479 X2 tables and use 1459 of them (98.6%), exactly as before.

