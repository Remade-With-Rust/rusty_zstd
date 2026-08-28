# M7 anatomy - rusty_zstd vs facebook/zstd v1.5.7, side by side

**Date:** 2026-08-22 (**third pass**) - every table on this page re-measured after
the **MatchFind monomorphisation campaign** (20 wins: the `HLOG`, `STEP` and `PIPE`
const-generic axes proven redundant on the BMI2 twins and collapsed, `emit_fast_seq`
outlined behind a `FastEmitCtx` with its own ISA twin, and eleven dead
computations removed). Earlier passes: the BMI2 twin campaign (`08d14e3`..`bfd0cf0`),
the 08-20 gate/dispatch session (`30e6863`..`0fbc57b`), and the per-finder campaign
(`..d012c6c`). Older boards are in git history.

> **PROVENANCE SPLIT, 2026-08-26 -- READ THIS BEFORE ANY CELL ON THIS PAGE.** The
> page no longer has one date, and pretending it does would be the exact instrument
> error `m7-benchmark-repair.md` exists to prevent.
>
> | what | when | status |
> |---|---|---|
> | **Speed boards** (sections 1, 2, 6) -- ms/MiB, MB/s, C/us ratios | 2026-08-22 | **NOT re-boarded.** Four decode campaigns have landed since (`inline-execution.md` 17-18, 23-27) and the host has carried a concurrent workstream all session. These cells are HISTORICAL. |
> | **Size / `us/c size` cells** | 2026-08-22 | **STILL EXACT.** `bytegate` GOLD `BE0071FB0CB0CED9` is re-verified after every brick and has not moved through any of it. |
> | **Section 3(c) function census, 3(d) loop counts, 3(e) op ladder** | **2026-08-26** | Re-taken on the current tree. |
>
> The rule this page has always run on applies to itself: **a speed cell and a count
> cell from different builds are not a comparison.** The counts below are current; the
> clocks above them are not, and no claim on this page pairs them.
>
> **Why the speed boards were NOT re-taken on 2026-08-26, with the receipt.** The
> `simserver` demo was launched on the current build and driven over the L3 corpora.
> Three consecutive runs of the IDENTICAL measurement (`dickens`, L3, 4 MiB,
> best-of-7) returned **577.4, 395.2 and 617.0 MB/s decompress** -- a 56% spread, with
> 106 busy processes on the host and the demo's own same-arm spread reading 13.22%,
> 5.93% and 2.85% across those same three runs. The first sweep's `dickens` cell
> (370 MB/s, "3.35x C") was a draw from the bottom of that distribution and means
> nothing; 577 and 617 bracket the 603 measured this morning, so **no regression is
> indicated and none is claimed.** A cell that moves 56% between identical runs cannot
> be put on a board, and the correct response is to publish the deterministic columns
> and leave the clocks alone.
>
> **What the demo session DID establish, deterministically:** `bytegate` GOLD
> `BE0071FB0CB0CED9` (59,760,356 bytes) unmoved on the current tree, and the L3 size
> ratios against C v1.5.7 -- `mozilla` **1.0036**, `webster` 1.0513, `dickens` 1.0528,
> `nci` **1.0982**. Compressed sizes are deterministic and carry no noise caveat.
> These confirm `inline-execution.md` section 21's finding (a ~5% L3 ratio gap that is
> an ENCODER search-quality question, not a decode one) and localise it: `nci` is the
> worst cell at 9.8% and `mozilla` is at parity, so whatever it is, it is
> content-dependent rather than uniform.

> **THE DETERMINISTIC RESULT, FIRST.** All **36** `us/c size` cells on this page --
> 18 at L1, 18 at L3 -- are unchanged for the **THIRD CONSECUTIVE BOARD**. The twelve
> identity totals agree: L1 43,313,087 | L2 52,103,625 | L3 40,681,863 | L4 40,242,889 |
> L5 39,281,120 | L7 38,243,653 | L9 37,836,902 | L12 37,259,874 | L13 12,603,507 |
> L16 12,381,377 | L19 12,365,806 | L22 12,365,762. Three campaigns in a row have moved
> **zero bytes**.

> **THIS BOARD IS COMPARABLE TO THE FIRST PASS; THE SECOND WAS NOT.** C v1.5.7 is a
> fixed binary and therefore this page's calibrator. It now lands within **3%** of its
> first-pass absolutes (`dickens` 334.6 -> 345.9, `versions-16m` 1604.8 -> 1641.7,
> `text-32m` 20072.9 -> 19454.1), against **0.85x** on the second pass. The session
> null arm is **6.83%** at L1 -- the LOWEST ever recorded on this instrument, against
> 13.20% on the first pass and 17.67% on the second. The host was quiet. Speed cells
> here can be read against the first-pass board; L3's null arm is 15.05% and its cells
> cannot be read as finely.

> **WHAT THE MATCHFIND CAMPAIGN DID AND DID NOT DO.** It cut the MatchFind instruction
> census from **572,667 to 130,584 (-77.2%)**, and the BMI2 path that actually executes
> from **282,373 to 22,268 (-92.1%)**. Encode wall-clock on this board is **FLAT**
> (median 1.01x of the first pass). Both facts are correct and they do not conflict:
> what the campaign deleted was 140-fold DUPLICATED monomorphisations of the same
> loop -- code that was never executed, only linked. Deleting never-executed code
> shrinks the binary and the I-cache footprint; it does not make the executed path
> shorter. **No encode speed claim is made here, and none was made when the wins were
> taken** -- every one shipped on a static instruction count plus byte-identity.

> **THE DECODE MOVEMENT ON THIS BOARD IS NOT FROM THIS CAMPAIGN.** Our decompress arm
> is up **13-24%** on every text-like corpus at L1 (`reymont` +24.1%, `mozilla` +24.0%,
> `jsonlog-16m` +23.8%, `smallmsg-8m` +22.7%, `nci` +20.4%) while C's decode arm is
> flat at +3%. That belongs to concurrent `fse.rs`/`huffman.rs` work (an `FseEntry`
> side-word fusion and an `FseView` handle), NOT to MatchFind, which does not run
> during decode at all. L1 mean `C/us` decomp moves 1.66 -> **1.49** on the strength
> of it. Section 3's `DecLits` shares fall correspondingly.

**Instrument:** repaired (see [`m7-benchmark-repair.md`](m7-benchmark-repair.md)) --
best-of-N both arms, phases timed separately as C does, N>=25/phase, discarded warmup,
per-row same-arm spread, cycles/byte. Dual gate 18/18 at both levels. C via `-b -T1`,
**unpinned** (see the pinning note below). Decompress is timed into a REUSED buffer via
`decompress_into`, as C's `-b` does. Checksum parity with the oracle
(`ZSTD_c_checksumFlag = 0`); the shipped default is still `checksum: true` -- that
changed the MEASUREMENT, not the product. Stage shares (section 3) come from a separate
`--features profile` build and are comparable only as shares within a run.

> **A NOTE ON PINNING.** An earlier revision of the protocol line said `affinity=4`.
> Taking that literally on this host is a MEASUREMENT BUG: a single-core pin under the
> Balanced power plan holds the i7-14650HX at its 2200 MHz base with no turbo, which
> drove the C calibrator to **0.29x** and manufactured an apparent 16-of-18 compress
> sweep for us. All boards from the second pass onward are UNPINNED.

`C/us` > 1 means **C is faster**. `us/c size` > 1 means **we emit more bytes**.
**Never average these files** -- the per-file spread is the whole story.

---

SIMD.rs AND XXH64.rs NEED OPTIMIZATION

## 1. Level 1 - the full board, all 18 corpora

**Re-measured 2026-08-22 (third pass).** Sorted by `us/c size`. C is v1.5.7, unpinned.
Session null arm (worst same-arm encode spread) **6.83%** -- the lowest this instrument
has ever recorded, and the reason this board's cells can be read against the first
pass at all.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  1641.7 |  7057.8 |   **0.23** |   3936.3 |   20481.3 |   **0.19** | **0.088** |
| zeros-32m    | 12579.3 | 31961.6 |   **0.39** |  38223.5 |   41710.1 |   **0.92** | **0.967** |
| incomp-32m   |  5921.6 |  3324.1 |       1.78 |  22293.3 |   24081.9 |   **0.93** |     1.000 |
| x-ray        |   906.4 |   363.5 |       2.49 |   1322.3 |    1109.4 |       1.19 |     1.000 |
| smallmsg-8m  |   527.9 |   290.5 |       1.82 |   2314.9 |    1191.5 |       1.94 |     1.009 |
| mr           |   481.2 |   205.5 |       2.34 |   1668.0 |    1046.3 |       1.59 |     1.010 |
| osdb         |   529.5 |   210.3 |       2.52 |   1729.6 |    1104.8 |       1.57 |     1.011 |
| ooffice      |   443.3 |   169.7 |       2.61 |   1143.5 |     818.5 |       1.40 |     1.011 |
| sao          |   449.4 |   225.5 |       1.99 |   1134.9 |     652.0 |       1.74 |     1.011 |
| samba        |   575.8 |   311.4 |       1.85 |   2431.4 |    1244.6 |       1.95 |     1.012 |
| webster      |   396.4 |   211.9 |       1.87 |   1732.6 |     909.8 |       1.90 |     1.014 |
| dickens      |   345.9 |   188.3 |       1.84 |   1648.8 |     867.9 |       1.90 |     1.015 |
| reymont      |   341.4 |   190.2 |       1.79 |   1767.2 |     800.6 |       2.21 |     1.015 |
| mozilla      |   716.8 |   348.8 |       2.06 |   2120.8 |    1270.2 |       1.67 |     1.034 |
| xml          |   798.2 |   410.3 |       1.95 |   2624.6 |    1357.3 |       1.93 |     1.050 |
| jsonlog-16m  |   647.0 |   336.0 |       1.93 |   1776.8 |    1189.7 |       1.49 |     1.063 |
| nci          |   954.9 |   487.5 |       1.96 |   2689.7 |    1375.8 |       1.95 |     1.106 |
| text-32m     | 19454.1 | 12777.5 |       1.52 |   9648.1 |   34379.0 |   **0.28** |     1.126 |

**mean C/us comp 1.83, decomp 1.49 | mean ratio 0.975 | worst ratio 1.126 (text-32m) |
we beat C: 2 comp, 4 decomp, 2 ratio**

**THE RATIO STORY IS STILL THE STORY, and it tightened again.** The worst
NON-DEGENERATE cell moved from **1.131 (`reymont`) to 1.106 (`nci`)** -- `text-32m`
1.126 now tops the sorted board only because everything beneath it collapsed toward
parity. Eleven of eighteen rows now sit at or below **1.016**:

| corpus | `us/c size` 08-20 | 08-21 |
| --- | ---: | ---: |
| `reymont` | 1.131 | **1.015** |
| `nci` | 1.129 | **1.106** |
| `mr` | 1.089 | **1.010** |
| `dickens` | 1.082 | **1.015** |
| `sao` | 1.065 | **1.011** |
| `osdb` | 1.045 | **1.011** |
| `webster` | 1.053 | **1.014** |
| `xml` | 1.086 | **1.050** |

These size moves belong to the 08-20 gate/dispatch session. **Neither 08-21 campaign
moved them** -- the BMI2 twins are byte-exact by construction, and the per-finder
campaign was held to byte-identity receipt by receipt; the `08-21` column above is
now the value on THREE consecutive boards. L1's mean ratio still reads **0.975**: at
matched settings we emit FEWER bytes than C on average, carried by `versions-16m`
0.088 and a mid-board that has converged to ~1.01.

### Where we win or tie

- **Compress at or better than C:** `versions-16m` **0.23**, `zeros-32m` **0.39**.
- **Decompress at or better than C:** `versions-16m` **0.19**, `text-32m` **0.28**,
  `zeros-32m` **0.92**, `incomp-32m` **0.93**.
- **Ratio at or better than C:** `versions-16m` **0.088**, `zeros-32m` **0.967**,
  `incomp-32m` 1.000, `x-ray` 1.000. **Unchanged, cell for cell, three boards running.**

Against mission section 7 (decompress <= 1.11, compress <= 1.25): **2 pass compress**
(versions 0.23, zeros 0.39), **4 pass decompress** (versions 0.19, text 0.28, zeros
0.92, incomp 0.93). These are the SAME counts the first pass reported, which is the
right result to get from a quiet host after a byte-exact campaign -- the second pass's
3-and-6 was the artifact.

**The decompress column is where this board actually moved.** Every text-like corpus
gained 13-24% against its first-pass self while C's decode arm stayed flat, pulling
mean `C/us` decomp from 1.66 to **1.49**. That is the concurrent FSE/Huffman decode
work, not MatchFind -- see the preamble.

### Where we lose

- **Ratio:** `text-32m` **1.126**, `nci` **1.106**, `jsonlog-16m` 1.063, `xml` 1.050.
  Everything else is at or below 1.034. The 08-20 losers are gone: `reymont` 1.131 ->
  **1.015**, `mr` 1.089 -> **1.010**, `dickens` 1.082 -> **1.015**.
- **Compress:** `ooffice` **2.61**, `osdb` 2.52, `x-ray` 2.49, `mr` 2.34. These are
  the honest, matched-output positions; none is an open regression, and all four sit
  within a point of their first-pass values (2.55 / 2.54 / 2.40 / 2.29) -- which is
  what a byte-exact campaign on a quiet host should produce.

**`versions-16m` remains the headline** -- 0.075 size, 13x smaller than C, from the
repcode bricks (67/70/71/73/75). It was also the corpus that exposed the missing
function: `find_fast` had a repcode search and no other finder did.

---

## 2. Level 3 (dfast) - the shipping default

**Re-measured 2026-08-22 (third pass).** Sorted by `us/c size`. Session null arm
**15.05%** -- much wider than L1's 6.83% on the same host and in the same session, so
these speed columns are INDICATIVE ONLY. Ratio is exact.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  4620.4 |  7180.7 |   **0.64** |  22229.5 |   20746.9 |       1.07 | **0.659** |
| zeros-32m    |  7735.5 | 27903.7 |   **0.28** |  38664.7 |   40795.5 |   **0.95** | **0.967** |
| x-ray        |   183.5 |    78.7 |       2.33 |   1005.3 |     490.5 |       2.05 | **0.983** |
| incomp-32m   |  5714.4 |  2469.2 |       2.31 |  22112.1 |   22805.0 |   **0.97** |     1.000 |
| jsonlog-16m  |   383.7 |   214.5 |       1.79 |   1901.1 |    1134.1 |       1.68 |     1.010 |
| osdb         |   339.5 |   156.2 |       2.17 |   1787.7 |     939.2 |       1.90 |     1.016 |
| mr           |   287.5 |   133.8 |       2.15 |   1424.9 |     549.5 |       2.59 |     1.022 |
| smallmsg-8m  |   315.9 |   149.8 |       2.11 |   2281.1 |    1113.1 |       2.05 |     1.029 |
| mozilla      |   390.2 |   200.5 |       1.95 |   1945.6 |    1174.2 |       1.66 |     1.029 |
| ooffice      |   221.9 |    94.8 |       2.34 |    974.1 |     574.5 |       1.70 |     1.032 |
| sao          |   194.7 |    84.8 |       2.30 |    955.9 |     556.3 |       1.72 |     1.034 |
| samba        |   418.6 |   238.5 |       1.76 |   2373.4 |    1200.2 |       1.98 |     1.036 |
| reymont      |   265.3 |   149.2 |       1.78 |   1466.1 |     657.1 |       2.23 |     1.041 |
| webster      |   254.4 |   141.3 |       1.80 |   1535.5 |     694.3 |       2.21 |     1.050 |
| dickens      |   223.8 |   120.2 |       1.86 |   1440.8 |     613.0 |       2.35 |     1.052 |
| text-32m     | 10194.4 | 23446.7 |   **0.43** |   9882.7 |   32746.6 |   **0.30** |     1.071 |
| xml          |   632.5 |   328.6 |       1.92 |   2493.8 |    1282.6 |       1.94 |     1.079 |
| nci          |   816.3 |   393.8 |       2.07 |   2556.5 |    1412.8 |       1.81 |     1.100 |

**mean C/us comp 1.78, decomp 1.73 | mean ratio 1.012 | worst ratio 1.100 (nci) |
we beat C: 3 comp, 3 decomp, 3 ratio**

**L3 ratio is UNCHANGED cell-for-cell -- now across FOUR consecutive boards** (08-20,
and all three 08-21/08-22 passes). Every `us/c size` matches to the third decimal,
which is what a byte-exact campaign predicts and doubles as an end-to-end identity
check on the monomorphisation cuts. Mean **1.012**, worst `nci` **1.100**. Three
corpora still BEAT C -- `versions-16m` 0.659, `zeros-32m` 0.967 and `x-ray` 0.983.

**Decompress narrowed here too** -- mean 1.73 against the first pass's 2.04, with
`jsonlog-16m` 2.04 -> 1.68, `dickens` 2.76 -> 2.35 and `webster` 2.67 -> 2.21. Same
cause as L1: the concurrent decode work, not MatchFind.

Against mission section 7: **3 pass compress** (zeros 0.28, text 0.43, versions 0.64),
**4 pass decompress** (text 0.30, zeros 0.95, incomp 0.97, versions 1.07). `versions`
decompress, which missed by one hundredth at 1.12 on the first pass, now PASSES at
1.07.

> **ONE CELL TO WATCH.** `incomp-32m` compress reads **2.31** here against **1.14** on
> the first pass, and both arms moved (C 3837.9 -> 5714.4, us 3365.3 -> 2469.2). Its
> size cell is exactly 1.000 on all four boards, so the parse is identical and this is
> purely a timing movement on a degenerate, incompressible 32 MiB corpus read at an
> 8 MiB cap. At a 15.05% null arm this is not a claim in either direction, but it is
> the widest unexplained swing on the page and it should be re-measured before anyone
> reasons from it.

> **WHAT THIS BOARD DOES NOT SAY.** It does not say the optimization campaign made us
> faster. Its speed columns cannot carry that claim at a 15.05% null spread, and the
> campaign never made it -- every item shipped on
> strictly-less-work plus byte-identity. See
> [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md) section 4.

---

## 3. Stage anatomy — where OUR time goes

L3 is the shipping default

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |
| smallmsg-8m  |  **82.6** |      5.0 |    4.4 |     4.8 |      7.4 | **89.5** |      2.7 |
| jsonlog-16m  |  **79.4** |      5.2 |    5.7 |     5.6 |      7.4 | **88.9** |      3.4 |
| sao          |  **76.7** |     11.5 |    4.8 |     3.0 |     31.6 | **65.9** |      2.3 |
| x-ray        |  **75.9** |     12.2 |    5.1 |     3.5 |     16.6 | **79.7** |      3.6 |
| dickens      |  **75.4** |      4.3 |    8.5 |     8.3 |      3.4 | **94.7** |      1.8 |
| webster      |  **74.5** |      5.1 |    8.8 |     8.1 |      3.9 | **93.7** |      2.3 |
| samba        |  **73.5** |      7.5 |    7.6 |     6.2 |      7.1 | **88.5** |      3.9 |
| mozilla      |  **73.3** |     10.7 |    5.3 |     5.1 |     14.2 | **81.7** |      3.9 |
| reymont      |  **73.0** |      4.4 |   10.4 |     8.5 |      3.1 | **94.7** |      1.9 |
| mr           |  **72.4** |      9.0 |    9.1 |     6.4 |      5.6 | **92.2** |      1.9 |
| ooffice      |  **72.1** |     13.1 |    6.1 |     5.4 |     17.8 | **79.7** |      2.3 |
| osdb         |  **70.6** |     15.9 |    5.6 |     4.4 |     10.5 | **86.4** |      2.9 |
| xml          |  **68.8** |      7.8 |    9.5 |     7.3 |      7.2 | **88.2** |      4.3 |
| nci          |  **66.2** |      6.8 |   11.0 |     9.1 |      6.5 | **88.3** |      5.0 |
| text-32m     |  **63.0** |      3.6 |    0.8 |     0.3 |      0.4 | **73.8** |     24.1 |
| versions-16m |  **57.4** |      0.8 |    2.7 |     3.8 |      0.2 | **76.7** |     22.3 |
| incomp-32m   |   **7.6** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **22.2** |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **25.9** |

**ENCODE leader: MatchFind 18/18, Huff 0/18. DECODE leader: DecSeq 16/18, DecCk 2/18,
DecLits 0/18.**

**The same board at L1** -- new to this page. L3 is the shipping default, but L1 is the
level the fast-path work serves, and its stage split is NOT the same shape:

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |
| smallmsg-8m  |  **81.8** |      8.7 |    3.9 |     3.1 |      9.2 | **87.4** |      3.3 |
| dickens      |  **81.5** |      7.0 |    4.6 |     4.5 |     10.1 | **87.0** |      2.8 |
| jsonlog-16m  |  **80.3** |      6.9 |    5.0 |     4.6 |      8.6 | **87.4** |      3.8 |
| webster      |  **78.5** |      7.8 |    5.3 |     5.5 |      8.9 | **87.7** |      3.2 |
| sao          |  **77.2** |     17.7 |    1.2 |     1.1 | **62.2** |     34.1 |      3.6 |
| reymont      |  **76.8** |      5.8 |    6.9 |     7.7 |      4.9 | **92.8** |      2.2 |
| mr           |  **74.4** |     16.4 |    3.6 |     3.1 |     18.1 | **77.7** |      4.0 |
| samba        |  **74.1** |      9.5 |    6.2 |     5.7 |     10.6 | **82.8** |      6.1 |
| xml          |  **73.5** |      8.0 |    7.2 |     6.9 |      8.2 | **86.3** |      5.0 |
| osdb         |  **72.6** |     18.0 |    3.9 |     3.2 |     13.3 | **82.6** |      3.9 |
| nci          |  **71.0** |      6.1 |    9.6 |     8.1 |      7.0 | **88.2** |      4.6 |
| ooffice      |  **70.8** |     20.6 |    3.2 |     2.7 |     28.9 | **67.2** |      3.7 |
| mozilla      |  **69.0** |     15.4 |    5.0 |     4.6 |     18.5 | **75.6** |      5.6 |
| text-32m     |  **68.7** |      2.7 |    0.4 |     0.2 |      1.1 | **73.0** |     24.6 |
| versions-16m |  **62.6** |      0.8 |    2.7 |     3.7 |      0.2 | **75.0** |     24.1 |
| incomp-32m   |   **8.5** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **21.4** |
| x-ray        |       6.2 | **86.7** |    0.0 |     0.0 | **75.9** |     17.8 |      5.5 |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **24.8** |

**ENCODE leader: MatchFind 17/18, Huff 1/18. DECODE leader: DecSeq 14/18, DecLits 2/18,
DecCk 2/18.**

Three differences from L3 that matter. (1) **`x-ray` inverts completely**: MatchFind
6.2%, Huff **86.9%**, and on the decode side DecLits **77.5%** against DecSeq 15.5%.
At L1 this corpus finds almost nothing and the whole frame becomes a literals problem
-- it is the ONE corpus on either board where MatchFind is not the encode leader, and
the only reason the L1 encode tally reads 17/18 rather than 18/18. (2) **Huff matters
much more at L1 generally** -- `ooffice` 20.1, `sao` 18.9, `osdb` 17.4, `mr` 16.0,
`mozilla` 15.0, against L3 values of 12.9, 12.5, 15.8, 8.9, 10.7. Fewer matches means
more literals to encode. (3) **The entropy coders shrink**: FseSeq+SeqCode is 1.1+1.1
on `sao` at L1 against 5.0+3.2 at L3. Any future literals work should be scoped against
this board, not the L3 one.

### MatchFind Function Anatomy

The largest stage (#1 on **18 of 18** corpora -- see section 3) and by far the most
dispatched.

**Function-level breakdown, 2026-08-21 (post twin campaign).** Two instruments, read
together: the profile clock allocates TIME per level (shares comparable only within
the run, `mfanat.rs`), and the asm census gives each function's footprint and ISA
state.

**(a) Time -- which function owns MatchFind at each level.** 18-corpus 8 MiB board,
profile build, re-measured 2026-08-22 third pass; milliseconds per input MiB. Absolute
times from an INSTRUMENTED build, so they are not comparable to the release boards in
sections 1 and 2 -- compare shapes and inversions across levels, and cells across
passes only with the null arm in hand.

| level | strategy | serving function(s)          | MF ms/MiB | MF % of encode |
| ----: | -------- | ---------------------------- | --------: | -------------: |
|    L1 | Fast     | `find_fast_impl`             |      5.17 |           73.9 |
|    L3 | DFast    | `find_dfast_impl`            |      5.40 |           74.1 |
|    L5 | Greedy   | `find_greedy` (walk inlined) |      9.19 |           83.0 |
|    L7 | Lazy     | `find_lazy` + `chain_find_best` |   20.59 |           91.6 |
|    L9 | Lazy2    | `find_lazy` + `chain_find_best` |   32.12 |           94.3 |
|   L12 | Lazy2    | `find_lazy` + `chain_find_best` |  134.11 |           98.3 |
|   L13 | BtLazy2  | `find_bt_lazy` + `bt_find_best` | 364.32 |           99.2 |
|   L15 | BtLazy2  | `find_bt_lazy` + `bt_find_best` | 301.26 |           99.2 |
|   L16 | BtOpt    | `find_opt` + `bt_find_best`  |    281.06 |           99.3 |
|   L19 | BtUltra2 | `find_opt` + `bt_find_best`  |    335.19 |           99.3 |
|   L22 | BtUltra2 | `find_opt` + `bt_find_best`  |    340.64 |           99.2 |

Four readings. (1) **MatchFind IS the encoder at every level**: 73.9% at L1 rising to
99.2-99.3% from L13 up -- above the chain ladder there is no second stage worth a row.
(2) The ladder spans **70x** in absolute cost, 5.17 -> 364.32 ms/MiB; the big jumps are
Greedy -> Lazy (9.19 -> 20.59, the look-ahead re-search at every found match), L9 ->
L12 inside Lazy2 itself (32.12 -> 134.11, pure `search_log` depth) and Lazy2 ->
BtLazy2 (134.11 -> 364.32, the tree walk). (3) **L16 still costs LESS than L15**
(281.06 vs 301.26) -- third pass running, and the mechanism is unchanged: the DP prices
candidates it then declines to re-search, where BtLazy2's look-ahead searches
unconditionally.

**A RETRACTION.** The second pass reported a SECOND inversion at the top of the
ladder, **L22 below L19** (387.34 vs 393.63), and called two independent inversions
"the strongest form this observation has taken". **It did not reproduce.** This pass
reads L22 **340.64** against L19 **335.19** -- the ordinary way round, on a host with
a far lower null arm than the board that produced the claim. One board is not evidence
for a 1.6% gap. The L16 < L15 inversion, which is a 7% gap and has now held on three
consecutive boards, stands; the L22 one is withdrawn.

**A SECOND CELL TO WATCH: L13.** It reads **364.32**, above BOTH L15 (301.26) and L16
(281.06), which is out of line with the ladder's shape -- and it has risen on every
pass (273.0 -> 290.54 -> 364.32) while every neighbouring level fell. Either L13 is
carrying a real regression that the identity boards cannot see (they check bytes, and
L13's bytes are exact at 12,603,507), or this instrument is noisy at that level. It
needs a repeat measurement before anything is concluded from it.

(4) **The chain ladder has now fallen on three consecutive passes** -- L7
44.1 -> 24.04 -> 20.59, L9 74.8 -> 40.93 -> 32.12, L12 174.3 -> 171.04 -> 134.11 --
tracking the per-finder campaign (`ChainCtx`, the `find_lazy` prologue hoist) and then
this one. **It is still not claimed.** These are absolute times from a profile build,
and the campaign's law is that no win ships on the clock; a monotone trend across three
boards is suggestive, not a receipt. It is recorded as the one place where the clock
and the static receipts point the same way.

**(b) The functions -- footprint and ISA state** (asm census, same build; "copies"
counts monomorphisations, plain + BMI2 twin).

**Re-censused 2026-08-22.** "copies" is baseline + BMI2 twin. The twin column is what
EXECUTES on modern hardware; the baseline column is linked but never entered.

| function | copies | total instrs | ISA receipt | unit of execution |
| --- | ---: | ---: | --- | --- |
| `find_fast_impl` | 48 + **8** | 73,499 / **12,679** | `HLOG`/`STEP`/`PIPE` axes collapsed on the twins (140 -> 8); baseline keeps brick 54's HLOG fold | per position, L1-L2 |
| `find_dfast` + `find_dfast_impl` | 1 + **1** | dispatcher 12,726; twin **1,816** | HLOG axis collapsed on the twin (6 -> 1); the 5 baseline copies inline into the dispatcher | per position, L3-L4 (default) |
| `find_greedy` | 1 + 1 | 2,514 / 2,506 | 27 shrx, 0 CL in twin | per position, L5 |
| `find_lazy` | 1 + 1 | 1,960 / 1,940 | 13 shrx, 0 CL in twin | per position, L6-L12 |
| `chain_find_best` | 2 + 2 | 870 / 857 | outlined by brick 48, ISA via per-block `ChainFn` pointer | per position + per look-ahead step, L5-L12 |
| `bt_find_best_impl` | 20 + **0** | 6,003 / **0** | the (hash_log, chain_log) spec list is BMI2-redundant; twins run the runtime arm | per position x ~30M, L13-L22 |
| `bt_find_best_runtime` | 1 + 1 | 276 / 291 | 5 shrx; now the ONLY bt arm on modern hardware | per position, L13-L22 |
| `find_sequences_strategy` | 1 + 1 | 2,554 / 2,470 | `find_opt` + `find_bt_lazy` inlined into BOTH arms | per block hub |
| `find_sequences` | 1 + 1 | 711 / 672 | ldm glue | per block |
| `emit_fast_seq` | 1 + 1 | 256 / **252** | outlined behind `FastEmitCtx` with its own `#[target_feature]` twin -- was inlined into all 280 `find_fast_impl` copies | per match, L1-L2 |
| `count_match` / `count_eq_len` | 1 / 1 | 189 / 92 (avx2) | AVX2 arm via `has_avx2()` | per candidate hit |
| `match_ok` (+ cold tail) | inlined + 1 | 66 (tail) | memcmp only in the cold mls>8 tail | per candidate |

**The census totals.** MatchFind **572,667 -> 130,584 (-77.2%)**; the whole library
**698,085 -> 250,336**; surviving bounds checks **326 -> 42**, and ZERO of them in
`find_fast_impl`, which carried 280. The BMI2 path that actually executes went
**282,373 -> 22,268 (-92.1%)**.

**Why the axes were redundant.** Brick 54 specialises `HLOG` so the hash shift folds to
an immediate -- a real win on a baseline x86 shift, whose count MUST live in `%cl`. It
buys nothing on a BMI2 twin, because `shrx` takes its count from any GPR, which is
exactly what this page's own twin receipt recorded ("1,878 shrx, 0 CL"). The twins were
paying a six-fold monomorphisation for a fold their ISA had already made free. `STEP`
is an addend (register and immediate cost the same, on every architecture), and `PIPE`
gated ONE per-block test. All three are byte-identical by the argument this file has
always used for the dispatch: the const takes the value the runtime variable held.

Every function above runs its BMI2 twin on modern hardware; the remaining CL shifts
in the binary are the plain fallback arms, two ISA-irreducible shapes (8-bit
constant shifts, memory-destination shifts), and cold paths -- the classification is
total (commits `08d14e3`..`bfd0cf0`).

**The gate inventory** (08-20 audit; line numbers predate the twin refactor -- the
gates themselves are unchanged):

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

### DecSeq Function Anatomy

| level | strategy | DecSeq ms/MiB | DecSeq % of decode | Header | Tables | Loop | Tail | decode ms/MiB |
| ----: | -------- | ------------: | -----------------: | -----: | -----: | ---: | ---: | ------------: |
|    L1 | Fast     |          1.15 |               73.7 |    0.0 |    1.3 | **97.2** |  1.3 |          1.56 |
|    L3 | DFast    |          1.78 |               85.7 |    0.0 |    1.0 | **98.8** |  0.0 |          2.08 |
|    L5 | Greedy   |          2.03 |               87.3 |    0.0 |    0.9 | **98.9** |  0.0 |          2.32 |
|    L7 | Lazy     |          1.91 |               87.3 |    0.0 |    0.9 | **98.9** |  0.0 |          2.19 |
|    L9 | Lazy2    |          1.79 |               85.8 |    0.0 |    1.0 | **98.8** |  0.0 |          2.08 |
|   L12 | Lazy2    |          1.73 |               86.5 |    0.0 |    1.2 | **98.6** |  0.0 |          2.01 |
|   L13 | BtLazy2  |          1.72 |               87.4 |    0.0 |    1.0 | **98.8** |  0.0 |          1.97 |
|   L15 | BtLazy2  |          2.24 |               87.3 |    0.0 |    1.1 | **98.7** |  0.0 |          2.57 |
|   L16 | BtOpt    |          1.61 |               85.7 |    0.0 |    1.2 | **98.5** |  0.0 |          1.88 |
|   L19 | BtUltra2 |          1.64 |               87.4 |    0.0 |    1.1 | **98.6** |  0.0 |          1.88 |
|   L22 | BtUltra2 |          1.73 |               87.2 |    0.0 |    1.1 | **98.7** |  0.0 |          1.99 |

**(b) Time by corpus, at L1 and at the shipping default.** Sorted by DecSeq share of
decode. `ns/seq` is `DecSeqLoop / nseq`.

| corpus | L1 ms/MiB | L1 % dec | L1 ns/seq | L1 seqs/MiB | L3 ms/MiB | L3 % dec | L3 ns/seq | L3 seqs/MiB |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `reymont` | 2.71 | 92.7 | 30.5 | 88,179 | 3.10 | 94.3 | 31.8 | 96,648 |
| `dickens` | 2.20 | 86.6 | 30.0 | 72,473 | 3.95 | 94.2 | 33.8 | 115,804 |
| `webster` | 2.03 | 88.5 | 31.4 | 64,240 | 3.18 | 93.8 | 34.5 | 91,412 |
| `mr` | 1.49 | 78.2 | 32.8 | 44,447 | 3.31 | 91.5 | 32.8 | 99,596 |
| `smallmsg-8m` | 1.81 | 88.2 | 31.6 | 56,874 | 2.09 | 89.6 | 31.3 | 66,456 |
| `jsonlog-16m` | 1.72 | 87.0 | 32.3 | 52,440 | 1.80 | 89.7 | 31.8 | 56,002 |
| `nci` | 1.14 | 87.9 | 31.9 | 34,988 | 1.17 | 88.3 | 32.9 | 34,793 |
| `samba` | 1.36 | 84.4 | 32.4 | 41,034 | 1.59 | 88.2 | 33.5 | 46,464 |
| `xml` | 1.14 | 86.3 | 31.5 | 35,608 | 1.41 | 86.6 | 36.3 | 38,144 |
| `osdb` | 1.45 | 83.2 | 30.6 | 46,947 | 1.86 | 86.5 | 32.7 | 56,425 |
| `mozilla` | 1.01 | 76.4 | 33.1 | 29,198 | 1.32 | 82.2 | 33.9 | 37,580 |
| `x-ray` | 0.18 | **16.9** | -- | **7** | 3.20 | 81.3 | 32.9 | 96,379 |
| `ooffice` | 1.27 | 67.6 | 34.2 | 36,429 | 2.41 | 80.5 | 30.5 | 78,160 |
| `versions-16m` | 0.20 | 76.3 | -- | 1,198 | 0.20 | 75.0 | -- | 1,487 |
| `text-32m` | 0.20 | 74.4 | -- | **8** | 0.18 | 70.1 | -- | **8** |
| `sao` | 0.71 | 33.7 | 39.7 | 17,230 | 2.03 | 66.9 | 33.8 | 58,543 |
| `zeros-32m` | 0.00 | 0.0 | -- | 0 | 0.00 | 0.0 | -- | 0 |
| `incomp-32m` | 0.00 | 0.0 | -- | 0 | 0.00 | 0.0 | -- | 0 |
| **board** | **1.13** | **76.0** | **32.4** | **33,939** | **1.80** | **85.7** | **33.3** | **53,509** |

`ns/seq` is suppressed where the corpus emits too few sequences for the number to mean
anything -- `text-32m` (8 sequences on the whole 8 MiB board), `versions-16m` (1,198/MiB)
and `x-ray` at L1 (7/MiB). Those rows are not slow per sequence; they have no
sequences. **`x-ray` at L1 is the extreme: Loop 5.8%, Tail 92.5%** -- the block is
literals with essentially nothing to execute, which is the same L1 raw-literal
behaviour section 1 records as `x-ray` ratio 1.000. At L3 the same corpus is an
ordinary row (96,379 seqs/MiB, Loop 99.0%).

**THE STAGE IS ONE NUMBER TIMES ONE COUNT, and the counts prove it without a clock.**
`ns/seq` is **30.0-39.7 ns at L1 and 30.5-36.3 ns at L3** -- across sixteen corpora,
two levels, and content from English text to a database dump, the per-sequence cost
moves by under 25%. Multiply it out:

| level | seqs/MiB (exact count) | x ns/seq | predicted ms/MiB | MEASURED ms/MiB |
| ---: | ---: | ---: | ---: | ---: |
| L1 | 33,939 | 32.4 | **1.10** | 1.13 |
| L3 | 53,509 | 33.3 | **1.78** | 1.80 |

Both close to within 3%, and the L1 -> L3 sequence count rises **1.58x** against a
measured DecSeq rise of **1.59x**. So DecSeq has exactly two levers, and the board
says which is which: **cut the ~33 ns, or emit fewer sequences.** The second is not a
decoder question at all -- it is the encoder's `(litlen, matchlen, offset)`
distribution, which is what gate #3-#6 below already hinted at and this now
quantifies.

**(c) The functions -- footprint and ISA state.** Asm census of the SHIPPING build
(`RUSTFLAGS="--emit asm"`), **re-taken 2026-08-26** after the copy_match campaign
(`inline-execution.md` sections 17-18 and 23-27) and the concurrent
`decode_seq_header` extraction. The 2026-08-21 table it replaces is kept below the
fold because its central finding has been RESOLVED, and a resolved trap is worth more
on the record than a deleted one.

| function | instrs | BMI2 | ymm | xmm | CL-shift | callq | unit of execution |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | --- |
| `decode_sequences` | **954** | 0 | 0 | 8 | **22** | 25 | per block, plain arm |
| `decode_sequences_avx2` | **862** | **22** | 0 | 8 | **0** | 23 | per block, twin |
| `decode_seq_header` | 933 | 0 | 0 | 12 | 0 | 29 | **per block, outlined (new)** |
| `decode_compressed_block` | 223 | 0 | 0 | 2 | 0 | 8 | per block, plain arm |
| `decode_compressed_block_bmi2` / `_avx2` | 147 / 147 | 0 | 0 | 2 | 0 | 4 | per block, twins |
| `decompress_into_history` | 570 | 0 | 0 | 9 | 0 | 17 | per frame |
| `match_cold_raw` | 164 | 0 | 0 | **70** | 0 | **1** | per COLD copy (~19% of copies) |
| `lit_cold_raw` | 32 | 0 | 0 | 10 | 0 | **0** | per cold literal copy |
| `copy_from_decoded_cold` x3 monomorphs | 226 / 217 / 182 | 0 | 0 | 30 / 10 / 0 | 0 | 10-12 | Vec-needing leftovers (0.24%) |
| `copy_match_dict_cold` | 179 | 0 | 0 | 4 | 0 | 8 | dictionary path only |
| `copy_literals_cold` | 96 | 0 | 0 | 10 | 0 | 3 | literal tail |
| `copy_match`, `copy_from_decoded`, `copy_literals`, `seq_table`, `init_state`, `resolve_offset`, `FseTable::entry`/`advance`, `BitRev::read_bits`/`reload` | -- | -- | -- | -- | -- | -- | **fully inlined into the twins** |

**THE 2026-08-21 SHIM TRAP IS CLOSED.** That census found `copy_match` and
`copy_from_decoded` as two nested CALLS per sequence, outside the twin, carrying
0 ymm -- "the exact shim trap the twin campaign was built to avoid, still open on the
single largest object in the decoder." Neither symbol exists today: both are inlined
into all three block twins, and the per-sequence execute path reaches its copies with
no call at all. What replaced them:

| then (2026-08-21) | now (2026-08-26) |
| --- | --- |
| `copy_match` -> `copy_from_decoded`, 2 calls/sequence | inlined; the ~80% joint fast path is **one branch + two `movups` pairs**, zero calls |
| cold rungs behind a `&mut Vec` boundary | `match_cold_raw` / `lit_cold_raw`: `op` in, `op` out, **1 and 0 `callq`** |
| `decode_sequences` 4,167 instrs | **954** (-77%); `_avx2` 4,055 -> **862** (-79%) |
| `decode_compressed_block` 2,942 | **223**, with the header/table parse outlined to `decode_seq_header` |

**The twin receipt is still real, and it is now clean.** The plain arm carries **22
CL-shifts and 0 BMI2**; the AVX2 twin carries **0 CL and 22 BMI2** -- a complete
swap, where the 2026-08-21 build still left 68 CL inside the twin. Every variable
shift in the sequence loop is now either a BMI2 `shrx`/`shlx` on the twin or an
`OF_PACK` table load on both (section 26, win 9).

> **RE-TAKEN 2026-08-27, and the page moved under it.** Crate total
> **146,955 -> 79,883 instructions (-46%)** in one day, almost all of it a
> CONCURRENT workstream's outlining, and the decode structure changed shape:
> `decode_sequences` no longer exists as its own symbol (folded into
> `decode_compressed_block`, now 1,031), and `decode_4x_x2_slow` has been
> split out. Current decode-path census:
>
> | symbol | instrs | BMI2 | ymm | xmm | CL |
> |---|---:|---:|---:|---:|---:|
> | `decode_compressed_block` | 1,031 | 0 | 0 | 10 | 22 |
> | `decode_sequences_avx2` | 875 | **22** | 0 | 8 | **0** |
> | `decode_4x` | 790 | 0 | 0 | 0 | 32 |
> | `decode_4x_bmi2` | 702 | **40** | 0 | 0 | **0** |
> | `decompress_into_history` | 570 | 0 | 0 | 9 | 0 |
> | `decode_seq_header` | 324 | 0 | 0 | 12 | 0 |
> | `row_find_best` | 330 | 0 | 0 | 7 | 3 |
> | `match_cold_raw` | 164 | 0 | 0 | **70** | 0 |
> | `lit_cold_raw` | 32 | 0 | 0 | 10 | 0 |
>
> **The two decode twins are healthy** -- each swaps its CL-shifts for BMI2
> cleanly (22/0 and 40/0). Hold that shape in mind for the ENCODE-side twin
> audited in section 3(h), which does not have it.
>
> **A file being edited by another session cannot be boarded.** `huffman.rs`
> was written at 05:36 while this census ran. Every number above is a
> deterministic count from one build; treat them as a snapshot of a moving
> tree, not a standing board.

**What the census says is NOT there, and why that is correct.** No symbol on the
decode path carries a single `ymm`. That is not a regression and not an oversight:
the fast-path copies are 16-byte `movups` by measurement (`bandcensus`: 80.40% of
match copies and 99.22% of literal copies are `len <= 16`, mean 7.4 bytes), and the
wide rungs live in `match_cold_raw`, which is `#[cold] #[inline(never)]` and therefore
compiles at baseline ISA by construction -- the deliberate trade recorded in
`inline-execution.md` section 27. T4's AVX2 audit already measured a
`#[target_feature]` copy helper here as a NET LOSS (call overhead vs 4 inline SSE
instructions). The 70 `xmm` in `match_cold_raw` are the strided wildcopy rungs doing
their work.

**(d) The loop interior, by COUNT.** Exact, not sampled -- the loop's shape is fixed by
the RFC.

| function                        | unit         | per sequence | calls/MiB @ L1 | calls/MiB @ L3 |
| ------------------------------- | ------------ | -----------: | -------------: | -------------: |
| `FseTable::entry`               | per seq x3   |         3.00 |        101,818 |        160,527 |
| `BitRev::read_bits`             | per seq x3   |         3.00 |        101,818 |        160,527 |
| `FseTable::advance`             | per seq x3   |         3.00 |        101,798 |        160,503 |
| `BitRev::reload`                | per seq x2   |         2.00 |         67,879 |        107,018 |
| `copy_literals`                 | per seq      |         1.00 |         33,939 |         53,509 |
| `resolve_offset`                | per seq      |         1.00 |         33,939 |         53,509 |
| `copy_match`                    | per seq      |         1.00 |         33,939 |         53,509 |
| `seq_table` (LL/OF/ML)          | per block x3 |           -- |             20 |             24 |
| `BitRev::new` + `init_state` x3 | per block x4 |           -- |             27 |             32 |

**The per-block functions are four orders of magnitude rarer than the per-sequence
ones** -- 24 `seq_table` calls per MiB against 160,527 `FseTable::entry`. That is the
count-side statement of the 1% Tables share, and it is why no table-build brick can
pay. **Eleven primitive calls happen per sequence**, and the three FSE `entry` +
three `advance` + three `read_bits` dominate that count nine to three over the copies.

> **STILL EXACT 2026-08-26, and the reason is worth stating.** These counts are fixed
> by RFC 8878 -- one literal copy, one match copy, three symbol decodes and three
> state advances per sequence, whatever the implementation does -- so the campaign
> that rewrote this loop five times could not move a single cell of this table, and
> `bandcensus` re-reads 6,215,835 match copies before and after every round to prove
> exactly that. **What changed is the unit, not the count:** every row except
> `seq_table` is now an INLINED operation rather than a `callq`, and the ~80% joint
> fast path executes its `copy_literals` + `copy_match` pair with no call, no
> capacity test and one branch (section 26). Read this table as the loop's SHAPE; read
> (c) for what each row costs to reach.

**(e) Breaking the LOOP open -- where the 98.8% goes.** `dsloop.rs`, `--features
dupladder`, L3, 13 sequence-bearing corpora, 6.5M sequences.

Sections (a)-(d) leave the whole stage inside one bar: the Loop is 97-99% of DecSeq
and nothing inside it can be timed, because one sequence costs ~34-40 ns and an
`Instant` pair costs 74.8. **The method that does work is DUPLICATION.** Each op is
executed K extra times per sequence and then UNDONE -- bit-reader state restored,
`out` truncated, `reps` restored -- so:

- **every arm produces byte-identical output**, asserted on every arm and every
  corpus, so a mis-built arm cannot quietly become a fast arm;
- the arm's delta over baseline prices K executions of that op, over 6.5M sequences
  instead of one.

> **THE INSTRUMENT PERTURBS THE LOOP IT MEASURES, AND HERE IS THE RECEIPT.** The arm
> dispatch is a per-sequence branch inside a register-starved loop. Built from
> identical source, `dsprobe` reports **40.26 ns/seq under `--features profile` and
> 53.07 ns/seq under `--features dupladder` with every arm OFF** -- a **+32%**
> baseline inflation, higher on 12 of 13 corpora. That is why `dupladder` is its own
> feature and NOT part of `profile`: it must be built on purpose, and its absolute
> numbers must never be quoted beside a `profile` board. Sections (a)/(b) above are
> `profile` builds and are unaffected.

**The validation that decides what this table may claim: is the answer invariant to
K?** Run the whole ladder at K=4 and again at K=8 -- two runs whose own baselines
differed by 38% (73.89 vs 53.46 ns/seq):

| op                     | ns/exec @K=4 | ns/exec @K=8 | **% of loop @K=4** | **% of loop @K=8** |
| ---------------------- | -----------: | -----------: | -----------------: | -----------------: |
| `copy_match`           |        30.53 |        22.94 |           **41.3** |           **42.9** |
| `copy_literals`        |         9.89 |         7.45 |           **13.4** |           **13.9** |
| `resolve_offset`       |         3.03 |         1.53 |                4.1 |                2.9 |
| `FseTable::entry` x3   |         0.37 |         0.29 |                1.5 |                1.6 |
| `BitRev::reload` x2    |         0.83 |         0.34 |                2.3 |                1.3 |
| `FseTable::advance` x3 |         0.24 |         0.13 |                1.0 |                0.7 |
| `BitRev::read_bits` x3 |         0.33 |         0.13 |                1.3 |                0.7 |
| **ladder total**       |    **47.95** |    **34.24** |           **64.9** |           **64.0** |

**Absolute ns is NOT K-invariant; the SHARES are.** Every op is 25-50% cheaper at K=8
-- more repetitions, warmer cache, cheaper marginal execution -- exactly the bias the
method carries. But `copy_match` moves 41.3 -> 42.9%, `copy_literals` 13.4 -> 13.9%,
and total coverage 64.9 -> 64.0%, across runs whose baselines differed by 38%. **So
this instrument reports proportions, and its absolute nanoseconds are lower bounds
only.** Every number below is a share for that reason.

**THE FINDING: DecSeq IS A MEMORY-MOVEMENT LOOP, NOT AN ENTROPY-DECODING LOOP.**

| what                                                             | share of the loop |
| ---------------------------------------------------------------- | ----------------: |
| `copy_match`                                                     |          **~42%** |
| `copy_literals`                                                  |            ~13.7% |
| `resolve_offset`                                                 |             ~3.5% |
| **the three execution ops together**                             |          **~59%** |
| ALL FOUR entropy primitives -- **eleven calls per sequence**     |         **~4-6%** |
| unattributed (loop branch/counter/bounds + warm-cache shortfall) |              ~36% |

**RE-RUN 2026-08-26, after the five copy_match rounds** (`inline-execution.md`
17-18, 23-27). Same instrument, same 13 corpora, `--features dupladder`:

| op | ns/exec | ns/seq | **% of loop** | share then (K=8) |
| --- | ---: | ---: | ---: | ---: |
| `copy_match` | 35.73 | 35.73 | **52.5** | 42.9 |
| `copy_literals` | 9.96 | 9.96 | **14.6** | 13.9 |
| `FseTable::advance` x3 | 2.65 | 7.95 | 11.7 | 0.7 |
| `BitRev::reload` x2 | 2.39 | 4.78 | 7.0 | 1.3 |
| `BitRev::read_bits` x3 | 1.44 | 4.33 | 6.4 | 0.7 |
| `resolve_offset` | 2.02 | 2.02 | 3.0 | 2.9 |
| `FseTable::entry` x3 | 0.52 | 1.57 | 2.3 | 1.6 |
| **ladder total** | | **66.33** | **97.4** | 64.0 |

**The two copies are now 67.1% of the loop and coverage rose 64.0% -> 97.4%** -- the
"unattributed ~36%" this section recorded (loop branch, counter, bounds, cursor
arithmetic) is largely GONE, which is precisely what the five rounds deleted: the
per-sequence capacity tests, the Vec-field traffic, the double `set_len`, and four of
the five fast-path branches. An instrument that once could not account for a third of
the loop now accounts for all but 2.6% of it.

> **DO NOT read the ns column against the older one, and do not trust the small
> shares to two digits.** The board baseline reads **68.08 ns/seq** here against
> 35.42 on 08-24 and 45.83 earlier on 08-26 -- the SAME source, three numbers, on a
> host carrying a concurrent workstream all session. That is section 15.2's law
> (un-interleaved runs on different days are not a comparison) applying to this
> instrument, and it is why the entropy rows appear to have grown 10x in share when
> the campaign never touched them: under load every op inflates, and the ladder's
> `dup` arms inflate unevenly. **What survives is the ORDER and the coverage jump**,
> both of which are structural and both of which agree with the deterministic counts
> in (c). The 08-24 run put `copy_match` at 60.8%, this one at 52.5%; the honest
> reading is "roughly half to three-fifths of the loop, still the single largest
> object by a factor of three." A quiet box is required before any finer claim.

That is the opposite of what the stage's name suggests. The FSE and bit-reader
machinery -- eleven primitive calls per sequence, 321,054 calls per MiB at L3, the
part that the BMI2 twin exists to accelerate -- is **under six percent of the loop**.
One `copy_match` per sequence is **seven times** the cost of all eleven of them
together.

Three consequences:

1. **The BMI2 twin cannot be worth much on the decode side, and now that is
   quantified rather than assumed.** It converts 172 CL-shifts to `shrx` in
   `decode_sequences` (section (c)) -- inside the ~5% of the loop that is entropy. The
   ceiling on the entire twin is a few percent of DecSeq, i.e. a few percent of ~86%
   of decode. The twin is byte-exact and free to keep; it is not a lever to pull
   harder on.
2. **`copy_match` at ~42% is the single largest object in the decoder**, and the route
   census below says its traffic is split between an enormous number of tiny copies
   (16B tier: 87% of calls, mean 8.0 bytes) and a small number of long ones
   (`extend_from_within`: 3.7% of calls, **33.9% of bytes**, mean 124.8). Any real
   decode win goes through that function.
3. **The encoder-side lever is the stronger one, and section (b) already priced it.**
   DecSeq = sequences x ~33-40 ns, and ~59% of that constant is copying whose cost is
   set by the `(litlen, matchlen, offset)` distribution the ENCODER chose. Emitting
   fewer, longer matches moves both terms at once; making `copy_match` faster moves
   42% of one of them.

**`copy_match` route census** -- which band each match copy takes:

| band | L1 calls | L1 % calls | L1 % bytes | L1 mean len | L3 calls | L3 % calls | L3 % bytes | L3 mean len |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| 16B tier | 3,720,978 | **80.5** | 42.5 | 9.5 | 6,351,371 | **87.1** | 50.9 | 8.0 |
| 32B tier (len>16) | 640,537 | 13.9 | 16.5 | 21.4 | 651,665 | 8.9 | 14.2 | 21.7 |
| `extend_from_within` | 246,087 | 5.3 | **29.2** | 98.1 | 270,359 | 3.7 | **33.9** | 124.8 |
| overlapping chunked | 12,526 | 0.3 | **11.6** | **766.9** | 13,953 | 0.2 | 1.0 | 71.9 |
| `offset==1` splat | 2,436 | 0.1 | 0.2 | 54.4 | 652 | 0.0 | 0.0 | 53.5 |
| 32B tier (len<=16) | **0** | **0.0** | 0.0 | -- | **0** | **0.0** | 0.0 | -- |

`copy_literals` tiers: **96.9%** of calls tiered at L1 (16B 4,256,559 / 32B 223,809 /
memcpy 142,196), **99.3%** at L3 (16B 7,015,583 / 32B 220,400 / memcpy 52,017).

**Three readings.**

1. **The `32B tier (len<=16)` band reads exactly ZERO at both levels.** That band
   exists only to catch short copies leaking into the wide tier, which is what the T4
   reorder (test 16 before 32) was meant to stop. It is airtight: not one copy in
   11.9M moves 32 bytes to publish 16 or fewer. **A census that reads zero is the
   receipt that the fix holds** -- and it is the only way to know.
2. **Calls and BYTES disagree about where the work is.** The 16B tier is 80-87% of
   CALLS but 42-51% of BYTES at a mean of 8-9.5, while `extend_from_within` is
   3.7-5.3% of calls and **29-34% of bytes** at a mean of 98-125. Tuning by call count
   tunes the half of the traffic that is already cheapest.
3. **`overlapping chunked` is an L1-only concentration.** At L1 it takes 0.3% of calls
   but carries **11.6% of all match bytes at a mean run of 767 bytes**; at L3 the same
   route carries 1.0% at a mean of 72. Long overlapping runs are an L1 phenomenon and
   they land in the slowest route in the file. That is the one un-tiered band left.

**(f) SCALAR CENSUS and the first SIMD attempt -- what is reducible and what is not.**

The census (`scalarcensus.py` over the shipping asm) classifies every instruction in
the live decode path. The headline is a REFUTATION:

| function | total | WIDE | SSE | BMI2 | CL-shift | byte-op | scalar |
| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |
| `decode_sequences_avx2` | 4,055 | 42 | **0** | 175 | 68 | 394 | 3,376 |
| `decode_sequences` (plain) | 4,167 | 0 | 57 | 0 | 240 | 410 | 3,460 |
| `decode_compressed_block_bmi2` | 2,792 | 0 | **57** | 78 | 112 | 196 | 2,349 |

1. **The CL-shift residue is IRREDUCIBLE, and counting settled it before any code was
   written.** Splitting the 68 shifts in `decode_sequences_avx2` by operand: **65 are
   IMMEDIATE** (`shlq $2`, `shrl $3`) and only **3 are variable**. BMI2's `shrx`/`shlx`
   exist to remove the CL-register constraint and flag dependency of a VARIABLE shift;
   against an immediate shift they are not faster. Same in the block twin: 110
   immediate, 2 variable. **The twin campaign already converted everything
   convertible** -- "replace the remaining shifts with BMI2" is a dead lever.
2. ~~**The LITERAL copy is already 256-bit; the MATCH copy is not.**~~ **SUPERSEDED
   2026-08-26 -- the gap closed, and the answer was not widening.** `copy_match` and
   `copy_from_decoded` are no longer outlined at all: both inline into every twin, and
   the ~80% joint fast path emits two 16-byte `movups` pairs under ONE branch (see (c)
   and `inline-execution.md` sections 23-27). The width question resolved AGAINST ymm
   on measurement -- `bandcensus` puts 80.40% of match copies and 99.22% of literal
   copies at `len <= 16` (mean 7.4 bytes), so a 32-byte move would publish 7.4 bytes
   at double the cost, which is exactly the T4 ordering law this document already
   records. The wide rungs live in `match_cold_raw`, cold and baseline by construction.
3. **The 394 byte-ops are mostly `movzbl` (190)** -- single-byte FSE symbol and
   length-code table reads. One-element table lookups are the gather case; vectorising
   them costs more than it saves.
4. **The one real SSE gap: `decode_compressed_block_bmi2` is `target_feature(bmi2,
   lzcnt)` with NO avx2**, so its 57 vector instructions are legacy SSE (27 `movdqa`,
   9 `movaps`, 7 `movdqu`, plus a `pshufd`/`punpcklbw` group). `decode_sequences_avx2`
   proves the mechanism -- it is `avx2,bmi2,lzcnt` and carries 0 SSE. Widening that
   twin is the outstanding item; it lands in DecLits, not DecSeq.

**The 64-byte tier: built, correct, NOT faster.** Section (e) put `copy_match` at ~42%
of the loop, and the route census left exactly one un-tiered band
(`extend_from_within`, ~34% of match bytes, falling through to a runtime-length
`memcpy` CALL). Its length histogram (`dsuntier.rs`) is not diffuse -- **65.9% of its
calls and 46.9% of its bytes are 33-64 bytes**, one 2x ymm pair. Its MEAN (67) would
have chosen the wrong width; the histogram chose it. Added as a 64-byte tier after the
16 and 32 (narrowest first, the T4 ordering law):

- **Capture, as predicted:** the 33-64 bucket fell from **170,358 calls to 980**
  (99.4% absorbed, the remainder being `offset < 64`), taking 8.05 MB out of the band.
- **Correctness:** 159/159 tests pass; the round-trip is asserted per corpus.
- **Speed: NEUTRAL.** ABBA-paired against a pre-tier binary, board DecSeqLoop
  **31.99 -> 31.52 ns/seq**, a 1.5% apparent gain inside a **6-8% within-arm spread**.
  **This instrument cannot resolve it, so no win is claimed.**

**WHY IT DID NOT PAY -- and the deterministic ledger says it without a clock.**
Diffing the emitted SHIPPING asm before and after the tier:

| symbol | instrs before | after | delta | ymm before | after |
| --- | ---: | ---: | ---: | ---: | ---: |
| `decode_sequences_avx2` | 4,055 | 4,055 | **+0** | 26 | **26** |
| `decode_sequences` | 4,167 | 4,167 | **+0** | 0 | 0 |
| `copy_from_decoded` | 211 | 230 | **+19** | **0** | **0** |

**The hot loop did not change by one instruction.** The tier landed entirely in
`copy_from_decoded` -- an outlined, baseline-compiled function (see the correction in
(c)) -- so its 64-byte move is **four SSE `movups` pairs, not two AVX2 ymm pairs**.
The change added a wider copy in the one function that cannot use wide registers. That
identical `4,055` is what sent this audit one layer down and found the call edge the
truncated dump had hidden.

The full deterministic ledger for the change, all counts, no clock:

| quantity | value |
| --- | ---: |
| WIN: runtime-length `memcpy` call sites converted to fixed-width | **169,378** |
| WIN: match bytes rerouted off the untiered band | **8,050,211** |
| WIN: 33-64 bucket captured (170,358 -> 980 calls) | **99.4%** |
| WIN: untiered band calls (258,626 -> 89,248) | **-65.5%** |
| WIN: untiered band bytes (17.24 MB -> 9.19 MB) | **-46.7%** |
| COST: store amplification, 64 B written for a 47.53 B mean | **+2.79 MB** |
| COST: static footprint of `copy_from_decoded` | **+19 instrs** |
| COST: vector width actually obtained | **128-bit, not 256** |

A large call-count win against a real store-traffic cost, executed at half the
intended width. The clock says the three cancel. **The tier is kept but unproven; the
change that would make it pay is inlining the match copy into the twin, not widening
it further.**

**(g) THE SIMD CAMPAIGN -- what shipped, what was ruled out, and the law it taught.**

Section (f) found the match copy running OUTSIDE the AVX2 twin at 128-bit through two
nested calls per sequence. Three changes were attempted against that.

**SIMD-1 -- inline the match copy into the twin. SHIPPED, the campaign's win.**
`copy_match` (141 instrs) and `copy_from_decoded` (211) carried no `#[inline]`, so LLVM
left them outlined -- and a function with no `#[target_feature]` is generated at the
crate's BASELINE ISA no matter which twin calls it. Marking both `#[inline(always)]`
lets LLVM regenerate the bodies under `avx2,bmi2` (a baseline callee's feature set is a
SUBSET of the twin's, so inlining is legal).

| deterministic quantity | before | after |
| --- | ---: | ---: |
| `copy_match` symbol | 141 instrs | **GONE (inlined)** |
| `copy_from_decoded` symbol | 211 instrs | **GONE (inlined)** |
| per-sequence CALL edges into the copy path | **2 nested** | **0** |
| call+ret pairs per MiB @L3 (53,509 seq/MiB x 2) | **107,018** | **0** |
| sequence twin, 256-bit ymm ops | 26 | **31** |
| sequence twin, legacy SSE | 0 | 0 |
| STATIC footprint of the whole path (4,055+141+211) | 4,407 | **4,227 (-180)** |

**Speed: DecSeqLoop 30.58 -> 28.00 ns/seq, -8.5%**, ABBA-paired x3, and **every AFTER
sample fell below every BEFORE sample** (27.30-29.14 vs 30.28-31.01, zero overlap).
Contrast the 64-byte tier in (f), whose 1.5% sat inside its own spread. **That is what
a real win looks like on this instrument: separated distributions, not a better mean.**

**SIMD-2 -- an `avx2,bmi2,lzcnt` arm for the block driver. KEPT FOR ISA CONTINUITY,
not for speed.** The bmi2-only twin emitted **57 legacy SSE** instructions; the new arm
emits **0 SSE and 71 ymm**, byte-identical by construction. But in-process ABBA over 14
corpora x9 measured **+0.3% DecLits / +0.5% decode** -- nothing, with the sign scattered
7 up / 7 down. Kept so the decode path is uniformly VEX-encoded with no legacy-SSE
island beside the AVX2 twin; `BLOCK_AVX2_ARM` keeps it adjudicable elsewhere without a
rebuild.

**WHY SIMD-2 MEASURED NOTHING, AND IT IS THE LAW OF THIS CAMPAIGN.** Both changes
improved the ISA by the same kind of margin. They differ only in how often the improved
code RUNS:

| change | path | executions per MiB @L3 | result |
| --- | --- | ---: | --- |
| SIMD-1 | **per SEQUENCE** | **53,509** | **-8.5%** |
| SIMD-2 | per BLOCK | 24 | 0% |

**An ISA count is necessary but not sufficient -- it has to sit where the count
multiplies.** SIMD-2's 57 instructions are STATIC instructions in a per-block function;
`movdqa` -> `vmovdqa` is the same work, the same number of times. ~2,200x difference in
execution frequency, and the speed result follows the frequency, not the ISA.

**Where wide ops genuinely LOSE, measured here rather than assumed:**

- **`vzeroupper` and AVX-SSE transitions.** Inlining the copy took the twin from **8 to
  22 `vzeroupper`**. Cheap per instance on modern cores; on Sandy/Ivy/Haswell an
  uncleared transition is ~70 cycles. SIMD-1 won anyway because 107,018 call/ret pairs
  per MiB dominated it.
- **Store overshoot, the dominant effect for zstd.** A fixed-width copy writes more than
  it publishes: the 64-byte tier writes 64 for a mean payload of **47.5** (+2.79 MB per
  board), the 16-byte tier writes 16 for a mean of **8.0**. Match lengths here are tiny,
  so **wider registers make this worse, not better** -- which is exactly why the 64-byte
  tier is neutral.
- **Cache-line splits.** An unaligned 32-byte store straddles a 64-byte line far more
  often than a 16-byte one.

**RULED OUT BY COUNTING, before any code was written** (section (f)): 65 of 68 CL-shifts
in the twin are IMMEDIATE, which BMI2 cannot improve; the literal copy was already
256-bit; the 394 byte-ops are single-element `movzbl` table reads, i.e. the gather case.

**END TO END, pre-SIMD vs final:** DecSeqLoop **35.29 -> 31.23 ns/seq, -11.5%**, ABBA x3,
again with **zero distribution overlap** (final 30.30-31.80 vs pre 34.45-36.07). All
159 tests pass at every step.

**The gate inventory:**

| # | gate | kind | selects | where |
| --- | --- | --- | --- | --- |
| 1 | `seqcheck_hoisted()` `SEQCHECK_ARM` / `RZSTD_SEQCHECK_HOIST` | ARM | hoist the per-sequence code-range check out of the loop | `compressed.rs` |
| 2 | `lut_on()` `LUT_ARM` | ARM | LL/ML baseline+nbits from a LUT vs computed | `compressed.rs` |
| 3 | `matchcopy_on() && len <= 32 && offset >= 32` | ARM x stream | 32-byte unsafe non-overlapping match copy | `copy_from_decoded` |
| 4 | `matchcopy_on() && len <= 16 && offset >= 16` | ARM x stream | 16-byte tier (tested FIRST -- T4) | `copy_from_decoded` |
| 5 | `offset == 1` | stream | byte-splat (C `ZSTD_overlapCopy8`) | `copy_from_decoded` |
| 6 | `offset < len` | stream | overlapping chunked copy | `copy_from_decoded` |
| 7 | FSE table mode per table | stream | predefined / RLE / built / repeat | `seq_table` |
| 8 | `seqloop_avx2_on()` `SEQLOOP_AVX2_ARM` + `has_avx2()` | ARM x CPU | `decode_sequences_avx2` twin vs the plain arm | `decode_sequences` |

**Four of eight gates are ARMs**, i.e. decisions already made and left switchable. The
genuine per-sequence dispatches (#3-#6) are all on `(len, offset)` -- a shape the
ENCODER chooses. The decoder's speed is therefore partly an encoder-side question: the
distribution of `(litlen, matchlen, offset)` we emit determines which copy tier fires.
**Section (b) now prices that claim: DecSeq = seqs x ~33 ns**, so the encoder-side
lever is not a nuance, it is one of only two levers that exist.

> **A methodology note, recorded because it nearly shipped a wrong number.** The first
> version of this instrument took ONE decode per corpus. Its per-level table reported
> L1 DecSeq at **3.82 ms/MiB** while its own per-corpus table, same corpora and same
> level in the same process, computed to **1.36**. The gap was entirely cold start --
> first-touch page faults on the output `Vec`, a cold allocator, cold caches -- and it
> was caught only because the two tables OVERLAPPED and disagreed. Shares survived it
> (they are ratios within one run); absolute ms/MiB did not. Warmup 2 + best-of-5
> brought (a) to 1.15 against (b)'s 1.13. **Two tables that must agree are worth more
> than one table that cannot be checked.**

---

### 3(h) THE HUFFMAN LITERAL ENCODER'S BMI2 TWIN IS A `jmp`. 2026-08-27.

Found while re-taking 3(c). The census puts two numbers side by side that
cannot both be right:

| symbol | instrs | BMI2 ops | CL-shifts |
|---|---:|---:|---:|
| `HuffCTable::encode_stream_unrolled_into` | **1,983** | 0 | **118** |
| `HuffCTable::encode_stream_unrolled_bmi2_into` | **1** | 0 | 0 |
| -- for contrast, a WORKING twin -- | | | |
| `fse::compress_using_ctable` | 1,100 | 0 | 43 |
| `fse::compress_using_ctable_bmi2` | 1,084 | **42** | 1 |

The twin's entire body:

```
encode_stream_unrolled_bmi2_into:
        jmp     encode_stream_unrolled_into
```

**The BMI2 dispatch for Huffman literal encoding has never done anything.**
The runtime `has_bmi2()` check fires, calls the twin, and the twin tail-calls
the baseline -- which runs its 118 variable `%cl` shifts (3 uops each on
Intel) exactly as if the dispatch were absent.

**The mechanism, and it is the shim trap in its purest form.** The twin is
`#[target_feature(enable = "bmi2")]` and its body is one call:

```rust
#[target_feature(enable = "bmi2")]
unsafe fn encode_stream_unrolled_bmi2_into(&self, src, buf) -> ... {
    self.encode_stream_unrolled_into(src, buf)   // plain fn, ~1,983 instrs
}
```

`encode_stream_unrolled_into` carries no `#[inline(always)]` and is far past
LLVM's inlining threshold, so it is NOT inlined into the twin. A function
without `target_feature` is generated at the crate's BASELINE ISA no matter
who calls it -- so the body exists once, at baseline, and the twin collapses
to a jump. The per-symbol loop it wraps (`encode_rev_into`) IS
`#[inline(always)]`, which is why all 118 shifts land in that one body.

**The fix is the one section 18's W1 already used** on `decode_into_x1/x2`,
and the working pattern is in this same file: `decode_4x_bmi2` calls
`decode_4x_inner`, which IS `#[inline(always)]` -- and that twin reads 702
instructions with **40 BMI2 ops and 0 CL-shifts**. The encode side needs the
same shape: an `#[inline(always)]` body with one thin `#[inline(never)]`
wrapper per ISA, so each twin instantiates its own copy.

**Why it is worth doing.** `Huff` is **86.7% of encode at L1 on `x-ray`** and
16-21% on `ooffice`/`sao`/`osdb`/`mr`, and this is its per-literal-byte emit
loop. 118 variable shifts is exactly the workload BMI2's `shrx`/`shlx` exist
to serve -- the same trade `decode_4x` and `compress_using_ctable` already
bank.

**FIXED 2026-08-27**, once `huffman.rs` had been quiet for ten minutes and
compiled clean. (It was deliberately left alone at first: editing a file
another agent is mid-write in is how the truncated `fn` earlier that day left
the crate unbuildable.)

The body became `encode_stream_unrolled_body`, `#[inline(always)]`, with one
thin `#[inline(never)]` wrapper per ISA calling it. **Both** twin pairs
needed it -- the no-buffer `encode_stream_unrolled` is itself
`inline(always)` and routed through the baseline `_into` wrapper, so it
re-created the same thunk for its own twin one level down.

| symbol | before | after |
|---|---|---|
| baseline `encode_stream_unrolled_into` | 1,983 instrs, **CL=118**, BMI2=0 | 1,897 instrs, CL=118, BMI2=0 |
| `encode_stream_unrolled_bmi2_into` | **1 instruction (`jmp`)** | **1,744 instrs, BMI2=118, CL=0** |

**A clean swap: all 118 variable shifts became 118 `shrx`/`shlx`** -- the
exact shape `decode_4x_bmi2` (40 BMI2 / 0 CL) and
`compress_using_ctable_bmi2` (42 BMI2 / 1 CL) already had. The twin is
SMALLER than the baseline it replaces (1,744 vs 1,897), because `shrx` takes
its count from any register and needs none of the `%cl` setup moves.

Crate total 79,883 -> 82,063 (+2,180): one duplicated body, the price section
10.5 budgets for an ISA twin, paid on a per-literal-byte loop.

Gate: `bytegate` GOLD `BE0071FB0CB0CED9` unmoved -- byte-identical by
construction, since both arms run the same body and differ only in ISA.

**The class, now three-for-three in this crate.** V1 (xxh64's AVX2 kernel
reachable only from a test), section 18 W1 (`decode_into_x1/x2` outlined at
baseline beneath three twins), and this. All three had correct code, a
correct runtime dispatch, and no effect -- and all three were invisible to
every correctness gate, because a thunk is byte-identical to what it wraps.
**The standing check is one line of asm per twin: if a `_bmi2`/`_avx2` symbol
is not carrying the ISA ops its baseline sibling carries in `%cl`, the twin
is not doing anything.**

### Cross-cutting: dispatches that are DEAD in the shipping build

Found while auditing, per the `rusty_curiosity` law that an unused thing is invisible
to every profiler:

> **ADDENDUM 2026-08-26 -- a THIRD category, and it is the one to check first now.**
> Beyond "dead" (nothing calls it) and "live" (runtime CPUID), the decode loop now
> carries arms that are **FOLDED**: their readers are `#[cfg(feature = "profile")]`
> twins whose shipping half returns a constant, so the arm survives for the A/B
> harness and costs the shipping build nothing -- not an atomic load, not a branch,
> not the body it guards. Seven decode arms are in this state (`seqcheck`, `litcopy`,
> `matchcopy`, `lut`, `prefetch`/D9, `pipeline`/D10, `pipe1`/D11), and the last three
> fold to `false`, which deletes two parked decode-ahead implementations from the
> shipping twins entirely. This is `inline-execution.md` section 10.1's pattern A
> applied at per-sequence frequency. **An arm census that greps for `set_*_arm` will
> now over-count what the shipping build actually executes by seven** -- read the
> `#[cfg]` on the READER, not the presence of the setter.

> **CORRECTION 2026-08-21:** the two bullets below were true when written and are now
> INVERTED by the BMI2 twin campaign (`08d14e3`..`bfd0cf0`). `has_bmi2()` is the
> central live dispatch: every bitstream engine, block driver, literal section, and
> finder (Fast x140, DFast, Greedy/Lazy, Bt x84, chain walk) selects a
> `#[target_feature(enable = "bmi2,lzcnt")]` twin at runtime, and the decoder's
> sequence loop additionally dispatches on `has_avx2()`. Kept for history:

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

5. **xxh64 -- the dispatch gap named here was REAL and is now CLOSED (D8a, shipped
   2026-08-22).** This item said xxh64 "is the only stage with NO dispatch of any
   kind." The reason turned out to be worse than missing dispatch: the AVX2 kernel
   EXISTED, tested and A/B-armed, and nothing on the shipping path called it --
   `Xxh64::update` ran the scalar stripe loop while the vector kernel sat two functions
   away, reachable only from `xxh64_seed` (a test and a bench). Wiring it moved kernel
   reach **50.00% -> 100.00% of checksummed bytes, and the DECODE side 0% -> 100%**
   (`d8acensus`, re-verified 2026-08-26: 516,624,384 hybrid bytes against 640 scalar,
   the sub-tile tails). `inline-execution.md` V1 carries the full trace; the standing
   lesson is that **a beautiful kernel benchmark proves nothing until a caller trace
   proves reachability.** Still true and unchanged: memory-bound not compute-bound
   (17.9/18.0/18.0 GB/s at 32 KiB/256 KiB/4 MiB vs **12.5 GB/s over 32 MiB**), the
   algorithm is RFC-fixed, and fusing the hash into the block loop measured ~12% WORSE
   (brick 85). **What is left is overlapping it with decode on a second thread.**

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


## 7. L1 AND L3: WHERE THE ENCODE TIME ACTUALLY GOES. 2026-08-27

`hotspot.rs` says MatchFind is **73.1%** of encode at L1 and **76.3%** at L3.
That number has been read for a long time as "match finding is the cost". It is
not, and the stage's own counters refute it.

### 7.1 The stage name is wrong about what it measures

`Stage::EncodeMatchFind` wraps the whole `find_sequences` call, which returns
`(Vec<Seq>, Vec<u8>)`. So the 73/76% covers probing, match extension, the
literal COPY into a Vec, and the SEQUENCE VECTOR build. It is the entire
sequence-production pipeline, not the search. Anything reasoning from "match
finding is 75% of encode" to "make probing cheaper" has skipped that step.

### 7.2 The search is not big enough to be the cost

`l13anat.rs` (new), all columns per INPUT byte, 12 corpora, 8 MiB:

```text
        probes/B  fills/B  seqs/B  matchB  litB   pos/B   mean mlen
  L1      0.333    0.072   0.0394   0.562  0.438  0.405     12.1
  L3      0.067    0.251   0.0666   0.728  0.272  0.318     11.6
```

CORRECTED 2026-08-27, and the first figures published here were WRONG.
`fill_hash_long_after_match` reported to `DF_ENDFILL` but never called
`note_hash_fill`, so `EncodeCounts::hash_fills` omitted DFast's ENTIRE
long-table fill -- DFast writes TWO tables per match and only the short one was
counted. L3's `fills/B` was published as 0.125 and is 0.251; `pos/B` was
published as 0.192 and is 0.318. Fixed at the source, so the counter is now
complete rather than compensated for here. Fourth instrument defect this
session, and the same shape as the others: a counter whose name claims more
than its code counts.

THE CORRECTION CHANGES THE READING. At L3 fills are **3.7x** probes, and DFast
writes **3.77 table slots per sequence** against roughly one probe. L1 is the
mirror image -- 4.6 probes per fill. So the two levels are not "few probes vs
many probes"; they are PROBE-bound and FILL-bound respectively:

```text
  L1   probe-heavy   0.333 probes/B,  0.072 fills/B
  L3   fill-heavy    0.067 probes/B,  0.251 fills/B
```

The per-EVENT conclusion below is unaffected -- the correlations that establish
it are unchanged, and `matchB` still correlates +0.06 / -0.56 -- but "L3 touches
half the positions L1 does" was an artefact of the undercount. It touches
0.318 against 0.405, and its dominant per-position term is the FILL.

`pos/B` is table positions TOUCHED per input byte -- every probe plus every
fill. At L3 that is **0.318** (see the correction above; it was first published
as 0.192). For
0.067 probes/byte to carry 76% of encode, a probe would have to cost ~250
cycles. A hash lookup does not. **L3 touches HALF the positions L1 does
(0.192 vs 0.405) and is still no cheaper per byte.**

Two identities worth knowing before modelling any of this, because both are
exact and both break a regression that includes them:
`matchB + litB == 1.0` always, and `fills == 2 x seqs` by construction
(`fill_fast_after_match` inserts exactly two positions per match).

### 7.3 The cost is per EVENT, not per BYTE -- shown without a model

Correlation of MatchFind ns/byte against each work term, 12 corpora per level:

```text
              probes/B   fills/B   matchB/B   litB/B   seqs/B
  L1            +0.93     +0.60      +0.06     -0.06    +0.57
  L3            +0.78     +0.70      -0.56     +0.56    +0.70
```

**`matchB` -- the bytes covered by matches, i.e. the match-extension work --
correlates +0.06 with the time at L1 and MINUS 0.56 at L3.** Extension is not
the cost. At L3 the sign is negative because more matched bytes means fewer,
LONGER matches, and the cost tracks the number of events rather than the bytes
they span. Everything that correlates positively (probes, fills, seqs) is a
per-EVENT counter; the two per-BYTE counters are the two that do not.

### 7.4 The attribution, and where it is and is not identifiable

Least squares on MatchFind ns/byte. `fills` and `matchB` are dropped as the
exact identities of 7.2 -- keeping them produced coefficients like -20.7
ns/sequence and -38.5 ns/fill, and a negative cost per unit of work is the
tell that a fit is unidentified rather than merely noisy.

The probe:sequence ratio decides whether the remaining two separate:

```text
  L1   1.86 .. 1632    separable -- most probes MISS, so the two vary apart
  L3   0.73 .. 1.27    collinear -- one probe per match; they are ONE event
```

```text
  L1  (R^2 0.980)   hash probes   16.2 ns x 0.3326/B = 5.39 ns/B   67%
                    sequences     88.2 ns x 0.0394/B = 3.48 ns/B   43%
                    literal bytes  1.09 ns x 0.4384/B = 0.48 ns/B    6%

  L3  (R^2 0.963)   match events  94.0 ns x 0.0670/B = 6.29 ns/B   77%
                    literal bytes  9.77 ns x 0.2715/B = 2.65 ns/B   33%
```

ABSOLUTE ns are inflated -- this is a `profile` build and the instrumentation
is in the measurement. Only the comparisons are read. The one that matters:
**at L1 a sequence costs ~5x what a probe costs** (88.2 vs 16.2), and L1 emits
one sequence per 25 bytes while probing every 3rd byte.

### 7.5 What this says

L1 and L3 cost almost exactly the same per input byte in this build --
MatchFind 8.04 vs 8.15 ns/B, encode total 10.13 vs 10.57 -- and they arrive
there by opposite routes:

```text
  L1   many cheap probes      0.333 probes/B,  0.0394 seqs/B
  L3   few expensive events   0.067 probes/B,  0.0666 seqs/B
```

**L3 emits 1.7x more sequences per byte than L1 while probing 5x less.** Since
the cost is per-event, that is where L3's time is. And it explains why the
row-finder and probe-cheapening work found no ceiling at L3 (section 6, and
[[l3-is-dfast-not-chain]]): L3 was never probe-bound. Probing less is not the
lever; it has already been pushed 5x past L1 with nothing to show.

THE LEVER IS THE COST OF EMITTING ONE SEQUENCE. That is `emit_fast_seq` and
its dfast twin -- the 12-byte `Seq` push, the literal copy, the repcode
bookkeeping and the back-extension. At L1 it is 88 ns against a probe's 16.
Note also that `back_ext_bytes` is 0.0069/B at L1 and **exactly 0.0000 at L3**:
the dfast path does no backward extension at all, so whatever L1 spends there
is not a shared cost and the two emitters are not the same code.

NOT YET MEASURED, and the honest next step: this section localises the cost to
the emit event but does not decompose it. The four candidates inside one
sequence -- `Seq` push, literal `extend_from_slice`, repcode update,
back-extension -- are not separated here, and the per-sequence scope needed to
separate them by clock would cost more than the body it measures. It wants an
ablation or an instruction-count decomposition, the way the DecSeq loop was
resolved.

## 8. DFAST HAS NO BACK-EXTENSION -- and that is 1% of L3. 2026-08-27

Section 7 localised the encode cost to the emit EVENT and left its decomposition
open. Opening it found something better than a speed win: a missing capability.

### 8.1 The gap

`emit_fast_seq_body` back-extends every Fast (L1) match -- a backward walk from
the match start converting literal bytes into match bytes. Its own comment calls
that walk "the seventh instance of a capability present in one path and absent
in its neighbour", and names greedy, lazy and bt_lazy as the paths it had to be
re-added to.

DFast -- the finder the DEFAULT level runs -- was never on that list. A search
of the entire `find_dfast_impl_inner` body for `back_eq`, `ip -= 1` or
`back_ext` returned NOTHING. Both of its commit sites push a sequence directly.
C's `ZSTD_compressBlock_doubleFast` does back-extend.

Section 7's table already showed this as a flat zero and nobody had read it as
one: `back_ext_bytes` is 0.0069 per input byte at L1 and **exactly 0.0000** at
L3.

### 8.2 The prize, measured before building

`dfastbext.rs` (new) probes the walk at DFast's commit point WITHOUT applying it
-- same guards, same `back_eq`, so the count is what applying it would recover:

```text
  corpus     matches   can_extend   share      bytes   vs literals emitted
  dickens     926306      144893    15.6%     169379          21.00%
  webster     718425       94507    13.2%     111533          16.13%
  nci         187560       15141     8.1%      16233           6.51%
  mr          796307       58618     7.4%      61377           5.86%
  osdb        446696       52772    11.8%      57252           3.10%
  TOTAL      6124550      588530     9.6%     681749           2.65%
```

The distribution is the tell: dickens and webster, the two corpora with the most
recoverable literals, are also the two worst natural-text ratio cells against C
at L3 (+5.16% and +5.00%).

### 8.3 The board

Implemented behind `DFAST_BEXT_ARM`, DEFAULTING OFF, because this CHANGES THE
BITSTREAM. With the arm off, bytegate is unchanged -- `BE0071FB0CB0CED9` -- so
the default path is byte-identical and the identity gate still means something.
The verdict comes from a SIZE board (`bextboard.rs`), which also round-trips
every frame:

```text
  L3   50,818,930 -> 50,308,336   -510,594   -1.0047%   smaller 15, larger 0, unchanged 3
  L4                                          -0.7772%   smaller 15, larger 0, unchanged 3
  L2                                           0.0000%   (Fast -- not this finder)
  L5                                           0.0000%   (lazy -- not this finder)
```

Per corpus at L3: dickens **-3.327%**, webster **-3.214%**, reymont -1.740%,
xml -1.634%, samba -1.616%, nci -1.607%, sao -1.204%, osdb -1.181%,
ooffice -1.130%, mr -1.103%. **Zero regressions on any corpus at any level.**

The scope is exactly right: L3 and L4 move, L2 and L5 do not, which is what
"this touches `find_dfast_impl_inner` and nothing else" should look like.

### 8.4 The cost

`find_dfast` 1409 -> **1459 instructions (+50)**. The loop body runs on the 9.6%
of matches that can extend, mean 1.16 iterations; every other match pays one
guard evaluation. Section 7 established the cost here is per-EVENT, and this
adds a small constant to an event that already costs ~94 units -- so the size
win is bought cheaply, but it is NOT free and the arm exists so that is
adjudicable.

TWO DETAILS THAT ARE EASY TO GET WRONG, both taken from L1's proven emitter:

  * the hash FILLS use the PRE-extension position (`found_ip`), exactly as
    `emit_fast_seq_body` passes `found_ip` to `fill_fast_after_match` rather
    than the walked-back `ip`. Filling from the extended position changes which
    slots the table holds -- a different change with a different verdict.
  * `best_ip` and `best_m` fall together, so the OFFSET is unchanged and
    `best_ip + best_ml` is unchanged. Only the literal/match split moves.

The REP path is deliberately not back-extended, matching C: a rep match starts
at `ip+1` by construction.

### 8.5 A hijack, self-inflicted, and what it says about the guard

Inserting the counters landed a new line BETWEEN `#[cfg(feature = "profile")]`
and the `pub static ROW_LOADS` it belonged to -- and again in `lib.rs` between a
`#[cfg]` and its `pub use`. The arm was gated out of shipping builds and
`ROW_LOADS` was gated INTO them; six compile errors said so immediately.

`scripts/twinguard.py` did not catch it, correctly: it scopes to attributes
that can ONLY apply to a function (`target_feature`, `inline`, `cold`,
`no_mangle`, `track_caller`), and `#[cfg]` applies to almost anything, so
including it would drown the check in false positives. The compiler catches the
`#[cfg]` variant on its own, which is why the scope is drawn there. Recorded so
the next person does not "fix" the guard by widening it.

### 8.6 What is still open in MatchFind

  * THE DEFAULT IS STILL OFF. Flipping it is a deliberate act: it moves
    bytegate's GOLD, which anchors every other identity claim in this campaign.
  * The emit event's SPEED decomposition -- `Seq` push, literal copy, repcode
    update, back-extension -- is still not separated. Section 7's caveat stands.
  * A capability matrix across the six finders shows `rep2` matched by NONE of
    them, while every one of them tracks three repcodes. Whether that is a real
    gap or an artefact of how the search is written is unmeasured, and it is the
    obvious next thing to probe the way 8.2 probed this one: measure the prize
    before building anything.

## 9. BREAKING OPEN THE FILL -- one win, four refutations, and where the lever is. 2026-08-27

Section 7 (as corrected) made DFast's fill the dominant per-position work at the
DEFAULT level: 0.251 fills per input byte against 0.067 probes, i.e. **3.77
table writes per sequence** across two tables at two positions. This opens it.

THE HEADLINE IS NOT AN INSTRUCTION WIN. Three separate micro-optimisations of
the fill's instruction count measured ZERO, because LLVM had already taken
them. The available lever is the fill COUNT, and it is large.

### 9.1 The one instruction win: fuse the pair, share position `b`

DFast called two helpers per match. Both recomputed `match_end - 2`, its
`<= ilimit` bound, the `!= a` test, `pack_tags`, and two `is_empty()` reads
through `&mut MatchTables`. More usefully, both called `hash4_tag_mls` at
position `b` -- which BOTH tables index identically, always. The short store
wants `(hash, tag)`; the long store wants only the tag and was recomputing the
pair to get it.

`fill_dfast_after_match` does one walk and computes that hash once:

```text
  find_dfast   1459 -> 1449 instructions   (-10),  cl 20 -> 19
  bytegate     BE0071FB0CB0CED9            UNCHANGED
```

### 9.2 Four refutations, each recorded so it is not retried

  * **Sharing position `a`'s tag across the two anchors: +31 instructions.**
    The anchors differ only when the next-long probe wins, so an `la == sa`
    test would share the hash in the common case -- but the branch costs more
    than the hash it saves on a path LLVM had as straight-line code. This is
    the same shape as the REFUTED table-surgery note in `find_dfast_impl_inner`:
    restructuring a fill that LLVM already has flat makes it worse.
  * **Block-hoisting `pack_tags` / `tags.is_empty()` / `ltags.is_empty()`
    out of the per-match path: ZERO.** They are loop-invariant and LLVM had
    already hoisted them. The explicit parameters were kept -- they cost
    nothing and they document the invariant -- but they bought nothing.
  * **Sharing the `load_u64le` between `hash4_tag_mls` and `hash8_shift`:
    ZERO.** Both begin with the same load at the same position, so this looked
    free. LLVM had already CSE'd it. `hash4_tag_from` / `hash8_from` (the
    mixing halves, split from the load) are kept because they make the sharing
    explicit, but the win was already banked.
  * **C's fill-anchor shape: +0.115% SIZE.** GATE 12's note says C fills both
    tables at the same two positions while we anchor the long table on the
    pre-probe `ip`. Boarding `dfast_fill_anchor_c` ON: 39,744,039 bytes against
    39,698,213 OFF. Our shape is BETTER than C's here, which is why that arm
    ships off. Worth stating plainly: this is a place where matching C would
    cost us.

### 9.3 The lever: half the fill work buys 0.371%

`fillcut.rs` (new) boards each setting of `dfast_fill_ends` -- the fills
actually performed against the size they buy. Both columns are deterministic:
one is a census, the other is the bitstream.

```text
  fill arm                    fills     vs base         bytes     size
  both ends (DEFAULT)      24497126       1.00x      39698213    0.000%
  start only  (a)          12289245       0.50x      39845447   +0.371%
  end only    (b)          12204440       0.50x      40199517   +1.263%
  neither     (none)              0       0.00x      40460080   +1.919%
```

THE MARGINAL READING. All of the fill work buys 1.919% of ratio. The START
half (`match_ip + 2`) buys 1.548% of that; the END half (`match_end - 2`) buys
only **0.371%** while costing exactly half the writes. The two halves are not
worth the same and the board says so 3.4x over.

So `start only` is 0.50x the fills for +0.371% size. For comparison, the row
finder's accepted trade was 0.28x fills for +0.97% -- this is a better rate per
fill removed. FLIPPED 2026-08-27. Boarded first across levels, because this arm is read by
`find_fast` TOO -- it is not DFast-only:

```text
  L1      1,872,359 -> 937,927 fills (0.50x)    size +0.150%
  L3      6,448,402 -> 3,230,205 fills (0.50x)  size +0.482%  (2 MiB cap)
  L9/L19  unaffected -- different finders, zero fills through this path
```

L1's cost is a third of L3's, which the single-level board would have missed.
Whole-board effect: **59,760,356 -> 59,841,188 bytes, +0.135%**, for HALF the
per-match table writes at every level that fills through here.

bytegate GOLD moved DELIBERATELY, `BE0071FB0CB0CED9` -> `CAE84167220B70DA`,
and the history is recorded in `bytegate.rs`'s header so the anchor's moves stay
auditable. xxhgold unchanged; 173 tests pass; clippy clean on all arms.

### 9.4 Two instrument defects found while doing this

  * `fill_hash_long_after_match` reported to `DF_ENDFILL` but never called
    `note_hash_fill`, so `EncodeCounts::hash_fills` omitted DFast's ENTIRE
    long-table fill. Section 7's L3 `fills/B` was published as 0.125 and is
    0.251; `pos/B` was 0.192 and is 0.318. Fixed at the source and corrected
    in place there.
  * `set_dfast_fill_n_arm(n)` STORES `n + 1`, while `dfast_fill_ends` matches
    the STORED value. So the argument is one less than the arm, and `2` lands
    on the `_` (both-ends) arm rather than a disabled one. The first run of
    this board passed the raw arm values, mislabelled every row, and reported
    the default as "0 fills". The numbers looked absurd -- more fills for
    "neither" than for "both" -- which is the only reason it was caught.

### 9.5 What is left

The fill's instruction count is now tight: three independent attempts to
shave it measured zero because LLVM had them already. Further gain from the
fill has to come from doing FEWER of them, and 9.3 prices that exactly.
`fill_hash_after_match` and `fill_hash_long_after_match` are now unreferenced
at the DFast commit point (LLVM drops them); they remain in source as the
reference shape.

## 10. THE EMPTY BUCKET: a lead, a wrong baseline, and two refutations. 2026-08-27

`walkexit.rs` (new) censuses the chain walk's EXIT REASON -- which of the seven
ways it can end actually fires. It was built to test one claim and it refuted
it; then the follow-up refuted a second. Nothing here is a win, and that is the
result.

### 10.1 The claim it was built to test, refuted

`l9cache.rs` showed L9's probe count collapsing 1.845 -> 0.553 per byte as the
tables shrink. I explained that as "smaller tables collide more, so the walk's
`next >= m` guard breaks sooner" and did not check it. The census:

```text
  tables      probes/B   walks     empty   LINK GUARD   depth spent
  8M+4M          1.845   9.44M     28.8%        12.4%         58.9%
  1M+512K        1.370   9.86M      7.9%        29.5%         62.6%
  256K+128K      0.832  10.85M      2.6%        26.3%         71.1%
  64K+32K        0.553  11.82M      0.6%        19.3%         80.1%
```

LINK GUARD is NOT monotone (12.4 -> 29.5 -> 26.3 -> 19.3). The explanation was
wrong. What actually moves is `empty bucket`, 28.8% -> 0.6%, while walks RISE
(9.44M -> 11.82M) and full-depth walks rise (58.9% -> 80.1%). The walk does
roughly 1.7x MORE iterations at small tables, not fewer.

So why does `probes/byte` fall? Because **`probes` only counts candidates that
survive the TAG FILTER** -- `probes += 1` sits in the `else` of the tag-reject
branch (`encode.rs`, the walk's per-candidate block). More collisions means more
tag mismatches, so a larger share of iterations are rejected before being
counted. `probes/byte` is not a work counter here; it is a
"candidates past the tag filter" counter, and shrinking the tables SHIFTS work
from full compares into cheap tag rejects rather than removing it.

This also corrects section 9's reading of `l9cache`: "the speedup is mostly less
work" is not supportable. Iterations went UP 1.7x while time went DOWN 2.9x.

### 10.2 The empty bucket is warm-up, not a hash defect -- and the baseline is
### the whole story

28.8% of L9 chain searches find an EMPTY bucket. That number means nothing
without a baseline, and the obvious one is wrong.

`e^(-n/s)` is the STEADY-STATE empty fraction for `n` positions in `s` slots.
But the table fills PROGRESSIVELY: the search at position `i` sees about `i`
entries, not `n`. Averaged over the pass the correct baseline is
`(s/n) * (1 - e^(-n/s))`, which is far higher:

```text
  load n/s   steady e^-L   fill-averaged   measured   vs fill-avg
      0.50         60.7%           78.7%      40.7%         0.52x
      1.00         36.8%           63.2%      36.6%         0.58x
      2.00         13.5%           43.2%      28.8%         0.67x
      4.00          1.8%           24.5%      22.8%         0.93x
```

Against `steady`, the bottom row reads **12.5x excess** -- severe clustering, a
hash defect, a lead worth days. Against `fill-avg` it reads **0.93x**: our hash
is doing BETTER than uniform. There is no clustering. `bucketfill.rs` prints
both columns so the wrong comparison cannot be made by accident.

### 10.3 The dead work on that path is real, and not worth removing

On an empty bucket both `lz_insert` and `lz_insert_only` still do two array
accesses describing a predecessor that does not exist: a LOAD (`tags[h]`, to
build `old_tag`) and a STORE (`ctags[ip & mask] = old_tag`). The load's result
is discarded by the caller on the `None` path; the store describes a link of 0,
which the walk never tag-filters, since position 0 is sentinel-ambiguous and
skips the filter via `m != 0`. Both are genuinely dead.

Removing them costs more than they do:

```text
  guard the LOAD and the STORE   find_lazy +75   find_greedy +45
  guard the LOAD only            find_lazy +51   find_greedy -29
```

**The branch that avoids the dead work is more expensive than the dead work.**
Reverted; `find_lazy` and `find_greedy` are back at 1568 / 1494 and bytegate is
unmoved. Recorded so the "obvious" cleanup is not attempted a third time.

### 10.4 The neighbours, and why they are closed

  * THREE of the seven exits fire at **0.0%**: window bound, `block_end`, and
    the entry guard. They are correctness bounds on adversarial input, and the
    walk's three validity tests were already folded to one (`low`). Nothing to
    take.
  * **58.9% of walks run their full depth** (`attempts = 16` at L9) and roughly
    half of those iterations are tag rejects rather than probes. That is the
    cost centre, and it is a DEPENDENT-LOAD problem: `m = chain[m & mask]`,
    where each address is the previous load's result.
  * Folding `cp`/`ca` to const generics would remove two register compares per
    iteration at the cost of duplicating the whole walk. The D4 note at the
    `cfb` selection already prices that shape: "457 instructions of duplicated
    walk converting THREE BMI2 ops, 152 per op, the worst ratio in the crate."
    Refuted by precedent, not attempted.

### 10.5 What this leaves

The chain walk's instruction count is not the lever at L9; its memory access
pattern is. The one structural answer -- C's row match finder, which turns N
dependent loads into ~N/16 independent ones -- exists here and measures 6.5
loads per candidate (`l9row.rs`), which is backwards. That ratio, not any
instruction count, is the open defect.

Still unmeasured and boardable: `attempts = 16` may be deeper than the chain
quality justifies, given 58.9% of walks spend it in full. Depth against ratio is
a two-column deterministic board of exactly the `fillcut.rs` shape.
