# M7 anatomy - rusty_zstd vs facebook/zstd v1.5.7, side by side

**Date:** 2026-08-21 - sections 1, 2 and 3 regenerated after the **BMI2 twin
campaign** (`08d14e3`..`bfd0cf0`: every hot path ISA-twinned, transitive trap trace
empty) and the 08-20 gate/dispatch session (`30e6863`..`0fbc57b`). The twin campaign
is byte-exact by construction, so every RATIO movement on this board against 08-20
belongs to the 08-20 session's adjudicated trades, not to the twins. Older boards are
in git history.

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

**Re-measured 2026-08-21.** Sorted by `us/c size`. C is the pinned v1.5.7. Session
null arm (worst same-arm encode spread) **13.20%** -- read every speed column beside it.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  1604.8 |  7282.0 |   **0.22** |   3884.3 |   19295.7 |   **0.20** | **0.088** |
| zeros-32m    | 12282.7 | 31164.8 |   **0.39** |  37855.6 |   41279.7 |   **0.92** | **0.967** |
| incomp-32m   |  5530.1 |  3216.5 |       1.72 |  20674.8 |   23201.9 |   **0.89** |     1.000 |
| x-ray        |   876.3 |   365.6 |       2.40 |   1299.2 |    1060.7 |       1.22 |     1.000 |
| smallmsg-8m  |   510.4 |   279.5 |       1.83 |   2243.2 |     970.9 |       2.31 |     1.009 |
| mr           |   466.0 |   203.4 |       2.29 |   1631.9 |     852.8 |       1.91 |     1.010 |
| osdb         |   509.0 |   200.5 |       2.54 |   1691.2 |     936.1 |       1.81 |     1.011 |
| ooffice      |   442.2 |   173.3 |       2.55 |   1146.9 |     724.3 |       1.58 |     1.011 |
| sao          |   393.0 |   224.6 |       1.75 |   1081.7 |     622.9 |       1.74 |     1.011 |
| samba        |   560.3 |   304.9 |       1.84 |   2329.6 |    1047.0 |       2.23 |     1.012 |
| webster      |   382.8 |   215.5 |       1.78 |   1499.0 |     774.6 |       1.94 |     1.014 |
| dickens      |   334.6 |   191.5 |       1.75 |   1601.2 |     727.0 |       2.20 |     1.015 |
| reymont      |   311.5 |   186.5 |       1.67 |   1579.0 |     645.3 |       2.45 |     1.015 |
| mozilla      |   696.3 |   326.7 |       2.13 |   1975.5 |    1024.2 |       1.93 |     1.034 |
| xml          |   781.0 |   423.4 |       1.84 |   2564.8 |    1154.8 |       2.22 |     1.050 |
| jsonlog-16m  |   634.4 |   328.8 |       1.93 |   1734.1 |     961.1 |       1.80 |     1.063 |
| nci          |   866.4 |   484.3 |       1.79 |   2520.9 |    1142.7 |       2.21 |     1.106 |
| text-32m     | 20072.9 | 13291.2 |       1.51 |   9705.4 |   33698.4 |   **0.29** |     1.126 |

**mean C/us comp 1.77, decomp 1.66 | mean ratio 0.975 | worst ratio 1.126 (text-32m) |
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

These size moves belong to the 08-20 gate/dispatch session -- the 08-21 BMI2 twin
campaign is byte-exact by construction (all identity boards unchanged). L1's mean ratio
now reads **0.975**: at matched settings we emit FEWER bytes than C on average, carried
by `versions-16m` 0.088 and a mid-board that has converged to ~1.01.

### Where we win or tie

- **Compress at or better than C:** `versions-16m` **0.22**, `zeros-32m` **0.39**.
- **Decompress at or better than C:** `versions-16m` **0.20**, `text-32m` **0.29**,
  `incomp-32m` **0.89**, `zeros-32m` **0.92**.
- **Ratio at or better than C:** `versions-16m` **0.088**, `zeros-32m` **0.967**,
  `incomp-32m` 1.000, `x-ray` 1.000.

Against mission section 7 (decompress <= 1.11, compress <= 1.25):
**2 corpora pass compress, 4 pass decompress.** `text-32m` compress flipped out of the
winners' column (0.78 -> 1.51) and `incomp-32m` narrowed (1.85 -> 1.72); both swings
sit on 32 MiB degenerate corpora where the 13.20% null arm and the 08-20 session's
work-shape changes overlap -- neither direction is a claim this instrument can carry.

### Where we lose

- **Ratio:** `text-32m` **1.126**, `nci` **1.106**, `jsonlog-16m` 1.063, `xml` 1.050.
  Everything else is at or below 1.034. The 08-20 losers are gone: `reymont` 1.131 ->
  **1.015**, `mr` 1.089 -> **1.010**, `dickens` 1.082 -> **1.015**.
- **Compress:** `ooffice` **2.55**, `osdb` 2.54, `x-ray` 2.40, `mr` 2.29. These are
  the honest, matched-output positions; none is an open regression.

**`versions-16m` remains the headline** -- 0.075 size, 13x smaller than C, from the
repcode bricks (67/70/71/73/75). It was also the corpus that exposed the missing
function: `find_fast` had a repcode search and no other finder did.

---

## 2. Level 3 (dfast) - the shipping default

**Re-measured 2026-08-21.** Sorted by `us/c size`. Session null arm **17.80%** --
this run's speed columns are NOISIER than usual; ratio is exact.

| corpus       |  C comp | us comp | **C/us c** | C decomp | us decomp | **C/us d** | us/c size |
| ------------ | ------: | ------: | ---------: | -------: | --------: | ---------: | --------: |
| versions-16m |  4311.8 |  7129.5 |   **0.60** |  21369.2 |   19097.6 |       1.12 | **0.659** |
| zeros-32m    |  7158.4 | 27155.5 |   **0.26** |  37286.4 |   39177.3 |   **0.95** | **0.967** |
| x-ray        |   189.1 |    77.3 |       2.45 |   1007.5 |     424.6 |       2.37 | **0.983** |
| incomp-32m   |  3837.9 |  3365.3 |       1.14 |  22450.8 |   22785.5 |   **0.99** |     1.000 |
| jsonlog-16m  |   369.4 |   207.1 |       1.78 |   1830.6 |     896.0 |       2.04 |     1.010 |
| osdb         |   338.4 |   151.2 |       2.24 |   1745.5 |     770.4 |       2.27 |     1.016 |
| mr           |   273.2 |   131.3 |       2.08 |   1500.6 |     479.4 |       3.13 |     1.022 |
| smallmsg-8m  |   318.2 |   147.3 |       2.16 |   2202.4 |     886.9 |       2.48 |     1.029 |
| mozilla      |   379.3 |   201.6 |       1.88 |   1892.6 |    1013.4 |       1.87 |     1.029 |
| ooffice      |   234.1 |    95.2 |       2.46 |   1038.3 |     511.9 |       2.03 |     1.032 |
| sao          |   195.2 |    80.3 |       2.43 |    922.1 |     488.8 |       1.89 |     1.034 |
| samba        |   406.1 |   231.8 |       1.75 |   2310.9 |     974.0 |       2.37 |     1.036 |
| reymont      |   264.0 |   143.7 |       1.84 |   1559.4 |     530.1 |       2.94 |     1.041 |
| webster      |   250.5 |   139.1 |       1.80 |   1488.5 |     557.0 |       2.67 |     1.050 |
| dickens      |   219.7 |   121.1 |       1.81 |   1387.8 |     502.3 |       2.76 |     1.052 |
| text-32m     |  9759.6 | 20936.9 |   **0.47** |   9481.7 |   31570.6 |   **0.30** |     1.071 |
| xml          |   626.0 |   318.1 |       1.97 |   2481.3 |    1087.9 |       2.28 |     1.079 |
| nci          |   783.1 |   390.4 |       2.01 |   2534.9 |    1162.9 |       2.18 |     1.100 |

**mean C/us comp 1.73, decomp 2.04 | mean ratio 1.012 | worst ratio 1.100 (nci) |
we beat C: 3 comp, 3 decomp, 3 ratio**

**L3 ratio is UNCHANGED cell-for-cell from the 08-20 board** -- every `us/c size`
matches to the third decimal, which is exactly what a byte-exact campaign predicts and
doubles as an end-to-end identity check on the twins. Mean **1.012**, worst `nci`
**1.100**. Three corpora still BEAT C -- `versions-16m` 0.659, `zeros-32m` 0.967 and
`x-ray` 0.983. `incomp-32m` compress narrowed to **1.14** and its decompress touched
**0.99**.

**Decompress remains the weaker half at L3** (2.06-3.53 against compress 2.00-3.23),
unchanged in character from the 08-17 board and still following from emitting Huffman
literals where we used to emit raw ones.

Against mission section 7: **4 pass compress** (zeros 0.26, text 0.47, versions 0.60,
incomp 1.14), **3 pass decompress** (text 0.30, zeros 0.95, incomp 0.99); `versions`
decompress misses by one hundredth at 1.12.

> **WHAT THIS BOARD DOES NOT SAY.** It does not say the optimization campaign made us
> faster. Its speed columns cannot carry that claim at a 13.45% null spread, and the
> campaign never made it -- every item shipped on strictly-less-work plus byte-identity.
> See [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md) section 4.

---

## 3. Stage anatomy — where OUR time goes

**Re-measured 2026-08-21 (fifth pass, post twin campaign), L3, 8 MiB board

| corpus       | MatchFind |     Huff | FseSeq | SeqCode |  DecLits |   DecSeq |    DecCk |
| ------------ | --------: | -------: | -----: | ------: | -------: | -------: | -------: |
| smallmsg-8m  |  **82.3** |      5.0 |    4.6 |     4.7 |     10.0 | **85.6** |      2.5 |
| jsonlog-16m  |  **78.7** |      5.6 |    5.8 |     6.1 |     10.9 | **84.3** |      2.8 |
| sao          |  **76.9** |     11.7 |    4.8 |     2.9 |     30.0 | **67.5** |      2.2 |
| x-ray        |  **75.4** |     12.3 |    5.5 |     3.6 |     15.9 | **82.2** |      1.7 |
| dickens      |  **75.0** |      4.0 |    8.8 |     8.6 |      3.4 | **94.8** |      1.7 |
| webster      |  **74.9** |      4.9 |    8.7 |     8.1 |      4.2 | **93.3** |      2.2 |
| samba        |  **74.7** |      7.1 |    7.3 |     5.9 |      7.4 | **88.5** |      3.6 |
| mozilla      |  **74.3** |     10.3 |    5.2 |     4.6 |     14.0 | **80.9** |      4.2 |
| reymont      |  **73.2** |      4.2 |   10.5 |     8.3 |      3.1 | **94.7** |      1.9 |
| mr           |  **73.0** |      8.8 |    9.0 |     6.1 |      5.5 | **92.6** |      1.7 |
| ooffice      |  **72.3** |     13.0 |    5.9 |     5.4 |     18.1 | **78.7** |      2.1 |
| osdb         |  **70.5** |     16.3 |    5.4 |     4.3 |      9.9 | **86.9** |      3.0 |
| xml          |  **69.0** |      7.8 |    9.2 |     7.6 |      7.6 | **87.9** |      4.2 |
| nci          |  **64.9** |      6.6 |   11.9 |     9.4 |      6.6 | **87.7** |      5.2 |
| text-32m     |  **59.9** |      3.7 |    1.0 |     0.3 |      0.4 | **75.9** |     22.2 |
| versions-16m |  **55.0** |      1.0 |    2.9 |     4.6 |      0.3 | **76.9** |     21.8 |
| incomp-32m   |  **11.4** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **23.7** |
| zeros-32m    |   **0.0** |      0.0 |    0.0 |     0.0 |      0.0 |      0.0 | **24.2** |

### MatchFind Function Anatomy

The largest stage (#1 on **18 of 18** corpora -- see section 3) and by far the most
dispatched.

**Function-level breakdown, 2026-08-21 (post twin campaign).** Two instruments, read
together: the profile clock allocates TIME per level (shares comparable only within
the run, `mfanat.rs`), and the asm census gives each function's footprint and ISA
state.

**(a) Time -- which function owns MatchFind at each level.** 18-corpus 8 MiB board,
profile build; milliseconds per input MiB.

| level | strategy | serving function(s)          | MF ms/MiB | MF % of encode |
| ----: | -------- | ---------------------------- | --------: | -------------: |
|    L1 | Fast     | `find_fast_impl`             |       6.0 |           73.9 |
|    L3 | DFast    | `find_dfast_impl`            |       7.0 |           76.5 |
|    L5 | Greedy   | `find_greedy` (walk inlined) |      11.0 |           83.4 |
|    L7 | Lazy     | `find_lazy` + `chain_find_best` |   44.1 |           95.1 |
|    L9 | Lazy2    | `find_lazy` + `chain_find_best` |   74.8 |           97.0 |
|   L12 | Lazy2    | `find_lazy` + `chain_find_best` |  174.3 |           98.6 |
|   L13 | BtLazy2  | `find_bt_lazy` + `bt_find_best` | 273.0 |           99.2 |
|   L15 | BtLazy2  | `find_bt_lazy` + `bt_find_best` | 329.9 |           99.3 |
|   L16 | BtOpt    | `find_opt` + `bt_find_best`  |     319.9 |           99.3 |
|   L19 | BtUltra2 | `find_opt` + `bt_find_best`  |     379.4 |           99.4 |
|   L22 | BtUltra2 | `find_opt` + `bt_find_best`  |     406.7 |           99.3 |

Three readings. (1) **MatchFind IS the encoder at every level**: 73.9% at L1 rising
to 99.3-99.4% from L13 up -- above the chain ladder there is no second stage worth a
row. (2) The ladder spans **68x** in absolute cost, 6.0 -> 406.7 ms/MiB; the two big
jumps are Greedy -> Lazy (11 -> 44, the look-ahead re-search at every found match)
and Lazy2 -> BtLazy2 (174 -> 273, the tree walk). (3) L16 costs LESS than L15
(319.9 vs 329.9): the DP prices candidates it then declines to re-search, where
BtLazy2's look-ahead searches unconditionally.

**(b) The functions -- footprint and ISA state** (asm census, same build; "copies"
counts monomorphisations, plain + BMI2 twin).

| function | copies | live-copy instrs | ISA receipt | unit of execution |
| --- | ---: | ---: | --- | --- |
| `find_fast_impl` | 140 + 140 | <= 2,458 / 2,425 | twins carry 1,878 shrx, 0 CL | per position, L1-L2 |
| `find_dfast` + `find_dfast_impl` | 1 + 6 + 6 | dispatcher 9,397; twin <= 1,908 | 30 shrx; 2 `shrb $const` per twin (no 8-bit shrx exists -- irreducible) | per position, L3-L4 (default) |
| `find_greedy` | 1 + 1 | 2,269 / 2,219 | 27 shrx, 0 CL in twin | per position, L5 |
| `find_lazy` | 1 + 1 | 1,958 / 1,930 | 13 shrx, 0 CL in twin | per position, L6-L12 |
| `chain_find_best` | 2 + 2 | <= 466 / 454 | 7 shrx; outlined by brick 48, ISA via per-block `ChainFn` pointer | per position + per look-ahead step, L5-L12 |
| `bt_find_best_impl` | 42 + 42 | <= 229 each | 84 shrx total; ISA chosen once per block in `bt_resolve` | per position x ~30M, L13-L22 |
| `bt_find_best_runtime` | 1 + 1 | 259 / 261 | 5 shrx | fallback arm |
| `find_sequences_strategy` | 1 + 1 | 2,377 / 2,271 | `find_opt` + `find_bt_lazy` inlined into BOTH arms (find_opt pinned 08-21) | per block hub |
| `find_sequences` | 1 + 1 | 722 / 672 | ldm glue | per block |
| `count_match` / `count_eq_len` | 1 / 1 | 153 / 77 | AVX2 arm via `has_avx2()` | per candidate hit |
| `match_ok` (+ cold tail) | inlined + 1 | 66 (tail) | memcmp only in the cold mls>8 tail | per candidate |
| `emit_fast_seq`, `prime_tables`, `fill_fast_after_match`, `fast_hash_relatch` | -- | fully inlined into the twins | -- | per match / per block |

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

### Cross-cutting: dispatches that are DEAD in the shipping build

Found while auditing, per the `rusty_curiosity` law that an unused thing is invisible
to every profiler:

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

