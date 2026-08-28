# inline-execution — converting inline scalar work to SIMD/NEON/AVX2

**Opened 2026-08-22.** Mission: find every place in the codec where inline scalar
byte/element work could be replaced by a vector kernel, list the decoder and the
encoder separately, and rank what is actually worth building.

Five independent scans were run over the codec. Each scan's completion is logged in
§1 with what it covered and what it returned.

---

## 0. The headline, before the catalogue

**The ISA-WIDTH lever is closed. The DATA-PARALLEL lever is barely opened.**

Those are two different things and the campaign has been conflating them.

*ISA-width* means: take an existing scalar body and recompile it under
`#[target_feature(enable = "avx2,…")]` so LLVM re-encodes its SSE as VEX and
auto-vectorises what it can. That is what the whole twin architecture does. It has
been swept exhaustively — see the refutation in §7 — and it is **done**.

*Data-parallel* means: replace a scalar algorithm with a genuinely different one
that processes N elements per instruction. The codec contains **exactly two**
hand-written kernels of this kind, totalling **20 instructions**:

| kernel | file | ymm ops | arch | reachable from the codec? |
| --- | --- | ---: | --- | --- |
| `count_eq_len_avx2` | `simd.rs` | 9 | x86-64 | yes |
| `premul_p2_avx2` | `xxh64.rs` | 11 | x86-64 | **YES since D8a — was NO, see V1** |
| `count_eq_len_neon` | `simd.rs` | — | aarch64 | yes |

Everything else that reads as "SIMD" in this codebase is an ISA-width twin.

**And the validation pass cut that list to one.** `premul_p2_avx2` is reachable only
through `xxh64_seed`, which nothing on the shipping path calls — the encoder, the
decoder and the streaming API all use `Xxh64::update`, which is scalar. So the
codec's entire hand-written vector surface, in production, is **`count_eq_len`**.
That is the single most important sentence in this document; the detail is V1 in §1b.

**Asm receipt (release, this build, `target/asmprobe`):** the whole library emits
**491 `ymm` references against 27,862 `xmm`**. 98.3% of the crate's vector surface
is 128-bit baseline SSE2. All 491 wide ops live in five functions:

| symbol | ymm | source |
| --- | ---: | --- |
| `encode::write_sequences_avx2` | 322 | LLVM auto-vec inside a twin |
| `compressed::decode_compressed_block_avx2` | 99 | LLVM auto-vec inside a twin |
| `compressed::decode_sequences_avx2` | 14 | LLVM auto-vec inside a twin |
| `xxh64::premul_p2_avx2` | 11 | **hand-written** |
| `simd::count_eq_len_avx2` | 9 | **hand-written** |

So 435 of the 491 wide ops were produced by the compiler for free, and 20 by hand.
That ratio is the whole argument for what to do next: the free wins have been
harvested, and further gains require kernels LLVM provably cannot invent.

**Where the time is** (§3 of `m7-anatomy.md`, L3, 8 MiB board, re-measured
2026-08-22): `EncodeMatchFind` is the encode leader on **18 of 18** corpora at
57.4–82.6%; `DecodeSeq` is the decode leader on **16 of 18** at 65.9–94.7%;
`DecodeChecksum` leads on the two incompressible corpora and takes 22–25% on
`text-32m` and `versions-16m`. At L1 `x-ray` inverts completely — Huff 86.7% of
encode, DecLits 75.9% of decode.

---

> **STATUS 2026-08-22 — §8's execution queue is COMPLETE.** All 14 scheduled bricks are
> resolved: **6 shipped** (D8a, E2, E11, D1+D2, D8b, D4), **3 pruned on ceiling probes**
> (E12, E4, D6), **1 refuted on its own premise** (D3), **2 closed on their stated
> ceilings** (D5, D9), **1 deferred for a reason outside the plan** (D7 — `simd.rs` was
> under concurrent edit all session), and **1 left open with its blocker named** (E5).
> E13 was verify-only and is closed. E1 remains its own campaign.
>
> Six of the ten items that did NOT ship were refused by a **deterministic counter**, not
> a clock — this box ran at ~70% CPU from a neighbouring job all session and its pinned
> CPU-time instrument turned out to be quantised to the 15.6 ms Windows tick, so no
> timing number here would have been admissible. Every probe built for those decisions is
> kept in the tree, `profile`-gated, so none of them get re-litigated from scratch.
>
> **§11 (N1–N21) is also complete — see §11.7.** Four shipped (N9, N13, N4, N8-via-D8a),
> N2 delivered as an instrument, the rest closed on measurement or rehomed to §12.2.
> **§12's disposition is in §12.7**: §12.2 (allocations) is now the richest remaining vein
> and deserves its own plan document; §12.3 was partly harvested here and its
> generalisation is written down; §12.4 is worth a low-yield pass; §12.5 remains
> unopened and is named as such.

## 0b. DEPLOYED — 2026-08-22

Four bricks shipped off the top of the §8 queue. **Every verdict below is a COUNT,
not a clock.** The box this ran on sits at ~70% CPU from a neighbouring job, its
unpinned null arm read ±7.7%, and its pinned CPU-time instrument turned out to be
quantised to the 15.6 ms Windows tick — so no timing number here would have been
admissible, and none is quoted.

| brick | deterministic verdict | gate |
| --- | --- | --- |
| **D8a** — wire `stripes_hybrid` into `Xxh64::update` | AVX2 kernel reach **50.00% -> 100.00%** of checksummed bytes; the DECODE side went **0% -> 100%** | `xxhgold` GOLD unmoved; new boundary-phase test; 54 external frames |
| **E2** — `ldm::count_eq` -> the shipping kernel | byte-at-a-time loop deleted | LDM fingerprint **unmoved** over 40 `--long`/`--rsyncable` frames |
| **E11** — answer `covers` from `freq` | **164x / 74x / 50x** less work in that pass at L1 / L3 / L9 | `bytegate` GOLD unmoved; oracle-parity test over 516 cases |
| **D1+D2** — DTable fill + upsample via `slice::fill` | scalar store chain -> `pshuflw` broadcast + 2x`movdqu`: **16 `u16` per 5 instructions** | `bytegate` GOLD unmoved |
| **D4** — dict-crossing copy: split, don't loop | per-byte boundary re-test deleted; branch **proven covered** (`dict_CROSS=4`) and 8 C-zstd dictionary frames decoded byte-exact | coverage census + external decode |
| **D8b** — the xxh64 NEON twin | aarch64 went from **no vector kernel at all** to 32 `umlal` + 16 `xtn`/`shrn`/`shl` per tile, accumulator chains verifiably still scalar | decomposition proven on-host; aarch64 asm receipt; x86 counts + GOLD unmoved. **Not executed on hardware — see below** |

### The instruments built for this

- **`bytegate.rs`** — 18 corpora x 9 levels, each round-tripped, all compressed
  bytes folded into one number: **`GOLD BE0071FB0CB0CED9`**. This is the §9 gate-2
  size table. If it moves, the brick was not byte-identical.
- **`d8acensus.rs`** (`--features rusty_zstd/profile`) — the D8a and E11 counters.
  V1 and V2 are claims about the CALL GRAPH and about WORK; both are settled by
  counting, in one run, immune to load.
- **`xxhgold.rs`** — 49320 one-shot + 432 incremental cells -> `GOLD
  10751459475B6849`. `xxhdiff` only compares the two ARMS and cannot catch a
  refactor that changes both; this can.
- **`ldmgate.sh`** — LDM is off by default, so `bytegate` never reaches E2's code.
  This drives `--long` explicitly and fingerprints the frames.

### Two corrections to this document, found while deploying

1. **E2's named fix does not exist in the shipping build.** §4 says "call
   `crate::simd::count_eq_len`". That function is `#[cfg(test)]` **by deliberate
   decision** — its own doc comment says so ("TEST-ONLY, and rustc had been saying
   so"). The correct target is `encode::count_match`, which has the *identical*
   `(src, m, ip, limit)` signature and contract. The equivalence needed one clamp:
   `count_match` requires `limit <= src.len()`, and `limit.min(src.len())` makes
   the two agree in every case — including the one LDM's defensive `min`s guarded.

2. **Law 2's instruction-count screen must not be applied to D1/D2.** Law 2 is
   about *twins* — whole-body AVX2 recompiles, where a growing body means the
   compiler found nothing. D1/D2 replace a scalar store chain with a vector fill,
   which necessarily *grows* the static body (broadcast prologue + scalar epilogue)
   while collapsing executed stores 16:1. Static size and executed instructions move
   in OPPOSITE directions here, and the screen reads the wrong one. The admissible
   receipt is the emitted loop body, quoted above.

### D8b — what is gated, and what is explicitly NOT

The NEON kernel is written, cross-compiles clean for `aarch64-unknown-linux-gnu`,
and emits the intended instruction selection. **No aarch64 machine or emulator was
available**, so state the boundary plainly rather than implying a win:

**Gated:**
- **The decomposition** — `a*P2 = alo*P2lo + ((alo*P2hi + ahi*P2lo) << 32) mod 2^64`
  is the only thing a lane-for-lane port can get wrong, and it is arch-independent.
  `premul_decomposition_matches_mul` proves it on ANY host over 200k deterministic
  values, all lane/carry edges, and every single-bit input.
- **Instruction selection**, from the emitted aarch64 asm: 16 iterations per
  256-byte tile, and **LLVM beat the literal translation** — it fused
  `vmull_u32` + `vaddq_u64` into `umlal`, giving **2 `umlal` + `xtn` + `shrn` +
  `shl` = 5 vector ops per 2 words** where a literal port emits 8.
- **The collapse trap did NOT fire.** The documented risk (enable the vector ISA
  over the accumulator loop and LLVM folds the four independent chains into one,
  1.23x -> 1.06x) is structurally worse on aarch64, where NEON is *baseline* and
  there is no `#[target_feature]` boundary to hide behind. The receipt says it held:
  **55 scalar `mul`, 46 scalar `ror`, and zero vector `mul`** in `xxh64`.
- **No x86 regression** from factoring `stripes_pre`: instruction counts are
  byte-for-byte identical (167/122/156/76/153) and both GOLDs unmoved.

**NOT gated — do these on hardware before claiming anything:**
- That it runs correctly on a real core. Run `xxhdiff` and `xxhgold` on the target;
  both already drive the NEON path through the existing arm toggle.
- **`PRE_TILE` is almost certainly wrong for aarch64.** 256 B is the knee of an x86
  sweep chosen to amortise a non-inlinable `#[target_feature]` call. NEON is
  baseline, so the kernel *inlines* and that call cost does not exist — the whole
  reason for a large tile is gone. Re-sweep it.
- Any speed number whatsoever.

**One structural change worth noting:** adding the second kernel would have
hand-copied the tile walk, the `MaybeUninit` discipline, the accumulator unroll and
the census — the exact twin BRICK 10 deleted from the scalar path. It is now
`stripes_pre`, arch-independent, with the kernel passed in; the x86 receipt proves
the abstraction cost zero instructions.

### E12 — PRUNED on its own ceiling probe, 2026-08-22. Do not build.

E12 was rated *MEDIUM, easy*. A `profile`-gated counter on `limit_nbits`'s inner
scans (`take_e12_scan`, kept in the tree as the evidence) priced it before anything
was written:

| level | calls | adjustment steps | inner-scan element visits |
| --- | ---: | ---: | ---: |
| L1 | 1035 | 50,611 | **7,276,900** |
| L3 | 1082 | 33,038 | 3,713,094 |
| L19 | 1034 | 16,408 | 1,468,825 |

**Both premises of the item were wrong.**

1. *"O(256 × steps)"* assumed many steps. It is **15.9–48.9 steps per call** — about
   7,000 element visits per table build, against a build that also histograms `n`
   literal bytes and walks a tree. `limit_nbits` is not the dominant cost of the
   thing it lives in.
2. *"easy"* underrates the tie-break. The scan takes the **first** symbol at the best
   `nbits` **in `present` order**. Bucketing must therefore MERGE promoted symbols
   into the destination bucket by present-index, not append — promotions arrive
   sorted, so it is a merge of two sorted lists, per step. Appending is the obvious
   implementation and it silently reorders Huffman code assignment.

**The arithmetic that prunes it:** the whole ceiling is ~7.3M element visits at its
worst level — **17% of what E11 removed** — and E11 was a pass *deletion* that cost
nothing to implement and whose own effect sat below the timing floor. E12 buys a
sixth of that for an order-sensitive data-structure rewrite. Expected pipeline gain
lands under the noise floor however good the bucket is, which is `codec-measurement`
§11's prune-before-building test.

*Reverted-because-not-worth-building, not because-measured-worse — the probe is
deterministic and the numbers above are the record so this is not re-litigated. If
the surrounding stages ever shrink enough to make 7M visits resolvable, re-run
`take_e12_scan` first.*

### E4 — PRUNED on its ceiling probe, 2026-08-22. Two of three targets are not there at all.

E4 was rated *"MEDIUM, best risk-adjusted brick."* Deterministic counters over 88–92 MiB
of Silesia, all levels, `profile` build:

| E4 target | measured | verdict |
| --- | --- | --- |
| `prime_tables` | **0 iterations at every level L1–L22** | **not on the one-shot path at all** |
| `fill_*_after_match` | **<=2 positions per call** (`match_ip+2` and one near `match_end`, each separately gated) | tile too small to exist |
| lazy / bt back-fill | L9: 6,079,897 sites, 54,604,770 inserts = **~9.0 per site**; L12 ~9.9; **zero at L1–L5 and L19–L22** | tile far below the measured knee |

**1. `prime_tables` is a V1-class finding, not an optimisation target.** It reads zero
on every level because it only runs when a prefix exists — dictionary, MT overlap, or
streaming. Optimising it would have been measured on a path the one-shot board never
takes, which is exactly the mistake D8a existed to fix. *(First read of this counter was
taken WITHOUT `--features profile` and returned zero for a different reason; the zero
survived re-measurement with the feature on. A counter reading zero is a trigger to
check the instrument before believing it — see `rusty-curiosity`.)*

**2. The fill helpers hash two positions.** E4's fix is "compute hashes for a tile of N
positions with a vector kernel, then run the scalar store loop." N is 2. An 8-wide
kernel is 4x over-provisioned and, by **Law 1**, a narrow `#[target_feature]` helper
called from a baseline caller *is a call* — measured twice on this codec at 7
instructions against 4 inline SSE, a deterministic loss before any clock.

**3. The back-fill's tile is ~9 positions.** That is the only live target, and it exists
only on the lazy ladder (L9/L12). Nine words is far below the knee the xxh64 hybrid
actually measured, where **32 words (256 B) was the only size that never read below
1.14x**, 128 B swung 1.05–1.32x, and 512 B fell to 0.98–1.15x. Per site you would pay
one non-inlinable call plus a scratch-array round-trip to issue a single 8-wide op and a
one-element tail.

**What the prune does NOT close.** Per `codec-measurement` §11, a prune constrains the
APPROACH it tested. The tile is too small *per site*; batching **across** sites —
accumulating positions from several matches before flushing one large tile — would clear
the knee. That is a different and much larger restructure of the lazy fill, and it should
be opened as its own item with its own ceiling probe rather than inherited from E4.

*Instruments kept in the tree: `take_prime_iters` (pre-existing), `take_lazy_fill`,
`take_opt_fill_ins`, and `examples/e4ceiling.rs` / `e4fill.rs`.*

### D3 — REFUTED on its own premise, 2026-08-22. Do not build.

D3 was the highest-rated remaining item (*HIGH, medium difficulty*). It rests on one
claim: *"For small offsets this is a `memcpy` call per period — for `offset == 1` it is
one call per byte."* **The code does not do that.**

- `offset == 1` is already special-cased to a `resize` byte-splat (band 0), which is
  **0.01% of calls**.
- The general band-4 loop recomputes `avail = out.len() - src` **inside** the loop, and
  `out` grows as it appends — so `take` DOUBLES each pass: offset, 2·offset, 4·offset…
  The loop is already logarithmic in `len/offset`.

Measured, `profile` build, 112 MiB decoded:

| | L1 | L3 |
| --- | ---: | ---: |
| band-4 calls | 12,522 (0.30% of calls) | 13,950 (0.21%) |
| band-4 bytes | 9,114,549 (13.25%) | 1,002,563 (1.18%) |
| **`extend_from_within` calls per band-4 call** | **3.40** | **3.25** |
| bytes per `extend_from_within` | **214** | 22 |

Three-and-a-bit memcpy calls of ~214 bytes each is already an efficient shape — memcpy
dispatches to the widest available width internally. Replacing that with an SSSE3
`pshufb` pattern-replicate + LUT would save perhaps two calls on 0.2–0.3% of copy calls,
for a technique the item itself rates *"correctness is subtle."*

*Refuted by READING the loop and then confirming with a counter — not by timing. The
harvest the item asked for was the right instruction; it just answered "no".*

### D6 — PRUNED on volume, 2026-08-22. The eligibility number is a trap.

D6's zstd-parity fast path applies when `high_threshold == table_size - 1`. That
condition holds far more often than expected — and it does not matter:

| level | dtable builds | eligible | eligibility | **entries the fast path would cover** |
| --- | ---: | ---: | ---: | ---: |
| L1 | 2,472 | 2,454 | **99.3%** | 501,824 (204/build) |
| L3 | 2,975 | 2,963 | **99.6%** | 629,824 (213/build) |
| L19 | 3,023 | 1,511 | 50.0% | 58,560 (39/build) |

**99.6% eligible, and still only ~630K entries across 112 MiB of decode** — about one
spread entry per 5,600 decoded bytes. That is **11x smaller than E12**, which was already
pruned on the same standard. Writing 8 symbols at a time as a broadcast `u64` would
remove ~550K stores from a decode that moves 112 MiB.

**The lesson worth keeping: eligibility is not volume.** A 99.6% hit rate reads as a
green light and would have justified building this; the entry count is what refuses it.
Always harvest the denominator, not just the predicate.

*(D6's `finalize` half — a read-modify-write scatter on `symbol_next[s]` — the item
already says to list and not build. Unchanged.)*

### D5, D9 — closed as not-worth-probing, on the items' own stated ceilings

Both were scheduled at the bottom with the outcome pre-declared, and nothing measured
this session changes that:

- **D5** (`x2_from_x1_into` gather): the item's own honest ceiling is *"AVX2 gather is
  5–12 cycles/element and frequently loses to scalar loads; libzstd does not vectorise
  this; treat a null result as the expected outcome."* Its precondition was *"build only
  if D1/D2 land AND the stage is still visible."* D1/D2 landed — and D6's probe shows the
  whole table-build stage runs a few thousand times per 112 MiB. The stage is not visible.
- **D9** (entropy front-end dot products): the item already says *"expect this one to be
  marginal"* and names why — **Law 1**. Both reductions are ≤256 elements and live inside
  `write_literals` / `select_seq_table`, the region where whole-twin AVX2 measured **+5.0%
  SLOWER**, so each would need to be a `#[target_feature]` call for a 256-element
  reduction. That is the exact shape Law 1 was written from.

Neither is refuted as an idea; both are unbuildable at the volumes this codec presents.

### E5 — LEFT OPEN, with the blocking conflict named. The only substantive item unresolved.

E5 is the largest remaining target by measured stage share (**3.0–9.1% of encode at L3**,
larger than `EncodeFseSeq` on every file), so it is not pruned here. But it is not
buildable as written, and the reason is in the source it proposes to change:

```
// The code histograms and the of_needs_comp scan were SEPARATE full
// passes over `coded`; both fold into this loop.
```

**A prior brick deliberately FUSED those passes into this loop.** E5 proposes splitting
it back, and the item already half-suspects this: *"Two passes over `coded` cost L1
traffic the single pass does not, and the current loop deliberately folded three separate
passes into one."*

**It is worse than two passes.** The histogram increments consume `llc`/`mlc`/`ofc`, so
they cannot precede the code computation. The split is necessarily:

1. serial — `offset_value_for` + the `reps[3]` recurrence, producing `ov` per sequence
2. vector — `ll_code` / `ml_code` / `of_code` + extra-bit extraction
3. scalar — the three histogram scatters and the `CodedSeq` push

That is **three passes over `coded`**, which is precisely the arrangement the fold
removed. E5 is therefore not "add SIMD to a loop" — it is "revert a measured
redundancy-elimination brick, then vectorise the middle third of it," and it must beat
the fused version net of the L1 traffic the fold was worth.

**What would decide it, in order:**
1. Recover the fold's original measurement. If the fold was worth more than the vector
   pass can return, E5 is closed without writing an intrinsic.
2. Only then, the item's own gate: instruction-count the three-pass split against the
   fused loop (Law 2), before any intrinsic.
3. Note `of_code` needs the float-exponent substitute (`_mm256_cvtepi32_ps` + exponent
   extract) because AVX2 has no integer highbit — added cost the screen must carry.

*Not pruned: the stage share is real and the idea may survive. Not built: it conflicts
with a shipped brick, and the burden is on E5 to beat it.*

### Still open at the top of the queue

**D7 is the only scheduled brick left, and it is deferred for a reason outside the
plan**: it lives in `simd.rs`, which was under active concurrent edit by another
workstream throughout this session (last written 13:05, while this was being worked).
Touching it would have collided. It is otherwise ready, it is pure parity work against
an existing oracle, and it is now the **last scalar-only aarch64 path in the codec**
after D8b — so it should be first up when `simd.rs` is quiet.

Everything else in §8 is closed: 1–4 and 7–8 shipped, 6 and 9 pruned on probes, 10–14
refuted or closed on their own ceilings above. **E1** (row match finder) remains what it
always was — bitstream-changing, needing its own plan document, corpus board and
per-corpus sign-flip discipline. **E13** was verify-only and is closed: the sweep runs at
most once per frame on a dispatch transition over a cold table, it is
memory-bandwidth-bound by the item's own reasoning, and its containing function is fully
inlined so there is no symbol to widen.

---

## 1. Scan log

Five passes, each with a different lens, so a candidate had to survive being looked
at from an angle that was not the one that found it.

### Scan 1 — the existing SIMD surface — **FINISHED**

*Lens: what is already vectorised, and is it real?*

Read `simd.rs` in full, grepped every `target_feature` / `target_arch` /
`is_x86_feature_detected` site (31 twin symbols), and counted vector intrinsics
per file.

**Returned:** intrinsics appear in exactly two files (`simd.rs` 14 hits, `xxh64.rs`
9 hits). Every other "SIMD" site is a `#[target_feature]` recompile. `prefetch_read`
exists but is **dead** — bricks 42 and 43 both measured it worse. NEON appears once
in the entire repo outside `simd.rs`, and that is a doc comment.

### Scan 2 — the decoder — **FINISHED**

*Lens: every inline loop on the decode path, block driver down to byte copy.*

Read `decode_compressed_block` / `decode_literals` / `decode_sequences` /
`copy_literals` / `copy_match` / `copy_from_decoded` in `compressed.rs`;
`table_from_weights`, `upsample_dtable`, `x2_from_x1_into`, `decode_4x_inner`,
`fast_4x2` in `huffman.rs`; the FSE dtable build in `fse.rs`; `BitRev` in `bit.rs`.

**Returned:** eight candidates (D1–D8, §3). The literal and match copies are already
tiered 16/32/64 with wide `copy_nonoverlapping` inside an AVX2 twin — that work is
done. What is **not** done is the table-build machinery (three separate
element-at-a-time fill loops) and the overlapping-copy band.

### Scan 3 — the encoder — **FINISHED**

*Lens: match finding, hashing, histograms, entropy coding, bitstream packing.*

Mapped the finder ladder (`find_fast_impl` → `find_dfast_impl` → `find_greedy` →
`find_lazy` → `chain_find_best` → the Bt ladder), read `MatchTables`,
`fast_hash_tag`, `hash4_tag_mls`, the three `fill_*_after_match` helpers,
`prime_tables`, `hist_count` / `segment_histograms`, the `EncodeSeqCode` transcode
loop, and the Huffman emitters (`emit_k`, `emit_fill`, `emit_tail`).

**Returned:** ten candidates (E1–E10, §4). The finding that matters: **the codec
already maintains per-slot tag bytes (`tags`, `ltags`, `ctags`) but compares them
ONE AT A TIME inside a serial pointer-chasing chain walk.** That is 90% of the way
to zstd's row match-finder without the part that makes it fast.

### Scan 4 — cross-cutting and infrastructure — **FINISHED**

*Lens: build baseline, portability, and the loops outside the two big files.*

Checked for `.cargo/config.toml` (none exists), read the workspace manifest,
censused NEON coverage repo-wide, and swept `ldm.rs`, `dict.rs`, `train.rs`,
`decode.rs`, `stream.rs`, `frame.rs`.

**Returned:** the compile baseline is plain `x86-64` — **SSE2 only** — which is why
every AVX2 path must be runtime-dispatched and why the `#[target_feature]` inline
barrier (§2) is a live constraint rather than a footnote. Also found `ldm::count_eq`,
a byte-at-a-time compare loop that **does not call the `count_eq_len` kernel that
already exists three modules away**.

### Scan 5 — feasibility, from emitted assembly — **FINISHED**

*Lens: stop guessing what the compiler does; read what it emitted.*

Built the library with `--emit asm` and attributed every `xmm` / `ymm` reference and
every instruction to its containing symbol.

**Returned:** the §0 receipt, the per-twin widening-headroom table (§6), and the
decisive negative result — **the 17-twin AVX2 promotion sweep has already been run**
and is recorded in `huffman.rs:decode_4x`: enabling avx2 on every bmi2-only twin
*adds 38,051 instructions overall and only two shrink.* Both survivors are already
shipped. This scan closed the lever the first four scans were converging on, and is
the reason this plan is a kernel plan rather than a twin plan.

### Validation pass — exhaustive loop census — **FINISHED**

*Lens: the first five scans were targeted. Enumerate EVERY loop mechanically and
prove nothing was missed.*

Scans 1–5 followed the hot stages, which is the right way to find the big items and
the wrong way to prove completeness. This pass enumerated **all 232 non-test loops**
in the crate (`for` / `while` / `loop`, each attributed to its enclosing function)
and walked the list.

| file | loops | file | loops |
| --- | ---: | --- | ---: |
| `huffman.rs` | 64 | `mt.rs` | 7 |
| `encode.rs` | 52 | `decode.rs` | 6 |
| `fse.rs` | 25 | `prof.rs` | 5 |
| `train.rs` | 20 | `ldm.rs` | 4 |
| `compressed.rs` | 14 | `seekable.rs` | 4 |
| `simd.rs` | 13 | `in_bench.rs` / `stream.rs` | 2 each |
| `xxh64.rs` | 12 | `bit.rs` / `params.rs` | 1 each |

**It found eight items the targeted scans missed, one of them serious**, and it
confirmed four judgements as correct. The misses cluster in one place and share one
cause: scans 2 and 3 followed the *per-byte* and *per-sequence* loops and skipped the
**per-symbol entropy front-end** — the 256- and 2048-entry table passes that run a
few times per block. Individually small, collectively a class (V5).

The serious one (V1) is different in kind: it is not a missed loop but a **missed
call graph**. The catalogue below carries all eight as V1–V8, and the revisions they
force are folded into §3, §4, §7 and §8.

---

## 1b. What the validation pass found

### V1 — the xxh64 AVX2 kernel is unreachable from the codec — **CRITICAL**

The hybrid `stripes_hybrid` / `premul_p2_avx2` — the codec's *only* hand-written
vector kernel besides `count_eq_len`, carrying a documented **1.14–1.26×** — is
called from exactly one place: the free function `xxh64_seed`.

**Nothing on the shipping path calls it.** Traced:

| caller of `xxh64_seed` / `xxh64()` | what it is |
| --- | --- |
| `encode.rs:13599` | a unit test |
| `rusty_zstd-bench/examples/xxhsweep.rs` | the bench that measured the 1.14–1.26× |

Every real checksum goes through the **streaming** type instead:

| site | path |
| --- | --- |
| `encode.rs:1406` — `encode_oneshot` | `Xxh64::new()` → `update()` |
| `decode.rs:443` — frame verify | `Xxh64::new()` → `update()` |
| `stream.rs:110, 356, 499` | `Xxh64::new()` → `update()` |

And `Xxh64::update` does **not** call `stripes_hybrid`. Its 128-byte and 32-byte
loops both call `consume_stripe`, which is the scalar `stripe`. So the encoder and
the decoder run a fully scalar xxh64.

`DecodeChecksum` is **22.2–25.9%** of decode on `incomp-32m` / `zeros-32m`, **24.1%**
on `text-32m` and **22.3%** on `versions-16m`. The win is sitting behind an API the
codec does not use.

This is the `rusty_curiosity` case of *a beautifully clean number that turns out to
measure nothing* — and the first pass produced it by reading the kernel and assuming
reachability instead of tracing callers. **It rewrites D8: the first move is wiring,
not NEON.**

### V2 — `HuffCTable::covers` is a redundant O(n) literal walk

`huffman.rs:encode_literals_section` builds `segs` (per-segment histograms) and sums
them into `freq`, then ~20 lines later calls `prev_ct.covers(lits)`, which walks
**every literal byte again** asking "does every byte present have a code?"

That is answerable from `freq` in **256 iterations**: symbol `s` is present iff
`freq[s] != 0`, and the test is then `prev_ct.entry[s] >> 16 != 0`.

This is brick 74's exact defect class, recurring. Brick 74's own comment says it:
*"A brick that removes expensive work can still add cheaper work nobody counted."*
It removed the double histogram walk and left `covers` as a third walk over the same
buffer — on `mozilla`'s 24.4 MB of literals, on every block with a reusable table.
**Not a SIMD problem. Delete the pass.**

### V3 — `all_same` is a fourth O(n) literal walk

Same function, line 1961: `lits.iter().all(|&b| b == lits[0])`. Also answerable from
`freq` (exactly one non-zero bin), though it runs *before* the histogram today, so
fixing it means reordering rather than deleting. Lower value than V2; recorded so the
walk count over the literal buffer is known: **`all_same`, `literals_worth_huffman`
(sampled, bounded — correctly so), `segment_histograms`, `covers`.**

### V4 — backward match extension is one byte per iteration, in four finders

Forward extension uses the AVX2/NEON `count_eq_len` kernel. **Backward extension does
not.** `back_eq` is a single-byte compare:

```rust
fn back_eq(src: &[u8], s: usize, mm: usize) -> bool {
    unsafe { *src.get_unchecked(s - 1) == *src.get_unchecked(mm - 1) }
}
```

called from a `while` loop in **four** places — `emit_fast_seq_body` (7579),
`find_greedy_impl` (9148), `find_lazy_impl` (9757), `find_bt_lazy` (11192).

**Named reason auto-vec fails:** reverse-direction compare with an early exit, plus
two independent lower bounds (`anchor`, `frame_start`) in the loop condition.
**Fix:** a reverse common-suffix kernel — the mirror of `count_eq_len`, using
`_mm256_cmpeq_epi8` with `leading_ones` instead of `trailing_ones`.
**Honest ceiling:** backward extension is bounded by the literal run, and the L3
literal-run mean is ~3.75 bytes — so most calls die in 1–2 iterations and a wide load
would be pure overhead. **This is very likely E10's argument again** (word-first,
vector-never). Harvest `take_lit_hist()` (the literal run-length histogram, which
already exists) **before** building anything. Recorded as a real asymmetry with a
probably-null ceiling, not as a scheduled brick.

### V5 — the missed CLASS: per-symbol entropy front-end passes

Scans 2 and 3 followed per-byte and per-sequence loops. These run a few times per
block over 256- or 2048-entry tables, and collectively they are the bulk of
`huffman.rs`'s 64 loops and `fse.rs`'s 25.

| site | shape | size | named reason auto-vec fails |
| --- | --- | --- | --- |
| `huffman::body_bytes_exact` | `bits += f * nb` | 4 × 256 | data-dependent `return None` inside the loop |
| `fse::FseCTable::bit_cost` | `c += n * nb` | ≤ 256, ×3/block × candidate tables | same early return |
| `huffman::ctable_from_nbits` | DTable → codes | 2048 | **scatter** + first-write-wins order dependence |
| `huffman::ctable_from_weights` | same | 2048 | same |
| `huffman::pack_huff_entries` | `nbits`+`code` → `u32` | 256 | none — should already vectorise; verify |
| `huffman::huff_mean_nbits_x10` | weighted mean | 256 | reduction with a zero-skip |
| `encode_literals_section` | `freq[s] += c` fold | 4 × 256 | none — should already vectorise; verify |
| `huffman::write_tree_fse` | weight histogram | ≤ 256 | scatter |
| `fse::normalize_count` | two proportion passes | ≤ 256 | `largest` argmax carried across |

**The two dot products (`body_bytes_exact`, `bit_cost`) are the real candidates.**
Both are `sum(count[s] × nbits[s])` with a `count == 0` skip that is *free to drop*
(multiplying by zero adds zero) and an error exit that can be hoisted to a masked
compare. `_mm256_mullo_epi32` widened to 64-bit accumulation is the kernel.
Both sit inside `write_literals` / `select_seq_table` — the region where whole-twin
AVX2 measured **+5.0% slower** (§7), so these must be **targeted kernels, not a twin
promotion.** Law 1 applies: they need to be inlined into an AVX2-compiled caller or
they will be a call.

### V6 — `limit_nbits` rescans 256 symbols per adjustment step

`huffman.rs:1288` and `:1304` — two `while kraft ≷ target` loops, each doing a **full
linear scan over `present` to find an argmin/argmax on every single adjustment.**

**The right fix is not SIMD.** Bucket the symbols by `nbits` (values are 1..=11, so
eleven small lists) and each step becomes O(1) instead of O(256). A vector argmin
would make a quadratic loop 8× less quadratic; the bucket makes it linear.
Filed under `codec-eliminate-redundancy`, listed here so the census is complete.

### V7 — the hash-table tag-unpack sweep

`encode.rs:3740`, inside `find_sequences_strategy_sel`:

```rust
for e in tables.hash.iter_mut() { *e &= 0x00FF_FFFF; }
```

An element-wise AND over the whole hash table — up to `1 << 24` u32, i.e. **64 MB**.
It is a simple enough pattern that LLVM should already vectorise it; it also fires at
most once per frame (on a dispatch transition). **Verify the emitted form, then leave
it alone.** It is memory-bandwidth bound, not compute bound.

### V8 — `rle_byte` is already word-at-a-time, and should stay that way

`encode.rs:2732` compares the block against a splatted byte 8 at a time, then a byte
tail. It looks like an AVX2 candidate and is not: **the overwhelming majority of
blocks are not RLE and fail in word 0**, so a 32-byte load would be setup cost for a
result the first 8 bytes already decided. **This is E10's argument verbatim.**
Recorded in §7 as closed so it is not re-flagged by the next census.

### Confirmed correct — four judgements the validation pass upheld

- **`find_opt`'s DP is genuinely serial** (`price[i+1]` depends on `price[i]`), and
  its array setup is *already* optimised: `reset_to` fills only `price`, and the
  other four arrays' per-block resets were deliberately deleted (~17 bytes of memset
  per input byte doing nothing). No opportunity; the first pass was right to skip it.
- **`count_match`'s sub-8 tail** is already one masked `u64` compare, not a byte loop.
- **`literals_worth_huffman` / `lit_sample_peak` are sampled** (`SAMPLE = 1024`, odd
  stride), so they are O(1) per block, not O(n). Not a miss.
- **`load_u64le_tail`'s byte loop** survives only for sub-8-byte inputs; the backoff
  load already handles the general case.

---

## 2. The two laws already earned here

Both were paid for with real measurements. Any brick in this plan violates them at
its own risk.

**Law 1 — the `#[target_feature]` inline barrier.** A `target_feature` function
cannot be inlined into a caller that lacks the feature, and this crate compiles at
baseline SSE2. So a narrow AVX2 helper becomes a **call** from a baseline caller.
Measured twice: the 32-byte match copy became `2 movs + callq` + a 4-instruction
callee (7 instructions) against 4 inline SSE instructions — a deterministic loss
before any clock. *The only shape that pays is a whole loop under `#[target_feature]`
with the kernel `#[inline(always)]` inside it.*

**Law 2 — the instruction-count screen.** Enabling AVX2 on a twin is worth it
**where the body's instruction count DROPS and never where it grows.** Three
measured data points:

| twin | instrs bmi2 → avx2 | measured | verdict |
| --- | --- | --- | --- |
| `write_sequences` | 11,640 → 10,590 (**−1,050**) | −1.8% | **shipped** |
| `decode_4x` | 6,436 → 6,311 (**−125**) | — | **shipped** |
| `write_literals` | 3,659 → 4,404 (**+745**) | **+5.0% SLOWER** | **reverted** |

The screen is deterministic, needs no pinning and no noise floor, and costs one
build. **Run it before any clock, on every candidate in this plan.**
(The `write_literals` numbers are quoted from its own source comment; the two
shipped rows are re-counted from this build. Per `knob-census-drift`, re-count —
never quote — before banking.)

---

## 3. DECODER opportunities

Ranked by expected value. "Named reason" is the required justification from
`codec-vectorize-kernel`: you may not hand-write a kernel without naming why the
auto-vectoriser cannot.

### D1 — Huffman DTable fill: broadcast store — **HIGH, easy**

`huffman.rs:table_from_weights`, the innermost loop:

```rust
for k in 0..length {
    *table.get_unchecked_mut(pos + k) = u16::from(sym) | (u16::from(nb_bits) << 8);
}
```

The stored value is **loop-invariant**. `length` is `1 << (w-1)` and reaches 1024 at
`table_log` 11. This is a `memset` of a `u16`, written as a scalar store loop.

**Named reason auto-vec fails:** the value is recomputed inside the loop body from
two `u16::from` conversions, and the bound is a runtime `pos + length` that LLVM
re-proves per iteration through `get_unchecked_mut`; the emitted form is a scalar
store chain. **Fix:** hoist the entry to a `let e`, then
`table[pos..pos+length].fill(e)` — `slice::fill` lowers to a vector store loop with
no `unsafe` and no intrinsics. An explicit `_mm256_set1_epi16` kernel is the fallback
only if `fill` under-performs.
**Risk:** none — byte-identical by construction, and it *removes* unsafe.
**Gate:** deterministic size table + instruction count.

### D2 — `upsample_dtable`: replicate-expand — **HIGH, easy**

Same shape, one level up. Each source `u16` is replicated `factor = 1 << scale`
times, walking the source **downward** to keep the in-place expansion safe.

**Named reason auto-vec fails:** reverse iteration over overlapping in-place ranges.
LLVM's dependence analysis cannot prove the source and destination windows disjoint,
so it will not vectorise the outer walk at all. **Fix:** the inner `for k in 0..factor`
is still a pure broadcast — `wide[base..base+factor].fill(e)` — which is vectorisable
even though the outer walk is not. **Risk:** none, byte-identical.
**Note:** D1 and D2 are one brick. They run per Huffman table build, i.e. per block
on `mr`-shaped content where `DecodeLiterals` is 69.6% of decode.

### D3 — overlapping match copy: pattern replicate — **HIGH, medium difficulty**

`compressed.rs:copy_from_decoded`, band 4 — `offset < len`. The tiered 16/32/64
fast paths all require `offset >= width`; everything below falls to:

```rust
while copied < len {
    let take = (len - copied).min(out.len() - src);
    out.extend_from_within(src..src + take);
    copied += take;
}
```

For small offsets (`offset` 1–15, i.e. run-length and short-period patterns) this is
a `memcpy` call **per period** — for `offset == 1` it is one call per byte.

**Named reason auto-vec fails:** the source overlaps the destination by construction;
no vectoriser may reorder it. **Fix:** the libzstd technique (`ZSTD_overlapCopy`) —
build one 16-byte repeating pattern with `_mm_shuffle_epi8` against a 16-entry
per-offset permutation LUT, then store it wide. This is **SSSE3, not AVX2**, which
matters: it is close enough to baseline to be worth checking whether it can ship
without a twin, sidestepping Law 1 entirely.
**Instrument already exists:** `take_dec_bands()` reports band 4's call count and
`take_dec_untiered()` its length histogram. **Harvest before building** — the 64-byte
tier was sized from exactly this histogram, and its mean would have chosen the wrong
width where the histogram chose right.
**Risk:** medium — correctness is subtle; needs the existing
`copy_from_decoded_matches_byte_push` test extended to every `offset ∈ 1..=32`.

### D4 — dict-crossing match copy: kill the byte loop — **MEDIUM, easy**

`compressed.rs:copy_match` pushes **one byte at a time** across the
dictionary/frame boundary, re-testing `src_pos < dict.len()` per byte:

```rust
for _ in 0..len {
    let b = if src_pos < dict.len() { dict[src_pos] } else { … };
    out.push(b);
    src_pos += 1;
}
```

**This is not a SIMD problem — it is a decomposition problem**, and it is exactly the
"pull out slower older processing" the mission names. The boundary is crossed *once*.
**Fix:** split at the boundary, then two slice copies (`extend_from_slice` for the
dict part, `copy_from_decoded` for the frame part). Vector stores fall out for free.
**Risk:** low. Dictionary decode only — scope it honestly before ranking it up.

### D5 — `x2_from_x1_into`: gather — **LOW confidence, do last**

A full pass over 2,048 entries per table build, each doing a data-dependent load
`table[second_index & mask]`.

**Named reason auto-vec fails:** **gather**. `_mm256_i32gather_epi32` is the
instruction. **Honest ceiling:** AVX2 gather is 5–12 cycles/element on most
microarchitectures and frequently loses to scalar loads. libzstd does not vectorise
this. **Build only if D1/D2 land and the stage is still visible**, and treat a null
result as the expected outcome.

### D6 — FSE dtable build: the zstd spread fast path — **MEDIUM**

`fse.rs:from_norm_buf`. Two loops, and they must be judged separately:

- The **spread** (`position = (position + step) & mask`) is *serially dependent* —
  each position needs the previous. Not vectorisable as written. But libzstd has a
  fast path this codec lacks: when `high_threshold == table_size - 1` (no
  low-probability symbols), symbols can be written **8 at a time as a broadcast
  `u64`** because the sequence is then known-regular. That is a real, zstd-parity
  move.
- The **finalize** (`for item in decode.iter_mut()`) is a read-modify-write on
  `symbol_next[s]` at a data-dependent index — a **scatter/histogram dependency**.
  Genuinely hard; list it, do not build it.

**Named reason auto-vec fails:** serial recurrence (spread) and scatter (finalize).
The same pair recurs in `FseCTable::from_norm`, which has its own spread and a
scatter into `state_table`.

### D7 — `count_eq_len_neon` parity with the AVX2 twin — **MEDIUM, easy**

Read side by side, the NEON kernel is missing two things its x86 twin has:
1. the **64-byte double-block tier** (AVX2 does two 32-byte compares per iteration
   before looping; NEON does one 16-byte compare),
2. the **overlapped sub-8 tail** (AVX2 replaces up to 7 byte-compare iterations with
   one shifted `u64` compare; NEON still runs the byte loop).

**Risk:** none — the scalar twin is already the oracle and
`count_eq_len_matches_byte_and_words` already covers it. Pure parity work.

### D8 — xxh64: wire the existing kernel in, THEN add NEON — **HIGH (revised by V1)**

**This item was rewritten by the validation pass.** It was filed as "aarch64 lacks a
NEON twin," which is true and is the *second* problem. The first is worse:

**D8a — the AVX2 kernel is unreachable from the codec.** Per V1, `stripes_hybrid` is
called only from `xxh64_seed`, and the shipping encoder (`encode.rs:1406`), decoder
(`decode.rs:443`) and streaming API (`stream.rs`) all use `Xxh64::update`, whose
128-byte and 32-byte loops call the scalar `consume_stripe`. **x86-64 is running a
scalar checksum today**, and the documented 1.14–1.26× is measured on a path nothing
takes.

**Fix:** route `Xxh64::update`'s bulk loops through `stripes_hybrid` exactly as
`xxh64_seed` does — take the hybrid's `done` byte count, then let the existing 32-byte
loop resume from there. The buffering discipline (`buf`, `buf_len`, `large`) is
unchanged; only the bulk path moves. **This is plumbing, not a kernel** — the kernel
already exists, is tested, and has an A/B arm.
**Risk:** low, but this is a **format checksum**: the streaming and one-shot results
must stay bit-identical to each other and to the reference. The existing
`frame_checksum_matches_oneshot_xxh64` test is exactly the right gate and must be
extended to drive `update()` in **irregular chunk sizes** (1, 7, 31, 32, 33, 127,
128, 129) so the hybrid's stripe boundary is exercised against the buffer boundary.

**D8b — then the NEON twin.** `premul_p2_avx2` is `#[cfg(target_arch = "x86_64")]`
with no aarch64 sibling, so **every aarch64 build runs the scalar stripe loop** even
after D8a. Direct translation: `vmull_u32` / `vshrq_n_u64` / `vaddq_u64` mirror
`_mm256_mul_epu32` / `_mm256_srli_epi64` / `_mm256_add_epi64` exactly, at 2 lanes
instead of 4. **The hard-won constraint transfers**: the accumulator half must stay
*outside* the `#[target_feature]` function or LLVM re-vectorises the four scalar
chains and the win collapses (documented: 1.23× → 1.06×). `PRE_TILE` must be
re-swept on aarch64 — 256 B was chosen against an x86 call cost.

**Do D8a first.** It is cheaper, it is on x86 where the board runs, and until it
lands the NEON twin would be wired into the same dead path.

### D9 — the entropy front-end dot products — **MEDIUM (new, from V5)**

`huffman::body_bytes_exact` (4 × 256) and `fse::FseCTable::bit_cost` (≤ 256, called
three times per block and once per candidate table) are both
`sum(count[s] × nbits[s])` reductions.

**Named reason auto-vec fails:** each carries a data-dependent `return` inside the
loop body (`nb == 0 → None`, `!can_encode_symbol → u64::MAX/4`), which forbids
vectorisation outright.
**Fix:** the `count == 0` skip is free to drop (multiplying by zero adds zero), and
the error exit hoists to a masked compare evaluated once after the reduction.
`_mm256_mullo_epi32` with 64-bit accumulation is the kernel.
**Constraint:** both live inside `write_literals` / `select_seq_table`, the region
where whole-twin AVX2 measured **+5.0% slower** (§7). These must be **targeted
kernels**, and Law 1 means each needs to be inlined into an AVX2-compiled caller or
it becomes a call — which for a 256-element reduction is likely a net loss.
**Screen with the instruction count first; expect this one to be marginal.**

---

## 4. ENCODER opportunities

### E1 — the row-based match finder — **HIGHEST VALUE, HIGHEST COST**

> **CEILING PROBE, 2026-08-24 -- E1 CANNOT HELP AT L3, AND THE PARAGRAPH BELOW
> SAYS WHY BY ACCIDENT.** `examples/rowceiling.rs` counts `WALK_EXAM`, the serial
> dependent chain-table loads a row scan would delete. Over 125.1 MiB of Silesia:
>
> | level | strategy | chain loads | loads/KiB |
> |---|---|---:|---:|
> | L1 | fast | **0** | 0.0 |
> | L3 | dfast | **0** | 0.0 |
> | L5 | greedy | 43,009,953 | 335.7 |
> | L7 | lazy | 139,427,236 | 1088.1 |
> | L9 | lazy2 | 220,979,813 | 1724.6 |
> | L12 | lazy2 | 674,337,493 | 5262.8 |
>
> **L3 walks no chain at all.** `Strategy::DFast` uses two hash tables and never
> enters `chain_find_best_inner`, so the row finder's ceiling at L3 is not small
> -- it is EXACTLY ZERO. The text below cites `MatchFind` leading on 18/18
> corpora **at L3** and then names `chain_find_best_inner` as its core; those are
> two different finders, and every headline gap on the m7 board is measured at L3.
>
> This does not refute E1. It RELOCATES it: E1 is an **L5-L12 lever**, priced at
> 336-5263 dependent loads per KiB, and it must be boarded there. Do not justify
> it with an L3 profile again.


This is the single largest SIMD opportunity in the codec, and it is not close.

`MatchFind` is the encode leader on **18 of 18** corpora (57.4–82.6% at L3). Its core
is `chain_find_best_inner`: a **serial pointer chase**, one dependent load per step,
with a one-byte tag compared **one at a time**:

```rust
if tag_filter && m != 0 && mtag != gtag { … m = next; continue; }
```

The codec already has all the raw material — `tags`, `ltags`, `ctags`, packed-tag
slots, and a proven tag-soundness argument (a tag mismatch cannot hide a match,
because acceptance verifies the bytes the tag is a function of).

What it does not have is the layout that makes tags pay. zstd's `ZSTD_row_*`
match-finder replaces the chain with **rows of 16/32 positions sharing a hash
bucket**, with the row's tags contiguous, and compares **all of them in one
instruction**: `_mm_cmpeq_epi8` + `_mm_movemask_epi8`, then iterates the set bits.
One dependent load per *row* instead of per *candidate*.

**Named reason auto-vec fails:** the candidates live in a linked list, not an array.
No vectoriser can transform a pointer chase into a parallel compare — that is a data
structure change, which is why this is a build and not a compiler flag.

**Two properties make this attractive here beyond the raw speedup:**
- The kernel is `_mm_cmpeq_epi8` / `_mm_movemask_epi8` — **SSE2, the compile
  baseline.** It needs no `#[target_feature]`, so **Law 1 does not apply** and it
  inlines into the existing finders. The NEON mirror (`vceqq_u8` plus a `shrn`-based
  mask narrow) is the same shape as the kernel already in `simd.rs`.
- The tag-soundness proof this codec already wrote is exactly the proof the row
  finder needs.

**The honest cost, stated up front:**
- **This is NOT byte-identical.** A row finder examines a different candidate set,
  so it finds different matches and emits a different bitstream. It cannot be gated
  on the size table. It must ship as a **new strategy** behind a parameter (zstd's
  own `useRowMatchFinder`), gated on `us/c size` per corpus **and** speed, with the
  existing chain finders untouched and default.
- It is a multi-brick build: row layout, insert, `getMatchMask`, the bit-walk, and
  wiring into greedy/lazy/lazy2.
- Per `codec-campaign-laws`: **"more is not monotonically better."** Filling the
  chain over repcode spans made `versions-16m` 84% slower *and* the ratio worse. A
  row finder changes which positions are reachable; expect sign-flips across content
  and read the per-corpus spread, never the mean.

**Recommendation:** this is the right target and the wrong first brick. Land D1/D2/E4
first to prove the kernel discipline on byte-identical ground, then open E1 as its
own campaign with its own plan document.

### E2 — `ldm::count_eq` does not use the kernel that exists — **FREE, do first**

```rust
fn count_eq(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    let max = …;
    let mut n = 0usize;
    while n < max && src[m + n] == src[ip + n] { n += 1; }
    n
}
```

A **byte-at-a-time bounds-checked compare loop**, in a codec that ships an
AVX2/NEON/word-loop common-prefix kernel with a tested scalar oracle.
**Fix:** call `crate::simd::count_eq_len` on the two slices with the length clamped
to `max`. One line. Byte-identical by definition — both compute the same common
prefix length.
**Scope honestly:** LDM is `enable_ldm: false` by default, so this is **off the
shipping path**. It is on this list because it is free and because the same oversight
class is what to hunt for elsewhere, not because it moves the board.

### E3 — `hist_count`: the 256-bin histogram — **MEDIUM, low confidence**

`huffman.rs:hist_count` is already the good scalar shape — four round-robin
sub-tables to break the store-to-load forwarding stall on repeated bytes, matching
C's `HIST_count_parallel`. `Huff` reaches **86.7% of encode at L1 on `x-ray`** and
16–21% on `ooffice` / `sao` / `osdb` / `mr`.

**Named reason auto-vec fails:** **scatter** — 256 data-dependent increments.
**What is actually available:** the honest answer is *not much*. The known vector
histogram techniques (nibble-split `pshufb` accumulation; `vpconflictd` on AVX512)
either need a wider ISA than baseline or only pay for small alphabets. libzstd does
not vectorise this either. **Cheaper adjacent win:** `segment_histograms` computes
four independent histograms over four disjoint slices — that is **thread**-parallel,
not lane-parallel, and it fits the existing `mt` machinery.
**Verdict:** list it, harvest it, do not build a lane-parallel version without a
refutable ceiling estimate first.

### E4 — batch hashing in the fill/prime paths — **MEDIUM, best risk-adjusted brick**

`prime_tables` walks up to a window of positions on a stride, hashing each one
independently; `fill_fast_after_match` / `fill_hash_after_match` /
`fill_hash_long_after_match` hash 2 positions per match; the `lazy` / `bt` back-fills
walk spans on a stride.

Each hash is `load → mask → multiply → shift → xor-fold`. Per position these are
**fully independent** — the only serial part is the table *store*.

**Named reason auto-vec fails:** the load/hash/store are interleaved in one loop body,
so LLVM sees a loop with stores to a data-dependent index (scatter) and gives up on
the whole body, including the independent hash half.
**Fix — the decomposition `xxh64` already proved:** split the loop. Compute hashes
for a tile of N positions into a scratch array with a vector kernel, then run the
scalar store loop over the tile. `xxh64`'s hybrid is the working precedent, including
its two traps: the vector half **must** live in its own `#[target_feature]` function
and the scalar half **must not** (or LLVM re-vectorises the scalar side and the win
collapses), and the tile must be sized to amortise the call (256 B was the knee there).
**Constraint:** the wide (`u64 × FAST_HASH_PRIME64`) hash has **no AVX2 64-bit
multiply**. It needs the `_mm256_mul_epu32` lo/hi decomposition — which
`premul_p2_avx2` already implements and documents. The narrow 4-byte hash is a
straight `_mm256_mullo_epi32`, 8 positions per instruction.
**Risk:** byte-identical (same hashes, same stores, same order) — gate on the size
table.

### E5 — `EncodeSeqCode` transcode — **MEDIUM, needs decomposition**

3.0–9.1% of encode at L3 (larger than `EncodeFseSeq` on every file), with *exactly
one decision*. Per sequence: `offset_value_for`, a repcode shuffle, `ll_code` /
`ml_code` (LUT), `of_code`, three histogram increments, and a `CodedSeq` push.

**Named reason auto-vec fails:** the `reps[3]` array is **carried across iterations**
— a genuine serial recurrence — and the histogram increments are scatters.
**The decomposition:** only the rep shuffle is serial. `of_code`, `ll_code`, `ml_code`
and the extra-bit extraction are pure per-element functions of `(offset, litlen,
matchlen, ov)`. A two-pass split — serial pass computes `ov` per sequence, vector pass
computes the three codes and their extra bits — is possible. `of_code` is a
`31 - leading_zeros` highbit, which AVX2 lacks; the float-exponent trick
(`_mm256_cvtepi32_ps`, then extract the exponent field) is the standard substitute.
**Confidence:** medium. Two passes over `coded` cost L1 traffic the single pass does
not, and the current loop deliberately folded three separate passes into one.
**Measure the split's instruction count before writing any intrinsic.**

### E6 — Huffman bitstream emitters — **LOW, list only**

`emit_k` / `emit_k5` / `emit_fill` / `emit_tail` pack variable-length codes into a
serial 64-bit container. **Named reason auto-vec fails:** the container's bit position
after symbol *i* depends on every symbol before it — an unbreakable serial recurrence.
libzstd does not vectorise this. **Listed for completeness; do not build.**
The *table build* upstream (`huffman_nbits`, `limit_nbits`, `ctable_from_nbits`) is
where any Huff-stage win lives, and per Law 2 that region is precisely where AVX2
measured **+5.0% slower** — see §7.

### E7 — `ldm::rsync_cut` rolling hash — **LOW**

Byte-at-a-time `rotate_left(1) + mul` rolling hash with an early exit.
**Named reason auto-vec fails:** serial recurrence plus a data-dependent early exit.
A parallel-prefix formulation exists (the rotate is a linear operator) but only pays
if the scan usually runs long. **`--rsyncable` only.** List, do not build.

### E8 — dictionary training loops — **LOW, cold**

`train.rs` (`packed_dmer`, the `for i in 0..=sample.len()-d` sweeps),
`dict::Dictionary::from_bytes` (860 xmm ops) and `train::finalize_dictionary`
(926 xmm ops) are the largest pure-SSE bodies outside the finders. They are also
**dictionary training** — run once, offline, off every shipping path.
**Correct verdict: do not spend vector work here.** Recorded so a future census does
not re-flag them.

### E9 — `MatchTables::clone` — **LOW, but suspicious**

505 xmm ops, 0 ymm. This is a bulk copy of the hash/chain/tag tables, on the
multi-threaded path (one clone per worker). It is a *memory* problem, not a lane
problem — `codec-memory-copies`, not this plan. Flagged so it is not lost.

### E10 — `count_eq_len`'s word-first ladder is correct; leave it alone — **CLOSED**

Recorded here because it looks like an opportunity and is not. `count_eq_len_ge8`
runs **words to 32 bytes before any vector**, because the L19 histogram (511M calls)
shows **50.5% die in word 0 and 83% by byte 31** — 83% of calls never need a ymm
register, its power-up, or the `vzeroupper` on exit. Widening the entry would be a
regression. The `EQLEN_ARM` / `take_eqlen_stats` instruments exist to re-adjudicate
this on other microarchitectures; use them rather than assuming.

### E11 — delete `covers`' O(n) literal walk — **HIGH, easy (new, from V2)**

Not a vector brick. `encode_literals_section` walks the literal buffer **four times**
per block on the table-reuse path: `all_same`, `literals_worth_huffman` (sampled —
correctly bounded), `segment_histograms`, and then `prev_ct.covers(lits)`.

`covers` asks "does every byte present in `lits` have a code?" — which the `freq`
array computed twenty lines earlier already answers, in **256 iterations instead of
n**: symbol `s` is present iff `freq[s] != 0`, and the test is
`prev_ct.entry[s] >> 16 != 0`.

**Byte-identical by construction** — same predicate, same answer, computed from the
same counts.
**Why it matters here:** `Huff` reaches 86.7% of encode at L1 on `x-ray` and 16–21%
on four other corpora, and this is the stage where AVX2 widening measured **+5.0%
slower**. The stage does not need wider instructions; it needs **fewer passes**.
This is `codec-eliminate-redundancy`, which the house rules say to reach for *first*,
before any SIMD — and it is the highest-confidence encoder item in the plan.

**`all_same` (V3)** is the fourth walk and is also answerable from `freq` (exactly one
non-zero bin), but it runs *before* the histogram today, so it needs a reorder rather
than a deletion. Lower value; bundle it only if the reorder proves free.

### E12 — `limit_nbits`: bucket by nbits — **MEDIUM, easy (new, from V6)**

Two `while kraft ≷ target` loops, each doing a **full linear scan over `present` to
find an argmin/argmax on every adjustment step** — O(256 × steps).

**The right fix is not SIMD.** `nbits` values are 1..=11, so bucket the symbols into
eleven small lists and each step becomes O(1). A vector argmin would make a quadratic
loop 8× less quadratic; the bucket makes it linear.
**Risk:** the tie-breaking must be preserved exactly — the current scan takes the
*first* symbol at the best `nbits` in `present` order, and the Huffman code
assignment downstream is order-sensitive. **Gate on the deterministic size table**,
which will catch any tie-break drift immediately.

### E13 — the hash-table tag-unpack sweep — **VERIFY ONLY (new, from V7)**

`for e in tables.hash.iter_mut() { *e &= 0x00FF_FFFF; }` — element-wise AND over up
to `1 << 24` u32 (64 MB). Simple enough that LLVM should already vectorise it, and it
fires at most once per frame on a dispatch transition. It is **memory-bandwidth
bound, not compute bound**. Check the emitted form once; if it is already wide, close
it. Listed so the census is complete, not because it is expected to pay.

## 5. Cross-cutting / infrastructure

**X1 — the compile baseline is SSE2.** There is no `.cargo/config.toml` in the repo,
so `x86-64` (SSE2) is the floor and `neon` is the aarch64 floor. Consequences:
every AVX2 kernel needs `#[target_feature]` + runtime CPUID dispatch, and Law 1
binds. **Do not "fix" this with `-C target-cpu=native`** — it makes the binary
non-portable and silently deletes the dispatch the crate is built around.

**X2 — aarch64 is one kernel deep.** `count_eq_len_neon` is the entire NEON surface.
`xxh64`'s hybrid (D8) is x86-only, and the AVX2/NEON tier parity gap (D7) is real.
Note the BMI2 twins are **not** an aarch64 gap: aarch64 has cheap variable shifts and
needs no `bextr` / `shrx` equivalent.

**X3 — the instruction-count screen is the gate.** Free, deterministic, no pinning,
no noise floor. Run it on every candidate here before any clock (Law 2).

**X4 — the CPUID dispatch pattern is already correct.** `has_avx2` / `has_bmi2` use a
cold-outlined detect with a hot single-load fast path, after inlining `std_detect`'s
probe cost 6 registers of prologue per dispatch site. **Reuse them; do not add a
third detector.** Any new kernel needing a new feature bit (SSSE3 for D3) should
follow the same shape.

**X5 — guard on the feature you compiled with.** `decode_4x` records the trap: a twin
compiled with avx2 but dispatched on bmi2 alone executes VEX on the Skylake
Pentium/Celeron parts that ship BMI2 with AVX2 fused off. Every guard must test
**every** feature named in its `target_feature`.

---

## 6. Widening headroom — where the 27,862 SSE ops actually are

Per-symbol `xmm` counts from this build, all with **0 ymm**. This is the map of
where 128-bit work still sits, and it is the input to any future screen:

| symbol | xmm | note |
| --- | ---: | --- |
| `encode::write_sequences_bmi2` | 2,407 | avx2 sibling shipped (1,085 xmm + 322 ymm) |
| `train::finalize_dictionary` | 926 | cold — E8 |
| `encode::write_literals_bmi2` | 922 | avx2 **refuted**, +5.0% — §7 |
| `dict::Dictionary::from_bytes` | 860 | cold — E8 |
| `encode::find_dfast` | 834 | baseline monomorph |
| `MatchTables::clone` | 505 | memory, not lanes — E9 |
| `encode::ncount_or_default` | 417 | cold (dict finalize) |
| `compressed::decode_compressed_block_bmi2` | 272 | avx2 sibling shipped |
| `encode::find_fast_impl_bmi2` | 257 | ×14 monomorphs at ~256 each |

Read it against §7 before acting: most of these have already been screened.

---

## 7. CLOSED — recorded refutations, do not retry

The four scans before Scan 5 were converging on "promote the BMI2-only twins to
AVX2." **That sweep has already been run.** From `huffman.rs:decode_4x`:

> enabling avx2 on EVERY bmi2-only twin **ADDS 38,051 instructions overall and only
> two shrink.**

Both survivors are shipped (`write_sequences` −1,050, `decode_4x` −125). This lever
is exhausted. Specifically closed:

1. **Whole-twin AVX2 promotion, generally.** 17-twin sweep, net +38,051 instructions.
2. **`write_literals` under AVX2.** +745 instructions, **+5.0% slower** on Huff
   (14-corpus in-process ABBA ×7, byte-identity asserted). LLVM vectorised
   histogram/ctable loops whose trip counts cannot amortise ymm setup and the
   `vzeroupper` on exit.
3. **A narrow `#[target_feature(enable="avx2")]` copy helper.** Law 1. Measured worse
   twice (T4, brick 4.56).
4. **`prefetch_read` on the decoder match source and the encoder candidate.** Bricks
   42 and 43, both worse — the target is already cache-resident, so there is no miss
   to hide and the hint is pure instruction overhead.
5. **Widening `count_eq_len`'s entry past 32 bytes.** E10 — 83% of calls die by byte 31.
6. **`decode_compressed_block_avx2`'s speed claim.** It measured +0.3% DecLits /
   +0.5% decode — *inside noise*. It is kept on the deterministic ground of ISA
   continuity (no legacy-SSE island beside the AVX2 sequence twin), **not** as a speed
   win. Do not cite it as evidence that promotion works.
7. **`rle_byte` under AVX2** (V8). It compares the block against a splatted byte 8 at
   a time. Most blocks are not RLE and fail in **word 0**, so a 32-byte load is setup
   cost for a result the first 8 bytes already decided. E10's argument verbatim.
8. **`find_opt`'s DP array resets** (validation pass). Already optimised — `reset_to`
   fills only `price`; the other four arrays' per-block resets were deliberately
   deleted as ~17 bytes of memset per input byte doing nothing. The DP itself is a
   serial recurrence.
9. **A wide backward-extension kernel, on current evidence** (V4). Real asymmetry —
   forward extension uses AVX2, `back_eq` is one byte per iteration in four finders —
   but backward extension is bounded by the literal run, whose L3 mean is ~3.75 bytes.
   **Harvest `take_lit_hist()` before building anything**; the expected verdict is
   E10's. Reopen only if the histogram says otherwise.

---

## 8. Execution order

Byte-identical bricks first, so the kernel discipline is proven on ground where the
size table is the gate and a regression is unambiguous.

**Revised after the validation pass.** Two items jump the queue: **D8a**, because the
codec's best existing kernel is not wired in, and **E11**, because deleting a pass
beats widening one — especially in the stage where widening measured worse.

| # | brick | side | risk | gate |
| --- | --- | --- | --- | --- |
| ~~1~~ | **D8a — SHIPPED 2026-08-22**, kernel reach 50% -> 100%, decode side 0% -> 100% | both | low | done: GOLD unmoved, 54 external frames |
| ~~2~~ | **E11 — SHIPPED 2026-08-22**, 164x/74x/50x less work at L1/L3/L9 | encode | none | done: GOLD unmoved + oracle parity |
| ~~3~~ | **D1+D2 — SHIPPED 2026-08-22**, 16 `u16` per 5 instructions | decode | none | done: GOLD unmoved |
| ~~4~~ | **E2 — SHIPPED 2026-08-22** via `encode::count_match` (not the cfg(test) `count_eq_len`) | encode | none | done: LDM fingerprint unmoved |
| ~~5~~ | **D7 — CLOSED 2026-08-22**: already done by the concurrent `simd.rs` work; verified 4 `cmeq` + 1 `and` in `count_match` | both | none | asm receipt, §13 |
| ~~6~~ | **E12 — PRUNED 2026-08-22** on its ceiling probe: 15.9–48.9 steps/call, 17% of E11's work, order-sensitive rewrite | encode | — | not built; see the probe table |
| ~~7~~ | **D4 — SHIPPED 2026-08-22**, coverage proven (`dict_CROSS=4`) + 8 C-zstd dict frames byte-exact | decode | low | done |
| ~~8~~ | **D8b — SHIPPED 2026-08-22** (hardware-unverified; math + asm gated) | both | low | done: on-host decomposition proof + aarch64 asm receipt |
| ~~9~~ | **E4 — PRUNED 2026-08-22**: prime=0 iters, fill=2 positions, back-fill tile ~9 vs a 32-word knee | encode | — | not built; see probe table |
| ~~10~~ | **D3 — REFUTED 2026-08-22**: the loop already doubles; 3.3 memcpy calls of ~214 B per band-4 call | decode | — | harvested; premise did not hold |
| **11** | **E5 — OPEN**, blocked on a conflict with a shipped fold; the split is 3 passes, not 2 | encode | medium | recover the fold's number FIRST |
| ~~12~~ | **D6 — PRUNED 2026-08-22**: 99.6% eligible but only ~630K entries / 112 MiB | decode | — | eligibility != volume |
| ~~13~~ | **D9 — CLOSED 2026-08-22** on Law 1: a <=256-element reduction cannot pay a target_feature call | encode | — | closed |
| ~~14~~ | **D5 — CLOSED 2026-08-22**: precondition was "stage still visible"; it is not | decode | — | closed |
| — | **E1** — row match finder | encode | **high / not byte-identical** | **own campaign** |

Bricks 1–9 are byte-identical (D8a is bit-identical output by a different route) and
gate on the deterministic size table. Bricks 10–14 are byte-identical but harder.
E1 changes the bitstream and needs its own plan document, its own corpus board, and
the per-corpus sign-flip discipline.

**Note the shape of the top of this list.** Four of the first six bricks are
`codec-eliminate-redundancy`, not `codec-vectorize-kernel` — a pass deleted, a call
graph fixed, a scan bucketed, a kernel reused. That ordering is not an accident and
it is not a retreat from the mission: it is the house rule that redundancy
elimination comes first because it has produced the largest wins at the lowest risk
on every campaign here, and this codec has already measured what happens when you
widen instructions in a stage that needed fewer passes instead (+5.0%).

**Verify-only:** E13 (check the emitted form of the hash sweep, then close it).

**Not scheduled, by decision:** E3 (no ceiling estimate), E6 (serial recurrence),
E7 (`--rsyncable` only), E8 (cold), E9 (belongs to `codec-memory-copies`),
V3 (`all_same` — bundle with E11 only if the reorder is free),
V4 (backward extension — harvest `take_lit_hist()` first; expected null).

---

## 9. Gates

Per `rusty-zstd-measurement-discipline`, in this order, for every brick:

1. **Instruction-count screen** (Law 2) — free, deterministic, before any clock.
2. **Deterministic size table over Silesia** — a speed brick must be **byte-identical**.
   E1 is the sole exception and is gated on `us/c size` per corpus instead.
3. **Full `cargo test -p rusty_zstd --release`** (155 tests).
4. **External round-trip** through `third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe`.
5. **Only then timing** — in-process ABBA, per-corpus, never averaged across the 18.

Every kernel carries its scalar twin as the oracle and a `*_matches_scalar` test, per
`codec-vectorize-kernel`. Every kernel gets a runtime arm (`set_*_arm`) so the ABBA
harness can flip it between adjacent measurements without a rebuild — and per
`knob-census-drift`, re-run `--example allgates` afterwards rather than hand-counting
the arms.

**A brick that measures FLAT gets reverted**, even if it is provably correct and
byte-identical.

**Board parity:** encode through `compress_with(src, CompressOptions { level,
checksum: false })` in any board compared against `zstd -b` — see
`board-checksum-parity`. This matters more than usual for D8, which *is* the checksum.

---

## 10. Deployment

§9 says every kernel ships behind a runtime arm. This section says **which arm, where
it goes, and how to back it out** — the part §1–§9 left implicit.

### 10.1 Two arm patterns, and how to choose

The codec already runs both. Picking the wrong one is a measurable regression, and
`simd.rs` records why:

| pattern | read frequency | shape | when to use |
| --- | --- | --- | --- |
| **A — fold-to-constant** | per *call* / per *symbol* | `static ARM` + accessor `#[cfg(feature="profile")]`; a `#[cfg(not(profile))]` twin returns the constant | any site inside a hot loop |
| **B — hoisted atomic** | per *block* / per *frame* | plain `static ARM: AtomicU8`, read once at the dispatch point | block-level dispatch |

**Pattern A exists because pattern B was too expensive at per-call frequency.**
`count_eq_len` runs 247M times at L19 — an atomic load there would be 247M loads in
the shipping build to serve a bench knob, so `eqlen_arm()` compiles to the constant
`0` unless `--features profile`. Precedent for B: `BLOCK_AVX2_ARM`, `ENC_AVX2_ARM`,
both read once per block.

**Related trap already in the tree:** `copy_match` read `matchcopy_on()` *three times
per sequence* until T4/brick-79 hoisted it into `MatchCtx.wide`. A new arm placed
inside a per-sequence body repeats that defect.

**Export line:** arms go in `lib.rs` beside their neighbours (`set_block_avx2_arm`
line 103, `set_enc_avx2_arm` line 104). Note those two are **inconsistently gated
today** — `set_block_avx2_arm` carries `#[cfg(feature = "profile")]` (twice) and
`set_enc_avx2_arm` carries none. Match pattern B's neighbours deliberately, and say
in the commit which you chose.

### 10.2 Per-brick deployment table

Sites are current as of this build; re-resolve before editing (files move).

| # | brick | primary site | arm | pattern | byte-identical? |
| --- | --- | --- | --- | --- | --- |
| 1 | D8a — wire hybrid into streaming xxh64 | `xxh64.rs:316` `update`, `:350` `consume_stripe` | `set_xxh_avx2_arm` **(exists)** | B | bit-identical output |
| 2 | E11 — `covers` from `freq` | `huffman.rs:1006` `covers`; caller `encode_literals_section:~2023` | `set_covers_freq_arm` | B (per block) | yes |
| 3 | D1+D2 — DTable broadcast fills | `huffman.rs:726` (in `table_from_weights:583`), `:781` (in `upsample_dtable:753`) | none needed | — | yes |
| 4 | E2 — `ldm::count_eq` → kernel | `ldm.rs:198` | none needed | — | yes |
| 5 | D7 — NEON tier parity | `simd.rs:525` `count_eq_len_neon` | reuses `EQLEN_ARM` | A | yes |
| 6 | E12 — `limit_nbits` buckets | `huffman.rs:1278` | `set_kraft_bucket_arm` | B | yes (watch tie-break) |
| 7 | D4 — dict-crossing copy split | `compressed.rs:1612` (in `copy_match:1540`) | reuses `MatchCtx.wide` | B (already hoisted) | yes |
| 8 | D8b — xxh64 NEON twin | `xxh64.rs` beside `premul_p2_avx2` | `set_xxh_avx2_arm` **(exists)** | B | bit-identical |
| 9 | E4 — batch hashing | `encode.rs:2119` `prime_tables` (+ the three `fill_*_after_match`) | `set_batch_hash_arm` | B (per block) | yes |
| 10 | D3 — overlapping-copy replicate | `compressed.rs:1767` (in `copy_from_decoded`) | `set_overlap_simd_arm` | B (via `wide`) | yes |
| 11 | E5 — SeqCode two-pass | `encode.rs:3279` (in `write_sequences_inner`) | `set_seqcode_split_arm` | B (per block) | yes |
| 12 | D6 — FSE spread fast path | `fse.rs:216` / `:580` | `set_fse_spread_arm` | B (per table build) | yes |
| 13 | D9 — entropy dot products | `huffman.rs:1702` `body_bytes_exact`; `fse.rs:1057` `bit_cost` | `set_bitcost_simd_arm` | B | yes |
| 14 | D5 — `x2_from_x1_into` gather | `huffman.rs:904` | `set_x2_gather_arm` | B | yes |
| — | E13 — verify hash sweep | `encode.rs:3740` | — | — | verify only, no edit expected |
| — | E1 — row match finder | new module | `useRowMatchFinder` **param, not an arm** | — | **NO** |

Bricks 3, 4 and 7 need **no arm**: they are pure decomposition with no alternative
algorithm to flip to, and the size table is the whole gate. Do not add an arm to
satisfy a checklist — an unused arm is a knob the next census has to classify
(`knob-census-drift`).

**E1 is not an arm.** A bitstream-changing strategy is a *compression parameter* with
its own default (off), not a bench knob. It ships through the advanced-parameter path
like every other strategy selector.

### 10.3 Commit and revert

- **One brick per commit**, per `codec-optimize`. A commit that lands two bricks
  cannot be bisected when the board moves.
- **The commit message carries the number**, not the adjective: instruction count
  before/after, the size-table verdict, and the ABBA delta with its arm count.
- **Revert is the default**, not the exception: a brick that measures FLAT gets
  reverted even when it is provably correct and byte-identical.
- **`git show HEAD:<path>` after committing**, never `cat` on the working tree —
  a stale IDE buffer has silently clobbered a doc in this repo before
  (`editor-clobber-protection`). Keep the write and the `git add && git commit` in
  one command.

### 10.4 After any brick that lands an arm

Run the census harness — it discovers each arm's deployed value and classifies it
LIVE / SZ-DEAD / DRIFT / STUCK, and regenerates the knob count:

```
cargo run --release -p rusty_zstd-bench --example allgates
```

~504 s at a 2 MiB prefix. **Do not hand-count the arms and do not quote a knob number
from any doc** — the count moved between two runs twenty minutes apart because
another session added an arm mid-session (`knob-census-drift`).

### 10.5 Build-cost budget

Each new `#[target_feature]` twin compiles its body a third time. The fast ladder
already compiles twice (build ~30 s → ~54 s), and baseline monomorphs are cold pages
on BMI2 hardware. **State the instruction-count and build-time delta in the commit**
so the next campaign can see what the ISA surface cost, rather than rediscovering it
from a slow build.

---

## 11. Second harvest — 20 more opportunities

**Opened after the validation pass.** V1 was found by auditing *reachability*, not
loops — so this harvest led with the same lens and then swept the plumbing. It is
organised the way the mission asks: **(a) make the SIMD/xxh64 we already have
functional**, and **(b) convert inline scalar work**.

Every item below was verified against source. Where a hypothesis was **refuted**, it
is recorded in §11.4 rather than deleted — a refuted candidate is evidence too.

### 11.1 The pattern V1 belongs to

V1 was not a one-off. The grep it implies — *"which correct, tested helpers does
nothing call?"* — returns a family:

| item | status | verdict |
| --- | --- | --- |
| `xxh64::stripes_hybrid` (via `xxh64_seed`) | callers: 1 test, 1 bench | **V1 — real, critical** |
| `fse::default_ll_ctable` / `_ml_` / `_of_` | callers: **tests only** | **N9 — real, free win** |
| `simd::look_n_bits` / `look_n_bits_bmi2` | `#[cfg(test)]` | dead **by design** — cleanup |
| `encode::bt_ins_spec_bmi2` / `bt_find_best_spec_bmi2` | `#[allow(dead_code)]` | dead **by design** (W9) — cleanup |
| `simd::prefetch_read` | `#[allow(dead_code)]` | dead **by design** (bricks 42/43) — keep |

**Three of the five are deliberate and documented**, which is the right ratio to
expect and the reason this grep must be read, not acted on blindly. But it found V1
and N9, and N9 is a per-block cost paid three times for a process constant.

**Standing recommendation:** make this a recurring audit. `#[allow(dead_code)]` on a
non-test item is either a documented refutation (fine, and the doc comment should say
so) or an unwired kernel (a bug). There is currently no way to tell them apart
without reading each one.

### 11.2 DECODER — N1..N8

**N1 — the X2 Huffman table is built unconditionally and often never read. HIGH.**
`table_from_weights:744` calls `x2_from_x1_into` on **every** Huffman table build,
producing 2,048 × `u32` (8 KiB) through a full data-dependent gather pass. But it is
only *used* when `use_x2` passes, and `use_x2` requires `select_x2(dst, src)`, which
returns false when `dst_size < 256` (every 1-stream literal section) or when the
literals compress poorly (`src_size >= dst_size` → Q=15).
**So on 1-stream blocks and on poorly-compressible literals, the decoder builds an
8 KiB gather-driven table per block and throws it away.**
**Fix:** build it lazily on first `use_x2` hit, or hoist the `select_x2` decision to
the table build. **This largely obviates D5** — vectorising a gather you can skip
entirely is the wrong brick. Do N1 first and re-price D5 afterwards.

**N2 — there is no instrument for the X2 fire rate. INSTRUMENT, do before N1.**
Nothing counts how often `use_x2` / `select_x2` actually return true per corpus. N1's
whole value is that number. Add a two-counter `take_x2_stats()` (`builds`, `uses`)
behind `profile` and harvest before writing the lazy build — same discipline the
64-byte copy tier followed, where the histogram chose the width and the mean would
have chosen wrong.

**N3 — the streaming decoder is built on front-drains. MEDIUM.**
`stream.rs` drains the front of a `Vec` in **six** decode-side places: `input.drain`
at 424, 457, 476, 496, 526, and `decoded.drain(..drop)` at 592. Each is an O(n)
memmove of the remainder, per call. This is the exact O(n²) plumbing pattern
`codec-memory-copies` opens with.
**Fix:** a read cursor (`in_off`, which the encode side already has) instead of
draining, compacting only when the prefix exceeds a threshold.

**N4 — `upsample_dtable` zero-fills then overwrites. LOW, free.**
`wide.resize(1 << FAST_TABLELOG, 0)` writes 4 KiB of zeros that the replicate loop
immediately overwrites in full. Pairs naturally with D2, which is already touching
this function — fold it in rather than as its own brick.

**N5 — `code_from_base` keeps a linear scan above the LUT domain. LOW.**
`compressed.rs:1322`. The LUT covers the measured-common range and this is the
correctness fallback above it; it is ~36 iterations for LL and ~53 for ML when taken.
**Verify the take rate first** (`lut_on()` already exists as an arm) — this is very
likely correctly rare and should then be closed, not built.

**N6 — `Reader`'s multi-byte reads assemble arrays byte-by-byte. LOW.**
`u16_le` / `u32_le` / `u64_le` go through `take(n)` then index element-wise
(`[s[0], s[1], ...]`), rather than `simd::load_u64_le`'s single unaligned read.
`peek_u32_le` already uses the `try_into()` array form and documents why (four bounds
checks removed). **Apply the same treatment to its three siblings.** Frame/block
header parsing only, so the ceiling is small — but it is free and consistent.

**N7 — RLE literal expansion. VERIFY ONLY.**
`decode_literals` arm 1 does `out.resize(regen, b)` — a `u8` memset that should
already lower to wide stores. Confirm from the emitted form, then close.

**N8 — the MT path multiplies V1 by the worker count. MEDIUM, follows D8a.**
`mt.rs:compress_mt` calls `encode_oneshot(chunk, …, checksum, …)` per range and
concatenates the resulting **independent frames** — so every worker computes its own
frame checksum through the scalar `Xxh64`. **D8a fixes this for free**; recorded so
the MT board is re-measured after D8a rather than before.

### 11.3 ENCODER — N9..N20

**N9 — `select_seq_table` rebuilds a process constant, three times per block. HIGH, free.**
```rust
let basic = fse::FseCTable::from_norm(default_norm, default_log)?;
```
`default_norm` is one of `DEFAULT_LL_NORM` / `DEFAULT_OF_NORM` / `DEFAULT_ML_NORM` at
a fixed log. **These never change for the life of the process.** `FseCTable::from_norm`
performs three heap allocations, a cumul pass, the **serial spread loop**, a scatter
into `state_table`, and the delta build — and `select_seq_table` is called **three
times per block** (LL, OF, ML).

`fse.rs` already contains `default_ll_ctable()`, `default_ml_ctable()` and
`default_of_ctable()` — correct, tested, marked `#[allow(dead_code)]`, and **called
only from tests** (`fse.rs:1339–1345`).
**Fix:** a `OnceLock<FseCTable>` per default, handed out by reference. This is V1's
defect class exactly: the right helper exists and nothing calls it.
**Gate:** size table. Byte-identical by construction — same norm, same log, same table.

**N10 — `bit_cost` runs up to nine times per block. Frequency evidence for D9.**
`select_seq_table` evaluates `basic.bit_cost`, `prev.bit_cost` and `ct.bit_cost`, ×3
tables. D9 filed this kernel as "marginal, expect it"; **9 calls/block is the number
that decides whether it is worth a kernel at all.** Count it with the same instrument
as N2 before building D9.

**N11 — `FseCTable::clone()` on the repeat-table path. LOW-MEDIUM.**
`best_table = Some(p.clone())` clones a table holding `state_table` and `delta`
`Vec`s — two allocations per accepted repeat, ×3 tables, per block.
**Fix:** hold `Cow`/an enum of borrowed-vs-owned; the table outlives the call in every
branch that uses it.

**N12 — three passes over `counts` where one would do. LOW, free.**
`select_seq_table` opens with `counts.iter().sum()`, `counts.iter().max()`, then
`counts.iter().position(...)`. Three walks of up to 256 for `(total, most, argmost)`,
which one fold returns.

**N13 — `huffman_nbits` sorts inside its own loop. MEDIUM.**
```rust
while active.len() > 1 {
    active.sort_by_key(|&i| nodes.get(i).map_or(0, |n| n.count));
    let a = active.remove(0);
    let b = active.remove(0);
```
A **full re-sort plus two O(n) `Vec::remove(0)` memmoves on every iteration** — up to
255 iterations for a 256-symbol alphabet. The function's own comment correctly notes
it runs once per *block*, not per literal, which is why it survived; but `Huff` is
**86.7% of encode at L1 on `x-ray`** and 16–21% on four more corpora.
**Fix:** the textbook two-queue Huffman merge — sort the leaves **once**, then keep
two FIFOs (leaves ascending, internal nodes in creation order; both are monotone) and
take the smaller head. O(n log n) once, then O(n).
**Risk:** tie-breaking must be preserved exactly or code lengths shift. Gate on the
deterministic size table, which catches it immediately.

**N14 — the streaming encoder's front-drains. MEDIUM.**
`out_acc.drain(..n)` at 196 and 249, `in_acc.drain(..in_off)` at 265,
`hist.drain(..drop)` at 300. Same fix as N3 — cursor, not drain. The encode side
already *has* `in_off`; it drains anyway in `compact_in`.

**N15 — the env-var census is stale; recount it. HOUSEKEEPING.**
`encode.rs` contains **63** `std::env::var` occurrences. Many sit behind atomic or
`OnceLock` caches — and at least one memory-recorded offender is **already fixed**:
`bt_fill_stride()` now caches in `BT_FILL_S_C` and is hoisted to one read per
`find_bt_lazy` call (`encode.rs:11111`), not per emitted sequence.
**Do not hand-count and do not quote a number from any doc** — run
`cargo run --release -p rusty_zstd-bench --example allgates`, whose §4 regenerates the
census from source (`knob-census-drift`). Listed because an uncached `std::env::var`
is ~115.6 ns plus a `String` allocation, and the codec's own comments price it that
way.

**N16 — `normalize_count` allocates per call. LOW-MEDIUM.**
`vec![0i16; max_sv + 1]` per call, ×3 per block, plus a second `n2` vector whenever
the scale-fallback fires. `norm` is bounded by 256 — the same bounded-buffer move
that put `symbol_next` and `symbols` on the stack (and removed 4,496 allocations
between them) applies here unchanged.

**N17 — `write_ncount` / `ncount_seq_table` allocate a `Vec<u8>` per call.** Same
class as N16, same fix: a recycled scratch buffer on `MatchTables`, which already
carries `coded_scratch` and `bits_scratch` for exactly this reason.

**N18 — `segment_histograms` heap-allocates its result. LOW.**
Returns `Vec<[u32; 256]>` — 4 KiB of `[u32; 256]` allocated per block. Bounded at 4
entries by construction (`n_streams` is 1 or 4), so it is a fixed-size array on the
stack or a recycled buffer.

**N19 — `encode_4_streams` builds four `Vec<u8>` then concatenates. LOW-MEDIUM.**
Four heap allocations plus a copy of every compressed literal byte, per block. The
sizes are known once each stream closes; encoding into one buffer with recorded
offsets removes the copy. **Note the header constraint:** the three 2-byte stream
sizes are written *before* the payloads, so this needs a reserve-and-backfill, not a
naive append.

**N20 — the chain table is materialised zeroed on first dispatch fire. LOW, verify.**
`find_sequences_strategy` does `alloc::vec![0u32; 1 << params.chain_log.min(24)]` when
the Fast→Lazy dispatch first trips — up to **64 MB of zeroing** inside an encode call.
This is deliberate (brick 47 keeps Fast's smaller L1 footprint for files that never
trip it), so the allocation is correct; **what is worth measuring is the latency spike
on the block that trips it**, and whether the zeroing can be deferred or the table
sized to the window rather than to `chain_log`.

### 11.4 Refuted during this harvest — recorded, not deleted

- **"The 1-stream literal decode has no BMI2/AVX2 twin."** False. `decode_stream` is
  `#[inline(always)]` and reached through `decode_huff_streams` →`decode_literals` →
  `decode_compressed_block_inner`, so it is compiled *inside* whichever block twin
  ran. It gets full ISA coverage by inlining. No gap.
- **"`bt_*_spec_bmi2` being `#[allow(dead_code)]` means the Bt ladder loses BMI2
  specialisation."** False, and documented as W9: the `(hash_log, chain_log)` spec
  list is **BMI2-redundant** — `shrx` takes its count from any GPR and `BtCtx` holds
  both operands in registers for the whole walk. `bt_resolve` deliberately returns
  the runtime BMI2 arm before consulting the spec list. The dead functions are
  leftovers; deleting them is cleanup, not a fix.
- **"`bt_fill_stride` reads the environment per emitted sequence."** **Was true, is
  now false** — it caches in `BT_FILL_S_C` and is hoisted to one call site
  (`encode.rs:11111`). The memory recording this is stale and has been corrected.
- **`bit.rs` `BitCStream::flush` / `add_bits`.** Looked like a per-flush memcpy
  candidate; brick 68 already fixed it with a fixed-width 8-byte store into spare
  capacity plus a `set_len` commit. Correct as written.
- **`reader.rs::peek_u32_le`.** Already uses the `try_into()` array form to state the
  length structurally (T4). Only its three siblings still assemble element-wise — N6.

### 11.5 How this harvest changes the ranking

**Two items belong in the top tier and were not in §8:**

| brick | why it jumps |
| --- | --- |
| **N9** — `OnceLock` the three default FSE ctables | free, byte-identical, 3 rebuilds of a process constant per block, and the helper already exists |
| **N1** — lazy X2 DTable build | removes an 8 KiB gather-driven pass per table build **and re-prices D5 down** |

**And the shape of the second harvest confirms the first's conclusion.** Of these 20,
**three are SIMD kernels; the rest are reachability, redundancy and plumbing.** That
is now two independent passes reaching the same verdict: this codec's remaining wins
are mostly *not* wider instructions. The ISA surface is close to saturated (§0, §7);
what is not saturated is **work that does not need doing at all** — a table rebuilt
for a constant, a table built and discarded, a sort inside its own loop, a kernel
nothing calls.

**Suggested insertion into §8:** N9 at rank 2 (beside E11 — same class, same gate,
same cost), N1 at rank 4, N13 and N3/N14 in the 7–10 band, and the remainder as a
housekeeping sweep once the top of the list has landed. Re-run the instrument-first
items (N2, N10) *before* their dependent bricks, not after.

### 11.6 N21 — found while assessing whether a third harvest is worth running

**The decoder rebuilds the three RFC-constant FSE tables, from scratch, per block.
HIGH — and structurally larger than N9.**

`compressed.rs:seq_table` mode 0 (Predefined) does:

```rust
predefined: fn() -> Result<FseTable, Error>,
...
0 => Ok((predefined()?, 0)),
```

where `predefined` is `fse::default_ll` / `default_ml` / `default_of`, each of which
is `FseTable::from_norm(&DEFAULT_LL_NORM, 6)` — **a full FSE decode-table build: a
heap allocation, the serial spread loop, and the finalize pass over `table_size`
entries.** The norms are `const` arrays fixed by RFC 8878. The result is
byte-for-byte identical on every call, for the life of the process.

This runs **per block, per table (LL/OF/ML), on every Predefined selection** — and
Predefined is a common mode. It lands in `DecSeqTables`, inside `DecodeSeq`, which is
the decode leader on **16 of 18** corpora at 65.9–94.7%.

**Why it survived every previous audit, including this document's first two passes:**
`predefined` is a **function pointer**. The `bmi2-twin-architecture` note already
records that fn pointers — naming `BtFn` and *"seq-table builder fn values"*
specifically — are **invisible to all three census instruments** and must be audited
by hand. That audit had not been done. This is the blind spot doing exactly what it
was flagged to do.

**Fix:** the same as N9 — build once into a `OnceLock<FseTable>` (or a lazily-filled
slot on `BlockState`, which already recycles the compressed-mode tables via W26) and
hand it out by reference. Note `seq_table` returns an owned `FseTable`, so this needs
either `Cow`, an enum, or the same move-through trick W25 used for Repeat mode.
**Gate:** deterministic size table + external round-trip. Byte-identical by
construction — same norm, same log, same table.

**N9 and N21 are the same defect on opposite sides of the codec**, and neither was
found by looking for it. That is the argument for the third harvest, not this
document's completeness.

---

---

## 11.7 SECOND-HARVEST DISPOSITION — 2026-08-22

All 21 N-items resolved. **Four shipped, two delivered as instruments, the rest closed
on measurement.** Every number below is a deterministic counter.

### Shipped

| item | before → after | gate |
| --- | --- | --- |
| **N9** — cache the RFC-constant FSE ctables | **1,949–2,628 rebuilds → 0** per 88 MiB (each: 3 heap allocs + cumul + serial spread + scatter + delta) | GOLD unmoved, 25 external frames |
| **N13** — head cursor instead of two `Vec::remove(0)` | per-iteration cost **~3n → ~n**; removes ~21M element-moves per 88 MiB | GOLD unmoved — this was the tie-break risk |
| **N4** — kill `upsample_dtable`'s dead zero-fill | up to **4 KiB of dead memset per table build**, ~1,400 builds/run | GOLD unmoved |
| **N8** — MT per-worker checksums | resolved for free by **D8a**; every worker's `Xxh64` now reaches the AVX2 kernel | D8a's census |

**N9 is the sharpest V1 recurrence in the document.** `default_ll_ctable()` and its two
siblings already existed in `fse.rs` — correct, tested, `#[allow(dead_code)]` — and were
**called only from tests**, while the shipping path rebuilt the same tables by hand
22–30 times per MiB.

**N9 also produced the session's best instrument story.** The first implementation
dispatched by pointer identity against `DEFAULT_*_NORM`. It compiled, passed every test,
and the rebuild counter **did not move at all**: those are `const`, not `static`, so each
use site gets its own inlined copy at its own address and `ptr::eq` never matches. A
silent no-op would have shipped as "done". Dispatch is now by shape, **verified by
content** — a 58–106 byte compare against three allocations and a spread.

**N13's exact-equivalence argument is the load-bearing part.** `sort_by_key` is stable,
so ties break by pre-sort position; the live region holds the identical sequence either
way, so sorting the subslice reproduces the tree bit-for-bit. The loop exit also had to
move from `active.first()` to `active[head]` — index 0 now holds a consumed node.

### Closed on measurement

| item | measured | verdict |
| --- | --- | --- |
| **N2** (instrument) | **DELIVERED** — `take_x2_stats()`, built, exported, harvested | done |
| **N1** — lazy X2 build | X2 used **31.8–49.1%** on our streams but **98.6% on C-zstd frames** | **re-priced DOWN, closed** |
| **N21** — cache Predefined decode tables | **0.13/MiB** on our streams, **2.28/MiB** on C-zstd (20x provenance swing); 411 rebuilds / 180 MiB | too small; naive fix only trades a spread for a memcpy |
| **N10** | frequency evidence for D9, and D9 is closed on Law 1 | moot |
| **N5, N7, N20** | verify-only by their own text | closed |
| **N11, N16–N19** | all allocation items — they belong to §12.2's census, not here | **rehomed to §12.2** |
| **N3, N14** | streaming front-drains, `codec-memory-copies`' opening pattern | **rehomed to §12.2**; real, but a streaming-plumbing brick, not an inline/SIMD one |
| **N12, N6** | LOW/free micro-items | left as a housekeeping sweep |
| **N15** | env-var census refresh | housekeeping |

### The provenance finding, stated once because it recurred twice

**N21 and N1 both inverted when measured on C-zstd-produced frames rather than our own.**
N21 swung **20x** (0.13 → 2.28 rebuilds/MiB); N1 swung the *verdict* (51–68% waste on our
streams → 1.4% on the reference's). `codec-measurement` §9 says bitstream provenance is
content; this document now has two independent confirmations on this codec.

**The operational rule: any DECODER item whose value depends on which modes the encoder
selected must be harvested on foreign frames before it is ranked.** Our own encoder's
mode choices are not the population a decoder serves. `examples/n21prov.rs` and
`examples/n2x2.rs` both take a directory of `.zst` files for exactly this.

### What N13 leaves on the table

The cursor is the *safe* half. The full two-queue Huffman merge the item describes would
take **sum(n²) = 29.1M → ~1.1M, a further ~26x**, because it sorts once instead of once
per iteration. It is not built here because it must reproduce the stable-sort tie order
exactly and that is a different, careful piece of work — but the measured prize is now on
record, and `take_n13_stats()` is in the tree to re-price it.

---

## 12.7 REMAINING-VEINS DISPOSITION — 2026-08-22

§12 lists four open veins. They are **not inline/SIMD work** and this document is the
wrong home for three of them; recording the disposition rather than leaving them
ambiguous:

- **§12.2 — the allocation and clone census. Now the richest remaining vein by a
  distance, and it grew this session.** N11, N16, N17, N18, N19, N3 and N14 all landed
  here, and N9's fix removed ~7,500 allocations per 88 MiB purely as a side effect of a
  redundancy fix — which is itself evidence the vein is real. **This should be its own
  plan document** with an allocation counter as its first instrument, exactly as this one
  opened with the ymm/xmm census.
- **§12.3 — frame- and block-invariant recomputation. Partly harvested here, and it was
  the highest-yield class in the whole campaign.** N9 (encoder) and N21 (decoder) are both
  instances, and N9 shipped. The generalisation is worth stating: *grep for
  `from_norm`, `from_weights`, `::new(` and `build` on any per-block path and ask what
  the arguments depend on.* If the answer is "a constant", it is an N9.
- **§12.4 — the outline/inline boundary audit.** Touched incidentally and repeatedly:
  `find_sequences_strategy_sel`, `table_from_weights`, `upsample_dtable`,
  `premul_p2_neon` and `stripes_pre` were all fully inlined, which is why several
  instruction-count probes had to be attributed by hand rather than by symbol. Worth a
  pass, low expected yield.
- **§12.5 — never opened.** Remains never opened. Named so it is not mistaken for
  covered.

**The campaign-level verdict, now that both harvests are fully worked:** of 35 catalogued
opportunities (14 in §8, 21 in §11), **three were SIMD kernels and two of those shipped**
(D8a wiring, D8b NEON). Everything else that paid was reachability, redundancy or
plumbing. The ISA surface is saturated; **work that does not need doing at all is not.**

---

## 13. D7 and N13 CLOSED — 2026-08-22 (second pass, once `simd.rs` was free)

### D7 — already done, by the concurrent `simd.rs` workstream. Nothing to build.

D7 named two gaps in `count_eq_len_neon`. **Both were closed before I got there**, and
the code says so in past tense (*"This twin had no unroll at all"*, *"THE TAIL IS A
VECTOR COMPARE, mirroring the AVX2 twin exactly"*). Verified rather than taken on trust:

- The symbol does not exist in the aarch64 build — it is inlined into
  `count_eq_len_ge8_raw`, so a symbol-table check reads **zero** and means nothing.
  Attributing by INSTRUCTION instead puts it in `encode::count_match`, the shipping hot
  path, at exactly **4 `cmeq` + 1 `and`**: two 16-byte compares fused under one mask in
  the loop, one for the 16-byte tier, one for the overlapped vector tail.
- That is the AVX2 twin's structure at NEON's register width, and it retires both of
  D7's items.

**The one real remaining difference is stride, not structure.** AVX2 advances 64 B per
iteration (2 x 32 B registers); NEON advances 32 B (2 x 16 B). Both issue two loads per
iteration, so matching AVX2's *byte* stride would need a 4-register NEON tier — more
live registers before the branch, for less loop overhead on long matches. **Unmeasurable
here** (no aarch64 hardware or emulator) and not what D7 asked for. Recorded, not built.

### N13 — the full two-queue merge SHIPPED, byte-identical

The cursor brick was the safe half. This is the other 26x:

| | before | after |
| --- | ---: | ---: |
| sort invocations | Σ(n−1) = **129,160** | 1 per call = **754** |
| `Vec::remove(0)` O(n) memmoves | **258,320** | **0** |
| total element operations | ~**29.1M** | ~**1.16M** (~25x) |

*(calls = 754 and mean n = 172.3 are measured by `take_n13_stats()`; the sort count is
one per call by construction where it was n−1.)*

**Why it is byte-identical, which is the whole risk.** Both queues are monotone: leaves
sorted once, and internal-node counts non-decreasing (if step k takes x ≤ y and emits
s = x+y, every survivor is ≥ y and s ≥ y, so step k+1 emits s′ ≥ 2y ≥ s). The tie rule
is the part that had to be derived rather than guessed: the old code **stable**-sorted
the live region, so equal counts kept pre-sort order — a still-live leaf was in the array
before any internal node was appended, so **the leaf wins a tie**, and among internal
nodes the earlier-created one wins because it was appended first. That is `l <= x`, not
`l < x`. One character, and it is the difference between byte-identical and a shifted
code length.

**Gate:** `bytegate` GOLD `BE0071FB0CB0CED9` unmoved, 137/137 tests release + debug,
36 external frames through C zstd v1.5.7, aarch64 cross-check clean.

## 12. The remaining veins — what has NOT been swept, and how to sweep it

Three passes have run: the five targeted scans (§1), the exhaustive loop census
(§1b), and the second harvest (§11). This section records **what those three passes
did not look at**, so the next campaign starts from a map instead of a hunch.

Each vein below carries (a) evidence that it is or is not rich, (b) the **method** to
run it, and (c) an honest estimate. A vein with no method is a wish, not a plan.

### 12.0 First: the value gradient is real, and it is bending

| pass | items | of which structural wins |
| --- | ---: | --- |
| §1 scans 1–5 | 22 (D1–D9, E1–E13) | the ISA verdict itself; E1, D1/D2, E4 |
| §1b validation | 8 (V1–V8) | **V1** (critical), V2, V5 |
| §11 harvest | 21 (N1–N21) | **N9, N21, N1, N13** |

**41 catalogued items, zero landed bricks.** Each pass still returns one or two
genuinely large findings — V1, then N9/N21 — so the veins are not exhausted. But the
tail is lengthening, and a fourth pass will likely be one N21-class item plus
housekeeping.

**Read this section as a menu, not a queue.** The correct next action is probably to
*land N9 and N21* — both free, both byte-identical, both gating on the size table —
before extending the catalogue again. A longer plan is not a faster codec.

### 12.1 CLOSED — the function-pointer audit

**This vein is now swept, and it is complete.** `bmi2-twin-architecture` flagged fn
pointers as invisible to all three census instruments — the per-symbol CL census, the
one-level twin trap trace, and the transitive trap trace — and named `BtFn` and
"seq-table builder fn values" specifically, with the instruction to audit them by
hand. That audit had never been run. It has now.

**The whole population is four sites:**

| site | type | verdict |
| --- | --- | --- |
| `compressed.rs:1075` | `predefined: fn() -> Result<FseTable, Error>` | **N21 — the bug** |
| `encode.rs:9244` | `ChainFn` | **clean** — `chain_find_best_bmi2_ptr` is a safe wrapper handed out only behind `has_bmi2()` |
| `encode.rs:10216` | `BtFn` | **clean** — `bt_resolve` selects the ISA once per block; W9 documents why the spec list is skipped on BMI2 |
| `encode.rs:10224` | `BtInsFn` | **clean** — same resolver |

**One finding out of four, and the other three are correct by construction.** Record
the vein as closed; do not re-run it unless a new fn-pointer type is introduced.
**If one is introduced, it must be audited by hand on the same day** — the three
census instruments will not see it.

### 12.2 OPEN — the allocation and clone census. **Richest remaining vein.**

**Density evidence:** `vec![…]` / `Vec::with_capacity` / `Vec::new()` / `to_vec()` /
`.clone()` occur **215 times** across the four hot files — `encode.rs` 123,
`huffman.rs` 56, `compressed.rs` 19, `fse.rs` 17.

**Why it is rich:** N9, N11, N16, N17, N18 and N19 all came from this vein
**incidentally** — they were noticed while reading for something else. It has never
been run as a deliberate pass. The codec's own history says the same thing: W30, W32,
W35, W36, W37, W40, W42, brick 63 and brick 74 are all allocation removals, and the
decode census once measured **~147 MB of allocation traffic** and 1,660 remaining
allocations.

**Method — and the tooling already exists:**
```
cargo run --release -p rusty_zstd-bench --example allocs      # per-size-class census
cargo run --release -p rusty_zstd-bench --example allocost    # cost attribution
cargo run --release -p rusty_zstd-bench --example g6alloc
```
Harvest the size-class histogram **first**, then attribute the top classes to call
sites. Do not start from the grep — the grep has 215 hits and no ranking; the census
has a ranking and will name the ten that matter.
**Belongs to `codec-memory-copies`**, not to this document's kernel discipline.
**Estimate: 8–12 items**, skewed toward per-block buffers that should be recycled
onto `MatchTables` / `BlockState`, both of which already carry scratch fields for
exactly this purpose (`coded_scratch`, `bits_scratch`, `lit_buf`, `opt_price`).

### 12.3 OPEN — frame- and block-invariant recomputation. **Highest value per item.**

**This is the vein that produced N9 and N21, and it has never been run as a pass.**
Both were found sideways: N9 while reading `select_seq_table` for something else, N21
while probing whether a third harvest was worth running. Two of the campaign's
largest remaining wins came from a vein nobody has swept.

**The question to ask at every per-block and per-sequence call site:** *does this
compute something that cannot change until the next frame?* The three answers already
found:

| found | recomputed | true scope |
| --- | --- | --- |
| N9 | 3 encoder FSE ctables from `DEFAULT_*_NORM` | process constant (RFC) |
| N21 | 3 decoder FSE tables from `DEFAULT_*_NORM` | process constant (RFC) |
| N1 | the 2,048-entry X2 Huffman table | needed only when `select_x2` fires |

**Method.** Walk the per-block entry points — `encode_block`, `write_literals`,
`write_sequences`, `find_sequences_strategy`, `decode_compressed_block_inner`,
`decode_literals`, `decode_sequences_inner`, `seq_table` — and for each callee ask
what its arguments actually depend on. A callee whose arguments are all `const`,
`static`, or frame-scoped is a hit. The codebase's own hoisting comments (`W1`, `W2`,
`W4`, `W5`, `W10`, `W18`, T4/brick-79) are the same move applied *within* a function;
this vein applies it *across* one.
**Estimate: 5–8 items**, and they are the most likely to be N9/N21-sized rather than
housekeeping. **If only one vein gets run, run this one.**

### 12.4 OPEN — the outline/inline boundary audit

**Why it matters:** the shim-trap rule — an un-twinned helper reachable from a twin
compiles at **baseline ISA**. Three census instruments exist for this and were run
(per-symbol CL census, one-level trap trace, transitive trap trace), which is why the
twin architecture is as complete as it is. But those instruments answer *"is this
callee twinned?"* They do not answer the inverse question: **"which callees should
not be outlined at all?"**

That inverse question is what SIMD-1 resolved for `copy_match` — the twin existed and
was still generating 0 ymm across ~42% of the DecSeq loop, purely because the copy
was outlined. One `#[inline(always)]` fixed it. **Nothing has swept for the rest of
that class**, and the codebase carries deliberate `#[inline(never)]` markers
(`count_eq_len_words`, `chain_find_best`, `find_dfast`, `bt_find_best_impl`) whose
justifications are individually sound but have not been re-checked together since the
twin campaign changed the surrounding code.

**Method:** from the asm, list every `callq` target inside a twin symbol, then for
each ask whether the callee (a) carries `target_feature`, (b) is `inline(always)`, or
(c) is neither — case (c) is the trap. Cross-check against the deliberate
`inline(never)` list and require a *current* justification for each, not a
historical one.
**Estimate: 3–5 items**, mostly small, one possibly large if a hot callee is sitting
in case (c).

### 12.5 OPEN — never opened at all

| unit | size | note |
| --- | ---: | --- |
| `crates/rzstd-alloc/src/lib.rs` | **9 lines** | the allocator seam; trivially auditable, and it is the `rusty-coding-requirements` house allocator hook |
| `crates/rusty_zstd-cli/src/main.rs` | **1,008 lines** | never read. Not on the library hot path, but it is what a user actually runs — and `codec-memory-copies` opens with *"the codec is slow but the kernels look fine"* being a plumbing problem in exactly this layer |

**Method:** read them. `rzstd-alloc` is nine lines. The CLI is one file and should be
checked for the classic wrapper defects — reading the whole input before compressing,
per-chunk `Vec` churn, a `BufWriter` that is not, and O(n²) accumulation.
**Estimate: unknown, and that is the point** — an un-read file has no estimate.

### 12.6 Sequencing

**Recommended order if the catalogue is extended before anything is landed:**

1. **12.3** (frame/block invariants) — highest value per item, and it is the vein that
   already produced the two biggest free wins.
2. **12.2** (allocation census) — richest, tooling already exists, harvest before grep.
3. **12.5** (unread units) — cheapest to close; nine lines and one file.
4. **12.4** (inline boundary) — most likely to be small, and it needs the asm
   pipeline that §5 Scan 5 already established.

**But the stronger recommendation is not to extend first.** N9 and N21 are free,
byte-identical, gate on the deterministic size table, and sit in the encode and
decode leaders respectively. Landing them converts this document from 41 findings
into 2 measured results, which is the only currency `codec-measurement` recognises.


## 13. The "2x slower than zstd" diagnosis, corrected (2026-08-24)

Two "missing functions" were named against the L3 board's 1.82x encode / 1.71x
decode gap. **Both were measured before building, and both are refuted at L3.**

### 13.1 `ZSTD_row_match_finder` -- ceiling EXACTLY ZERO at L3

See the ceiling probe in E1. L1 and L3 walk no hash chain (`Fast`, `DFast`), so
there are no dependent chain loads for a row scan to delete. The lever is real
but lives at **L5-L12** (336-5263 loads/KiB). It was mis-sited, not mis-valued.

### 13.2 `ZSTD_decompressSequencesLong` -- never selected at any level we board

zstd picks its prefetching `_Long` sequence decoder on **window size**, not on
level or corpus. `examples/mlceiling.rs` measured our windows:

| level | L1 | L3 | L5 | L9 | L19 | L22 |
|---|---|---|---|---|---|---|
| window | 512 KiB | 2 MiB | 2 MiB | 4 MiB | 8 MiB | 64 MiB |

Every level up to L19 sits **below** the threshold, and L3's 2 MiB fits in L3
cache outright. zstd is running the same short decoder we are. A prefetch
pipeline cannot explain a gap that exists where zstd does not prefetch either --
and being byte-identical and work-identical, it would have **no deterministic
counter**, leaving only a clock, against a null arm of 10.88-16.74%.

### 13.3 What the gap actually is: FIXED COST PER SEQUENCE

`examples/seqdensity.rs` counts executed match copies (one per sequence):

| level | copies/MiB | bytes/copy | vs L3 |
|---|---:|---:|---:|
| L3 | 66,774 | 15.3 | 1.00x |
| L5 | 72,506 | 14.8 | 1.09x |
| L9 | 58,971 | 18.6 | 0.88x |
| L19 | 49,519 | 24.0 | **0.74x** |

Now price the two hypotheses against the boards. Our excess decode time is
`(gap - 1) x C_time`: **0.71x** at L3 (gap 1.71) and **0.49x** at L19 (gap 1.49),
so the excess falls by **0.69x** from L3 to L19 -- against a sequence count that
falls by **0.74x**.

**The excess tracks the sequence COUNT, not the byte count and not the window.**
A per-byte or cache-latency cost would have grown with the window; it shrank, in
proportion to sequences. The target is the fixed per-sequence cost in
`decode_sequences_inner` and the copy dispatch around it.

### 13.4 And the encode gap is L3-only

The L19 board: **mean C/us comp 0.70** -- we compress **1.43x FASTER than zstd**
at L19, winning 16 of 18 corpora. The 1.82x encode deficit is specific to L3's
`DFast`. "We are 2x slower" is not a property of this codec; it is a property of
one strategy. Board L5-L12 before spending another day on it.



### 13.5 Where the per-sequence cost actually is (dsloop ladder, L3, 5 rounds, K=8)

DecSeqLoop = **40.41 ns/sequence**. Cost of ONE execution of each op:

| op | executions/seq | ns per execution | ns/seq | % of loop |
|---|---:|---:|---:|---:|
| `copy_match` | 1 | 23.29 | 23.29 | **57.6** |
| `copy_literals` | 1 | 7.11 | 7.11 | **17.6** |
| `FseTable::advance` x3 | 3 | 1.26 | 3.77 | 9.3 |
| `BitRev::reload` x2 | 2 | 1.29 | 2.59 | 6.4 |
| `BitRev::read_bits` x3 | 3 | 0.79 | 2.38 | 5.9 |
| `resolve_offset` | 1 | 2.28 | 2.28 | 5.6 |
| `FseTable::entry` x3 | 3 | 0.33 | 0.99 | 2.4 |
| **total** | | | **42.41** | **105.0** |

**The entropy machinery is 24% of the loop and the two copies are 75%.** Every
brick in section 11 that shaved the FSE/bit-reader path was working inside a
quarter of the loop. Stop optimising there.

`copy_match` at **23.29 ns** for a mean length of 7.4 bytes (T4's census: 86.6%
of match copies are `len <= 16`) is not copy cost -- a 16-byte `movups` pair is
~1 ns. It is **memory latency**: the source is a random offset up to the 2 MiB
window back into `out`, which does not fit L2. That is the fixed per-sequence
cost 13.3 identified, now located.

### 13.6 The brick this justifies: a DEPTH-1 decode-ahead pipeline

This is the prefetch idea 13.2 refuted, rebuilt on the right evidence. It is NOT
zstd's `_Long` decoder (window-gated, never selected here). It is the same
principle at depth 1, and the match address is computable one sequence early:

    src(N+1) = out.len_after_N + litlen(N+1) - offset(N+1)
             = (out.len() + litlen_N + matchlen_N) + litlen(N+1) - offset(N+1)

Every term is known once N+1's SYMBOLS are decoded, and symbol decode is the
cheap 24%. So: decode N+1's symbols, compute and `prefetch` its match source,
then execute N's copies. The miss for N+1 overlaps N's ~30 ns of copy work.

Gate: `examples/bytegate.rs`, GOLD **BE0071FB0CB0CED9** -- the brick reorders
DECODE relative to EXECUTE but publishes the same bytes in the same order, so it
must be byte-identical. Note it is work-identical too, so per 13.2 it carries no
deterministic counter of its own; the ladder's `copy_match` line is the
instrument, and it must be re-run rather than a whole-decode clock trusted.



### 13.7 D9 built, sized, and PARKED -- a noise verdict, stated as one

The 13.6 brick was built in its cheapest form: prefetch the match source right
after the symbols are decoded, so the miss overlaps `copy_literals` +
`resolve_offset` (9.4 ns) instead of stalling `copy_match`. Only literal offsets
are prefetched (`offset_value > 3`); rep codes need `state.reps`, resolved later.

**Reachability confirmed:** `prefetcht0` appears **2x** in the emitted asm, so
the kernel is not one of the five unreachable ones this workspace has shipped.

**Deterministic size (`examples/d9cover.rs`):** 6,847,222 issued / 424,319
rep-skipped / 4,497 oob-skipped = **94.1% coverage**, ceiling **21.9% of
DecSeqLoop**. Not a coverage problem -- D9 reaches nearly every sequence.

**The clock could not see it (`examples/d9probe.rs`, L3, 21 ABBA rounds,
DecSeqLoop stage isolated):** OFF 34.31 -> ON 34.38 ns/seq = **+0.2%**,
129/273 pairs, **z = -0.91**, against a **NULL ARM of 27.70%**.

A 21.9% effect would have cleared even that floor. It read +0.2%. The mechanism
is almost certainly that **9.4 ns is not far enough**: the out-of-order window
already reaches past `copy_literals` to the `copy_match` load, so the software
prefetch buys an instruction and no latency. That is the standard failure mode
of short-distance software prefetch on a modern core, and it does NOT refute
13.6 -- it constrains the DISTANCE, not the idea (codec-measurement 11: a prune
constrains the approach it tested, not the idea it belonged to).

**Disposition:** `set_prefetch_arm` kept, **defaulted OFF**. Point estimate leans
mildly negative, so it does not ship unverified. This is a
*revert-because-noise*, not a *revert-because-worse* -- and per section 15's
corollary the floor moved for a NAMED reason: `faucet.exe` (86,147 CPU-s, up
since 2026-08-19) plus eight `Code.exe` at 40-90k CPU-s each. The null arm went
10.88% -> 16.74% -> 27.70% across one session on this box.

**Next, and it needs a quiet box, not more code:** the depth-1 pipeline gives
~40 ns of distance instead of 9.4. Decode sequence N+1's symbols before
executing N's copies -- the address is `out.len_after_N + litlen(N+1) -
offset(N+1)`, all known. Do NOT build it until `d9probe`'s null arm is under
~3%, or the result will be exactly as unreadable as this one.



## 14. E1 BUILT AND BOARDED -- the row match finder ships behind an arm

`crates/rusty_zstd/src/rowfind.rs`, wired into the LAZY ladder via `ChainFn`.
`set_row_arm(true)`. Defaults OFF: it is bitstream-changing and costs size, so
it may not ship until a quiet box prices the speed it buys.

**Reachability, first, because this workspace has shipped five unreachable
kernels.** `ROW_LOADS` reads 30.6M/35.1M/34.6M at L7/L9/L12 -- the finder is on
the shipping path, not just in the tests.

**The board (`examples/rowboard.rs`, 12 Silesia corpora, 8 MiB each):**

| level | chain loads | row loads | fewer dependent loads | size |
|---|---:|---:|---:|---:|
| L7 | 97,758,176 | 30,648,648 | **3.19x** | 1.0059x |
| L9 | 153,594,793 | 35,147,365 | **4.37x** | 1.0054x |
| L12 | 460,726,801 | 34,619,518 | **13.31x** | 1.0191x |

Round-trip asserted on every cell of every level -- zero failures. At L7 the row
finder is SMALLER than the chain on four corpora (nci 0.9602, ooffice 0.9806,
osdb 0.9867, mozilla 0.9973).

The reduction grows with level exactly as the mechanism predicts: deeper chains
mean more dependent loads per position, while a row is always one load. The size
cost grows for the same reason -- a row holds 16 candidates and a deep chain
holds more, so the row finder trades candidates for loads. That trade is the
whole design, and it is now priced instead of assumed.

### 14.1 What is NOT yet known, and why

**The speed.** Row loads are not free, and 3-13x fewer DEPENDENT loads is a
claim about latency that only a clock can convert into a verdict. This box
cannot: the decode null arm ran 10.88% -> 16.74% -> 27.70% -> 33.30% across one
session, with `faucet.exe` at 86,147 CPU-s and eight `Code.exe` at 40-90k.

So E1 is at the same gate D10 was, with the OPPOSITE deterministic evidence:
D10 removed no work and added structure (correctly refuted at three distances),
while E1 removes 3-13x of the dominant dependent-load class and adds a 16-lane
SIMD compare. Do not ship it on that asymmetry -- ship it when the board can be
run with a null arm under ~3%, on a corpus speed board paired with the size
column above.

### 14.2 Scope

Only the LAZY ladder is wired. `find_greedy_impl` (L5) carries its own
hand-copied walk instead of going through `ChainFn`, so it would need the same
treatment separately -- worth 43M loads, against lazy's 139M/221M/674M.
`Fast` (L1) and `DFast` (L3) walk no chain and are out of scope permanently.



### 14.3 The speed verdict -- and a HARNESS DEFECT that nearly buried it

14.1 said E1's speed "needs a quiet box." **That was wrong, and the error was in
the instrument, not the box.**

A null arm bounds the MAGNITUDE readable from one pair. It does not bound the
SIGN: noise scatters pairs symmetrically, so a null arm wins ~50% of its own
pairs however wide it is. D10 was decided on this same box at z = -14.59 for
exactly that reason. E1 was decidable all along.

Worse, `d9probe`/`d10probe` reported only the WORST null pair, which is the most
pessimistic statistic there is. `examples/rowspeed.rs` reports both:

    NULL ARM (chain vs chain): median 4.79%, worst 20.24%

**The floor for a median delta is ~4.8%, not the 27-33% that got quoted.**
Quoting the worst pair as if it were the floor overstated it ~7x and caused a
real, resolvable result to be recorded as unmeasurable.

**`examples/rowspeed.rs` @ L9, EncodeMatchFind stage, ABBA, 21 rounds:**

| corpus | chain ns | row ns | delta | wins | z |
|---|---:|---:|---:|---:|---:|
| dickens | 971.0 | 508.9 | **-47.6%** | 21/21 | +4.58 |
| reymont | 504.8 | 344.8 | -31.7% | 21/21 | +4.58 |
| webster | 620.5 | 426.2 | -31.3% | 21/21 | +4.58 |
| sao | 803.3 | 561.3 | -30.1% | 21/21 | +4.58 |
| x-ray | 941.0 | 677.1 | -28.0% | 21/21 | +4.58 |
| samba | 365.7 | 309.2 | -15.4% | 21/21 | +4.58 |
| mr | 486.2 | 437.9 | -9.9% | 19/21 | +3.71 |
| xml | 159.6 | 145.8 | -8.6% | 19/21 | +3.71 |
| mozilla | 416.1 | 386.7 | -7.1% | 20/21 | +4.15 |
| osdb | 406.7 | 401.2 | -1.4% | 9/21 | -0.65 (inconclusive) |
| ooffice | 327.9 | 331.7 | +1.2% | 9/21 | -0.65 (inconclusive) |
| **nci** | 163.4 | 180.7 | **+10.6%** | 3/21 | **-3.27 (regression)** |

**BOARD: -23.6% | 205/252 pairs | z = +9.95 | size 1.0054x**

Nine significant wins, ONE significant loss, two inconclusive.

`nci` is the cell to understand before shipping: it is the only corpus that gets
slower, and it is also where the row finder compresses BEST (675,624 vs 701,808
= 0.963x). It finds different and better matches there and pays for them. That
is a content-dependent split, which is what `codec-content-adaptive-dispatch`
exists for -- not a reason to discard the lever.

Plausibility check (codec-measurement 7): at L9 the row finder examines FEWER
candidates than the chain (127.3M vs 153.6M) and takes 4.37x fewer dependent
loads, so faster is the expected direction and -23.6% is not an impossible
number. The genuine surprise is the 0.54% size cost -- capping at 16 candidates
barely hurts, which says the 16 most-recent candidates carry the value.

### 14.4 Disposition

Still defaults OFF. It is bitstream-changing, costs 0.54% size, and regresses
one corpus; flipping the default is a product call, not a measurement call. What
has changed is that the trade is now PRICED on both axes instead of one:
**-23.6% match-find time for +0.54% size, at z = +9.95.**

Before shipping: board L5-L12 (only L9 is measured), decide the `nci` case, and
wire `find_greedy_impl` (L5), which carries its own hand-copied walk outside the
`ChainFn` seam and is therefore still on the chain.

### 14.7 TEN CUTS ON THE ROW WALK -- 2026-08-26

E1's finder had never been through a cuts pass. Ten landed, and the gate is
sharper than byte-identity usually gets: **the row arm is bitstream-changing,
so `rowboard` at L9 was run before and after and all TWELVE corpora returned
IDENTICAL compressed sizes** -- proof that the candidate SET, its ORDER and the
acceptance test are unchanged and only the cost of reaching them moved.

| # | cut | what it removes, per position |
|---|---|---|
| 1 | `row_mask` cached in `RowTable` | `head.len()` Vec load off `row_of`'s dependency path |
| 2 | **the bit-walk** -- rotate so newest is the high bit, consume set bits | the old loop stepped k=1..=16 testing one slot per step; now one iteration per CANDIDATE |
| 3 | row base resolved once inside the walk | `r * ROW + s` per candidate |
| 4 | `probe()` fuses tag-compare + collection | `r * ROW` derived once instead of twice; the tag slice never materialises |
| 5 | `tag_row` -> `&[u8; ROW]`, kernel takes it | runtime slice length + the kernel's re-assert |
| 6 | `low.max(1)` folds the sentinel into the window floor | the `m == 0` test |
| 7 | fused reject `m.wrapping_sub(low) >= span` | 3 branches -> 1 |
| 8 | `lz_insert_rowknown` | `row_of(h)` recomputed inside `row_insert` (the finder already held `r`), plus the `(Option, u8)` return that caller discards |
| 9 | `src_len` hoisted | two slice-field re-reads |
| 10 | `ip <= low` early return | the entire walk when no position can qualify |

**THE STATIC SCREEN IS THE WRONG INSTRUMENT HERE, AND SAID SO.** Measured by
reverse-patching this session's edits and rebuilding: `row_find_best`'s two
monomorphs read **360 + 345 = 705 before and 363 + 341 = 704 after** -- flat.
That is section 0b's screen-inversion in its purest form: cut 2 changes a
loop's TRIP COUNT, and a trip count is invisible to a static count.

**The admissible receipt is the walk census** (`take_row_walk`, `rowwalk.rs`,
both new). It prices BOTH cost models from the SAME mask on every probe, so
one run compares them with no A/B build and no clock:

| corpus | probes | OLD slot visits | NEW | ratio |
|---|---:|---:|---:|---:|
| dickens | 2,874,645 | 34,219,143 | 19,877,919 | 1.72x |
| mozilla | 4,750,092 | 15,463,311 | 7,703,484 | **2.01x** |
| samba | 2,517,663 | 13,757,167 | 8,280,604 | 1.66x |
| webster | 2,132,871 | 24,181,103 | 14,917,657 | 1.62x |
| osdb | 2,881,425 | 19,348,903 | 12,682,537 | 1.53x |
| reymont | 1,783,155 | 22,124,641 | 14,693,590 | 1.51x |
| xml | 573,392 | 5,814,696 | 4,236,302 | 1.37x |
| nci | 527,196 | 6,135,316 | 4,824,766 | 1.27x |
| **total** | **18,040,439** | **141,044,280** | **87,216,859** | **1.62x** |

**53.8 MILLION slot visits deleted** over 64 MiB at L9 -- 7.82 per probe down
to 4.83. Gate: `rowboard` L9 identical on 12/12, `bytegate` GOLD
`BE0071FB0CB0CED9` unmoved (the arm-off default path), rowfind's kernel-vs-
oracle and ring-order tests green, 173 tests release + debug.

**Recorded and NOT taken here:** the chain maintenance. Taken in 14.8.

### 14.8 THE ROW FINDER WAS FEEDING A TABLE NOBODY READ -- 2026-08-26

14.7 flagged it and declined to take it. This section takes it, with the board
the flag asked for. **The result: 32 cells byte-identical with the entire
chain path removed.**

**The find.** `row_find_best` reads `rows` and nothing else. But every
position it visited called `lz_insert`, which maintains FOUR other structures:
the chain link (`chain_masked_set`), the chain tags (`ctags`), the hash head
(`lz_head_put`) and the head tags (`tags`) -- plus the `lz_head_raw` load and
the `old_tag` derivation that feed them. While the row arm is on, all of it is
written and never read.

**The proof is the board, not an argument.** Removing every one of those
writes and re-boarding:

| level | corpora | verdict |
|---|---:|---|
| L7 | 10 | **all identical** |
| L9 | 12 | **all identical** |
| L12 | 10 | **all identical** |

32 cells, three levels, zero changed bytes. A table that is read cannot be
stopped from being written without moving a single output byte.

| # | win (this round) | what it removes |
|---|---|---|
| 1 | the `[u32; ROW]` candidate array | 64 bytes of stack per position, and its init |
| 2 | per-candidate stores into it | ~4.83 per probe (measured popcount) |
| 3 | per-candidate loads back out | ~4.83 per probe |
| 4 | the `n` count and `take = n.min(attempts)` | a count, a min, a loop bound |
| 5 | attempt budget as a COUNTDOWN on the walk | the bound computed against a count that no longer exists |
| 6 | `probe_mask` returns `Copy` state | no out-parameter, no borrow plumbing at the call |
| 7 | the chain link store | one random-access store into a table up to 64 MB |
| 8 | the `ctags` store | one store |
| 9 | the head-tag store | one store |
| 10 | the hash-head store (`lz_head_put`) | one store |
| 11 | the `lz_head_raw` load | one random-access load |
| 12 | the `old_tag` derivation | a load or a shift, per position |
| 13 | `lz_insert_rowknown`'s signature: 7 args -> 3 | four argument set-ups per call |
| 14 | `cp`, `ca`, `chain_mask` dropped from the `ChainCtx` destructure | three struct loads per call |

**Static receipt:** `row_find_best` went from **two monomorphs at 360 + 345 =
705** instructions to a **single 301-instruction symbol** -- the bodies became
identical enough to fold. Unlike 14.7 (where the static screen read flat
because the win was a trip count), this round IS visible statically, because
it deletes straight-line work.

**The dynamic side is larger than the static side.** Six memory operations
per position disappear, four of them random-access stores into the multi-MB
chain and hash tables -- so this is cache pressure removed from the encoder's
hottest stage, not just instructions. At L9's measured 18,040,439 probes over
64 MiB, that is ~108M memory operations.

**THE RESIDUAL RISK, STATED PLAINLY.** 32 identical cells is an empirical
result about the paths these boards exercise, not a proof by construction. A
future caller that reads the chain while rows are active would find a stale
table. Two things bound it: the row arm is bitstream-changing and defaults
OFF, and `rowboard` is its shipping gate -- any such caller appears there as a
changed cell. The `lz_insert_rowknown` doc comment carries this warning at the
site.

### 14.9 THE 16-TO-1 BUCKET SHARING -- REFUTED as a cost. 2026-08-26

`row_of(h) = (h >> 4) & mask` folds SIXTEEN hash buckets into one 16-slot row,
so a tag match does not imply a bucket match. The hypothesis -- mine, written
into 14.7's follow-up list -- was that most tag matches are collisions from
the other fifteen buckets, each one costing an `mls_eq` (a RANDOM load into
`src`) that for `mls >= 4` cannot possibly succeed. If true, that is a large
byte-identical win: reject them from data already in the tag cache line.

**It is not true.** `examples/rowbucket.rs` (new) recomputes each examined
candidate's bucket and compares it to the probe's, L9, 8 corpora, 64 MiB:

| | count | share |
|---|---:|---:|
| probes | 18,040,439 | |
| candidates examined | 86,876,674 | 4.82/probe |
| **same bucket** | **86,472,281** | **99.5%** |
| cross bucket | 404,393 | **0.5%** |
| passed `mls_eq` | 85,461,170 | 98.4% of examined |
| `gtag == 0` probes | 117,318 | 0.7% |

**The 8-bit tag is doing its job almost perfectly.** Cross-bucket collisions
are 0.5% of examined candidates -- there is no meaningful wasted work to
reclaim, and the ceiling on the entire idea is half a percent. Pruned on the
arithmetic before anything was built (`codec-measurement` 11).

**Two corrections this census forces:**

1. **The sharing is not a defect; it is an implicit SPEED/RATIO trade, tuned
   toward speed.** A bucket-private row would give a hot bucket up to 16 slots
   instead of the 4.82 it averages today -- MORE candidates, better matches,
   and MORE work per position. Undoing the sharing would COST instructions and
   buy ratio. That is the opposite sign from the one this section set out to
   find, and it re-frames E1's +0.54% size as a knob rather than a flaw.
2. **The walk is near-optimal in candidate quality.** 98.4% of examined
   candidates pass `mls_eq`, and the range reject (`low`/`ip`) eliminates
   almost nothing (86.88M examined against 87.22M slot visits). There is no
   filter to add that would pay.

**The one win the census DID expose (W15).** `attempts = 1 << search_log` is
32 at L9 and 64+ at L12, against a measured **4.82 candidates per probe** -- so
the per-candidate countdown 14.7 introduced could never fire, yet cost a
decrement, a compare and a branch on all 86.9M of them. The budget now applies
to the MASK once per probe: the walk consumes set bits from the high end, so
"the newest `attempts` candidates" is "clear the lowest set bits until
popcount == attempts", and `w &= w - 1` clears exactly the lowest. The fixup
loop runs only when a row over-delivers.

Executed: roughly **-3 instructions x 4.82 candidates, +3 per probe** -- about
**-200M instructions at L9's probe count**. Static: **301 -> 324** for the
symbol, the usual inversion (0b) when a per-iteration cost is traded for a
per-call one. Gate: `rowboard` L9 12/12 identical, GOLD unmoved, 173 tests
release + debug.

**A correction to my own first reading of this census.** I initially wrote
that the vein was 0.5% deep and stopped at one win. That conflated two
different claims: *the bucket-collision hypothesis is dead* (true, and
permanently) with *the row path is exhausted* (false). The census does not
just refute -- it MAPS the per-position work, and reading it that way found
nine more. The refutation stands; the "nothing left here" did not.

**The nine, all byte-identical, `rowboard` L9 12/12 after each batch.** The
governing fact is one this section had to notice first: `row_find_best` is
`#[inline(never)]` and reached through a `ChainFn` POINTER, so nothing hoists
across positions -- every `ChainCtx` field it names is a real per-position
load, and every constant it derives is derived again.

| # | win | what it removes per position |
|---|---|---|
| W16 | `probe_view` merges `probe_mask` + `row_ref` | one of the two `r * ROW` derivations, and one call |
| W17 | `insert_at` takes the probe's `at` | the THIRD derivation of the same row offset |
| W18 | `insert_at` takes the probe's `head` | a reload of the byte the probe just read, on the dependency path of two stores |
| W19 | advanced head computed from the register value | a second read-modify-write of `head[r]` |
| W20 | `hash_shift32`/`hash_shift64` hoisted into `ChainCtx` | a `min` + a `saturating_sub`, on whichever of the three hash entries runs |
| W21 | `lowest1 = lowest.max(1)` hoisted | a `max` (cmov) per position |
| W22 | `hash_wide_link_tag_shift` | the third and last hash entry re-deriving the same block constant |
| W23 | `hash_log` dropped from the destructure | one ctx field load -- W20/W22 made it unreachable |
| W24 | `wchain` + `wide_hash` -> one `hash_mode` byte | one ctx field load, arm order preserved exactly (including a wide-hash block near the buffer end falling through to the wide-chain arm) |

Static: **324 -> 330** for the symbol. The ctx grew by three fields, so the
prologue that copies it grew; what shrank is the per-position work inside,
which is the 0b inversion this document keeps re-learning and the reason the
receipt is the operation list, not the symbol size.

**A process failure worth recording, because the gate caught it and a lint
did not.** W22-W24 removed `wchain`, `wide_hash` and `hash_log` from the
destructure -- and the `profile`-only census block added earlier in this very
section still referenced all three. `cargo check` passed because I ran it
without the feature; the PROFILE build did not compile, so `rowboard` never
ran, and the comparison printed an empty column that a careless `uniq` then
rendered as "12 CHANGED". Two lessons, both already in this repo's rules:
**check every cfg arm you have code under**, and **an empty result is not a
result** -- read the raw numbers before believing a summary of them.

`rowbucket.rs` stays in the tree so the next person with the bucket
hypothesis can re-run it in a minute instead of building it.

### 14.10 THE BACK-FILL WAS 2.3x THE FINDER, AND 14.8 HAD NOT TOUCHED IT

Asked whether more was hiding, the answer was yes, and it was the largest
single item in the whole E1 campaign -- found by asking where the row-mode
inserts actually COME FROM.

14.8 proved the chain link, chain tags, hash head and head tags are never
read while the row arm is on, and removed them from `row_find_best`'s insert.
**It stopped there.** `find_lazy_impl` also re-inserts every position a match
covered, striding one byte, and that fill kept doing the full thing.
`examples/lazyfill.rs` (new) prices the two against each other at L9:

| | count |
|---|---:|
| finder probes (14.8's site) | 18,040,439 |
| fill sites | 3,419,223 |
| **fill inserts** | **41,742,765** |
| **fill : probe ratio** | **2.31x** |

Per corpus the ratio runs 0.75x (`mozilla`) to **10.80x** (`nci`) -- the
content with the longest matches back-fills the most, which is exactly the
content the row finder is otherwise best on.

| # | win | what it removes |
|---|---|---|
| W25 | the fill's two hash shifts hoisted | a `min` + `saturating_sub` on **41.7M** inserts -- the fill strides ONE byte, so this was once per matched byte |
| W26 | the fill's dead chain path, in row mode | chain link + ctags + head tags + hash head stores, plus the head load and tag derivation that feed them: **~6 memory ops x 41.7M ~= 250M**, four of them random-access stores into multi-MB tables |
| W27 | the non-row arm takes `lz_insert_only` | W10 built that entry for callers that discard the result; this site still called full `lz_insert` and threw the `(Option<usize>, u8)` away with `let _ =`. **This one lands on the DEFAULT path**, so `bytegate` is its gate |

**W26 is larger than everything 14.8 removed** -- 2.31x the volume, same six
operations. The lesson is the one `rusty_curiosity` keeps charging for:
14.8 found a dead table and fixed the site it was standing in, instead of
asking who else wrote to it. **When a structure turns out to be dead, grep
for every writer before closing the finding.**

Gate: `bytegate` GOLD `BE0071FB0CB0CED9` unmoved (W27's path), `rowboard`
identical at L7, L9 and L12 (W25/W26's path), 173 tests release + debug.

### 14.11 THE FILL'S MARGINAL VALUE IS PRICED -- and half of it is nearly free to drop

14.10 removed the fill's DEAD work. This asks the harder question: is the
work that remains WORTH doing? The fill exists for defect B1 (positions inside
a match are absent from the finder's table, so later searches see a thin table
and find worse matches), and that defect was real -- but "necessary" is not
"necessary at stride 1".

**First, two hypotheses killed by counting:**

- *Fill sites that insert nothing, paying setup for zero work.* **83 of
  3,419,223 (0.0%).** Dead.
- *The fill is a long tail on a few corpora.* No -- it is 12.21 inserts per
  NON-EMPTY site everywhere, ranging 6.19 (`dickens`) to **37.56 (`nci`)**.
  The content with the longest matches fills the most, which is the content
  the row finder is otherwise best on.

**The sweep (`examples/fillsweep.rs`, new; the stride knob already existed).**
L9, row arm ON, 8 corpora, round-trip asserted on every cell:

| stride | fill inserts | vs stride 1 | size | vs stride 1 |
|---:|---:|---:|---:|---:|
| **1** (shipping) | 41,742,765 | 1.00x | 17,906,078 | 1.0000x |
| **2** | 21,749,282 | **0.52x** | 17,955,152 | **1.0027x** |
| 3 | 15,024,082 | 0.36x | 18,011,261 | 1.0059x |
| 4 | 11,713,837 | 0.28x | 18,079,418 | 1.0097x |
| 8 | 6,740,929 | 0.16x | 18,204,714 | 1.0167x |

**Stride 2 removes ~20 MILLION inserts -- 48% of the fill -- for 0.27% of
size.** The curve is steeply diminishing: the second half of the fill buys a
quarter of a percent, and the fourth stride buys another 0.4%.

**Put that beside what this codec already trades.** E1 itself ships (when it
ships) at +0.54% size for -23.6% match-find time. Stride 2 asks +0.27% for
roughly half of a stage that is 2.31x the finder's own probe count. That is a
comparable exchange rate on a knob that already exists and needs no new code.

**NOT flipped here, deliberately.** It is bitstream-changing, the sweep is one
level on eight corpora, and E1's own default is still off -- stacking a second
un-boarded ratio decision underneath it would make both unattributable. What
this section delivers is the PRICE, so the decision is a reading rather than a
guess. Board it across levels and the full corpus set before touching the
default.

**W28, the one byte-identical win left in the fill path:** `row_insert`
re-tested `!rows.head.is_empty()` on every call, but the fill only reaches it
when `use_rows` -- which is exactly that predicate, decided once per block. A
`Vec` length load and a branch, on 41.7M inserts.

### 14.12 THE STRIDE LANDS, ROW-SCOPED -- and the mean was hiding the spread

14.11 priced the fill and the call was made to take stride 4. Two things had
to be established before landing it, and both changed the shape of the change.

**1. The stride is SHARED, and the chain pays more for it.** `fill_stride`
feeds `find_lazy_impl` whichever finder is running, so moving the default
moves the SHIPPING path. Swept separately at L9 (the row column was measured
with the arm genuinely off -- `ROW probes = 0` is the witness):

| stride | fill inserts | ROW size | CHAIN size |
|---:|---:|---:|---:|
| 1 | 41.7M | 1.0000x | 1.0000x |
| 2 | 21.7M (0.52x) | 1.0027x | 1.0034x |
| **4** | **11.7M (0.28x)** | **1.0097x** | **1.0116x** |
| 8 | 6.7M (0.16x) | 1.0167x | 1.0194x |

So the stride is now **row-scoped**: `row_fill_stride()` defaults to 4, the
chain keeps `lazy_fill_stride()` at 1, and `bytegate` GOLD
`BE0071FB0CB0CED9` is UNMOVED. Two bitstream decisions, two boards, neither
dragging the other.

**Landed effect:** fill inserts **41,742,765 -> 11,713,837**, and the fill
falls from **2.31x the finder's probe count to 0.65x** (12.21 inserts per
non-empty site -> 3.45). Thirty million random writes into an 80 MB table,
gone.

**2. THE AGGREGATE UNDERSTATES THE COST, and this is `codec-campaign-laws`
charging its usual fee.** "+0.97%" is a total over eight corpora, and totals
are dominated by the big poorly-compressing files. Per corpus at L9:

| corpus | s2 vs s1 | s4 vs s1 | s4 vs CHAIN |
|---|---:|---:|---:|
| dickens | +0.67% | **+1.94%** | **1.0517x** |
| reymont | +0.67% | **+1.93%** | **1.0497x** |
| webster | +0.49% | **+1.94%** | 1.0375x |
| mr | +0.91% | +1.74% | 1.0169x |
| x-ray | +1.48% | +1.54% | 1.0271x |
| samba | +0.02% | +0.58% | 1.0137x |
| sao | +0.21% | +0.35% | 1.0129x |
| osdb | +0.16% | +0.30% | 0.9891x |
| mozilla | +0.10% | +0.24% | 1.0022x |
| ooffice | +0.34% | +0.80% | 0.9856x |
| nci | **-0.06%** | +0.83% | 0.9707x |
| xml | **-0.27%** | +0.46% | 1.0317x |

**On TEXT the cost is double the mean** -- ~1.9% for stride 4 -- and stacked
on E1's own it puts `dickens` and `reymont` at **+5% against the chain**,
which is a large price for an encoder to pay for speed.

**The marginal rate is where the argument is:**

* 1 -> 2: **-20M inserts** for ~+0.5% on text, and `nci`/`xml` get SMALLER.
* 2 -> 4: **-10M inserts** for another ~+1.2% on text.

The second step buys half as much work for more than twice the size.

**SHIPPED: stride 2.** The spread decided it. Landed effect at L9:

| | stride 1 | **stride 2 (shipped)** |
|---|---:|---:|
| fill inserts | 41,742,765 | **21,749,282** |
| fill : probe ratio | 2.31x | **1.21x** |
| inserts per non-empty site | 12.21 | **6.42** |
| worst text corpus | -- | **+0.67%** (`dickens`, `reymont`) |
| best | -- | **`xml` -0.27%, `nci` -0.06% -- SMALLER** |

**Twenty million inserts removed -- half the fill -- for 0.27% aggregate and
at most 0.67% on any text corpus, with two corpora improving.** `x-ray` is
the outlier at +1.48% and is the one cell to watch when this is boarded
wider.

The `set_row_fill_stride_arm` / `RZSTD_ROW_FILL_S` knob keeps every other
stride one call away, and the sweep that priced them is `fillsweep.rs`.

### 14.13 THE FILL LOOP ITSELF -- ten cuts on "inserts per non-empty site"

14.12 halved HOW MANY inserts run. This works what each one COSTS. The loop
had never been read as a hot loop: it re-asked two block constants and a
bounds pair on every position, and the sibling fill in `find_greedy_impl`
had drifted a full campaign behind.

**The tell was in the numbers already on the page.** 6.42 inserts per
non-empty site is a tight inner loop, and `find_greedy_impl`'s fill already
carried a bounds fold (its W7) that this one never received -- two fills, one
defect class, fixed in only one of them. That asymmetry is where the round
started.

| # | win | what it removes per inserted position |
|---|---|---|
| W29 | `stop = end.min(ilimit + 1)` | one of the two bounds compares -- the fold greedy's fill had and lazy's did not |
| W30 | the hash MODE hoisted out of the loop | 1-2 branches; each arm now has ONE hash in its body |
| W31 | the `use_rows` table choice hoisted out | one branch |
| W32 | `p + 8 <= src_len` gone from the two non-wide arms | an add and a compare, on every position that is not using the 8-byte hash |
| W33 | `row_mask` hoisted | a struct load per insert -> one per fill site |
| W34 | `insert_h` folds `row_of` INTO the insert | one call boundary per position |
| W35 | the `&mut rows` borrow taken once | lets LLVM hoist the `pos`/`tags`/`head` base pointers out of the loop entirely |
| W36 | greedy's fill: both hash shifts hoisted | a `min` + `saturating_sub` **twice per matched byte** -- greedy strides 1 |
| W37 | greedy's fill: hash mode hoisted | 1-2 branches per position, same shape as W30 |
| W38 | greedy's fill keeps `lz_insert_only` | W10's entry, now reached from all three specialised arms |

**Gate:** `bytegate` GOLD `BE0071FB0CB0CED9` unmoved -- which matters more
here than usual, because W36-W38 land on `find_greedy_impl`, the SHIPPING
finder at L5. `rowboard` L9 12/12 identical against the stride-2 baseline,
`no_std` and `profile` clean, 173 tests release + debug.

**The pattern worth keeping.** Three fills exist in this encoder and they
drift apart: greedy's had the bounds fold but not the shift hoist, lazy's had
neither until 14.11, and each fix has been applied to one site at a time.
Section 14.10 already charged for this once ("when a structure turns out to
be dead, grep for every writer"). The same rule applies to a LOOP SHAPE:
**when a fill gets a cut, check the other fills the same day.**
**Still to do before E1's own default flips:** board this at L7 and L12. L7
is where E1 is already a wash, so it has the least headroom to spend, and a
stride that is free at L9 is not automatically free there.

### 14.6 The board COMPLETED -- L7 and L12 measured, 2026-08-24/26

14.4 required boarding L5-L12 before any ship decision. L5 stays out of reach
(greedy's hand-copied walk bypasses `ChainFn` -- 14.2). The other two:

| level | chain -> row (total ms) | delta | pairs | z | size |
|---|---|---:|---:|---:|---:|
| L7 | 2892.5 -> 2854.8 | **-1.3%** | 121/252 | -0.63 | 1.0059x |
| L9 (14.3) | -- | **-23.6%** | 205/252 | +9.95 | 1.0054x |
| L12 | 15973.2 -> 5172.5 | **-67.6%** | **248/252** | **+15.37** | 1.0191x |

**L12 is a 3.1x encode speedup, every corpus 21/21.** The profile is monotone
with chain depth exactly as `rowceiling`'s load counts predicted (1088 ->
1725 -> 5263 loads/KiB). L7 is a WASH overall with hard per-corpus
sign-flips (webster -7.8% z +3.71 against nci +19.6% z -4.58, ooffice +18.3%
z -3.71) -- `codec-content-adaptive-dispatch`'s trigger, not a ship-or-revert.

**Ship shape this evidence supports:** row finder ON by default at L9+ (where
it wins broadly), chain kept at L7 (or dispatched per-content later), greedy
(L5) still needing its `ChainFn` wiring before it can even be measured. The
default flip remains a product call; the measurement debt named in 14.4 is
now PAID.

### 14.5 Standing instrument fix

`prof::scope` times with `Instant` -- WALL. On a loaded box every deschedule
inside a scope is charged to that scope, which is a large part of what inflated
the worst-pair null. codec-measurement 2 measured wall spread 0.78-1.50 against
CPU-time 0.950-1.089 on identical work. Moving the stage clock to a THREAD CPU
clock (`QueryThreadCycleTime` / `CLOCK_THREAD_CPUTIME_ID`) tightens every
in-process measurement in this project at once. That is the highest-leverage
measurement work outstanding.



## 15. The stage clock: fixed, verified, and SMALLER than advertised

14.5 called the wall-clock stage timer "the highest-leverage measurement work
outstanding." It was worth doing and that billing was too high. Both halves are
recorded here because a wrong instrument diagnosis is as expensive as a wrong
codec one.

### 15.1 What was fixed

`crates/rusty_zstd/src/profclock.rs`. `prof::scope` timed with `Instant` --
WALL -- so any moment the thread spent DESCHEDULED was charged to whichever
scope was open. Now: `QueryThreadCycleTime` on Windows (thread CPU cycles),
`clock_gettime(CLOCK_THREAD_CPUTIME_ID)` on Linux, `Instant` elsewhere so no
platform loses the profiler. Profile-only; the shipping codec is untouched.

Windows counts cycles and every consumer reports nanoseconds, so the ratio is
calibrated once against `Instant` over short busy spins, keeping the HIGHEST
cycles-per-wall-ns sample (descheduling inflates wall and leaves cycles alone,
so the largest ratio is the least-interrupted trial).

### 15.2 The calibration is SOUND -- and a wrong inference, corrected

A cycles->ns error is invisible in ratios and fatal in absolutes, and every
"ns/seq" in this document is an absolute. `examples/clockcheck.rs` settles it on
one inequality: **thread CPU time cannot exceed wall time for the same region.**

| corpus | stage ns | wall ns | ratio |
|---|---:|---:|---:|
| dickens | 33,368,771 | 34,821,900 | 0.958 |
| webster | 27,952,205 | 28,473,100 | 0.982 |
| samba | 20,334,526 | 20,916,800 | 0.972 |
| nci | 13,926,490 | 14,271,300 | 0.976 |
| xml | 8,683,929 | 9,456,100 | 0.918 |
| mozilla | 14,823,140 | 15,216,100 | 0.974 |
| **total** | **119,089,061** | **123,155,300** | **0.967** |

Consistent everywhere, never above 1.0.

It was briefly concluded from `d9probe`'s OFF arm reading 33.91 ns/seq on wall
and 44.08 on CPU that the conversion was ~30% out. **That was wrong.** Those are
different runs minutes apart on a variable-load box; the OFF arm ranged
33.9-44.1 across this session on both clocks. Two numbers from un-interleaved
runs are not a comparison -- the same rule that section 3 of codec-measurement
states, applied to an instrument instead of a codec.

### 15.3 Why it did not shrink the null arm, and what that PROVES

| clock | median null | worst null |
|---|---:|---:|
| wall (`Instant`) | 3.79% | 18.87% |
| thread CPU | 4.28% | 19.81% |

Unchanged. The 0.967 ratio above says why, quantitatively: only **3.3% of wall
sits outside the profiled scopes AT ALL**, descheduling included. Descheduling
inside scopes therefore cannot account for a 4% median or a 19% worst pair --
the ceiling on what this fix could ever have removed was ~3%.

**So the residual noise is NOT scheduling. It is memory-system contention.**
Eight `Code.exe` processes plus `faucet.exe` thrash L3 and memory bandwidth, and
our thread pays that in REAL CYCLES. A CPU clock excludes time spent
descheduled; it does not exclude cycles burnt on cache misses another process
caused, and nothing does.

The practical consequence: **~4% median is close to this box's true floor**, and
no further instrument work will move it. Effects under ~4% need deterministic
counters (codec-measurement 15) or the sign test. That is a more useful thing to
know than the fix itself.

### 15.4 Kept anyway

It measures the right quantity, is verified consistent, costs nothing at runtime
(profile-only), and on a scheduling-contended box it would matter. The
diagnostic value is separate: `clockcheck.rs` now prices scope coverage against
wall in one run, which is how 15.3 was settled at all.

### 15.5 A second defect, found by the same run: the tap taxed its own brick

`d9probe` on a plain `profile` build read D9 at **+14.3%, z = -10.26**, against
**+0.2%, z = -0.91** before. Nothing about D9 changed. The difference was the
`note_pf` COVERAGE census added for `d9cover.rs`: two atomics PER SEQUENCE, in
the build used to TIME the brick they describe. codec-measurement 6, exactly.

Fixed by a new `pfcensus` feature, separate from `profile`, so a timing build
does not carry the counters. `d9cover` now requires it. Re-measured clean:
**D9 = +0.4%, 60/117 pairs, z = +0.28** -- confirming the original reading and
leaving D9 where it was, a genuine null.



## 16. COMPLETENESS RE-VALIDATED -- 2026-08-24. The census diffed against the tree, and the residue adjudicated.

The 1b census is this document's completeness argument, and it had never been
checked by anyone who did not write it. It was re-run independently: every
non-test `for`/`while`/`loop` in the crate, counted per file, attributed per
function, and diffed against the 1b table.

### 16.1 The census reproduces, and every delta has a named cause

**Nine files match the 1b table EXACTLY** (fse 25, train 20, mt 7, decode 6,
prof 5, seekable 4, stream 2, bit 1, params 1) -- so the 232-loop census was a
real measurement whose method an independent count can reproduce. The files
that moved, moved for reasons this document already records:

| file | 1b | now | cause |
| --- | ---: | ---: | --- |
| `simd.rs` | 13 | 9 | shipped bricks DELETED loops (D7 tail, twin cleanup) |
| `xxh64.rs` | 12 | 8 | `stripes_pre` factoring (D8b) collapsed hand-copied walks |
| `ldm.rs` | 4 | 3 | E2 deleted the byte-compare loop |
| `huffman.rs` | 64 | ~86 | the allocation campaign's `_into` writers, `pop_min` (N13), `covers_freq` (E11) -- i.e. the FIXES, plus kept reference twins |
| `compressed.rs` | 14 | 14+ | the D9/D10 arms |
| `encode.rs` | 52 | 54 | E1's `row_find_best` wiring |

A shrinking census in the files where bricks shipped, growing only where fixes
landed, is what a healthy campaign should read like.

**One census-method correction, recorded so the next run does not re-flag it:**
`compressed.rs`'s `pack_ll` / `pack_ml` / `build_code_lut` are **`const fn`** --
`const LL_PACK: [u32; 36] = pack_ll();` runs at COMPILE TIME. Four of the
file's counted loops execute zero times at runtime. A loop census counts
source, not execution; these are the reminder to check which one a candidate is
before ranking it.

### 16.2 seekable.rs -- the one walked-but-silent file, now adjudicated

The 1b table counted seekable.rs's 4 loops and no section of this document ever
gave them a verdict. Read in full today:

| loop | frequency | verdict |
| --- | --- | --- |
| `compress_seekable_adv` frame walk | once per **2 MiB** frame (default) | per-frame plumbing; the work is `encode_oneshot`, already the whole campaign's subject |
| `append_seek_table` / `parse_seek_table` | once per FILE, over one 12-byte entry per 2 MiB of content | header I/O; no per-byte work exists here |
| `decompress_frame_at` entry walk | O(entries) linear scan **per seek** | honest note: a binary search over prefix sums beats it -- at one entry per 2 MiB that matters from ~millions of frames, i.e. terabyte files. Not built; recorded with its threshold |

**CLOSED.** No inline scalar byte work, nothing for this plan. The gap was in
the paper trail, not the code.

### 16.3 The small-parse residue -- named, read, closed

Functions with loops that no D/E/V/N item ever named, each read today:

| site | shape | verdict |
| --- | --- | --- |
| `huffman::write_tree_raw` | nibble-pack, <=64 iterations/table, pooled (ALLOC-15) | E12-class volume (11x SMALLER than E12's pruned 7M visits); below every floor |
| `huffman::read_table` / `read_ctable` direct-weights arm | nibble-unpack, <=128 symbols/table | same class, decode side |
| `fse::parse_ncount_into`, `weights_into_inner`, `decompress_weights_inner` | bit-serial FSE decode, <=256 symbols, W29-presized, WEIGHT_TBL-recycled | **serial recurrence** -- the E6/D6 argument verbatim; not vectorisable, already allocation-clean |
| `compressed::build_code_lut`, `pack_ll`, `pack_ml` | `const fn` | compile-time; zero runtime loops (16.1). Runtime LUT use was adjudicated by brick 35 (`set_lut_arm`) |

### 16.4 The census is a SNAPSHOT, not a standing gate

Everything written after 2026-08-22 is outside the 1b census by construction.
Audited today: **`rowfind.rs`** -- doctrine-compliant (scalar oracle dead by
`cfg_attr` on x86/aarch64, shipping kernel baseline SSE2 so Law 1 does not
apply, NEON movemask mirror, lane order pinned by test); **`profclock.rs`** --
2 profile-only calibration loops; **`scratch.rs`** -- zero loops. The
fn-pointer population (12.1's blind spot) was re-checked: **still exactly the
four audited sites**, and E1 wires through the already-audited `ChainFn` seam.

**The standing rule this earns:** the census, the 11.1 dead-code grep, and the
12.1 fn-pointer check are point-in-time instruments. Re-run all three whenever
a new source file or a new fn-pointer type lands -- the census by mechanical
count with per-function attribution, exactly as here -- and append the diff to
this section rather than trusting the 1b table, which is now a historical
record. The 11.1 recommendation ("make this a recurring audit") is hereby its
own precedent: this section IS that audit's first recurrence, and it found the
tree clean.


## 17. TEN CUTS ON THE DECODER'S SEQUENCE LOOP -- 2026-08-24

The m7 anatomy says Loop is 97.2-98.9% of DecSeq at every level; the 13.5
ladder prices its ops. The finders all had their ten-cuts pass; this is the
sequence loop's. **Every receipt is a count; the gate is byte-identity.**

| | before | after | delta |
|---|---:|---:|---:|
| `decode_sequences` (baseline twin) | 2,755 | **1,608** | **-41.6%** |
| `decode_sequences_avx2` | 2,708 | **1,570** | **-42.0%** |
| crate total | 238,749 | 236,934 | -1,815 net |

Gate: `bytegate` GOLD **BE0071FB0CB0CED9** unmoved; 173 tests green in release
AND debug (debug carries the hostile-input `copy_literals` case and every new
`debug_assert`); both cfg arms (`profile` on/off) compile clean.

The cuts, each with its mechanism:

1. **Pattern-A `pipeline_on()`** -- the parked D10 block, including TWO macro
   expansions of the full symbol-decode body, folded out of both twins in
   non-`profile` builds. The single biggest static cut. Probes keep the arm
   under `--features profile`.
2. **Pattern-A `prefetch_on()`** -- the parked D9 `prefetch_hist` body and its
   per-sequence branch, same fold.
3. **Pattern-A `seqcheck_hoisted()`** -- shipping builds fold to `true` (the
   brick-45 default); the per-sequence `!seqcheck && (range tests)` vanishes.
4. **`offset_value` branch deleted by algebra** -- `read_bits(0) == 0` and
   `1 << 0 == 1`, so `(1 << of_code) + offset_add` IS the special case. One
   branch per sequence, in both loop bodies.
5. **`resolve_offset` single dispatch** -- the old body re-tested
   `offset_value > 3` and re-matched the rep index a second time to apply the
   update the first match had determined. Each RFC repcode case now fuses its
   update; every condition tested once. The 5-instruction outlined residual
   symbol is gone entirely.
6. **Pattern-A `litcopy_on()` / `matchcopy_on()`** -- const `true` in shipping;
   the per-sequence `arm` tests in every copy tier become tautologies.
7. **`reps` register promotion** -- the rep history moves from behind the
   `&mut BlockState` pointer into a local `[u32; 3]`, written back once per
   block.
8. **`copy_literals` overflow branch** -- `checked_add` per sequence replaced
   by a remaining-length compare that cannot overflow on any width; the sum is
   computed only after it is proven to fit.
9. **Dict-path arm outlined cold** -- `copy_match` (inline(always)) was
   stamped into both twins as the `else` of `if nodict`; it now sits once
   behind `copy_match_dict_cold` (203 instructions, its own symbol) per the
   round-five duplication rule.
10. **`copy_match_nodict` guard fold** -- `off == 0` and `off > produced`
    share one compare via `off.wrapping_sub(1) >= produced`; both rejected the
    same inputs to the same error.

Cuts 3-8 and 10 are EXECUTED-path reductions (fewer instructions per sequence
retired); 1, 2 and 9 are static-size cuts that shrink the loop bodies the
front-end streams. No timing number is quoted -- this box's floor is ~4%
median (15.3) and the deterministic receipts above are the admissible record.


## 18. TEN MORE -- the second decode sweep, 2026-08-24

Section 17 worked the sequence loop; this round follows the same laws one
level out: the per-SYMBOL Huffman loops, the streaming plumbing, and the
checks that re-prove build-time invariants per element. Gate: `bytegate` GOLD
**BE0071FB0CB0CED9** unmoved, 173 tests green in release AND debug (debug
carries every new invariant as a `debug_assert`), all five configs clean.

| # | cut | deterministic reduction |
|---|---|---|
| 1 | **decode_into_x1/x2 get their ISA back.** Round five's outlining left the per-symbol loops as ONE baseline symbol called from all three `decode_4x` twins -- the 12.4 shim trap, live. `#[target_feature]` twins, statically selected by `const ISA` from the enclosing twin (and by one per-SECTION CPUID pick on the 1-stream path). | x1: 14 `shl %cl` -> **14 `shrx/shlx`**; x2: 17 -> **17** -- every variable shift in the per-symbol loop goes 3 uops -> 1 on the hardware that runs it. +1,132 static for 4 twin symbols, the deliberate inverse of the trap. |
| 2 | **`decode_one` infallible.** Its `nbits == 0` reject re-proved Kraft-completeness per SYMBOL; `read_table` already rejects any non-exactly-complete tree (`rest` a power of two, derived last weight, `rank[1]` parity). | One test-and-branch per LITERAL deleted from every unrolled copy: `decode_4x` 2,875 -> **2,712**, `_bmi2` 2,610 -> **2,388**, `_avx2` 2,435 -> **2,275**; `decode_into_x1` 273 -> **241**. |
| 3 | **`FseView::entry_u32` unmasked.** `init_state < 2^log` and `baseline + (2^nb - 1) <= len - 1` hold for every entry `from_norm_buf` can emit (it rejects `ns == 0`, `hb > log`), and `rle` pins state 0. | 3 AND instructions per sequence deleted; `decode_sequences_avx2` 1,570 -> **1,546**. |
| 4 | **Streaming decoder: the per-block payload copy.** `try_block` did `input[a..b].to_vec()` -- a heap alloc + full copy of EVERY payload -- purely to end a borrow the cursor makes unnecessary. | `streamcost` (10.2 MB, 256 blocks, 64 KiB chunks): **278 -> 23 allocator calls (1.09 -> 0.09/block), 17,423,464 -> 7,452,088 bytes requested (-57%)**; the remainder is the decoded buffer itself. |
| 5 | **Streaming decoder: input cursor.** Five sites drained `input` per parsed unit -- an O(remaining) memmove per BLOCK. `in_off` + reclaim only on fully-consumed (a `clear`) or a 64 KiB dead prefix. | **258 O(remaining) memmoves per streamcost run -> amortised compactions**; total bytes moved now bounded by bytes fed, not units x remaining. |
| 6 | **`lut_on()` -> pattern A.** The brick-35 LL/ML code LUT has been the shipping default since it landed. | The linear-scan alternative folds out of the three `write_sequences` twins in non-`profile` builds; the per-block atomic read goes with it. |
| 7 | **Compressor stream hygiene.** `out_acc.drain(..n)` ran per CALL on the unread remainder, and `compact_in` drained on every call. | Per-call O(remaining) memmoves -> `out_off`/threshold reclaim, same shape as cut 5, encode side. |
| 8 | **`Reader` N6, finally closed.** `u16_le`/`u32_le`/`u64_le`/`u24_le` assembled element-wise through `take(n)`'s unknown-length slice. | 2/4/8/3 per-byte bounds checks -> ONE structural length proof each (`try_into`), the form `peek_u32_le` has carried since T4. |
| 9 | **`copy_from_decoded` hot entry.** `copy_match_nodict`'s fused guard already proves `src < out.len()`, RFC `matchlen >= 3` proves `len > 0`, and `offset == 0` was unreachable behind the first check. | **3 per-sequence entry tests bypassed** on the no-dict path; the checked wrapper remains for the dictionary path and tests. |
| 10 | **The `offset == 1` splat demoted.** Band 0 of the census is 0.01% of match copies, and offset 1 can never satisfy any tier's `offset >= width`. | Its test moves off the 86.6% tier-1 path into `copy_from_decoded_cold`; dispatch outcomes provably unchanged. |

Crate total: 236,934 -> 237,713 -- **+779 net**, the +1,132 ISA-twin cost less
~350 of cuts, and that trade is the point: rounds one and two together hold
the sequence loops at **-42%** while this round returns the per-symbol loops
to the ISA the twin campaign bought them. `examples/streamcost.rs` is the new
standing instrument for the streaming plumbing.


## 19. THE STREAMING DECODER'S HIDDEN QUADRATIC -- 2026-08-24

Section 18's cut 4 removed the per-block payload copy and left a residue the
census could still smell. Dug into with a new instrument (`take_dec_compact`,
`profile`-gated: compaction count + bytes memmoved), it turned out to be the
biggest single overhead in the whole streaming path -- and invisible to every
whole-frame gate, because only `Decompressor::stream` reaches it.

**What the counter found (webster, 32 MiB decoded, 10.2 MB frame):**

| feed pattern | compactions | bytes MEMMOVED | vs decoded bytes |
|---|---:|---:|---:|
| 64 KiB chunks | 147 | 306,315,264 | **9.1x** |
| whole frame, one call | 255 | **4,278,190,080** | **133x** |

`compact()` drained `decoded` on EVERY `stream()` call, memmoving the retained
2 MiB window each time; and the progress loop's only early exit was
`output.is_empty()`, so a one-shot feed decoded ALL 32 MB into `decoded`
before returning a byte -- 121 MB of allocator traffic -- and then paid the
classic O(n^2) front-drain reading it back out. N3 rehomed the drain pattern
to 12.2; the allocation campaign fixed the ENCODER and never came back for
this.

**Three bricks:**

- **A -- decode no further than the caller can drink.** The loop now stops
  once pending output can fill the caller's buffer, bounding `decoded` at
  ~output + window + block regardless of how much input is fed at once.
- **B -- amortise the reclaim.** Compact only once a full WINDOW of dead
  prefix has accumulated, so each compaction reclaims at least what it moves
  and total traffic is bounded by ~1x decoded bytes, for one extra window of
  memory held.
- **C -- size `decoded` from the header.** Reserve
  `min(content_size + block, 2*window + 2*block, 8 MiB)` at header accept;
  the caps keep a hostile header from allocating past 8 MiB unverified.

**After (same instrument, same corpus):**

| feed pattern | compactions | bytes memmoved | allocator calls | bytes requested |
|---|---:|---:|---:|---:|
| 64 KiB chunks | **16** | **31,588,352 (0.9x)** | 24 -> **10** | 7.45 MB -> **4.96 MB** |
| one-shot | **16** | **31,588,352 (0.9x, was 133x)** | 25 -> **7** | 121.4 MB -> **14.7 MB** |

The one-shot column now READS THE SAME as the chunked one -- feed pattern no
longer changes the cost, which is the property a streaming API is supposed to
have. Behavioural note: a caller that fed everything in one call now gets
`done` after draining rather than in the first call; `StreamStatus::done` has
always been the loop condition the API documents.

Gate: `bytegate` GOLD unmoved, 173 tests release + debug, `no_std + alloc`
and wasm32 clean. `streamcost.rs` prints both feed patterns and the counter,
so the next regression here has an instrument waiting for it.


## 20. THE ENCODER'S MIRROR -- half a billion inserts of slide overhead, 2026-08-24

Asked what else was hiding around section 19, the symmetric question answered
itself: `Compressor::emit_block` slides `hist` past the window at
`hist > window`, which at steady state is EVERY BLOCK -- and each slide
memmoves the retained window, zeroes six match tables (`tables.reset()`), and
re-primes the ENTIRE window at stride 1. One-shot encode never runs this path,
so no board, gate or census had ever priced it. `take_enc_slide` (new,
`profile`-gated) plus the pre-existing `take_prime_iters` did:

**Before (webster 32 MB, L3, 256 KiB chunks):**

| | count |
|---|---:|
| window slides | **240** (one per block) |
| hist bytes memmoved | **503,316,480 -- 15.0x the source** |
| prime inserts | **503,314,800 -- half a BILLION** |
| table resets (x6 fills each) | 240 |

For scale: E1's whole L9 chain walk is 221M dependent loads over 96 MiB; the
streaming encoder was doing 2.3x that many table inserts on a THIRD of the
data, as pure bookkeeping.

**The fix is section 19 brick B, mirrored:** slide at `hist >= 2 * window`,
retaining one window -- each slide then reclaims at least what it moves and
all three costs amortise to ~1x the stream, for one extra window of memory.
Safe for the reason one-shot already proves daily: the finders clamp offsets
to `window` internally regardless of how much history the buffer holds.

**After:** **15 slides, 31,457,280 bytes memmoved (0.9x), 31,457,175 prime
inserts** -- a **16x reduction** on all three axes.

**The honest size trade, anchored to one-shot.** This is bitstream-changing
on the STREAMING path (one-shot is untouched; `bytegate` GOLD unmoved):

| | bytes | vs one-shot |
|---|---:|---:|
| one-shot `compress_with` | 10,234,302 | -- |
| streamed, per-block slide (old) | 10,150,089 | **-0.82%** |
| streamed, hysteresis (new) | 10,221,196 | **-0.13%** |

The old code was spending 15x the work to BEAT one-shot by 0.8% -- per-block
stride-1 re-priming builds denser tables than natural finder operation. The
new point is ~one-shot parity on ratio AND work, which is the principled place
for a streaming API to sit. If the denser tables are ever wanted back, the
knob is the hysteresis factor, and `take_enc_slide` prices it.

Gate: round-trip asserted in `streamcost`; the streamed frame decodes
**byte-exact through C zstd v1.5.7** (the external gate); `bytegate` GOLD
unmoved (one-shot untouched); 173 tests release + debug; `no_std + alloc` and
wasm32 clean. Before flipping any default that depends on streamed SIZE,
board the streaming ratio across the 18 corpora, not this one file.


## 21. SEQUENCE-DENSITY PARITY -- C's frames have MORE sequences, and C still wins. 2026-08-24

The demo's decode arms each decode their own encoder's bitstream, so "C
decodes 1.71x faster" confounds decoder speed with bitstream shape. 13.3
counted OUR density only; `examples/cseqdensity.rs` (new) decodes C-produced
L3 frames through the same counters:

| | copies/MiB | total c-size (8 corpora, 16 MiB caps) |
|---|---:|---:|
| our L3 frames | 66,774 | 28,578,403 |
| C v1.5.7 L3 frames | **69,755 (+4.5%)** | **27,263,908 (-4.6%)** |

**Two findings, one per axis:**

1. **The decode gap is ALL per-sequence decoder cost.** C's decoder handles
   4.5% MORE sequences 1.71x faster -- ~18 ns/seq against our ~33. Nothing
   about our bitstream excuses the gap; the fixed per-sequence cost 13.5
   located (`copy_match`'s ~23 ns history-load stall, 57.6% of the loop) is
   the whole story. The existence proof for hiding it is C's own SHORT
   decoder: `ZSTD_decompressSequences_body` decodes sequence N+1's symbols
   BEFORE executing N's copies -- a depth-1 interleave with no queue. D10
   refuted the QUEUE (PIPE=4/8, three arrays, cost grew with depth); its own
   comment says the prune constrains the approach, not the idea. Depth-1
   without structure -- 13.6's original shape -- has never been built.

2. **L3 has a ~4.6% RATIO gap too, and it points the other way.** C finds
   more (and evidently better) matches at DFast: more sequences AND smaller
   output. That is an encoder search-quality gap at L3, separate from speed,
   and per `codec-rate-allocation-vs-efficiency` it needs its own diagnosis
   before anyone tunes toward it.

Both numbers are deterministic counts; `cseqdensity` takes the level and a
directory of C frames, per the 11.7 provenance rule.

### 21.1 THE TAG FILTER IS NOT THE RATIO GAP -- and the counter that said it was, lied

The leading hypothesis for 21's gap was structural: our DFast runs a TAG
FILTER on both hash tables that C's `ZSTD_compressBlock_doubleFast` does not
have at all. A tag is a lossy hash; when it mismatches we skip the candidate
without loading it. That is free when the candidate would have failed anyway
and a LOST MATCH when it would not -- which would explain "C finds 4.5% more
sequences" exactly.

`examples/dfasttag.rs` (new) reads the audit counters that already existed.
The first read was alarming:

| | rejections | "FALSE" | share |
|---|---:|---:|---:|
| short table | 6,193,286 | 3,658,617 | **59.07%** |
| long table | 4,255,691 | 0 | 0.000% |

**59% would have been the whole gap. It was an instrument defect.** The LONG
audit verifies its claim -- on a rejection it reloads the candidate and runs
`match_ok` + `count_match` before counting a FALSE. The SHORT audit, despite
`TAG_FALSE_REJECT`'s name and its "rejections that `fast_probe` would have
ACCEPTED" doc, only tested `m.is_none()`: it counted EVERY rejection of a
non-empty slot and called it false. Given the same verification as its
sibling:

| | rejections | FALSE (verified) |
|---|---:|---:|
| short table | 6,193,286 | **0** |
| long table | 4,255,691 | **0** |

**Zero on both tables, over 64 MiB at L3.** The tag filter loses nothing; it
is a pure work reduction, and the ratio gap is elsewhere. Hypothesis refuted
on measurement, and the counter now checks what its name claims.

*The lesson is `rusty_curiosity`'s, in its most expensive form: a beautifully
clean number that turns out to measure something else. Two audits sat ten
lines apart in the same function, one verifying and one not, and the
non-verifying one was the alarming one. When two instruments of the same kind
disagree by 59 points, suspect the instruments before the codec.*

**Where the gap is NOT:** not the tag filter (here), not the decoder (13.1),
not the window (13.2), and the "next long" capability C has at `ip + 1` we
also have (`nl_on`, added for exactly that reason). What remains untested is
the step/acceleration schedule and the repcode handling -- and those want
their own section, not this one.

### 21.2 ELEVEN CUTS ON THE SHIFT FAMILY -- the same defect, eight more sites

Hunting the ratio gap read every hash entry in the encoder, and found the
defect 14.9/14.11/14.13 had already cut four times sitting in eight more
places. Every hash in this crate derives its shift as
`N - hash_log.min(32)`, both terms per-BLOCK constants; the sites below were
re-deriving it per POSITION, per CANDIDATE or per MATCH.

| # | site | frequency |
|---|---|---|
| W40 | `dfast_hash_pair`'s long hash | **per position at L3**, twice when the speculation arm is on |
| W41 | `fill_hash_long_after_match`'s two `hash8` | twice per match on the DFast ladder |
| W42 | the `next long` probe at `ip + 1` | per match on the nl path |
| W43 | the GATE-12 stride fill | per filled position -- a loop that had already hoisted its four table BASES |
| W44 | `find_greedy_impl`'s MAIN loop hash | per searched position at L5 (its FILL was fixed in 14.13; the loop was not) |
| W45 | `chain_find_best_inner`'s head hash | per searched position at L7-L12 -- `ChainCtx` had carried both shifts resolved since W20, and the row finder was the only consumer |
| W46 | the RUNTIME Bt entry (+ `hash4_shift`) | per searched position whenever the (hash_log, chain_log) pair is off the spec list |
| W47 | the RUNTIME Bt entry's `src.len()` | same line as W46 |
| W48 | `find_greedy_impl`'s walk-entry guard | `src_len` was hoisted at block scope; this guard re-read the field anyway |
| W49 | `chain_find_best_inner`'s two `src.len()` | per searched position |
| W50 | the SPEC Bt entry's `src.len()` | its shifts already fold (`HLOG` is a const generic); the field read was the last per-position cost on that line |

**Correctly NOT taken:** the four `src.len()` reads at the DFast candidate
level sit inside `#[cfg(feature = "profile")]` census blocks, and
`prime_tables`' four `hash8` calls are on a path E4 measured at **zero
iterations at every level L1-L22**. Cold is cold.

Gate: `bytegate` GOLD `BE0071FB0CB0CED9` unmoved -- and it covers all four
shipping finders these touch (DFast L3, Greedy L5, chain Lazy L7-L12, Bt
L13+). `rowboard` L9 identical, tag audit unchanged, `no_std` clean.

**The standing rule this earns, third time of asking:** 14.10 said grep every
WRITER when a structure turns out dead; 14.13 said check the other FILLS the
same day. This says the same thing about a computation: **when a block
constant is found being re-derived in a loop, grep the whole crate for that
expression before closing the finding.** Four sites were fixed today one at a
time before anyone ran the grep, and it returned eight more.


## 22. D11 BUILT AND REFUTED -- the third distance closes decode-ahead. 2026-08-24

Section 21 pointed at the depth-1 interleave as the decode gap's biggest
lever, so it was built: ONE pending sequence in registers, the next
sequence's symbols decoded and its history load prefetched before the pending
copies run -- 13.6's original shape, none of D10's queue, bit order identical
to the classic loop by construction (round-trip asserted on every one of the
probe's ~2,000 armed decodes).

**`d11probe` @ L3, 21 ABBA rounds, DecSeqLoop isolated, null median 2.16%:
OFF 30.27 -> ON 30.56 ns/seq = +0.9%, 110/273 pairs, z = -3.21.** Worst
corpora xml (z -3.71) and mozilla (-2.84); best ooffice (+1.96,
insignificant).

That is the THIRD distance, and together they refute the DIRECTION, not an
implementation (codec-measurement 11's three-probe rule, satisfied for real
this time):

| probe | distance | structure | verdict |
|---|---|---|---|
| D9 | 9.4 ns | none | +0.2%, z -0.91 -- null |
| **D11** | **~30 ns (one sequence)** | **registers only** | **+0.9%, z -3.21 -- worse** |
| D10 | ~120-240 ns | queue | +15.8/18.4%, z -13/-14 -- much worse |

**What this proves about the loop:** the out-of-order window already overlaps
adjacent sequences' history misses on its own -- hardware memory-level
parallelism is doing the decode-ahead, for free, and every software shape
that "helps" only adds retired instructions. The DecSeq loop is
instruction-THROUGHPUT bound, not latency bound. Section 21's C gap
(~18 ns/seq vs our ~30) is therefore FEWER INSTRUCTIONS PER SEQUENCE on C's
side -- exactly the axis sections 17-18 were already cutting -- and the
campaign's decode direction is: keep removing instructions from the loop, and
stop trying to hide a latency the hardware already hides.

Disposition: `set_pipe1_arm` kept, default OFF, reader folded to const
`false` in non-`profile` builds like its D9/D10 siblings -- shipping builds
carry zero cost and a future microarchitecture can re-adjudicate all three
with one call each. A negative at z = -3.21 with the idea CLOSED is worth
more than the +0.9% it cost to learn.


## 23. TEN CUTS ON COPY_MATCH -- the 60.8% row, 2026-08-26

Section 22 redirected the decode campaign to instruction reduction and the
dsloop ladder crowned `copy_match` king (60.8% of the loop). This round works
the copy chain: `copy_match_nodict` -> `copy_from_decoded_hot` -> tier 1,
plus the literal copy that shares its bookkeeping. Gate: `bytegate` GOLD
**BE0071FB0CB0CED9** unmoved, 173 tests release + debug -- the debug run
matters doubly here because cuts 3/4 replace runtime tests with invariants
the `debug_assert`s now carry, and the suite's external C-encoded frames
prove cut 2's budget rejects no conformant stream.

**The architecture: one reserve + one budget make capacity a BLOCK INVARIANT.**
Nothing reserved `out` before this round -- the per-sequence capacity tests in
the copy tiers were the only guard, and the only bound on a hostile block's
output was the frame-end content check (a real amplification hole: one block
could legally emit gigabytes before anything objected).

| # | cut | deterministic reduction |
|---|---|---|
| 1 | `out.reserve(block_max + 64)` once per block | the enabler; at most one allocation per frame in practice |
| 2 | running output BUDGET, one subtract-and-branch per sequence | enforces the RFC 128 KB block bound for the first time; rejects over-long hostile blocks at the first overrunning sequence |
| 3 | match tier-1 capacity test DELETED | -1 branch, -2 Vec field reads per sequence (invariant, asserted in debug) |
| 4 | literal tier-1 capacity test DELETED (`copy_literals_hot`) | -1 branch, -2 Vec field reads per sequence |
| 5 | `off == 0` / `off > produced` / `off > window` fused: `off-1 >= min(dst_at, win_lim)` | window reject's branch folds into cut 10's compare: -1 branch per sequence |
| 6 | per-sequence `len > block_max` DELETED (subsumed by cut 2's tighter bound) | -1 branch per sequence |
| 7 | `dst_at` threaded through the match chain | no `out.len()` re-read after the literal `set_len`, no `offset`/`src` re-derivation -- the whole chain runs on caller registers |
| 8 | `dst_at` threaded through the literal copy | same, literal side |
| 9 | trailing-literals budget check | one compare per BLOCK completes the invariant chain cuts 3/4 stand on |
| 10 | 8-byte sub-tier PRUNED by census (`bandcensus.rs`, new) | post-cuts band shares: tier-1 80.4% of calls, `off < 16` bands 2.03% -- a rung's whole ceiling is 2% of calls on the already-cold path. Recorded, not built |

**The per-sequence executed ledger (the admissible receipt):** branches
-4 +1 = **net -3 per sequence** (~160K sequences/MiB at L3), Vec field loads
-5 (two capacity pairs and one length re-read), one offset re-derivation
gone. Static size ~flat (crate 237,713 -> 237,798; the sequence-loop twins
moved +7/+25 while the reserve/budget prologue landed per block).

**Instrument caveat, stated so nobody misreads the ladder:** a dsloop re-run
two days after section 17's baseline shows every row -- including `advance`
and `read_bits`, which this round never touched -- inflated 15-30%. That is
15.2's lesson (un-interleaved runs on different days are not a comparison),
and it is why this round's receipts are counts. Within the new run the
shares hold their shape: `copy_match` 54.6%, the copies together 71.1%.

**What the census leaves as copy_match's remaining structure:** tier-1 takes
80.4% of calls but only 49.4% of BYTES -- the 64-byte tier carries 16.0% of
bytes at 4.6% of calls and `extend_from_within` 12.9% at 1.8%. The next
lever, if one is wanted, is byte-weighted: widen what the cold rungs move,
not how the hot rung tests.


## 24. TEN MORE ON COPY_MATCH -- the cold rungs, byte-weighted. 2026-08-26

Section 23 took the entry chain and tier 1; this round takes the rungs by the
census's byte weights, on section 23's invariant. Gate: `bytegate` GOLD
**BE0071FB0CB0CED9** unmoved, 173 tests release + debug, and the strongest
routing receipt available: **`bandcensus` before and after are IDENTICAL to
the call** (6,215,835 copies, every band unchanged) -- the cuts moved zero
copies between paths, exactly as claimed.

| # | cut | deterministic reduction |
|---|---|---|
| 1 | splat test repositioned by CALL weight | its compare moved from ahead of 19.6% of copies (cold entry) to ~2.0% (after the fixed tiers); band 0 is 0.01% |
| 2 | 32-tier capacity test folded under `G` | -1 branch, -2 Vec loads on 13.0% of copies |
| 3 | 64-tier capacity test folded under `G` | -1 branch, -2 Vec loads on 4.6% of copies |
| 4 | dead band-5 sub-census branch DELETED | census receipt: 0 calls since the 16-first reorder (tier 1 provably owns `len<=16, off>=32`); orphaned brick-82 comment removed with it |
| 5 | band-3 strided wildcopy under `G` (`off >= 32`) | **12.9% of all match BYTES** stop paying `extend_from_within`'s call + internal range checks; inline 32-byte strides, overshoot absorbed by the invariant's pad |
| 6 | band-3's hidden `reserve` eliminated | via cut 5; the invariant already owns the space |
| 7 | overlap loop: `avail` as a register RECURRENCE | per-iteration `out.len() - src` load+subtract gone (3.3 iters/call measured by D3), dead `avail == 0` reject deleted (`off >= 2` proven: splat took 1, entry proves >= 1) |
| 8 | splat under `G`: `resize` -> `write_bytes` | element-wise extend becomes one memset; source read unchecked under the entry invariant |
| 9 | `const G: bool` monomorph split | the MECHANISM: one source, two cold symbols (226 + 219 instrs) -- the budgeted path gets cuts 2/3/5/8, the dictionary path and oracle tests keep every runtime check, drift impossible |
| 10 | cold signature slimmed to `(out, dst_at, off, len, wide)` | `src` derived once inside; the hot callsite passes registers it already owns |

**Attribution note, because the tree is CO-AUTHORED this session:** a
concurrent workstream landed `decode_seq_header` (the header/table parse
outlined from the sequence-loop twins -- 933-instruction symbol, twins
1,611/1,571 -> **827/771**) and encoder-side cuts; crate total moved 237,798
-> **213,723 (-10.1%)** with MOST of that theirs. This section claims only
the copy-chain ledger above; the combined tree passes every gate. Same-file
concurrent editing is how clobbers happen (`editor-clobber-protection`) --
flagged to the user, not silently absorbed.


## 25. ROUND THREE ON COPY_MATCH -- the loop goes RAW-CURSOR. 2026-08-26

Rounds one and two took the checks; what remained was HOW the loop talks to
memory. The emitted body (not the source) named every target: `out` behind
two levels of indirection with `ptr`/`len` reloaded twice and `set_len`
stored twice per sequence, tier conditions as branch PAIRS, `budget`/`rem`/
`nodict` in stack slots, the literal cursor re-derived per sequence, and the
cold fallback re-inlining the very tier test that had just failed.

**The campaign's hardest-won lesson this round: register-allocation wins are
NOT wins.** `budget` moved into `%r15` on one build and back to a stack slot
on the next; a `&`-fused setcc branch pair held on one build and re-split on
the next. Every claim below is STRUCTURAL -- a load, store, instruction or
branch that no allocator decision can reintroduce -- verified by reading the
emitted body after each build.

| # | win | receipt (from the emitted body) |
|---|---|---|
| 1 | the loop runs on RAW CURSORS (`base`, `dst`); `set_len` publishes only at cold boundaries, taps and exit | BOTH per-sequence `set_len` stores gone, BOTH `len` field reloads gone, the double indirection gone; tier copies address `(%rcx,%r13)` directly |
| 2 | budget + literal-bound rejects: the SUBTRACTION is the test | both `cmp` instructions deleted -- `subq`+`jb` pairs, results reused as the updates |
| 3 | per-sequence `nodict` test DELETED from the always-path | dict blocks get `win_eff = 0`, so cut 5's guard routes them through its reject arm, where the real test lives; plain frames pay zero |
| 4 | literal tier: ONE branch, arithmetically forced | `orq` + `js` (the setcc form re-split on a later build; the wrapping-difference OR cannot) |
| 5 | match tier: ONE branch, same form | second `js`; was `cmp/ja` + `cmp/jbe` |
| 6 | literal source as (pointer, remaining) recurrences | the copy is DIRECT-addressed `movups (%rdx)` -- the per-sequence index arithmetic is gone; the slots the allocator spills them to are its own affair |
| 7 | `lits_len` hoisted, one derivation per block | feeds wins 2/4/6 |
| 8 | match-cold: `wide` as `const W`, argument deleted | the outlined symbols test nothing the caller already folded |
| 9 | literal-cold: `arm` as `const A`, argument deleted | single live 96-instruction monomorph in shipping |
| 10 | the literal fallback goes STRAIGHT to cold | the re-inlined tier double-test is gone: symbol `movups` count 6 -> 4 |

Static size: twins 827/771 -> 906/813 -- the growth is publish/refetch sync
at the RARE cold boundaries while the 80%-path sheds memory traffic, the
exact 0b screen-inversion this document first recorded for D1/D2: the
receipt is the loop body, and it was read after every build. Gate: GOLD
unmoved, band census identical, 173 tests release + debug, `no_std` clean.
Still stack-resident and honestly not claimed: `rem`'s countdown and
whatever the allocator spills next -- registers on x86-64 are a budget this
loop now spends to the last byte.

**CORRECTION (2026-08-26, found while building section 26):** section 25's
win 2 as WRITTEN never landed -- the patch's anchor mismatched and Python's
`replace` no-ops silently, so the source kept the two separate reject `if`s.
The emitted `subq`+`jb` receipt quoted above was LLVM's own flag reuse, not
the source change. The intended form landed for real in section 26 (its win
8), and the lesson is recorded: **assert every patch anchor, and never let a
receipt's story outrun the source diff.**


## 26. THE JOINT-SEQUENCE MEGAFUSE -- twenty wins, one branch. 2026-08-26

`jointrate.rs` (new) priced it first: the literal tier fires on **99.22%** of
sequences and the match tier on **80.40%**, so the joint fast path is
**[79.62%, 80.40%]** of every sequence decoded. This round collapses that
path to ONE predicate branch and then hunts the small tweaks hiding around
it. Receipts are the emitted body after every build; gates: GOLD unmoved,
band census identical to the call, `jointrate` unchanged, 173 tests release
+ debug, `no_std` clean.

**The fused fast path, in full:** seven borrow-free differences OR'd into one
sign test, then nine instructions -- two `movups` pairs, three cursor
updates -- and back around. Zero Vec-field operations, zero `set_len`, zero
variable shifts, zero cmovs, one branch.

| # | win | receipt |
|---|---|---|
| 1 | the MEGAFUSE: budget reject + lit tier + match guard + match tier = **5 branches -> 1** on ~80% of sequences | single `js` in the body |
| 2 | input-bound term DROPPED on the fast path | `n <= 16 <= lit_rem` makes it redundant; slow arm keeps it |
| 3 | `off >= 1` wrap-guard DROPPED on the fast path | implied by the `off >= 16` term |
| 4 | the guard's `min`/cmov DECOMPOSED into two OR terms | no cmov in the fast trace |
| 5 | `b_ok` dual-use: predicate term IS the budget update | one `sub`, stored once |
| 6 | `dst - off` dual-use: predicate term becomes the match SOURCE | `subq %r15` off the copy address, no fresh derivation |
| 7 | ONE cursor advance for both copies (`op += need`) | `addq %r9, %r12`; two adds before |
| 8 | slow-arm rejects fused 2 -> 1 branch | section 25's win-2 intent, now actually in source |
| 9 | `OF_PACK` LUT for `1 << of_code` | variable `shll %cl` (3 uops + 2 setup movs) -> one load; **zero `%cl` shifts remain in the symbol** |
| 10 | the cursor is `op` -- C's form -- stores address it DIRECTLY | `movups (%r13)` / `(%r12,%r8)`: the per-copy `base + dst` adds are gone |
| 11 | `fast_arms` conjunction hoisted per block | profile builds drop an AND per sequence; shipping folds it |
| 12 | `lit_rem` DELETED as a loop variable | its per-sequence subtract + spill store gone; tier test compares against hoisted `lit_guard` |
| 13 | lit-fallback reuses the predicate's `lit_off` | no `lits_len - lit_rem` recomputation at the cold boundary |
| 14 | tail + anatomy-tap `lit_pos` derivations are one pointer subtract | per block |
| 15 | fast path touches ZERO Vec fields end-to-end | body: only loop-invariant spill slots and the two buffers |
| 16 | `need` dual-use: one `lea` feeds `b_ok` AND the advance | `leaq (%r8,%rbx)` used twice |
| 17 | `off`/`mlen` computed once ABOVE the fuse, shared by both arms | the slow arm's re-lets deleted |
| 18 | `lit_guard` hoist: the literal bound is load+sub, not lea+cmp chains | `-40(%rbp)` term in the predicate |
| 19 | census fidelity under restructure: the fused path feeds BOTH tier counters and band 2 | `bandcensus`/`jointrate` byte-identical across the change |
| 20 | the section-25 silent no-op FOUND and corrected | the hunt's meta-win: source and receipt now agree everywhere |

The aliasing proof the fused copies rest on, stated once: the match write is
`[op+n, op+n+16)`, its read `[op+n-off, op+n-off+16)`; `off >= 16` puts the
read wholly at or before `op+n` -- disjoint from the write -- and reads of
bytes the literal half just wrote are CORRECT (matches may reference the
literals of their own sequence) and well-defined in program order. This is
the same argument the split tiers relied on; the fuse only states it in one
place.

Twins: 906/813 -> **898/844** -- baseline SHRANK while carrying the fuse;
avx2's growth is its slow-arm scaffolding. The fast trace is the receipt.


## 27. THE RAW COLD PROTOCOL -- the byte-weighted middle rungs. 2026-08-26

The cold bands move HALF the match bytes (t32 20.9%, t64 16.0%, within
12.9%, overlap 0.9% -- against tier 1's 49.4%), and every cold call paid the
full Vec boundary: publish (`set_len`), a `&mut Vec` argument, callee derefs,
refetch. Under the block invariant the fixed-width rungs CANNOT grow `out` --
so the boundary was pure freight. `match_cold_raw` / `lit_cold_raw` (164 and
32 instructions) take `op` in and hand a new `op` back: no Vec anywhere.
`None` marks the genuinely Vec-needing leftovers -- the overlap band at
0.24% of copies and the `off < 32` within-cases -- which keep the old
byte-exact boundary.

| # | win | receipt |
|---|---|---|
| 1-2 | publish AND refetch deleted per raw cold call | ~5 memory ops off every t32/t64/within/splat call -- **~18% of ALL copies** |
| 3 | `&mut Vec` arg -> three scalars | callee has zero derefs |
| 4 | return-`op` protocol | no `out.len()` reload after cold; one register move |
| 5 | every `(G \|\| capacity)` test GONE inside raw | the invariant is baked in; zero capacity code |
| 6-7 | t32 / t64 rungs: ONE or-sign branch each | was two compares + a capacity test |
| 8 | within rungs carry NO length test | `len > 32` / `len > 64` implied by rung order |
| 9 | within strides 64 bytes when `off >= 64` | iterations HALVED on the 12.9%-of-bytes band (old path strode 32 always) |
| 10 | splat: `*op.sub(1)` + `write_bytes` | no indexed Vec read, no `resize` machinery |
| 11 | rungs call-ordered by census | t32 (12.99%) tested first |
| 12 | `wide` gating folds at the callsite | shipping calls raw unconditionally |
| 13 | `None`-isolation | only the truly-growing 0.24% keeps the boundary; fallback byte-exact |
| 14-15 | literal t32/t64: boundary deleted | same protocol, lit side |
| 16 | lit rungs: one or-sign branch each | was arm + two bounds + capacity |
| 17 | `rem` passed in a register | callee re-derives nothing; no `&mut lit_pos` |
| 18 | return-(lit_p, op) pair | both cursors advance from returned registers |
| 19 | the strided rung GENERALISES to overlapping copies | the stride proof (each read ends at or before the write cursor) needs `off >= W`, not `off >= len` -- so 225 former OVERLAP-band calls (off in [32, len)) now stride instead of paying ~3.3 doubling `extend_from_within` calls each. The census records the migration: band 4 14,987 -> 14,762, band 3 +225, totals identical, GOLD unmoved |
| 20 | the attribute-hijack caught mid-patch | inserting between an item's attributes and its `fn` re-parents them; the lint caught it, the fix is recorded |

Gate: GOLD unmoved; band census totals identical with exactly the win-19
migration (225 calls, 0.004%) and every counter preserved; streaming
round-trip clean; 173 tests release + debug; `no_std` clean. Twins 898/844
-> 954/862: the raw call plumbing lands in the slow arm (static, rare) while
every raw cold call sheds ~10 boundary operations (executed, ~19% of
copies).



## 28. THE ISA-TWIN AUDIT -- a portability bug, two hijacks, and the lever's floor. 2026-08-27

Nine suspected problem points, worked structurally. Four were real defects, one
of those a live portability bug; three were correctly-retired twins whose
arithmetic I re-derived and confirmed; one hypothesis died on its own census.
Every number here is a count from the emitted asm or from a census, never a clock.

### 28.1 The portability bug: an attribute hijack put AVX2 behind a BMI2 guard

Retiring the `decode_4x` AVX2 arm deleted its `fn` line and body but left its
attribute block sitting above the next doc comment. The attributes re-parented
onto `decode_4x_bmi2`:

    #[target_feature(enable = "avx2,bmi2,lzcnt")]   <- orphan, from the dead arm
    #[allow(unsafe_code)]
                                                   <- blank line
    /// The BMI2-compiled twin of `decode_4x_inner`.
    #[target_feature(enable = "bmi2,lzcnt")]
    unsafe fn decode_4x_bmi2(...)

The shipping BMI2 twin was therefore compiled with AVX2 ENABLED while dispatched
on `has_bmi2()` alone -- VEX permitted on a Skylake Pentium/Celeron, which ships
BMI2 with AVX2 fused off. That is verbatim the hazard the retirement note two
screens above says "goes with" the removed arm. It did not go; it moved.

Nothing could see it. The suite cannot: a twin is byte-identical to its sibling
on any host that runs it. The census read `vex=0`, but that is LLVM declining
the opportunity, not lacking it. `decode_sequences_avx2` had already been
converted to BMI2-only for exactly this reason, with a comment naming the same
parts -- so the hazard was understood here and reintroduced anyway.

    decode_4x_bmi2   702 instr -> 681 instr   (-21, from dropping the dead feature)

### 28.2 The weights twin sat on the cold path; the hot path had none

`decompress_weights` carried a BMI2 twin whose note claimed "per new-table block
on the literal decode path". W39 had moved the decode path off it. Its two
remaining callers are `read_ctable` (once per dictionary) and a test. And the
twin was not working regardless: `decompress_weights_inner` is
`#[inline(never)]`, so the twin compiled to a single `jmp` into a baseline
symbol -- the fourth thunk twin found in this crate.

Meanwhile `decompress_weights_into` -> `weights_into_inner`, which IS the
per-block path (`huffman::read_table`), had no twin at all. Retired the thunk;
built a real twin on the live path using the body/arm split:

    weights_into_inner (baseline)   444 instr   12 cl    0 bmi2
    weights_into_bmi2  (twin)       431 instr    0 cl   13 bmi2

### 28.3 The second hijack, and why the compiler only half-sees this class

`encode::lz_insert_rowknown` had `row_insert_live` inserted between its doc
block and its `fn` line. The doc block re-parented (documenting a function that
has its own doc), the `#[inline(always)]` became a duplicate on the wrong
function, and `lz_insert_rowknown` -- the row finder's per-position insert --
lost its inline hint.

HONEST OUTCOME: no performance change. LLVM inlined it anyway on size and no
symbol appears in the asm. The mis-attribution was real, the cost was zero, and
claiming a win here would be inventing one.

A hijack lands in one of two shapes and NEITHER GATE SEES BOTH:

  (a) the stolen attribute is one the victim did not carry (28.1) -- rustc is
      silent; only a source scan finds it
  (b) it duplicates one the victim already has (this section) -- rustc reports
      `unused_attributes`, and it names the innocent line

Both gates are now wired: `#![deny(unused_attributes)]` in `lib.rs` for (b), and
`scripts/twinguard.py` for (a), in CI. The scanner is verified in BOTH
directions against reconstructed copies of both real defects, and it runs on
COMMENT-STRIPPED source -- this crate writes `#[inline(always)]` and
`#[target_feature]` in prose constantly, and matching raw lines gave about 25
false positives against 2 real findings. A check nobody can read the output of
is a check nobody runs.

### 28.4 The guard now tests what the twins actually enable

Eight twins enabled `bmi2,lzcnt` while `has_bmi2()` tested only `bmi2` -- the
same shape as 28.1, reached by omission rather than by a stray attribute. It
held only by an argument about hardware history (every part with BMI2 has
LZCNT), and `twinguard` needed a hand-written exemption to stay quiet about it.

`has_bmi2()` now tests both: one extra CPUID bit, once per process, rejecting no
part that exists. The exemption table is empty, so the invariant is checked
instead of asserted. `twinguard` reads each `has_*()` predicate's feature set out
of the DETECTION SOURCE, so deleting the `lzcnt` test re-trips every dependent
twin immediately -- verified by doing it.

`encode_stream_unrolled_bmi2_into` -- the only twin still enabling `bmi2` alone,
and the one that ships -- was brought in line. HONEST OUTCOME: zero instruction
change (1830 -> 1830). Correct as a consistency fix; not a win.

### 28.5 The encoder win: a dispatcher carrying its own dead arm

`encode_stream_into` is the route `write_literals` takes, four times per
4-stream block. It measured 457 instructions -- for a function whose job is to
pick an arm. `encode_stream_scalar_into`, the `RZSTD_HUFF_FAST=0`
re-adjudication arm, carried no inline attribute, so LLVM stamped its whole body
into the dispatcher. `huff_fast_enabled()` defaults TRUE, so on every shipping
run that body is not executed once.

    encode_stream_into        457 instr -> 134 instr   (-323, -71%)
    encode_stream_scalar_into  (inlined) -> 337 instr, outlined, #[cold]

Same finding as `decode_4x_x2_slow`, whose census read 0 of 509 sections: a
fallback that stays correct and reachable does not have to stay resident.

### 28.6 THE LEVER HAS A FLOOR, and the encoder is already sitting on it

The screen is instructions-of-duplicated-body per converted shift op. Below
about 40 a twin tends to pay; above it the duplicated body costs more I-cache
than the shifts are worth. Measured across every remaining baseline `%cl`
carrier in the crate:

    parse_ncount_into      747 / 56 =  13.3   best ratio in the crate
    decode_into_x2         351 / 17 =  20.6
    decode_4x_x1           532 / 20 =  26.6   <- TWINNED, see 28.6a
    decode_4x_x2_slow      577 / 20 =  28.9
    prime_tables          1115 / 17 =  65.6
    find_dfast            1409 / 20 =  70.5   <- D6 retired it, measuring 72
    encode_block          4223 / 47 =  89.9   <- retired, measuring 1 shrx / 1291
    find_lazy             1616 / 14 = 115.4
    find_greedy           1386 / 12 = 115.5
    ncount_or_default      868 /  6 = 144.7
    select_seq_table      2208 / 12 = 184.0

D6 and the `encode_block` retirement are CONFIRMED by independent
re-derivation (70.5 against their 72). Their arithmetic was right.

One correction to the record: D6's stated REASON -- "`shr %cl` and `shrx` are
both one uop on any CPU that HAS BMI2" -- is wrong, and contradicts this crate's
own fast-trans section 3. A CL-count shift is 3 uops on Intel (the flags-merge
quirk); `shrx` is 1. The retirement stands on its I-cache arithmetic, not on
that claim, and the claim must not be reused as precedent.

RATIO IS NOT SUFFICIENT -- frequency gates it as well, and three candidates with
good ratios were declined on exactly that:

  * `parse_ncount_into` has the best ratio in the crate and was DECLINED: it
    runs at most three times per block (ll/of/ml).
  * `decode_into_x2` was DECLINED: its own comment establishes that it decodes
    only the TAIL. `fast_4x2` handles the bulk and is already
    `#[inline(always)]` into the twins, so the bulk ALREADY runs under BMI2.
  * `decode_4x_x2_slow` was DECLINED: `#[cold]`, census 0 of 509 sections.

`decode_4x_x1` was the one that PASSED both screens -- see 28.6a, which is also
where a stale census turned up.

CONCLUSION, STATED PLAINLY: every un-twinned shift carrier on the ENCODE side is
at 65 instructions per converted op or worse. The ISA-twin lever is exhausted on
the encoder. Further encoder gains have to come from search cost or from
structure of the 28.5 kind -- not from another twin.

### 28.6a `decode_4x_x1`: a stale census, and an instrument that could not be read

`decode_4x_x1` runs at BASELINE ISA beneath `decode_4x_bmi2`, which calls it --
a `#[target_feature]` function cannot be inlined into a baseline caller, and the
converse is what bites here: an outlined baseline helper stays baseline no
matter who calls it. 532 instructions to convert 20 shifts is 26.6 per op,
which clears the screen. The question was frequency, and the answer was already
written down: "509 of 517 sections (98.45%) to the X2 arm and 8 (1.55%) here",
which is why the arm had been outlined out of the twins.

THAT NUMBER IS STALE BY ~16x. Re-measured with `x1arm.rs` over twelve Silesia
corpora at L1/L3/L9/L19 -- deliberately the same corpus family the original
used, because comparing against a different population is not a measurement:

    cap  1 MiB    x1=110   x2=292   x1_share 27.36%
    cap 16 MiB    x1=1004  x2=3044  x1_share 24.80%

One section in FOUR, not one in sixty-five. Stable across a 16x change in input
size, and rising steeply with level (6.97% at L1 to 38.72% at L19). The six
SYNTHETIC corpora contribute 0.00% -- so this is not a stress-corpus artefact;
it is what real data does.

AND THE INSTRUMENT COULD NOT HAVE SAID OTHERWISE. That share was read as
`X4_X1_CALLS / X2_STATS[1]`, and `X2_STATS[1]` is incremented at TWO sites: the
4-stream arm in `decode_4x_inner` AND the 1-stream path in `decode_stream`. Its
denominator therefore contains sections its numerator can never count, so the
ratio understates the X1 share by however many 1-stream X2 sections a run
happens to contain. A shared counter cannot express a two-way split, in either
direction. `X4_X2_CALLS` now counts the sibling arm at exactly one site, and
`take_x4_arms()` returns the pair -- so the ratio is readable rather than
merely printable.

The outlining stays (that trade was about I-cache and is unaffected); what
changes is that the arm no longer runs at baseline ISA beneath a BMI2 caller.
`decode_4x_inner` gained a `const BMI2: bool` so the arm selection folds at
compile time -- Law 1 means the only way into an outlined helper is a different
SYMBOL, and the choice has to be made where the ISA is already known.

    decode_4x_x1      (baseline)   532 instr   20 cl    0 bmi2
    decode_4x_x1_bmi2 (twin)       482 instr    0 cl   24 bmi2

-50 instructions and all 20 variable shifts converted, on ~25% of 4-stream
literal sections. RESIDUAL, STATED: `decode_into_x1` (105 instr, 3 cl) is still
outlined at baseline beneath both arms. Three shifts did not justify a third
copy; it is recorded so the next reader does not have to re-derive it.

### 28.7 Refuted: the literal double-encode

`write_literals_inner` builds two candidate sections and fully Huffman-ENCODES
each before comparing, while `body_bytes_exact` already computes each
candidate's exact body size WITHOUT encoding it. That looked like a free halving
of literal emit cost. `littry.rs` (new) says otherwise:

    L     blocks    encodes   prev_ENC   new_ENC   skipped   WASTED
    1        123        137         14       123        53       14 (10.2%)
    3        115        127         12       115        42       12 ( 9.4%)
    9        112        127         15       112        47       16 (12.6%)
   19        120        145         25       120        45       23 (15.9%)

1.11 encodes per block at L1, against a floor of 1.00. The `futile` predicate --
which already calls `body_bytes_exact` for both tables -- skips 53 of them.
There is no halving available; the path sits at 0.11 wasted encodes per block
and the hypothesis is dead.

### 28.8 Dead, and left dead deliberately

`row_insert_live` has zero call sites: the fill loop calls `rows.insert_h`,
which is strictly better (hoisted mask, no `row_of` re-derivation, no emptiness
test). W28 was SUPERSEDED, not reverted -- worth stating, because a dead
optimisation helper looks identical to a reverted one. `block_avx2_on` is dead
from the same AVX2 retirement that caused 28.1. Both are dead CODE, not dead
paths: LLVM drops them and they cost nothing at runtime.

### 28.9 Tooling repaired along the way

`scripts/isaudit.sh` demangled by stripping ALL digits to remove Rust's length
prefixes -- which also ate `4x`, `x1`, `bmi2` and `avx2`, rendering
`decode_4x_x1` and `decode_4x_x2_slow` indistinguishable and making a filter for
them match NOTHING. It now parses length prefixes properly, and handles both
legacy `_ZN` and v0 `_R` mangling, because this tree emits both depending on how
the build was invoked and an audit that silently returns zero rows for the wrong
one reads exactly like "clean".

### 28.10 Gates

    bytegate  BE0071FB0CB0CED9   unchanged (59,760,356 bytes, 18 corpora x 9 levels)
    xxhgold   10751459475B6849   unchanged
    tests     173 passed, 0 failed
    twinguard 37 files clean, verified against both reconstructed defects
    cfg arms  default / profile / no-default+alloc -- all clean under the new deny
    clippy    clean, --all-targets
    harness   `cargo check -p rusty_zstd-bench --all-targets --features profile`
              was FAILING before this section: `d9cover` reads `take_pf_census`,
              which is gated on `pfcensus`, not `profile`. Given a
              `required-features` declaration so the harness job stops building
              it under the wrong feature set.
