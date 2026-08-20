# M7 anatomy — the optimization campaign, side by side with its own evidence

**Date:** 2026-08-20. Companion to [`m7-anatomy.md`](m7-anatomy.md), which measures
us against facebook/zstd v1.5.7. This one measures the OPTIMIZATION WORK: what was
shipped, what was refused, and — the part worth the most — which instruments could
decide anything at all.

> **THE INSTRUMENT RULE THAT GOVERNS EVERY NUMBER BELOW.** The per-frame clock cannot
> decide changes at this scale. A **null arm** — one setting measured against
> ITSELF — reads up to **±24.15%** on encode (`jsonlog` +24.15%, `x-ray` +13.82%) and
> **±25%** on decode. Two findings and one whole dispatch were built on readings inside
> that band and had to be thrown away. Every shipped item here rests on a
> **deterministic** instrument — instructions counted from emitted asm, allocator
> traffic through a counting `GlobalAlloc`, work counters, or byte-identity — never on
> a stopwatch.

> **NEVER AVERAGE THESE FILES.** Same rule as the parent document. Where a mean is
> quoted it is quoted beside its worst corpus, and a mean that hides a regression is
> treated as a refusal.

---

## 1. What shipped

Every row is **byte-identical** unless the row says otherwise, and every row was
verified against a fingerprint board, not by inspection.

| # | change | instrument | result |
| --- | --- | --- | --- |
| 1 | DFast packed rejection tag (T1) | candidate loads avoided | **29.8%** of non-empty short slots — 2,938,472 loads/pass |
| 2 | Match-copy tier order | copy instructions | **5,452,942** calls moved to a 2-instruction copy |
| 3 | AVX2 sequence-loop twin | copy instructions | **12,054,710** — lowest of every arrangement |
| 4 | `tag_min` 0.50 → 0.00 | loads avoided | 1,859,598 → **4,538,058** (+2,678,460) |
| 5 | Finder scratch, Greedy/Lazy/BtLazy | realloc bytes | L5 **172,166,620 → 8,075,964 (−95.3%)** |
| 6 | `find_opt` DP arrays on the frame | ≥128 KiB allocations | L19 **1,920 → 559** |
| 7 | `opt_ops` on the frame | realloc bytes | L19 **340,718,933 → 1,322,651 (−99.61%)** |
| 8 | Codec-path bounds checks | panic sites | **228 → 106** |
| 9 | `huffman.rs` bounds checks | panic sites | **91 → 0**, file −7.5% |
| 10 | `ldm_hash` load coalescing | loads | 16 byte loads → **2** |
| 11 | Arm hoisting (`lut_on`, `matchcopy_on`) | atomic reads | 2/sequence and 3/call → **1/block** |

### 1.1 The allocation family — one defect, four homes

`find_fast_impl` had been given a frame scratch buffer; its neighbours had not.

| finder | levels | before | after |
| --- | --- | --- | --- |
| `find_greedy` | L5 | `Vec::new()`, no reserve | frame scratch |
| `find_lazy` | L6–L12 | `Vec::new()`, no reserve | frame scratch |
| `find_bt_lazy` | L13–L15 | `Vec::new()`, no reserve | frame scratch |
| `find_dfast_impl` | L3/L4 | reserved, but per block | frame scratch |

Bytes memcpy'd by `realloc`, 18-corpus 8 MiB board:

| board | before | after | |
| --- | ---: | ---: | ---: |
| L5 | 172,166,620 | 8,075,964 | **−95.3%** |
| L9 | 164,212,638 | 8,100,305 | **−95.1%** |
| L13 | 153,983,310 | 7,926,989 | **−94.9%** |
| L3 | 9,172,321 | 6,899,677 | −24.8% |

**164 MB removed per pass at L5.** Unlike `opt_ops`, these buffers hold LIVE data, so
every doubling was a real memcpy of real bytes. Byte-identical on **198 (corpus, level)
cells**, A/B'd in-process through the arm — a build-to-build fingerprint could not
attribute anything while a second session was editing `encode.rs` live.

### 1.2 The bounds-check family

| function | before | after | frequency |
| --- | ---: | ---: | --- |
| `huffman::decode_4x` | 48 | **0** | per literal |
| `huffman::decode_into_x2` / `x1` | 15 | **0** | per literal |
| `huffman::encode_stream` | 8 | **0** | per literal |
| `bt_find_best` | 132 | **0** | per tree node |
| `find_dfast_impl` | 13 | **0** | per table access |
| `FseTable::entry` | 7 | **0** | 3× per sequence |
| back-extension walk (×3 finders) | 6 | **0** | per byte extended |
| `reader::peek_u32_le` | 4 | **0** | per block |

---

## 2. What was refused, and why

A refusal with a number beside it is worth as much as a win.

| candidate | verdict | evidence |
| --- | --- | --- |
| `pair_rep_max` 0.7 → 0.2 | **REFUSED** | TRAIN −0.79% but **HOLDOUT +0.096%**, worst corpus `nci` **+1.19%** |
| AVX2 on the 32-byte copy alone | **REFUSED** | `target_feature` cannot inline into a baseline caller → `call` + 2 `vmovups` + `vzeroupper` + `ret` replacing 4 inline SSE instructions |
| `huffman::encode_literals_section` reuse | **REFUSED** | ~63 µs against a 96 ms frame = **0.07%** |
| Greedy/Lazy chain accessors | **REVERTED** | panic count 10 → 10, `find_lazy` grew 807 → 1,141 instructions (+41%) |
| Gate 6 exact-vs-blanket dispatch | **DISSOLVED** | a real content split that vanished once the underlying dead-byte memcpy was fixed |

### 2.1 Open — a priority call, not a technical one

Both pass worst-corpus and both generalise to holdout. Both buy size with time at the
level whose purpose is speed, so neither was shipped unilaterally.

| candidate | TRAIN | HOLDOUT | worst corpus | time cost |
| --- | ---: | ---: | --- | --- |
| `pair_gain_min` 1.0 → 0.25 | −2.34% | **−1.83%** | `x-ray` **+0.0003%** | +23…**+38%** |
| `pair_rate_hi` 1.0 → 4.0 | −1.91% | **−0.99%** | `osdb` **+0.0075%** | +10…+25% |

`dickens` is the shape of the trade: **−6.96% size for +32.6% time.**

---

## 3. Prometheus adjudication — the fitted constants

No `Prometheus/` workspace exists in this repo, so the full refinery (harvest → symreg
→ SMT → forge) is unavailable. The question it asks FIRST is available, and decides
whether the rest is worth running: **is each fitted constant live, inert, or
mis-fitted?**

| constant | shipped | verdict |
| --- | ---: | --- |
| `pair_gain_lo` | 0.71 | LIVE, best at the shipped value |
| `rep_yield_min` | 0.10 | LIVE, best at the shipped value |
| `G5_REP_MIN` | 0.30 | LIVE, best at the shipped value |
| `dfast_spec_min` | 0.50 | LIVE **on the work axis** — spec made 8.1M → 539K across the sweep |
| **`tag_min`** | **0.50** | **MIS-FITTED → 0.00** |
| `pair_rep_max` | 0.70 | mis-fitted but REFUSED on worst-corpus |
| `pair_gain_min`, `pair_rate_hi` | 1.0, 1.0 | mis-fitted, OPEN (§2.1) |

**`tag_min` swept on the work axis**, candidate loads avoided out of 8,248,621 probes:

| `tag_min` | avoided | share |
| ---: | ---: | ---: |
| **0.00** | **4,538,058** | **55.0%** |
| 0.25 | 2,055,500 | 24.9% |
| 0.50 *(was shipped)* | 1,859,598 | 22.5% |
| 0.90 | 356,859 | 4.3% |
| 1.00 | 29,487 | 0.4% |

The mechanism: `store_fast` writes the tag **unconditionally**, while only the COMPARE
was gated. A high `tag_min` pays the store and then declines to use it.

> **A CORRECTION TO THE METHOD, recorded because it nearly cost two findings.**
> `tag_min` and `dfast_spec_min` were first adjudicated INERT because a SIZE sweep moved
> nothing. Both gate byte-identical mechanisms — the tag cannot hide a match, a
> speculation is either consumed or discarded — so size-invariance was evidence the
> WRONG AXIS had been measured, not evidence of inertness. Re-measured on work, one was
> mis-fitted and the other live.

---

## 4. What the instruments actually cost

Priced so nobody re-derives them.

| quantity | measured |
| --- | --- |
| per-frame clock null floor, encode | **±24.15%** |
| per-frame clock null floor, decode | **±25%** (±4.6% after warm-up settles) |
| one uncached `std::env::var` | **115.6 ns** vs 0.2 ns cached |
| fresh vs reused buffer, 128–512 KiB | **±0** — the sign flips |
| fresh vs reused buffer, **1 MiB** | **392,206 ns vs 33,277 ns (+1078%)** |

> **THE 128 KiB THRESHOLD WAS A DETECTOR, NOT A PRICE.** Three Gate 6 cells were argued
> partly on "allocations ≥128 KiB become VirtualAlloc". Measured, there is **no cliff
> there** — a fresh 128–512 KiB buffer costs what a kept one costs, and the sign even
> flips. The cliff is at **1 MiB**. That is why the huffman allocation work was declined
> at ~138 KB and why `find_opt`'s `prev` array, at exactly **1.00 MiB**, was not.

---

## 5. The recurring defect — five instances

The single most reusable finding of the campaign: **a capability present in one path
and absent in its neighbour.**

| # | capability | had it | missing it |
| --- | --- | --- | --- |
| 1 | packed rejection tag | `find_fast_impl` (28 uses) | `find_dfast_impl` (`PACKED=false`) |
| 2 | copy tier order (16 before 32) | `copy_literals` | `copy_match` |
| 3 | arm hoisted to a parameter | `copy_literals(.., arm)` | `copy_match`, `ll_code`/`ml_code` |
| 4 | frame scratch buffers | `find_fast_impl` | `find_greedy`, `find_lazy`, `find_bt_lazy` |
| 5 | `get_unchecked` under brick 69's proof | `emit_fill` | `emit_k5`, `emit_k` |

**"Does its neighbour have this?" is now a standing check**, not an observation.

---

## 6. Latent bugs found while optimizing

Three, none of which the optimization was looking for.

1. **`bt_find_best` addressed index 3 of a 2-entry table.** `bt_log` came from the const
   generic `CLOG` rather than the table, and `btlog` floors at `.max(1)`, so the tree
   needs `chain.len() >= 4` while the only guard was `chain.len() < 2`. Reachable with
   `chain_log = 1` through the advanced API. The bounds check was **load-bearing** —
   removing it blindly would have converted a panic into UB. Fixed by guarding the
   worst case the tree can form.
2. **`read_table`'s twin had no bound of its own.** It computed `_nbytes` and discarded
   it, and was safe only because `read_table(src)` two lines earlier validated the same
   bound for the same header. An indirect argument across a call boundary; now explicit.
3. **`normalize_count` and `FseCTable::from_norm` could index an empty table.** `mask =
   len.saturating_sub(1)` turns an empty table into `mask == 0` and then indexes it.
   Five sites now reject it — a correctness fix on untrusted input, not merely an
   enabler for the `unsafe`.

---

## 7. What this says to do next

Ranked by evidence, not by appetite.

1. **The gap is not at the gates.** `C/us c` is **1.93 → 2.95, mean 2.36** across 14
   unrelated corpora; we BEAT C on exactly the four where the search loop does not run
   (`zeros` 0.26, `text` 0.50, `versions` 0.68, `incomp` 0.90). A flat multiplier across
   unrelated content is a fixed per-position cost. Decode is **2.04×** behind with
   almost nothing to gate — decisive on its own.
2. **`find_sequences_strategy`** — 17 panic sites, 4,385 instructions, per block, the
   last warm cluster. Everything after it is setup code where the checks are doing their
   job.
3. **DFast register pressure.** The hot loop is only **151 instructions** but carries 27
   reloads, 23 of them loop-invariant across **12 distinct slots**. It is short of
   registers, and what it spends them on is **the gates' own telemetry** — ~9
   accumulators feeding `rep_yield`, `dfast_spec_yield`, `nl_off_worse`. Reducing that
   means changing how gate signals are gathered, so it belongs to a gate cell.
4. **18 uncached `env::var` accessors remain**, ~1,875 reads per 32 MiB (~217 µs).
   Mechanical.

**Do NOT pursue a zero panic census.** 13 of the remaining sites are core/std (vtable
shims, quicksort, backtrace) and cannot be removed at all; 74 more are setup paths
(`train` 27, `dict` 13, `seekable` 12, `mt` 9) where a check costs nothing and guards
user-supplied input. The census earned its keep by LOCATING the per-literal
concentration, not by its total.
