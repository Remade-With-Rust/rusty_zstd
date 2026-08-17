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
| versions-16m |  1123.0 |  4014.6 |   **0.28** |   2923.3 |   15038.7 |   **0.19** | **0.075** |
| zeros-32m    | 10728.3 | 22919.7 |   **0.47** |  27197.5 |   39318.5 |   **0.69** | **0.901** |
| incomp-32m   |  5168.6 |  6639.4 |   **0.78** |  12976.2 |   15122.8 |   **0.86** |     1.000 |
| x-ray        |  1035.6 |   395.9 |       2.62 |   1543.0 |    1034.2 |       1.49 |     1.000 |
| smallmsg-8m  |   546.2 |   298.2 |       1.83 |   2403.6 |     919.3 |       2.61 |     1.031 |
| jsonlog-16m  |   695.1 |   307.6 |       2.26 |   1872.7 |     990.6 |       1.89 |     1.061 |
| sao          |   460.5 |   240.3 |       1.92 |   1241.5 |     561.7 |       2.21 |     1.067 |
| osdb         |   572.6 |   226.8 |       2.52 |   1878.7 |    1052.4 |       1.79 |     1.100 |
| text-32m     | 15427.8 | 26751.5 |   **0.58** |  10435.0 |   34369.0 |   **0.30** |     1.100 |
| mr           |   527.8 |   213.9 |       2.47 |   1850.0 |    1363.8 |       1.36 |     1.111 |
| ooffice      |   482.4 |   188.7 |       2.56 |   1285.3 |     869.4 |       1.48 |     1.113 |
| webster      |   421.5 |   212.4 |       1.98 |   1796.5 |     909.0 |       1.98 |     1.137 |
| samba        |   660.2 |   333.7 |       1.98 |   2393.0 |    1168.4 |       2.05 |     1.144 |
| dickens      |   373.7 |   194.1 |       1.93 |   1770.4 |     865.8 |       2.04 |     1.168 |
| mozilla      |   593.8 |   235.0 |       2.53 |   1552.6 |     906.5 |       1.71 |     1.186 |
| xml          |   884.5 |   451.6 |       1.96 |   2845.3 |    1367.0 |       2.08 |     1.217 |
| reymont      |   358.1 |   209.9 |       1.71 |   1801.6 |     848.8 |       2.12 |     1.240 |
| nci          |  1107.4 |   547.7 |       2.02 |   3024.1 |    1380.6 |       2.19 |     1.301 |

### Where we win or tie

- **Compress at or better than C:** `versions-16m` **0.28**, `zeros-32m` **0.47**,
  `text-32m` **0.58**, `incomp-32m` **0.78**.
- **Decompress at or better than C:** `versions-16m` **0.19**, `text-32m` **0.30**,
  `zeros-32m` **0.69**, `incomp-32m` **0.86**.
- **Ratio at or better than C:** `versions-16m` **0.075**, `zeros-32m` **0.901**,
  `incomp-32m` 1.000, `x-ray` 1.000.

Against mission section 7 (decompress <= 1.11, compress <= 1.25):
**4 corpora pass compress, 4 pass decompress.** Down from 6 and 7 -- see the correction
in the header. `sao` and `x-ray` left the winners' column because they stopped being
scored against our own inflated output, not because either got worse at equal size.

### Where we lose

- **Ratio:** `nci` **1.301**, `reymont` 1.240, `xml` 1.217, `mozilla` 1.186. The old
  headline losers are gone -- `smallmsg-8m` 1.615 -> **1.031** and `jsonlog-16m`
  1.528 -> **1.061** are now among our BEST ratios.
- **Compress:** `x-ray` **2.62**, `ooffice` 2.56, `mozilla` 2.53, `osdb` 2.52.
  `x-ray` is the campaign's one KNOWN OPEN REGRESSION: 1968.9 -> 395.9 MB/s, bought
  with 1.212 -> 1.000 ratio. It needs the `(H1, distinct)` dispatch (section 4).

**`versions-16m` remains the headline** -- 0.075 size, 13x smaller than C, from the
repcode bricks (67/70/71/73/75). It was also the corpus that exposed the missing
function: `find_fast` had a repcode search and no other finder did.

---

## 2. Level 3 (dfast) - the shipping default

**Re-measured 2026-08-17.** Sorted by `us/c size`.

| corpus       | C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | -----: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m | 4943.4 |  4126.5 |       1.20 |  24229.1 |   20830.9 |       1.16 | **0.648** |
| zeros-32m    | 7779.9 | 22369.6 |   **0.35** |  27028.2 |   39406.3 |   **0.69** | **0.972** |
| x-ray        |  222.0 |   100.5 |       2.21 |   1136.5 |     441.8 |       2.57 | **0.982** |
| incomp-32m   | 4773.9 |  4000.3 |       1.19 |  12687.0 |   15692.8 |   **0.81** |     1.000 |
| mr           |  304.3 |   151.9 |       2.00 |   1630.8 |     482.1 |       3.38 |     1.020 |
| ooffice      |  271.4 |   113.8 |       2.38 |   1170.1 |     517.3 |       2.26 |     1.029 |
| osdb         |  384.1 |   175.1 |       2.19 |   1910.7 |     822.1 |       2.32 |     1.032 |
| jsonlog-16m  |  408.4 |   209.0 |       1.95 |   1991.1 |     955.4 |       2.08 |     1.036 |
| sao          |  227.7 |   120.1 |       1.90 |   1048.4 |     549.2 |       1.91 |     1.039 |
| smallmsg-8m  |  340.4 |   158.9 |       2.14 |   2360.6 |     848.4 |       2.78 |     1.049 |
| reymont      |  299.2 |   180.2 |       1.66 |   1666.4 |     607.8 |       2.74 |     1.054 |
| webster      |  279.0 |   167.1 |       1.67 |   1652.0 |     613.3 |       2.69 |     1.057 |
| dickens      |  250.8 |   143.2 |       1.75 |   1603.7 |     532.3 |       3.01 |     1.066 |
| mozilla      |  387.2 |   165.5 |       2.34 |   1483.9 |     652.2 |       2.28 |     1.071 |
| samba        |  481.8 |   258.3 |       1.87 |   2309.5 |     917.9 |       2.52 |     1.073 |
| text-32m     | 8969.0 | 25745.7 |   **0.35** |  10397.5 |   34717.5 |   **0.30** |     1.082 |
| nci          |  977.2 |   510.7 |       1.91 |   2935.8 |    1365.3 |       2.15 |     1.100 |
| xml          |  742.3 |   407.4 |       1.82 |   2830.1 |    1270.9 |       2.23 |     1.104 |

**L3 is the shipping default, and on RATIO it is now excellent: 15 of 18 corpora are
within 11% of C, and three BEAT it** -- `versions-16m` 0.648, `zeros-32m` 0.972 and
**`x-ray` 0.982**, which is new. The worst cell on the whole board is `xml` at 1.104.

Compress is 1.66-2.38 across Silesia; decompress 1.91-3.38. **Decompress is now the
weaker half at L3**, the reverse of the pre-gate picture, and it follows directly from
emitting Huffman literals where we used to emit raw ones.

Against mission section 7: **4 pass compress** (zeros 0.35, text 0.35, incomp 1.19,
versions 1.20), **3 pass decompress** (text 0.30, zeros 0.69, incomp 0.81).

---

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

### MatchFind Function Anatomy -- Great Gate

### Huff Function Anatomy -- Great Gate

### FseSeq Function Anatomy -- Great Gate

### SeqCode Function Anatomy -- Great Gate

### DecLits Function Anatomy -- Great Gate

### DecSeq Function Anatomy -- Great Gate

### DecCk Function Anatomy -- Great Gate