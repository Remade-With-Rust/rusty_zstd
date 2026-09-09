# asmcensus -- deterministic loop verdicts from the emitted assembly

Built 2026-09-09 for the matchfind campaign, when the box's CPU load made every
clock inadmissible. All of it reads `target/release/deps/rusty_zstd-*.s` from

    cargo rustc --release -p rusty_zstd -- --emit asm

- `cfg.py`        -- shared: regions -> real CFG (every jump inside a region is an
                     edge; jump tables resolved), dominators, NATURAL loops.
- `loops2.py`     -- per symbol: each natural loop's instrs / spill stores / stack
                     reads / rip-relative static loads / calls.
- `paths2.py`     -- the TRUE shortest header->latch path of one loop (Dijkstra on
                     executed instructions), `nocall` to isolate the no-match path,
                     `-v` to print it. This is the per-candidate / per-position cost.
- `pathdump.py`   -- print an explicit block path with its stack reads marked.
- `hotslots2.py`  -- hot stack slots of one loop with provenance (spill / hoisted
                     invariant / incoming argument).
- `loopsig.py`    -- loops with a content signature (constants, stores) to match
                     them across two builds.
- `riploads.py`   -- which statics (knob arms) a loop reads, and how often.

Verdict discipline: compare PATHS, not loop totals -- a loop total is the union of
its arms. See CHANGELOG "matchfind" for the bricks these decided.

## The three-target board (2026-09-09, bricks 35-71)

Four more scripts, written for the lazy ladder's candidate / insert / position
campaign. They price PATHS (Dijkstra over executed instructions, header to latch)
selected by the FEATURES a path must execute -- bit 0 an indirect call, bit 1 an
`xor` (the first-word compare), bit 2 a register byte compare (the tag test), bit
3 `shrq %cl` (the lazy step), bit 4 `bsf` (the fused short count), bit 5 a
memory-operand `cmpb` (the `pre_eq` byte test) -- because the shortest cycle of a
loop is almost never the common one.

- `verdict3.py <a.s> [<b.s> ...]` -- the board: per chain kernel the prologue,
  the walk loop, and the tag-skip / first-word-miss / **pre_eq-fail** (the 83%
  path at L9) / fused-short paths; per fill body its per-byte loops; per finder
  the no-match position cycle with and without the rep probe, the look-ahead
  step, and the INLINED walk loops. `verdict3.py <a.s> 'K cp.wc' 38` dumps one
  path (the number is the feature mask).
- `score.py <a.s> ...` -- ONE number per state: the modelled instructions per
  input byte at L9 (`mfbudget`'s unit rates: 0.297 walks, 1.725 candidates,
  0.147 tag skips, 0.087 fused resolutions per byte) summed over the four
  shipping kernel shapes, for both the pointer-dispatched and the inlined form.
  A brick that moves two arms in opposite directions is decided here.
- `pathdump2.py <a.s> <symbol-regex> <header> <mask> [nocall]` -- any loop's
  shortest cycle executing the mask's features, segment by segment.
- `poscycle.py <a.s>` -- the lazy finder's per-position cycles: plain no-match,
  through the walk (one tag test), and through an examined candidate.

What they found, in order: the candidate path priced for twenty bricks was the
0.6% one; the per-walk CALL frame was second only to the real candidate path;
inlining the walk (`find_lazy_impl::<MLS, KIND>`) cut the model 396 -> 339, and
eleven bricks on the inlined form took it to 285. Full record in CHANGELOG.md.

- `gwalk.py <a.s>` -- the greedy (L5) finder's inline walk instances: dominant,
  tag-skip and fused paths, and each position loop's cycle through the walk.
- `reppath.py <a.s>` -- the lazy finder's inlined position loops: the no-match
  cycle with and without the rep probe executed.

- `fillloops.py <a.s> ...` -- every fill body's loops (each `lz_fill_range`
  instantiation and `row_fill_range`): size, mnemonics and the per-call
  prologue to each. The board's one `F` row folds a body's arms together; this
  is where a producer change that drops the shipping quad 68 -> 56 while a
  cold arm grows +12 is told apart.