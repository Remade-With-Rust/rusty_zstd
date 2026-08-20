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

**Re-measured 2026-08-20.** Sorted by `us/c size`. C is the pinned v1.5.7 via
`-b1 -i1 -T1`; ours at CHECKSUM PARITY (`checksum: false`, matching
`ZSTD_c_checksumFlag = 0`), best-of-25 per phase with warmup discarded, decode into a
REUSED buffer via `decompress_into`.

> **READ THE TWO HALVES DIFFERENTLY ON THIS BOARD.** `us/c size` is a byte count --
> deterministic, no clock, fully trustworthy. The four SPEED columns are weaker than the
> 2026-08-17 board: that run pinned affinity=4 at High priority, this one could not, and
> a second session was active on the box throughout. The **same-arm null spread was
> 14.09%**. Read speed cells as a band, not a value, and do not difference individual
> speed cells against the older board.

> **THE RATIO GAINS BELOW ARE NOT FROM THE OPTIMIZATION CAMPAIGN.** Every change in
> [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md) is byte-identical by construction
> and CANNOT move a ratio. These come from the concurrent gate work.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  1814.0 | 10875.5 |   **0.17** |   4364.9 |   22271.7 |   **0.20** | **0.082** |
| zeros-32m    | 13059.0 | 26845.6 |   **0.49** |  41872.7 |   44642.9 |   **0.94** | **0.967** |
| incomp-32m   |  6178.8 |  3333.6 |       1.85 |  25151.2 |   26711.2 |   **0.94** |     1.000 |
| x-ray        |   965.9 |   341.5 |       2.83 |   1468.9 |    1024.1 |       1.43 |     1.000 |
| smallmsg-8m  |   578.7 |   211.6 |       2.73 |   2511.5 |     940.5 |       2.67 |     1.017 |
| mozilla      |   794.3 |   321.1 |       2.47 |   2344.6 |    1152.6 |       2.03 |     1.021 |
| ooffice      |   480.6 |   130.8 |       3.68 |   1271.9 |     795.1 |       1.60 |     1.027 |
| samba        |   596.0 |   223.3 |       2.67 |   2462.2 |    1061.7 |       2.32 |     1.028 |
| osdb         |   584.9 |   185.8 |       3.15 |   1935.5 |    1001.0 |       1.93 |     1.045 |
| webster      |   431.9 |   159.0 |       2.72 |   1845.8 |     817.8 |       2.26 |     1.053 |
| jsonlog-16m  |   715.7 |   228.6 |       3.13 |   1935.3 |     960.2 |       2.02 |     1.063 |
| sao          |   469.3 |   237.7 |       1.97 |   1197.1 |     586.2 |       2.04 |     1.065 |
| dickens      |   380.4 |   141.0 |       2.70 |   1808.3 |     798.4 |       2.26 |     1.082 |
| xml          |   755.4 |   342.1 |       2.21 |   2731.4 |    1160.3 |       2.35 |     1.086 |
| mr           |   539.2 |   156.2 |       3.45 |   1883.5 |    1098.2 |       1.72 |     1.089 |
| text-32m     | 19490.9 | 24844.7 |   **0.78** |  10837.2 |   40383.6 |   **0.27** |     1.126 |
| nci          |  1035.2 |   430.6 |       2.40 |   2836.3 |    1242.0 |       2.28 |     1.129 |
| reymont      |   372.3 |   160.6 |       2.32 |   1900.8 |     731.7 |       2.60 |     1.131 |

**mean C/us comp 2.32, decomp 1.77 | mean ratio 1.001 | worst ratio 1.131 (reymont) |
we beat C: 3 comp, 4 decomp, 2 ratio**

**THE RATIO STORY IS THE STORY.** The worst cell on the board moved from **1.301
(`nci`) to 1.131 (`reymont`)**, and the mean landed at **1.001**. Every row that used to
sit above 1.13 came down:

| corpus | `us/c size` 08-17 | 08-20 |
| --- | ---: | ---: |
| `nci` | 1.301 | **1.129** |
| `reymont` | 1.240 | **1.131** |
| `xml` | 1.217 | **1.086** |
| `mozilla` | 1.186 | **1.021** |
| `dickens` | 1.168 | **1.082** |
| `samba` | 1.144 | **1.028** |
| `webster` | 1.137 | **1.053** |

L1 has moved from a board that was smaller than C on 3 corpora and badly larger on 5,
to one whose mean is parity and whose worst cell is +13.1%.

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

**Re-measured 2026-08-20.** Sorted by `us/c size`. Same protocol and the same two
caveats as section 1: the ratio column is a byte count and is trustworthy; the speed
columns were taken without pinned affinity or raised priority, with a second session
active, at a **13.45% same-arm null spread**.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m | 5169.8 |  8048.3 |   **0.64** |  23579.5 |   19097.6 |       1.23 | **0.659** |
| zeros-32m    | 8543.1 | 33543.0 |   **0.25** |  42477.2 |   45532.2 |   **0.93** | **0.967** |
| x-ray        |  226.3 |    70.1 |       3.23 |   1140.7 |     410.0 |       2.78 | **0.983** |
| incomp-32m   | 5854.8 |  3408.3 |       1.72 |  24674.9 |   24676.1 |   **1.00** |     1.000 |
| jsonlog-16m  |  418.1 |   182.3 |       2.29 |   2018.1 |     907.7 |       2.22 |     1.010 |
| osdb         |  385.1 |   153.1 |       2.51 |   1952.9 |     829.5 |       2.35 |     1.016 |
| mr           |  292.1 |   126.1 |       2.32 |   1677.9 |     474.7 |       3.53 |     1.022 |
| smallmsg-8m  |  357.6 |   126.1 |       2.84 |   2455.7 |     836.1 |       2.94 |     1.029 |
| mozilla      |  460.3 |   187.9 |       2.45 |   2216.5 |    1074.3 |       2.06 |     1.029 |
| ooffice      |  275.0 |    87.2 |       3.15 |   1177.5 |     512.4 |       2.30 |     1.032 |
| sao          |  224.5 |    78.2 |       2.87 |   1039.2 |     496.6 |       2.09 |     1.034 |
| samba        |  466.3 |   229.8 |       2.03 |   2696.0 |    1009.5 |       2.67 |     1.036 |
| reymont      |  316.6 |   158.4 |       2.00 |   1762.5 |     529.7 |       3.33 |     1.041 |
| webster      |  299.4 |   141.2 |       2.12 |   1754.5 |     617.9 |       2.84 |     1.050 |
| dickens      |  250.0 |   113.8 |       2.20 |   1617.0 |     503.1 |       3.21 |     1.052 |
| text-32m     | 11228.2 | 24585.1 |   **0.46** |  10774.2 |   37986.7 |   **0.28** |     1.071 |
| xml          |  745.9 |   343.1 |       2.17 |   2876.0 |    1213.8 |       2.37 |     1.079 |
| nci          |  941.1 |   435.8 |       2.16 |   2926.0 |    1257.3 |       2.33 |     1.100 |

**mean C/us comp 2.08, decomp 2.25 | mean ratio 1.012 | worst ratio 1.100 (nci) |
we beat C: 3 comp, 3 decomp, 3 ratio**

**L3 ratio was already excellent and it tightened.** The mean is unchanged at **1.012**,
but the worst cell moved from `xml` **1.104** to `nci` **1.100**, and the middle of the
board compressed toward parity: `mozilla` 1.071 -> **1.029**, `samba` 1.073 ->
**1.036**, `xml` 1.104 -> **1.079**, `dickens` 1.066 -> **1.052**. Three corpora still
BEAT C -- `versions-16m` 0.659, `zeros-32m` 0.967 and `x-ray` 0.983.

**Decompress remains the weaker half at L3** (2.06-3.53 against compress 2.00-3.23),
unchanged in character from the 08-17 board and still following from emitting Huffman
literals where we used to emit raw ones.

Against mission section 7: **3 pass compress** (zeros 0.25, text 0.46, versions 0.64),
**3 pass decompress** (text 0.28, zeros 0.93, incomp 1.00).

> **WHAT THIS BOARD DOES NOT SAY.** It does not say the optimization campaign made us
> faster. Its speed columns cannot carry that claim at a 13.45% null spread, and the
> campaign never made it -- every item shipped on strictly-less-work plus byte-identity.
> See [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md) section 4.

---

## 3. Stage anatomy — where OUR time goes

**Re-measured 2026-08-20 (fourth pass), L3, 8 MiB board, after the optimization
campaign** ([`m7-optimize-anatomy.md`](m7-optimize-anatomy.md)). Share of encode
(`stage / EncodeTotal`) and of decode (`stage / DecodeTotal`); bold marks the LEADING
stage on each half. Leaf stages only -- `EncodeEntropy`, `EncodeBlocks` and
`DecodeBlocks` are PARENT scopes and ranking them against their own children is
meaningless (a mistake made twice while deriving this board).

> **THESE ARE SHARES, NOT TIMES.** They come from a `--features profile` build whose
> own instrumentation is part of what it measures, so a share may move because its
> stage shrank OR because another grew. Do not read an absolute speedup out of this
> table; read WHERE THE TIME IS.

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |
| smallmsg-8m  |  **78.7** |      5.9 |    4.8 |     6.2 |     10.2 | **86.0** |      2.2 |
| sao          |  **78.7** |     10.8 |    4.8 |     2.6 |     33.3 | **64.4** |      2.1 |
| x-ray        |  **77.3** |     10.9 |    5.0 |     3.4 |     17.5 | **78.8** |      3.5 |
| mozilla      |  **74.1** |     10.2 |    5.8 |     4.6 |     13.9 | **82.2** |      3.6 |
| jsonlog-16m  |  **74.1** |      6.6 |    7.0 |     7.0 |      9.9 | **85.8** |      2.7 |
| ooffice      |  **73.7** |     12.2 |    5.3 |     4.8 |     19.2 | **77.5** |      1.9 |
| samba        |  **73.7** |      7.3 |    7.7 |     6.2 |      7.5 | **88.7** |      3.3 |
| dickens      |  **72.7** |      4.1 |    9.4 |     9.2 |      3.2 | **94.5** |      2.3 |
| reymont      |  **72.0** |      3.8 |   10.3 |     9.1 |      2.8 | **95.3** |      1.5 |
| webster      |  **72.0** |      4.9 |    9.7 |     8.9 |      4.0 | **94.1** |      1.8 |
| mr           |  **68.6** |      8.7 |    9.5 |     8.3 |      5.9 | **92.1** |      1.8 |
| osdb         |  **68.2** |     16.4 |    5.8 |     5.8 |     10.5 | **86.0** |      3.1 |
| xml          |  **67.7** |      7.7 |   10.3 |     7.9 |      7.1 | **88.3** |      3.9 |
| nci          |  **64.8** |      6.7 |   12.6 |     9.2 |      6.3 | **88.8** |      4.6 |
| text-32m     |  **61.8** |      3.5 |    0.9 |     0.3 |      0.2 | **65.0** |     33.8 |
| versions-16m |  **54.6** |      0.8 |    3.7 |     4.8 |      0.3 | **67.4** |     31.6 |
| incomp-32m   |  **11.5** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **17.9** |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **28.3** |

**`EncodeMatchFind` is now #1 on 18 of 18** -- up from 15 of 18. **Huffman no longer
leads ANY corpus on encode**, where it previously led three:

| corpus | Huff, 08-16 | Huff, 08-20 |
| --- | ---: | ---: |
| `x-ray` | **56.8** | 10.9 |
| `mr` | **52.5** | 8.7 |
| `osdb` | **40.5** | 16.4 |
| `mozilla` | 33.7 | 10.2 |

**`DecodeSeq` is #1 on 16 of 18**, up from 13. **`DecodeLiterals` leads NOWHERE**, down
from 2 -- and it collapsed on exactly the corpora that used to be its own:

| corpus | DecLits, 08-16 | DecLits, 08-20 |
| --- | ---: | ---: |
| `mr` | **65.7** | 5.9 |
| `x-ray` | **47.5** | 17.5 |
| `dickens` | 31.9 | 3.2 |
| `mozilla` | 32.0 | 13.9 |

**Both halves moved together, and that is the point.** The literal path was the
campaign's densest target: 63 per-literal bounds checks removed from
`decode_4x`/`decode_into_x1`/`decode_into_x2`, 8 more from `encode_stream`, and
`huffman.rs` taken from 91 panic sites to **zero** with the file 7.5% smaller. The
alphabet-flatness axis that used to drive BOTH sides now drives neither: the encoder's
Huffman share and the decoder's literal share fell in step.

**`DecodeChecksum` still leads the degenerate corpora** (`text-32m` 33.8%,
`versions-16m` 31.6%, `zeros-32m` 28.3%, `incomp-32m` 17.9%) but by less than before
(53.4 / 43.6 / 32.6 / 43.7). The standing conclusion holds unchanged: on high-ratio and
incompressible content there is little to decode, so **verification IS the decoder**,
and `--no-check` / `DecompressOptions::force_ignore_checksum` removes the largest single
stage there rather than shaving a micro-option.

**WHAT THIS SAYS NOW.** The encoder has ONE target and no second: match-find leads every
corpus, from 54.6% (`versions`) to 78.7% (`smallmsg`, `sao`). The decoder has one too --
sequence execution, 64-95% on every compressible file. The entropy stages that used to
compete for the lead are no longer in the running on either half.

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

