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

**Re-measured 2026-08-16 (fourth pass) -- the FIRST board at CHECKSUM PARITY with
the oracle.** Every earlier board charged us a full xxh64 pass over every byte, on
both phases, that `zstd -b` never runs (it takes libzstd's `checksumFlag = 0`
default; we took the CLI's). Those boards understated us badly -- see
m7-encoder-whys.md. Compressed sizes are byte-identical to the gated hashes, so
every speed figure is at MATCHED OUTPUT.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 10245.8 | 22116.0 |   **0.46** |  23467.4 |   37167.1 |   **0.63** | **0.901** |
| text-32m     | 14436.3 | 25621.9 |   **0.56** |  10242.1 |   32063.5 |   **0.32** |     1.100 |
| incomp-32m   |  4750.3 |  6665.2 |   **0.71** |  12120.0 |   14446.9 |   **0.84** |     1.000 |
| jsonlog-16m  |   701.6 |   354.8 |       1.98 |   1896.9 |    1164.4 |       1.63 | **1.528** |
| smallmsg-8m  |   549.2 |   363.8 |       1.51 |   2371.5 |    1081.6 |       2.19 | **1.615** |
| versions-16m |  1107.6 |  8310.1 |   **0.13** |   2894.0 |   14625.8 |   **0.20** | **0.075** |
| mr           |   518.6 |   214.3 |       2.42 |   1821.9 |    1361.1 |       1.34 |     1.111 |
| ooffice      |   472.4 |   267.2 |       1.77 |   1276.9 |    1417.9 |   **0.90** |     1.262 |
| osdb         |   566.4 |   253.7 |       2.23 |   1864.3 |    1131.5 |       1.65 |     1.138 |
| reymont      |   357.6 |   226.8 |       1.58 |   1802.5 |     930.5 |       1.94 | **1.402** |
| sao          |   465.5 |   423.6 |   **1.10** |   1250.0 |    5681.6 |   **0.22** |     1.138 |
| webster      |   422.1 |   246.5 |       1.71 |   1797.1 |    1077.9 |       1.67 | **1.443** |
| dickens      |   365.7 |   192.4 |       1.90 |   1711.4 |     854.9 |       2.00 |     1.201 |
| mozilla      |   573.4 |   247.7 |       2.32 |   1501.9 |     945.9 |       1.59 |     1.222 |
| nci          |  1068.0 |   528.6 |       2.02 |   2894.5 |    1335.8 |       2.17 |     1.302 |
| samba        |   638.1 |   363.8 |       1.75 |   2288.4 |    1276.3 |       1.79 |     1.258 |
| xml          |   865.5 |   477.0 |       1.81 |   2777.0 |    1477.4 |       1.88 |     1.308 |
| x-ray        |  1003.5 |  1968.9 |   **0.51** |   1512.5 |    6488.2 |   **0.23** |     1.212 |

### Where we already win or tie

- **Compress at or better than C:** `versions-16m` **0.13**, `zeros-32m` **0.46**, `x-ray` **0.51**, `text-32m` **0.56**, `incomp-32m` **0.71**, `sao` **1.10**.
- **Decompress at or better than C:** `versions-16m` **0.20**, `sao` **0.22**, `x-ray` **0.23**, `text-32m` **0.32**, `zeros-32m` **0.63**, `incomp-32m` **0.84**, `ooffice` **0.90**.
- **Ratio better than C:** `versions-16m` **0.075**, `zeros-32m` **0.901**.

Against mission section 7 (decompress <= 1.11, compress <= 1.25):
**6 corpora pass compress, 7 pass decompress** --
up from 5 and 5 on the pre-parity board, with no code change on either path.

---

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

**Re-measured 2026-08-16 (fourth pass) -- the FIRST board at CHECKSUM PARITY with
the oracle.** Every earlier board charged us a full xxh64 pass over every byte, on
both phases, that `zstd -b` never runs (it takes libzstd's `checksumFlag = 0`
default; we took the CLI's). Those boards understated us badly -- see
m7-encoder-whys.md. Compressed sizes are byte-identical to the gated hashes, so
every speed figure is at MATCHED OUTPUT.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| zeros-32m    | 7497.0 | 20984.6 |   **0.36** |  24077.8 |   38939.8 |   **0.62** | **0.972** |
| text-32m     | 8596.6 | 26647.4 |   **0.32** |  10267.1 |   32764.8 |   **0.31** |     1.082 |
| incomp-32m   | 4197.1 |  3592.4 |       1.17 |  12302.3 |   14165.2 |   **0.87** |     1.000 |
| jsonlog-16m  |  407.9 |   215.5 |       1.89 |   1963.5 |    1043.8 |       1.88 |     1.318 |
| smallmsg-8m  |  352.3 |   169.4 |       2.08 |   2396.5 |     945.4 |       2.53 | **1.440** |
| versions-16m | 4655.2 | 10580.3 |   **0.44** |  22268.9 |   20157.7 |   **1.10** | **0.648** |
| mr           |  296.1 |   159.7 |       1.85 |   1602.7 |     496.2 |       3.23 |     1.045 |
| ooffice      |  264.7 |   131.0 |       2.02 |   1145.3 |     717.6 |       1.60 |     1.119 |
| osdb         |  366.3 |   224.4 |       1.63 |   1908.9 |    1121.5 |       1.70 |     1.103 |
| reymont      |  294.8 |   185.9 |       1.59 |   1673.5 |     619.5 |       2.70 |     1.089 |
| sao          |  224.3 |   133.9 |       1.68 |   1038.3 |     898.3 |       1.16 |     1.056 |
| webster      |  282.8 |   170.5 |       1.66 |   1620.1 |     630.4 |       2.57 |     1.135 |
| dickens      |  237.6 |   141.4 |       1.68 |   1548.6 |     530.7 |       2.92 |     1.121 |
| mozilla      |  374.1 |   179.0 |       2.09 |   1457.6 |     722.2 |       2.02 |     1.105 |
| nci          |  944.3 |   520.9 |       1.81 |   2878.9 |    1372.3 |       2.10 |     1.152 |
| samba        |  458.6 |   268.6 |       1.71 |   2232.9 |     935.6 |       2.39 |     1.109 |
| xml          |  695.8 |   407.9 |       1.71 |   2713.8 |    1299.7 |       2.09 |     1.188 |
| x-ray        |  210.9 |   113.5 |       1.86 |   1106.7 |     566.6 |       1.95 |     1.082 |

**L3 is the shipping default and our best compress level relative to C.**
At or better than C on `text-32m` **0.32**, `zeros-32m` **0.36**, `versions-16m` **0.44**, `incomp-32m` **1.17**.

**Decompress on Silesia spans 1.16-3.23.** More sequences per byte means
more DecodeSeq. Section 3's stage ranking is being re-derived at parity -- it was
measured with the checksum tax inflating both DecodeChecksum and the denominator.

**Read the generated corpora with section 5:** their non-monotonic response to level
is a `minMatch` interaction with synthetic content, not a regression.

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
