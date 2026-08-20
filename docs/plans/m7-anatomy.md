# M7 anatomy - rusty_zstd vs facebook/zstd v1.5.7, side by side

**Date:** 2026-08-17 - sections 1, 2 and 5 regenerated after the **literals entropy
gate** (`ee9a2eb`) and the **chain-fill fixes** (`9a7250e`, `c2f8d63`). Section 6 is
new. Older boards are in git history.

> **READ THIS BEFORE COMPARING TO THE 2026-08-16 BOARD.** That board predates
> `ee9a2eb`, and it is stale IN OUR FAVOUR on speed and AGAINST us on ratio.
> `literals_worth_huffman` was measuring PEAK FREQUENCY where the deciding quantity is
> ENTROPY, so flat-alphabet files fell out to RAW literals. Fixing it moved ratio hard
> (`smallmsg-8m` L1 1.615 -> 1.031, `jsonlog-16m` 1.528 -> 1.061, `webster` 1.443 ->
> 1.137, `x-ray` 1.212 -> 1.000) and cost real speed on exactly those files, on BOTH
> phases, because a raw-literal block decodes as a memcpy and a Huffman one does not.

> **THE CORRECTION THAT MATTERS.** Several cells the old board listed as "we beat C"
> were flattering us for emitting a WORSE FILE. `x-ray` L1 decompress at **0.23** was
> not a decode win -- it was a 21% larger output that was trivial to decode. At a
> matched 1.000 ratio the same cell reads **1.49**. Mission passes fall from 6/7 to 4/4
> at L1 for this reason and this reason only: `sao` and `x-ray` stopped being scored
> against our own inflated output. **A speed number is only meaningful beside its
> `us/c size`.** Read the two columns together, always.

**Instrument:** repaired (see [`m7-benchmark-repair.md`](m7-benchmark-repair.md)) --
best-of-N both arms, phases timed separately as C does, N>=25/phase, discarded warmup,
per-row same-arm spread, cycles/byte. Session null-arm **0.9799**. Dual gate 18/18 at
both levels. Pinned affinity=4, High priority, C via `-b -T1`. Decompress is timed into
a REUSED buffer via `decompress_into`, as C's `-b` does. Checksum parity with the
oracle (`ZSTD_c_checksumFlag = 0`); the shipped default is still `checksum: true` --
that changed the MEASUREMENT, not the product. Stage shares (section 3) come from a
separate `--features profile` build and are comparable only as shares within a run.

`C/us` > 1 means **C is faster**. `us/c size` > 1 means **we emit more bytes**.
**Never average these files** -- the per-file spread is the whole story.

---

## 1. Level 1 - the full board, all 18 corpora

**Re-measured 2026-08-17**, after the literals entropy gate. Sorted by `us/c size`.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  1099.3 |  7852.7 |   **0.14** |   2853.0 |   14414.7 |   **0.20** | **0.075** |
| zeros-32m    | 11113.8 | 22473.0 |   **0.49** |  29988.9 |   40470.9 |   **0.74** | **0.901** |
| incomp-32m   |  5377.4 |  6598.1 |   **0.81** |  14066.8 |   14997.7 |   **0.94** |     1.000 |
| x-ray        |   983.7 |   389.4 |       2.53 |   1498.7 |     983.0 |       1.52 |     1.000 |
| smallmsg-8m  |   551.4 |   304.7 |       1.81 |   2414.2 |     922.3 |       2.62 |     1.031 |
| jsonlog-16m  |   681.9 |   309.0 |       2.21 |   1859.1 |     994.1 |       1.87 |     1.061 |
| sao          |   464.7 |   249.1 |       1.87 |   1244.8 |     572.8 |       2.17 |     1.067 |
| osdb         |   571.8 |   232.4 |       2.46 |   1881.8 |    1078.2 |       1.75 |     1.100 |
| text-32m     | 15439.6 | 26462.5 |   **0.58** |  10544.0 |   33719.7 |   **0.31** |     1.100 |
| mr           |   521.8 |   214.2 |       2.44 |   1838.8 |    1337.1 |       1.38 |     1.111 |
| ooffice      |   473.8 |   183.5 |       2.58 |   1268.8 |     855.8 |       1.48 |     1.113 |
| webster      |   428.3 |   215.3 |       1.99 |   1828.2 |     914.5 |       2.00 |     1.137 |
| samba        |   646.7 |   330.4 |       1.96 |   2332.2 |    1147.8 |       2.03 |     1.144 |
| dickens      |   367.3 |   191.2 |       1.92 |   1748.1 |     850.4 |       2.06 |     1.168 |
| mozilla      |   581.7 |   231.9 |       2.51 |   1527.4 |     891.4 |       1.71 |     1.186 |
| xml          |   865.9 |   441.9 |       1.96 |   2770.4 |    1340.4 |       2.07 |     1.217 |
| reymont      |   360.7 |   210.3 |       1.71 |   1816.8 |     852.0 |       2.13 |     1.240 |
| nci          |  1077.8 |   531.3 |       2.03 |   2919.1 |    1339.4 |       2.18 |     1.301 |

### Where we win or tie

- **Compress at or better than C:** `versions-16m` **0.14**, `zeros-32m` **0.49**,
  `text-32m` **0.58**, `incomp-32m` **0.81**.
- **Decompress at or better than C:** `versions-16m` **0.20**, `text-32m` **0.31**,
  `zeros-32m` **0.74**, `incomp-32m` **0.94**.
- **Ratio at or better than C:** `versions-16m` **0.075**, `zeros-32m` **0.901**,
  `incomp-32m` 1.000, `x-ray` 1.000.

Against mission section 7 (decompress <= 1.11, compress <= 1.25):
**4 corpora pass compress, 4 pass decompress.** Down from 6 and 7 on the 2026-08-16
board -- see the correction in the header. `sao` and `x-ray` left the winners' column
because they stopped being scored against our own inflated output, not because either
got worse at equal size.

### Where we lose

- **Ratio:** `nci` **1.301**, `reymont` 1.240, `xml` 1.217, `mozilla` 1.186. The old
  headline losers are gone -- `smallmsg-8m` 1.615 -> **1.031** and `jsonlog-16m`
  1.528 -> **1.061** are now among our BEST ratios.
- **Compress:** `ooffice` **2.58**, `x-ray` 2.53, `mozilla` 2.51, `osdb` 2.46. These
  are the honest, matched-output positions; none is an open regression.

**`versions-16m` remains the headline** -- 0.075 size, 13x smaller than C, from the
repcode bricks (67/70/71/73/75). It was also the corpus that exposed the missing
function: `find_fast` had a repcode search and no other finder did.

---

## 2. Level 3 (dfast) - the shipping default

**Re-measured 2026-08-17.** Sorted by `us/c size`.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m | 5094.9 | 10710.0 |   **0.48** |  24626.6 |   19911.2 |       1.24 | **0.648** |
| zeros-32m    | 7951.6 | 22002.9 |   **0.36** |  28701.2 |   39364.7 |   **0.73** | **0.972** |
| x-ray        |  214.3 |    98.1 |       2.18 |   1111.6 |     423.8 |       2.62 | **0.982** |
| incomp-32m   | 4662.1 |  3971.7 |       1.17 |  13961.1 |   15769.5 |   **0.89** |     1.000 |
| mr           |  302.1 |   151.9 |       1.99 |   1637.7 |     479.8 |       3.41 |     1.020 |
| ooffice      |  271.6 |   109.9 |       2.47 |   1167.7 |     514.9 |       2.27 |     1.029 |
| osdb         |  387.3 |   175.9 |       2.20 |   1949.5 |     816.0 |       2.39 |     1.032 |
| jsonlog-16m  |  417.7 |   202.9 |       2.06 |   2007.2 |     926.1 |       2.17 |     1.036 |
| sao          |  227.6 |   120.6 |       1.89 |   1048.5 |     550.9 |       1.90 |     1.039 |
| smallmsg-8m  |  351.3 |   162.6 |       2.16 |   2410.9 |     838.4 |       2.88 |     1.049 |
| reymont      |  307.3 |   178.4 |       1.72 |   1693.3 |     594.1 |       2.85 |     1.054 |
| webster      |  281.0 |   164.1 |       1.71 |   1651.9 |     599.0 |       2.76 |     1.057 |
| dickens      |  241.8 |   136.6 |       1.77 |   1544.2 |     515.6 |       2.99 |     1.066 |
| mozilla      |  367.4 |   157.6 |       2.33 |   1449.8 |     629.8 |       2.30 |     1.071 |
| samba        |  468.0 |   254.5 |       1.84 |   2290.9 |     894.1 |       2.56 |     1.073 |
| text-32m     | 9358.9 | 25645.4 |   **0.36** |  10591.4 |   34674.4 |   **0.31** |     1.082 |
| nci          |  945.3 |   489.6 |       1.93 |   2858.6 |    1321.7 |       2.16 |     1.100 |
| xml          |  709.5 |   393.0 |       1.81 |   2751.7 |    1244.1 |       2.21 |     1.104 |

**L3 is the shipping default, and on RATIO it is now excellent: 16 of 18 corpora are
within 11% of C, and three BEAT it** -- `versions-16m` 0.648, `zeros-32m` 0.972 and
**`x-ray` 0.982**, which is new. The worst cell on the whole board is `xml` at 1.104.

`versions-16m` compress moved **4126.5 -> 10710.0 MB/s** (C/us 1.20 -> **0.48**) from
brick 88 alone, at unchanged 0.648 ratio -- the same tree-amortization defect that
halved it at L1 was costing more than half of L3.

Compress is 1.71-2.47 across Silesia; decompress 1.90-3.41. **Decompress is now the
weaker half at L3**, the reverse of the pre-gate picture, and it follows directly from
emitting Huffman literals where we used to emit raw ones.

Against mission section 7: **4 pass compress** (zeros 0.36, text 0.36, versions 0.48,
incomp 1.17), **3 pass decompress** (text 0.31, zeros 0.73, incomp 0.89).

---

> **THE OPTIMIZATION CAMPAIGN HAS ITS OWN ANATOMY.** What shipped, what was refused
> with numbers beside it, the Prometheus adjudication of the fitted constants, and the
> measured cost of every instrument used to decide any of it, are in
> [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md). Read its instrument rule before
> quoting a speed number from this document: the per-frame clock has a **±24.15%**
> null-arm floor on encode and **±25%** on decode, which is wider than most of the
> effects the campaign ships.

## 3. Stage anatomy — where OUR time goes

**Re-measured 2026-08-16 (third pass), at CHECKSUM PARITY.** Share of encode
(`stage / EncodeTotal`) and of decode (`stage / DecodeTotal`); bold marks the LEADING
stage on each half. Leaf stages only -- `EncodeEntropy`, `EncodeBlocks` and
`DecodeBlocks` are PARENT scopes and ranking them against their own children is
meaningless (a mistake made twice while deriving this board).

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |
| sao          |  **85.8** |      2.7 |    2.4 |     2.4 |     17.8 | **58.2** |     23.5 |
| webster      |  **66.7** |      7.8 |   12.4 |     8.9 |     10.9 | **82.1** |      6.9 |
| smallmsg-8m  |  **66.5** |      0.9 |   16.7 |    10.0 |      2.6 | **89.5** |      7.8 |
| ooffice      |  **65.1** |     18.6 |    7.2 |     4.6 |     27.3 | **63.7** |      8.8 |
| jsonlog-16m  |  **62.9** |      2.7 |   16.4 |    12.9 |      3.8 | **89.3** |      6.8 |
| reymont      |  **60.5** |     15.4 |   12.5 |     8.0 |     16.5 | **77.9** |      5.5 |
| dickens      |  **60.2** |     22.5 |    9.2 |     5.0 |     31.9 | **62.2** |      5.8 |
| samba        |  **60.1** |     11.0 |   13.8 |     9.7 |     13.7 | **77.9** |      8.2 |
| xml          |  **54.5** |     10.3 |   16.9 |    11.9 |     10.4 | **80.4** |      9.2 |
| mozilla      |  **47.4** |     33.7 |    8.7 |     6.2 |     32.0 | **61.8** |      6.0 |
| nci          |  **47.3** |     11.8 |   20.4 |    13.5 |     11.8 | **79.6** |      8.6 |
| text-32m     |  **40.8** |      1.9 |    0.5 |     0.3 |      0.5 |     44.2 | **53.4** |
| mr           |      40.3 | **52.5** |    1.7 |     1.5 | **65.7** |     24.4 |      9.6 |
| versions-16m |  **39.2** |      0.9 |    9.9 |     8.8 |      0.3 | **55.7** |     43.6 |
| osdb         |      39.1 | **40.5** |   10.4 |     6.0 |     26.1 | **66.2** |      7.6 |
| incomp-32m   |  **30.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **43.7** |
| x-ray        |      22.2 | **56.8** |    0.4 |     0.0 | **47.5** |      3.6 |     24.4 |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **32.6** |

**`EncodeMatchFind` is #1 on 15 of 18**, Huffman on 3 (`mr`, `x-ray`, `osdb`) --
**unchanged** from the pre-parity board. The encoder ranking was never distorted.

**`DecodeSeq` is #1 on 13 of 18** -- DOWN from the 16 of 18 previously claimed, and
the correction matters. It is still the single biggest decode stage and still the right
standing target, but its dominance was overstated.

**The runner-up changed identity: it is now `DecodeChecksum`, leading on 3 of 18.**
Previously the #2 was `DecodeLiterals`; bricks 63/79/80/81/82 shrank that side (a
per-block Huffman table clone, the arm read, two copy tiers, the inlined literal copy),
so literals fell to 2 of 18 and the checksum surfaced behind it. Where it leads it
leads by a lot: **`text-32m` 53%, `incomp-32m` 44%, `versions-16m` 44%, `zeros-32m`
33% of decode.**

That is a real finding, not an artefact of the parity fix. On high-ratio and
incompressible content there is little to decode, so VERIFICATION IS THE DECODER.
It also means `--no-check` / `DecompressOptions::force_ignore_checksum` is not a
micro-option on those corpora -- it removes the largest single stage.

**The two halves remain near mirror images** on the compressible files: where the
encoder spends its time in Huffman (`mr`, `x-ray`, `osdb`) the decoder spends its time
in literals. One content axis -- alphabet flatness -- drives both.

---

### Dispatch inventory — how to read these

A **dispatch-gated function** is one whose behaviour is selected at RUN TIME rather
than fixed at the call site. Four kinds appear below, and they are not equally
trustworthy:

| kind | selected by | trustworthy? |
| --- | --- | --- |
| **MEASURED** | a statistic of the CONTENT, recomputed per block or per section | yes -- this is the real dispatch |
| **LEVEL** | `CompressionParameters` from the level table | yes, but static per level |
| **ARM** | an env var / atomic, for in-process A/B | ships pinned; a shipped arm is a decision, not a dispatch |
| **CPU** | `is_x86_feature_detected!` at first use | yes, but invisible to the corpus |

`ARM` entries are listed because they are live branches in the shipping binary --
several were left switchable after their brick landed, and each one is an atomic load
or a `OnceLock` read that some hot loop may still be paying for. Bricks 49, 64 and 77
were all exactly that bug.

---

### MatchFind Function Anatomy -- Great Gate

The largest stage (#1 on 15 of 18 corpora) and by far the most dispatched.

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `params.strategy` | LEVEL | `find_fast` / `find_dfast` / `find_greedy` / `find_lazy(1)` / `find_lazy(2)` / `find_bt_lazy(2)` / `find_opt` | `find_sequences_strategy` encode.rs:1280 |
| 2 | `tables.rep_yield >= REP_YIELD_MIN` (0.125) | **MEASURED** | repcode-1 probe on/off, **per block**, from the PREVIOUS block's hit rate | encode.rs:1350, 2299, 2410, 2585, 2812 |
| 3 | `tables.last_search_per_byte >= lazy_fill_threshold()` | **MEASURED** | lazy chain back-fill on/off | encode.rs:2591 |
| 4 | `(packed, rep_on, pipe_on, s0)` | ARM x MEASURED | one of 13 const-generic `find_fast_impl<PACKED,REP,HLOG,STEP,PIPE>` monomorphizations | encode.rs:1351 |
| 5 | `tables.hash_log` in {12..16} | LEVEL | folds the hash shift to an immediate | encode.rs:1354, 1366 |
| 6 | `pair = step0 > 2` | LEVEL | pair-search at `ip+1` AND gates the 2-way software pipeline | encode.rs:1478, 1518 |
| 7 | `tag_enabled()` `RZSTD_TAG` + strategy==Fast + window<2^24 | ARM | packed tag slots (`store_fast`/`load_fast`) | encode.rs:288, 1948 |
| 8 | `pipe_enabled()` `RZSTD_MF_PIPE` | ARM | 2-way software pipelined probe loop | encode.rs:1337, 2005 |
| 9 | `step0_default()` `RZSTD_STEP0` | ARM | probe density (2 = ship, 1 = C's density) | encode.rs:1919 |
| 10 | `rep1_enabled()` `RZSTD_REP1` | ARM | force repcode-1 regardless of yield | encode.rs:1350 |
| 11 | `lazy_fill_enabled()` `RZSTD_LAZY_FILL` | ARM | back-fill on/off (also btlazy2) | encode.rs:1810, 2884 |
| 12 | `lazy_fill_stride()` `RZSTD_LAZY_FILL_S` | ARM | back-fill stride (1 = every position) | encode.rs:1839 |
| 13 | `lit_push_enabled()` / `litpush_hoist_enabled()` | ARM | 16-byte unsafe literal copy in `push_literals` | encode.rs:2112, 2098 |
| 14 | `1 << params.search_log.min(12)` | LEVEL | chain walk depth | `chain_find_best` encode.rs:2525 (also 2398 greedy, 2746 bt) |
| 15 | `has_avx2()` | CPU | `count_eq_len` AVX2 / NEON / scalar -- the match-length compare | simd.rs:84, encode.rs:3088 |
| 16 | `early_raw_skip` / `incomp_skip_on` `RZSTD_INCOMP_SKIP` | **MEASURED** x ARM | abandon the block to RAW when match bytes fall under a threshold | encode.rs:670, 817, 844 |
| 17 | `params.strategy` -> `extra` (BtUltra2=2, BtUltra=1, else 0) | LEVEL | the DP's repcode price `12 - extra + 2` | `find_opt` encode.rs:2938 |
| 18 | `params.strategy == BtUltra2` | LEVEL | DP length step: every length vs `(bml-len).clamp(1,4)` | `find_opt` encode.rs:3012 |
| 19 | `price[j] < inf` / `np < price[j]` | **MEASURED** | the optimal-parse DP itself -- per position, per candidate length | `find_opt` encode.rs:2949-3002 |
| 20 | `try_rep1` candidate admitted to the DP | **MEASURED** | brick 75 repcode candidate at `ip+1` | `find_opt` encode.rs:2964 |

**Only #2, #3, #16, #19 and #20 are true content dispatches**, and #19/#20 exist only
at L16+ (`find_opt`). At the shipping levels the biggest stage has exactly THREE live
content-adaptive decisions, and two of them (`rep_yield`, `last_search_per_byte`) are
single scalars carried from the previous block.

`find_opt` (rows 17-20) is the codec's only true cost model on the encode side -- a DP
over per-position prices. Its price terms are the known-crude part (literals flat 6,
offsets `12 + log2(offset)` from brick 72); bricks 76 and 83 both tried to make the
LITERAL term accurate while leaving the sequence term invented, and both reverted.

---

### Huff Function Anatomy -- Great Gate

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `n < 8` | size | straight to raw/RLE | huffman.rs:1548 |
| 2 | all-bytes-equal | **MEASURED** | RLE literals section | huffman.rs:1542 |
| 3 | `distinct * 2 >= lits.len()` | **MEASURED** | **BRICK 88** tree amortization -- reject when the weight table cannot be paid back | `literals_worth_huffman` huffman.rs:1492 |
| 4 | `sum_sq * 128 >= n^2` (collision entropy H2 <= 7 bits) | **MEASURED** | brick 86 entropy gate | `literals_worth_huffman` huffman.rs:1496 |
| 5 | `lits.len() < 64` | size | accept unconditionally (too small to sample) | huffman.rs:1422 |
| 6 | `n >= 256` | size | 4-stream vs 1-stream Huffman | huffman.rs:1563 |
| 7 | `prev_ct.covers(lits)` | **MEASURED** | treeless reuse of the previous block's table is even legal | huffman.rs:1595 |
| 8 | `futile` via `body_bytes_exact` | **MEASURED** | skip the previous-table encode when it provably loses to the new table by more than the tree | huffman.rs:1604 |
| 9 | `sec.len() < best_len` | **MEASURED** | final raw vs treeless vs new-table selection | huffman.rs:1622, 1645 |
| 10 | 4-stream encode returned `None` | fallback | retry at 1 stream | huffman.rs:1629 |
| 11 | `huff_fast_enabled()` `RZSTD_HUFF_FAST` | ARM | fast ctable construction | encode.rs:2032, huffman.rs:687 |
| 12 | `seqs.is_empty() && !literals_worth_huffman(block)` | **MEASURED** | whole-block raw, before any sequence work | encode.rs:651 |

**This is the most content-dispatched stage in the codec** -- 8 of 12 gates are
measured, and the pipeline is a genuine cost model: predict (3,4), prove (8), then
verify against the real number (9). It is also where the campaign's two worst
regressions came from, because gates 3 and 4 sit UPSTREAM of an O(n) histogram, a
ctable build and a `write_tree`. **A false accept there is not a wasted branch, it is
a wasted table build** -- that is what halved `versions-16m`.

---

### FseSeq Function Anatomy -- Great Gate

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | header-cost comparison | **MEASURED** | Predefined(0) / RLE(1) / FSE-compressed(2) / Repeat(3), **independently per table** | encode.rs:1140-1196 |
| 2 | single distinct symbol | **MEASURED** | RLE mode (`best_mode = 3` path) | encode.rs:1170 |
| 3 | built table beats predefined | **MEASURED** | FSE-compressed mode | encode.rs:1181 |
| 4 | `force_compressed && best_mode == 0` | caller | override away from Predefined | encode.rs:1189 |

Three tables (`ll`, `of`, `ml`) each run this dispatch independently, so a block picks
one of 4^3 = 64 mode combinations. All four gates are content-measured and all compare
REAL header bytes -- there is no estimator here to be wrong, which is why this stage
has produced no regressions. `note_seq_mode` records the outcome (predef/rle/comp/
REPEAT) and it is already in the profiler output.

---

### SeqCode Function Anatomy -- Great Gate

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `offset_value_for(offset, litlen, reps)` | **MEASURED** | repcode slot 1/2/3 vs a literal offset -- per sequence | encode.rs:982 |
| 2 | -- | -- | -- | -- |

**SeqCode has essentially NO dispatch, and that is the point.** It is a straight-line
transcode of `Seq -> CodedSeq` plus the ll/of/ml histogram walk, scoped at
encode.rs:979. The only decision is the repcode-vs-explicit-offset choice, which is
forced by the RFC, not chosen.

That matters because SeqCode was measured at **20-26% of encode on five corpora**
(nci 26.3, xml 25.1, samba 21.8, reymont 21.4, webster 20.7) -- larger than FseSeq on
every file. **A stage with a fifth of encode time and one forced decision is a pure
throughput problem**, not a dispatch problem: the levers are the table lookup
(`code_from_base`, already de-linearized) and the second histogram pass, not a smarter
gate.

---

### DecLits Function Anatomy -- Great Gate

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | literals block type (2 bits) | stream | Raw / RLE / Compressed / Treeless | `decode_literals` compressed.rs:87 |
| 2 | Size_Format | stream | 1/4 streams and 3-5 byte header | compressed.rs:118 |
| 3 | `n_streams == 1` | stream | single vs 4-way interleaved bit readers | compressed.rs:194 |
| 4 | `litcopy_on()` `LITCOPY_ARM` | ARM | 16-byte unsafe `copy_literals` vs checked | compressed.rs:370, 601, 693 |

**Every gate here is dictated by the bitstream except #4.** The decoder cannot
content-dispatch -- it must do what the encoder said. That is why this stage fell from
#2 to 2-of-18 leadership after bricks 63/79/80/81/82: the only levers are
*execution* levers, and they have largely been taken.

---

### DecSeq Function Anatomy -- Great Gate

The #1 decode stage on 13 of 18 corpora.

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `seqcheck_hoisted()` `SEQCHECK_ARM` / `RZSTD_SEQCHECK_HOIST` | ARM | hoist the per-sequence code-range check out of the loop | compressed.rs:371, 385, 559 |
| 2 | `lut_on()` `LUT_ARM` | ARM | LL/ML baseline+nbits from a LUT vs computed | compressed.rs:689, 753, 768 |
| 3 | `matchcopy_on() && len <= 32 && offset >= 32` | ARM x stream | 32-byte unsafe non-overlapping match copy | compressed.rs:966 |
| 4 | `matchcopy_on() && len <= 16 && offset >= 16` | ARM x stream | 16-byte tier | compressed.rs:990 |
| 5 | `offset == 1` | stream | byte-splat (C `ZSTD_overlapCopy8`) | compressed.rs:951 |
| 6 | `offset < len` | stream | overlapping copy, byte-at-a-time | `copy_match` compressed.rs:878 |
| 7 | FSE table mode per table | stream | predefined / RLE / built / repeat | decode side |

**Four of seven gates are ARMs**, i.e. decisions already made and left switchable. The
genuine per-sequence dispatches (#3-#6) are all on `(len, offset)` -- a shape the
ENCODER chooses. The decoder's speed is therefore partly an encoder-side question: the
distribution of `(litlen, matchlen, offset)` we emit determines which copy tier fires.
That is the one unexplored lever here, and it does not live in this stage.

---

### DecCk Function Anatomy -- Great Gate

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `header.checksum` | stream | whether a 4-byte trailer exists at all | decode.rs:368 |
| 2 | `opts.force_ignore_checksum` | caller | skip VERIFICATION but still CONSUME the 4 bytes | decode.rs:373 |
| 3 | `data.len() >= 32` | size | 4-lane stripe loop vs the tail path | xxh64.rs:42, 50 |

**xxh64 has NO CPU dispatch and no arms -- it is the only stage in the codec with
none.** That is not an oversight: XXH64's round needs a full 64x64 multiply, which
SSE2 and AVX2 do not have (`_mm_mul_epu32` is 32x32->64); only AVX-512DQ adds
`_mm_mullo_epi64`. This is precisely why XXH3 exists, and RFC 8878 fixes us to XXH64.

So the stage that is **#2 on decode overall and leads outright on 3 of 18** (text-32m
53%, incomp-32m 44%, versions-16m 44%, zeros-32m 33%) has exactly one tunable bit,
gate #2, and it is a correctness trade. **Everything else must come from moving the
work, not changing it** -- overlapping the hash with decode on a second thread.
Fusing it into the block loop was tried and measured ~12% WORSE (brick 85, reverted).

---

### Cross-cutting: dispatches that are DEAD in the shipping build

Found while auditing, per the `rusty_curiosity` law that an unused thing is invisible
to every profiler:

- **The entire BMI2 bit-extract dispatch is dead.** `simd::look_n_bits` (which calls
  `has_bmi2()` and picks `look_n_bits_bmi2` vs `look_n_bits_shift`) has exactly two
  call sites: its own definition and `simd.rs:401`, which is inside `#[cfg(test)]`.
  The shipping bit reader is `BitRev::look_bits_fast` (bit.rs:63) -- a single shift,
  no dispatch, no feature detection. `has_bmi2()`, `look_n_bits`, `look_n_bits_bmi2`
  and `look_n_bits_shift` are all test-only in practice.
- Consequence: the codec's only live CPU dispatch is `has_avx2()` in `count_eq_len`,
  which serves MatchFind alone. **The decoder has no SIMD dispatch at all.**

---

### What the inventory says

1. **Content dispatch is concentrated in Huff, not MatchFind.** 8 of 12 Huff gates are
   measured against 3 of 16 in MatchFind, even though MatchFind is the bigger stage.
2. **The decoder barely dispatches on content** -- it executes what the encoder chose.
   Six of its eleven gates are ARMs. Decoder speed is largely an ENCODER-side
   distribution question.
3. **Every regression this campaign produced came from a gate that sits upstream of
   expensive work** (Huff #3/#4 above an O(n) histogram + ctable build + `write_tree`).
   The FseSeq gates, which compare REAL header bytes with nothing expensive behind
   them, have never regressed. **Cheap-to-evaluate is not the property that matters;
   cheap-to-be-WRONG is.**
4. **The knob count is a standing liability: 17 of them.** 12 atomic arms, 3
   `OnceLock`s, and 2 UNCACHED `std::env::var` reads. Bricks 49, 64 and 77 were each a
   case of one sitting inside a per-symbol loop. They should be audited for hoisting,
   or retired to `cfg`. Full list in the addendum below.

---

### Addendum

The seven inventories above cover only the stages in the section 3 table. These gates
are real, live, and belong to no stage in it.

**Frame and block level (encode):**

| gate | kind | selects | where |
| --- | --- | --- | --- |
| `adv.nb_workers > 0` | caller | whole-frame multithreaded path `mt::compress_mt` | encode.rs:193 |
| `dict` / `prefix` present | caller | dictionary priming, `--patch-from`, `frame_start` offset | encode.rs:108-177 |
| `write_dict_id` | caller | Dictionary_ID field in the frame header | encode.rs:112 |
| `opts.checksum` | caller | 4-byte xxh64 trailer (default ON for us, OFF for `zstd -b`) | encode.rs:478 |
| `RZSTD_BLOCK_KB` | ARM (uncached env) | caps `block_max` below 128 KiB | encode.rs:488 |
| `payload_reserve_enabled()` `RZSTD_PAYLOAD_RES` | ARM | pre-reserve the block payload `Vec` | encode.rs:694, 2064 |
| `raw_limit` vs payload length | **MEASURED** | final Raw / RLE / Compressed block type | encode.rs:713-735 |
| single-byte block | **MEASURED** | RLE block | encode.rs:631 |

**Frame level (decode):**

| gate | kind | selects | where |
| --- | --- | --- | --- |
| `opts.window_max` (default 128 MiB) | caller | reject over-large-window frames (`-d --long`) | decode.rs:18, 36 |
| `dict` / `prefix` present | caller | history priming before block decode | decode.rs:48-77 |
| frame magic | stream | zstd frame vs SKIPPABLE frame vs end of input | decode.rs |

**The full knob list -- 17 runtime-selected switches in the shipping binary:**

12 atomic arms:
`SEQCHECK_ARM`, `LUT_ARM`, `LITCOPY_ARM`, `MATCHCOPY_ARM` (compressed.rs:548, 662-664);
`LAZY_FILL_ENABLED_ARM`, `REP1_ENABLED_ARM`, `TAG_ARM`, `PIPE_ARM`,
`HUFF_FAST_ENABLED_ARM`, `PAYLOAD_ARM`, `LITPUSH_ARM`, `LITPUSH_HOIST_ARM`
(encode.rs:1800, 1869, 1937, 1995, 2022, 2048, 2078, 2088).

3 `OnceLock`s: `lazy_fill_threshold` (1827), `lazy_fill_stride` (1839),
`step0_default` (1917).

2 UNCACHED `std::env::var` reads -- these allocate and hit the process environment on
EVERY call, unlike the arms and `OnceLock`s:
- `RZSTD_BLOCK_KB` (encode.rs:488) -- once per frame, harmless.
- **`RZSTD_INCOMP_SKIP` (encode.rs:826) -- once per BLOCK**, via
  `incomp_skip_on` <- `early_raw_skip`. Same shape as bricks 49/64/77: a knob read
  living inside a loop. It is per-block rather than per-symbol so the cost is small,
  but it is the only uncached env read left on a hot path.

**A methodology note, recorded because it nearly shipped a wrong number.** The first
pass at this count used `grep "static [A-Z_]*ARM"`, which does not match digits, so
`REP1_ENABLED_ARM` was invisible and the count read 11. The doc briefly claimed "16
shipped arms" on that basis. Correct pattern: `[A-Z0-9_]*`. An audit that counts is
only as good as its pattern -- verify the pattern against a case you KNOW exists.

---

## 4. What this says to do next

Rewritten 2026-08-17, after the literals-gate and chain-fill work.

1. **The mid levels are our strongest ground and were the least measured.** L5-L9
   compress runs **C/us 1.09-1.93** against 1.71-2.58 at L1 (section 6). The chain
   self-loop defect lived there for the whole campaign precisely because no board
   covered those levels. **Measure a level before believing anything about it.**

2. **TOP TARGET -- our ratio gains LESS per level than C's.** On 6 of 10 mid-level
   corpora `us/c size` gets WORSE as the level rises (nci 1.111 -> 1.161, dickens
   1.079 -> 1.120, webster 1.060 -> 1.093). Our output does shrink -- the monotonicity
   gate proves it -- but C's shrinks faster. This is one well-localized question: the
   chain search's marginal return on `search_log`. Two ready candidates bear on it:
   the **pair search at `step0 == 2`** (proven ~10% nci / 3.6% osdb / 5% webster,
   recorded at `515764b`, still unbuilt) and the residual **L9-over-L8 osdb
   non-monotonicity** (+308 bytes, the last one the gate tolerates).

3. **Decompress is now the weaker half at every level** -- L3 1.90-3.41, L5-L9
   1.78-3.67, worst on `mr` and `dickens`. This is a DIRECT consequence of the literals
   gate: blocks that used to decode as a memcpy now Huffman-decode. It is the price of
   the ratio wins, it was paid knowingly, and it is where the decoder's remaining work
   is. `DecodeSeq` itself is near its structural limit (brick 42 rewrite ~2x worse,
   brick 78 reserve 4-11% worse, brick 84 already-memset).

4. **The literals gate needs a THIRD term, and the pattern is now established.**
   Brick 86 priced the body (entropy); brick 88 priced the tree (alphabet vs section).
   Both were found by a corpus behaving impossibly, not by review. The gate still has
   no term for DECODE cost, which is what sections 1-2 now show it silently spends --
   `x-ray` L1 decompress 6488 -> 1034 MB/s, `sao` 5681 -> 562. **A gate that trades
   decode speed for size should know it is doing that.**

5. **xxh64 remains the best-evidenced decode lever.** #2 overall, leads outright on
   3 of 18 at 33-53% of decode. **Memory-bound, not compute-bound** (17.9/18.0/18.0
   GB/s at 32 KiB/256 KiB/4 MiB vs **12.5 GB/s over 32 MiB** -- `xxh64::locality_probe`).
   Two dead ends paid for: the algorithm is fixed by RFC 8878 (XXH64's 64x64 multiply
   has no SSE2/AVX2 equivalent -- why XXH3 exists), and fusing the hash into the block
   loop measured **~12% WORSE** (brick 85, reverted). Per section 3 it is also the only
   stage with NO dispatch of any kind. **What is left is overlapping it with decode on
   a second thread.**

6. **`SeqCode` is 20-26% of encode on five corpora with exactly ONE decision** (section
   3). Larger than `FseSeq` on every file, and the decision is RFC-forced. That makes it
   a pure throughput target -- the transcode and the second histogram walk -- and the
   only large stage where no gate can help.

7. **`find_opt`'s price model is still crude, and BOTH repairs failed the same way.**
   Literals cost a flat 6; brick 72 made the offset term ~log2(offset), brick 75 added
   repcode candidates. Brick 76 (block byte histogram) and brick 83 (the previous
   block's ACTUAL Huffman code lengths) were both REVERTED. The unifying cause: each
   made the LITERAL term accurate while sequences kept brick 72's invented
   `12 + log2(offset)` scale, which SKEWS the comparison rather than improving it.
   **A correct attempt must price literals AND sequences in real bits as one unit.**

8. **Housekeeping with real cost:** 17 runtime knobs (section 3 addendum), one of them
   -- `RZSTD_INCOMP_SKIP` -- an uncached `std::env::var` read on every block. And the
   entire BMI2 bit-extract dispatch is dead code with no non-test call site.

9. **RESOLVED, no longer a target:** the product-corpus ratio gap. `smallmsg-8m` and
   `jsonlog-16m` were 1.615 / 1.528 at L1; they are now 1.031 / 1.061, and at L5-L9 we
   BEAT C on smallmsg and match jsonlog (section 5). Replace both with real captured
   traffic before making any product claim from them.

---

## 5. Front 3 RESOLVED -- the product-corpus ratio gap is GONE

**Superseded 2026-08-17.** This section previously argued that `smallmsg-8m` 1.615 and
`jsonlog-16m` 1.528 were "an L1 artefact", explained by C's `minMatch` 7->5 switch, and
that we were "really" 1.28-1.32 at L5. **That framing was wrong in its conclusion, and
the cause was ours, not C's.** The gap was `literals_worth_huffman` measuring peak
frequency instead of entropy (brick 86, `ee9a2eb`). Fixed, the gap does not shrink --
it INVERTS:

| corpus        |    L1 |    L3 |        L5 |        L7 |        L9 |
| ------------- | ----: | ----: | --------: | --------: | --------: |
| smallmsg us/c | 1.031 | 1.049 | **0.960** | **0.975** | **0.964** |
| jsonlog us/c  | 1.061 | 1.036 | **0.992** |     1.002 | **0.996** |

**We now BEAT C on `smallmsg-8m` at every mid level and match `jsonlog-16m` to within
0.4%.** Against the pre-fix numbers in git history (smallmsg 1.317 / 1.369 / 1.387,
jsonlog 1.281 / 1.326 / 1.349) that is a 27-35% swing.

**What survives from the old analysis:** C's own output really is non-monotonic on this
content (smallmsg 2,524,581 at L1 -> 2,853,176 at L5, recovering to 2,371,244 at L19),
and the `minMatch` 7->5 switch really is why. That observation was correct and it is
why we now beat C at L5-L9: C admits short noisy matches there, and we no longer pay a
literals penalty that was masking the advantage.

**What does NOT survive:** the claim that our gap was "roughly HALF what the L1 headline
says" and that the corpus was a poor target. The corpus was a fine target; we were
misreading our own encoder. **The lesson is the one this campaign keeps relearning: a
gap attributed to the REFERENCE's behaviour deserves one more look at your own.**

The remaining caveat stands unchanged: `smallmsg-8m` and `jsonlog-16m` are GENERATED.
Replace them with real captured SpaceDB/CRDT traffic before treating any of these
numbers -- win or loss -- as a product statement.

---

## 6. Mid levels (L5-L9) -- the chain finders, first board ever taken

**New 2026-08-17.** L5/L6 are `greedy`, L7-L12 `lazy`/`lazy2`, L13-L15 `btlazy2`. No
board had ever covered them, which is how the chain self-loop defect (`9a7250e`) went
unseen: it only touched levels nobody measured. Sorted by L9 `us/c size`.

| corpus       | L5 C/us c | L5 size | L7 C/us c | L7 size | L9 C/us c | L9 size | L9 C/us d |
| ------------ | --------: | ------: | --------: | ------: | --------: | ------: | --------: |
| smallmsg-8m  |      1.46 | **0.960** |    1.85 | **0.975** |      1.82 | **0.964** |      1.93 |
| jsonlog-16m  |      1.40 | **0.992** |    1.61 |   1.002 |      1.60 | **0.996** |      1.78 |
| mr           |      1.67 |   1.032 |      1.52 |   1.020 |      1.81 |   1.033 |      3.08 |
| osdb         |      1.65 |   1.026 |      1.86 |   1.030 |      1.93 |   1.028 |      1.89 |
| samba        |      1.58 |   1.046 |      1.60 |   1.059 |      1.67 |   1.070 |      2.39 |
| sao          |      1.44 |   1.048 |      1.70 |   1.062 |      1.80 |   1.077 |      2.31 |
| webster      |      1.58 |   1.060 |      1.52 |   1.089 |      1.51 |   1.093 |      2.56 |
| dickens      |      1.80 |   1.079 |      1.64 |   1.114 |      1.56 |   1.120 |      3.11 |
| xml          |      1.50 |   1.106 |      1.42 |   1.116 |      1.37 |   1.111 |      2.32 |
| nci          |      1.21 |   1.111 |      1.09 |   1.144 |      1.12 |   1.161 |      2.32 |

**Compress is our BEST relative position anywhere: C/us 1.09-1.93**, against 1.71-2.58
at L1 and 1.71-2.47 at L3. `nci` at L7 is **1.09** -- within 9% of C. The mid levels are
where the encoder is most competitive, and they were the least measured.

**The ratio trend is the finding, and it is the wrong way round.** On 6 of 10 corpora
`us/c size` gets WORSE as the level rises: nci 1.111 -> 1.161, dickens 1.079 -> 1.120,
webster 1.060 -> 1.093, sao 1.048 -> 1.077, samba 1.046 -> 1.070. Our output does shrink
with level -- the monotonicity gate proves that -- but **C's shrinks FASTER**. We gain
less per level than C does across the whole chain-finder family.

That is a single, well-localized target: the chain search's marginal return on depth.
`search_log` rises with level and buys us less than it buys C. Two candidates already
on the docket bear directly on it -- the pair search at `step0 == 2` (proven ~10% on
nci, section 4) and the remaining L9-over-L8 non-monotonicity on osdb (+308 bytes).

**Decompress is the weak half here too: C/us 1.78-3.67**, worst on `mr` (3.08-3.67) and
`dickens` (3.11-3.31). Same cause as L3 -- more sequences per byte means more DecodeSeq,
and per section 3 that stage is near its structural limit.

---

