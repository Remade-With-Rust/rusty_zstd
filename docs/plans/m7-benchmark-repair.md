# M7 benchmark repair — get the numbers admissible on all content

**Date opened:** 2026-08-14
**Governing:** `codec-measurement` (every rule cited below is from it), `codec-analyzer`
**Why:** 36 full Silesia L1 sessions in `bench/ledger.jsonl` show the encode campaign
(bricks 16, 17, 22, 29, 32 kept; 31, 33, 34 reverted) moved compress C/us by **0–9% on
every file, all inside the measured noise band**. The decode campaign moved it 13–63%.
The instrument cannot currently tell a keep from a coin flip, so the encoder campaign has
been running blind for ~20 sessions.

This document is the repair plan. **No optimization brick lands until Phase A closes.**

---

## 0. What the ledger proves about the current instrument

Analysis over 829 `m7_speed` rows / 36 full L1 sessions:

| Finding                                                                                | Number                                            |
| -------------------------------------------------------------------------------------- | ------------------------------------------------- |
| Box swing between sessions (C's own **unchanged** v1.5.7 binary, median over 12 files) | **0.678× … 1.269× = 1.87× spread**                |
| Session-to-session CoV of raw `us` MB/s                                                | 17–20%                                            |
| Session-to-session CoV of same-session `C/us`                                          | 5–10%                                             |
| `r(us_mbps, c_mbps)` across sessions                                                   | **0.86–0.94** (common-mode: the box, not C drift) |
| null-arm CoV                                                                           | 5.0%                                              |
| `r(null_arm, actual box scale)`                                                        | **−0.16** (the null arm is blind to it)           |
| Repeatability of `C/us` decomp with **zero code change** (sessions 69–77, n=9)         | **±12% to ±32% per file**                         |

Two consequences, both load-bearing:

1. **`us`-vs-`us` across sessions is the contaminated metric**, yet the mission doc
   headlines it ("headline us-vs-us, C drifted") on nearly every brick since 14. It is
   backwards: the ratio is 2–3× tighter than the thing replacing it.
2. **The null arm certifies a session it cannot see.** It compares two `us` arms
   back-to-back *inside* one session, so it reads ~1.00 while the whole session sits 30%
   low. Sessions 53–56 ran at 0.68–0.72× box scale with null arms of 0.99–1.03.

Worked example (`codec-measurement` §7 — an impossible number is the instrument asking
for help): brick 32 cites mr compress `4.04 → 3.07` as the keep. Across all 36 sessions
mr compress C/us is mean 3.22, sd 0.30. **The 4.04 is the maximum of the entire run
(z = +2.73); the 3.07 is the mean (z = −0.49).** The brick measured regression to the
mean.

---

## 1. Instrument defects, ranked by how much they bias the answer

Read against `codec-measurement`. Each is a concrete code site.

### D1 — Both arms are WALL time. (§2, the single biggest fix)

`rusty_zstd::bench_roundtrip` in [in_bench.rs](../../crates/rusty_zstd/src/in_bench.rs)
times with `Instant::now()`. `measure.rs` *already has* `process_cpu_ms()` and the C child
already yields `GetProcessTimes` — but the throughput numbers on both sides are wall.

> §2: elapsed wall counts time you spent DESCHEDULED; CPU time does not accrue off-core.
> Measured spread: wall 0.78–1.50, CPU **0.950–1.089**. 5× tighter, needs no quiet box.

Affinity mask 4 **restricts** us to CPU 2; it does not **reserve** it. Every foreign
process scheduled onto CPU 2 (or its SMT sibling) is currently charged to our arm.

### D2 — `us` is MEAN-of-N; C is BEST-of-N. (§4 + §8, a systematic bias FOR C)

`time_loops` accumulates `compress_ns +=` across every loop and `mbps()` divides the
**sum**. That is an arithmetic mean over ~200 loops — dragged down by every descheduling
event in the 3-second window.

facebook/zstd's `-b` (`BMK_benchMemAdvanced`) tracks the **fastest** round and reports
that. So we are comparing *our mean* against *C's best*. This inflates every `C/us`
figure in the ledger by an unknown, content-dependent factor.

**This must be verified empirically before it is treated as fact** (§8: benchmark the
reference against itself). Test: run C `-b1 -i1` vs `-i10` on one fixed file. Best-of-N
rises with N; a mean is flat. → Task A0.

### D3 — Work parity: we allocate per loop, C does not. (§4)

Each `bench_roundtrip` iteration calls `compress(src, level)` → a **fresh `Vec`** (and
`decompress` likewise), so a 32 MiB alloc + page-fault walk is inside the timed region
every loop. C `-b` compresses into a **preallocated** dst buffer allocated once.

We are timing an allocator that C does not run. Fix: hoist the buffers, add
`compress_into` / `decompress_into` on the bench path.

### D4 — n = 1 pair per file per session; no z, no win rate. (§3)

`m7_one` runs ABBA as `C, us, us, C` — **one pair**. §3 requires N ≥ 20 pairs with a
paired win rate and `z = (wins − N/2)/(0.5·√N)`, and explicitly warns: "never quote a
single pair as the effect size — one 1.18× pair became 1.033× (22/24, z = +4.08) at
proper N."

Every brick in the mission doc is decided on n = 1 against a ±12–32% band.

### D5 — The null arm cannot see session-level drift. (§15 corollary)

It is a single us-vs-us on `files[0]` at `levels[0]`, run once at session start. Proven
blind (r = −0.16). §15 says: when the floor moves, **go find the process** — do not call
it weather.

### D6 — Two different timers for the two arms. (§13)

`us` MB/s comes from our `Instant`; `C` MB/s is parsed out of `zstd -b` stdout, i.e. C's
*internal* timer, its own loop shape, its own best-of-N policy. §13: "ONE compliant
timing harness; everything else calls it." Right now neither arm's number is produced by
the same instrument.

### D7 — Deterministic and timed quantities share one loop. (§13)

`bench_roundtrip` verifies `decode(encode(x)) == x` inside the timing loop. The compare
itself sits outside the accumulators (correct), but sizes, census counts and the gate all
ride the same pass as the timings. §13: "never share a loop between DETERMINISTIC and
TIMED quantities. Split the passes."

### D8 — Content coverage is a fraction of the plan. (§9)

Mission §6.1 names 8 corpus classes. The harness has **2**: Silesia (12 files) and 3
generated 32 MiB files. Missing: Calgary/Canterbury, enwik8/9, the small-file/dictionary
set, binary/tar-of-versions for LDM, SpaceDB-shaped CRDT/CBOR, media/model blobs.

Levels: the board runs `-1`, `--fast=1`, `--fast=4`, `-3`. The product is **−7…22**.
Strategies 4–9 (lazy…btultra2) have **never been speed-measured at all**.

Modes: streaming, dict, MT, LDM, seekable are dual-gated for correctness but have **no
speed board**. §9: measure on real content — and provenance is content.

---

## 1b. RESULTS — Phase A findings as they land

### A0 (closed) — C `-b` is best-of-N; we were mean-of-N

Measured, v1.5.7, `silesia/mr -b1 -T1`, three processes per setting:

| `-i` | reported compress MB/s |
| ---- | ---------------------- |
| 1    | 360.6, 405.4, 404.9    |
| 3    | 413.0, 413.5, 413.4    |
| 8    | 409.3, 406.7, 413.0    |

Converges from below and the spread collapses from ~45 MB/s to ~0.5 MB/s. That is a
best-of-N estimator; a mean holds its centre. Our `time_loops` summed every loop and
divided total bytes by total time — a mean.

### A2 (landed) — estimator parity, and what the bias was worth

`in_bench.rs` now records per-loop samples and exposes `*_best_ms` / `mbps_best`; both
arms take best-of-N, including across the two ABBA arms. Measured **within each row**
(`us_compress_mbps` vs `us_compress_mbps_mean`, no cross-session comparison, no box
dependence):

| file      | compress | decompress |     | file       | compress | decompress |
| --------- | -------: | ---------: | --- | ---------- | -------: | ---------: |
| reymont   |   +29.6% |     +30.8% |     | xml        |    +9.5% |      +9.9% |
| ooffice   |   +24.8% |     +28.3% |     | sao        |    +9.0% |     +11.4% |
| mr        |   +23.3% |     +25.8% |     | text-32m   |    +7.5% |      +9.1% |
| x-ray     |   +17.9% |     +22.4% |     | mozilla    |    +6.8% |      +5.1% |
| dickens   |   +16.0% |     +18.9% |     | incomp-32m |    +5.5% |      +9.5% |
| osdb      |   +12.9% |     +15.2% |     | samba      |    +3.6% |      +4.5% |
| zeros-32m |    +9.5% |     +12.6% |     | nci        |    +2.1% |      +3.7% |
|           |          |            |     | webster    |    +0.8% |      +1.6% |

**Median +9.5% compress / +11.4% decompress.** Every historical `C/us` in the ledger is
overstated by that much — more than the entire measured effect of bricks 16–34 combined.

And it is **content-dependent** (+0.8% webster … +29.6% reymont), so it is not a constant
you can subtract: it differentially distorted the per-file axis on which every dispatch
decision in the Great Gate campaign was made.

Dual-gate 12/12, sizes unchanged, lib tests 115/115.

### D5 (root-caused) — the box swing is THERMAL DECAY, and it happens mid-session

The 1.87× "box swing" is not random. In one 5-minute L1 session, C's own unchanged
v1.5.7 binary read:

| file (in run order) | C compress MB/s |
| ------------------- | --------------: |
| mr                  |           442.8 |
| ooffice             |           407.2 |
| osdb                |           475.7 |
| reymont             |           294.9 |
| sao                 |       **204.6** |
| webster             |       **214.2** |
| dickens             |       **201.2** |

A 2× collapse partway through, under sustained single-core load. The **null arm for that
session read 1.0012** — pristine — because it is measured once, at session start, before
the decay. §15's "when the floor moves, go find the process" applies: the process is our
own harness heating the box.

**No wall-clock MB/s figure can survive this**, which is why 20 sessions of encode bricks
were unresolvable.

### A1' (landed) — cycles per byte, the frequency-invariant metric

Wall time confounds work with clock rate. CPU cycles do not: a throttled box executes the
same code in the same cycles and more milliseconds. `bench_roundtrip_clocked` takes an
injected tick source (the library stays portable and dependency-free); the harness passes
`QueryThreadCycleTime`, which additionally does not accrue while descheduled.

Ledger gains `us_compress_cycles_per_byte` / `us_decompress_cycles_per_byte`.

> **This is now the cross-session progress metric for brick verdicts.** `C/us` MB/s stays
> as the standing cross-implementation number, ABBA-adjacent, and is only quoted with its
> session noted.

---

## 2. Phase A — make the instrument admissible (blocks all optimization)

Ordered so each task's result is checkable before the next.

| #      | Task                                                                                                                                                                                         | Gate / expected outcome                                                                                                                                     |
| ------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------------------------------------------------------------------------------------------- |
| **A0** | Empirically settle D2: C `-b -i1` vs `-i10` vs `-i30` on one fixed file, pinned. Also read whether our own numbers are mean or min.                                                          | A number, in the ledger, saying which policy each arm uses. If C is best-of-N, **every historical `C/us` in the ledger is biased and must be labelled so.** |
| **A1** | `bench_roundtrip` → **CPU time** via a new `cpu_now()` (Windows `GetProcessTimes`, Unix `getrusage`). Keep wall alongside; report `cpu/wall` (already `cores_busy`).                         | Re-run the same binary 9× ⇒ CoV of `us` CPU-MB/s **< 1/3** of the wall CoV.                                                                                 |
| **A2** | `time_loops` → record **per-loop** durations; report **min**, median, and mean. Headline the **min** (§"order of instruments" #2: pinned CPU time, best-of-N).                               | Same-binary repeatability tightens; min is stable where mean is not.                                                                                        |
| **A3** | Work parity: preallocate compress/decompress buffers once per arm (`compress_into`/`decompress_into`). Print **bytes in / bytes out / loops** for both arms.                                 | Alloc count per loop = 0 in the timed region. Counts match C's.                                                                                             |
| **A4** | Real paired A/B: N ≥ 20 ABBA pairs per (file, level). Report **paired win rate + z**. Refuse to print a verdict at \|z\| < 2.                                                                | A brick either clears z=2 or is labelled *below instrument resolution* (§15).                                                                               |
| **A5** | Cross-session **anchor**: fixed anchor workload run at session start AND end; record `anchor_scale`. Ledger gains `box_scale`. Abort/flag a session whose start and end anchors differ > 5%. | The 1.87× box swing becomes a *recorded covariate* instead of an invisible one.                                                                             |
| **A6** | Split deterministic from timed passes (§13). One pass for sizes/census/gate, N passes for timings.                                                                                           | Sizes and counts identical run-to-run; timings carry the distribution.                                                                                      |
| **A7** | Method line gains: timer (cpu/wall), estimator (min/mean), pairs N, z, null-arm, anchor scale, alloc-in-loop yes/no.                                                                         | Every ledger row auditable after the fact.                                                                                                                  |
| **A8** | §15 process hygiene: dump CPU-heavy foreign processes + start times at session start into the ledger row.                                                                                    | A moved floor gets a **named, dated cause**, not "the box got noisy".                                                                                       |

**Phase A exit gate:** re-run the *identical* binary 9 sessions spread over hours. Per
file, `C/us` CoV must be **< 5%** (today: 5–10% at best, ±12–32% range). Until that
holds, no brick verdict is admissible.

### A9 (NEW, from the brick-38 attempt) — cross-session A/B does not work on this box

Bricks 35–37 were judged by comparing adjacent sessions, which worked while the box was
stable. Brick 38 exposed the limit: three consecutive sessions were invalidated by
contention (C's own binary moving 20–27%), and the null arm read 1.0162 / 0.9918 /
1.0344 / 0.9382 through all of it.

Three fixes, in priority order:

1. **Interleave the arms inside one session.** Built for brick 38 as an env toggle
   (`RZSTD_LIT_PUSH`) with the arm resolved once per block, never inside the probe loop.
   This is what `codec-measurement` §3 has always required; the campaign had been
   approximating it with adjacent sessions. **Every future brick gets a toggle.**
2. **Discard the first measured file.** sao is measured first and swung **63%** for
   identical code while later files held to 3–8% — it absorbs cold caches and the
   frequency ramp. Add a warmup pass whose result is thrown away.
3. **Replace the null arm with the ABAB same-arm spread.** The null arm has now failed
   to detect four separate contaminated sessions. The same-arm spread across interleaved
   repeats measures the same thing and actually works.

**Process hygiene is not optional here.** Eight `Code.exe` NodeService processes had
accumulated ~107,000 CPU-seconds *each* (~850 CPU-hours total) since 2026-08-12. Killing
them (user-authorised) raised C's own throughput to the highest figures of the whole
campaign — nci **1043 MB/s** vs a prior best of 932, xml **843** vs 738. Every number
taken before that point was measured against a contended box. VS Code respawns these
services, so the check must be run *per session*, not once.

---

### Phase B progress (2026-08-14) — three product-corpus classes landed

`corpus.rs` gains three deterministic, offline, seeded generators covering mission 6.1
classes Silesia does not represent. They immediately showed what the old board was blind
to (`--levels 1`, repaired instrument):

| corpus       | class                                          | us/c size |   C/us c |   C/us d |
| ------------ | ---------------------------------------------- | --------: | -------: | -------: |
| smallmsg-8m  | many 1-16 KiB messages (dictionary payoff)     | **1.615** |     1.74 | **3.49** |
| jsonlog-16m  | SpaceDB-shaped CRDT/JSON logs (product corpus) | **1.527** |     2.30 |     2.22 |
| versions-16m | tar-of-versions (long-range / LDM)             | **0.755** | **0.94** |     1.22 |

**The product corpus is our worst ratio anywhere** -- 1.53-1.62x vs C, beyond every
Silesia file (worst was reymont 1.437). This is the content MATA actually ships through
the codec, and it was not on the board. Conversely `versions-16m` is a class where we
**beat C on both size and compress speed**, which no Silesia file showed either.

Neither fact was visible from Silesia. Still outstanding in Phase B: Calgary/Canterbury
and enwik8 (need network), levels beyond 1 and 3, the streaming/dict/MT/LDM mode boards,
and RSS for the in-process arm.

---

## 3. Phase B — all content, all levels, all modes

Only after A. Each row is a corpus class from mission §6.1 with a fetch/generate recipe,
pinned hashes, and a train/holdout split.

| Class                                         | Source                        | Why it must be on the board                                                                                                                |
| --------------------------------------------- | ----------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------ |
| Silesia (12)                                  | have                          | The standing bar. **Never averaged.**                                                                                                      |
| Generated zeros / text / incomp               | have                          | Path coverage (RLE / one-match / raw). Text-32m is **forbidden** as a stage ranking (entropy 4.6% — it hid the real machine once already). |
| Calgary + Canterbury                          | fetch                         | Small-file, classic; exercises table setup vs steady state                                                                                 |
| enwik8 excerpt                                | fetch                         | Large-window text, LDM                                                                                                                     |
| Small-message set (1–16 KiB × many)           | generate                      | Dictionary payoff; per-frame overhead dominates here and is currently unmeasured                                                           |
| Binary / tar-of-versions                      | generate from the repo itself | `--long` / LDM, the only class where window logic is hot                                                                                   |
| SpaceDB-shaped (CRDT deltas, CBOR, JSON logs) | generate                      | The product corpus — this is what actually ships                                                                                           |
| Media / model blobs                           | fetch small samples           | remade_ffmpeg_rs / FFAI consumers                                                                                                          |

Level and mode coverage to add:

- **Levels:** −7, −4, −1, 1, 3, 6, 9, 12, 15, 19, 22 (strategies 1–9 all represented).
  Today only strategies 1–2 are ever timed.
- **Modes:** oneshot (have), **streaming**, **dict**, **MT `-T0`**, **LDM `--long`**.
  Each needs its own board; a mode with no number is not shipped.
- **Footprint:** peak RSS is captured for C children but **not** for the in-process `us`
  arm. Mission §7 has an RSS target (≤1.2× C) with no instrument behind it.

---

## 3b. PHASE B RESULTS (2026-08-14) — the level board exists, and it found two defects

`--levels -1,1,3,4,5,7,9,11,13,16` on mr + xml. **Strategies 3-9 had never been
speed-measured in this project.** Warmup pass discarded; same-arm spread reported per row.

|   L | strategy     | mr C/us c |   mr cyc/B |   mr us/c | xml C/us c | xml cyc/B |  xml us/c |
| --: | ------------ | --------: | ---------: | --------: | ---------: | --------: | --------: |
|  -1 | fast+tlen    |      3.38 |       28.3 |     0.807 |       2.17 |      12.2 |     1.087 |
|   1 | fast (1)     |      2.83 |       26.6 |     1.114 |       2.10 |      11.2 |     1.308 |
|   3 | dfast (2)    |      2.29 |       39.8 |     1.052 |       1.73 |      13.0 |     1.185 |
|   4 | greedy (3)   |      2.02 |       40.8 | **1.054** |       1.63 |      14.9 | **1.171** |
|   5 | lazy (4)     |      1.73 |       66.9 |     1.089 |       1.65 |      32.5 |     1.249 |
|   7 | lazy2 (5)    |      2.14 |      125.8 |     1.155 |       1.44 |      42.3 |     1.305 |
|   9 | btlazy2 (6)  |      2.49 |      230.5 |     1.084 |       1.46 |      55.6 | **1.324** |
|  11 | btopt (7)    |      3.60 |  **620.4** |     1.087 |       1.48 |  **92.1** |     1.314 |
|  13 | btultra (8)  |  **0.64** |      456.2 |     1.088 |   **0.45** |      70.2 |     1.196 |
|  16 | btultra2 (9) |      1.29 | **1368.8** |     1.063 |       1.72 | **999.6** |     1.117 |

**CORRECTION (2026-08-14): the strategy labels above are WRONG, and B2 was my error.**
The level -> strategy table is **source-size dependent** (`table_for_size(src_hint)`), and
I derived the mapping from `/dev/null`. For a real 5.3 MB file the mapping is
L5=greedy, L7=lazy, L9/L11=lazy2, L13=btlazy2, L16=btopt -- so "btultra cheaper than
btopt" never happened. L11 is lazy2 at **searchLog=6** and L13 is btlazy2 at
**searchLog=4**; a cheaper higher level is legitimate there.

**Our level table was then checked against C v1.5.7 directly** (`--show-default-cparams`,
both binaries, same file): all 7 parameters MATCH on every probed level 1/3/5/7/9/11/
13/16/19. Mission §3.1.1 satisfied. **B2 is closed as NOT A DEFECT.**

Law for this repo: never read the level table without a real `src_hint`. A
size-dependent table read at size 0 mislabels every row.

**Defect B1 — CONFIRMED and FIXED.** `find_greedy` back-fills the hash chain for every
position a match covers; **`find_lazy` and `find_bt_lazy` did not** -- they jumped
`ip = best_ip + best_ml` and inserted nothing, so every byte inside a match was absent
from the chain. On matchy content that is most of the file, so later searches saw a
nearly empty chain. C gets this right via `nextToUpdate` back-filling inside
`ZSTD_insertAndFindFirstIndex`. Fixed in both, behind `RZSTD_LAZY_FILL` (default ON).

Ratio is deterministic, so this A/B is exact byte counts, not timing:

| file    |   L | strat | ratio (us/c) | speed (cyc/B) | gate        |
| ------- | --: | ----- | -----------: | ------------: | ----------- |
| mr      |   7 | lazy  |   **-6.81%** |         -7.9% | rt + C-d OK |
| webster |   7 | lazy  |   **-5.23%** |    **-21.0%** | OK          |
| dickens |   7 | lazy  |       -1.58% |    **-23.3%** | OK          |
| webster |   9 | lazy2 |       -2.75% |    **-36.8%** | OK          |
| mr      |   9 | lazy2 |       +1.63% |    **-36.7%** | OK          |
| xml     |   7 | lazy  |       -0.95% |   **+108.6%** | OK          |
| xml     |   9 | lazy2 |       -0.85% |   **+123.2%** | OK          |

**Ratio improves on 6 of 7. Speed improves on 5 of 7, by up to 37%** -- a populated
chain finds long matches sooner, so `ip` advances further and there are fewer search
positions overall. The back-fill pays for itself twice on most content.

### The `xml` sign-flip: dispatch attempted, PRUNED (2026-08-14)

Ran the full `codec-content-adaptive-dispatch` method. **Both honest endpoints were
reached in order, and the answer is (b) prune.**

**1. Truth table first (the skill's cardinal law) -- and it refuted the obvious signal.**
Back-fill ON vs OFF at L7, sorted by `match_frac`:

| file    | match_frac |      speed | verdict                                 |
| ------- | ---------: | ---------: | --------------------------------------- |
| sao     |      0.023 |      +3.1% | loss (noise: almost no matches to fill) |
| mr      |      0.309 |      -4.4% | WIN                                     |
| dickens |      0.364 |      -8.5% | WIN                                     |
| reymont |      0.575 | **-17.0%** | WIN                                     |
| osdb    |      0.628 | **-24.2%** | WIN                                     |
| samba   |      0.732 |      -0.5% | flat                                    |
| xml     |      0.886 | **+11.8%** | LOSS                                    |
| nci     |      0.943 |      -2.6% | flat                                    |

Not monotonic: `nci` (0.943) is MORE matchy than `xml` and does not lose. `match_frac`
is refuted.

**2. `searches/byte`, calibrated on the DEPLOYED estimator** (not the offline probe --
`find_lazy` had no counters at all; they were added). Separates the high end cleanly
(all WINs >= 0.180, flat/loss <= 0.157) but **fails on the pair that matters**: xml
0.0696 (LOSS) sits ABOVE nci 0.0412 (WIN), with near-identical hit_rate and longer mean
matches on the winner. No threshold exists between them.

**3. xml's regression is REAL** -- 4-arm ABAB, same-arm spreads 0.4-2.0%: xml ON
20.112/20.230 vs OFF 18.489 (**+9.1%**), nci ON 16.960/16.868 vs OFF 17.731 (-4.6%).
(An earlier +108.6% reading on xml was a contaminated sample; the same-arm spread field
would have caught it.)

**4. Back-off attempt (stride the fill) -- REFUTED BY THE RATIO GATE.** Striding looked
like a clean win on speed alone: xml OFF 18.489 -> s2 17.684 -> s4 16.310, turning the
loser into a WIN while nci went -22% and osdb -35%. **The deterministic byte counts
killed it**: stride 4 is worse than stride 1 on EVERY file (mr +8.02%, nci +5.12%,
webster +4.85%, samba +4.41%) and worse than OFF on most. Striding does not remove a
cost, it slides down the speed/ratio curve -- the wrong direction at a
high-compression level. **A speed-only A/B flattered a ratio loss as a win; the cheap
deterministic ratio gate is what caught it.**

**DISPOSITION (user call, 2026-08-14): ship full back-fill (stride 1, no threshold) and
accept xml.** The aggregate is strongly positive -- ratio improves on 6 of 8 files at
L7 (mr -6.8%, webster -5.2%, samba -4.8%) and speed improves on 5 of 8 -- against one
file at +9.1% speed / +0.96% ratio. Knobs `RZSTD_LAZY_FILL`, `RZSTD_LAZY_FILL_S`
(stride) and `RZSTD_LAZY_FILL_T` (threshold) stay in-tree at their shipping defaults so
the experiment is cheap to re-run if the chain structure changes.

**What would still be worth trying** (not attempted): the cost is chain DENSITY, so the
lever is the chain WALK, not the fill -- an early-exit in `chain_find_best` once a
match long enough to satisfy `targetLength` is found (C's `ZSTD_HcFindBestMatch` breaks
on `ml > targetLength`). That reduces walk cost without discarding candidates, so it
should not trade ratio the way striding did.

### RSS (landed)

`current_peak_rss()` (Windows `GetProcessMemoryInfo` on the current process) plus the C
child's peak are now on every speed row as `us_peak_rss_bytes` / `c_peak_rss_bytes`.
Mission §7 sets a <= 1.2x-C RSS target that previously had no instrument at all. Peak is
monotonic per process, so read it per level, not per file.

---

## 4b. PHASE C RESULTS (2026-08-14)

### C1 (CLOSED) — Huffman emit batch (bricks 16 + 29 + 32) is CONFIRMED

Re-adjudicated as ONE batch behind `RZSTD_HUFF_FAST` (`codec-measurement` §15: batch the
bricks, let the batch carry the timing verdict). `=0` routes literal emit through
`encode_stream_scalar`, which was already the byte-identity oracle and is now compiled in
release as the second arm. **Interleaved ABAB in one session**, not across sessions.

| file          | Huff share of encode | ON mean | OFF mean | cost of removing |
| ------------- | -------------------: | ------: | -------: | ---------------: |
| mr            |                60.6% |   29.09 |    37.95 |       **+30.5%** |
| mozilla       |                35.4% |   25.76 |    30.26 |       **+17.5%** |
| osdb          |                38.4% |   22.38 |    25.51 |       **+14.0%** |
| xml (control) |                 9.9% |   11.21 |    11.95 |            +6.6% |

Effect tracks each file's Huffman share; same-arm reproducibility 0.1-6.7% (mostly <3%),
so the effect is 5-30x the floor. Sizes byte-identical in every arm, dual-gate held.
**Verdict: KEEP. These three bricks were real all along** -- instrument v1 could not
resolve them, but it did not make them false.

**The same-arm spread metric paid for itself on its first run:** mozilla's OFF#2 reading
came back at **39.4% spread** and was excluded as a bad sample. The null arm for that same
session read normal. This is exactly the failure mode the null arm could never catch.

### Still open in Phase C

- Bricks **17** (FSE `FseCDelta` AoS) and **22** (`find_fast` hoist/unaligned load) have
  no toggle yet; they need the same treatment.
- The refutations **23, 24, 28, 31, 33, 34** remain *unproven on instrument v1*. Each
  needs re-implementation behind a toggle before it can be re-judged; the Huffman result
  above shows v1 was capable of hiding a 30% effect, so a v1 "flat" verdict means nothing.

---

## 4. Phase C — re-baseline and re-adjudicate

1. **Label the existing ledger.** 829 rows stay (they are history), but every `C/us` in
   them is annotated `instrument=v1 (wall, mean-vs-best, n=1)`.
2. **Re-measure the standing board** on the repaired instrument. This is the new zero.
3. **Re-adjudicate the kept encode bricks.** 16, 17, 22, 29, 32 added real complexity —
   packed LUTs, a census-driven dispatch menu, unroll variants — for a campaign-total
   compress movement of 0–9%, all inside noise. Each gets: does it clear z=2 on the
   repaired instrument? If not → revert for simplicity, or keep with a **counter** as
   primary evidence (§15: for sub-1% effects the counter is the verdict and the clock is
   confirmation).
4. **Re-open the refutations.** §11: a refutation expires when its baseline moves. Bricks
   23, 24 (BMI2), 28, 31, 33, 34 were refuted on the broken instrument at n=1. They are
   **not** settled. The "do not retry" list in the mission doc is downgraded to
   "unproven on instrument v1".

---

## 5. Then: optimize as if nothing is optimized

Per the goal, the campaign restarts from `codec-analyzer`'s methodology, not from the
brick history:

1. Build/repair the spine — stage profiler with an **`rdtsc` timer** (analyzer #1; the
   current profiler uses `Instant::now()`, ~2× the tax) and price the tap before trusting
   any residue.
2. Profile the **deployment** config on **real** content, decompose the residue until
   every line is named, subtract `calls × per-scope-overhead` before calling anything
   hidden work.
3. Classify every fat stage: compute-bound / memory-bound / redundant / entropy.
4. Cheap probes first — cache sweep (#3), bounds-check ceiling (#4) — to kill hypotheses
   before refactors.
5. `codec-eliminate-redundancy` first, then memory-copies, then cache-tiles, then
   vectorize, asm last. One brick, gated byte-identical, measured at z ≥ 2 or counted.

**Standing structural lead already on the board and never tried:** Z3 block splitting.
C emits **138** compressed blocks on `mr` where we emit **77**; on x-ray C emits 116 and
we emit 65. Nine kernel-level encode bricks moved compress 0%. The gap is plausibly
structural, not another LUT shape — but it gets a **count** before it gets code.

---

## 6. Non-negotiables carried into every task here

- Correctness gate first; scalar twin stays in-tree as the oracle (§0).
- Never average Silesia. Per-file, per-class, sign-flips are dispatch triggers.
- A skipped gate is never a pass.
- Say **which kind** of revert (measured worse / inside noise) with the arithmetic (§12).
- Counter before clock for anything under ~1% (§15).
- Delegating to a subagent? The spawn prompt must name `codec-measurement` explicitly and
  demand the method line + work counts back (§14).
