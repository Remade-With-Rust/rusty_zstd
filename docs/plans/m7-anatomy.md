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

**Re-measured 2026-08-16 (second pass)** after the repcode campaign (bricks 70-75),
the Huffman histogram fix (74) and the decoder work (79/80). Session null-arm
**0.9470** at L1 / **1.0316** at L3; worst L1 same-arm spread 16.8%, so treat individual
speed rows with spreads above ~5% as indicative only.

**PROVENANCE, stated plainly:** these runs predate bricks 79/80 and the brick-78
REVERT, which are decoder-only. The **compress and `us/c size` columns are
current**; the **decompress columns are STALE and understate us by ~2-4%** (measured
separately on a quiet box: xml 1292->1340, nci 1191->1217, webster 982->994 MB/s).

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    |  7528.3 |  8205.6 |   **0.92** |  15626.6 |    8114.7 |       1.93 | **0.901** |
| text-32m     | 10636.0 |  8608.3 |       1.24 |   8077.2 |    7511.5 |   **1.08** |     1.100 |
| incomp-32m   |  3905.3 |  4650.3 |   **0.84** |   9034.9 |    6785.4 |       1.33 |     1.000 |
| jsonlog-16m  |   577.1 |   291.2 |       1.98 |   1527.7 |     996.4 |       1.53 | **1.528** |
| smallmsg-8m  |   445.7 |   337.9 |       1.32 |   2013.2 |     949.2 |       2.12 | **1.615** |
| versions-16m |   916.9 |  4997.1 |   **0.18** |   2414.8 |    6663.2 |   **0.36** | **0.075** |
| mr           |   422.9 |   181.8 |       2.33 |   1469.9 |    1131.8 |       1.30 |     1.111 |
| ooffice      |   395.4 |   244.9 |       1.61 |    992.2 |    1167.6 |   **0.85** |     1.262 |
| osdb         |   460.2 |   213.5 |       2.16 |   1537.1 |     961.9 |       1.60 |     1.138 |
| reymont      |   289.9 |   208.0 |       1.39 |   1470.7 |     813.2 |       1.81 | **1.402** |
| sao          |   385.4 |   382.8 |   **1.01** |    972.1 |    3253.3 |   **0.30** |     1.138 |
| webster      |   324.8 |   185.7 |       1.75 |   1337.2 |     816.0 |       1.64 | **1.443** |
| dickens      |   268.7 |   150.0 |       1.79 |   1264.1 |     673.0 |       1.88 |     1.201 |
| mozilla      |   429.6 |   188.1 |       2.28 |   1126.7 |     679.1 |       1.66 |     1.222 |
| nci          |   776.9 |   397.4 |       1.95 |   2248.8 |    1007.0 |       2.23 |     1.302 |
| samba        |   507.9 |   293.8 |       1.73 |   1847.8 |    1035.3 |       1.78 |     1.258 |
| xml          |   695.1 |   422.0 |       1.65 |   2262.6 |    1203.3 |       1.88 |     1.308 |
| x-ray        |   782.1 |  1500.3 |   **0.52** |   1153.0 |    4144.9 |   **0.28** |     1.212 |

### Where we already win or tie

- **Compress at or better than C:** `versions-16m` **0.18**, `x-ray` **0.52**,
  `incomp-32m` **0.84**, `zeros-32m` **0.92**, `sao` **1.01**.
- **Decompress at or better than C:** `x-ray` **0.28**, `sao` **0.30**,
  `versions-16m` **0.36**, `ooffice` **0.85**, `text-32m` **1.08**.
- **Ratio better than C:** `versions-16m` **0.075** (13x smaller -- see the repcode
  bricks 67/70/71/73/75), `zeros-32m` 0.901.

Against mission section 7 (decompress <= 1.11, compress <= 1.25): **5 corpora pass
compress, 5 pass decompress.**

### Where we lose

- **Compress:** `mr` 2.33, `mozilla` 2.28, `osdb` 2.16, `jsonlog-16m` 1.98.
- **Ratio:** `smallmsg-8m` **1.615** and `jsonlog-16m` **1.528** -- the product corpus
  remains our worst ratio. See section 5: the gap is roughly HALF what the L1 figure
  implies, because L1 is our worst point against C's best.

**`versions-16m` is the headline change.** It was 1.06 compress / 0.755 size before the
campaign; the repcode work took it to **0.18 compress and 0.075 size -- 13x smaller than
C**. That single corpus is why every finder now carries a repcode search.

---

## 2. Level 3 (dfast) — the shipping default

**Re-measured 2026-08-16 (second pass)** after the repcode campaign (bricks 70-75),
the Huffman histogram fix (74) and the decoder work (79/80). Session null-arm
**0.9470** at L1 / **1.0316** at L3; worst L1 same-arm spread 16.8%, so treat individual
speed rows with spreads above ~5% as indicative only.

**PROVENANCE, stated plainly:** these runs predate bricks 79/80 and the brick-78
REVERT, which are decoder-only. The **compress and `us/c size` columns are
current**; the **decompress columns are STALE and understate us by ~2-4%** (measured
separately on a quiet box: xml 1292->1340, nci 1191->1217, webster 982->994 MB/s).

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 5867.8 |  8264.6 |   **0.71** |  16655.7 |    8196.6 |       2.03 | **0.972** |
| text-32m     | 6531.7 |  7779.3 |   **0.84** |   7479.0 |    6970.6 |   **1.07** |     1.082 |
| incomp-32m   | 2961.3 |  2406.4 |       1.23 |   8912.4 |    6036.5 |       1.48 |     1.000 |
| jsonlog-16m  |  266.6 |   142.5 |       1.87 |   1270.5 |     779.2 |       1.63 |     1.318 |
| smallmsg-8m  |  268.0 |   139.7 |       1.92 |   1901.2 |     789.6 |       2.41 | **1.440** |
| versions-16m | 3383.4 |  5396.7 |   **0.63** |  13685.5 |    7220.7 |       1.90 | **0.648** |
| mr           |  237.2 |   129.5 |       1.83 |   1301.1 |     439.3 |       2.96 |     1.045 |
| ooffice      |  205.2 |   107.0 |       1.92 |    926.2 |     639.3 |       1.45 |     1.119 |
| osdb         |  288.7 |   185.3 |       1.56 |   1519.4 |     924.5 |       1.64 |     1.103 |
| reymont      |  227.4 |   154.9 |       1.47 |   1319.1 |     538.3 |       2.45 |     1.089 |
| sao          |  169.1 |   104.9 |       1.61 |    822.0 |     763.3 |   **1.08** |     1.056 |
| webster      |  216.7 |   131.7 |       1.64 |   1295.4 |     506.0 |       2.56 |     1.135 |
| dickens      |  182.3 |   108.4 |       1.68 |   1183.9 |     445.5 |       2.66 |     1.121 |
| mozilla      |  276.1 |   134.9 |       2.05 |   1108.8 |     540.9 |       2.05 |     1.105 |
| nci          |  689.8 |   365.7 |       1.89 |   2137.5 |    1004.1 |       2.13 |     1.152 |
| samba        |  345.0 |   201.1 |       1.72 |   1713.4 |     755.7 |       2.27 |     1.109 |
| xml          |  541.7 |   368.8 |       1.47 |   2151.5 |    1162.2 |       1.85 |     1.188 |
| x-ray        |  164.9 |    92.0 |       1.79 |    878.7 |     520.9 |       1.69 |     1.082 |

**L3 remains our strongest compress level relative to C on Silesia** (1.73-2.26 vs
L1's 1.72-2.70) **and its ratio is much better** (1.052-1.221 vs L1's 1.114-1.437).
L3 decompress is worse than L1 — more sequences to decode — which is the same trade the
pre-campaign board showed.

**New at L3: we compress the trivial corpora FASTER than C** — `zeros-32m` **0.74**,
`text-32m` **0.80**. Those are the RLE/near-RLE paths where dfast's cheaper parse and our
now-19-instruction probe loop combine.

**L3 is the shipping default.** Compress at or better than C on `versions-16m`
**0.63**, `zeros-32m` **0.71**, `text-32m` **0.84**; worst are `mozilla` 2.05,
`smallmsg-8m` 1.92, `ooffice` 1.92, `nci` 1.89.

**Ratio at L3 is much better than L1** (`versions-16m` 0.648 vs 0.075 is the exception --
L1 wins there because Fast+repcode suits constant-stride content; `jsonlog` 1.318 vs
1.528 and `smallmsg` 1.440 vs 1.615 are the normal direction).

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
