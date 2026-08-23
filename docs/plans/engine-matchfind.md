# Engine anatomy — MatchFind's functions, ranked by what optimizing them could buy

**Date:** 2026-08-21 (updated post BMI2 twin campaign, `08d14e3`..`bfd0cf0`; original
2026-08-20). Companion to [`m7-optimize-anatomy.md`](m7-optimize-anatomy.md)
(what the campaign already did) and `gg-matchfind.md` (the gates -- an internal
document, see the note in [`README.md`](README.md)).

> **BASELINE SHIFT (2026-08-21).** Every finder in this file now runs a
> `#[target_feature(enable = "bmi2,lzcnt")]` twin on modern hardware; the transitive
> trap trace is empty and the CL-shift ledger is fully classified. The
> cheaper-instructions well is DRY: every remaining win below is either LESS WORK
> (fewer probes, fewer positions, fewer wasted loads) or MEMORY BEHAVIOUR (prefetch,
> one-array-not-two). Section 0b is the ranked shortlist with locations.

**Why this document exists.** The gates over match-find are set — this week gated
every major decision in it. But gates choose *which* path runs; the 2.36× against C
lives *inside* the paths. Three facts, all measured, say so: the multiplier is nearly
flat across 14 unrelated corpora (1.93→2.95); we beat C on exactly the four corpora
where the search loop does not run; and `probes/B` is **below 1.0 on 9 of 12** — we do
not issue too many probes, **each probe costs too much and too many are wasted**.
Match-find is **75.3% of encode at L3** and #1 on 18 of 18 corpora. This is the
engine-function inventory for it.

> **INSTRUMENT RULE.** The per-frame clock has a **±24.15%** null-arm floor here.
> Nothing below may be decided by stopwatch. Admissible: instructions from emitted asm,
> deterministic work counters (`probes/B`, `hit%`, `take_tag_rejects`), byte-identity
> boards. Every "candidate lever" below names its deciding instrument.

## Priority order

| #   | item| ladder| confidence| deciding instrument|
ALMOST DONE | ~~1~~ | **1a** long-table tag — **DONE 2026-08-21**: the long tag IS the short tag (zero new arithmetic), packed under the same < 16 MiB proof. `ltag` census: **10,955,562 avoided of 15,175,449 long probes (72.2%)**, 36/36 byte-identical, FALSE asserted 0 per cell. x-ray 99.6%, sao 91.8%, ooffice 91.2%, mozilla 85.4%; controls (zeros/incomp/versions) ~0 probes. | L3/L4 default | shipped | `ltag` |
DONE | 2   | **4** first-word early-exit      | all, worst L16–L22 | high — histogram  | `EQ_LEN_HIST` |
| 3   | **3a** chain prefetch (address known ahead)   | L5–L12 | medium-high | 80-cell board + parity |
| 4   | **2b** hoist Bt loop invariants  | L13–L22            | high, small | asm loop-body count |
| 5   | **2a** Bt child prefetch (dependent chase)  | L13–L22 | medium — | rdtsc micro-harness |
ALMOST DONE | 6   | **6** `find_sequences_strategy` chain alloc + 17 sites | all| high, | `g6alloc` |
| 7   | **1b** telemetry register pressure  | L3/L4| blocked on gate campaign| stack movs in hot region|
| 8   | **2c/5a** dead-copy censuses| L1, L13+| census first| per-copy call counters|

**The standing pattern check applies here first:** items 1a and 3b are both "the
capability exists in one path and not its neighbour" (tag: short table but not long;
Fast but not Lazy). That shape is 6-for-6 this campaign — prefetch built with zero
callers being the sixth. Check the neighbour before inventing anything new.


## 0. Current asm footprint (2026-08-21, release, per symbol family)

Post twin campaign: "copies" counts plain + BMI2-twin monomorphisations. Full table
with ISA receipts lives in m7-anatomy's MatchFind Function Anatomy; the shape here:

| function | copies | live-copy instrs | runs |
| --- | ---: | ---: | --- |
| `find_fast_impl` | 140 + 140 | <= 2,458 | per position, L1/L2 |
| `find_dfast` (+impl twins) | 1 + 6 + 6 | 9,397 dispatcher / <= 1,908 twin | per position, **L3/L4 (default)** |
| `find_greedy` | 1 + 1 | 2,269 | per position, L5 (walk inlined) |
| `find_lazy` | 1 + 1 | 1,958 | per position, L6-L12 |
| `chain_find_best` | 2 + 2 | <= 466 | per position + look-ahead (lazy path), `ChainFn` ptr per block |
| `bt_find_best_impl` | 42 + 42 | <= 229 | per position x ~30M, L13-L22, ISA in `bt_resolve` |
| `find_sequences_strategy` | 1 + 1 | 2,377 | per block (carries `find_opt` + `find_bt_lazy` inlined, both arms) |
| `count_match` / `count_eq_len` | 1 / 1 | 153 / 77 | per candidate hit |
| `match_ok` cold tail | 1 | 66 | cold (mls > 8) |
| `emit_fast_seq` / `prime_tables` / `fill_fast_after_match` | -- | fully inlined | per match / per block |

**Time by level** (mfanat, 18-corpus 8 MiB board, ms per input MiB): L1 **6.0**
(73.9% of encode) -> L3 **7.0** (76.5%) -> L5 **11.0** (83.4%) -> L7 **44.1** (95.1%)
-> L9 **74.8** -> L12 **174.3** (98.6%) -> L13 **273.0** -> L19 **379.4** -> L22
**406.7** (99.3%). MatchFind IS the encoder; the ladder spans 68x.

## 0b. TARGETS 2026-08-21 -- the remaining deterministic / instruction-reducing wins

Ranked by (confidence x traffic). Locations are current as of `bfd0cf0` + the
find_opt pin. Everything CL-shaped is DONE; these are work and memory levers.

| rank | target | location | lever | instrument |
| ---: | --- | --- | --- | --- |
| 1 | **Chain-walk prefetch** (old §3) | `chain_find_best_inner`, encode.rs:8516; greedy's inlined walk, `find_greedy_impl` encode.rs:8155 | `chain[next]` is known BEFORE `count_match` runs on the current candidate -- a NON-dependent prefetch, the easiest in the file. `simd::prefetch_read` (simd.rs:24) still has no callers | instr parity (+1/step) + 80-cell L5-L12 board; serves the 44-174 ms/MiB band |
| 2 | **Bt tree prefetch** (old §2a) | `bt_find_best_impl_inner` walk, encode.rs:9473 | prefetch the child row when `bt_idx` is computed -- both children share a line. C's `ZSTD_insertBt1` does this | instr parity + 60-cell Bt board; serves the 273-407 ms/MiB band |
| 3 | **Bt loop-invariant hoist** (old §2b) | the flagged comment, encode.rs:9518 and :9767 (both copies) | hoist the saturating_sub + max + field load to locals before the walk | asm instr count of loop body; byte-identity by construction |
| 4 | **`count_eq_len` first-word early-exit** (old §4) | simd.rs:177 | make XOR+TZCNT on word 0 the unconditional head of the AVX2 arm (C does); most calls die inside 8 bytes | `EQ_LEN_HIST`; byte-identical by definition |
| 5 | **Dead-copy census of the twin families** (old §2c/§5c) | `FF_ARM`, `BT_SPEC_CALLS`, `DFAST_SPEC_CALLS` counters | 140+140 fast and 42+42 bt copies: which serve 0%? GATE 12 found one dead bt spec before; the families have since doubled. Cull = binary size only (measurement-parity arms STAY) | per-copy call census under profile |
| 6 | **DFast register pressure is gate telemetry** (old §1b) | `find_dfast_impl_inner`, encode.rs:7221 | ~9 per-block gate accumulators live across the hot loop; derive post-loop from `seqs` or gate under `profile` | stack movs in the hot-loop region |
| 7 | **DFast speculation waste** (old §1c) | dpipe in `find_dfast_impl_inner` | 14.5% of speculated loads discarded at the shipped threshold; make the speculation cheaper (it recomputes two hashes + probes both tables) | `take_dfast_spec` |
| 8 | **Brick 48 A/B under the twins** | `chain_find_best`, encode.rs:8434 region | the outlining decision predates the twins and the `ChainFn` pointer; one experiment: point `cfb` at an `#[inline(always)]` variant in a scratch build and read the L5-L12 identity board + instr counts | asm + 80-cell board; owner decides |
| 9 | **`dfast_fill_ends` per-match atomic** (old §5d) | encode.rs:5678 | one atomic load per MATCH for a process constant -- brick-79 shape | asm; trivial |
| 10 | **Strategy-hub panic residue** (old §6) | `find_sequences_strategy` | the 17-site count predates the twin refactor -- RE-CENSUS first, then the huffman-style safe restructures | panic census |

**Shipped since the 08-20 original:** §1a long-table rejection tag is IN and default
ON (`LONG_TAG_ARM` encode.rs:13829, `get_hl_tag` encode.rs:1073, LTAG_* ledger);
§5d's `accel` variable-shift item is ABSORBED by the twins (shrx in every live copy;
const-specialisation would now buy ~0).

**Do not chase (the irreducible ledger):** the dfast twins' two `shrb $const, %cl`
set-membership tests (no 8-bit shrx exists); the encode twins' one memory-destination
shift each (no memory-operand shrx); every plain fallback arm (dead code on BMI2
hardware, kept for museum correctness and measurement parity).

**Output-changing, adjudication-class (not in this file's lane):** the mls-wide hash
residue on `versions` (cross-block state divergence, §5d4 -- family floor reached);
DFast's missing back-extension; probe-density and depth gates (the gate campaign
owns those).

---

## 1. `find_dfast_impl` — the shipping default's engine, and the top target

**Why first:** L3 is the default, match-find is 75.3% of its encode, and this function
IS that stage. `hit%` runs 3–27%: on `sao`/`ooffice`/`x-ray`, **95%+ of probes find
nothing**, and a missing probe still pays the hash, the candidate load, and the compare.

### 1a. The long table has no rejection tag *(the T1 follow-on — highest confidence)*
The short table now carries the packed tag (T1: 29.8% of non-empty slots rejected
without loading the candidate, byte-identical). **`hash_long`/`get_hl` has nothing** —
every 8-byte-hash probe that misses still takes the random-access candidate load. DFast
probes BOTH tables per position, and the long probe is tested FIRST, so on
miss-dominated corpora the untagged table eats the cache misses the tagged one was
built to avoid. The `next_long` probe at `ip+1` is a third untagged long access.
*Lever:* pack a tag into the long slot exactly as `put_h_tag` does (24-bit position +
8-bit tag; same ≥16 MiB fallback). *Instrument:* a `take_tag_rejects` twin for the long
table; byte-identity 72 cells. *Risk:* low — same proof shape as T1, `min_match`=5 ≥ 4.

### 1b. Register pressure is gate telemetry *(known, needs a gate-cell decision)*
Hot loop = **151 instructions carrying 27 reloads, 23 loop-invariant across 12
slots**. The live values crowding the registers are ~9 per-block accumulators
(`rep_hits`, `spec_made/used`, `mm_total`, `nl_probes/hits`, `band_hits/worse`,
`d_rep_bytes`) that exist to feed gate signals, not to compress. Splitting the tail
does NOT help (they are incremented in-loop). *Lever:* derive signals after the loop
from `seqs` where possible; gate the rest under `profile` as `MM_TOTAL` now is.
*Instrument:* stack movs in the hot-loop region of the asm. *Owner:* the gate campaign
— changing how signals are gathered changes gate inputs.

### 1c. Speculation waste
`dpipe` issues next-position loads that are discarded on every match/rep hit: **14.5%
of speculated loads wasted at the shipped threshold** (8.03M made → 6.54M used at
0.25; the sweep runs 19.4% → 16.6% across thresholds). The gate exists
(`dfast_spec_yield`); the engine question is making the speculation itself cheaper —
it recomputes two hashes and probes both tables. *Instrument:* `take_dfast_spec`.

## 2. `bt_find_best_impl` — the Bt tree walk, L13–L22's whole cost

21 monomorphisations × 337 instrs; the code's own comment: runs **~30M times per level
per corpus set**. Bounds checks are gone (132→0); what remains is the walk itself.

### 2a. The pointer chase has no prefetch — and `prefetch_read` HAS NO CALLERS
Each tree step loads `chain[(m & mask) << 1]` — a dependent random access; the next
node address is unknown until the current load retires. This is the textbook
memory-latency-bound loop. **`simd::prefetch_read` exists and is called from
NOWHERE** — the sixth instance this campaign of a capability built and never wired in.
C's `ZSTD_insertBt1` prefetches the next candidate row. *Lever:* prefetch the child
row as soon as `bt_idx` is computed, before the match comparison that decides
direction — both children share a cache line (`(m&mask)<<1` and `+1`). *Instrument:*
this one is genuinely hard to decide without a cycle counter; instructions won't move.
Propose: instruction-count parity check (must be ~+1/node) + the 60-cell Bt
byte-identity board, and accept it on C-precedent + parity, or build an rdtsc
micro-harness on the walk alone.

### 2b. Loop-invariant recomputation *(already flagged in-code, unresolved)*
The comment at the walk head: *"Loop-INVARIANT, recomputed on every node of every
walk: a saturating_sub, a max and a field load through `&mut MatchTables`."* ~3
instructions × 30M walks × avg depth. *Lever:* hoist to locals before the loop.
*Instrument:* asm instruction count of the loop body.

### 2c. 21 copies is a census question, not (yet) a speed one
GATE 12 already found one Bt specialisation serving **0% of calls**. Which of the 21
serve traffic? *Instrument:* `BT_SPEC_CALLS` per-copy census under `profile`. Dead
copies cost binary size only — do not price as speed (m7-optimize-anatomy §5 rule) —
but a dead-copy cull shrinks the I-cache-relevant dispatch too.

## 3. `find_greedy` / `find_lazy` / `chain_find_best` — the chain walk, L5–L12

Same shape as 2a: `chain[m & chain_mask]` is a serial pointer chase with no prefetch,
and unlike Bt it has no tag anywhere — `get_h` on the lazy ladder loads every
candidate the hash offers. The prefilter (`pre_eq`, C's `match[ml]==ip[ml]`) is now
check-free but still costs the random access it reads. *Levers:* (a) prefetch
`chain[next]` before `count_match` runs on the current candidate — the next address is
already known, this chase is NOT dependent, which makes it the *easiest* prefetch win
in the file; (b) a tag experiment on the lazy ladder's shared hash table
(`use_tags` is currently `Fast`-only by allocation). *Instrument:* probes/attempt
counters exist (`note_search`); byte-identity 80-cell L5–L12 board (built, `btlev.rs`).

## 4. `count_match` / `count_eq_len` — the compare kernel

Runs **247M times at L19** (in-code figure). GATE 15's standing finding: the AVX2 arm
reads 64 bytes/side before it can answer, but mean match length at L3 is ~9.6 bytes
and mean literal run 3.75 — **most calls die inside the first 8-byte word**. The gate
(`EQLEN_ARM`, "peek 8 bytes then go wide") exists under `profile` only. *Lever:* make
the first-word early-exit the unconditional head of the AVX2 path (C does this: one
XOR+TZCNT before any wide load). *Instrument:* `EQ_LEN_HIST` (length histogram,
already present) decides how much traffic dies at ≤8; asm confirms the head is 3–4
instructions. Byte-identical by definition (it returns a length).

## 5. `find_fast_impl` — BROKEN OPEN 2026-08-20 (rusty-curiosity pass)

**The finding: the specialised copies served ZERO blocks.** A dispatch-arm census
(`FF_ARM`, profile-gated) at default configuration:

```
spec=0   TAG-GENERIC=58..640   rep-generic=6..596     on all 8 probe corpora
```

`ut` (the tag filter) defaults ON and `rep_on` covers most remaining blocks, so 100%
of L1 traffic ran the **HLOG=0/STEP=0 generic bodies** — the exact runtime-shift/
runtime-step cost the file documents as a work-parity break. The five-hlog × four-step
family the dispatch comment calls "the shipping configuration" was dead weight; GATE
12's finding inverted. **FIXED** (`1e0a45e`): the three live combinations —
`(tag,¬rep)`, `(¬tag,rep)`, `(tag,rep)` × s0∈{1,2} × hlog 12–16 — now specialise;
census reads spec=100%; byte-identical 36/36 via `set_fast_spec_arm` A/B.

**How it was found (the method matters):** stage share (81.5% matchfind on dickens) →
per-position closure: ~10 ns ≈ **45 cycles/position** against a ~20-instruction miss
path → implied IPC ≈ 0.5, arithmetic does not close → before blaming memory, prove
which code runs → census said *none of the specialised code runs at all*.

### 5a. tags as a SECOND array — **SHIPPED 2026-08-20** (`2f4383a`)
Packed-in-slot on the Fast ladder, guarded < 16 MiB like T1; the one hazard (the
mid-frame Fast→Lazy shared table — the exact reason for the historical refutation)
is handled by a one-time unpack at the switch site. Receipts: dickens L1 tag compares
OFF tag-array **4,027,923** / ON tag-array **0**, rejects **913,538 = 913,538** to the
unit; byte-identical **54/54** (L1/L2/L3 × 18) in-process; the second store per
position gone structurally (array dropped on packed frames). No clock claim.
*Open flag:* cross-session `tagprice` totals moved 19.4% → 50.7% at identical probe
counts — unattributed drift from the concurrent gate work, does not affect the A/B.

#### The original analysis (kept for the mechanism)
The 45-cycle closure still is not fully explained by the generic body. On the live tag
path every probe touches **two arrays**: `load_fast::<true>` reads `hash[h]` then
`tags[h]` (two random cache lines), and `store_fast::<true>` writes both, every
position. C's fast loop touches ONE array. The packed-in-slot form (tag in the slot's
top 8 bits — T1's mechanism, which came FROM this ladder) exists disabled in
`store_fast` behind `if false`, refuted historically for exactly one reason: **the
mid-frame Fast→Lazy switch shares one table, and Lazy reads entries Fast truncated.**
DFast's T1 solved the same problem with `pack_tags` + a per-frame guard. The path here:
pack on Fast, and **unpack the table once on the (rare) Fast→Lazy switch** — a 16K-entry
walk per switch. *Instrument:* allocator/asm parity + 72-cell identity; the tags-array
allocation disappearing is the deterministic receipt.

### 5b. Sibling finding: x-ray's L1 loss is NOT this function
Stage anatomy at L1: `x-ray` is **84.9% EncodeHuff, 7.0% matchfind** (min_match=7
finds nothing in 12-bit greyscale). Its C/us 2.83 belongs to the **Huffman encoder**
— a separate engine target (`encode_stream`/`emit_fill`), not listed in this file's
priority order until now. `sao` decode similarly flipped to DecLits 78.3%.

### 5b2. Hit/store-path wins — **SHIPPED** (`15065b3`)
`emit_fast_seq`'s back-extension was the Fast ladder's un-de-checked copy of the T2
walk — two bounds checks **per extended byte, per match**; `back_eq` applied verbatim
(211 → 194 instrs, 2 → 0 panic sites; the **seventh** neighbour-capability instance).
`store_fast` ran the tags-array length check ahead of the pack branch — a per-position
check against a guaranteed-empty array on packed frames; the packed write now returns
early. Byte-identical 54/54 + 36/36; rejects equal to the unit.

### 5d3. Round-4 results (2026-08-20 later)
* **SHIPPED**: table base register-resident (`8bbf719`, iteration-window rbp-reloads
  3 → 0); deferred tag allocation (alloc receipt **−1,548,740 B**/board, the per-frame
  memset gone).
* **REFUSED**: culling the dead `(false,false)` family — those arms are the gate
  campaign's measurement-parity arms (RZSTD_TAG=0), and culling rebuilds the
  work-parity break they exist to prevent.
* **PRICED for the gate campaign**: the 4-byte hash wastes **82.9% of candidate
  passes at L1** (13.96M/board: sao 96.1%, mr 95.7%, x-ray 98.9%), 75.8% at L2 —
  C hashes `mls` bytes. Output-changing; strongest known L1 lever; needs
  worst-corpus gates. Census: `ffwaste.rs`, counters inside `fast_probe`.

### 5d4. The mls-wide hash — **SHIPPED DEFAULT ON** (owner's call, protections found)
**Seven**-design refutation ladder recorded in-code (#7, rep-substitution: the probe's
new gram knowledge swaps a far match for the same gram at rep1 — census found ALL 80
veto-block accepts had **no gram at rep1** at any slack; inert on every corpus, removed
per the OPT_SKIP_FLOOR precedent). 311/391 accepts land *before* rep dominance: the
divergence is seeded early, amplified by cross-block state — frame-level foresight no
block-local dispatch can have. Survivors: switch-latch + window
**re-seed** (lazy inherits real heads) + ML≥16 anchor bar on rep-dominated blocks.
Final: L1 TOTAL −2.49%, **HOLDOUT −4.92%**, worst versions **+6.33%** (from +14.8%;
segment experiment proves the residue is cross-block state-path divergence — the
family's floor). L2 TOTAL −2.82% with versions itself **−8.14%**. Waste receipt
82.9% → **0.1%**. Worst-corpus law waived for versions@L1 only, explicitly.

#### Original entry
Waste receipt with the arm on: **13,956,827 → 2,676 wasted candidates (82.9% → 0.1%)**.
Adjudication: L1 TOTAL −2.59%, **HOLDOUT −4.92%** (reymont −10.2%, mr −7.2%), worst
**versions-16m +14.8%** — fails worst-corpus, so DEFAULT OFF. Two protections
**refuted and recorded**: per-block key dispatch (+34.6% — mixed keys poison the
shared table) and a frame latch with table clear (+17.2%, degrades L2 too). The
descent showed versions' hash path sees ~2K candidates total: the loss is dispatch
COUPLING (early matches shift `rep_yield`/`rep_run`, breaking the repcode chain), so
the protection belongs with Gate 5/8/10's versions playbook, not in the key.

### 5d. Named, not taken *(next descent's shortlist)*
* **Hash width diverges from C**: we hash 4 bytes at every `min_match` while C's
  `ZSTD_hashPtr` hashes `mls` bytes (6–7 at L1) — lower precision, wasted
  `count_match` on sub-mls hits. **Output-changing** — needs worst-corpus gates, not
  byte-identity. Potentially the largest remaining L1 lever.
* `accel` is a per-block variable shift (`shr %cl`) that is the constant 7 for Fast
  unless pinned — const-specialisable.
* `dfast_fill_ends()` is one atomic load **per match** for a process constant
  (brick-79 shape, small).

### 5c. Residue
The 13 panic sites in one inlined instantiation still need the CodeView inline-site
chain (OPEN). The dead-copy census question is now answered for the OLD family (0%);
the new family's per-copy split is measurable with the same `FF_ARM` counter.

## 6. `find_sequences_strategy` — the dispatch hub, 4,391 instrs

Carries `find_opt` inlined, 217 call sites, 1,553 stack movs, and the **last 17 warm
panic sites** in the codec. Its `tables.chain` allocation
(`alloc::vec![0u32; 1 << chain_log]`) fires on strategy switches mid-frame
(Fast→Lazy) — an allocation inside the per-block path on exactly the frames the
switch gate fires. *Levers:* (a) the 17 sites, same treatment as the huffman
table-builds (safe restructures first); (b) keep the chain allocation on
`MatchTables` like every other buffer this campaign hoisted. *Instrument:* panic
census; `g6alloc` board.

## 7. `match_ok` / `fill_hash_after_match` / `try_rep1` — small, hot, already lean

`match_ok` is 82 instrs with early-outs and is called per candidate; its four `call`
sites are profile plumbing. `fill_hash_after_match` fully inlines. `try_rep1`
inlines. **No identified lever** — listed so nobody re-audits them: they were swept
in the keyword audit (clean on allocation, arms, and bounds).

---

## 8. The tag system — states, transitions, invariants (audit 2026-08-20)

Five incidents in one campaign traced to this machinery (the 190ad8b stale-tag
day, the fill-representation bug that moved 12/18 corpora, the oracle parity
break, the ffpack A/B silently becoming a finder comparison, and the
misattributed packed-tag refutation). A full audit was run; this section is the
contract that came out of it.

**Why it bites: the axes.** "The tag filter" is not one feature. It is the
product of: THREE short-table representations x TWO key widths x mid-frame
transitions x FOUR arms (`tag_alloc`, `fast_pack`, `dfast_tag`, `fast_hash_wide`)
x per-block read gates (`ut`, `dtag_on`). Every incident happened at a pairwise
interaction of axes, never inside one.

**States of the short table (per frame):**

| state | `pack_tags` | `tags` | slot contents | who |
| --- | --- | --- | --- | --- |
| S0 legacy | false | empty | `pos+1` | no filter |
| S1 array | false | `1<<hlog` bytes | `pos+1`, tag beside | Fast >=16 MiB, streaming Fast |
| S2 packed | true | empty | `(pos+1)&0xFFFFFF \| tag<<24` | Fast/DFast <16 MiB |

Transitions: frame init picks the state (`enable_packed_tags` + the Fast-only
array fallback); the Fast->Lazy switch runs S2 -> S0; `fast_hash_relatch`
(wide frames) re-seeds a window and latches legacy. **After a relatch the
unpack loop is SKIPPED** (the relatch already cleared `pack_tags`), so slots
outside the re-seeded window keep packed, wide-keyed bits while the flag says
unpacked. That is safe BY `match_ok` DISCIPLINE, not by representation:
`m >= ip` is its first test, every consumer routes through it (audited:
chain walk, lazy heads/fills, Bt never sees tags at all), and the clear-vs-not
experiment measured byte-identity. The invariant to preserve: **`pack_tags ==
false` does not promise tag-free slots; never add a consumer that trusts
positions without `match_ok`.**

**The one output-corrupting failure mode** is a false reject — the filter
hiding a candidate that would have matched. Receipt (`tagaudit`, profile):
L1 23,384,133 rejects / **0 false**; L2 17,198,345 / **0 false**; L3 ledger
2,040,379 rejections of 8,010,108 nonempty slots (25.5% avoided). The proof
shape behind the zeros: the tag derives from the same bytes as the index, so
an accepted match implies an equal tag.

**Instrument trap:** `TAG_FALSE_REJECT`/`TAG_REJECT_TOTAL` carry DIFFERENT
semantics per finder — Fast counts provably-lost matches (must be 0) after
re-probing the raw slot; DFast reuses the same counters as (rejections,
nonempty slots), where rejections are the WINS. Never assert across levels.

**De-confusion (shipped with this audit):** the `PACKED` const generic used to
exist in five signatures with three liveness states — dead in `hash4_tag`,
`fast_hash_tag`, `store_fast`, and `fill_hash_after_match` (tags are computed
unconditionally since the Gate-7 per-block poison fix), live only in
`fast_slot_load` as the compare gate. The dead ones are removed; `PACKED` now
means exactly one thing: this monomorphisation compares tags. The dead
`load_fast` (superseded by `fast_slot_load`) is deleted.

**Routing holes — CLOSED 2026-08-21, priced on the T1 instrument (`tagbig`):**

1. **DFast >= 16 MiB frames ran with NO tag filter** (`enable_packed_tags`
   refuses the length; the array fallback was Fast-only, so `dtag_on` was
   silently false). The frame-init fallback now routes the S1 array to DFast.
2. **Streaming DFast: same hole** — `alloc_fast_tags` extended likewise.

`tagbig` (full-length frames, 9 corpora x L3/L4, in-process arm A/B where
arm-off == pre-fix behavior): **18/18 byte-identical**, and the filter
rejects **12,202,975 of 37,031,552 nonempty probes (33.0%)** — each one a
random `src[m]` load + gram compare + dying `count_match` the frames
previously paid in full. `mozilla` (51 MB) rejects **61.2%** at L3, `samba`
29.6%; the degenerate big corpora (versions/text/zeros/incomp) probe the
short table almost never and are unaffected. Cost: one `tags[h]` byte read
per nonempty probe, a tag write per store, and a `1 << hash_log` (<= 256 KiB)
allocation per frame. The Fast array precedent accepted this same trade at a
22.7% rejection rate; the DFast big-frame board clears it at 33.0%.

**Long-table array route — CLOSED 2026-08-21 (`ltagbig`):** the last unfiltered
tag surface. `ltags` byte array beside `hash_long`, same g4 tag, engaged only
where pack is refused. Full-length board: **28,187,616 of 36,885,171 long
probes rejected (76.4%)** — `mozilla` 91.2%/90.6% — and the consume-site
ledger lands at the collision floor again (28,897,386 unfiltered wasted loads
→ 125,441, 0.43%). 18/18 byte-identical, FALSE 0.

**The mls-width SHORT tag — SHIPPED 2026-08-21 (`stag`):** the short consume
census (mirror of the long one) found the 4-byte tag's blind spot: survivors
share the tag's whole 4 bytes and die at byte 5 against mls = 5 —
**8,453,099 wasted random loads/board (32.2% of unfiltered)** where the long
table's same class measured 0.42%. `hash4_tag_mls` keeps the index bit-exact
and widens only the tag to `min(mls, 8)` bytes from the u64 the site already
loads for `hash8`. Result: short residual **8,453,099 → 286,170 (1.1%)**,
short load-site rejection 25.5% → 65.2%, accepted count bit-identical
(4,460,182) across arms, and the long tables ride it for free (big-frame
residual 125,441 → 112,755, the exact 0.39% floor). One AND + one MUL per
position; the mlx-long refutation stands for dedicated long-side arithmetic
and is subsumed as a free side effect.

Every tag surface in the encoder (short packed, short array, long packed,
long array) now runs at or near the 8-bit collision floor at every frame
size.

---