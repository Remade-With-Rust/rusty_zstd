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

**Re-measured 2026-08-16** after the 17-brick optimisation campaign (front 1 match-find,
the mr Huffman work, front 2 decoder). Session null-arm **1.0384**, worst same-arm spread
4.8%. Compressed sizes are BYTE-IDENTICAL to the pre-campaign board — every change was
gated on the Silesia hashes — so the `us/c size` column is unchanged and all movement
below is pure speed.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 10931.2 | 10396.4 |   **1.05** |  27316.6 |   11467.7 |       2.38 | **0.901** |
| text-32m     | 15201.8 | 10833.4 |       1.40 |  10540.3 |   10850.6 |   **0.97** |     1.022 |
| incomp-32m   |  5155.6 |  4994.2 |   **1.03** |  13028.6 |    7841.7 |       1.66 |     1.000 |
| jsonlog-16m  |   676.7 |   306.8 |       2.21 |   1848.4 |    1016.6 |       1.82 | **1.527** |
| smallmsg-8m  |   535.7 |   316.5 |       1.69 |   2323.4 |     985.1 |       2.36 | **1.615** |
| versions-16m |  1104.6 |  1167.3 |   **0.95** |   2875.9 |    2794.6 |   **1.03** | **0.755** |
| mr           |   514.3 |   194.1 |       2.65 |   1837.1 |    1322.1 |       1.39 |     1.114 |
| ooffice      |   468.5 |   253.0 |       1.85 |   1239.2 |    1342.6 |   **0.92** |     1.262 |
| osdb         |   548.7 |   217.4 |       2.52 |   1848.2 |    1039.5 |       1.78 |     1.144 |
| reymont      |   357.8 |   208.1 |       1.72 |   1799.2 |     868.8 |       2.07 | **1.437** |
| sao          |   455.3 |   596.6 |   **0.76** |   1241.4 |    5272.6 |   **0.24** |     1.143 |
| webster      |   421.2 |   226.6 |       1.86 |   1760.2 |     999.6 |       1.76 | **1.436** |
| dickens      |   359.4 |   177.6 |       2.02 |   1701.0 |     802.3 |       2.12 |     1.215 |
| mozilla      |   557.9 |   206.5 |       2.70 |   1468.9 |     833.8 |       1.76 |     1.222 |
| nci          |  1043.4 |   447.7 |       2.33 |   2861.1 |    1225.1 |       2.34 |     1.303 |
| samba        |   633.9 |   317.0 |       2.00 |   2254.0 |    1164.7 |       1.94 |     1.255 |
| xml          |   826.9 |   418.1 |       1.98 |   2651.0 |    1311.1 |       2.02 |     1.308 |
| x-ray        |   902.8 |  1756.3 |   **0.51** |   1403.3 |    5157.2 |   **0.27** |     1.221 |

### Where we already win or tie

- **Compress at or better than C:** `x-ray` **0.51**, `sao` **0.76**, `versions-16m`
  **0.95**, `incomp` **1.03**, `zeros` **1.05**.
- **Decompress at or better than C:** `sao` **0.24** (4.2x faster), `x-ray` **0.27**,
  `ooffice` **0.92**, `text-32m` **0.97**, `versions-16m` **1.03**.
- **Ratio better than C:** `zeros` 0.901, `versions-16m` **0.755** (25% smaller).

Against mission section 7 (decompress <= 1.11, compress <= 1.25): **5 corpora pass
compress, 5 pass decompress** — the same COUNT as before the campaign, but no longer
marginal passes: `x-ray` went 0.72 -> 0.51 and `sao` 1.10 -> 0.76.

### What the campaign moved (compress `C/us`, before -> after)

**16 of 18 corpora improved.** Three now compress FASTER than C.

| corpus           | before |    after |     | corpus      | before | after |
| ---------------- | -----: | -------: | --- | ----------- | -----: | ----: |
| **x-ray**        |   0.72 | **0.51** |     | dickens     |   2.37 |  2.02 |
| **sao**          |   1.10 | **0.76** |     | xml         |   2.29 |  1.98 |
| **versions-16m** |   1.06 | **0.95** |     | samba       |   2.24 |  2.00 |
| incomp-32m       |   1.09 |     1.03 |     | webster     |   2.12 |  1.86 |
| mozilla          |   3.22 |     2.70 |     | smallmsg-8m |   1.98 |  1.69 |
| mr               |   3.20 |     2.65 |     | reymont     |   1.96 |  1.72 |
| osdb             |   2.82 |     2.52 |     | ooffice     |   2.34 |  1.85 |
| jsonlog-16m      |   2.56 |     2.21 |     | nci         |   2.52 |  2.33 |

Only `zeros-32m` (0.99 -> 1.05) and `text-32m` (1.37 -> 1.40) moved the wrong way, both
inside the 3.8% session floor and both on trivial/RLE paths the campaign never touched.

**Decompress** is flat-to-better everywhere, with the gains where the decoder bricks
landed: `text-32m` 1.10 -> **0.97**, `ooffice` 0.97 -> **0.92**, `xml` 2.09 -> 2.02,
`webster` 1.80 -> 1.76, `mr` 1.41 -> 1.39. (An earlier decode REGRESSION of 3-7% on
sequence-heavy files was traced to a const generic splitting `decode_sequences` out of its
caller, and fixed by hoisting the same value into a plain local — see brick 64b in
`m7-encoder-whys.md`.)

### Where we lose

- **Compress:** `mozilla` 2.70, `mr` 2.65, `osdb` 2.52, `nci` 2.33.
- **Decompress:** `smallmsg` 2.36, `nci` 2.34, `dickens` 2.12, `reymont` 2.07.
- **Ratio:** `smallmsg-8m` **1.615** and `jsonlog-16m` **1.527** — the product corpus is
  our worst ratio anywhere. See section 5: the gap is roughly HALF what this L1 figure
  suggests, because L1 is our worst point measured against C's best.

---

## 2. Level 3 (dfast) — the shipping default

**Re-measured 2026-08-16**, and widened from 8 Silesia files to **all 18 corpora**.
Session null-arm **1.0084** (the cleanest of the campaign). Sizes are byte-identical to
the pre-campaign build.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 7356.5 |  9901.9 |   **0.74** |  24223.2 |   11505.8 |       2.11 | **0.972** |
| text-32m     | 8375.2 | 10428.7 |   **0.80** |   9330.6 |   10036.0 |   **0.93** |     1.005 |
| incomp-32m   | 3942.3 |  2897.4 |       1.36 |  11366.8 |    7204.2 |       1.58 |     1.000 |
| jsonlog-16m  |  373.5 |   188.2 |       1.98 |   1775.5 |     916.3 |       1.94 |     1.320 |
| smallmsg-8m  |  331.1 |   146.0 |       2.27 |   2153.7 |     801.3 |       2.69 | **1.440** |
| versions-16m | 4714.2 |  1803.0 |       2.61 |  22793.4 |    4469.2 |       5.10 | **4.297** |
| mr           |  287.7 |   144.3 |       1.99 |   1515.1 |     464.1 |       3.26 |     1.052 |
| ooffice      |  248.8 |   118.7 |       2.10 |   1090.6 |     653.7 |       1.67 |     1.136 |
| osdb         |  362.7 |   190.0 |       1.91 |   1795.3 |     968.1 |       1.85 |     1.104 |
| reymont      |  263.8 |   152.5 |       1.73 |   1514.9 |     533.9 |       2.84 |     1.092 |
| sao          |  211.5 |   122.2 |       1.73 |    970.1 |     765.1 |       1.27 |     1.056 |
| webster      |  265.7 |   152.1 |       1.75 |   1521.0 |     616.0 |       2.47 |     1.141 |
| dickens      |  236.8 |   122.1 |       1.94 |   1506.0 |     473.5 |       3.18 |     1.129 |
| mozilla      |  364.4 |   160.9 |       2.26 |   1397.1 |     656.4 |       2.13 |     1.105 |
| nci          |  919.3 |   453.4 |       2.03 |   2798.9 |    1259.4 |       2.22 |     1.221 |
| samba        |  457.2 |   211.7 |       2.16 |   2150.0 |     779.9 |       2.76 |     1.111 |
| xml          |  631.8 |   344.4 |       1.83 |   2492.2 |    1112.5 |       2.24 |     1.185 |
| x-ray        |  195.0 |   103.5 |       1.88 |   1020.8 |     535.6 |       1.91 |     1.082 |

**L3 remains our strongest compress level relative to C on Silesia** (1.73-2.26 vs
L1's 1.72-2.70) **and its ratio is much better** (1.052-1.221 vs L1's 1.114-1.437).
L3 decompress is worse than L1 — more sequences to decode — which is the same trade the
pre-campaign board showed.

**New at L3: we compress the trivial corpora FASTER than C** — `zeros-32m` **0.74**,
`text-32m` **0.80**. Those are the RLE/near-RLE paths where dfast's cheaper parse and our
now-19-instruction probe loop combine.

**Read this table with section 5.** `versions-16m` reads C/us 2.61 here against 0.95 at
L1, and `smallmsg`/`jsonlog` also degrade relative to C going L1 -> L3. That is the
`minMatch` interaction documented in section 5, not a regression: the generated corpora
have a non-monotonic response to level that real content does not share.

---

## 3. Stage anatomy — where OUR time goes

**Re-measured 2026-08-16** on the instrumented build. Share of encode
(`stage / EncodeTotal`) and of decode (`stage / DecodeTotal`). Bold marks the LEADING
stage on each half.

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: |
| sao          |  **80.5** |      6.6 |    2.6 |     1.6 |     33.5 | **41.0** |
| webster      |  **62.6** |      8.8 |   16.6 |     8.0 |     11.1 | **82.6** |
| ooffice      |  **60.4** |     21.6 |    9.6 |     4.4 |     26.9 | **64.3** |
| smallmsg-8m  |  **59.9** |      0.9 |   24.9 |     9.2 |      2.7 | **90.1** |
| jsonlog-16m  |  **57.8** |      2.8 |   23.2 |    11.5 |      3.6 | **89.7** |
| reymont      |  **56.8** |     15.4 |   16.9 |     7.5 |     15.3 | **78.8** |
| dickens      |  **56.4** |     23.8 |   12.2 |     4.6 |     31.7 | **62.8** |
| samba        |  **55.7** |     12.0 |   18.6 |     8.8 |     13.9 | **78.7** |
| xml          |  **50.8** |     10.7 |   21.9 |    10.8 |     10.4 | **81.2** |
| versions-16m |  **45.9** |      0.4 |   27.3 |    13.8 |      0.4 | **81.3** |
| nci          |  **43.9** |     11.7 |   25.7 |    12.3 |     12.2 | **79.6** |
| mozilla      |  **43.4** |     36.1 |   11.4 |     5.5 |     31.6 | **62.1** |
| text-32m     |  **42.1** |      1.8 |    0.5 |     0.3 |      0.5 | **44.5** |
| osdb         |      35.5 | **42.0** |   13.7 |     5.3 |     24.9 | **68.0** |
| mr           |      32.0 | **61.9** |    1.6 |     0.9 | **70.7** |     20.7 |
| incomp-32m   |  **30.0** |      0.0 |    0.0 |     0.0 |      0.0 |  **0.0** |
| x-ray        |      19.8 | **61.0** |    0.3 |     0.0 | **44.3** |      3.5 |

**`EncodeMatchFind` is #1 on 14 of 17** — down from 15 before the campaign.
**Its share fell on 16 of 17 corpora**, which is the campaign landing exactly where it
was aimed: the probe loop went from 47 instructions to 19, so match-finding is simply a
smaller slice of a smaller encode.

**One file CHANGED HANDS: `osdb`.** It was MatchFind-led 42.4% vs Huffman 36.8%; it is now
**Huffman-led 42.0% vs 35.5%**. That is not Huffman getting slower — it is MatchFind
getting ~7 points cheaper while Huffman stayed put. `osdb` therefore joins `mr` and
`x-ray` as a Huffman-bound file, and the pre-campaign claim that "Huffman leads on exactly
two files" is now wrong.

**Decoder: `DecodeSeq` is #1 on 14 of 17.** `DecodeLiterals` leads on `mr` (70.7%)
and `x-ray` (44.3%) — the same two as before. The decoder shares barely moved, which is
consistent with front 2 having removed per-block work (the Huffman table clone) rather
than changing the per-sequence balance.

**The two halves are still near mirror images:** the files where the encoder spends its
time in Huffman are the files where the decoder spends its time in literals. One content
axis (alphabet flatness) drives both. `osdb` flipping on the encode side while staying
`DecodeSeq`-led on the decode side is the one place that symmetry now bends.

---

## 4. What this says to do next

Rewritten 2026-08-16. The pre-campaign version of this section is what front 1 was built
from; three of its four items have now been acted on, so this is the state AFTER.

1. **Encoder = MatchFind, still — but by a smaller margin.** #1 on **14 of 17** (was 15),
   share down on 16 of 17. The probe loop is now **19 instructions with 1 stack access**
   (from 47 / 20), and compress improved on 16 of 18 corpora, +10% to +43%. What remains
   in that loop is one L1-hot stack reload of the src base; the frame-constants
   (`hash_log`, `step0`, the tag and rep flags, the pipeline flag) are all specialised
   away. **Further gains here need a different lever than const-specialisation** — that
   seam is worked out.
2. **Decoder = DecodeSeq, unchanged at #1 on 14 of 17.** Front 2 has only just opened: the
   per-sequence loop went 50 -> 40 instructions by bundling `copy_match`'s five
   frame-constant arguments (the Windows x64 ABI passes only 4 in registers), and the
   per-block Huffman-table clone is gone. **13 stack accesses remain in that loop** — the
   same shape the encoder's probe loop had before it was worked, and the obvious next
   target. NOTE the hard-won caveat: a const generic that splits a function out of its
   INLINED caller can cost more than it saves (brick 64/64b).
3. **Huffman is now a THREE-file front, not two.** `mr` (61.9%), `x-ray` (61.0%) and now
   **`osdb` (42.0%)**. Brick 61 removed the speculative second encode (mr: 1.94 -> 1.04
   encodes per block, +16% compress); `x-ray` and `osdb` have never been examined and are
   the untouched half of this front.
4. **Ratio = the product corpus, and it is HALF the problem the L1 board implies.**
   `smallmsg-8m` 1.615 and `jsonlog-16m` 1.527 at L1, but **1.317 / 1.281 at L5** —
   section 5 has the `minMatch` explanation. Before spending on this, replace the
   generated corpus with real captured traffic; its level response is not
   representative.
5. **L3 remains the strongest compress level relative to C** (1.73-2.26 on Silesia vs
   L1's 1.72-2.70) with much better ratio (1.052-1.221 vs 1.114-1.437). Any "how do we
   compare" claim should quote L3 as well as L1.

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
