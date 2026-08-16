# M7 encoder — instrument repair and restart (2026-08-14)

The descent below (2026-08-13) was run on **instrument v1**, whose defects are
catalogued in [`m7-benchmark-repair.md`](m7-benchmark-repair.md): our mean vs C's
best (median +9.5%/+11.4% in C's favour, content-dependent), mid-session thermal
decay invisible to the null arm, and best-of-N with N as low as 5. Its `MEASURED`
lines are not admissible; its `COUNTED` lines still are.

## D2' — where does encode time ACTUALLY go? (repaired instrument, 2026-08-14)

- ASKED: the D3e/D3d ranking put Huffman and FSE emit at the top. Is that right?
- COUNTED: `EncodeEntropy`'s named children (`EncodeHuff` + `EncodeTableSelect` +
  `EncodeFseSeq`) summed to only **54%** of it on samba and **52%** on xml. A 46-48%
  residue sat inside the entropy stage, unnamed, at small call counts (165, 41) where
  profiler tax is negligible.
- ANSWER: the residue is the `seqs -> CodedSeq` transcode plus a second walk over
  `coded` to build the ll/of/ml histograms. Scoped as `EncodeSeqCode`. Share of encode:

| file    | EncodeSeqCode | EncodeFseSeq | EncodeHuff | EncodeMatchFind |
| ------- | ------------: | -----------: | ---------: | --------------: |
| nci     |      **26.3** |         21.1 |        9.4 |            38.4 |
| xml     |      **25.1** |         18.1 |        8.1 |            44.3 |
| samba   |      **21.8** |         15.7 |        9.5 |            49.3 |
| reymont |      **21.4** |         13.5 |       11.8 |            50.6 |
| webster |      **20.7** |         13.1 |        6.7 |            56.7 |
| dickens |          14.0 |         10.0 |       19.3 |            54.4 |
| mozilla |          12.8 |          9.1 |       35.4 |            39.0 |
| mr      |       **1.4** |          1.5 |       60.6 |            33.0 |

- ANSWER: **`EncodeSeqCode` is larger than `EncodeFseSeq` on every file.** Bricks 17 and
  33 optimised the FSE emit (10-15%) and never saw the bigger pass in front of it,
  because it was attributed to "entropy".
- CONFIDENCE: high -- named scope, small call counts, reproduced across runs.

## D3' — why is the transcode 85 cycles per sequence?

- COUNTED: samba `EncodeSeqCode` 26.46 ms / 741,851 seqs = **35.7 ns = ~85 cycles/seq**
  for five small helpers. Impossible for the arithmetic involved.
- ANSWER (by reading, per the rs_h264 law): `code_from_base` is a **linear scan from the
  TOP** of the base table. `LL_BASE` has 36 entries rising to 65536; `ML_BASE` has 53
  rising to 65539. Measured mean lengths are ll ~7.8 and ml ~21.3, so `val >= base[i]`
  fails nearly all the way down: **~36 + ~53 = ~89 iterations per sequence**. That is
  the 85 cycles. C keeps `LL_Code[64]` / `ML_Code[128]` byte tables for exactly this.
- CLASS: `codec-eliminate-redundancy` move #2 (replace a search with a table) -- the
  same shape as the MP3 Huffman-decoder case study.

## Brick 35 (kept) -- compile-time LL/ML code LUT

`build_code_lut::<N>()` is a `const fn` that **evaluates `code_from_base` at compile
time**, so the fast path cannot drift from the oracle by construction. `LL_LUT_LEN=64`,
`ML_LUT_LEN=256`; above that the linear scan still runs (rare). The scan stays in-tree
as the oracle.

The oracle test caught a real defect before it shipped: `code_from_base` falls off the
bottom as `(0, val, 0)`, **not** `val - base[0]`, and `ML_BASE[0]` is 3 -- so a naive
fast path underflows `u32` for any `val < 3`. Tests `ll_ml_code_lut_matches_linear_scan`
(every base, base-1, base+1, both LUT boundaries, 65536-ish, `u32::MAX-1`) and
`code_lut_exhaustive_over_lut_domain` (exhaustive 0..512).

MEASURED (repaired instrument, profiler OFF, best-of-N both arms, N>=25/phase; noise
floor +/-2.8% measured over two prior repeats). **`mr` is the negative control** -- its
SeqCode share is 1.4%, so it must NOT move:

| file             | SeqCode share | cyc/byte before |      after |     change | C/us c before |    after |
| ---------------- | ------------: | --------------: | ---------: | ---------: | ------------: | -------: |
| nci              |         26.3% |           7.807 |  **6.447** | **-17.4%** |          2.79 | **2.34** |
| webster          |         20.7% |          16.943 | **14.447** | **-14.7%** |          2.42 | **2.08** |
| samba            |         21.8% |          11.323 |  **9.764** | **-13.8%** |          2.46 | **2.12** |
| dickens          |         14.0% |          20.323 | **17.932** | **-11.8%** |          2.53 | **2.22** |
| mozilla          |         12.8% |          17.129 | **15.416** | **-10.0%** |          3.35 | **3.05** |
| **mr (control)** |      **1.4%** |          16.321 |     16.545 |  **+1.4%** |          2.95 |     2.93 |

The gain tracks each file's SeqCode share (a consistent 65-85% removal of that stage),
and the control is correctly null -- the mechanism is confirmed, not just a number that
moved. Sizes **byte-identical** on all six. Dual-gate 6/6. Lib tests **117/117**.

For scale: the previous ~20 encode bricks moved compress **0-9% in total, all inside
noise**. This one brick moved five of six files 10-17% at a 2.8% floor.

Next in this vein (separate bricks, not yet built): fuse the histogram walk into the
transcode loop (one pass, not two); `resolve_offset`'s return value is discarded
(`let _ =`) and it is called purely for the rep-shuffle side effect; `CodedSeq` is a
20-byte AoS where C keeps three u8 SoA arrays.

## D4' — re-profile after brick 35 (the bottleneck moved)

`EncodeSeqCode` collapsed independently of the timing A/B, which confirms the mechanism
a third time: nci **26.3 -> 10.5%**, webster **20.7 -> 6.3%**, samba **21.8 -> 6.9%**,
xml 25.1 -> 9.0%, mozilla 12.8 -> 4.2%, dickens 14.0 -> 3.4%.

New ranking, share of **encode** (stage / EncodeTotal):

| file    | MatchFind |     Huff |   FseSeq | SeqCode | --  | DecodeSeq / DecodeTotal |
| ------- | --------: | -------: | -------: | ------: | --- | ----------------------: |
| sao     |  **84.5** |      3.9 |      2.3 |     1.2 |     |                      46 |
| webster |  **66.4** |      8.0 |     15.8 |     6.3 |     |                  **89** |
| ooffice |  **61.9** |     22.6 |      8.3 |     3.1 |     |                      71 |
| dickens |  **60.9** |     21.6 |     11.3 |     3.4 |     |                      73 |
| reymont |  **60.2** |     14.2 |     16.7 |     5.7 |     |                  **86** |
| samba   |  **59.3** |     11.6 |     17.7 |     6.9 |     |                  **85** |
| xml     |  **53.7** |      9.9 |     22.3 |     9.0 |     |                  **86** |
| nci     |      46.2 |     11.5 | **25.7** |    10.5 |     |                  **83** |
| mozilla |      42.7 | **38.3** |     10.4 |     4.2 |     |                      69 |
| osdb    |      39.5 | **38.4** |     14.2 |     4.3 |     |                      73 |
| mr      |      33.9 | **60.2** |      1.4 |     0.7 |     |                      21 |

**Two clear next targets.** Encode: `EncodeMatchFind` is now #1 on 7 of 12 files (54-85%);
Huffman emit is #1 only on mr and osdb/mozilla. Decode: `DecodeSeq` is 69-89% of decode
on 9 of 12 files, and `DecodeLiterals` is #1 only on mr and x-ray.

Both areas were worked on instrument v1 (bricks 22/24/34 on `find_fast`; 19/23/30 on the
seq decode) and every one of those refutations is void -- `codec-measurement` 11, a
refutation expires when its baseline moves, and the baseline moved twice (instrument
repair, then brick 35). Re-open them with a **count** first, per D3'.

## Brick 36 (kept) -- fixed-width literal copy in the seq decode loop

`DecodeSeq` is **per-sequence** (~740k calls per block-set), so it may NOT be scoped:
`codec-measurement` 6 says a per-element scope becomes the thing being measured. Priced
by structure and counts instead.

- COUNTED: samba `DecodeSeq` 28.01 ms / 741,851 seqs = **37.8 ns = ~90 cycles/seq**.
- COUNTED: mean literal run **7.8 bytes** (samba `lit_b`/`seqs`), mean match run 21.3.
- ANSWER: two `extend_from_slice` calls per sequence, each a **runtime-length** memcpy.
  `codec-analyzer`: "a hot copy whose SIZE is a runtime parameter is a codegen trap --
  the compiler cannot inline a fixed-width move." At 8 bytes the call overhead IS the
  cost. C solves it with fixed-width `ZSTD_copy16` over an over-allocated output.

`copy_literals` takes a 16-byte `copy_nonoverlapping` when 16 source bytes are readable
AND 16 destination bytes are already reserved, publishing only `n <= 16` via `set_len`;
otherwise the checked path runs unchanged. This is `rusty-unsafe-optimizations` pattern
2, in a documented island under `#[allow(unsafe_code)]` (the crate root keeps
`deny(unsafe_code)`), with the checked path retained as the oracle.

Gate: `copy_literals_fast_matches_checked` sweeps every length 0..=40 against 7 source
offsets and 5 capacities -- deliberately covering the three FALLBACK cases (short source
tail, no spare capacity, n > 16) as well as the fast path -- plus
`copy_literals_rejects_overrun`. Lib tests **119/119**.

MEASURED (`--levels 1`, profiler OFF, null-arm 1.0651). **mr is the control**: its
`DecodeSeq` is 21% of decode, so it must stay flat.

| file             | DecodeSeq/decode | cyc/B d before |     after |    raw | net of control |
| ---------------- | ---------------: | -------------: | --------: | -----: | -------------: |
| webster          |              89% |          4.318 | **3.543** | -17.9% |      **~-15%** |
| xml              |              86% |          2.865 | **2.424** | -15.4% |    **~-12.6%** |
| samba            |              85% |          3.329 | **2.887** | -13.3% |    **~-10.5%** |
| nci              |              83% |          2.695 | **2.421** | -10.2% |     **~-7.4%** |
| **mr (control)** |          **21%** |          2.144 |     2.083 |  -2.8% |             ~0 |

The control's -2.8% is this session's drift, so it is subtracted rather than banked.
C/us d: webster 2.67->**2.24**, samba 2.62->**2.32**, xml 2.71->**2.37**, nci 2.68->2.48.
Sizes byte-identical. Dual-gate 5/5.

## Brick 37 (kept) -- fixed-width match copy (`copy_from_decoded`)

Same class as 36, applied to the other per-sequence copy. The hot arm is
`offset >= len` -> `extend_from_within(src..src+len)`, a runtime-length copy at a
measured mean match run of **21.3 bytes** (samba `match_b`/`seqs`).

The overlap analysis is the whole brick: source and destination are in the **same**
buffer. `offset >= 32` does double duty -- it guarantees 32 readable source bytes
(`src + 32 <= out.len()`) AND that the source range ends at or before the destination
start, so a fixed 32-byte `copy_nonoverlapping` is sound. Gate: `len <= 32 && offset >= 32
&& spare capacity >= 32`; everything else keeps the existing paths (offset-1 splat,
`extend_from_within`, the overlapping doubling loop).

Gate tests: `copy_from_decoded_matches_byte_push` widened to straddle the 32-byte
threshold in both directions (offsets 1..300, lens 1..1000, four capacities so each
(off, len) runs once through the fast path and once through the fallback), plus
`copy_from_decoded_publishes_exactly_len` proving no byte past `len` becomes visible and
the prefix is undamaged. Lib tests **120/120**.

MEASURED (`--levels 1`, null-arm **1.0201** vs the standing board's 1.0206 -- near
identical session conditions). **mr is the control** (DecodeSeq 21% of its decode):

| file             | cyc/B d before |     after |     change | C/us d before |    after |
| ---------------- | -------------: | --------: | ---------: | ------------: | -------: |
| webster          |          3.723 | **2.742** | **-26.3%** |          2.26 | **1.70** |
| jsonlog-16m      |          3.184 | **2.543** | **-20.1%** |          2.11 | **1.67** |
| samba            |          2.905 | **2.383** | **-18.0%** |          2.32 | **1.82** |
| xml              |          2.461 | **2.085** | **-15.3%** |          2.40 | **1.96** |
| nci              |          2.485 | **2.229** | **-10.3%** |          2.40 | **2.16** |
| **mr (control)** |          2.127 |     2.112 |  **-0.7%** |          1.25 |     1.31 |

Compress is unmoved (decode-only change), as required. Sizes byte-identical.
Dual-gate 6/6. Cumulative webster decode across 36+37: **4.318 -> 2.742 (-36.5%)**.

## D5' -- `EncodeMatchFind` opened with COUNTS (2026-08-14)

- COUNTED per file (`--m7-profile`): `fills` is exactly **2x `hits`** everywhere, so the
  two-slot insert is already C-correct and is not the lever.
- COUNTED `hit_rate` spans **0.0085 (sao) to 0.472 (nci)**, and MatchFind ns/probe tracks
  it -- 6.7 ns/probe at hit_rate 0.009, 19.8 ns/probe at 0.472. **The probe is not the
  cost; what happens on a HIT is.** That redirected the brick away from the probe loop
  (where bricks 22/24/34 all failed) and into `emit_fast_seq`.
- ANSWER (by reading): `emit_fast_seq` does `lits.extend_from_slice(&src[anchor..ip])` --
  the **third** instance of the runtime-length-memcpy shape that bricks 36 and 37 fixed in
  the decoder. COUNTED mean literal run per sequence: **nci 1.9 B, xml 3.6, samba 7.8,
  webster 8.9, mozilla 15.7**, over 0.17-1.84M sequences per file. Additionally `seqs` and
  `lits` were `Vec::new()` -- no reservation, doubling from zero every block.

## Brick 38 (IN TREE, speed verdict UNPROVEN -- box contaminated)

Reserves `lits` (block length + 16 slack) and `seqs`, and routes the literal run through
a fixed-width `push_literals` (16-byte `copy_nonoverlapping`, checked fallback, gated by
`push_literals_matches_extend_from_slice` over every length 0..40 x 7 offsets x 5
capacities). Lib tests **121/121**, dual-gate and sizes unchanged on every file measured.

**Correctness is proven. Speed is not.** Sequence of measurements:

1. Fixed `block_len / 8` reservation, clean session (null-arm 1.0090): mozilla **-7.2%**,
   nci **-6.3%**, samba **-4.5%**, webster -3.2%, xml -1.2% cyc/byte compress -- but the
   **sao control went +3.9%**. sao's `match_frac` is 0.023, so it pays a 196 KB per-block
   reservation and uses ~357 slots. That is the same holdout sign-flip that reverted
   bricks 31 and 34, and it is a dispatch trigger, not a revert.
2. Reservation made adaptive (`last_nseq` on `MatchTables`, +25% slack, capped at
   `block_len / mls`). Re-measured **twice**; BOTH sessions inadmissible.

**Why inadmissible, named and dated:** C's own unchanged v1.5.7 binary fell 20-27% in the
first re-run (sao 386->313, mozilla 490->357, nci 932->738) and further in the second
(mozilla 275, samba 404). Our cycles/byte rose 6-13% then doubled on samba -- rising
cycles under falling throughput is the SMT-contention signature, not throttle. The
**null arm read 1.0162 and 0.9918** through both: blind again, as documented in D5.

Cause found per `codec-measurement` 15 (name it, date it -- do not call it weather):
**eight `Code` processes at ~107,000 CPU-seconds each (~30 CPU-hours), running since
2026-08-12**, plus `faucet` (33,623 s) and two `Cursor` processes. Not killed -- that is
the owner's call.

**Status: brick 38 is a `codec-measurement` 12 "unproven" state, not "kept" and not
"reverted because flat".** It is **in the tree behind `RZSTD_LIT_PUSH`, defaulting OFF**
-- `codec-optimize` rule 5 says an unproven change does not ship, and 12 says keep
reverted machinery behind a toggle so re-testing is cheap. Output is byte-identical in
either arm (sizes 1.143 / 1.303 verified both ways).

3. The runaway processes WERE killed (user-authorised) and the box confirmed the
   diagnosis: C jumped to the highest figures of the whole campaign (nci 1043 vs a prior
   best of 932, xml 843 vs 738). But that first post-kill run had **null-arm 1.7399** --
   taken while the box was still ramping -- so it is also inadmissible.
4. An **interleaved ABAB** A/B was then built (`RZSTD_LIT_PUSH`, arm resolved once per
   block, never inside the probe loop -- `codec-measurement` 15) and run. Verdict:
   **inconclusive.** Same-arm reproducibility across two runs of *identical* code was
   3-8% (sao **63%**), against a between-arm effect of 3-7%. The effect is smaller than
   the noise it must clear. VS Code respawned 19 NodeService processes during the run.

### Harness defects this exposed (fix before the next campaign)

- **The first file measured in a session is systematically unreliable.** sao is measured
  first and swung 5.102 -> 8.322 -> 5.789 cyc/byte for identical code, while later files
  in the same runs held to 3-8%. It absorbs cold caches and the frequency ramp. Add a
  discard-first warmup pass, or never place a control/canary first.
- **Cycles/byte is robust to frequency but NOT to SMT contention.** Under contention it
  rose 6-13% (and doubled on samba) while throughput fell, because
  `QueryThreadCycleTime` accrues stall cycles while the thread is resident. It is the
  right cross-session metric on a quiet box and still needs the box to be quiet.
- **The null arm failed a fourth time**, reading 1.0162 / 0.9918 / 1.0344 / 0.9382
  through sessions where C moved 20-27%. Replace it with the ABAB same-arm spread.

## Brick 39 (KEPT, default ON) -- 2-way software-pipelined `find_fast` probe

Opened with arithmetic, not a kernel (D5' said the probe COST is the gap, not the count):

| quantity (webster)                          | value                                  |
| ------------------------------------------- | -------------------------------------- |
| MatchFind total                             | 401.1 Mcyc                             |
| less `emit_fast_seq` (1.84M hits x ~66 cyc) | 121.5 Mcyc                             |
| probe loop                                  | 279.6 Mcyc over 10.76M probes          |
| **per probe**                               | **26.0 cycles**                        |
| our probes/byte                             | **0.259** (C fast L1 probes ~1.0/byte) |

We probe **4x less often than C and are still slower**, so the per-probe cost is the
whole gap. Each probe is two DEPENDENT random loads -- the 256 KiB hash table, then
`src[m]` for the u32 compare -- with no independent work between them. The loop is
latency-bound, not throughput-bound, which is why bricks 22/24/34 (wider loads, prefetch,
`get_unchecked`) all failed: none of them added independent work.

Fix: issue the NEXT probe's hash-table load before consuming the current probe's result,
so the two miss latencies overlap. Byte-identical -- same probe order, same stores, same
results, only the issue order moves. When the next slot aliases the current one
(`h1 == h0`) the just-stored value is forwarded by hand instead of re-read, preserving
the original read-then-write ordering. Only the non-`pair` path is pipelined
(`step0 == 2`, i.e. every level with `target_length == 0`); `--fast=N` keeps the old loop.

**GATE: byte-identical compressed output on all 12 Silesia files** (sha256 of
`rzstd -1 -c` compared across `RZSTD_MF_PIPE=1` / `=0`), lib tests **121/121**.

MEASURED, interleaved ABAB in one session (`RZSTD_MF_PIPE`):

| file                     | MatchFind share |   PIPE | SERIAL |     delta | pairs |
| ------------------------ | --------------: | -----: | -----: | --------: | ----: |
| sao                      |           84.5% |  8.040 |  8.913 | **-9.8%** |   2/2 |
| webster                  |           66.4% | 22.855 | 23.992 | **-4.7%** |   2/2 |
| dickens                  |           60.9% | 29.477 | 30.535 | **-3.5%** |   2/2 |
| samba                    |           59.3% | 15.962 | 16.330 |     -2.3% |   2/2 |
| ooffice                  |           61.9% | 20.743 | 21.019 |     -1.3% |   1/2 |
| **mr** (natural control) |       **33.9%** | 28.152 | 27.718 |     +1.6% |   1/2 |

**Paired win rate 10/12, z = +2.31** -- clears `codec-measurement` 3's |z| > 2 bar. The
effect tracks MatchFind share; `sao` (hit_rate 0.0085, so almost pure probe loop) wins
biggest; `mr` (lowest MatchFind share, Huffman-dominated) is correctly flat.

Next in this vein: the same pipelining applied to the `pair` path (`--fast=N`), and
`emit_fast_seq` at ~66 cyc/hit is still 30% of MatchFind and untouched.

## Brick 38 (still OFF) -- settled as BELOW THE BAR, not reverted

Re-run as a proper interleaved ABAB with the warmup and same-arm spread in place:
**6 of 9 pairs favour ON, z = +1.0** -- short of |z| > 2. The adaptive `last_nseq`
reservation DID fix the holdout regression (sao +3.9% -> -1.2%, inside its own 0.6-6.9%
spread), and webster/nci/xml consistently favour ON (-4.4%, -7.0%, -4.7%). It is a real
but ~5% effect that needs N >= 20 pairs on a quiet box to bank. Left default OFF behind
`RZSTD_LIT_PUSH`; reaching the bar costs ~40 minutes of box time, against 10x more
headroom in MatchFind -- which is why brick 39 was taken first.

## D6' -- WHERE THE BYTES GO (bit accountant, 2026-08-14). The quality gap is LITERALS.

Built `codec-analyzer` instrument #6, which this project never had: per-section emitted
bytes on the encode side, and the SAME counters run over C's own frame on the decode
side, so both sides are in identical units and the gap is attributable.

- COUNTED: literals are **70-99% of our compressed output** on almost every file
  (mr 96.9%, sao 99.0%, ooffice 92.7%, mozilla 86.0%).
- COUNTED, us vs C at L1:

| file    |   our lits |     C lits |           **lit gap** |    seq gap |
| ------- | ---------: | ---------: | --------------------: | ---------: |
| webster | 14,277,513 |  6,675,244 |        **+7,602,269** | -1,641,919 |
| mozilla | 20,984,372 | 14,926,907 |        **+6,057,465** | -1,104,053 |
| samba   |  4,912,782 |  2,979,440 |        **+1,933,342** |   -298,710 |
| dickens |  4,021,460 |  2,310,178 |        **+1,711,282** |   -793,138 |
| reymont |  2,125,719 |    694,352 | **+1,431,367 (3.1x)** |   -490,549 |
| mr      |  4,122,363 |  2,740,017 |        **+1,382,346** |   -945,420 |

- **ANSWER: our SEQUENCES are smaller than C's on 9 of 11 files; our LITERALS are
  1.4-3.1x bigger.** C spends more sequence bytes to avoid far more literal bytes. The
  gap is NOT the Huffman coder -- it is that **C finds more matches**, so it has fewer
  literals left to code. Every future ratio brick should be judged on "does it reduce
  literal bytes", not on entropy-coder tuning.

## Brick 40 (built, default OFF -- NOT shippable yet) -- repcode-1 search

`find_fast` never SEARCHED the repeat offset; it only ever ENCODED a repcode when a
found offset happened to coincide with one. C's `ZSTD_compressBlock_fast` tests the rep
offset at `ip+1` on every position. Implemented in both the serial and pipelined loops
with local `rep1` state mirroring C's `offset_1` (reps threaded through
`find_sequences` -> `find_sequences_strategy` -> `find_fast`).

Correctness is good: `finder_recon` passes, all 28 round-trip tests pass, and the full
file dual gate passes on mr and xml through BOTH our decoder and C's.

MEASURED (exact bytes, deterministic, L1 Silesia): net ratio win on **9 of 12** files --
reymont **-3.05%**, x-ray -1.15%, samba -1.06%, webster -0.56%, dickens -0.49%,
osdb -0.46%, sao -0.45%, mr -0.31%, mozilla -0.14% -- but **xml +3.39%**, nci +0.34%,
ooffice +0.34%. Nowhere near brick 34's claimed nci 1.303 -> 1.145.

**CORRECTION -- the "Repeat FSE mode lost" claim was WRONG.** It came from the tiny
synthetic coverage corpus. A new seq-mode counter (`note_seq_mode`, reported as
`COUNTED seq_modes`) measured real content: with repcode ON, webster still selects
Repeat **843** times (vs 807 OFF), samba and xml are essentially unchanged. There is no
broken interaction. Only **nci** genuinely shifts away from Repeat (446 -> 186), and nci
is one of the three files whose ratio got worse -- so for nci the two facts are the same
fact, not a general defect.

**Why it does not ship yet:** it is a **content sign-flip**, the classic
`codec-content-adaptive-dispatch` trigger. Net across 12 files is only about -0.30%
mean, and xml's +3.39% is larger than most of the individual wins. A ~0.3% mean ratio
change that ships a +3.4% regression to one content class is not a quality win; it needs
either a signal that separates xml/nci/ooffice, or a demonstrated speed payoff (fewer
literals should mean less Huffman work) to justify the trade.

**Two test-fixture defects this exposed and fixed** (both were hostages to matcher
quality, which is a bad property for a coverage gate):
- The finder/entropy oracles re-ran `find_fast` with a hardcoded `[1,4,8]` rep state per
  block instead of threading the evolving reps the encoder uses. Now mirrored.
- `literals_and_sequence_modes_coverage` reached RLE literals only via a residue the
  match finder happened to leave. RLE literals are now gated DIRECTLY on the emit path
  (`rle_literals_section_emits_type_1_and_round_trips`), which no matcher change can
  invalidate.

## D7' -- THE BINDING CONSTRAINT: we are on a WORSE speed/ratio CURVE than C

Two independent attempts to close the literals gap were measured and BOTH are on the
wrong side of the trade. Together they identify the real constraint.

**Attempt 1 -- repcode-1 search (brick 40, default OFF).** Ratio: net win on 9 of 12
(reymont -3.05%, x-ray -1.15%, samba -1.06%) but xml **+3.39%**, mean only ~-0.30%.
Speed: **1.5-8% SLOWER** (webster +8.1%, reymont +6.8%, samba +3.5%, xml +1.5%).
Wrong direction on both axes. Kept behind `RZSTD_REP1`, off.

**Attempt 2 -- full probe density (`RZSTD_STEP0=1`, matching C's ~1.0 probes/byte).**
Ratio: **every file improves, 3-11%, no sign-flip** -- webster -10.79% (us/c 1.436 ->
**1.281**), mozilla -10.13% (-> **1.098**), samba -9.87%, reymont -9.77%. Exactly what
the bit accountant predicted. Speed: **12.7-30.2% SLOWER** (mr +30.2%, dickens +28.2%,
webster +27.2%). Default stays 2.

### The conclusion these two share

C at level 1 probes ~1.0 positions/byte AND is ~2x faster than us. We probe 0.259/byte
and are still slower. When we raise our density to C's, our ratio approaches C's
(us/c 1.436 -> 1.281 on webster) and our speed gets 27% worse.

**So the gap is not "we search too little" and not "our entropy coder is weak" -- it is
that our PER-PROBE COST is too high, which forces us onto a worse speed/ratio curve.**
Buying ratio with density is available any time; it is just a compression level. The
only move that shifts the CURVE is making a probe cheaper.

That reframes the whole encoder campaign: brick 39 (pipelined probe, -9.8% on the
probe-dominated file) was working the right lever, and the follow-ups are the ones that
attack per-probe cost --

- **the `pair` path (`--fast=N`) still runs the serial loop** -- pipeline it too;
- **`emit_fast_seq` is ~30% of MatchFind** (~66 cyc/hit) and untouched;
- the probe's second dependent load is `src[m]` for the u32 compare -- a candidate-side
  prefetch or a wider first-compare may hide it;
- only once a probe is materially cheaper does raising density become a Pareto move
  rather than a level change.

`RZSTD_STEP0` stays in-tree as the density dial so that trade can be re-measured the
moment per-probe cost drops.

## D8' -- re-profile after 35/36/37/39, and a counted PRUNE inside MatchFind

Ranking after the landed bricks (share of encode / of decode):

- **Encode: `EncodeMatchFind` is #1 on 7 of 11** -- sao 83.6%, webster 64.5%,
  ooffice 60.7%, reymont 59.1%, dickens 58.4%, samba 57.5%, xml 52.3%. Huffman is #1
  only on mr (63.3%) and roughly tied on mozilla/osdb (~40%).
- **Decode: `DecodeSeq` is still #1 on 9 of 11** -- webster 82.8%, nci 81.5%,
  samba 79.1%, xml 81.5% -- even after bricks 36/37. `DecodeLiterals` leads only on mr.

**PRUNED on arithmetic: the backward match-extension loop in `emit_fast_seq`.** It is a
byte-at-a-time loop with two bounds-checked loads per iteration, so it looked like a
word-wise-rewrite candidate. Counted instead of built:

| file    | matches that extend |           of hits | mean bytes |
| ------- | ------------------: | ----------------: | ---------: |
| mozilla |             641,463 | 1,551,369 (41.3%) |       2.05 |
| webster |             673,011 | 1,840,362 (36.6%) |       2.20 |
| samba   |             233,691 |   741,851 (31.5%) |       2.19 |
| nci     |             190,032 | 1,015,138 (18.7%) |       1.81 |

~0.8 iterations per hit at ~6 cycles = ~5 of the ~66 cyc/hit in `emit_fast_seq`, i.e.
**~2% of MatchFind, ~1.3% of encode** -- under the noise floor even if made free.
Two minutes of counting, no code written (`codec-measurement` 11).

**Instrument hygiene, caught in the act:** the first run of this counter read **0 on
every file across millions of hits**. That was not "the extension never fires" -- the
`note_back_ext` call had never been applied (a patch script crashed before that edit).
`codec-measurement` 6: a counter reading zero for work that must be happening is a stale
instrument, not free work. Verified the call site existed before believing the number.

So `emit_fast_seq`'s ~66 cyc/hit is DIFFUSE -- literal copy (brick 38, ~5% at z=1.0),
`seqs.push`, and `fill_hash_after_match`'s 2 hashes + 2 stores -- with no single
dominant piece. Consistent with D7': the lever is per-PROBE latency, not per-hit work.

## Brick 41 (KEPT, default ON) -- tagged hash table + hit-rate dispatch

**Idea.** A probe is two DEPENDENT random loads: the hash table, then `src[m]` for the
u32 compare. Brick 39 hid the first; the second was still exposed, and the counts said
it is almost always wasted -- miss rates sao **99.1%**, mr 98.7%, ooffice 94.7%,
dickens 89.8%. Store an 8-bit tag per slot (a fold of the same hash) and reject a
candidate on tag mismatch **without ever reading `src[m]`**. Swiss-table control byte,
applied to an LZ match finder.

**Why it is byte-identical by construction:** the tag is a pure function of the 4 bytes
at a position, and `fast_probe` requires those 4 bytes to be EQUAL -- so a tag mismatch
implies the bytes differ, i.e. the tag can only reject candidates the probe would have
rejected anyway. Verified: all 12 Silesia hashes unchanged at L1 and L3.

**The prediction was WRONG and the measurement corrected it.** Estimated ~15% on
MatchFind-dominated files; the honest number is 3.6-7.1%. `src[m]` turns out to be
mostly cache-resident (matches are recent and nearby), so skipping it buys far less than
a DRAM miss would.

**Two measurement traps caught in the act:**

1. **Work-parity strawman (`codec-measurement` 4).** The first A/B gated only the tag
   READ, so the `RZSTD_TAG=0` arm still WROTE tags -- extra work no real baseline does.
   It read mr **-27.9%**, which is impossible when MatchFind is 32% of mr's encode. That
   impossible number is what exposed it. Gating the writes too collapsed mr to -2.6%.
2. **Stale tags changed the BITSTREAM.** Dispatching per block let a tag-off block stop
   writing tags; a later tag-on block then read stale tags and false-rejected real
   matches. Caught by the byte-identity gate (nci/xml/samba hashes moved). Fixed by
   LATCHING the decision for the whole frame, so the array is coherent in either state.

**Dispatch.** The tag's value scales with the MISS rate, so it sign-flipped: sao -9.6%,
ooffice -5.8%, but nci (52.8% miss) **+4.0%**. Gated on the previous block's hit rate
(`RZSTD_TAG_HR`, default 0.40), latched per frame. Final, monotone non-regression:

| file    | miss rate | TAG on | TAG off |                change |
| ------- | --------: | -----: | ------: | --------------------: |
| ooffice |     94.7% | 13.248 |  14.265 |             **-7.1%** |
| sao     |     99.1% |  6.184 |   6.549 |             **-5.6%** |
| webster |     82.9% | 13.811 |  14.437 |                 -4.3% |
| mr      |     98.7% | 16.286 |  16.892 |                 -3.6% |
| dickens |     89.8% | 17.872 |  17.915 |                 -0.2% |
| nci     |     52.8% |  6.447 |   6.490 | **-0.7%** (was +4.0%) |

Session was noisy (nci showed a 50% same-arm swing on one sample; the quoted TAG=1
figures are the cleaner repeat). **Confirm on a quiet box before quoting as standing.**

Knobs: `RZSTD_TAG` (off), `RZSTD_TAG_HR` (threshold).

## Brick 42 (REVERTED -- measured WORSE, ~2x) -- decode one-ahead + match prefetch

**Idea, and it was well-founded.** `DecodeSeq` is 62-83% of decode on 9 of 11 files.
The match copy reads from a random earlier offset -- the decoder's one unpredictable
load. C ships an entire separate code path for exactly this
(`ZSTD_decompressSequencesLong` + `ZSTD_DECODESEQUENCE_PREFETCH`), which is about as
strong a prior as this project gets. Decoding sequence i+1's symbols before executing
sequence i lets you compute where i+1's match will read from and prefetch it.

**Built and correct.** Bit-read order preserved exactly (decode i, reload+advance,
decode i+1, ...); only the copies move later. Round-trip passed on all 12 Silesia files
through our own streams AND through **C's streams** (the stricter arm), 122/122 tests.
`prefetch_read` is a pure hint, so output is byte-identical by construction.

**REVERTED. Two findings, both negative:**

1. **The prefetch itself does nothing.** With the restructure held constant in both
   arms, `RZSTD_DEC_PF=1` vs `=0` was neutral-to-harmful: webster **+16.9%**, nci +5.5%,
   mr +2.7%, xml +2.2%, mozilla -0.2%, samba -3.9%.
2. **The restructure needed to enable it costs ~2x.** Both arms measured far above the
   standing decode board: webster 2.751 -> **5.6-6.6**, nci 2.218 -> 4.5, samba
   2.403 -> 5.1, xml 2.120 -> 4.0. Reverting recovered it (webster 6.572 -> 3.275).

**This is the SAME class brick 23 already recorded** -- "the restore-tables wrapper
stopped the seq loop from inlining", nci C/us d 2.96 -> 3.63 -- and the mission's
standing note already says **do not wrap `decode_sequences`**. The macro-based
one-ahead form carries more live state across the loop (an extra decoded triple plus
three FSE entries), which is enough to spill and defeat whatever the compiler was doing
with the tight version. The lesson generalises: **on this decode loop, ANY structural
change that lengthens the live range is a ~2x tax, which no amount of latency hiding
pays back.**

Why C can do this and we cannot, as far as the evidence goes: C's long path keeps a
*ring of several* decoded sequences and is written against a hand-managed register
budget in C; our version pays Rust's optimizer losing the tight loop. Any retry should
prove the restructure is FREE (a no-prefetch restructure that matches the current board)
BEFORE adding the hint -- that ordering would have killed this in one measurement
instead of two.

`simd::prefetch_read` is retained (`#[allow(dead_code)]`): the primitive is correct and
a future attempt should not have to rewrite it.

## Brick 43 (REVERTED -- measured WORSE on 6/6) -- encoder candidate prefetch

Prefetch the NEXT probe's candidate bytes. Unlike brick 42 this needed **no
restructure** -- brick 39's pipeline already computes `m1` early, so no live range
lengthened. Byte-identical (a pure hint), 122/122 tests.

Measured on a clean box, interleaved: **worse on every file** -- samba +4.2%,
webster +3.1%, xml +2.2%, nci +1.9%, mr +1.3%, dickens +0.2%.

**This closes the prefetch line of attack, and bricks 41/42/43 together explain why.**
Brick 41 showed that SKIPPING the `src[m]` load buys only 3-7%; bricks 42 and 43 show
that PREFETCHING it costs. Both follow from one fact: **the probe's memory is already
cache-resident** (matches are recent and nearby), so there is no miss to hide. The
binding cost is the NUMBER of accesses per probe, not their latency.

That reframes the remaining encode lever: **remove an access, do not try to hide one.**
The concrete candidate is packing brick 41's tag INTO the hash entry (24-bit position +
8-bit tag, positions reconstructed modulo 2^24 since any candidate must lie within
`window` of `ip`). Today the tag lives in a separate 64 KiB array, so brick 41 trades
one random access for another -- which is exactly why it only won 3-7%. Packing removes
the added access outright. Note this is NOT byte-identical (a slot whose 24-bit residue
is 0 is indistinguishable from empty, ~1 position in 16M), so it gates on ratio, not
`cmp` -- and ratio is deterministic, so it can be gated even on a busy box.

## MEASUREMENT BLOCKED (2026-08-15)

The cumulative effect of the landed encode bricks (39 + 41) could NOT be measured. Three
repeats of each arm on three files gave same-arm spreads up to **40.7%** and cross-repeat
swings of **57-70% for identical code** (ooffice arm-on read 12.851 / 19.113 / 20.217).

Cause, named and dated per `codec-measurement` 15: **`ocr_text` running at ~17 cores**
(three processes: 11.9 + 4.3 + 1.07), **~21 foreign cores busy** in total. Not a runaway
utility -- a real user workload, so it was left alone.

Earlier in the same session, five respawned `Code.exe` NodeService processes at ~44,900
CPU-seconds each were killed (user-authorised for this recurring case); that helped, but
`ocr_text` is the dominant load and is not mine to touch.

**Consequence: the "bricks 39+41 give ~15%" question is OPEN.** One clean-ish pair
suggested only -0.4% to -4.0% combined, which would contradict the individual ABAB
results (-9.8% for brick 39 on sao, -5.6% for brick 41 on sao). Either the individual
numbers were inflated by cross-session drift, or this pair was contaminated. **Do not
quote either until a quiet box settles it** -- re-run
`RZSTD_MF_PIPE/RZSTD_TAG` on/off, >=3 pairs, requiring same-arm spread < 5%.

## D7'' -- FRONT 1: the per-probe cost measured on a SINGLE-VARIABLE instrument (2026-08-15)

The anatomy's own arithmetic did not close: 0.259 probes/byte against C's ~1.0 while
being 2x slower implies a ~7.7x per-probe cost, which is not credible for the same
five instructions. So MatchFind's time had to be decomposed, and the decomposition
needed a file where the probe loop is the ONLY thing running.

**`sao` is that instrument.** Hit rate **0.85%**, 20,014 sequences against 2,352,826
probes, match_frac 0.023 -- essentially no match extension, no literal pushes, no
back-fill. MatchFind there IS the probe loop, so one division gives the per-probe cost
with no model and no simultaneous equations.

|                                    |    probes | MatchFind | cycles/probe |
| ---------------------------------- | --------: | --------: | -----------: |
| us (profiled build)                | 2,352,826 |  24.79 ms |           37 |
| us (**de-taxed**, x0.63)           | 2,352,826 |  ~15.6 ms |      **~23** |
| C (whole encode, ~1.0 probes/byte) |   ~7.25 M |   16.8 ms |    **<=8.1** |

The de-tax factor is real and must be applied: the profiled build reports sao
EncodeTotal 29.40 ms where the clean build encodes it in 18.5 ms (x0.63). **A per-probe
number taken straight off the profiled build overstates by 60%** -- the profiler's own
tax, `codec-measurement`.

**The finding: our probe costs ~23 cycles to C's <=8, and we hide it by probing 3x less
often.** Per byte that is 0.324x23 = 7.5 vs C's 1.0x8.1 = 8.1 -- which is why sao reads
C/us 1.10 despite MatchFind being 85.5% of our encode. The two errors cancel on sao and
stop cancelling everywhere else.

### A three-file linear model was tried and REFUTED -- record it so it is not retried

Solving `probes*a + seqs*b + match_bytes*c` across sao/nci/webster returns **c = -5.4
cycles per match byte**. Negative cycles are impossible, so the per-probe cost is NOT
constant across files -- it varies with whether `src[m]` is resident, which depends on
hit rate and file size. Do not model MatchFind as a fixed cost per probe across the
corpus; measure it on a file where one term dominates.

### REFUTED: "our hash table is 4x C's and spills L2"

The profiler prints `tables hash=65536` while C reports `hashLog 14` (16,384 entries),
which reads as a 4x oversize and a per-probe L2 miss -- a clean, plausible, WRONG
story. **65536 is BYTES** (16,384 x 4). Our L1 row is `(19, 13, 14, 1, 7, 0, Fast)`
against C's `windowLog 19 / chainLog 13 / hashLog 14 / searchLog 1 / minMatch 7 /
targetLength 0 / ZSTD_fast` -- **identical in every field.** Both tables are 64 KB.
Killed by reading the units before writing the patch.

### Brick 46 -- `packed` from a runtime field to a const generic (SHIPPED, byte-identical)

Two costs were being paid on EVERY probe for the shelved, measured-slower tag arm:
`hash4_tag` computed `(hv ^ (hv >> 15)) as u8` and discarded it, and `store_fast` /
`load_fast` each branched on `self.packed` inside the hottest loop in the encoder.
`packed` is fixed at table construction, so it lifts to a const generic and
`find_fast` dispatches ONCE per block into `find_fast_impl::<true|false>`.

Gates: 172 tests pass; Silesia hashes **unchanged** (`mr 63a5b48165dd`,
`webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`,
`dickens 46b3ac4154fd`) -- a pure hoist, as designed.

**Its speed effect is BELOW OUR RESOLUTION and is not claimed as a win.** Two ALU ops
and a predictable branch out of ~23 cycles is ~1-2%, and the in-process ABBA harness
cannot resolve that; measuring it cross-process would be exactly the error this campaign
already corrected twice. It ships because it deletes dead work from the hot loop and
removes a shelved arm from the probe path, not because a number said so.

### Brick 47 -- allocate ONLY the tables the strategy reads (SHIPPED, byte-identical)

Found by following the hashLog thread past its own refutation. `MatchTables::new`
allocated `hash`, `hash_long` AND `chain` unconditionally, but **`find_fast` reads
neither of the last two, and `find_dfast` reads `hash_long` but not `chain`** (verified
against every access site: `put_hl`/`get_hl` are dfast-and-prefill only, and `get_hl`
has exactly one caller, inside `find_dfast`).

The profiler had been printing the evidence the whole time as
**`unused_long_chain=98304`** -- 96 KiB allocated per table set at L1 and never read.

Exact, deterministic footprint (entries x 4 B, no timing involved):

| input class           | level | strategy |   before |        after | saved |
| --------------------- | ----- | -------- | -------: | -----------: | ----: |
| large (TABLE_DEFAULT) | L1    | Fast     |  160 KiB |   **64 KiB** | 60.0% |
| large                 | L2    | Fast     |  640 KiB |      256 KiB | 60.0% |
| large                 | L3    | DFast    | 1280 KiB |     1024 KiB | 20.0% |
| large                 | L5    | Greedy   | 5120 KiB | **3072 KiB** | 40.0% |
| **small (TABLE_16K)** | L1    | Fast     |  320 KiB |  **128 KiB** | 60.0% |
| small                 | L3    | DFast    |  320 KiB |      256 KiB | 20.0% |

**Where it does NOT pay, stated plainly:** `vec![0; n]` lowers to `alloc_zeroed`, so the
unused tables were lazily-faulted OS zero pages that a one-shot large-file encode never
touches. **Brick 47 therefore does NOT explain the 23-vs-8 cycles/probe gap and is not
offered as a fix for it.** The measured Silesia board should not move.

**Where it does pay:**
1. **Per-entry CRDT blobs** -- the product path. A table set is built per payload, so a
   200-byte blob was committing 320 KiB of tables to compress it. Now 128 KiB.
2. **Streaming** (`stream.rs:296`) -- `reset()` is `fill(0)`, which unlike the initial
   alloc genuinely TOUCHES every page, on every window slide. It now memsets 64 KiB
   instead of 160 KiB at L1.
3. **Dictionary training** (`encode.rs:2639`) -- `reset()` per sample.

Gates: 172 tests pass; L1 Silesia hashes unchanged (`webster 49802f78f8`,
`nci 98f578b79f`, `xml c859311701`); byte-identity checked across all five strategies
(L1 Fast, L3 DFast, L5 Greedy, L7 Lazy, L13 BtOpt, L19 BtUltra2) and all six levels
round-trip to the exact source hash. Dead tables cannot reach the bitstream, so this is
byte-identical by construction -- the gates confirm the construction.

**Honest gap:** the speed win on the small-blob and streaming paths is ARGUED from the
footprint arithmetic, not measured -- our harness benchmarks one-shot whole-file encodes
only. A small-blob arm is the missing instrument, and it is the one the product actually
runs.

## FRONT 1 CRACKED OPEN -- the probe loop read in ASSEMBLY (2026-08-15)

Timing verdicts were unavailable (the box was carrying ~22 cores of unrelated work), so
front 1 was attacked with the one instrument that does not care: **the emitted
assembly**. `cargo rustc --release -p rusty_zstd --lib -- --emit asm`.

**Expectation named first** (`rusty_curiosity` step 1): C's fast probe should be ~11-14
instructions -- hash (load/imul/shr), table load, table store, `lea` for the candidate,
4-byte compare, branch, `ip` advance -- with no bounds checks and no calls. If we cost
~3x, expect either ~30-40 instructions OR the same count with spills.

**It was the spills.** One iteration of the no-match path -- the common case -- reloaded
TWELVE loop-invariant values from the stack:

| loop lines | what                                        | reloaded                             |
| ---------- | ------------------------------------------- | ------------------------------------ |
| 5          | bounds-check hash index vs `hash.len()`     | `112(%rbp)`                          |
| **16-19**  | **test `use_rep` and `rep1 != 0`**          | **`120(%rbp)`, `80(%rbp)`**          |
| 37-42      | next `ip` = `ip + step0 + ((ip-anchor)>>8)` | `48(%rbp)`, `160(%rbp)`              |
| 50-54      | hash the next position                      | `176(%rbp)`, `156(%rbp)`, `56(%rbp)` |
| 64-74      | validate candidate + 4-byte compare         | `64(%rbp)`, `328(%rbp)`, `176(%rbp)` |

src base, hash_shift, hash_mask, hash.len(), lowest, window, step0, anchor and two flags
-- **all frame-constant, all living on the stack.** C holds every one in a register. That
is where the ~3x went.

### Brick 48 -- `#[inline(never)]` on the finders. Premise REFUTED, kept for other reasons

First hypothesis: `find_sequences_strategy` had every strategy inlined into it -- 4143
instructions, 584-byte frame, **26.2% of instructions touching stack**, against
neighbouring standalone functions (`count_match` 143 instrs, `bt_find_best` 211 instrs)
that spill **0.0%**. Splitting should restore register allocation.

**It did not.** After splitting, `find_fast_impl` still spilled 24.6% and `find_lazy`
29.2%. The cause is not cross-strategy inlining -- `find_fast_impl` ALONE cannot hold ten
invariants because it contains the pipelined loop, the non-pipelined loop, the pair path,
the rep path and the tail.

**A second error, self-caught:** spill density averaged over a 1019-instruction function
says nothing about a 33-instruction hot loop. The whole-function percentage was the wrong
statistic; only the loop-level count means anything.

Kept anyway, with corrected rationale: it prevents a 4143-instruction monolith (icache,
compile time) and it is what made the hot loop findable at all. **Not a measured speed
win and not claimed as one.**

### Brick 49 -- `REP` as a const generic (the real one)

Lines 16-19 above: `use_rep` is **default-OFF and measured SLOWER** (brick 40: 0/6,
z=-2.45, sao -23.0%), yet every probe paid two stack reloads and two branches to ask
about it. It is frame-constant, so it joins `PACKED` as a const generic and the dispatch
happens ONCE per block in `find_fast`.

**Deterministic result, measured in the tightest loop containing the hash multiply.** The
binary carries all four monomorphizations, so the comparison is WITHIN ONE BINARY -- the
`REP=true` body is the old shape (and the old shape additionally paid the two runtime
flag tests that are now gone from every monomorphization):

| loop (PACKED=false)       | instructions | stack reloads |
| ------------------------- | -----------: | ------------: |
| `REP=true` (~= before)    |           47 |      20 (43%) |
| **`REP=false` (SHIPS)**   |       **33** |   **6 (18%)** |
| C's equivalent (expected) |       ~11-14 |             0 |

**-30% instructions and -70% stack traffic in the hottest loop in the encoder**, taking
us from ~3.4x C's instruction count to ~2.4x. Verified by counting emitted instructions,
so it needs no quiet box, no ABBA and no z-score.

Gates: 172 tests pass; Silesia hashes **unchanged** (`mr 63a5b48165dd`,
`webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`,
`dickens 46b3ac4154fd`).

**Not yet claimed: a speed number.** Fewer instructions and fewer loads is a strong
mechanism, but the wall-clock win must still be measured by in-process ABBA on a quiet
box before it goes on the board. The remaining SIX stack reloads in the shipping loop are
the next target.

### Bricks 50 + 51 -- hammering the remaining stack reloads (SHIPPED, byte-identical)

Continuing down the six reloads the assembly exposed. Every step measured by counting
emitted instructions, so none of it needed a quiet box.

**Brick 50 -- the bounds check the mask already proves.** `h` always arrives from
`hash4_tag` as `(hv >> hash_shift) & hash_mask`, and `hash_mask` is `hash.len() - 1` over
a non-zero power-of-two length, so `h < len` ALWAYS. LLVM could not see it because
`hash_mask` spills, and emitted both a bounds check and a reload of `hash.len()` on every
probe. `get_unchecked`/`get_unchecked_mut` behind the existing `#[allow(unsafe_code)]`
pattern, invariant documented, `debug_assert` retained. **33 -> 31 instructions, 6 -> 5
stack accesses.**

**Brick 51 -- counters feeding nothing.** `incq 112(%rbp)` was a read-modify-write to
MEMORY on every probe. Following its consumers found that `probes`/`hits` feed only
`note_search` (a no-op without the `profile` feature) -- because their other two
consumers, `last_hit_rate` and `tag_latch`, are **write-only dead state orphaned by the
brick-41 revert** (set in three places, read in none, confirmed across the whole crate).
Removed the dead fields, gated the counters and the per-block
`seqs.iter().map(..).sum()` on `const COUNT: bool = cfg!(feature = "profile")`.
**31 -> 30 instructions, 5 -> 4 stack accesses.**

| shipping probe loop                  | instructions | stack accesses |
| ------------------------------------ | -----------: | -------------: |
| before (~= the `REP=true` shape)     |           47 |       20 (43%) |
| brick 49 -- `REP` const generic      |           33 |              6 |
| brick 50 -- elide bounds check       |           31 |              5 |
| **brick 51 -- compile out counters** |       **30** |    **4 (13%)** |
| C's equivalent (expected)            |       ~11-14 |              0 |

**-36% instructions, -80% stack traffic** in the hottest loop in the encoder.

### DECLINED: removing the hash mask (it looks redundant and is NOT)

`h = (hv >> hash_shift) & hash_mask` with `hash_shift = 32 - hash_log` already yields a
value under `2^hash_log`, so the mask reads as a provable no-op worth one instruction and
one stack load. **Audited before cutting: the level tables contain a row with
`hash_log = 25`** (max over 92 rows), and the `"hashLog"` setter at `params.rs:146`
applies only a LOWER bound. A `clamp(6, 24)` at line 338 currently saves it, but with
`hash_log = 25` the shift would produce 25 bits into a `1 << 24` table -- and brick 50
now indexes that table UNCHECKED. The mask is load-bearing. Kept.

*(Brick 50 itself is unaffected: its invariant is that `h` is masked by `len - 1` over a
power-of-two length, which holds for every `hash_log`.)*

### Also fixed: `RZSTD_REP1` was an INVERTED override

`std::env::var("RZSTD_REP1").map(|v| v == "0")` -- so `RZSTD_REP1=1` turned rep1 OFF and
`=0` turned it ON. The default (absent => `false`) was correct and the ABBA harness drives
`set_rep1_arm` directly, so **no shipped verdict is contaminated** -- but an env-driven
A/B would have measured off-vs-off and read as a clean "no effect". Every other arm uses
`v != "0"`/`unwrap_or(true)` or `v == "1"`/`unwrap_or(false)` correctly; this was the only
inverted one.

Gates for all of the above: 172 tests; Silesia L1 hashes unchanged (`mr 63a5b48165dd`,
`webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`,
`dickens 46b3ac4154fd`); all six levels round-trip to the exact source hash; zero
dead-code warnings.

**STILL UNMEASURED IN WALL CLOCK.** The box has carried ~23 cores of unrelated work
throughout (`ocr_text`), so no timing verdict is admissible. Bricks 49/50/51 are
structural (compiled out, no runtime arm), so they cannot be A/B'd in-process either --
the honest measurement is a cross-process board comparison on a quiet box, and the
cumulative effect may be large enough to survive drift. **The instruction counts are
banked; the speed claim is not yet made.**

### Bricks 52 + 53 -- the last four reloads: ONE won, one reverted, two irreducible

**Brick 52 (WON) -- the mask, removed STRUCTURALLY.** Previously declined because the
invariant was not robust: the level tables contain a `hash_log = 25` row while the table
caps at `1 << 24`, and one setter path applies only a lower bound. Rather than work
around it, the invariant was MADE robust -- `MatchTables` now carries its own clamped
`hash_log` and the finder derives `hash_shift` from THAT, so the table size and the shift
can never disagree. `h = hv >> (32 - hash_log)` is then `< 1 << hash_log == hash.len()`
by construction, and `& hash_mask` is provably redundant. **30 -> 29 instructions,
4 -> 3 stack accesses.** This also closes a latent inconsistency (a 25 escaping the clamp
would have indexed 25 bits into a 24-bit table -- and brick 50 now indexes it UNCHECKED).

Follow-on dead code the change exposed and removed: `find_fast_impl` no longer reads
`params.hash_log` at all; `hash_log` params on `emit_fast_seq` and
`fill_hash_after_match`; and `tag_hit_rate_max` + `RZSTD_TAG_HR`, orphaned when the
write-only `tag_latch` went. Build is now warning-clean.

**Brick 53 (REVERTED) -- `anchor` as an incremental gap.** `anchor` is needed in the hot
path only for `(ip - anchor) >> 8`, so carrying a running `gap` should have pushed it to
the cold match path. **Measured WORSE: 29 -> 31 instructions, 3 -> 4 stack accesses.**
`gap` became its own live value and evicted `ilimit` AND `step0` -- one spill traded for
two. Reverted. *The live set was already at the register ceiling; removing a USE does not
help when the replacement is equally live.*

**REFUTED -- omitting the frame pointer.** `%rbp` serves as a frame register, so freeing
it would hand the allocator a 16th GPR. `-C force-frame-pointers=no` changed **nothing**
(29/3, still `%rbp`-relative): Windows x64 SEH pins the frame register, as the
`.seh_pushreg %rbp` directives show. Not available on this target.

**The three survivors are irreducible with this loop shape** -- src base, `hash_shift`
and `anchor` are each genuinely live on every iteration, and the remaining competitors
(ip, h0, m0, h1, m1, table ptr, ilimit) are all essential. Dropping the pipeline would
free h1/m1 but its own loop measures 46 instrs / 10 stack -- worse. Going further means a
pointer-based rewrite, which is the class brick 42 showed costs ~2x.

## FRONT 1 CAMPAIGN TOTAL (deterministic, no timing)

| shipping probe loop              | instructions | stack accesses |
| -------------------------------- | -----------: | -------------: |
| start (~= the `REP=true` shape)  |           47 |       20 (43%) |
| brick 49 -- `REP` const generic  |           33 |              6 |
| brick 50 -- elide bounds check   |           31 |              5 |
| brick 51 -- compile out counters |           30 |              4 |
| **brick 52 -- remove the mask**  |       **29** |    **3 (10%)** |
| C's equivalent (expected)        |       ~11-14 |              0 |

**-38% instructions, -85% stack traffic** in the hottest loop in the encoder, every step
byte-identical (172 tests; `mr 63a5b48165dd`, `webster 49802f78f8ca`,
`nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`, `dickens 46b3ac4154fd`;
all six levels round-trip to the exact source hash; zero warnings).

**The wall-clock win remains UNMEASURED** -- the box has carried ~23 cores of unrelated
work throughout, and bricks 49-52 are structural (no runtime arm), so they cannot be
A/B'd in-process. The honest next step is a cross-process board run on a quiet box, where
a -38%/-85% change to MatchFind (32-85% of encode by file) should clear the drift floor
that killed the 3-7% verdicts.

### Bricks 54 + 55 -- SPECIALIZE THE FRAME-CONSTANTS (the breakthrough)

The last three reloads (`anchor`, src base, `hash_shift`) all lost the same fight: too
many frame-constant values competing for 16 GPRs. The fix is not to remove a USE (brick
53 proved that fails) but to remove the VALUE -- turn it into a compile-time immediate.

**Brick 54 -- `HLOG`.** `hash_shift == 32 - hash_log`, and only five hash_log values ever
reach the Fast path across all four level tables: **{12, 13, 14, 15, 16}**. A const
generic (`HLOG == 0` = runtime fallback, so correctness never depends on that list being
complete) turns `shrl %cl, %edx` into `shrl $17` -- no register for the shift amount, no
`mov` into `%cl`. **29 -> 28 instructions, 3 -> 2 stack.**

**Brick 55 -- `STEP`.** `step0` is live in the hot advance, and the pipelined loop only
runs when `!pair`, i.e. `step0 <= 2`, so the default 2 is worth specializing.
**28 -> 19 instructions, 2 -> 1 stack** -- far bigger than expected, because a constant
step lets LLVM fold the entire advance into address arithmetic. **`anchor` fell out to a
register as a direct consequence.**

The shipping loop is now essentially C's:

```
.LBB79_87: movq %rdi,%rax / subq %r13,%rax / shrq $8,%rax / addq %rax,%rdi / addq $2,%rdi
.LBB79_88: cmpq %rsi,%rdi / ja out
           movq 120(%rbp),%rax          <-- the ONLY remaining spill: src base
           movl (%rax,%rdi),%eax / imull $-1640531535 / shrl $17
           movl (%r12,%rcx,4),%r8d / movl %edi,%edx / incl %edx / cmovel %r14d,%edx
           movl %edx,(%r12,%rcx,4) / testq %r8,%r8 / je loop
```

`anchor` in `%r13`, step folded to `addq $2`, shift folded to `shrl $17`.

**Brick 53 RE-TRIED and REFUTED AGAIN.** The `anchor`-as-running-`gap` idea failed the
first time under heavy pressure; retried after 52 and 54 freed two registers, on the
theory that the refutation was a pressure artifact. **It was not** -- 28 -> 30
instructions, 2 -> 3 stack. `gap` is exactly as live as `anchor` AND needs `adv`
alongside it. Refuted under two different register regimes; do not try a third time.

## FRONT 1 CAMPAIGN TOTAL (deterministic, no timing required)

| shipping probe loop               | instructions | stack accesses |
| --------------------------------- | -----------: | -------------: |
| start (~= the `REP=true` shape)   |           47 |       20 (43%) |
| brick 49 -- `REP` const generic   |           33 |              6 |
| brick 50 -- elide bounds check    |           31 |              5 |
| brick 51 -- compile out counters  |           30 |              4 |
| brick 52 -- remove the mask       |           29 |              3 |
| brick 54 -- specialize `HLOG`     |           28 |              2 |
| **brick 55 -- specialize `STEP`** |       **19** |     **1 (5%)** |
| C's equivalent (expected)         |       ~11-14 |              0 |

**-60% instructions, -95% stack traffic.** Of the six reloads the assembly exposed, FIVE
are closed (`hash.len()`+bounds check, the two rep flags, the probe counter, the mask,
the shift) plus `anchor` and `step0`; only **src base** survives -- one L1-hot stack slot
per probe.

Every step byte-identical: 172 tests; `mr 63a5b48165dd`, `webster 49802f78f8ca`,
`nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`, `dickens 46b3ac4154fd`;
all six levels round-trip to the source hash; the `RZSTD_STEP0=1` fallback (which
exercises the `HLOG=0/STEP=0` runtime monomorphization) round-trips correctly; build is
warning-clean. 10 monomorphizations total.

**THE WALL-CLOCK WIN IS STILL UNMEASURED** -- the box has carried ~23 cores of unrelated
work (`ocr_text`) throughout, and these bricks are structural (no runtime arm), so they
cannot be A/B'd in-process. A -60%/-95% change to MatchFind, which is 32-85% of encode by
file, is far above the drift floor that destroyed the old 3-7% verdicts, so a
cross-process board run on a quiet box should resolve it cleanly. **That run is the one
piece of work outstanding on front 1.**

## FRONT 1 -- THE WALL CLOCK (2026-08-15) -- measured, admissible, dose-responsive

Null arm **1.0355** (3.6% session floor); every same-arm spread **0.0-3.5%**. C's own
numbers came in 0.3-4.7% FASTER than the anatomy board, so nothing below is inflated by a
degraded reference.

| file    | MatchFind % of encode | us board |    us now |       gain | C drift |      NET |
| ------- | --------------------: | -------: | --------: | ---------: | ------: | -------: |
| **sao** |              **85.5** |    391.3 | **564.3** | **+44.2%** |   +2.4% | **+41%** |
| dickens |                  60.0 |    143.3 |     169.3 |     +18.1% |   +3.5% |   +14.6% |
| webster |                  65.8 |    184.9 |     217.6 |     +17.7% |   +3.2% |   +14.5% |
| xml     |                  54.1 |    361.2 |     408.3 |     +13.0% |   +0.3% |   +12.7% |
| nci     |                  48.1 |    396.1 |     447.7 |     +13.0% |   +4.7% |    +8.3% |
| mr      |                  34.1 |    153.6 |     171.7 |     +11.8% |   +1.3% |   +10.5% |

**DOSE-RESPONSE: the gain tracks MatchFind's share of encode** -- 85.5% -> +44%,
34.1% -> +12%. That is the signature of a real effect in the stage that was changed, and
it is what separates this from drift (`codec-measurement`).

**C/us compress, board -> now** (ratios `us/c size` UNCHANGED on every row -- pure speed at
matched output):

|         |      sao |  webster |  dickens |      nci |      xml |       mr |
| ------- | -------: | -------: | -------: | -------: | -------: | -------: |
| board   |     1.10 |     2.12 |     2.37 |     2.52 |     2.29 |     3.20 |
| **now** | **0.78** | **1.86** | **2.08** | **2.33** | **2.03** | **2.90** |

**`sao` compress is now 0.78 -- we are 28% FASTER than C**, from 10% slower.

### CORRECTION -- the earlier per-loop instruction counts were not one loop

Brick 59 (making `pipe_enabled()` a const generic) exposed a defect in the measurement,
not the code: the loop detector picks the SMALLEST hash-bearing loop in a function, and
while both the pipelined and non-pipelined loops were compiled into every monomorphization
it frequently selected the **non-pipelined** loop -- which is not the one that runs. The
progression reported earlier (47 -> ... -> 16) was therefore not a like-for-like series.

With `PIPE` const each monomorphization holds exactly one loop, and the arms label
themselves. Honest numbers for the **shipping (pipelined) loop**:

|   loop | stack |      fn | config                                              |
| -----: | ----: | ------: | --------------------------------------------------- |
|     43 |     9 |     459 | `REP=true`, HLOG=0, STEP=0 -- pre-campaign shape    |
|     26 |     4 |     579 | `REP=false` (brick 49)                              |
| **25** | **1** | **407** | **SHIPPING** -- `REP=false`, HLOG {12..16}, STEP=2  |
|     18 |     1 |     383 | `PIPE=false` -- the loop the detector kept grabbing |

**43 -> 25 instructions (-42%), 9 -> 1 stack accesses (-89%)** on the loop that actually
runs -- and that 43-baseline ALREADY includes bricks 50/51/52 (structural, not
arm-selectable), so the true starting point was worse.

The lesson is the skill's own: *a whole-function statistic cannot answer a hot-loop
question, and a loop detector must be checked for WHICH loop it selected.* Caught by a
brick shipped for an unrelated reason.

## FRONT: `mr` -- the literals section is encoded TWICE (2026-08-15)

`mr` is the corpus outlier and our worst compress gap (C/us **2.94**). Its anatomy is the
mirror of everything front 1 fixed: **MatchFind only 34.1%, Huffman 61.3%** of encode, and
`DecodeLiterals` **69.6%** of decode. So the match-finder work barely touches it.

**The finding, read straight out of `encode_literals_section`:** we build up to THREE
complete encodings of every literal byte and keep the shortest.

1. `write_raw_or_rle(lits, false)` -- a full copy of all literals, purely to hold a
   baseline LENGTH.
2. `try_huff_section(3, ..)` -- a complete Huffman encode against the PREVIOUS table.
3. `build_ctable` + `try_huff_section(2, ..)` -- a complete Huffman encode against a NEW
   table.

For `mr` that is **two full Huffman encodes of 6.9 MB of literals**, plus a raw copy, per
block. **C encodes once** -- `ZSTD_compressLiterals` decides from the table and a repeat
heuristic rather than by encoding both and comparing.

### Brick 60 -- defer the raw copy (SHIPPED, byte-identical, BELOW RESOLUTION)

The raw section's size is exact arithmetic (`hdr + n`, hdr in {1,2,3}), so the baseline
can be carried as a NUMBER and the bytes built only if raw actually wins.

Measured on an unusually quiet box -- null arm **1.0059** (0.6% floor), same-arm spreads
0.1-0.7%: `mr` 171.7 -> 169.3 MB/s. **No measurable change.** A ~90 KB memcpy per block is
trivial beside two Huffman encodes. Kept because it is strictly less work and
byte-identical (172 tests, all six Silesia hashes unchanged), **not claimed as a win.**
*The arithmetic said this was the small half of the finding, and the measurement agreed --
which is the useful part: it localizes the cost in the ENCODES, not the copy.*

### The actual lever, not yet pulled: encode ONCE

Two routes, with different risk:

**(a) Exact size estimation -- BYTE-IDENTICAL.** The 4-stream body size is deterministic
given `nbits` and the segment split: `6 + sum_i ceil(bits_i / 8)`, where `bits_i` is the
sum of `nbits[b]` over segment `i`. One cheap pass over the literals accumulating BOTH
candidates' bit counts (two lookups + adds per byte) replaces one full encode (lookup +
shift + or + conditional flush + store per byte). Then encode only the winner. Estimated
~35-40% off Huffman => **~20-25% off mr's total encode.** Byte-identical PROVIDED the
estimate is exact -- it must reproduce `pack_huff_section`'s header sizing and the
1-stream fallback when the 4-stream attempt fails, or the winner can flip and the
bitstream moves.

**(b) C-style repeat heuristic.** Skip building/encoding the new table when the previous
one covers and prices acceptably. Cheaper still, but it CHANGES THE BITSTREAM (we would
sometimes keep treeless where a new table was smaller), so it needs a ratio gate --
deterministic, and runnable on a busy box.

Do (a) first: it is the larger, safer win and its gate is the existing hash set.

### THE COUNT: `mr` encodes its literals 1.94x per block (2026-08-15)

`rusty_curiosity` step 1 -- a DETERMINISTIC number before building anything. The
"two full encodes per block" claim was REASONED, not measured, and the whole refactor
would have been misdirected if the prev-table branch rarely fired. Instrumented
`encode_literals_section` (profile-feature only, no-op in shipping):

| file    | blocks | prev ENCODED | new ENCODED | encodes/block |  WASTED |
| ------- | -----: | -----------: | ----------: | ------------: | ------: |
| **mr**  |     77 |       **72** |          77 |      **1.94** | **48%** |
| webster |    105 |           16 |         105 |          1.15 |     13% |
| nci     |    252 |           30 |         252 |          1.12 |     11% |
| sao     |      1 |            0 |           1 |          1.00 |      0% |

**Confirmed, and it is an `mr` PATHOLOGY.** The new table wins **74 of 77** blocks, yet we
compute and discard 72 complete Huffman encodes against the previous table. mr's literals
are homogeneous enough that `prev_ct.covers()` is almost always true, so the speculative
encode is always paid; webster/nci waste only 11-13%.

**Predicted (dose-response, to be checked against the measurement):** removing the wasted
encode is 48% of Huffman, and Huffman is 61.3% of mr's encode => **~29% of mr's total
encode**. webster/nci should move ~1% -- and that near-null control is what will
distinguish a real effect from drift.

*(The instrument itself first read all-zeros: the counter was wired into `huffman.rs` but
the PRINT was a silent no-op replace. "A counter reading zero for work that must be
happening is a stale instrument" -- caught by the skill's own law, on the skill's own
instrumentation.)*

### The implementation path is CHEAPER than last turn's estimate

`build_ctable` already walks the literals to build a frequency histogram, and **both
candidate body sizes are computable from that histogram alone** -- `sum freq[s]*nbits[s]`,
O(256), with NO second pass over the data. Better, the new table is Huffman-OPTIMAL for
these frequencies, so `body_new_bits <= body_prev_bits` **always**; prev can only win when
the tree bytes outweigh the body difference.

**The exactness wrinkle:** a 4-stream body is `6 + sum_i ceil(bits_i / 8)`, and
`sum ceil(bits_i/8) != ceil(sum bits/8)` -- up to 3 bytes apart, which is enough to flip a
winner and MOVE THE BITSTREAM. Fix: build **four segment histograms in the same single
pass** that already exists, making the estimate exact. Then encode only the winner.

Gate: the existing six Silesia hashes, plus the near-null control on webster/nci.

### BRICK 61 -- prove the speculative encode futile, then skip it (SHIPPED, byte-identical)

**The idea:** `build_ctable` produces the Huffman-OPTIMAL table for these literals, so
`body_new_bits <= body_prev_bits` **always**. The previous table can therefore only win by
saving the tree bytes. If it loses by more than the tree plus a header-slack margin, the
speculative encode is PROVABLY wasted and can be skipped without changing the outcome.

**Making it exact.** A stream is `ceil((sum nbits + 1) / 8)` bytes (`close()` appends a
1-bit end sentinel) and a 4-stream body is `6 + sum_i` over segments of `ceil(n/4)`.
Crucially `sum_i ceil(bits_i/8) != ceil(sum bits/8)` -- up to 3 bytes apart, enough to flip
the winner and MOVE THE BITSTREAM -- so `segment_histograms` builds the four per-segment
histograms in one pass and `body_bytes_exact` mirrors `encode_4_streams` exactly,
including its empty-piece and `> 65535` failure modes. A `Some` result also proves the
4-stream encode would have SUCCEEDED, so the 1-stream retry could not have run either.

The new table + tree are hoisted above the prev attempt -- **pure computation, emits
nothing** -- so the ordering (and therefore tie-breaking, where prev wins) is unchanged.
Margin `tree.len() + 8` absorbs the header and jump-table slack.

**The counts (deterministic):**

| file    | prev ENCODED before -> after | SKIPPED |    encodes/block |
| ------- | ---------------------------: | ------: | ---------------: |
| **mr**  |                  **72 -> 3** |  **69** | 1.94 -> **1.04** |
| nci     |                     30 -> 12 |      18 |     1.12 -> 1.05 |
| webster |                      16 -> 6 |      10 |     1.15 -> 1.06 |
| dickens |                       9 -> 4 |       5 |               -- |

**96% of mr's wasted Huffman encodes eliminated; it now encodes once per block, like C.**

**The wall clock.** The box degraded mid-run (C's own throughput roughly halved;
webster/nci spreads 77%/54% = inadmissible), but `C/us` is ABBA-interleaved WITHIN the run
and so survives a box-wide slowdown. Admissible rows only (spreads 0.2-0.9%):

| file    |  skips | C/us before | C/us now |     speed gain |
| ------- | -----: | ----------: | -------: | -------------: |
| **mr**  | **69** |        2.94 | **2.53** |     **+16.2%** |
| dickens |      5 |        2.10 |     2.11 | flat (CONTROL) |
| sao     |      0 |        0.78 |     0.82 | flat (CONTROL) |

**Dose-response with two correctly-null controls** -- the file with 69 skips gains 16%,
the files with 5 and 0 skips do not move. That is the signature that separates the effect
from drift.

Predicted ~28% and measured ~16%: the remaining Huffman cost is the table build,
`write_tree`, and the surviving encode, so the wasted encode was never the whole of it.
**Re-measure on a quiet box** -- this run could not resolve webster/nci at all.

Gates: 172 tests; **byte-identical on ELEVEN files** (`mr 63a5b48165dd`,
`webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`,
`dickens 46b3ac4154fd`, `ooffice 6991cd449649`, `osdb ccddecea9644`,
`reymont d2672c58dadf`, `sao 754f26b87fe7`, `mozilla f70f4a022a04`).

### BRICK 62 -- stop calling the DECODER's `resolve_offset` from the encoder (SHIPPED, byte-identical, UNRESOLVED)

`webster` and `nci` do not have mr's disease -- Huffman is only 7.9% / 11.1% there. Their
non-MatchFind cost is the SEQUENCE path: `FseSeq + SeqCode` is **22.7% (webster)** and
**34.8% (nci)**.

Inside `write_sequences`, per sequence:

```rust
let ov = offset_value_for(s.offset, s.litlen, reps);  // offset -> offset_value
let _  = resolve_offset(ov, s.litlen, reps)?;         // offset_value -> offset, DISCARDED
```

`resolve_offset` is the DECODER's function. The encoder calls it purely to advance
`reps`, but it reconstructs the offset through a branchy match plus a `Result` -- an
offset the encoder already holds in `s.offset`. That is **1.84M redundant reconstructions
on webster, 1.02M on nci.**

Replaced with a direct repcode advance. Provably the same value:
* `ov > 3` => `offset_value_for` produced `s.offset + 3`, so `ov - 3 == s.offset`
  (offsets are window-bounded, so its `saturating_add(3)` never saturates);
* `ov == 3 && litlen == 0` => that arm is only taken when `s.offset == reps[0] - 1`,
  which is exactly what it reconstructs.

The repcode SHUFFLE is `resolve_offset`'s verbatim, so the state evolution is unchanged.

**Gates PASS: 172 tests; byte-identical on all ELEVEN Silesia files; all six levels
round-trip to the source hash.**

**MEASUREMENT: UNRESOLVED, and NOT claimed.** The box degraded badly for this run --
`webster` same-arm **62.6%**, `sao` 11.3% (both inadmissible), C's own webster throughput
half the clean board (199.4 vs 403.9), null arm **1.0671**. The admissible rows
(spreads 0.3-0.4%) read mr 2.53 -> 2.45, dickens 2.10 -> 1.99, nci 2.33 -> 2.31 -- **all
below the 6.7% floor**, and webster, the actual target, could not be resolved at all.

Shipped because it is byte-identical and strictly less work per sequence, **not because a
number said so.** Re-measure webster and nci on a quiet box: that is the outstanding item,
along with brick 61's webster/nci rows (10 and 18 skipped encodes) which were also never
resolved.

## FRONT 2 OPENED -- the decoder read in assembly (2026-08-15)

`DecodeSeq` is #1 on 14/17 files and had never been read at this level. First look at
`decode_compressed_block`: **1528 instructions, 616-byte frame, 27.7% stack traffic** --
the same monolith shape `find_sequences_strategy` had before front 1, plus 5
`panic_bounds_check` and 5 `slice_index_fail` sites.

### Brick 63 -- the decoder CLONED its Huffman table every block (SHIPPED)

`compressed.rs:169`:

```rust
let (table, tree_size) = huffman::read_table(section)?;
state.huff = Some(table.clone());                        // clone into state...
decode_huff_streams(&table, &section[tree_size..], ..)   // ...decode with the original
```

The clone existed only to stash a copy in `state`; the decode used the original.
`HuffmanTable` is **two heap Vecs** (`table: Vec<u16>`, `table_x2: Vec<u32>`, each up to
`1 << max_bits` entries), so that is two allocations and up to ~12 KiB copied **per
block**, discarded immediately -- plus the matching `drop_in_place` churn visible in the
same assembly. Fixed by MOVING the table into `state` and borrowing it back.

Lands where it matters: **`DecodeLiterals` is 69.6% of mr's decode** and mr takes this arm
on nearly every block. Emitted `HuffmanTable::clone` calls: **present -> 0.**

### Brick 64 -- `SEQCHECK` as a const generic (SHIPPED)

The per-sequence guard called `seqcheck_hoisted()` -- an **atomic load** plus a match --
on EVERY sequence (1.8M times on webster). LLVM will not hoist an atomic out of a loop, so
the shipping build paid it per sequence to ask a question fixed for the whole process.
Now a const generic dispatched ONCE per block.

Emitted `SEQCHECK_ARM` references: **0 in `decode_sequences`**, 3 in
`decode_compressed_block` (the per-block dispatch). Per-sequence -> per-block.

### Honest accounting

`decode_compressed_block` reads 1528 -> 503 instructions, but **most of that is the const
generic splitting `decode_sequences` into its own symbol**, not a thousand instructions
removed -- the same bookkeeping caveat as bricks 48 and 59. The attributable wins are the
clone (per block) and the atomic (per sequence).

**New front-2 baseline: the per-sequence loop is 50 instructions with 18 stack accesses
(36%)** -- almost exactly where the encoder's probe loop began (47 instrs / 20 stack)
before specialization took it to 19/1. The same lever is available: **`ll_mode`,
`of_mode`, `ml_mode` are per-block constants in {0,1,2,3}** that drive table-kind
behaviour, exactly the `HLOG`/`STEP` pattern.

Gates: 172 tests; encoder hashes **unchanged** (these are decoder-only:
`mr 63a5b48165dd`, `webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`,
`samba f931441f5952`, `dickens 46b3ac4154fd`); **us-decodes-C 48/48** across 8 files x 6
levels; round-trip 32/32. **Wall clock not measured** -- the box is still ~22/24 cores
busy.

### Front 2, round 2: two REFUTATIONS then the win

**REFUTED before building -- specializing `ll_mode`/`of_mode`/`ml_mode`.** The obvious
analogue of `HLOG`/`STEP` looked like the three table modes. Checked first: `FseTable` is
a plain struct (not an enum), `entry()` is a masked index, and RLE is simply a 1-entry
table. **The modes are consumed at table-BUILD time and never reach the per-sequence
loop**, so specializing them buys nothing. Cost: one grep. Recorded so it is not retried.

**REFUTED by measurement -- brick 65, hoisting the decode tables.** `FseTable::entry` is
`dt[(state) & dt.len().wrapping_sub(1)]`, so each call appears to re-derive the Vec
pointer and length three times per sequence. Hoisting them to `(slice, mask)` locals
measured **exactly flat: 50 instrs / 18 stack, unchanged.** LLVM had already done it --
the `rusty-unsafe-optimizations` trap "the compiler already did it". Reverted rather than
leave code whose comment claims a benefit it does not deliver.

### BRICK 66 -- `copy_match` had 8 arguments on a 4-register ABI (SHIPPED)

Reading the loop instead of guessing showed the real cost: the call to `copy_match`.

```
movl 408(%rbp),%eax; movl %eax,72(%rsp)     movq 400(%rbp),%rax; movq %rax,64(%rsp)
movq 448(%rbp),%rax; movq %rax,40(%rsp)     movq 440(%rbp),%rax; movq %rax,32(%rsp)
```

`copy_match(out, dict, frame_start, frame_skipped, offset, matchlen, window_size,
block_max)` -- **8 arguments, and the Windows x64 ABI passes only FOUR in registers.**
Every sequence marshalled the rest onto the stack, and because they were already spilled
that was a **stack-to-stack shuffle**. **Five of the eight are frame-constant**
(`dict`, `frame_start`, `frame_skipped`, `window_size`, `block_max`).

Bundled into a `MatchCtx` built once per block, so the call is
`copy_match(out, &mctx, offset, matchlen)` -- 4 arguments, all in registers.

| per-sequence decode loop  | before |         after |
| ------------------------- | -----: | ------------: |
| instructions              |     50 | **40** (-20%) |
| `%rbp` stack accesses     |     18 | **13** (-28%) |
| argument stores to `%rsp` |      6 |  **1** (-83%) |

**This is an ABI-shaped win, not an algorithm one** -- worth remembering on Windows,
where the 4-register limit makes wide signatures expensive in a way the SysV ABI (6
registers) would partly hide.

Gates: 172 tests; **us-decodes-C 48/48**; round-trip 32/32; encoder hashes untouched.
Wall clock still unmeasured (box ~22/24 cores busy).

### BRICK 64 CAUSED A DECODE REGRESSION -- found, attributed, and fixed as 64b

The quiet-box board showed the encoder winning everywhere but **decompress DOWN 3-7% on
the sequence-heavy files** (samba -7.0%, xml -4.9%, nci -4.5%, webster -4.4%,
dickens -3.3%) while the literals-heavy files IMPROVED (mr +4.6%, ooffice +2.4%,
sao +1.9%). C's decompress was flat, so the regression was real.

**The split in the data was the diagnosis.** The files that improved are exactly brick
63's target (the per-block Huffman-table clone); the files that regressed are
sequence-heavy -- pointing at the sequence loop, i.e. brick 64 or 66.

**Attribution by A/B, not by hypothesis.** First suspect was brick 66 (`MatchCtx` passed
by reference means the callee LOADS five invariants per call). **Refuted:** normalising by
C, brick 66 is a small WIN (webster 1.92 vs 1.96, nci 2.45 vs 2.50, samba 2.09 vs 2.13,
xml 2.16 vs 2.22). Reverting brick 64 instead recovered everything (null arm 1.0000).

**The cause: the const generic SPLIT the function.** Making `decode_sequences` generic
stopped it being inlined into `decode_compressed_block`, and the call overhead, lost
cross-function optimisation and changed register allocation cost MORE than the
per-sequence atomic it removed.

**Brick 64b -- the same saving with no structural change.** A plain local
(`let seqcheck = seqcheck_hoisted();` before the loop) hoists the atomic out without
touching the function's shape. Strictly better than either alternative:

| C/us decompress | brick 64 (generic) | 64 removed | **64b (local hoist)** |
| --------------- | -----------------: | ---------: | --------------------: |
| webster         |               1.92 |       1.80 |              **1.80** |
| nci             |               2.45 |       2.37 |              **2.33** |
| samba           |               2.09 |       2.01 |              **1.99** |
| xml             |               2.16 |       2.10 |              **2.08** |
| mr              |               1.41 |       1.40 |              **1.34** |

Against the ORIGINAL board, decompress is now ahead on 4 of 5: webster +4.3%, nci +5.3%,
xml +1.0%, **mr +7.5%** (brick 63 landing exactly where predicted), samba -0.3%.

Gates: 172 tests; encoder hashes unchanged; **us-decodes-C 48/48**; round-trip 32/32.

**LESSON: a const generic is not free.** Every previous use of it in this campaign
(`PACKED`, `REP`, `HLOG`, `STEP`, `PIPE`) specialised a function that was ALREADY its own
symbol, so the only effect was folding constants. Applying the same tool to a function
that was being INLINED changes its linkage, and that structural side-effect can dominate
the win. **Check whether the target is currently inlined before reaching for a const
generic** -- and prefer a plain local when all you need is to hoist a loop-invariant read.

## MOZILLA -- the audit for dead code, silent zeros and poor gating (2026-08-16)

`mozilla` is now our **worst compress ratio on the board (C/us 2.70)**, and is
MatchFind-heavy on encode (43.4%) and DecSeq-heavy on decode (62.1%). Audited it the way
`rusty_curiosity` says to: counts first, and chase what reads impossible.

### 1. SILENT ZERO -> DEAD GATING: `early_raw` is 0 on all 18 corpora

`incomp_skip_on` requires `strategy == Fast && 1 <= target_length <= 7`. **Level 1 has
`target_length = 0`** (row `(19, 13, 14, 1, 7, 0, Fast)`), so the predicate is FALSE at
every normal level -- it is only true for `--fast=N`. Consequences:

* **`early_raw_skip` is dead code at every shipping level**, which is exactly why its
  counter reads zero on all 18 corpora.
* `raw_limit` loses its margin: `if incomp_skip_on { block.len() - mg } else { block.len() }`.
  At L1 we only fall back to Raw when compression makes the block **bigger or equal**,
  whereas C bails whenever the gain is under `minGain`.

Measured consequence on mozilla: **C emits 5 RAW blocks, we emit 0.** Small in itself
(5 of 609 blocks) -- the value is that a documented "depth-1 skip tree" brick is not
running anywhere in the shipping configuration, and its counter said so all along.

### 2. STALE INSTRUMENT (FIXED): `unused_long_chain=98304` after brick 47 deleted it

`note_tables(hash_b, hash_b, chain_b)` reported all three tables at level-table size
regardless of what was ALLOCATED, so it kept printing 96 KiB of `hash_long` + `chain`
that **brick 47 removed two bricks earlier**. Anyone reading the profile would conclude
brick 47 never shipped. Now reports actual allocations. Gated: 172 tests, hashes
unchanged.

### 3. REFUTED: the "unused BMI2 bit extractor"

A dead-function sweep flagged `has_bmi2` / `look_n_bits` / `look_n_bits_bmi2` -- a
complete, tested `BIT_lookBitsFast` implementation that nothing calls, sitting in the bit
reader at the heart of DecSeq. **It is `#[cfg(test)]` on purpose.** `BitRev` keeps its
container LEFT-JUSTIFIED, so the hot peek is already a single shift
(`container >> (64 - n)`); BMI2's `_bextr_u64` would need start/len setup to do the same
work. The BMI2 path is SUPERSEDED, not unused, and the comment in `simd.rs` says so.

**The sweep that found it was too noisy to trust** (200 "dead" functions, mostly `#[test]`
fns and public API called from the CLI). A dead-code sweep that cannot distinguish
test-only oracles from production code produces exactly this kind of false lead.

### 4. THE OPEN LEAD: C emits 609 blocks where we emit 391

Same 51.2 MB input. Ours is 391 x 128 KiB -- maximum-size blocks throughout. **C emits
604 compressed + 5 raw = 609**, averaging ~84 KiB, i.e. it is SPLITTING blocks we keep
whole. C therefore re-adapts its entropy tables ~1.5x more often, and the bit accountant
shows our gap is **entirely literals**: `lit_gap = +6,057,465 bits (757 KiB)` while
`seq_gap = -1,104,053` (we spend FEWER sequence bits).

That is the ratio story for mozilla, and it is a block-splitting question, not a
match-finder one. **Not yet investigated** -- it needs C's splitter behaviour at L1
confirmed before any work is planned.

### MOZILLA: block splitting REFUTED, probe density CONFIRMED (2026-08-16)

**Units check first** (`rusty_curiosity` step 2). The block-header size field is the
COMPRESSED payload, not the regen size, so the size histograms cannot show "C splits
blocks" directly. The arithmetic still forces it: total regen is fixed at 51,220,480 for
both, and 609 blocks CANNOT each hold 128 KiB (that regenerates 79.8 MB). So C really does
average **~84 KiB regen per block** against our ~131 KiB.

**Then the cheap test instead of the expensive build.** Rather than implement a block
splitter, add one env knob and sweep block size. Ratio is deterministic, so this needs no
quiet box:

| mozilla block size |   us/c |
| ------------------ | -----: |
| 128 KiB (current)  | 1.2216 |
| 64 KiB             | 1.2137 |
| 32 KiB             | 1.2131 |
| 16 KiB             | 1.2120 |

**REFUTED: 0.8% across a 8x block-size range, against a 22% gap.** Block splitting is not
what makes mozilla our worst file. Cost of finding out: one knob, not a splitter.

**CONFIRMED, at the neighbouring site: probe density.**

| mozilla                          |       us/c | share of gap closed |
| -------------------------------- | ---------: | ------------------: |
| STEP0=2 (default)                |     1.2216 |                  -- |
| 16 KiB blocks alone              |     1.2120 |                  4% |
| **STEP0=1 (C-matching density)** | **1.0979** |             **56%** |
| both                             |     1.0894 |                 60% |

We probe **0.249 positions/byte to C's ~1.0**, find fewer matches, and pay the difference
in literals -- exactly what the bit accountant said, now demonstrated directly.

### The density trade, priced DETERMINISTICALLY (no clock)

Density was last priced at "3-11% ratio for 13-30% speed" -- measured when the probe cost
47 instructions. It now costs **19**, so the trade needed re-pricing.

**A first attempt used wall-clock C/us and is WITHDRAWN.** The machine was thermally
unstable across those runs -- C's own throughput moved **30%** between them (webster C
301.7 vs 390.9) -- which makes a CROSS-RUN speed comparison inadmissible, not merely
caveated. Recording that rather than quietly keeping the numbers.

**Priced on deterministic work counts instead.** `STEP0` changes probe COUNT by an exact,
countable amount, and the ratio column is exact byte counts:

| file        | probes           |       work | literals         | ratio gain | MatchFind share | est. encode cost |
| ----------- | ---------------- | ---------: | ---------------- | ---------: | --------------: | ---------------: |
| **nci**     | 2.15M -> 2.56M   | **+19.1%** | 1.93M -> 1.27M   |   **5.9%** |           43.9% |         **~+8%** |
| **mozilla** | 12.77M -> 18.94M |     +48.3% | 24.40M -> 19.32M |  **10.2%** |           43.4% |            ~+21% |
| webster     | 10.76M -> 16.52M |     +53.6% | 16.35M -> 13.11M |      10.8% |           62.6% |            ~+34% |

Same RANKING the drifted timing suggested, from numbers that cannot drift. **`nci` is the
standout** -- +19% probe work for 5.9% ratio. `webster` is worst on both axes: the most
extra work AND the largest MatchFind share to multiply it by. That share column is why
mozilla out-trades webster despite near-identical probe growth.

*(`est. encode cost` = probe growth x MatchFind share. It OVERSTATES, because MatchFind is
not purely probes -- match extension grows too but sub-linearly. Treat it as an upper
bound and a ranking, not a speed prediction.)*

**The content SPLIT is the finding, not a new default.** The trade is favourable on nci and
mozilla and poor on webster -- a sign-flip across content, which
`codec-content-adaptive-dispatch` says is a DISPATCH trigger, not a global knob. Any such
dispatch changes the bitstream, so it needs a per-file ratio gate plus a speed verdict
taken on a THERMALLY STABLE box. **Not shipped.**

### BRICK 61 -- prove the speculative encode futile, then skip it (SHIPPED, byte-identical)

**The idea:** `build_ctable` produces the Huffman-OPTIMAL table for these literals, so
`body_new_bits <= body_prev_bits` **always**. The previous table can therefore only win by
saving the tree bytes. If it loses by more than the tree plus a header-slack margin, the
speculative encode is PROVABLY wasted and can be skipped without changing the outcome.

**Making it exact.** A stream is `ceil((sum nbits + 1) / 8)` bytes (`close()` appends a
1-bit end sentinel) and a 4-stream body is `6 + sum_i` over segments of `ceil(n/4)`.
Crucially `sum_i ceil(bits_i/8) != ceil(sum bits/8)` -- up to 3 bytes apart, enough to flip
the winner and MOVE THE BITSTREAM -- so `segment_histograms` builds the four per-segment
histograms in one pass and `body_bytes_exact` mirrors `encode_4_streams` exactly,
including its empty-piece and `> 65535` failure modes. A `Some` result also proves the
4-stream encode would have SUCCEEDED, so the 1-stream retry could not have run either.

The new table + tree are hoisted above the prev attempt -- **pure computation, emits
nothing** -- so the ordering (and therefore tie-breaking, where prev wins) is unchanged.
Margin `tree.len() + 8` absorbs the header and jump-table slack.

**The counts (deterministic):**

| file    | prev ENCODED before -> after | SKIPPED |    encodes/block |
| ------- | ---------------------------: | ------: | ---------------: |
| **mr**  |                  **72 -> 3** |  **69** | 1.94 -> **1.04** |
| nci     |                     30 -> 12 |      18 |     1.12 -> 1.05 |
| webster |                      16 -> 6 |      10 |     1.15 -> 1.06 |
| dickens |                       9 -> 4 |       5 |               -- |

**96% of mr's wasted Huffman encodes eliminated; it now encodes once per block, like C.**

**The wall clock.** The box degraded mid-run (C's own throughput roughly halved;
webster/nci spreads 77%/54% = inadmissible), but `C/us` is ABBA-interleaved WITHIN the run
and so survives a box-wide slowdown. Admissible rows only (spreads 0.2-0.9%):

| file    |  skips | C/us before | C/us now |     speed gain |
| ------- | -----: | ----------: | -------: | -------------: |
| **mr**  | **69** |        2.94 | **2.53** |     **+16.2%** |
| dickens |      5 |        2.10 |     2.11 | flat (CONTROL) |
| sao     |      0 |        0.78 |     0.82 | flat (CONTROL) |

**Dose-response with two correctly-null controls** -- the file with 69 skips gains 16%,
the files with 5 and 0 skips do not move. That is the signature that separates the effect
from drift.

Predicted ~28% and measured ~16%: the remaining Huffman cost is the table build,
`write_tree`, and the surviving encode, so the wasted encode was never the whole of it.
**Re-measure on a quiet box** -- this run could not resolve webster/nci at all.

Gates: 172 tests; **byte-identical on ELEVEN files** (`mr 63a5b48165dd`,
`webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`, `samba f931441f5952`,
`dickens 46b3ac4154fd`, `ooffice 6991cd449649`, `osdb ccddecea9644`,
`reymont d2672c58dadf`, `sao 754f26b87fe7`, `mozilla f70f4a022a04`).

### BRICK 62 -- stop calling the DECODER's `resolve_offset` from the encoder (SHIPPED, byte-identical, UNRESOLVED)

`webster` and `nci` do not have mr's disease -- Huffman is only 7.9% / 11.1% there. Their
non-MatchFind cost is the SEQUENCE path: `FseSeq + SeqCode` is **22.7% (webster)** and
**34.8% (nci)**.

Inside `write_sequences`, per sequence:

```rust
let ov = offset_value_for(s.offset, s.litlen, reps);  // offset -> offset_value
let _  = resolve_offset(ov, s.litlen, reps)?;         // offset_value -> offset, DISCARDED
```

`resolve_offset` is the DECODER's function. The encoder calls it purely to advance
`reps`, but it reconstructs the offset through a branchy match plus a `Result` -- an
offset the encoder already holds in `s.offset`. That is **1.84M redundant reconstructions
on webster, 1.02M on nci.**

Replaced with a direct repcode advance. Provably the same value:
* `ov > 3` => `offset_value_for` produced `s.offset + 3`, so `ov - 3 == s.offset`
  (offsets are window-bounded, so its `saturating_add(3)` never saturates);
* `ov == 3 && litlen == 0` => that arm is only taken when `s.offset == reps[0] - 1`,
  which is exactly what it reconstructs.

The repcode SHUFFLE is `resolve_offset`'s verbatim, so the state evolution is unchanged.

**Gates PASS: 172 tests; byte-identical on all ELEVEN Silesia files; all six levels
round-trip to the source hash.**

**MEASUREMENT: UNRESOLVED, and NOT claimed.** The box degraded badly for this run --
`webster` same-arm **62.6%**, `sao` 11.3% (both inadmissible), C's own webster throughput
half the clean board (199.4 vs 403.9), null arm **1.0671**. The admissible rows
(spreads 0.3-0.4%) read mr 2.53 -> 2.45, dickens 2.10 -> 1.99, nci 2.33 -> 2.31 -- **all
below the 6.7% floor**, and webster, the actual target, could not be resolved at all.

Shipped because it is byte-identical and strictly less work per sequence, **not because a
number said so.** Re-measure webster and nci on a quiet box: that is the outstanding item,
along with brick 61's webster/nci rows (10 and 18 skipped encodes) which were also never
resolved.

## FRONT 2 OPENED -- the decoder read in assembly (2026-08-15)

`DecodeSeq` is #1 on 14/17 files and had never been read at this level. First look at
`decode_compressed_block`: **1528 instructions, 616-byte frame, 27.7% stack traffic** --
the same monolith shape `find_sequences_strategy` had before front 1, plus 5
`panic_bounds_check` and 5 `slice_index_fail` sites.

### Brick 63 -- the decoder CLONED its Huffman table every block (SHIPPED)

`compressed.rs:169`:

```rust
let (table, tree_size) = huffman::read_table(section)?;
state.huff = Some(table.clone());                        // clone into state...
decode_huff_streams(&table, &section[tree_size..], ..)   // ...decode with the original
```

The clone existed only to stash a copy in `state`; the decode used the original.
`HuffmanTable` is **two heap Vecs** (`table: Vec<u16>`, `table_x2: Vec<u32>`, each up to
`1 << max_bits` entries), so that is two allocations and up to ~12 KiB copied **per
block**, discarded immediately -- plus the matching `drop_in_place` churn visible in the
same assembly. Fixed by MOVING the table into `state` and borrowing it back.

Lands where it matters: **`DecodeLiterals` is 69.6% of mr's decode** and mr takes this arm
on nearly every block. Emitted `HuffmanTable::clone` calls: **present -> 0.**

### Brick 64 -- `SEQCHECK` as a const generic (SHIPPED)

The per-sequence guard called `seqcheck_hoisted()` -- an **atomic load** plus a match --
on EVERY sequence (1.8M times on webster). LLVM will not hoist an atomic out of a loop, so
the shipping build paid it per sequence to ask a question fixed for the whole process.
Now a const generic dispatched ONCE per block.

Emitted `SEQCHECK_ARM` references: **0 in `decode_sequences`**, 3 in
`decode_compressed_block` (the per-block dispatch). Per-sequence -> per-block.

### Honest accounting

`decode_compressed_block` reads 1528 -> 503 instructions, but **most of that is the const
generic splitting `decode_sequences` into its own symbol**, not a thousand instructions
removed -- the same bookkeeping caveat as bricks 48 and 59. The attributable wins are the
clone (per block) and the atomic (per sequence).

**New front-2 baseline: the per-sequence loop is 50 instructions with 18 stack accesses
(36%)** -- almost exactly where the encoder's probe loop began (47 instrs / 20 stack)
before specialization took it to 19/1. The same lever is available: **`ll_mode`,
`of_mode`, `ml_mode` are per-block constants in {0,1,2,3}** that drive table-kind
behaviour, exactly the `HLOG`/`STEP` pattern.

Gates: 172 tests; encoder hashes **unchanged** (these are decoder-only:
`mr 63a5b48165dd`, `webster 49802f78f8ca`, `nci 98f578b79ffa`, `xml c85931170147`,
`samba f931441f5952`, `dickens 46b3ac4154fd`); **us-decodes-C 48/48** across 8 files x 6
levels; round-trip 32/32. **Wall clock not measured** -- the box is still ~22/24 cores
busy.

### Front 2, round 2: two REFUTATIONS then the win

**REFUTED before building -- specializing `ll_mode`/`of_mode`/`ml_mode`.** The obvious
analogue of `HLOG`/`STEP` looked like the three table modes. Checked first: `FseTable` is
a plain struct (not an enum), `entry()` is a masked index, and RLE is simply a 1-entry
table. **The modes are consumed at table-BUILD time and never reach the per-sequence
loop**, so specializing them buys nothing. Cost: one grep. Recorded so it is not retried.

**REFUTED by measurement -- brick 65, hoisting the decode tables.** `FseTable::entry` is
`dt[(state) & dt.len().wrapping_sub(1)]`, so each call appears to re-derive the Vec
pointer and length three times per sequence. Hoisting them to `(slice, mask)` locals
measured **exactly flat: 50 instrs / 18 stack, unchanged.** LLVM had already done it --
the `rusty-unsafe-optimizations` trap "the compiler already did it". Reverted rather than
leave code whose comment claims a benefit it does not deliver.

### BRICK 66 -- `copy_match` had 8 arguments on a 4-register ABI (SHIPPED)

Reading the loop instead of guessing showed the real cost: the call to `copy_match`.

```
movl 408(%rbp),%eax; movl %eax,72(%rsp)     movq 400(%rbp),%rax; movq %rax,64(%rsp)
movq 448(%rbp),%rax; movq %rax,40(%rsp)     movq 440(%rbp),%rax; movq %rax,32(%rsp)
```

`copy_match(out, dict, frame_start, frame_skipped, offset, matchlen, window_size,
block_max)` -- **8 arguments, and the Windows x64 ABI passes only FOUR in registers.**
Every sequence marshalled the rest onto the stack, and because they were already spilled
that was a **stack-to-stack shuffle**. **Five of the eight are frame-constant**
(`dict`, `frame_start`, `frame_skipped`, `window_size`, `block_max`).

Bundled into a `MatchCtx` built once per block, so the call is
`copy_match(out, &mctx, offset, matchlen)` -- 4 arguments, all in registers.

| per-sequence decode loop  | before |         after |
| ------------------------- | -----: | ------------: |
| instructions              |     50 | **40** (-20%) |
| `%rbp` stack accesses     |     18 | **13** (-28%) |
| argument stores to `%rsp` |      6 |  **1** (-83%) |

**This is an ABI-shaped win, not an algorithm one** -- worth remembering on Windows,
where the 4-register limit makes wide signatures expensive in a way the SysV ABI (6
registers) would partly hide.

Gates: 172 tests; **us-decodes-C 48/48**; round-trip 32/32; encoder hashes untouched.
Wall clock still unmeasured (box ~22/24 cores busy).

### BRICK 64 CAUSED A DECODE REGRESSION -- found, attributed, and fixed as 64b

The quiet-box board showed the encoder winning everywhere but **decompress DOWN 3-7% on
the sequence-heavy files** (samba -7.0%, xml -4.9%, nci -4.5%, webster -4.4%,
dickens -3.3%) while the literals-heavy files IMPROVED (mr +4.6%, ooffice +2.4%,
sao +1.9%). C's decompress was flat, so the regression was real.

**The split in the data was the diagnosis.** The files that improved are exactly brick
63's target (the per-block Huffman-table clone); the files that regressed are
sequence-heavy -- pointing at the sequence loop, i.e. brick 64 or 66.

**Attribution by A/B, not by hypothesis.** First suspect was brick 66 (`MatchCtx` passed
by reference means the callee LOADS five invariants per call). **Refuted:** normalising by
C, brick 66 is a small WIN (webster 1.92 vs 1.96, nci 2.45 vs 2.50, samba 2.09 vs 2.13,
xml 2.16 vs 2.22). Reverting brick 64 instead recovered everything (null arm 1.0000).

**The cause: the const generic SPLIT the function.** Making `decode_sequences` generic
stopped it being inlined into `decode_compressed_block`, and the call overhead, lost
cross-function optimisation and changed register allocation cost MORE than the
per-sequence atomic it removed.

**Brick 64b -- the same saving with no structural change.** A plain local
(`let seqcheck = seqcheck_hoisted();` before the loop) hoists the atomic out without
touching the function's shape. Strictly better than either alternative:

| C/us decompress | brick 64 (generic) | 64 removed | **64b (local hoist)** |
| --------------- | -----------------: | ---------: | --------------------: |
| webster         |               1.92 |       1.80 |              **1.80** |
| nci             |               2.45 |       2.37 |              **2.33** |
| samba           |               2.09 |       2.01 |              **1.99** |
| xml             |               2.16 |       2.10 |              **2.08** |
| mr              |               1.41 |       1.40 |              **1.34** |

Against the ORIGINAL board, decompress is now ahead on 4 of 5: webster +4.3%, nci +5.3%,
xml +1.0%, **mr +7.5%** (brick 63 landing exactly where predicted), samba -0.3%.

Gates: 172 tests; encoder hashes unchanged; **us-decodes-C 48/48**; round-trip 32/32.

**LESSON: a const generic is not free.** Every previous use of it in this campaign
(`PACKED`, `REP`, `HLOG`, `STEP`, `PIPE`) specialised a function that was ALREADY its own
symbol, so the only effect was folding constants. Applying the same tool to a function
that was being INLINED changes its linkage, and that structural side-effect can dominate
the win. **Check whether the target is currently inlined before reaching for a const
generic** -- and prefer a plain local when all you need is to hoist a loop-invariant read.

## MOZILLA -- the audit for dead code, silent zeros and poor gating (2026-08-16)

`mozilla` is now our **worst compress ratio on the board (C/us 2.70)**, and is
MatchFind-heavy on encode (43.4%) and DecSeq-heavy on decode (62.1%). Audited it the way
`rusty_curiosity` says to: counts first, and chase what reads impossible.

### 1. SILENT ZERO -> DEAD GATING: `early_raw` is 0 on all 18 corpora

`incomp_skip_on` requires `strategy == Fast && 1 <= target_length <= 7`. **Level 1 has
`target_length = 0`** (row `(19, 13, 14, 1, 7, 0, Fast)`), so the predicate is FALSE at
every normal level -- it is only true for `--fast=N`. Consequences:

* **`early_raw_skip` is dead code at every shipping level**, which is exactly why its
  counter reads zero on all 18 corpora.
* `raw_limit` loses its margin: `if incomp_skip_on { block.len() - mg } else { block.len() }`.
  At L1 we only fall back to Raw when compression makes the block **bigger or equal**,
  whereas C bails whenever the gain is under `minGain`.

Measured consequence on mozilla: **C emits 5 RAW blocks, we emit 0.** Small in itself
(5 of 609 blocks) -- the value is that a documented "depth-1 skip tree" brick is not
running anywhere in the shipping configuration, and its counter said so all along.

### 2. STALE INSTRUMENT (FIXED): `unused_long_chain=98304` after brick 47 deleted it

`note_tables(hash_b, hash_b, chain_b)` reported all three tables at level-table size
regardless of what was ALLOCATED, so it kept printing 96 KiB of `hash_long` + `chain`
that **brick 47 removed two bricks earlier**. Anyone reading the profile would conclude
brick 47 never shipped. Now reports actual allocations. Gated: 172 tests, hashes
unchanged.

### 3. REFUTED: the "unused BMI2 bit extractor"

A dead-function sweep flagged `has_bmi2` / `look_n_bits` / `look_n_bits_bmi2` -- a
complete, tested `BIT_lookBitsFast` implementation that nothing calls, sitting in the bit
reader at the heart of DecSeq. **It is `#[cfg(test)]` on purpose.** `BitRev` keeps its
container LEFT-JUSTIFIED, so the hot peek is already a single shift
(`container >> (64 - n)`); BMI2's `_bextr_u64` would need start/len setup to do the same
work. The BMI2 path is SUPERSEDED, not unused, and the comment in `simd.rs` says so.

**The sweep that found it was too noisy to trust** (200 "dead" functions, mostly `#[test]`
fns and public API called from the CLI). A dead-code sweep that cannot distinguish
test-only oracles from production code produces exactly this kind of false lead.

### 4. THE OPEN LEAD: C emits 609 blocks where we emit 391

Same 51.2 MB input. Ours is 391 x 128 KiB -- maximum-size blocks throughout. **C emits
604 compressed + 5 raw = 609**, averaging ~84 KiB, i.e. it is SPLITTING blocks we keep
whole. C therefore re-adapts its entropy tables ~1.5x more often, and the bit accountant
shows our gap is **entirely literals**: `lit_gap = +6,057,465 bits (757 KiB)` while
`seq_gap = -1,104,053` (we spend FEWER sequence bits).

That is the ratio story for mozilla, and it is a block-splitting question, not a
match-finder one. **Not yet investigated** -- it needs C's splitter behaviour at L1
confirmed before any work is planned.

### MOZILLA: block splitting REFUTED, probe density CONFIRMED (2026-08-16)

**Units check first** (`rusty_curiosity` step 2). The block-header size field is the
COMPRESSED payload, not the regen size, so the size histograms cannot show "C splits
blocks" directly. The arithmetic still forces it: total regen is fixed at 51,220,480 for
both, and 609 blocks CANNOT each hold 128 KiB (that regenerates 79.8 MB). So C really does
average **~84 KiB regen per block** against our ~131 KiB.

**Then the cheap test instead of the expensive build.** Rather than implement a block
splitter, add one env knob and sweep block size. Ratio is deterministic, so this needs no
quiet box:

| mozilla block size |   us/c |
| ------------------ | -----: |
| 128 KiB (current)  | 1.2216 |
| 64 KiB             | 1.2137 |
| 32 KiB             | 1.2131 |
| 16 KiB             | 1.2120 |

**REFUTED: 0.8% across a 8x block-size range, against a 22% gap.** Block splitting is not
what makes mozilla our worst file. Cost of finding out: one knob, not a splitter.

**CONFIRMED, at the neighbouring site: probe density.**

| mozilla                          |       us/c | share of gap closed |
| -------------------------------- | ---------: | ------------------: |
| STEP0=2 (default)                |     1.2216 |                  -- |
| 16 KiB blocks alone              |     1.2120 |                  4% |
| **STEP0=1 (C-matching density)** | **1.0979** |             **56%** |
| both                             |     1.0894 |                 60% |

We probe **0.249 positions/byte to C's ~1.0**, find fewer matches, and pay the difference
in literals -- exactly what the bit accountant said, now demonstrated directly.

### The density trade has been REPRICED by the campaign

Density was last measured at "3-11% ratio for 13-30% speed" -- when the probe cost 47
instructions. It now costs **19**. Re-measured:

| file        | ratio gain (EXACT) | speed cost (C/us, cross-run) | trade               |
| ----------- | -----------------: | ---------------------------: | ------------------- |
| **mozilla** |          **10.2%** |                         7.5% | **better than 1:1** |
| **nci**     |               5.9% |                         4.8% | **better than 1:1** |
| webster     |              10.8% |                          26% | poor                |

**Caveat on the speed column:** C's own throughput drifted 30% between the two runs
(webster C 301.7 vs 390.9), so the box moved. `C/us` is ABBA-interleaved WITHIN each run
and is the more robust figure, but comparing it ACROSS runs still carries risk. The ratio
column is exact byte counts and carries none.

**The content SPLIT is the finding, not a new default.** webster stays a bad trade while
mozilla and nci turn favourable -- a sign-flip across content, which
`codec-content-adaptive-dispatch` says is a DISPATCH trigger, not a global knob. Any such
dispatch changes the bitstream, so it needs a ratio gate per file plus a re-measured speed
verdict on a quiet box. **Not shipped.**

### THE DENSITY TRUTH TABLE -- and why it is NOT a dispatch (2026-08-16)

`codec-content-adaptive-dispatch` says get the per-clip TRUTH TABLE before designing a
signal. Built it across all 18 corpora, entirely deterministically (exact byte counts for
the return, exact probe counts for the cost -- no clock, valid on any box).

`trade` = ratio gain % per work-increase %.

| corpus       | ratio+ |      work+ |    TRADE | hit_rate | match_frac |
| ------------ | -----: | ---------: | -------: | -------: | ---------: |
| text-32m     |  1.28% |       0.0% |      inf |    0.818 |      1.000 |
| versions-16m |  5.89% |  **12.7%** | **0.47** |    0.576 |      0.987 |
| nci          |  5.94% |      19.1% | **0.31** |    0.472 |      0.943 |
| samba        |  9.87% |      42.7% |     0.23 |    0.219 |      0.732 |
| reymont      |  9.77% |      46.5% |     0.21 |    0.178 |      0.575 |
| mozilla      | 10.13% |      48.3% |     0.21 |    0.121 |      0.524 |
| webster      | 10.79% |      53.6% |     0.20 |    0.171 |      0.606 |
| xml          |  5.05% |      39.8% |     0.13 |    0.321 |      0.886 |
| dickens      |  6.26% |      61.2% |     0.10 |    0.102 |      0.364 |
| ooffice      |  5.76% |      78.8% |     0.07 |    0.053 |      0.279 |
| osdb         |  3.51% |      65.6% |     0.05 |    0.148 |      0.628 |
| jsonlog-16m  |  2.71% |      57.8% |     0.05 |    0.278 |      0.758 |
| x-ray        |  1.87% |      47.1% |     0.04 |    0.000 |      0.000 |
| smallmsg-8m  |  2.46% |      66.3% |     0.04 |    0.227 |      0.636 |
| mr           |  3.18% |      87.5% |     0.04 |    0.013 |      0.309 |
| sao          |  1.91% | **132.4%** | **0.01** |    0.009 |      0.023 |
| incomp-32m   |  0.00% |      17.2% |     0.00 |    0.000 |      0.000 |

### VERDICT: PRUNE. This is a LEVEL knob, not a dispatch axis.

1. **There is NO SIGN-FLIP.** Every corpus gains ratio and pays work -- a monotonic
   continuum from 0.47 to 0.01. The skill's trigger is a sign-flip; a monotone trade-off
   is a compression LEVEL, which is what the anatomy doc already calls `STEP0`.
2. **Neither candidate signal predicts the TRADE.** `hit_rate` and `match_frac` both nail
   the top three and then break: `xml` has the 4th-highest hit_rate (0.321) but ranks 8th
   on trade; `jsonlog` (0.278) ranks 12th; `smallmsg` (0.227) ranks 14th. A signal that
   predicts "activity" but not the OUTCOME is the exact failure the skill warns about.
3. **Only half the equation is predictable.** `match_frac` predicts the COST cleanly --
   high match fraction means most bytes sit inside matches we skip anyway, so denser
   probing adds little work (`versions` 0.987 -> +12.7%; `sao` 0.023 -> +132.4%). **Nothing
   predicts the RETURN**: ratio gain spans 1.87-10.79% unrelated to either signal.

So a gate can reliably AVOID expensive cases but cannot SELECT profitable ones.

**The one defensible term the data supports** is one-sided and cost-based: do not spend
extra probes when `match_frac` is very low -- `sao` (132% work for 1.91%), `x-ray` (47%
for 1.87%), `incomp` (17% for 0%). That is justified by clips it demonstrably classifies,
which is the skill's bar. Anything richer would be fitting noise.

**Recorded as a PRUNE with its measurements**, per the skill: "either you found the signal
that turns the loser non-negative, or you PROVED no cheap signal separates them -- then
PRUNE, recording what you measured."

*(Note: `last_hit_rate` was removed in brick 51 as write-only dead state. If a density
dispatch is ever revisited, that is the field it would need -- removing it was still
correct, since nothing read it.)*

### IDEA 1 REFUTED: the K-rung ladder is bypassed; `emit_fill` is the real hot path

**Hypothesis:** mozilla's literals are near-incompressible (24.40 MB of literals -> a
20.98 MB section = 6.88 bits/byte), which implies `max_nbits >= 10` and therefore the
SLOWEST rung `_ => emit_k5` (5 symbols per flush vs 16). Huffman is 36.1% of mozilla's
encode, so that rung looked like a large target.

**Measured instead of assumed.** Instrumented `encode_rev_into` to census which path each
block takes. Totals across all 18 corpora:

| path            |   blocks |
| --------------- | -------: |
| **`emit_fill`** | **8867** |
| `emit_k::<6>`   |        8 |
| `emit_k5`       |        8 |

**`emit_fill` takes >99.8% of blocks.** `emit_k5` -- the predicted target -- runs EIGHT
times in the entire corpus.

**The larger finding: the whole K-rung ladder is nearly dead code.** `use_fill()` is
checked FIRST and declines only when `mean_nbits_x10 > 70` (mean above 7.0 bits/byte).
Virtually all real content sits below that, so the `16/14/11/9/8/7/6/5` ladder built by
bricks 16/29/32 is a rarely-taken fallback, not the hot path. `max_nbits = 10` dominates
the census, which is exactly the regime where `600/mean > k+2` holds comfortably.

**Consequence for planning:** any future Huffman emit work belongs in **`emit_fill`**, and
the K-ladder should be evaluated for removal or left explicitly as the documented
fallback. Its per-rung tuning cannot matter at 8 blocks in 18 corpora.

*(Instrument note: these counters are CUMULATIVE across corpora -- `prof` is not reset
between files in `--m7-profile` -- so only the totals and the ordering are meaningful,
not the per-corpus values. Fix before quoting per-file numbers.)*

## TOP OPEN DEFECT: `versions-16m` is 4.3x worse than C at L2-L4 (2026-08-16)

Spotted as an IMPOSSIBLE value in the L3 board: `us/c size` = **4.297**, on a corpus where
we are **0.755 (25% SMALLER than C) at L1**. A file cannot go from beating C by 25% to
losing by 4.3x one level up. Verified:

| level  |      us |           C |      us/c |
| ------ | ------: | ----------: | --------: |
| L1     | 820,848 |   1,087,656 | **0.755** |
| **L2** | 557,643 | **128,346** | **4.345** |
| L3     | 375,347 |      87,361 |     4.297 |
| L4     | 374,954 |      87,756 |     4.273 |
| L5     | 149,938 |      76,880 |     1.950 |

**C's output collapses 8.5x from L1 to L2** (1,087,656 -> 128,346). Ours improves 1.47x.

### The parameters are IDENTICAL -- this is not a config gap

|       | windowLog | chainLog | hashLog | minMatch | strategy |
| ----- | --------: | -------: | ------: | -------: | -------- |
| C L2  |        20 |       15 |      16 |        6 | fast     |
| us L2 |        20 |       15 |      16 |        6 | Fast     |

Same window, same table sizes, same minMatch, same strategy -- 4.3x different output.

### Two obvious explanations, both REFUTED

* **Probe density:** `STEP0=1` gives 557,643 -> 520,706. **6.6%.** Not the cause. (Contrast
  mozilla, where density explained 56% of the gap -- so this is a DIFFERENT defect.)
* **Window reach:** forcing a larger window sweeps 557,643 (wlog 20) -> 422,984 (21) ->
  358,274 (22) -> **338,605 (23)**. Even an 8 MB window leaves us **2.6x worse than C's
  128,346 achieved with a 1 MB window.** We cannot buy our way to C's result with reach.

**So C extracts ~4x better compression from IDENTICAL parameters and a SMALLER window.**
That is structural in the Fast finder on long-range repeated content (`versions-16m` is
16 MB of the same document re-emitted with edits, so matches are long and far apart).

### Why this is the highest-value open item

* It is the **largest single ratio defect** measured anywhere in the codebase (4.3x).
* It is **deterministic** -- exact byte counts, diagnosable on a thermally unstable box.
* It affects **L2, L3 and L4**, which brackets **L3, our shipping default**.
* L1 beating C on the same corpus proves the machinery CAN handle this content; something
  specific to the L2+ configuration is defeating it.

**Next probe (not yet run):** count sequences and match lengths at L1 vs L2 on this
corpus. If L2 emits far more, shorter matches than L1, the finder is fragmenting long
matches -- and `fill_hash_after_match` (which inserts only 2 positions after a match, at
`match_ip+2` and `match_end-2`) is the first place to look, because on 100 KB matches that
leaves almost the entire span unhashed.

## THE MISSING FUNCTION: no repcode search above Fast (2026-08-16)

"C is not special, we are missing a function." Correct. C's `ZSTD_compressBlock_fast`
checks **`offset_1` -- the repcode -- at EVERY position, before any hash lookup.** On
content with a constant stride (`versions-16m` is 16 MB of one document re-emitted with
edits) the next match is almost always at the SAME offset as the last, so repcode catches
it for free.

We have that function -- `try_rep1`, brick 40 -- and it is **DEFAULT OFF**, shelved
because it measured SLOWER on Silesia. Turned on, on the content it exists for:

| level | strategy |         C |        rep1 OFF |             **rep1 ON** |
| ----- | -------- | --------: | --------------: | ----------------------: |
| L1    | Fast     | 1,087,656 | 820,848 (0.75x) |      **81,206 (0.07x)** |
| L2    | Fast     |   128,346 | 557,643 (4.34x) |      **79,782 (0.62x)** |
| L3    | DFast    |    87,361 | 375,347 (4.30x) | 375,347 (**UNCHANGED**) |
| L4    | DFast    |    87,756 | 374,954 (4.27x) | 374,954 (**UNCHANGED**) |

**L1 goes from 0.75x to 0.07x -- we emit 13x FEWER bytes than C.** L2 goes from 4.34x
worse to 0.62x. The entire "4.3x defect" is repcode absence.

### L3/L4 do not move, and THAT is the structural finding

`try_rep1` is called from **`find_fast_impl` only**:

| finder           | repcode search |
| ---------------- | -------------- |
| `find_fast_impl` | present        |
| `find_dfast`     | **NONE**       |
| `find_greedy`    | **NONE**       |
| `find_lazy`      | **NONE**       |
| `find_bt_lazy`   | **NONE**       |

**Every strategy above Fast has no repcode search at all -- including DFast, which is L3,
our SHIPPING DEFAULT.** C implements repcode matching in `_doubleFast`, `_greedy` and
`_lazy` alike. This is a missing feature, not a tuning gap, and it is the single largest
structural hole found in the codebase.

### Why brick 40 shelved it -- and why that verdict was measured on the wrong axis

Brick 40 was A/B'd for SPEED on Silesia (0/6, z=-2.45, sao -23.0%) and shelved as "slower,
marginal ratio". Both halves were true and both were beside the point:

* It was measured on **Silesia**, which has little constant-stride repetition -- the exact
  content where repcode cannot pay.
* It was judged on **speed alone**. Its value is RATIO, and on the right content that
  ratio is **10x**.

A feature whose benefit is content-specific cannot be evaluated on a corpus that lacks the
content. This is `codec-content-adaptive-dispatch`'s sign-flip rule arriving from the
other direction: rep1 is a LOSS on Silesia and a 10x WIN on versioned data, which makes it
a dispatch candidate, not a global on/off.

### Next steps (none shipped -- all measured above)

1. **Port repcode search into `find_dfast`** (and then greedy/lazy). L3 is the shipping
   default and currently has none. Expect L3 to move from 4.30x toward C's 87,361.
2. **Re-adjudicate brick 40 on ratio, per corpus**, not on Silesia speed alone.
3. **Dispatch, not a global default**: rep1 costs speed where stride repetition is absent.
   The truth table wants building on ratio across all corpora BEFORE choosing a gate --
   and unlike the density knob, this one has a genuine SIGN-FLIP (loss on sao, 10x win on
   versions), which is the documented dispatch trigger.

## CRACKING OPEN `emit_fill` -- the real Huffman hot path (2026-08-16)

The K-rung census showed `emit_fill` takes **>99.8% of all Huffman blocks**, so the emit
primitives underneath it are where Huffman time actually lives (61.9% of mr's encode,
36.1% of mozilla's). Audited it for silent zeros, panics, unused primaries and dispatch.

### Is the two-path structure a DISPATCH? Yes -- and it is CORRECT

`emit_fill` runs two loops that emit the same bits by different means:

* the **K-loop** emits `k` symbols UNCONDITIONALLY -- safe by construction because
  `k * max_nbits + 7 < 64`, so it needs no per-symbol check;
* the **fill loop** emits CONDITIONALLY (`huff_fits(nb)` per symbol) because it is
  operating past the provable bound.

Branch-free-but-pessimistic vs opportunistic-but-branchy, with `use_fill()` as the outer
dispatch (fires when `600/mean > k+2`). `k` comes from `max_nbits` (10) while the mean is
6.88 bits, so the K-loop deliberately under-commits and the fill loop reclaims ~4 more
symbols per flush. **Sound as designed** -- which is why the wins were in the primitives
below it, not the structure.

### BRICK 68 -- `flush()` was a memcpy CALL per flush

```rust
self.buf.extend_from_slice(&bytes[..nbytes]);   // nbytes a RUNTIME 0..8
```

A variable-length `extend_from_slice` compiles to a memcpy call, and `flush` runs once per
K-group (~every 9 symbols) -- **~2.7M calls on mozilla's 24.4 MB of literals**. Replaced
with an unconditional 8-byte store into reserved capacity, committing only `nbytes`: the
trick bricks 36/37 proved on the decode copies.

### BRICK 69 -- the emit path's bounds checks were provably dead

`emit_fill` and `emit_tail` index `src[i]` where `i` starts at `src.len()` and only ever
DECREASES, under `while i >= k` / `while i > 0` guards. Provably in range;
`get_unchecked` with the invariant documented, `debug_assert` retained.

| `HuffCTable::encode_stream` | start | after 68 | after 69 |
| --------------------------- | ----: | -------: | -------: |
| memcpy calls                |     9 |    **0** |        0 |
| `panic_bounds_check`        |    19 |       11 |    **8** |
| `slice_index_fail`          |     9 |       -- |    **0** |
| whole-function instrs       |  2104 |     1896 |     2347 |

**The whole-function count rising is not a regression -- it is the WRONG STATISTIC**, the
same trap this campaign hit twice before. The panic paths were cold and out-of-line;
removing them let LLVM unroll, duplicating loop bodies. Measured where it matters, the
**hot loops are 8 instructions with 1 stack access and NO panic edge.**

Gates: 172 tests; **C-decodes-us 48/48** (12 files x 4 levels); generated corpora 6/6;
debug-build round-trips clean (exercises the `set_len` bound).

**Hashes RE-BASELINED after bricks 67/68/69** (67 changed the bitstream intentionally, and
I failed to re-record before 68, so 68's byte-identity could not be attributed --
correctness was established by conformance instead):
`mr ef0422922c20`, `webster 4f9bb0352478`, `nci 27d4d45d938e`, `xml 6f6e863aa533`,
`samba a3d3a03b18d4`, `dickens 99f624e9afd4`, `mozilla 72a90ae10a48`, `sao 586c515096e7`.

**Still open in this path:** `std::env::__var` appears inside `encode_stream` -- the
`huff_fast_enabled()` arm, same disease as brick 64's atomic but at per-STREAM (4/block)
rather than per-symbol frequency, so lower value. And 8 `panic_bounds_check` sites remain,
not yet attributed to source lines.

### Final scan of `emit_fill`: the hot path is WORKED OUT

Swept the remaining leads rather than assume they were wins.

**The 8 surviving `panic_bounds_check` sites are COLD.** They sit contiguously at the
function tail (offsets 2442-2518, ~11 lines apart -- the signature of 8 near-identical
regions, matching the 8 K-rungs), and each is an out-of-line LANDING PAD (`movb $1, N(%rbp)`
plus a distinct anon location constant) reached by a jump, not straight-line code. The hot
loops measure **8 instructions with NO panic edge**, so the checks feeding these pads are
outside them -- in the K-rung ladder, which the census proved takes **8 blocks of 8867**.
Removing them would add `unsafe` to code that runs 0.09% of the time. **Declined.**

**`reserve(8)` inside the new `flush` is a small residual.** `BitCStream::with_capacity`
pre-sizes to `src.len() + 8`, so the reserve almost never grows -- but Huffman output CAN
exceed input when common symbols code longer than 8 bits, so it cannot simply be deleted.
It costs a compare-and-branch per flush (~1 per 9 symbols). Hoistable by reserving the true
worst case (`ceil(len * max_nbits / 8) + 8`) up front, at the price of a larger allocation.
**Logged, not taken** -- it is ~2 instructions per 9 symbols against a memory cost.

**`std::env::__var` in `encode_stream`** is the `huff_fast_enabled()` arm -- brick 64's
disease, but at per-STREAM frequency (4 per block) rather than per-symbol. Real but low
value; the OnceLock means only the first call touches the environment.

**Verdict: the `emit_fill` hot path is finished for now.** Its loop is 8 instructions, 1
stack access, no panic edge, no memcpy call. The three remaining items are all cold,
bounded, or memory-trading; none justifies more `unsafe`. Further Huffman gains need a
different lever than micro-optimising this loop -- most plausibly reducing the NUMBER of
literals (a match-finder question) rather than the cost of coding each one.

### NEXT HUFFMAN LEVER (identified, NOT taken): a redundant full pass I introduced

Beyond `emit_fill`, the per-block Huffman work is `build_ctable` + `write_tree` + one
encode. Auditing those found a duplicated O(n) pass over the literals -- **added by brick
61**, and worth owning:

| pass                                        | cost                        | added by               |
| ------------------------------------------- | --------------------------- | ---------------------- |
| `build_ctable(lits)` -> its own `freq[256]` | full O(n)                   | original               |
| `segment_histograms(lits, 4)`               | full O(n)                   | **brick 61**           |
| `literals_worth_huffman`                    | 256-position strided SAMPLE | original (cheap, fine) |

**The two full passes are redundant: the sum of the 4 segment histograms IS the overall
histogram.** On mozilla that is a wasted pass over 24.4 MB of literals per block.

**The fix is clean** because `build_ctable_from_freq(&freq)` already exists as a separate
entry point: build the segment histograms once, sum them into a `[u32; 256]`, and pass
that in place of `build_ctable(lits)`.

**NOT TAKEN, deliberately.** This touches the entropy DECISION path -- a subtly wrong
histogram changes which section wins and therefore moves the bitstream. It needs the full
byte-identity gate across all eleven Silesia files plus the generated corpora, and starting
it without room to run that gate is how a bad stream ships. It is the first thing to do
next in Huffman.

*(Note the shape of the lesson: brick 61 was a real win -- 96% of mozilla's speculative
encodes eliminated -- and it silently added a full data pass to pay for it. A brick that
removes expensive work can still ADD cheaper work that nobody counted. Count the passes,
not just the calls.)*

## MATCHFIND, AUDITED THE WAY HUFFMAN WAS (2026-08-16)

The Huffman audit found its wins NOT in the loop structure (which was sound) but in the
PRIMITIVES underneath -- a variable-length memcpy in `flush`, provably-dead bounds checks,
and a duplicated data pass. Applied the same lens to MatchFind, whose probe loop is
already 19 instructions / 1 stack access.

Aggregated over `find_fast_impl`'s 10 monomorphizations:

| symbol               | instrs | memcpy | panic | slice_fail | realloc |
| -------------------- | -----: | -----: | ----: | ---------: | ------: |
| **`find_fast_impl`** |   8396 | **39** |     2 |     **40** |  **46** |
| `find_dfast`         |    459 |      3 |     4 |          3 |       4 |
| `emit_fast_seq`      |    250 |      0 |     4 |          0 |       2 |
| **`count_match`**    |    143 |  **0** | **0** |      **0** |   **0** |
| `push_literals`      |    119 |      1 |     0 |          1 |       1 |

**~4 memcpy, ~4 `slice_index_fail` and ~4.6 `RawVec` growth calls per monomorphization.**

**`count_match` is the control that makes this readable: 0/0/0/0.** The MATCHING itself is
tight; the PLUMBING around it is not. That is the same shape Huffman had -- `emit_fill`'s
structure was correct while `flush` underneath it called memcpy 2.7M times.

### The three suspected sources (located, NOT yet fixed)

1. **Literal copies** -- `lits.extend_from_slice(&src[anchor..ip])` and `push_literals`.
   The `&src[a..b]` range check is the `slice_index_fail`; the variable-length copy is the
   memcpy. Brick 38 already made the COMMON case a fixed-width 16-byte push, so what
   remains is the tail and the rep/flush paths.
2. **`seqs.push(Seq{..})`** -- a capacity check per SEQUENCE (1.84M on webster). Brick 38
   reserves `last_nseq + 25% + 64`, but that is a GUESS, so the check cannot simply be
   deleted; it needs a per-block "capacity known good" fast path.
3. **`RawVec` growth** -- 46 call sites, the other half of the same story.

### Why this is NOT yet a brick

The Huffman equivalents (bricks 68/69) were safe because the invariants were provable in
one line: `nbytes <= 8`, and `i` only decreases from `src.len()`. **The literal-copy
invariants are not** -- `anchor..ip` bounds depend on the match finder's state machine, and
`seqs` capacity depends on a heuristic guess that can be exceeded. Getting either wrong is
memory corruption, not a wrong byte.

This wants the same treatment the Huffman path got, in this order:
1. attribute the 39 memcpy / 40 slice_fail sites to SOURCE LINES (the `.loc` trick failed
   here -- release has no debug info; build with `-C debuginfo=1` to map them);
2. fix only the sites whose invariant is provable in one line;
3. gate on the eleven-file byte-identity set, which is now re-baselined and committed.

**Expected value:** MatchFind is 43.9% of nci's encode, 43.4% of mozilla's, 62.6% of
webster's -- so plumbing that costs ~4 calls per monomorphization is worth real time, but
it is NOT the 19-instruction probe loop. The loop is done; the copies around it are not.

## THE SHELF, RE-MEASURED (2026-08-15) -- every cross-process verdict re-run in-process

Runtime arms added to every brick that had been judged with the broken method, then all
re-run through `--ab-tag`. Spreads on the clean runs are 0.2-3.9%.

| brick                             | old verdict (cross-process) | RE-MEASURED (in-process ABBA)                                                                                                                                                                                                                                                                                             | disposition             |
| --------------------------------- | --------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------- |
| **16/29/32** Huffman emit         | "+14-30%"                   | **CONFIRMED, biggest win in the campaign** -- mr **+33.3%**, dickens **+16.5%**, webster +4.2%, nci +4.1% on the 4 clean rows; tracks Huffman share exactly                                                                                                                                                               | SHIPPED (on)            |
| **35** LL/ML LUT                  | "-10 to -17% cyc/B"         | CONFIRMED -- webster +16.3%, nci +17.6%, mr -0.8% (correct null)                                                                                                                                                                                                                                                          | SHIPPED                 |
| **36** literal copy               | "-7 to -15% cyc/B"          | CONFIRMED -- decode +9.5 to +23.5%, 5/6                                                                                                                                                                                                                                                                                   | SHIPPED                 |
| **37** match copy                 | "-10 to -26% cyc/B"         | CONFIRMED -- decode +10.7 to +28.7%, **6/6 z=+2.45**                                                                                                                                                                                                                                                                      | SHIPPED                 |
| **38** scratch reserve + lit push | "+5%, z=1.0, NOT shippable" | **OVERTURNED -> WIN** -- compress **6/6 z=+2.45**, +0.7 to +4.1%                                                                                                                                                                                                                                                          | SHIPPED (was shelved)   |
| **39** pipelined probe            | "-9.8% on sao"              | real but SMALL -- +0.3 to +3.5%, 6/6 z=+2.45                                                                                                                                                                                                                                                                              | SHIPPED                 |
| **B1** lazy back-fill             | mixed                       | CONFIRMED at **L7** (L1 never runs `find_lazy` -- the earlier L1 run measured nothing): dickens **+10.1%**, webster **+8.4%**, samba +3.8%, nci +2.2%, mr -1.3%, **xml -9.2%** (sign-flip REAL, spread 0.6%). Decode uniformly slightly worse -- it changes the bitstream, so more matches means more sequences to decode | SHIPPED, xml documented |
| **40** repcode-1                  | "slower, marginal ratio"    | **CONFIRMED SLOWER** -- 0/6, z=-2.45, sao **-23.0%**, dickens -4.3%, mr -4.0%                                                                                                                                                                                                                                             | stays shelved           |
| **41** tagged hash table          | "+3.6 to +7.1% win"         | **OVERTURNED -> LOSS** -- slower 6/6, -2.1 to -7.7%, z=-2.45                                                                                                                                                                                                                                                              | REVERTED                |
| **44** payload reserve            | (new)                       | +0.2 to +1.1%, z=+1.63 -- below resolution                                                                                                                                                                                                                                                                                | kept, labelled          |
| **45** seqcheck hoist             | (new)                       | +0.5 to +2.0%, z=+0.82 -- below resolution; the **RLE bound is a security fix**                                                                                                                                                                                                                                           | kept, labelled          |

**Two verdicts flipped in opposite directions** (38 shelved->win, 41 win->loss), and both
sat in the 3-7% band. Everything at 10%+ survived unchanged. That is the rule:
**cross-process drift destroys sub-10% verdicts and leaves large ones intact.**

**A measurement lesson worth keeping:** B1 read dead-flat at L1 with beautiful 0.4-1.1%
spreads -- a clean, trustworthy, MEANINGLESS number, because L1 uses the Fast strategy and
`find_lazy` is never called. **Confirm the code under test actually RUNS at the level you
measure** (`codec-measurement` 10, "prove the fast path RAN").

Bricks 42 (decode one-ahead) and 43 (encoder candidate prefetch) were reverted and their
code removed. Both measured worse, and together with 41 they form one consistent family
result -- the probe's memory is cache-resident, so neither skipping nor prefetching a load
pays. Not rebuilt.

## Bricks 38 / 44 / 45 (2026-08-15) -- first wins on the reliable instrument

### Brick 38 -- SHIPPED (was shelved as unproven)

Reserved `seqs`/`lits` scratch + fixed-width literal push in `find_fast`. Its original
verdict was "+5%, z=1.0, not shippable" -- taken with the cross-PROCESS method, and it
sat squarely in the 3-7% band that drift destroys. Re-adjudicated with in-process ABBA:

| file    |  compress |     | file    | compress |
| ------- | --------: | --- | ------- | -------: |
| dickens | **+4.1%** |     | nci     |    +2.0% |
| sao     | **+3.4%** |     | ooffice |    +1.8% |
| webster | **+3.1%** |     | mr      |    +0.7% |

**compress 6/6, z=+2.45**; decompress correctly null (2/6, z=-0.82) for an encode-only
change. Byte-identical -- all 12 Silesia hashes unchanged. **Default ON.**

The effect was real all along; only the instrument was wrong.

### Brick 44 -- KEPT, below instrument resolution

Reserve the per-block `payload` buffer (it grew from zero by doubling on every block).
Same shape as brick 38. Measured 5/6 compress but only +0.2 to +1.1%, z=+1.63.
`codec-measurement` 15 disposition: byte-identical and strictly less work, so KEEP and
label **below resolution** -- not a claimed win. Should be batched with other sub-1%
bricks behind one switch for a combined timing verdict.

### Brick 45 -- a SECURITY fix that was hiding as a hot-loop cost

The per-sequence test `ll_code > 35 || ml_code > 52 || of_code > 31` ran ~1M times per
file. Hoisting it to table-build time exposed a real hole:

| seq-table mode | symbol bounded at build?                            |
| -------------- | --------------------------------------------------- |
| 0 predefined   | yes -- by its norm table length (36 / 53 / 29)      |
| 2 compressed   | yes -- `read_ncount` rejects `charnum > max_symbol` |
| 3 repeat       | yes -- inherits an already-validated table          |
| **1 RLE**      | **NO -- raw stream byte, unvalidated**              |

So on an RLE sequence table the hot-loop test was the ONLY thing stopping untrusted
input from reaching `1u32 << of_code` with `of_code >= 32` (**undefined behaviour**) and
indexing `LL_BITS` / `ML_BITS` out of range. Mission section 10 lists "sequence codes
in-range" as a launch blocker; it was being enforced in the wrong layer.

Fixed: `seq_table` mode 1 now rejects `sym > max_sym` once per block. Gated by
`rle_seq_table_rejects_out_of_range_symbol` (in-range accepted, out-of-range rejected at
each of the three max values, plus the 255 worst case). With all four modes bounded, the
per-sequence test is provably redundant and was removed (a `debug_assert` keeps it in
debug builds).

**Speed: below resolution** (clean deltas +0.5 to +2.0%, z=+0.82) -- the branch was
well-predicted. **The correctness fix is the deliverable**, not the cycles.

Decode gate after the change: **36/36** (12 files x our stream, C `-1`, C `-9`).

## D9' -- IN-PROCESS ABBA, and the brick verdicts it OVERTURNED (2026-08-15)

**The defect.** Every brick A/B in this campaign ran the two arms as separate PROCESS
invocations, minutes apart. `codec-measurement` 3 requires interleaving precisely
because drift lives between the arms -- and on this box the drift is ~2x on that
timescale. Proof: two A/B pairs of the SAME brick, same session, read **-36.7% and
+2.6%**.

**The fix.** Arms are now runtime-settable (`set_tag_arm` / `set_pipe_arm`) and
`--ab-tag` runs **A,B,B,A per file inside ONE process**, seconds apart, reporting the
same-arm spread and a paired z. Effect on the noise floor:

|                 | separate processes | in-process ABBA |
| --------------- | ------------------ | --------------- |
| same-arm spread | 20-73%             | **0.1-2.1%**    |

**It overturned its own campaign on the first run.**

| brick                | reported (cross-process) | ACTUAL (in-process ABBA)                  |
| -------------------- | ------------------------ | ----------------------------------------- |
| 41 tagged hash table | "+3.6 to +7.1% win"      | **SLOWER on 6/6: -2.1 to -7.7%, z=-2.45** |
| 39 pipelined probe   | "-9.8% on sao"           | real but SMALL: +0.3 to +3.5%, z=+2.45    |

**Brick 41 is REVERTED (default off).** The packed form was built first (tag folded into
the top byte of the slot, positions reconstructed mod 2^24) and is strictly better than
brick 41's separate 64 KiB array -- ratio came out EXACTLY identical on all 12 Silesia
files and the dual gate passed 12/12 -- and it is STILL slower than no tag at all. That
is consistent with bricks 42/43: the probe's memory is cache-resident, so the tag's
compare-and-branch costs more than the `src[m]` load it avoids.

**Brick 39 is KEPT** but its value is ~1-3.5%, not ~10%.

### RE-ADJUDICATION of bricks 35 / 36 / 37 on the reliable instrument (2026-08-15)

Runtime arms added (`set_lut_arm`, `set_litcopy_arm`, `set_matchcopy_arm`) and the A/B
harness extended to report BOTH phases, since 36 and 37 are decode bricks. All three
**CONFIRMED**:

| brick                           | phase      | result                                                                                                                                                   |
| ------------------------------- | ---------- | -------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **35** LL/ML code LUT           | compress   | webster **+16.3%**, nci **+17.6%**, sao +3.8%, **mr -0.8%** (correct null: SeqCode is 1.4% of mr) -- 4/6, z=+0.82 with 2 samples auto-discarded as noisy |
| **36** fixed-width literal copy | decompress | webster **+23.5%**, nci +13.9%, dickens +13.7%, ooffice +9.5%, mr +2.0% -- 5/6, z=+1.63                                                                  |
| **37** fixed-width match copy   | decompress | webster **+28.7%**, dickens +20.5%, ooffice +19.2%, sao +10.9%, nci +10.7% -- **6/6, z=+2.45**                                                           |

Bricks 36 and 37 show ~-1% on COMPRESS, which is the correct null for a decode-only
change and a good control on the harness itself.

**These match the ORIGINAL numbers once cyc/byte is converted to MB/s** (a -15% cyc/byte
is a +17.6% MB/s). So 35/36/37 were measured correctly all along.

**The rule that explains which verdicts survived:** large effects (10-30%) survive
cross-process drift; small ones (3-7%) do not. Brick 41 (+3.6-7.1% claimed) sat in the
fragile band and evaporated; brick 39 (claimed ~10%) shrank to a real +0.3-3.5%.

### Consequences for the record

- Every per-brick speed number in this file taken before 2026-08-15 was produced by the
  cross-process method and is **suspect in magnitude**, including bricks 35, 36, 37 and
  the B1 fix. Bricks 35/36/37 additionally carry mechanism evidence (dose-response
  against a measured stage share, plus a correctly-null control), which is much stronger
  than a bare timing delta -- but their MAGNITUDES need re-running under `--ab-tag`.
- The cumulative "encode is 15% faster" claim is **not supported**. With 41 reverted and
  39 at ~1-3.5%, the verified encode gain is small.
- **Every future brick gets a runtime arm and is judged by `--ab-tag`.** A brick without
  one cannot be measured on this box.

## Next target -- `EncodeMatchFind` (count first, per D3')

After bricks 35-37, `EncodeMatchFind` is #1 on 7 of 12 Silesia files (54-85% of encode).
It was worked three times on instrument v1 (bricks 22, 24, 34) and all three verdicts
are void. **Do not open it with a kernel.** Open it with the counts already in the
profiler dump -- `probes`, `hits`, `fills`, `hit_rate`, `probes/byte` -- and ask which
of "too many probes", "probe too expensive", or "hash table thrash" the numbers
actually support. samba reads probes=3,385,620 / hits=741,851 (**hit_rate 0.219**),
i.e. four out of five probes are wasted; that ratio, not a wider load, is the question.

Also still unpriced on the encode side: the per-block `seqs` / `literals` / `payload`
`Vec`s (fresh per block, grown by push) and the per-block `entropy.clone()` kept purely
for the raw-fallback rollback -- the rav1e "rollback-wrapped mutation" shape.

---

# M7 encoder unknown — six whys (2026-08-13)

Unknown: **text-32m compress ~1.52× C** after brick 13. Zeros `-1` 1.11× and incomp `-1` 1.08× already inside 1.25×. Do not average those three.

Method: counts before times (`codec-six-whys-unknowns`). Per-content dual-gate (us round-trip + C `-d`). `--m7-profile` dumps work counts + C vs us block census. Size sweep = analyzer cache probe. Silesia = real content (never a mean).

---

## D6 — is the measurement sound?

- ASKED: same work, same flags, noise floor?
- COUNTED: input bytes equal (generated recipes). C and us block census compared per file. `zstd -1` / `--fast=1` / `--fast=4` only. Checksum on both (C default; us `CompressOptions.checksum`).
- MEASURED: session null-arm after the brick **1.125** (wide). Brick 13 session was 1.014. C compress MB/s drifted ~35% vs brick 13 (text `-1` 6765 → 8201). **Headline us-vs-us, not C/us.**
- ANSWER: D6 is open on the cross-impl ratio this session. Counts and us throughput are admissible. C/us is standing-only and contaminated by C drift.
- SPAWNED: D1 per content (do not average).
- STATUS: closed enough to descend on counts

## D1 — is the gap real, per content?

- COUNTED (oneshot, profile ON, before brick 14; trust COUNTED lines not the doubled dump):

| content         | probes | hits | match_frac | us blocks | C blocks            | us/c size |
| --------------- | -----: | ---: | ---------: | --------- | ------------------- | --------: |
| zeros-32m `-1`  |      0 |    0 |          0 | rle=256   | rle=240 **comp=16** | **0.901** |
| text-32m `-1`   |    313 |  256 |  **1.000** | comp=256  | comp=256            |     1.022 |
| incomp-32m `-1` | 381952 |    0 |          0 | raw=256   | raw=256             | **1.000** |

- ANSWER: three different machines. Zeros: we RLE everything, C spends 16 compressed blocks (we win ratio). Text: same 256 compressed blocks, match covers the whole file, ratio 1.022. Incomp: identical raw dump, 0 hits. **The 1.52× is not extra search work vs C on text.**
- CONFIDENCE: high (counts, one run)

## D2 — which stage, absolute?

- COUNTED: text `EncodeMatchFind` 256 calls (= blocks). `EncodeChecksum` 1 call × 32 MiB. After brick 13, oneshot split was MatchFind ~2.9 ms + checksum ~3.2 ms.
- ANSWER: checksum is ~half of text encode and is **content-invariant** (same 32 MiB xxh64 on all three). MatchFind is content-varying (0 / 2.9 / ~5–6 ms).
- SPAWNED: D3a checksum second pass, D3b unused tables, D3c scratch allocs, D3d incomp probes

## D3a — checksum: compute or a second cold walk?

- COUNTED: `xxh_b = src_len` always. Oneshot hashed **after** the block loop. RLE already scanned each 128 KiB; text `count_eq_len` already compared ~32 MiB; then xxh64 walked src from byte 0 again (cache cold).
- COUNTED (prune): unused Fast tables `hash_long+chain = 98 KiB` memset ≈ microseconds. Scratch 768 Vecs on text (1 seq + 114 lit bytes/frame) ≈ sub-1%. **Do not build those.**
- ANSWER: redundant full-file walk (`codec-eliminate-redundancy` move 4). Streaming already incremental. Oneshot was the miss.
- STATUS: rebuilt as brick 14

## D3b — cache-bound?

- COUNTED (size sweep, same density): text L1 oneshot **rises** 1 MiB→4 MiB (5267→5657 MB/s) then flat. Incomp **rises** with size (1510→2708 MB/s). Not an L2 cliff.
- ANSWER: not `codec-cache-tiles`.

## D3c — Silesia (real content gate)

- COUNTED: 12 files fetched. Dual-gate at `-1`: **x-ray OK**; **mr, ooffice, osdb, reymont, sao, webster, dickens, mozilla, nci, samba, xml FAIL** (us `-t` and C `-t` both reject).
- COUNTED: smallest failing `mr` prefix = **277521** B (OK at 277520). Finder recon of sequences **equals src**. Nocheck decode length matches; first mismatch at **277510** (last 11 bytes of block 2). Last block: 39 seqs, 11242 lits, 1 trailing lit, last seq `{litlen:4, matchlen:7, offset:31178}`.
- ANSWER: pre-existing **entropy encode/decode asymmetry** on varied matches (synthetic one-match-per-block never hits it). Correctness outranks speed. Not caused by brick 14 (`--no-check` also fails).
- STATUS: **closed** — Repeat FSE rejected zero-prob last LL (`FSE_getMaxNbBits` rounding). Dual-gate 12/12 Silesia × 3 flags. Standing board is `--m7-speed` Silesia, not generated text.

## D3d — Silesia re-board: which stage on REAL content?

- ASKED: after brick 14 + entropy fix, is the C gap still MatchFind?
- COUNTED (`--m7-profile`, oneshot, not standing MB/s). Generated text `-1`: EncodeMatchFind **49.5%** / EncodeEntropy **4.6%** / 256 seqs / match_frac=1.0. Silesia `-1` (never averaged):

| file    | probes | match_frac |    seqs | Entropy % | MatchFind % |  us/c |
| ------- | -----: | ---------: | ------: | --------: | ----------: | ----: |
| mr      | 3.11e6 |      0.309 |   41628 |  **62.7** |        35.1 | 1.114 |
| mozilla | 1.28e7 |      0.524 | 1551369 |  **56.9** |        40.9 | 1.222 |
| nci     | 2.15e6 |      0.943 | 1015138 |  **55.5** |        41.3 | 1.303 |
| xml     | 5.23e5 |      0.886 |  168003 |  **52.8** |        45.0 | 1.308 |
| reymont | 1.90e6 |      0.575 |  338038 |      48.3 |        50.3 | 1.437 |
| sao     | 2.35e6 |  **0.023** |   20014 |      11.5 |    **84.6** | 1.143 |
| x-ray   | 1.26e5 |  **0.000** |      51 |  **64.3** |        21.6 | 1.221 |

- COUNTED block census: mr us 77 compressed vs C **138** (C splits smaller). x-ray `-1` us 54 raw+11 comp vs C **116 compressed**. x-ray `--fast=1` C **63 raw + 2 comp**; us 31 raw + **34 Huffman**.
- MEASURED standing (`--m7-speed` ABBA, null-arm **1.158**): Silesia `-1` compress C/us **2.5–4.7** except sao 1.08 and x-ray **0.90**. Decode C/us **2.5–8.4** except sao **0.49** (us faster) and x-ray 1.12. Dual-gate all true.
- ANSWER: generated text is the wrong machine (one match/block, entropy ~5%). On Silesia, **EncodeEntropy is co-equal or #1** except sao (incomp-like search) and ooffice (MatchFind 58%). Catch-up vs C is Huffman/FSE emit + C's raw/RLE skip, then decode. Not more `find_fast` SIMD.
- SPAWNED: D3e entropy emit vs C (Huffman/FSE bitstream); D3f incompressible skip (x-ray `--fast=1`); D3g decode literals/seq on Silesia (speed board, profiler currently dumps encode-only).
- STATUS: open — Great Gate Rung 0 is Z1 skip (`_greatgate/zstd-great-gate.md`); entropy emit is Rung 2 after skip is gated.

## D3f — incompressible skip (x-ray `--fast`)

- ASKED: is x-ray `--fast=1` C/us 9.81 a kernel or a missing dispatch?
- COUNTED: C 63 raw + 2 comp; us 31 raw + 34 Huffman. match_frac 0.0004. Force-on Raw everywhere would destroy nci/mr ratio (great-gate: big force-on gap → dispatch, not force-on).
- COUNTED (brick 15): default skip = Fast + `target_length` in 1..=7 + `match_bytes < minGain`. x-ray `--fast=1` now 65 raw / 0 Huffman, `early_raw=46`, EncodeEntropy **0%**. `--fast=4` 65 raw, `early_raw=58`, us/c **1.000** vs C (C also all raw). `-1` unchanged (54 raw + 11 comp, us/c 1.221). nci/mr `-1` us/c 1.303 / 1.114.
- MEASURED standing (null-arm **1.125**): x-ray `--fast=1` C/us **9.81 → 1.35** (us 1563 MB/s); `--fast=4` **15.85 → 2.34**. Dual-gate 12/12 × 3.
- ANSWER: **dispatch, landed.** Knob `RZSTD_INCOMP_SKIP=0` is cmp-identical at `-1`. Do not default-on at greedy (L3 tlen is a search knob).
- STATUS: **closed** as brick 15. Brick 16 (population-relative `literals_worth_huffman`) not needed — `--fast` under-fire is gone on x-ray.

## D3f2 — why x-ray `-1` is ~6–9× our other Silesia (not brick 15)

- ASKED: standing us encode x-ray **747** vs mr **86** / dickens **72**. Is brick 15 (`early_raw`, `--fast` only) firing at `-1`?
- COUNTED (`--m7-harvest --levels 1`, 12 files, `early_raw=0` on every block). Brick 15 is off (`tlen=0` at `-1`).
- COUNTED the actual `-1` skip, two ANDs in `encode_block`: `seqs.is_empty() && !literals_worth_huffman(block)` (sample 256, skip Huffman if `max_freq * 8 < n` i.e. lit_peak < 125). Then, if we still entropy-encode, dump to Raw when `payload >= src` (Gate B, paid Huffman).

| file                | blocks | nseq==0 | Gate A (empty ∩ peak<125) | us raw/comp | C raw/comp  | match_frac | EncodeHuff ms |
| ------------------- | -----: | ------: | ------------------------: | ----------- | ----------- | ---------: | ------------: |
| **x-ray**           |     65 |  **43** |                    **43** | **54 / 11** | **0 / 116** | **0.0001** |           6.8 |
| sao                 |     56 |   **0** |                         0 | 1 / 55      | 0 / 62      |      0.023 |           1.0 |
| mr                  |     77 |       0 |                         0 | 0 / 77      | 0 / 138     |      0.309 |      **61.5** |
| every other Silesia |      — |   **0** |                     **0** | 0 / all     | ~0 / all    |  0.28–0.94 |             — |

- COUNTED x-ray split of 54 raw: **43 Gate A** (no entropy; MatchFind 65, Entropy 22) + **11 Gate B** (Huff then dump). The remaining 11 compressed still cost **EncodeHuff 6.8 ms / 10.97 ms encode**.
- COUNTED sao is the refutation: mean lit_peak **54.8** (same as x-ray, below 125) but **nseq==0 is 0** — every block has ~357 seqs. Gate A never fires. Encode is MatchFind 74%, not Huffman. Decode stays fast because literals inside those compressed blocks are nearly raw.
- ANSWER: x-ray is not a faster kernel. It is the only Silesia file whose match-finder returns empty 128 KiB blocks **and** whose alphabet is flat, so we memcpy instead of Huffman. C at `-1` Huffmans all 116 blocks (us/c **1.221** — we are larger). Turning Gate A into “skip whenever peak<125 even if nseq>0” is brick 15 at `-1`; already known to smash nci/mr ratio. Do not default-on.
- STATUS: closed as a measurement. Lever for the other files is still Huffman/FSE on blocks that have matches, not this gate.

## D3e — entropy emit split (after skip)

- COUNTED (`--m7-profile`, dump includes decode; quote encode-only = stage / EncodeTotal): mr `-1` EncodeHuff **67%** of encode / FseSeq 1% / TableSelect 0.6%. mozilla Huff **41%** / FseSeq 8%. nci Huff 13% / FseSeq **20%**. TableSelect is not a kernel.
- ANSWER: Huffman bit-pack is the mr/mozilla kernel. FSE seq emit is the nci kernel. Wave 2 landed BitCStream word store, Huffman `with_capacity`, move-not-clone FSE tables. Brick 16: packed LUT + 4-symbol unroll (`add_bits_huff`). 4-stream lockstep reverted. Not a SIMD job (serial container); no NASM.
- MEASURED brick 16: mr `-1` us **66.6 → 91.6** (size 1.114). mozilla/nci inside floor. Dual-gate held.
- COUNTED brick 17: FSE CTable deltas packed `FseCDelta { nb, find }`. nci `-1` encode **179 → 186** (inside prior 1.147 floor; this session 1.027).
- STATUS: Huff 1X + FSE emit packed. Remaining encode gap vs C is still the pack vs C BMI2, not another LUT shape.

## D3g — decode ranking (Wave 0 wrap)

- COUNTED (decode % of DecodeTotal, not sao): mr `-1` DecodeLiterals **92%** / DecodeSeq 6%. mozilla lits **61%** / seq 37%. nci lits 20% / seq **76%**. webster lits 29% / seq **69%**. dickens C/us d still **7.35**.
- ANSWER: two decode machines. Huffman 4-stream on mr/mozilla/dickens; FSE seq on nci/webster. Copy residue is small (DecodeBlocks ≈ lits+seq).
- MEASURED bricks 18-19 (`--m7-speed --levels 1`, null-arm **1.027**, sizes unchanged). us-vs-us vs brick 16: mr d **178 → 282**; mozilla **185 → 289**; nci **385 → 485**; webster **222 → 288**; dickens **124 → 204**; x-ray **877 → 1326** (C/us d **0.67**, we win). 4-stream decode stayed sequential. Dual-gate 12/12 x 3.
- MEASURED bricks 20-21 (null-arm **1.016**). 5-symbol unroll + 4X decode lockstep. Same-session C/us d (C rose with the box): mr **3.45 → 2.65**; mozilla **3.05 → 2.37**; xml **3.19 → 3.14** (lockstep did not sign-flip). Dual-gate 12/12 x 3.
- STATUS: **closed** as bricks 18-21. Remaining decode gap is copy / C BMI2, not another DTable walk.
- MEASURED brick 23 attempts (reverted): seq-loop wrapper nci C/us d **2.96 → 3.30–3.63**; period splat **3.16**; hashed-u32 reuse hurt encode (ooffice C/us c **2.34 → 2.71**). Do not wrap the seq loop. Do not add `fast_probe` args.
- MEASURED brick 24 (SIMD/BMI2, null-arm **0.976**): Huffman `_bextr_u64` reverted in every shape (per-peek mr C/us d **2.65 → 4.50**; stream-level **3.49**; inlined-bextr still a loss). `overlap_wildcopy` reverted (nci DecodeSeq did not move). Kept C-formula shift peek + AVX2/NEON `count_eq_len` u64 tail. mozilla us d **339 → 391**; nci **532 → 684**; mr Huffman gap vs C remains (C/us d **2.67**). rustc cannot make a BMI2 HUF island as fast as C's BMI2-compiled object.
- MEASURED brick 25 (Huffman X2, null-arm **1.018**): C `HUF_DEltX2` compose + `HUF_selectDecoder` (decode256 only; tableTime sunk). Always-X2 sign-flipped nci (C/us d **3.15→3.59**) — reverted that shape. With select: mr C/us d **2.67→2.45** (C d ~1006→1013, not drift); dickens **4.03→3.27**; nci **3.15→3.04** (no sign-flip); xml **3.21→3.01** (no sign-flip). X1 scalar stays the oracle. Remaining gap is C's BMI2 HUF object, not another LUT.
- MEASURED brick 26 (fast 4X2 + upsample to 11, null-arm **1.034**): C left-justified `bits>>53` + CTZ reload. mr C/us d **2.45→1.49** (us d **413→617**). nci/xml no sign-flip. dickens flat (1-stream). mozilla C d 448 is a C stall — do not headline. Not 15% faster than C yet.
- MEASURED brick 27 (left-justify `BitRev`, null-arm **1.075**): hoist `(container << consumed)` out of every peek. dickens C/us d **3.27→2.95** (us **267→302**). mr held 1.49→1.53. nci −9% at session floor. xml no sign-flip.
- MEASURED brick 28 attempt (reverted): 1-stream CTZ `fast_1x1`/`fast_1x2`. null-arm **0.988**. dickens **302→284**, mr C/us d 1.53→1.52. C has no 1X1 fast loop; short 1-stream bodies do not enter. Do not retry.
- MEASURED brick 29 (encode `covers` + drop `Result` on `huff_sym`, null-arm **1.013**): nbits==0 is treeless miss, not a bug. mr us c **83.8→85.8**; nci **175→184**; mozilla **86.4** (prior 67.9 was cores 0.76). xml flat. Sizes unchanged. Dual-gate 12/12. Decode not touched.
- MEASURED brick 30 (FSE `FSE_decode_t` 4-byte + `read_bits(0)`, null-arm **1.052**): nci us d **479→769** (C/us d **3.11→2.81**); xml **433→673** (**3.16→2.84**); mozilla **351→570**. mr Huffman d only +19% (unique seq signal). Sizes unchanged. Dual-gate 12/12. Do not wrap the seq loop.
- MEASURED brick 31 attempts (reverted): Huffman 16/8/`emit_fill` dispatch. Always-fill: mr us c **101→115**, **sao C/us c 0.99→1.20** (sign-flip). mean_nbits gate session null-arm **1.070** inadmissible. Do not retry always-fill.
- COUNTED brick 32 census (`silesia_huff_nbits_census`): mr mean **4.7** (100% ≤5.5); mozilla **6.3** (100% ≤7.0, 12% ≤5.5); nci **4.1** + **52% max≤7**; sao **one** table mean **7.5** (0% ≤7.0). max≤3 = 0% on Silesia.
- MEASURED brick 32 (K-from-max + mean≤7 fill, K-then-extras; null-arm **0.974**): mr us c **101→144** (C/us c **4.04→3.07**). sao C/us c **0.99→0.97** (holdout held). mozilla C/us c 3.36→3.38 (Huff 33%, knife-edge mean). Sizes unchanged. Dual-gate 12/12. Do not retry always-fill.
- MEASURED brick 33 attempts (reverted): FSE CTable `[u16;512]` arrays nci C/us c **2.99→3.03**. Always-flush `encode_fast` extra Vec-extend tax. Opportunistic `fits(27)` nci **2.95** (noise) with sao C/us **0.97→1.26**. Do not retry FSE arrays or per-seq flush.

## D3h — level 3 is a different machine

- MEASURED `--m7-speed --levels 3`: compress C/us **2.5–4.0** on every Silesia file. x-ray **2.86** (we do **not** win). sao **2.55**. Do not port Fast skip or Huffman SIMD here until a greedy re-board.
- STATUS: baseline only; no greedy brick

## D4 / D5 — checksum primitive

- COUNTED: incremental `Xxh64::update` per block == oneshot `xxh64` (existing hasher tests + `frame_checksum_matches_oneshot_xxh64`).
- MECHANISM: hash while the 128 KiB block is still hot from RLE/match/raw copy.
- Prize: text checksum stage 3.2 ms → digest-only (~0 in the profiler; work lands in EncodeBlocks).

## Rebuild

| D3 class                                  | skill                                                            | brick                                                                                                                                                                                                                                                                                                                        |
| ----------------------------------------- | ---------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- |
| second cold walk of src                   | `codec-eliminate-redundancy`                                     | **14: incremental xxh64 in `encode_oneshot`**                                                                                                                                                                                                                                                                                |
| unused Fast hash_long/chain 98 KiB        | prune (arithmetic)                                               | not built                                                                                                                                                                                                                                                                                                                    |
| 768 scratch allocs                        | prune (arithmetic)                                               | not built                                                                                                                                                                                                                                                                                                                    |
| Silesia entropy tail                      | `codec-bringup-encoder`                                          | **landed** — Repeat FSE 0-prob + last-symbol decrement                                                                                                                                                                                                                                                                       |
| x-ray `--fast` Huffman-then-dump          | `codec-search-skip-gate`                                         | **15: early minGain Raw on Fast `--fast=N`**                                                                                                                                                                                                                                                                                 |
| Huffman emit (mr 67% of encode)           | `codec-eliminate-redundancy` + `codec-content-adaptive-dispatch` | **16: packed LUT + 4-symbol unroll**; 4-stream lockstep reverted; **29: `covers` once, drop `Result` on `huff_sym`** (naive debug_assert panics on treeless miss); **31 reverted** — always-fill sign-flipped sao; **32: K-from-max (16/8/6/5) + fill gated mean≤7.0** (sao one table at 7.5 stays K6)                       |
| FSE seq emit (nci 20% of encode)          | `codec-eliminate-redundancy`                                     | **17: `FseCDelta` AoS**; **33 reverted** — CTable arrays / per-seq `encode_fast` (flush tax; nci ~noise)                                                                                                                                                                                                                     |
| Huffman decode (mr 92% of decode)         | `codec-eliminate-redundancy`                                     | **18: packed DTable + 4-unroll + skip_bits**; **20: 5-unroll**; **21: 4X lockstep**; **25: X2 DTable + `HUF_selectDecoder`** (always-X2 reverted — nci sign-flip); **26: fast 4X2 `bits>>53` + DTable upsample to 11**; **27: left-justify `BitRev`** (1-stream / FSE / remainder); **28 reverted** — 1-stream CTZ fast loop |
| FSE seq decode (nci 76% of decode)        | `codec-eliminate-redundancy`                                     | **19: stash `FseEntry`; extra bits still between peek and advance**; **30: C `FSE_decode_t` 4-byte layout + `read_bits(0)` no-op** (seq-loop wrapper still forbidden)                                                                                                                                                        |
| Fast match-find (ooffice 60% of encode)   | `codec-eliminate-redundancy`                                     | **22: hoist hash4; unaligned LE load; no Option on the hash slot**; **24: AVX2/NEON `count_eq_len` u64 tail**; **34 reverted** — C Fast repcode1 (sao sign-flip); prefetch; hash `get_unchecked`                                                                                                                             |
| Huffman BMI2 peek / wildcopy              | `codec-vectorize-kernel`                                         | **24 reverted** — bextr island lost ~2× vs shift; wildcopy did not move nci DecodeSeq                                                                                                                                                                                                                                        |
| FSE seq copy / extras (nci 76% of decode) | `codec-eliminate-redundancy`                                     | **23 reverted** — seq-loop wrapper, period splat, probe `u32` reuse; see mission                                                                                                                                                                                                                                             |

Keep rule: per-content, no sign-flip. Dual-gate held on generated. Ratio unchanged (checksum bytes identical).

### Standing us throughput (profiler OFF, vs brick 13 us — C drifted)

Session null-arm **1.125** — treat &lt;~12% as noise.

| corpus     | flag       | us before | us after | note                                     |
| ---------- | ---------- | --------: | -------: | ---------------------------------------- |
| zeros-32m  | `-1`       |      4403 | **5664** | above floor                              |
| zeros-32m  | `--fast=4` |      3617 | **6021** | above floor                              |
| text-32m   | `-1`       |      4453 | **5085** | near floor                               |
| text-32m   | `--fast=4` |      4334 | **5530** | above floor                              |
| incomp-32m | `-1`       |      2466 |     2748 | inside floor; C/us **0.99** this session |
| incomp-32m | `--fast=4` |      2000 |     2014 | flat, no loss                            |

No content class got slower. Sizes unchanged (zeros us/c 0.901, text 1.022 / 0.818, incomp 1.000).
