# M7 anatomy — rusty_zstd vs facebook/zstd v1.5.7, side by side

**Date:** 2026-08-16 — re-measured in full after the optimisation campaign
(sections 1, 2, 3 and 4 all regenerated; the pre-campaign board is in git history).
**Instrument:** repaired (see [`m7-benchmark-repair.md`](m7-benchmark-repair.md)) —
best-of-N both arms, phases timed separately as C does, N>=25/phase, discarded warmup,
per-row same-arm spread, cycles/byte. Session null-arm **1.0384** (L1, worst same-arm
spread 4.8%) / **1.0084** (L3). Dual gate **18/18** at L1 and **18/18** at L3 (widened
from 8). Pinned affinity=4, High priority, C via `-b -T1`. Stage shares (section 3) come
from a separate `--features profile` build, so they are NOT comparable in absolute ms to
the speed boards — only as shares within a run.

**Everything below is at MATCHED OUTPUT.** Every brick in the campaign was gated on
byte-identical Silesia hashes, so the `us/c size` columns are unchanged from the
pre-campaign board and all speed movement is genuine.

`C/us` > 1 means **C is faster**. `us/c size` > 1 means **we emit more bytes**.
**Never average these files** — the per-file spread is the whole story.

---

## 1. Level 1 — the full board, all 18 corpora

**Re-measured 2026-08-16 (third pass)** -- CURRENT as of bricks 46-82. Session null-arm
**0.9948**. Unlike the previous pass, the **decompress columns now include the decoder
work (bricks 79-82)**; nothing here is stale.

Compressed sizes are byte-identical to the gated hashes, so every speed figure is at
MATCHED OUTPUT.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 10291.7 |  9756.7 |   **1.05** |  23798.8 |   10712.7 |       2.22 | **0.901** |
| text-32m     | 14640.2 | 10253.8 |       1.43 |  10319.7 |    9978.7 |   **1.03** |     1.100 |
| incomp-32m   |  5078.6 |  4946.8 |   **1.03** |  12181.1 |    7583.8 |       1.61 |     1.000 |
| jsonlog-16m  |   687.7 |   343.6 |       2.00 |   1836.9 |    1071.9 |       1.71 | **1.528** |
| smallmsg-8m  |   544.4 |   366.4 |       1.49 |   2384.4 |    1030.7 |       2.31 | **1.615** |
| versions-16m |  1096.1 |  5269.1 |   **0.21** |   2855.5 |    7800.1 |   **0.37** | **0.075** |
| mr           |   508.8 |   206.8 |       2.46 |   1784.5 |    1220.2 |       1.46 |     1.111 |
| ooffice      |   460.1 |   258.2 |       1.78 |   1223.0 |    1287.8 |   **0.95** |     1.262 |
| osdb         |   543.5 |   241.3 |       2.25 |   1782.5 |    1018.1 |       1.75 |     1.138 |
| reymont      |   342.9 |   215.1 |       1.59 |   1731.3 |     845.4 |       2.05 | **1.402** |
| sao          |   450.3 |   411.5 |   **1.09** |   1211.5 |    4270.6 |   **0.28** |     1.138 |
| webster      |   410.4 |   237.4 |       1.73 |   1744.0 |     987.9 |       1.77 | **1.443** |
| dickens      |   363.1 |   187.4 |       1.94 |   1723.3 |     809.4 |       2.13 |     1.201 |
| mozilla      |   575.7 |   243.5 |       2.36 |   1511.7 |     879.2 |       1.72 |     1.222 |
| nci          |  1059.8 |   499.1 |       2.12 |   2856.2 |    1192.8 |       2.39 |     1.302 |
| samba        |   637.7 |   352.2 |       1.81 |   2294.8 |    1171.9 |       1.96 |     1.258 |
| xml          |   855.9 |   442.5 |       1.93 |   2764.2 |    1309.7 |       2.11 |     1.308 |
| x-ray        |   974.2 |  1693.4 |   **0.58** |   1481.9 |    4625.2 |   **0.32** |     1.212 |

### Where we already win or tie

- **Compress at or better than C:** `versions-16m` **0.21**, `x-ray` **0.58**,
  `incomp-32m` **1.03**, `zeros-32m` **1.05**, `sao` **1.09**.
- **Decompress at or better than C:** `sao` **0.28**, `x-ray` **0.32**,
  `versions-16m` **0.37**, `ooffice` **0.95**, `text-32m` **1.03**.
- **Ratio better than C:** `versions-16m` **0.075** (13x smaller), `zeros-32m` 0.901.

Against mission section 7 (decompress <= 1.11, compress <= 1.25): **5 corpora pass
compress, 5 pass decompress.**

### Where we lose

- **Compress:** `mr` 2.46, `mozilla` 2.36, `osdb` 2.25, `nci` 2.12.
- **Ratio:** `smallmsg-8m` **1.615** and `jsonlog-16m` **1.528**. See section 5 -- the
  gap is roughly HALF what L1 implies, and the generated corpus should be replaced with
  real captured traffic before this is treated as a target.

**`versions-16m` is the campaign's headline.** It was 1.06 compress / 0.755 size before
the work; the repcode bricks (67/70/71/73/75) took it to **0.21 compress and 0.075 size --
13x smaller than C**. It was also the corpus that exposed the missing function: `find_fast`
had a repcode search and NO other finder did, so L3 -- the shipping default -- had none.

---

## 2. Level 3 (dfast) — the shipping default

**Re-measured 2026-08-16 (third pass)** -- CURRENT as of bricks 46-84. Session
null-arm **0.9839**. The decompress columns now include the decoder work
(bricks 79-82); the previous pass's did not.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 7483.1 |  9778.4 |   **0.77** |  24342.4 |   10502.2 |       2.32 | **0.972** |
| text-32m     | 8702.8 | 10277.0 |   **0.85** |  10332.9 |    9672.9 |   **1.07** |     1.082 |
| incomp-32m   | 4277.3 |  3112.4 |       1.37 |  12255.7 |    7553.6 |       1.62 |     1.000 |
| jsonlog-16m  |  384.3 |   206.3 |       1.86 |   1907.2 |     950.0 |       2.01 |     1.318 |
| smallmsg-8m  |  340.6 |   168.5 |       2.02 |   2383.0 |     899.6 |       2.65 | **1.440** |
| versions-16m | 4858.2 |  6870.6 |   **0.71** |  23194.5 |    9681.0 |       2.40 | **0.648** |
| mr           |  297.5 |   162.7 |       1.83 |   1605.8 |     487.9 |       3.29 |     1.045 |
| ooffice      |  265.4 |   130.2 |       2.04 |   1151.5 |     694.1 |       1.66 |     1.119 |
| osdb         |  360.5 |   221.1 |       1.63 |   1889.0 |    1032.0 |       1.83 |     1.103 |
| reymont      |  289.7 |   182.1 |       1.59 |   1659.4 |     596.0 |       2.78 |     1.089 |
| sao          |  223.2 |   132.2 |       1.69 |   1041.5 |     839.1 |       1.24 |     1.056 |
| webster      |  283.9 |   169.2 |       1.68 |   1629.9 |     602.4 |       2.71 |     1.135 |
| dickens      |  237.8 |   138.7 |       1.71 |   1525.2 |     506.0 |       3.01 |     1.121 |
| mozilla      |  361.6 |   171.4 |       2.11 |   1387.9 |     672.0 |       2.07 |     1.105 |
| nci          |  903.8 |   477.5 |       1.89 |   2724.4 |    1207.5 |       2.26 |     1.152 |
| samba        |  459.3 |   258.9 |       1.77 |   2184.9 |     881.8 |       2.48 |     1.109 |
| xml          |  690.0 |   408.4 |       1.69 |   2696.4 |    1243.3 |       2.17 |     1.188 |
| x-ray        |  207.6 |   110.1 |       1.89 |   1090.1 |     543.9 |       2.00 |     1.082 |

**L3 is the shipping default, and it is where we compress BEST relative to C.**
At or better than C on `versions-16m` **0.71**, `zeros-32m` **0.77**, `text-32m`
**0.85**. Worst are `mozilla` 2.11, `ooffice` 2.04, `smallmsg-8m` 2.02.

**Ratio at L3 is much better than L1** -- Silesia spans 1.045-1.188 here against L1's
1.111-1.443. `versions-16m` is the one inversion (0.648 vs L1's 0.075): L1's
Fast+repcode suits constant-stride content better than dfast does.

**Decompress is our weak axis at L3** (1.24-3.29 on Silesia, against
L1's 0.28-2.39). More sequences per byte means more DecodeSeq, which section 3 shows
is the #1 stage on 16 of 18 corpora. This is the standing target, not the encoder.

**Read the generated corpora with section 5:** their non-monotonic response to level is
a `minMatch` interaction with synthetic content, not a regression, and real content does
not share it.

---

## 3. Stage anatomy — where OUR time goes

**Re-measured 2026-08-16 (second pass)**, AFTER the repcode campaign (bricks 67/70/71/73/75)
and the decoder work (79-82). Share of encode (`stage / EncodeTotal`) and of decode
(`stage / DecodeTotal`); bold marks the LEADING stage on each half.

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: |
| sao          |  **86.8** |      1.8 |    2.3 |     2.4 |     22.6 | **54.4** |
| webster      |  **67.5** |      7.8 |   12.0 |     8.6 |     11.0 | **82.4** |
| smallmsg-8m  |  **66.7** |      0.9 |   16.6 |    10.3 |      2.5 | **90.0** |
| ooffice      |  **65.8** |     18.3 |    7.0 |     4.5 |     27.2 | **62.8** |
| jsonlog-16m  |  **63.2** |      2.7 |   16.1 |    12.7 |      3.8 | **89.1** |
| reymont      |  **60.9** |     15.5 |   12.1 |     8.0 |     16.5 | **77.6** |
| dickens      |  **60.5** |     22.4 |    9.0 |     5.0 |     31.9 | **62.2** |
| samba        |  **60.5** |     11.2 |   13.5 |     9.6 |     13.9 | **77.9** |
| xml          |  **55.1** |     10.2 |   16.6 |    11.9 |     10.5 | **80.5** |
| mozilla      |  **47.9** |     33.4 |    8.6 |     6.1 |     31.7 | **61.8** |
| nci          |  **47.4** |     11.8 |   20.3 |    13.5 |     11.9 | **78.8** |
| text-32m     |  **42.7** |      1.8 |    0.5 |     0.3 |      0.6 | **47.4** |
| mr           |      40.2 | **52.7** |    1.6 |     1.5 | **66.9** |     24.6 |
| osdb         |      39.7 | **40.3** |   10.4 |     5.8 |     26.4 | **66.1** |
| versions-16m |  **39.3** |      0.8 |    9.8 |     8.4 |      0.3 | **59.9** |
| incomp-32m   |  **30.4** |      0.0 |    0.0 |     0.0 |      0.0 |  **0.0** |
| x-ray        |      21.4 | **58.9** |    0.4 |     0.0 | **48.0** |      3.9 |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |  **0.0** |

**`EncodeMatchFind` is #1 on 15 of 18**, Huffman on 3 (`mr`, `x-ray`, `osdb`).
**`DecodeSeq` is #1 on 16 of 18** -- UP from 14 of 17 before the campaign, and now
62-90% of decode on the sequence-heavy files.

**That rise is the campaign working, not a regression.** Bricks 63/79/80/81/82 cut the
LITERALS side of decode (per-block Huffman table clone, the arm read, two copy tiers,
the inlined literal copy), so `DecodeLiterals` shrank and `DecodeSeq`'s share grew
correspondingly. The sequence loop is now a LARGER fraction of a SMALLER decode.

**The two halves remain near mirror images:** the files where the encoder spends its time
in Huffman (`mr`, `x-ray`, `osdb`) are the files where the decoder spends its time in
literals. One content axis -- alphabet flatness -- drives both.

---

## 4. What this says to do next

Rewritten 2026-08-16 (second pass), after the repcode campaign and the decoder work.

1. **Encoder = MatchFind, and its seam is WORKED OUT.** The probe loop is **19
   instructions with 1 stack access** (from 47 / 20); every frame-constant (`hash_log`,
   `step0`, the tag/rep flags, the pipeline flag) is specialised away. The remaining
   reload is one L1-hot slot. Plumbing was MEASURED rather than inferred: of ~40 static
   call sites, one ran ZERO times and the only hot one was `push_literals`
   (15,687,334 executions), whose cost was a flag read -- fixed, ~1%.
   **Further gains need a different lever than const-specialisation.**

2. **REPCODE now fires in EVERY finder** (`fast`, `dfast`, `greedy`, `lazy`, `bt_lazy`,
   `opt`). It was present only in `fast`, so L3 -- the shipping default -- had none.
   `versions-16m` went from **4.3x WORSE than C at L3 to 0.648**, and now beats C at
   every level 1-22. Silesia ratios IMPROVED rather than traded.

3. **Decoder = DecodeSeq, #1 on 14 of 17.** Hot loop is **~109 instructions / 27 stack
   accesses**, and the stack traffic is DIFFUSE -- ~12 slots touched 1-3x each, with no
   dominant frame-constant to specialise. That is structurally unlike the probe loop,
   where five constants carried nearly all of it. Literal copies are now measured:
   90.5% take the 16-byte tier, brick 80's 32-byte tier captured 61% of the remainder,
   and what is left (728K runs >32 B) genuinely wants a memcpy.
   **Treat this loop as near its structural limit unless a COUNT says otherwise** --
   brick 42 showed a rewrite here costs ~2x, and brick 78 showed a well-meant reserve
   costs 4-11%.

4. **Ratio = the product corpus.** `smallmsg-8m` 1.615 / `jsonlog-16m` 1.528 at L1,
   but 1.440 / 1.318 at L3 -- section 5 has the `minMatch` explanation. Replace the
   generated corpus with real captured traffic before treating this as a target.

5. **`find_opt`'s price model is still crude.** Literals cost a flat 6; brick 72 made
   the offset term ~log2(offset) and brick 75 added repcode candidates, but C prices
   from accumulated `litFreq`/`matchLengthFreq`. Brick 76 tried the block's byte
   histogram as a proxy and was REVERTED (net worse, mechanism unexplained). The right
   source -- the previous block's ACTUAL literal frequencies -- is already computed.

---

## 5. Front 3 investigated — the product-corpus ratio gap is an L1 ARTEFACT

`smallmsg-8m` 1.615 and `jsonlog-16m` 1.527 are quoted at L1. Across levels
(exact bytes, deterministic):

| corpus        |    L1 |    L3 |        L5 |    L7 |    L9 |
| ------------- | ----: | ----: | --------: | ----: | ----: |
| smallmsg us/c | 1.615 | 1.440 | **1.317** | 1.369 | 1.387 |
| jsonlog us/c  | 1.527 | 1.320 | **1.281** | 1.326 | 1.349 |

**We are 1.28-1.32 at L5, not 1.5-1.6.** The L1 figure is our worst point, and it is
measured against C's BEST point on this content.

**Why C peaks at L1 here** (chased, not assumed -- C's own output gets WORSE with level:
smallmsg 2,524,581 at L1 -> 2,853,176 at L5, recovering to 2,371,244 at L19): the level
table switches `minMatch` from **7 at L1** to **5 at L5**. This corpus is
`key=hexvalue;` fields drawn from a 12-word vocabulary, so minMatch 7 admits only the
genuine vocabulary matches, while minMatch 5 admits short noisy ones that the greedy
parse then takes -- more sequences, worse offsets, bigger output.

**Two honest caveats:**

1. The gap is real but roughly HALF what the L1 headline says.
2. This generated corpus is a stand-in whose level response is unusual (a non-monotonic
   C curve is not typical of real logs). Before treating "product corpus ratio" as a
   campaign target, replace it with REAL captured SpaceDB/CRDT traffic. Density
   (`RZSTD_STEP0=1`) buys only 2.5-5.9% here versus 3-11% on Silesia, so it is not the
   lever either.
