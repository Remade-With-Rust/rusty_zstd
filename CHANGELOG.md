# Changelog

All notable changes to this project are documented here. The format is loosely
based on [Keep a Changelog](https://keepachangelog.com/); this project uses
[Semantic Versioning](https://semver.org/).

## [Unreleased]

### Fixed -- the census counters cost the crate BARE METAL; they no longer do

`cargo check -p rusty_zstd --no-default-features --features alloc` failed with
**204 errors** on `thumbv7em-none-eabihf` (Cortex-M4F) and the same on
`riscv32imac-unknown-none-elf`. Every one of them was
`cannot find AtomicU64 in atomic`, and every one was an INSTRUMENT: reload
counts, decode band histograms, kernel reach, walk exits, copy bytes. Not one
was a line of codec. Neither part has 64-bit atomics.

This was found by `rusty_RTOS`'s house gate, which consumes the crate for
on-chip OTA payloads and trace capture, and it is the reason its plan says a
`no-std` category on the registry is not a claim: **the target that compiles is
the claim.** The plan estimated eight statics from a capped rustc report; the
real count is 427 fully-qualified uses plus six imports across sixteen files.

`census64` is now the one seam. On every target that HAS 64-bit atomics it is
`pub use core::sync::atomic::AtomicU64` -- the same type, not a wrapper -- so
the public statics keep their published type and the emitted code is unchanged.
**Proved, not asserted:** the board over every shipping kernel, fill and finder
is identical to the pre-change assembly in all thirty-two columns, and the
identity gates still read GOLD `2F6594F7EEDBD12B` / 59,680,638 and LDM
`57BE83EA4E1199E8` / 57,796,847.

On a part without them it is a zero-sized stub: the statics leave BSS entirely
and every `fetch_add` folds away.

**Why a stub rather than `portable-atomic`.** That crate would keep the census
READABLE there, and it is the other honest answer. But on a core with no 64-bit
atomic instruction it needs a critical-section implementation, and a library
that turns that feature on conscripts every downstream firmware's interrupt
policy so a diagnostic counter can increment. The seam is one type wide, so a
firmware that does want the count can supply `portable_atomic::AtomicU64`.

**A zero there means "not measurable on this target", never "measured zero"**,
which is the same trap this crate's own rules warn about -- so it is published,
not buried: `census64::CENSUS_LIVE` is `false` on such a build. The reverse
mistake would be far worse (a stub selected on a HOSTED target would make every
count-based verdict fiction while every gate still passed), so it is gated two
ways: a compile-time size check, and a unit test that asserts on BEHAVIOUR
rather than on the constant -- it counts to 42 and reads it back.

CI gains the rung, in the job that already guards the portable configurations:
`--target thumbv7em-none-eabihf` and `--target riscv32imac-unknown-none-elf`,
both `--no-default-features --features alloc`, on every push.

### Measured -- what the encode campaign bought: 1.08-1.27x at L3-L12 (2026-09-09)

Every verdict in the sections below is an INSTRUCTION COUNT, because this box
runs at ~65% load from other work and cannot resolve a stopwatch. This section
is the wall-clock check on those counts, taken the only way that is admissible
here: the released `v0.2.3` (`e672cd0`) and this tree, each compiled against
ONE measurement program (`speedab`) that loops `compress` in memory with no
file I/O in the timed region; whole processes alternated ABBA, pinned to one
core at High priority; the estimator is the FLOOR (the fastest loop either arm
reached across all pairs), which is what survives a busy machine.

**The floor of the instrument, measured first** by running the same binary
against itself: encode +-3%, decode +-0.5%. Nothing below +-3% on encode is a
result, and the rows that sit there are reported as flat rather than dressed up.

Encode, MB/s at the floor, `v0.2.3` -> this tree:

| L | strategy | dickens 9.7 MiB | samba 20.6 MiB | speedup |
|---|---|---|---|---|
| 1 | Fast | 189.8 -> 186.9 | 272.3 -> 275.5 | **flat** (0.99 / 1.01, inside the floor) |
| 3 | DFast | 129.4 -> 140.3 | 261.0 -> 268.4 | 1.08 / 1.03 |
| 5 | Greedy | 67.2 -> 79.1 | 101.8 -> 128.7 | **1.18 / 1.27** |
| 7 | Lazy | 11.0 -> 13.4 | 22.1 -> 28.0 | **1.23 / 1.26** |
| 9 | Lazy2 | 18.2 -> 20.9 | 38.0 -> 46.0 | **1.14 / 1.21** |
| 12 | Lazy2 | 3.5 -> 3.9 | 12.3 -> 13.3 | 1.11 / 1.08 |
| 15 | BtLazy2 | 3.0 -> 3.0 | xml 8.3 -> 8.9 | flat / 1.07 |
| 19 | BtUltra | 2.5 -> 2.5 | xml 5.3 -> 5.6 | flat / 1.05 |

The shape matches what the bricks say they did and is the reason to trust both:
**the win is exactly where the campaign worked.** L5-L12 are the greedy and lazy
chain finders, which took nearly every brick, and they move 1.08-1.27x. L1 is the
Fast ladder, which took none, and it does not move. L15-L19 are the tree ladder,
which took a single brick (99, a per-call prologue trim), and they read flat to
1.07. A campaign that claimed a uniform win across all levels would be measuring
the box, not the code.

Compressed sizes are IDENTICAL at every level except 3, which is DFast and is
the one ladder carrying a deliberate bitstream change in this release: the
next-long OFFSET-TRADE dispatch now defaults ON (see its section below). It
reads samba **-1.14%** (smaller) and dickens +0.04% here -- the same trade that
section measured at -0.36% across an 18-corpus L3 board, and the reason the L3
decode row moves too. Decode is
inside the floor everywhere the bitstream is unchanged; the one row that reads
+12.5% (samba L3) is NOT a decode win but that smaller frame giving the decoder
less to do, so it is not work-parity comparable and is not claimed.

### Changed -- rusty_alloc 2.0.0 -> 2.0.5 in the deliverable seam

`rzstd-alloc` moves its exact pin to `=2.0.5` (`cargo tree` on the CLI shows
only 2.0.5). The doc comment beside the pin moves with it, which is the drift
this file already calls out by name.

**Priced, and it is neutral -- recorded that way rather than as a win.** Same
codec on both arms, same instrument and floor as above:

| workload | encode | decode | sign test |
|---|---|---|---|
| bulk (dickens, samba @ L1/5/9/12) | 0.96-1.05 | 0.97-1.02 | 17/40, z = -0.95 |
| allocation-heavy (smallmsg-8m, jsonlog-16m @ L1/L3) | 1.00-1.02 | 1.00-1.03 | 12/28 |

Every cell is inside the instrument's floor and the sign test is a coin flip, so
the honest verdict is **no measurable throughput change on this workload**. The
allocation-heavy arm was run because bulk codec loops are the wrong place to
judge an allocator -- small inputs at fast levels are where per-call table
allocation is the biggest share -- and it reads flat too. (`zeros-1m` was
dropped from the table: at ~0.07 ms per loop it is timer quantisation, not a
measurement.) This is a dependency-currency update, and the reason to take it
is that the seam exists so the pin can move without touching feature code.
### Changed -- matchfind, the three targets: candidate, insert, position (2026-09-09)

The diagnosis below priced three units against zstd 1.5.7's own assembly and
found them dear: the chain walk's per-CANDIDATE first-word path, the fill's
per-INSERTED-BYTE loop, and the lazy finder's per-POSITION loop. This section
works those three, round-robin. Every verdict is a shortest header-to-latch
PATH on the emitted assembly (`verdict3.py` in the census tools: the candidate
path is the one that executes the tag test and the first-word xor, the
position path the one that executes the kernel's indirect call and the
`lazy_step` shift and no direct call), never a loop total. Byte-identical
throughout: GOLD `2F6594F7EEDBD12B` / 59,680,638 and LDM `57BE83EA4E1199E8` /
57,796,847 after every landed brick; 173 tests, 176 with `profile`.

| # | target | brick | verdict (path instrs / stack reads / stack stores) |
|---|---|---|---|
| 35 | K | `min_match` is 3..=7 by contract (`compression_params` and the advanced setter both clamp; zstd's own max is 7), but the chain-ladder finders read `.max(3)` and the kernel tested `mls > 8` on EVERY examined candidate to pick `mls_eq_wide`. Finders now `clamp(3, 7)`; `mls_xor` has no wide arm | walk loop union **176 -> 118** instrs, spills 19 -> 10, reloads 46 -> 22 (cp, walk_cont); kernel 341 -> 279. The candidate PATH is unchanged at 28 (the two freed instructions were spent by the allocator on a chain-base reload and an xor copy) -- landed for the contract and the loop body, **not counted as a path win** |
| 36 | F | the fill's packed link decode -- `(q - 1) \| (raw & 0xFF00_0000)` guarded by `q == 0` -- was seven instructions per inserted byte. Every packed writer stores `pos + 1 >= 1`, so `q == 0` is `raw == 0` and `raw - 1` cannot borrow out of the field: one `saturating_sub(1)` serves all three representations | per inserted byte **29 -> 23**, stack reloads 1 -> 0 (the freed register took `PRIME64`, which had been rematerialised per byte) |
| 37 | P | the lazy loop tested its search result twice per position (`best_ml >= mls` for the look-ahead, then `best_ml != 0` for the emit -- the same predicate by W9/W10) and spilled `best_ip`/`look_hi` across the join on the no-match path. One guard; the probes are born inside it | no-match path **27 -> 22** instrs, reads 10 -> 9, stores 1 -> 0; with the rep probe 43 -> 38 |
| 38 | P | the rep probe's four admission tests per position (`use_rep` byte flag, `rep1 == 0`, `ip + 1 < rep1`, `ip + 1 - rep1 < lowest_rep`, four stack reads) are one bound on `ip`: `rep_bar = rep1 + lowest_rep - 1`, `usize::MAX` when the probe is off, refreshed only where `rep1` changes | rep-probe path **38 -> 34**, reads 12 -> 10; no-match path reads 9 -> 8 (`rep_bar` lives in a register) |
| 39 | F | the packed head write masked `p + 1` to 24 bits per inserted byte; `pack_tags` already bounds it below 0x00FF_FFFF | per inserted byte **23 -> 22** |
| 41 | P | `lazy_step`'s `sh == 0` arm (`cmp`, `mov $1`, `cmov`, a stack read) ran on every no-match position to select the historical step of 1. The knob is mapped once per block (`0 -> 63`; `(ip - anchor) >> 63` is 0 for any span a block holds) and the step is one expression | no-match path **22 -> 19**, reads 8 -> 7; rep-probe path 34 -> 31 |
| 43 | F | the fill's stride is a runtime knob (1 by default), so LLVM could neither count nor unroll the loop and every byte paid the trip (`add`, `cmp`, `jb`). The stride-1 case is named and inserts two positions per trip, chain arms only (the rows arm ships at stride 2 and measured +3 with the pair loop present) | per inserted byte **22 -> 20** (packed), 23 -> 21 (tag array), 14 -> 11.5 (no tags); rows arm unchanged at 35 |
| 46 | K/inst | the PHANTOM position-0 candidate, counted (profile only). A chain link of 0 is both "no link" and position 0, so a walk whose chain ends inside the first window continues to m = 0 and examines it -- the reason the walk guards `m != 0` before the tag test. `phantoms` census, Silesia 5 corpora | examined at position 0: **0.29% / 0.34% / 0.20%** of all candidates at L7 / L9 / L12, accepted 3 / 3 / 3. So a representation with an unambiguous null would save the two-instruction guard and ~0.3% of examinations at the risk of three accepts per level: **not taken**; the guard stays |
| 47 | P | the chain kernel's per-CALL insert decoded the old head with the fill's old seven-instruction field split (`lz_link_from_head`); brick 36's identity applies: `raw.saturating_sub(1)` | kernel prologue (entry to walk header, per position) **89 -> 84** (cp, walk_cont), 87 -> 81 (cp) |
| 48 | P | `lz_head_put` masked `pos + 1` to 24 bits per chain insert; `pack_tags` bounds it (brick 39's argument, per position) | prologue **84 -> 83**, 81 -> 80 |
| 49 | P | the kernel's entry guard tested `ip + mls <= src_len` per call (add, compare, branch, both operands reloaded) for the caller's own invariant: `find_lazy_impl` calls at `ip <= ilimit = block_end - 8`, `mls <= 7` | prologue **83 -> 80** (cp.wc), 80 -> 75 (cp), 99 -> 86 (ca.wc), 88 -> 81 (ca), 81 -> 76, 76 -> 72; candidate path 28 -> 27 (cp.wc, ca) and **23 -> 25 on ca.wc** (the allocator re-spilled `smask` there; at 0.3 walks and 1.7 candidates per byte the two move ~equal amounts on that one kernel, the other five are clear wins) |
| 50 | P/K | the walk's `tables.wcls.0 += 1` / `.1 += 1` (the walk_cont classification, written on a rare accept) were in-loop memory read-modify-writes, so LLVM's scalar promotion loaded both at kernel ENTRY, spilled them, and stored them back at every exit -- per call. Two locals, added to the table once after the loop, guarded so the common walk never touches it | ca.wc candidate path **22 -> 19 at brick 56's state** (25 -> 22 here), tag-skip 18 -> 17; cp.wc prologue +2 (the allocator's re-shuffle) -- modelled at L9's unit rates (0.3 walks, 1.7 candidates, 0.15 skips per byte) a net **-4 instructions per byte** across the four shipping kernels |
| 56 | K | the walk's loop is register-starved (fifteen live values), and two of them existed only for the RARE long-count continuation: `ip + 8` and `ip + 16`, hoisted as `count_match` arguments and spilled at every entry. The `x == 0` continuation is outlined behind `ctx` (`walk_count8`, ~0.04 calls per byte); the `x != 0` head stays inline | kernel **283 -> 239** (cp.wc), 279 -> 190 (cp), 282 -> 254, 278 -> 198, 337 -> 234, 257 -> 178; walk-loop unions **115 -> 79** instrs, spills 8 -> 4, reloads 25 -> 17 (cp.wc). The candidate PATH: 27 -> 27 (cp.wc), 28 -> 28 (cp), 22 -> 23 (ca.wc, +1 store), 27 -> 28 (ca), 20 -> 19, 23 -> 23 -- landed for the loop body and the frame, **not counted as a path win** |
| -- | K | **CORRECTION.** Every kernel verdict above priced the candidate path that FAILS the first-word compare. `mfbudget` section B says that path is 0.5-0.6% of examined candidates at L7-L12: the bucket and the tag already guarantee the first bytes, so 99.4% of examined candidates PASS the first word and go on to `pre_eq` (the byte at the current best length), which is where most of them die. The dominant per-candidate path is first word passes -> `pre_eq` fails -> advance (~83% of candidates at L9: 5.8 examined per walk, of which one sets the best and the rest fail `pre_eq`). Re-priced on that path, brick 35 was **+4 on cp.wc and +10 on ca.wc** (36 -> 40, 30 -> 40), not neutral, and the ordering of the refuted kernel bricks changes (55 was -5 on it). From here every kernel number is the `pre_eq`-fail path, and the verdict is `score.py`'s modelled instructions per byte at L9 -- walks x frame + candidates x pre-path + skips x skip-path + fused x (fused - pre), summed over the four shipping shapes | at the session's start the model read **374.7** instrs/byte (cp.wc 100.4, cp 92.6, ca.wc 90.3, ca 91.4); after bricks 35-56 it read **396.0** -- the kernel bricks had been a net loss on the path that runs |
| 58 | K/P | the walk INLINED into the lazy finder. Every search was a call through a per-block pointer: eight pushes and pops, the marshalling, a `nop`, ~12 context loads and ~11 entry spills in the callee, a 16-instruction exit -- about 35 instructions per walk of frame at 0.3 walks per byte, which zstd's `_lazy` parser never pays. `find_lazy_impl` takes a `KIND` const for the four shipping shapes (packed / tag-array x walk_cont) and `lazy_search::<MLS, KIND>` resolves to the `inline(always)` inner, so the walk lands inline at both call sites; rows and tags-off keep the pointer (`KIND == 7`). `find_lazy` grows 1200 -> 6890 instructions (five instances, one hot per block) | per-walk frame (position cycle through one candidate, minus that candidate) **116 -> 78** (cp.wc), 103 -> 47 (cp), 120 -> 73 (ca.wc), 109 -> 48 (ca); the inlined `pre_eq`-fail path 38 / 36 / 37 / 33 with 8-11 stack reads (against 40 / 31 / 46 / 28 with 2-11 before). Model **396.0 -> 338.9** instrs/byte; stack reads on the candidate path +3..+6 (a clock question this box cannot answer -- recorded, not hidden) |
| 59 | K | ONE length in the walk. `best_ml` is born `mls - 1`, so `ml > best_ml` is the accept test before and after the first accept (W7's `bar = best_ml + 1` seeded with `mls` is gone), `pre_eq` at that index is sound on the first candidate (the first-word compare just verified byte `mls - 1`), and "no match yet" is `best_ml < mls`. One fewer loop-carried value and one fewer test on the dominant path | model **338.9 -> 327.8**; inlined pre path 38 -> 36 (cp.wc), 36 -> 33 (cp), 37 -> 33 (ca.wc), 33 -> 33 (ca) |
| 61 | K | the inlined walk read its chain link through TWO dependent loads per candidate (the `&mut MatchTables` pointer from the frame, then `chain`'s base from it) because `chain_masked` goes through `self` and LLVM would not hoist the base inside a function that stores to the tables elsewhere. The bases are taken once per walk (`chain.as_ptr()`, `ctags.as_ptr()`), as the fill did in brick 12 | model **327.8 -> 319.7**; inlined pre path 36 -> 35 (cp.wc), 33 -> 32 (cp), 33 -> 31 (ca.wc), 33 -> 30 (ca); per-walk frame 82 -> 85 / 52 -> 55 / 72 -> 78 / 51 -> 56 (the bases are two more spills per walk, paid 5.8 times over per walk) |
| 62 | K | the inlined walk counted attempts UP against a limit in the frame (`cmp limit(%rbp)`, `jae`, `inc`: three instructions and a load per candidate) where the standalone kernel had `dec`/`je`. An explicit countdown that nothing else reads | model **319.7 -> 311.5**; pre path 35 -> 34 (cp.wc), 32 -> 31, 31 -> 29 (ca.wc), 30 -> 29, one read fewer on each |
| 63 | K | both miss arms wrote `missed_before = true`, so LLVM hoisted the one constant above the tag test (`movb $1` on EVERY candidate) and restored the old value at the join on the paths that never miss. The arms write different non-zero values (1 tag reject, 2 first-word reject; readers test `!= 0`), so there is nothing to hoist | model **311.5 -> 306.5**; pre path **34 -> 31** (cp.wc) |
| 64 | P | brick 50 undone for the inlined form: its two locals cost two zero stores per walk entry (duplicated across the entry's split edge: four) and a five-instruction two-load `wc0 \| wc1 != 0` test per walk exit. The direct read-modify-write on the rare accept is two instructions, there, and nothing elsewhere | model **306.5 -> 301.8**; per-walk frame 83 -> 73 (cp.wc), 81 -> 71 (ca.wc) |
| 65 | P | the walk's lower bound `lowest.max(ip.saturating_sub(window))` was a saturating subtract and a max (two compares, two `cmov`s, a zero) per walk. `lowest + window` is a block constant in the context; per walk it is one compare and a select | model **301.8 -> 292.5**; per-walk frame 73 -> 66 (cp.wc), 56 -> 48 (cp), 71 -> 66 (ca.wc), 58 -> 51 (ca) |
| 66 | F | brick 52 retried once the walk was inlined: the chain-link tag is the mls-byte product's TOP byte (`tv >> 56`, seated with `shr $32` + `and`) instead of the xor-fold (copy, shift, xor, `shl $24`); the two wide-chain producers take the byte under their bucket. Five producers changed together; sound, so byte-identical by the `taggate` argument | fill pair loop **40 -> 38** per two bytes (19 per byte); model 292.5 -> 292.0 (the walk's per-call tag is two instructions cheaper) |
| 68 | P | brick 54 retried inlined: the dead `wide_hash && ip + 8 <= src_len` arm out of the walk's per-call hash select (`mls >= 8` is outside the contract) | per-walk frame 48 -> 46 (cp), 66 -> 63 (ca.wc), 51 -> 48 (ca), cp.wc unchanged at 66; model unchanged at 292.0 (the cheapest site did not move) |
| 69 | P | the walk's per-call insert on raw table bases taken ONCE (the fill's brick 12): `lz_insert` re-derived the hash base, the chain base and the rows length from the tables pointer in the frame, and brick 61 derived the chain base a third time for the loop. One `as_mut_ptr` each at entry; the insert is the fill's body; the loop reads the same `chp` | model **292.0 -> 285.3**; per-walk frame 66 -> 58 (cp.wc), 46 -> 43 (cp), 63 -> 60 (ca.wc), 48 -> 45 (ca) |
| 70 | F | the fill's pair loop paid six instructions of overhead per pair (`lea p+2`, a reload of the spilled `stop`, `p + 3`, compare, copy, branch) because its bound was `p + 1 < stop` on the `p` that addresses every load and store. A countdown of the remaining positions carries the bound in the counter | packed pair loop **38 -> 36** per two bytes (18 per byte, from 29 at the session's start), its stack reload 1 -> 0; tag-array 38 -> 38, no-tags 23 -> 22 |
| 71 | P | the walk's per-call row mirror asked `rows.head.is_empty()` through the tables pointer (a reload and a compare at offset 160) for a per-block fact; `ChainCtx` carries it | model **285.3 -> 284.7**; per-walk frame 58 -> 57 (cp.wc) |
| -- | -- | **Second goal (2026-09-09): ten more.** Same gates, same score. Where they hide: the dominant candidate path is 72% of the modelled search cost, so its 31 instructions come first; the greedy (L5) finder's inline walk still has every shape the lazy walk shed (40 instructions, 11 reads on its dominant path); the fill's loop overhead; the emit path | |
| 74 | K | tagged NULL links. A link of 0 is both "no link" and position 0, so a chain that ends sends the walk to m = 0 and the walk exempted position 0 from the tag test (`m != 0`, two instructions on every candidate) because that link carried no real tag -- and the phantom cannot simply be dropped (`phantoms`: three accepts per level). Now every writer of an empty head's link (the fill, the walk's insert, `lz_insert`, priming, the wide-chain re-insert) writes `tag0 << 24`, position 0's own tag under the block's producer (`MatchTables::set_null_tag`), so the phantom is tag-tested like any other candidate: a sound tag rejects exactly what its first-word compare would have rejected, and both arms leave the walk in the same state. Packed shape only: the tag-array version (74a-d) cost its fill 38 -> 45..48 per pair for a walk that barely moved, so that shape keeps its guard and its writers | pre path **31 -> 28** (cp.wc and cp), tag-skip 22 -> 18; model **284.7 -> 271.8**; packed fill unchanged at 36 per pair (the select folded into the decode's `cmov`); greedy's walk 40 -> 39 |
| -- | K | **The greedy (L5) finder's inline walk**, priced the same way (`gwalk.py`: its dominant path, tag-skip path and per-walk frame; the sum below is 0.332 candidates + 0.155 skips + 0.25 walks per byte at L5 over the four shaped instances). Before this round: one instance, dominant path **40 / 11 reads**, frame 49; the sum **125.0** |
| 79 | K | the greedy finder monomorphised over its kernel shape (`find_greedy_impl::<MLS, KIND>`, brick 58's dispatch): its `cp`, `ca`, `tag_filter` and `walk_cont` were RUNTIME bools, so the dominant path tested three of them per candidate (`cmpb $0, slot`, branch) and selected the link decode on the fourth | dominant path **39 -> 35 / 32 / 37 / 39** (cp, ca, cp.wc, ca.wc), reads 11 -> 8; sum **125.0 -> 118.4**; `find_greedy` 1376 -> 5460 (five instances, one hot per block) |
| 75 | K | brick 59 for the greedy walk: one length (`best_ml` born `mls - 1`, `ml > best_ml`, unconditional `pre_eq`) | dominant paths **35/32/37/39 -> 32/29/30/37**; sum 118.4 -> 115.8 |
| 76 | K | brick 62 for the greedy walk: an explicit attempt countdown | reads on the dominant path 7 -> 5 on every instance; sum 115.8 -> 114.9 |
| 77 | K | brick 63 for the greedy walk: the two miss arms write different values | one instance's dominant path 39 -> 38; sum 114.9 -> 114.6 (the weakest of the round, recorded as such) |
| 78 | P | brick 65 for the greedy walk: the lower bound as one compare | per-walk frames -3 on every instance (80 -> 77, 79 -> 74, 89 -> 86, 85 -> 82); sum **114.6 -> 111.1** |
| 80 | F | four positions per trip in the fill's stride-1 loop (the pair loop stays for the remainder) | packed **36 per 2 -> 69 per 4** (18 -> 17.25 per byte; 29 at the session's start), tag-array 38 per 2 -> 71 per 4 (19 -> 17.75), no tags 22 per 2 -> 40 per 4 (11 -> 10) |
| 82 | P | bricks 68 and 49 for the greedy finder: the dead `wide_h && ip + 8 <= src_len` hash arm out of the per-position select, and the entry guard's `ip + mls <= src_len` (the loop's own invariant) out of the per-position path | per-walk frames (cycle through the walk minus the skip path) **49/49/56/54 -> 38/39/51/51**; greedy sum 111.1 -> **105.9** |
| 85 | P | the lazy finder's emit path refreshed `rep_bar` through `rep_bar_for`, whose `rep1 != 0` arm is dead there (a match offset is never 0): LLVM's branchless helper was seven instructions. At the emit site it is the add and one select | lazy emit cycle (position loop, through the literal push and the sequence store) **77 -> 73** |
| 86 | P | the lazy finder's rep probe chose its 8-byte or 4-byte compare with `at + 8 <= block_end` (`lea 9(ip)`, a compare against `block_end` reloaded from the frame) on every probing position. The loop holds `ilimit = block_end - 8`, and `ip + 9 <= block_end` is `ip < ilimit`: one compare on operands the latch reads anyway | inlined position loops, rep-probe path **79 -> 78, 81 -> 78, 94 -> 91**; no-match paths 52 -> 51, 78 -> 75; lazy model 271.5 -> **271.2** |
| -- | -- | **Third goal (2026-09-09): five more.** Same gates, same scores | |
| 87 | K | brick 74's guard for the greedy walk: since brick 79 its `cp` is a const per instance, so `(cp \| m != 0)` folds away on the packed instances (74 had left greedy alone because `cp` was a runtime bool there and the form would have ADDED a test) | greedy dominant paths **30/29/38/32 -> 29/30/33/31**; greedy sum 105.9 -> 104.5 |
| 88 | F | the fill's dead 8-byte-hash arm (`mls >= 8`, outside the contract): the per-call test and three loop bodies | `lz_fill_range` (packed) **798 -> 387** instructions, per-call prologue 41 -> 33; packed quad loop 69 -> 68; tag-array quad 71 -> 74 (its arm re-allocated; at 0.06 calls per byte the prologue saving covers it), no-tags 40 -> 40 |
| 91 | P | brick 86 at `try_rep1` itself: the width test `at + 8 <= block_end` (`lea 9(ip)`, a compare against a reloaded `block_end`) is `ip < ilimit`, and `ilimit` is a parameter every caller passes | `lea 9(` sites: greedy **5 -> 0**, dfast 4 -> 2; greedy per-walk frames 43/45/49/47 -> 38/39/50/47, sum 104.5 -> **102.8** |
| 95 | P | the lazy finder's no-match step `((ip - anchor) >> sh) + 1` (copy, subtract, the shift count from the frame, shift, add, increment) with the `+ 1` folded into the anchor: `(ip - anchor + 2^sh) >> sh` is the same value for every span a block holds, and `anchor_adj = anchor - 2^sh` (wrapping) moves only at a match | smallest inlined no-match cycle **45 -> 44** (the `inc` gone); lazy model 264.7 -> **264.4** (the board's pre_eq feature now prices the byte-load form; on it the session's start is 374.7 and the state before this brick 264.7) |
| 99 | K | the tree ladder's kernel (`bt_find_best_runtime`, ~2 calls per byte at L13-L15) selected its hash with `wide_hash && ip + 8 <= src_len` on every call, for `mls >= 8` which the contract rules out; the three `BtCtx` builders (priming, bt-lazy, bt-opt) now clamp `mls` to 3..=7 and the select is gone | entry-to-walk prologue **65 -> 62** per call, kernel 243 -> 234; walk cycle 42 -> 42 |
| -- | -- | **Fourth goal (2026-09-09): "is there more to win here?" -- the fill re-priced.** The packed quad loop's 17 per inserted byte were: tag 4 (`and smask`, `imulq`, `shrq 32`, `andl`), link decode 2 (`sub 1`, `cmovb`), hash 2, load/stores/mask/lea 8, loop 1 | |
| 100 | F | the chain ladder's link tag is the LAST BYTE of the `mls`-byte gram (`src[pos + mls - 1]`, `link_tag`) instead of the top byte of the masked gram times a 64-bit prime -- sound (a function of the bytes an accept verifies), so byte-identical by `taggate`'s argument; and where the product's top byte rejected 255 in 256 of the same-4-bytes-differ-at-byte-`mls` mates, the byte rejects all of them (census, corpus pass: skips 10.34M -> 10.27M at L5, 8.85M -> 8.79M at L7, 10.03M -> 9.97M at L9, false skips 0 -- 0.6% fewer skips, ~61K more loads per level). Every producer takes the one function: both fills, the greedy goal, both kernels, the primer, the wide re-insert, the null tag | packed fill quad **68 -> 56** (17 -> 14 per inserted byte; the `imull` now takes its load as a memory operand), tag-array quad 74 -> 56, row fill 21 -> 18, `lz_fill_range` 387 -> 345; kernel prologues cp 75 -> 72, ca 77 -> 76, none 67 -> 66; greedy no-match cycles **49/48/49/48/57 -> 45/44/46/44/50**; lazy walk paths unchanged (28/28/25/29), frames 58/38/57/43 -> 58/37/57/42, model 264.4 -> 263.8 |

**Refuted on the path, reverted (the numbers stay so nobody retries them):**

- **40 (K): carry the acceptance bar only, derive `best_ml` after the loop.**
  Aimed at the phi round trip on the miss arms. The `bar == mls` tests kept
  `mls` live and `pre_eq`'s `bar - 1` added an op: candidate path 28 -> 28
  (reads 2 -> 3) on the walk_cont kernels, and **28 -> 31 / 28 -> 30 /
  23 -> 26** on the three without walk_cont.
- **42 (K): `missed_before` as a count identity** (`seen > passes` at the
  accept, no flag write on the miss arms). LLVM turned the named attempt
  counter into an up-counter with a `lea`/`cmp $1` test and re-derived
  `attempts - left`, and the extra live values spilled: candidate path
  **28 -> 33**, tag-skip path 22 -> 23. The one-instruction flag write is
  cheaper than any identity that needs the counter observable.
- **44 (K): the search position's first word loaded once per walk** (an
  invariant `src[ip..ip+8]` reloaded per candidate because the `wcls` store
  hides `src` from LLVM's alias analysis). One more live value, and it
  spilled: candidate path **28 -> 29** (reads 2 -> 4) on cp.wc, **23 -> 28**
  on ca.wc; only the untagged kernels gained (20 -> 19, 23 -> 22).
- **45 (K): the advance duplicated on both miss arms** so they reach the
  header without the accept arm's phi join (the greedy walk carries exactly
  this on its tag arm). LLVM rebuilt the loop with three latches and spilled
  across them: tag-skip path **22 -> 40**, candidate path 28 -> 40, 9 stack
  reads and 2 stores where there were 1 and 0.
- **51 (P): the count offsets applied inside the outlined counter**
  (`count_match_raw8/16`, so the walk passes `m`/`ip` as they sit). One of
  the two hoisted `lea`s left the prologue; the other stayed, the prologue
  moved 82 -> 81 / 75 -> 76 / 88 -> 87 / 81 -> 82, and ca.wc grew by 73
  instructions of duplicated cold count code. Neutral on every path; brick
  56 takes the same two values off the loop without the duplication.
- **52 (F): the tag as the product's TOP byte** (`tv >> 56`, two seat
  instructions instead of four, five producers changed together, sound by
  the `taggate` argument). Fill **22 -> 20/21** per byte and prologue 81 ->
  79 -- but the kernel loop re-allocated around the new `gtag` and the cp.wc
  candidate path went 27 -> 28 with a spill store. At L9's rates the two
  cancel (+0.1 per byte). Parked, to retry once the loop is stable.
- **53 (K): the link carried XOR'd with the goal tag** so the tag test is one
  masked `test` instead of copy/shift/compare. LLVM carried BOTH forms:
  tag-skip path **21 -> 39**, candidate path 27 -> 39, reads 1 -> 3.
- **54 (P): the dead `wide_hash` arm out of the kernel's hash select**
  (`mls >= 8` is outside the contract). Prologue 82 -> 80 and kernel -21,
  but the ca candidate path went **27 -> 30** (reads 3 -> 5) and cp.wc's
  27 -> 28: at the unit rates a net +7 per byte. The arm is dead; removing
  it re-shuffles a loop that is one register short. Retry with 55 once the
  loop has slack.
- **55 (P): the kernel's two entry tests folded to one** (`usize::MAX` for an
  empty head, caught by `m < ip`). Prologue 80 -> 77 / 76 -> 73 / 69 -> 65 --
  and every candidate path +1 (cp.wc 28 -> 29, cp 28 -> 29, ca.wc 22 -> 23).
  Same story as 54; same retry condition.
- **57 (P): ONE call site for the search and its look-ahead** (the first
  iteration is the search, `stop = min(ip + depth, ilimit)` bounds the rest).
  The merged loop's phis for seven values are materialised on every entry:
  no-match path **18 -> 34** with four spill stores, for the look-ahead step
  24 -> 15. zstd's `_lazy` has three inlined searches for a reason.
- **60 (K): `ip` out of the walk's loop** (the search word and a pointer to
  `src[ip + best_ml]` carried instead, `room = block_end - ip` for the accept
  test). The pre path -1 (36 -> 35) but the per-walk frame +3..+4 for the
  two extra invariants, and ca.wc's pre path 33 -> 35: model **327.8 ->
  335.7**. The standalone (pointer) kernels LIKED it (386.6 -> 358.0), which
  is the clearest statement yet that the two forms allocate differently.
- **67 (P): brick 55 retried inlined** (one entry guard, the empty head as
  `usize::MAX`). Model 292.0 -> **295.6**; the allocator re-shuffled the walk
  again. Twice refuted, in both forms; not retried a third time.
- **72 (F): the head word's operands swapped** (`tag << 24 | p + 1`) so the
  `or` could land in the dead tag register. Pair loop 36 -> 36; LLVM had
  already chosen. Dropped as neutral.
- **73 (K): the long count outside the walk loop** (a hot inner loop with no
  call, breaking to a cold block for the count, re-entering) -- aimed at the
  seven per-candidate invariant reloads, which exist because a call inside
  the loop confines every value live across it to Win64's six usable
  callee-saved registers. The two-loop shape did what brick 57's did: model
  284.7 -> **375.7**, loop unions 66 -> 108 with 6 spills. (The first draft's
  macro `break`s also targeted the wrong loop; the shape was refuted on the
  count before the gate could refute the semantics.) The diagnosis stands;
  the fix is not a second loop.
- **81 (K): bricks 61/69 for the greedy walk** (raw bases for the insert and
  the walk's link reads). Dominant paths -1 on all four instances, but the
  per-position frames +9 in total (four base loads per position against
  `lz_insert`'s two): greedy sum 111.1 -> 111.3. Neutral; not kept.
- **83 (P): brick 71 for the greedy walk** (the row mirror behind a block
  bool). Same instructions, one more read per position: the tables pointer
  is live there anyway for the insert. Not kept.
- **84 (P): brick 38 for the greedy finder** (`rep_bar`). `find_greedy` +104
  instructions, one instance's dominant path 37 -> 39, frames +6 in total:
  greedy sum 107.3 -> 109.7. The greedy position loop allocates differently
  from the lazy one; refuted on its numbers.
- **89 (K): the long count as ONE outlined call** (`walk_count8` taking the
  counter's body instead of calling `count_match_raw`). The callee lost a
  frame; the CALLER's per-walk frames grew +2..+4 (LLVM re-allocated around
  the changed callee): lazy model 271.2 -> **275.2**. Not kept.
- **90 (P): brick 55's one-guard fold for the greedy finder.** No-match paths
  **+6..+10** on every instance. Third refutation of the same fold, in a
  third allocation; retired.
- **94 (K): brick 53 retried inlined** (the link carried XOR'd with the goal
  tag, one unsigned compare). LLVM moved the tag test ahead of the link load
  and the dominant path went 28 -> 30 / 28 -> 29, tag-skip 18 -> 21: lazy
  model 271.2 -> 278.8. Second refutation, second allocation; retired.
- **96 (K): the goal byte `src[ip + best_ml]` carried instead of re-addressed.**
  One byte in a register cost the loop its allocation: dominant path 28 ->
  46 with three spill stores. Retired.
- **97 (P): brick 95 for the greedy finder.** Greedy no-match cycles +1/-1/+1/-1,
  frames +2/-1/+1/+1: neutral. Not kept.
- **98 (P): brick 95 for the bt-lazy finder.** Its tight cycle 18 -> 17, but its
  two larger position cycles +2 with +3 reads. Mixed; not kept.
- **100 -> 100b/c/d, THE LEAK (found by the curiosity discipline, not by the
  gates).** The byte tag's first form loaded the byte in the kernel too, and
  the lazy walk's dominant path read **28 -> 29** (packed) / 25 -> 27 (tag
  array), model 264.4 -> 273.2 -- for a change of one instruction OUTSIDE the
  loop. The two paths dumped side by side: the product tag's `load_u64le(src,
  ip)` had been CSE'd with `mls_xor`'s hoisted goal word and lived in a frame
  slot, one load per candidate; without the tag's load LLVM rematerialised the
  compare's word from `src[ip]` on every candidate and added a register copy.
  `link_tag_from(v, mls)` = `(v >> 8(mls-1)) as u8` -- the same byte, from the
  shared word -- restored every path (261.7); but the same form in the fills'
  wide arm cost a variable shift per position (wide quad 64 -> 76), in the
  greedy finder gave back 4 per no-match position, and on the kernels' wide arm
  moved the tag-array shapes +1 (265.4). Landed: the kernels' hash4 arm from
  the word, everything else the byte load (263.8, every path at its brick-99
  value). One value, three forms, chosen per site by the emitted path.
- **101 (F): the fill's head word with the tag as the OR's first operand**, so
  the tied destination would be the dead tag register rather than the `p + 1`
  the next chain index needs (one copy per inserted byte). Quad 56 -> 56,
  byte-identical asm: LLVM canonicalises commutative operands, so source
  order cannot steer the tied register. Neutral; not kept.
- **102 (F): the source slice in `FillCtx`**, so the fill's signature is four
  registers and nothing rides the stack. It IS a structural saving -- call
  site 7 -> 4 (no `lea`, no two stack stores, no `len` load), callee prologue
  34 -> 33, i.e. ~5 per match; the greedy's no-match cycle 45 -> 43; the
  packed lazy frames 58/37 -> 56/35. But the tag-array shapes re-rolled: `ca`
  pre 29 -> 30, frame 42 -> 46, `ca.wc`'s second site +1. The sum: packed
  shapes -0.6 -0.6, fill -0.6 (0.125 calls/byte), `ca` +3.0 -> lazy model
  263.8 -> 265.6, **+1.2/byte with the fill counted**. Refuted under the sum
  rule; on packed frames (< 16 MiB) alone it reads -1.8/byte. Recorded with
  both numbers: `brick102.py` re-applies it in one line if the tag-array
  shapes' weight is judged differently.
- **103 (P/K): each lazy `KIND` body its own `#[inline(never)]` symbol**, on
  the theory that five bodies sharing one allocation is why every edit
  re-rolls all four shapes. Outlined (five symbols, +775 instructions of
  duplicated finder code): cp.wc frame 58 -> 57, cp 37 -> 38 and its loop
  48 -> 52, ca.wc 57 -> 60 with pre 25 -> 26, ca unchanged; model 263.8 ->
  **266.5**. The pressure is intrinsic to each body (Win64's six usable
  callee-saved registers against a loop that wants seven invariants), not
  to their union. Not kept; the board tools now read either layout
  (`cfg.merged`).

Five source shapes (40, 42, 44, 45, and 35's freed register) have now failed to
move the chain walk's candidate path below 27-28. Of its instructions, 18 are
the algorithm (bounds, link, tag, first word, next), 3 are semantics
(`m != 0`, the walk_cont flag), and 6-7 are the register allocator's: a chain
base or `smask` reload, a copy in the xor, a phi round trip on `best_ml`, a
copy of `m`. They resist source-level shaping because every hoist adds a live
value to a loop that is already one register short.

Running score against the goal of ten path wins per target, re-based on the
dominant path and the per-byte model: **K 13** (58, 59, 61, 62, 63, 74; on the
L5 walk 79, 75, 76, 77, 87; on the tree kernel 99), **F 8** (36, 39, 43, 66,
70, 80, 88, 100), **P 18** (37, 38, 41, 47, 48, 49, 50, 64, 65, 68, 69, 71, 78,
82, 85, 86, 91, 95). Second goal (ten more): done. Third goal (five more):
done (87, 88, 91, 95, 99). Fourth goal ("is there more to win here?"): **100**;
101-103 refuted, the fill at its floor under the current link representation
and call ABI. The lazy model: 374.7 instrs/byte at the session's start, **263.8** now
(30% fewer); the greedy sum 125.0 -> 102.8 and its no-match cycles 49 -> 45; the
packed fill 29 -> **14** per inserted byte; the tree kernel's per-call prologue
123 -> 62 across the campaign.

What the kernel loop's refutations established still holds for the miss path;
for the path that runs, the lever was the CALL, and brick 58 took it. The
inlined loops carry 8-11 stack reads per candidate where the standalone
kernels carried 2-6: the finder's frame is where the walk's invariants live
now, and every one the allocator cannot keep in a register is a load per
candidate. That is the next target, and it is countable.

The model's history, one number per landed state (`score.py`, the L9 search, four
shipping shapes; the pointer form until brick 58, the inlined form after):

| state | instrs/byte | note |
|---|---:|---|
| session start | 374.7 | position loop then 27, priced as 18 here |
| after 35 | 400.5 | the contract brick, on the path that runs |
| after 50 | 375.0 | |
| after 56 | 396.0 | the outlined long count |
| after 58 | 338.9 | the walk inlined |
| after 59 | 327.8 | one length |
| after 61 | 319.7 | raw chain base in the loop |
| after 62 | 311.5 | countdown |
| after 63 | 306.5 | no hoisted flag store |
| after 64 | 301.8 | `wcls` direct |
| after 65 | 292.5 | one-compare lower bound |
| after 66 | 292.0 | top-byte tag |
| after 69 | 285.3 | raw-base insert |
| after 71 | 284.7 | rows bool |
| after 74 | 271.8 | tagged null links (packed) |
| after 86 | **271.2** | second goal's last brick |
Instruments: `verdict3.py` (the three-target board: per-kernel tag-skip and
examined-candidate paths, the four fill bodies' per-byte loops, the lazy
no-match path with and without the rep probe, each as instrs / stack reads /
stack stores), `phantoms` (bench example: the walk's position-0 candidate
census, see brick 46 below when it lands).
### Diagnosis -- why the matchfind bricks do not close the gap (2026-09-09)

Twenty-three bricks priced UNITS of work. `mfbudget` counts how many units run per
input byte, and zstd 1.5.7's finders were compiled to assembly with the same LLVM
(`third_party` source, `clang -O3`) and priced with the same path tool. Both halves
are deterministic; no clock was used.

**Units per byte match C's algorithm.** L9: 0.30 kernel walks, 1.7 candidates
examined, 0.15 tag-skipped links, 0.69 fill inserts, 0.04 count calls per byte;
51-63% of walks exhaust their attempts budget, as C's would. L13+: one tree walk
per byte at 9-11 nodes, as C's `ZSTD_updateTree` does. Skips and routing are not
the problem.

**Unit PRICES are the gap, like-for-like (same compiler, same flags):**

| unit (lazy ladder, mls 5) | zstd 1.5.7 HC | this crate (packed tags) |
|---|---:|---:|
| per candidate, first-word mismatch | **14** instrs / 2 reads | 22 / 0 |
| per inserted byte (fill) | **11** (unrolled x2) | 29 (14-19 with tags off) |
| per position (block loop) | **40** / 10 reads | 65 / 22 reads / 9 stores |
| search kernel | 329 | 315 |
| dfast per position | 86 / 18 | **55** / 18 |
| fast per position | 105 / 23 | **40** / 13 |

Modelled at L9: **~96 instructions per byte here against ~60 for C's chain finder**
-- examine 37 vs 24, insert 20 vs 11, position+call 33 vs 22. L1-L4 are at parity
or better in instruction terms.

**Two things the instruction count cannot see, and both point the same way:**

1. zstd 1.5.7 runs the ROW finder by default for L5-L12 on x86-64 (SSE2 tag scan
   of a 16/32-entry row, one dependent load per ROW). This crate's row finder is
   gated to Lazy/Lazy2 in 512 KiB..2 MiB by the size crossover recorded above:
   beyond 4 MiB it saves **9-11x dependent loads at L12** for 0.5-1.6% size. C took
   that trade; we did not. That is the routing decision that matters.
2. The chain-link TAGS reject only ~10% of links (0.5-0.7 per walk of 5-19) while
   costing a second multiply on every inserted byte and a decode on every link.
   `taggate`: tags OFF is **byte-identical at L5/L7/L9/L12** (the walk budget counts
   links the same way). In instruction terms OFF wins ~13/byte at L9; the only value
   ON can have is skipping the candidate load on the 10% it rejects, which is a
   memory-latency question that needs a quiet box and a clock. Parked with the
   knob (`set_chain_tag_arm(false)`), not flipped.

Instruments: `mfbudget` (units/byte + modelled budget), `taggate`, and the C asm
census in `tools/asmcensus/` now accepts plain C symbols.
### Changed -- matchfind: twenty-three deterministic bricks, fifteen refutations, all byte-identical

Every verdict below is an EMITTED-ASSEMBLY count or a deterministic profile
counter -- this box sits at ~78% CPU with same-arm nulls of 7-18%, so no
clock was admissible and none was used. Every brick is byte-identical: GOLD
`2F6594F7EEDBD12B` / 59,680,638 bytes before and after each one, 173 tests
(174 minus one phantom test retired below).

| # | brick | verdict (static instrs unless noted) |
|---|---|---|
| 1 | `mls_eq`'s `mls > 8` tail outlined cold | greedy 1516->1494, chain 354->332; walk-path guards **6 -> 0**; `memcmp` copies 6 -> 1 |
| 2 | `count_match` on raw pointers, FOUR register args | 105 -> 68, guards **4 -> 0**; stack-arg stores at 36 call sites **30 -> 0** |
| 3 | every knob's env-resolve arm behind a cold helper (52 sites, 3 helpers) | lazy **1681 -> 1472**, bt 1016 -> 834, dfast 1446 -> 1368, opt 1971 -> 1939; `env::var`/`trim`/`from_str` sites in all finders -> 0 |
| 4 | the reserve lives in `chain_finder_prologue` | greedy 1494 -> 1400, bt 1123 -> 1033; lazy GAINS its block-0 reserve (it had none) |
| 5' | literal fast-path copy is constant-width again | `memcpy` call sites per finder **2 -> 0** (fast/dfast/greedy/lazy/emit); 16-byte vector stores +2 per site |
| 7 | dfast's default-off stride fill outlined | dfast 1378 -> 1344; L3 per-position loop **865 -> 815**, reloads 201 -> 192 |
| 8 | vestigial BT specialisation retired | bt_lazy 856 -> 791, opt 1954 -> 1890; 349 dead lines; 2 per-block knob reads -> 0 |
| 10 | per-matched-byte fill loops outlined | greedy **1430 -> 1205**, lazy **1467 -> 1135**; per byte 4 reloads -> 1-2 (chain) / 0 (row) |
| 11 | fused head in the chain walks (`mls_xor` + `fused_ml`) | DYNAMIC: 57-63.5% of accepted candidates resolve in the first word -> **-2.8M..-5.7M instrs per level** over 16 MiB (static: row -104, chain +28, greedy +5) |
| 14 | the chain kernel's ABI: `walk_cont` into `ChainCtx`, then the accept accumulator into `MatchTables` -- FIVE arguments to THREE, all in registers | chain **359 -> 336**, lazy 1135 -> 1126 -> 1086; the stack-argument store before each of the two indirect calls per position 1 -> 0, callee prologue 26 -> 23 instrs / 6 -> 4 spills |
| 19 | the fill loops take their block constants through one `&FillCtx` | stack-argument stores at the three per-MATCH fill calls **5/6/6 -> 2/2/2**; lazy 1126 -> 1097; the mid fill arms 31i/4r -> 27i/3r |
| 17 | `find_opt`'s jump-fill knobs are block constants, unconditionally: the per-jump re-read arm (an A/B hook, `set_opt_hoist_arm`, now a no-op) leaves the DP loop | DP inner loop **456 -> 354** instrs, 24 -> 19 spills, 94 -> 78 stack reads, static loads **9 -> 0** per jumped position; `find_opt` 1890 -> 1780 |
| 18 | LDM (`--long`): the per-candidate `src[m..m+mls] == src[ip..ip+mls]` -- a libc `memcmp` of 64 bytes -- was redundant with the `count_eq >= mls` that followed it; one masked 8-byte head test rejects the hash collisions and the count decides; the two checked `hash[h]` indexings go through the proven form | `find_sequences` 889 -> 858, `bcmp` **1 -> 0**, guards **2 -> 0**; over five corpora x L3/L9/L19 the head lets 0.2-45% of the 56K-108K candidates per file through to the count (`ldmgate --features profile`); `--long` output byte-identical (`ldmgate` GOLD `57BE83EA4E1199E8`, 57,796,847 bytes) |
| 20 | the L6-L12 chain kernel is specialised on its TAG REPRESENTATION (`CP` packed link+tag / `CA` tag array / neither): two per-block bools the walk re-tested on every candidate now fold, the block selects one of three instantiations through the fn pointer it already dispatches on | per-CANDIDATE paths in the packed walk (the one-shot shape): tag-mismatch **28 -> 22** instrs, stack reads **4 -> 2**; first-word reject **30 -> 25**, reads 4 -> 3; the tag-array shape 28 -> 20 / 4 -> 4; three kernels of 269/304/310 instrs replace one of 336 |
| 21 | the DEFAULT level's finder (dfast, L3-L4) is specialised on `packed` -- the tag representation reaches its probe, its insert, `dtag_on` and the after-match fill, and the per-position path tested it and kept the array-form state live | per-POSITION no-match path, packed body: **60 -> 55** instrs, stack reads **20 -> 18**, stack stores 3 -> 2; array body 60 -> 56; `find_dfast` 1344 -> 2336 (the second body), one body runs per block |
| 23 | the greedy finder's after-match fill takes the row-free loop (`lz_fill_range::<false>`, the lazy finder's, already in the binary) whenever the row table is off -- every input outside the 512K-2M row band, and every greedy block, since `row_auto_ok` only arms rows for Lazy/Lazy2 | per inserted BYTE: the rows fill's paths 38-42 instrs / 4-5 stack reads -> the row-free fill's **25-32 / 2-3**; same head/link/tag words, GOLD unchanged |
| 24 | `walk_cont` joins `CP`/`CA` as the chain kernel's third const axis -- the last per-block flag the tag-mismatch path reloaded and tested per candidate | packed-shape mismatch path **22 instrs, stack reads 2 -> 0** (the freed slot let the chain base take a register too); tag-array shape 20 / 4 -> **18 / 1**; no-tag shape 23 / 2 -> 19 / 0; six kernels of 264-323 instrs replace three of 269-310, one runs per block |
| 26 | the chain FILL loop (every byte of every match at L5-L12) is specialised on the tag representation for its row-free shape; the rare rows body keeps runtime flags behind a `SPEC` const so it costs no extra bodies | per inserted BYTE: packed shape **32 -> 29** instrs, stack reads 2 -> 0-2; tag-array 24-27 / 2-3 -> 23-25 / 1-2; no-tag 25 / 3 -> **14-19 / 0**; the per-byte `ca` flag test is gone from every shape; four bodies of 122/171/181/371 replace two of 338/371 |
| 27 | the binary-tree walk (L13-L15, and the opt levels' fill and priming) reads ONE child per node, after the write: the eager pair of child loads spilled both words and needed a forwarding compare to stay exact; reading after `chain_set` returns `m` exactly when that compare selected it | per-NODE path **55 -> 43** instrs, stack reads **8 -> 4**, stack stores **5 -> 1**; the walk loop 155 / 8 spills / 24 reads -> 133 / 2 / 18; `bt_find_best_runtime` 269 -> 245; byte-identical by the store-forwarding argument, GOLD unchanged |
| 31 | `prime_ldm` (the `--long` primer over a dictionary, prefix or retained window) indexed `hash[h]` checked on every primed position; `ldm_hash` masks to `hash_log` bits and the table is exactly that size, the proof brick 18 already used for `collect_ldm` | guards in the emitted crate **67 -> 66**; `--long` bytes unchanged |
| 33 | `match_ok` (dfast's per-candidate validity test) ran `ip + mls > len \|\| m + mls > len` BEFORE its 8-byte arm, where both are implied (`mls <= 8`, `ip + 8 <= len`, `m < ip` from the order check); they now guard only the cold tail, which is the one path that needs them | `find_dfast` **2345 -> 2319** across the five inlined per-candidate sites; the four guards left in `match_ok` are all in the cold tail; per-position paths 55/56 -> 54/55 |
| 34 | the binary-tree kernel's PROLOGUE was per-block work done per CALL -- `bt_log`, `bt_mask`, the worst-case `chain_len` guard and both hash shifts, all functions of `chain_log`/`hash_log`/`chain_len`. Into `BtCtx`, via `bt_geom`, built once per block at its three construction sites | entry-to-loop-header **123 -> 105** instrs PER CALL, and this kernel is called per position + per look-ahead step + per fill insert (61.9% of tree work at L13-L15); `bt_find_best_runtime` 245 -> 222, walk loop 133 -> 127 |
| 12 | raw-pointer insert on lazy's fill arm | small fill loops **22i/2r -> 14-16i/1r** per matched byte (greedy's arm measured WORSE, 22/1 -> 29/6, and keeps the method call) |

**Refuted, each on a count, recorded beside the code so it is not retried:**

- **Dropping `push_literals`' capacity check.** It is the function's soundness
  contract and its own test exercises `spare = 0`; the alternative is `unsafe`
  at 13 hot sites for ~3 instructions per push (~0.3%). Pruned on arithmetic.
- **Replacing the chain kernel's function pointer.** A direct branch removed
  both indirect calls but duplicated the five-argument marshalling in both
  arms (+18 per-position, +70 look-ahead); inlining the walk landed its 331
  instructions TWICE (1472 -> 1982, spills 10 -> 20). W24's pointer stands.
- **Removing the fast emitter's dead `packed` parameter.** 1843 -> 1843: LLVM's
  dead-argument elimination had already dropped it from these internal
  functions. The source ABI is not the machine ABI.
- **The fused head in dfast.** The counter that justified brick 11 refutes it
  here: dfast's long-hash candidates match eight bytes by construction, so only
  37% resolve in the first word and the net is **+43,877 instrs at L3,
  +103,109 at L4**. Same idea, opposite sign, decided by the resolution rate.

- **Const-generic split of the BT walk on its `search` flag.** `_inner` is
  `#[inline(always)]` and tests `search` at ONE site per accepted candidate;
  doubling a 269-instruction body to remove one predicted test is the I-cache
  trade the D-notes already refused. Pruned on arithmetic.
- **An `mls == 5` shape of the chain kernel** (every default row L5-L12 has
  min_match 5, so the runtime shape's `mls <= 8` test and reloaded byte mask
  looked foldable). Twelve kernels instead of six, total instructions down
  (211-305 against 264-323) -- and the per-candidate PATHS did not improve:
  23/1 and 30/2 against the runtime shape's 22/0 and 28/1. The same verdict
  the greedy split got, for the same reason: a loop total is the union of its
  arms, and the path is what a candidate pays. Reverted.
- **Hoisting `prime_tables`' chain-arm flags** (`chain_pack`, `ctags`,
  `chain_wide`, the plain-head test) out of the primed-position loop without
  outlining it -- brick 16 refuted the outlining, this kept the frame. The
  function fell 964 -> 924 and the loop 293 -> 287 with its one spill gone,
  but across the six shortest per-position paths instructions went 228 -> 224
  while stack reads went **56 -> 60**. Two counters disagreeing in sign is not
  a win, and the path is off plain `compress()` regardless. Reverted.
- **Folding `try_rep1`'s `rep1 == 0 || at < rep1` into one wrapping subtract**
  -- attempted, then found the guard was deliberately phrased as the caller's
  own loop condition by an earlier pass, precisely so LLVM deletes it at the
  six loop call sites. The assert-before-write caught it; nothing was changed.
- **Keeping `tables` out of the chain walk** by accumulating the accept
  classification in locals (the premise: its pointer pinned a register across
  every candidate while the chain base reloaded). After brick 24 the packed
  mismatch path already read nothing from the stack; the change moved no path
  and added 7 static instructions. Reverted -- the premise was stale by the
  time it was built. Re-census after every brick.
- **The greedy finder specialised on its tag representation** (brick 20's split,
  applied to the L5 finder that inlines its own walk): three bodies, 1211 ->
  2889 static, and the per-candidate walk path went **29 -> 30** instructions,
  stack reads **10 -> 11** (cp body) / 9 (array body). Folding two flags inside
  a 500-instruction per-position loop freed registers the allocator spent on
  other invariants; the kernel split paid because the kernel is 300
  instructions with nothing else live. Reverted on its own verdict.
- **A fused 8-byte head for the FAST finder** (brick 11's shape, third
  probe after chain/row/greedy kept it and dfast refused it). The count
  histogram split at n < 3 -- the exact population a head resolves at mls 5
  -- reads **31.1%** at L1 against the 37.5% break-even of the -5/+3 model,
  and the count is called on only 4.1% of positions there. Pruned on
  arithmetic; the instrument (`eqshare`, six buckets) stays.
- **The finders' census counters marshalled to their `#[inline(never)]`
  epilogues** (probes, hits, ...): a per-position increment each, on the
  face of it. The emitted call sites say LLVM's dead-argument elimination
  already strips the ones no shipping heuristic reads -- dfast's epilogue
  takes 14 stack arguments in the source and 11 in the binary -- so there
  is nothing to hoist. Pruned before building.
- **`emit_fast_seq`'s eleven-argument ABI** (7 on the Win64 stack per
  MATCH at L1/L2): bundling the four buffers behind one pointer would force
  their headers into memory across the per-position loop that reads them 18
  times a block. ~14 instructions per match at ~0.03 matches per position
  is under half an instruction per byte. Pruned on arithmetic.
- **Outlining `prime_tables`' chain-strategy loop** (the bricks 10/12 shape:
  own frame, invariants hoisted, mask-proven indexing). Inline, LLVM had
  unswitched the two shipping arms into their own loops -- dfast 42 instrs /
  12 reloads per position, lazy 50 / 15; outlined, the function IS the loop
  and LLVM no longer unswitches its eight invariant tests: 46 / 9 and 57 / 13,
  964 -> 671 + 318 static. Instructions up, reloads down: not a win. The path
  also never runs on plain `compress()` (dictionary/prefix and streaming
  slides only). Reverted.

- **Folding `push_literals`' runtime `arm` test into a const generic** for the
  three chain finders whose width is never zero: bt -3, but greedy **+20** and
  lazy **+6** static -- the armed monomorphisation re-laid out larger. Reverted.
- **Retiring the fast finder's BMI2 twin** on the density test that retired the
  dfast twin: PARKED, not decided. It converts 41 ops in 1,811 instructions,
  44 per op -- denser than every twin the D-notes retired (72-152) -- and its
  conversions execute per position (`shr r, cl` is two uops where `shrx` is one),
  so the 2,095-instruction static drop would RAISE executed uops. That is the
  static-count trap by name; only a clock can price it, and this box has none.

- **Word-at-a-time backward extension in the fast emitter.** `bextcount`: at
  L1 only 13.7% of matches extend backward at all -- 0.195 bytes per match --
  and 7-8% at L7/L9. A word form costs ~10 fixed instructions per match to
  save ~8 per extended byte, i.e. about +8 per match. Pruned on arithmetic.

**Correction to yesterday's note.** "`BT_SPEC_PAIRS` regenerated ... so the
specialisation keeps covering everything" was wrong: `bt_resolve` and
`bt_resolve_ins` returned the runtime body on every path, the 20 pairs fed
only a test, and `bt_find_best_impl_inner` had one caller and it was dead.
The test passed while selecting nothing. Brick 8 removed all of it.

**The rest of the matchfind surface, audited and left alone.** Named so the
next pass does not re-open them: `try_rep1` (already hoisted, above);
`fill_fast_after_match` (two conditional stores, no guards); `RowTable`'s
`insert`, `insert_h` and `insert_at` (raw-pointer throughout); `row_tag_mask`
(already SSE2/NEON with a scalar oracle); `probe_view` (raw array views);
`count_eq_len_words_raw` and `count_eq_len_avx2` (leaf kernels, no guards, no
calls); and the six per-BLOCK prologues and epilogues (`dfast_finder_prologue`
at 573 instructions is the largest) -- those run once per 256 KiB block, about
0.002 instructions per byte, so they are an arithmetic prune rather than a
target. One adjacent lead, deliberately out of scope here because it is the
emitter and not the finder: `write_literals`, 2,421 instructions with **32
`memcpy` call sites**.

**Instruments built for this** (all deterministic): a natural-loop census with
per-loop spill/reload/static-load counts and spill-slot provenance, a
per-symbol guard-branch and `memcpy`/`lock`/TLS census, `fusedcount` (first-
word resolution rate), `bextcount` (backward-extension histogram).
Second pass, now in-tree under `tools/asmcensus/`: a CFG-correct natural-loop
census (every jump inside a label-delimited region is an edge; dominance-checked
back edges; the first version reported phantom loops that swallowed the prologue),
a per-PATH cost tool (Dijkstra over a loop's blocks on executed instructions, with
call-free isolation of the no-match path -- the number every specialisation verdict
above is read from), hot-slot provenance per loop, and `ldmgate` (the `--long` byte
gate with candidate/count counters); `eqshare`'s histogram gained the `<3` bucket.
### Changed -- rusty_alloc 1.1.4 -> 2.0.0 in the deliverable seam

`rzstd-alloc` now pins `rusty_alloc-api = "=2.0.0"`, so the four CLI binaries
and the bench main run the 2.x allocator. Side-by-side against the 1.1.4
build, same source, same flags.

**No regression found.**

| check | result |
|---|---|
| compressed output | **byte-identical**, 72 (corpus x level) pairs |
| round-trip | 48/48 clean |
| cross-decode v1<->v2 | 6/6 clean |
| library tests | 174 pass / 177 with `profile` |
| peak RSS | flat: -0.7% .. +0.5% over 9 (corpus, level) cells |
| binary size | 760,832 -> 764,416 (+3,584, +0.47%) |
| encode speed | **not resolvable on this box** |

Byte-identity is the load-bearing check: an allocator that changed compressed
output would be a correctness defect, not a performance one.

**Speed is reported as UNRESOLVED, not as parity.** ABBA, min-of-3, six
(corpus, level) cells, with a same-arm null: treatment 0.968x .. 1.187x
against a null of 0.934x .. 1.089x -- treatment mean 1.029 +- 0.075 against
null mean 0.987 +- 0.053. The two distributions overlap almost entirely on a
box sitting at 94% CPU. Re-run on a quiet machine before claiming either way.

A first attempt at that timing was DISCARDED: driving each run through
PowerShell `Start-Process -Wait` added 250-500 ms of fixed overhead per
invocation, which swamped the work and produced a fake null of 0.999x. The
tell was `dickens` (10 MB) and `webster` (41 MB) both reading exactly 1,010 ms
at L3. Timed directly they are 503 ms and 786 ms.

### Known -- the two allocator paths are on different majors

`rusty_zstd`'s optional `rusty-alloc` feature installs through
`rusty_alloc_default`, which as of 0.1.2 still tracks the 1.x line (it moved
1.1.4 -> 1.1.6, not 2.0.0). So:

```text
  CLI + bench   -> rzstd-alloc        -> rusty_alloc-api 2.0.0 -> rusty_alloc 2.0.0
  rusty-alloc   -> rusty_alloc_default -> rusty_alloc-api 1.1.6 -> rusty_alloc 1.1.6
```

**No single binary links both** -- `cargo tree -p rusty_zstd-cli` shows only
the 2.0.0 chain -- so this is a workspace-lock artifact rather than a shipped
defect. But the lock now carries both lines, and the feature and the CLI are
on different majors until `rusty_alloc_default` publishes a 2.x. Recorded
rather than worked around.

`rusty_alloc 2.0.0` also adds `portable-atomic` and `libc` as target-
conditional dependencies; neither is linked on windows-msvc, where the tree
ends at `windows-sys`.

Also fixed: `rzstd-alloc`'s own doc comment claimed the pin was `=1.1.0` while
the manifest said `=1.1.4`. A stale version in the one comment whose whole job
is to state the pin.
### Refuted -- C's `sufficient_len` look-ahead exit does NOT pay on our lazy

Recorded in full so it is not rediscovered. `find_greedy_impl` and
`find_lazy_impl` never read `target_length`; only `find_opt` did. C's
`ZSTD_compressBlock_lazy_generic` does:

```c
    const U32 sufficient_len = MIN(cParams->targetLength, ZSTD_OPT_NUM - 1);
    if ((matchLength > sufficient_len) || (ip + matchLength >= iend))
        goto _storeSequence;   /* best possible: avoid search */
```

That is the same shape as the incompressible-section accel that DID pay 3-7x,
and `target_length` is 8 at L7 and 16 at L8-L12 against measured mean match
lengths of 8-35 -- so it looked reachable exactly where MatchFind is 94-96% of
encode. Built it, swept the cut as a multiplier in sixteenths (16 = C exactly):

```text
           mul 8            mul 16 (C)       mul 32
  L7    +2.366%  1.24x    +0.460%  1.00x   +0.217%  1.04x
  L9    +0.645%  1.14x    +0.268%  0.95x   +0.069%  0.94x
  L12   +0.256%  1.01x    +0.066%  1.09x   +0.010%  1.10x
  L13     +0%    1.03x      +0%    1.03x     +0%    1.03x   (BtLazy2, no path)
```

**C's own setting costs 0.46% of ratio at L7 and buys nothing measurable** --
1.00x against a null of -4.4%. The reason is structural: `search_log` is 3-4 on
our lazy rows, so `depth` is 1-2 and the look-ahead being skipped was never
expensive. Skipping it just loses the match it would have found.

**REVERTED, not kept behind a default-off knob.** This crate usually keeps a
refuted arm with its rationale, but that convention assumes the disabled form
is free, and here it was not: with the knob resolving to 0 the emitted
`find_lazy` still went **1,655 -> 1,788 instructions, +133**, because LLVM did
not fold the per-position `good_enough` test away. A refuted feature does not
get to tax the hottest loop in the encoder. Measured both ways rather than
assumed.

Also ruled out on the same pass, so the next reader does not re-chase them:

- **No memory leak.** Two allocations failed during these sweeps (4 MB and
  16 MB), which looked like unbounded growth. Sampling the working set across
  80 L19 compressions shows a sawtooth between ~5 and ~41 MB with peaks the
  same early and late -- no trend. The box had 2.4 GB free physical of 32 GB.
  A loaded machine and a leak look identical from outside; this one was the
  machine.
- Each L19 call still allocates and frees ~40 MB of tables. That is the case
  for a REUSABLE one-shot context (C's CLI reuses a CCtx across iterations,
  our `compress()` does not), which is an API question rather than a defect.
### Changed -- source-sized hash extended to EVERY strategy

**GOLD 7FB4E822473412A3 -> 2F6594F7EEDBD12B, 59,685,682 -> 59,680,638 bytes.**
On the identity board this is a **-5,044 byte GAIN**, not a cost: at that
board's 1 MiB cap the chain levels measure -2,522 each. The +0.1% the trade
was accepted for exists only at 64 KiB-256 KiB.

The strategy gate is gone. What licensed removing it:

```text
                  64K       256K      1M        4M      tables
  L1..L3 F/DFast   +0        +0        +0        +0      0%    never bites
  L4  DFast      +469 B      +0        +0        +0    -50%    (64K only)
  L5  Greedy     +384 B      +0        +0        +0    -33%
  L7  Lazy       +393 B   +1453 B      +0        +0    -33%
  L9  Lazy2      +402 B   +1207 B   -2522 B      +0    -33%
  L13 BtLazy2      +1 B    +144 B      +0        +0    -25..-33%
  L16/19/22        +0        +0        +0        +0    -25%
```

**Fast and DFast are untouched BY CONSTRUCTION** -- their `hash_log` already
sits at or below `src_log`, so the clamp never fires and L1-L3 read exactly
zero at every cap. Everything that does move costs at most +0.11%, only at the
two smallest caps, and is zero or negative from 1 MiB up.

The original finding is closed: table per input byte goes **12x -> 8x** on the
chain ladder (64K/256K/1M alike) and **16x -> 12x** at L12.

| input | L7/L9 before | after | L12 before | after |
|---|---:|---:|---:|---:|
| 64K | 12x | **8x** | 16x | **12x** |
| 256K | 12x | **8x** | 16x | **12x** |
| 1M | 12x | **8x** | 12x | **8x** |
### Changed -- the hash is now sized from the SOURCE, not the window

**GOLD D8F9B47AD5DDD2AB -> 7FB4E822473412A3, total 59,685,682 bytes BOTH
SIDES.** The anchor moves because individual frames shift; the sum does not
move at all. This is a memory change, not a ratio change.

C clamps `hashLog <= windowLog + 1` and we matched it. But once the window has
already been reduced to the source, two buckets per window position is two
buckets per BYTE OF INPUT -- 8 bytes of hash on top of the chain's inherent 4,
which is exactly the **12 bytes of table per input byte** measured earlier. A
1 MiB input at L9 allocated 12 MiB of table.

| input | L7 | L9 | L12 |
|---|---:|---:|---:|
| 64K | 12x input | 12x | 16x |
| 256K | 12x | 12x | 16x |
| 1M | 6x | 12x | 12x |

**GATED ON STRATEGY, because the cost is not uniform.** The tree finders reach
candidates through the binary tree, so extra hash buckets buy them almost
nothing; the chain finders resolve every collision by walking, so taking
buckets away lengthens their walks. Measured over 16 corpora:

```text
                  64K       256K      1M        tables
  L16 BtOpt       +0 B      +0 B      +0 B      -25%
  L19 BtUltra2    +0 B      +0 B      +0 B      -25%
  L22 BtUltra2    +0 B      +0 B      +0 B      -25%
  L13 BtLazy2     +1 B    +144 B      +0 B      -25..-33%
  L9  Lazy2     +402 B   +1207 B   -2522 B      -33%   <- NOT taken
  L7  Lazy      +393 B   +1453 B      +0 B      -33%   <- NOT taken
```

BtLazy2 and above is the band that is free or within 0.009%. The chain ladder
keeps C's sizing, so no ratio is sold for memory it does not need.

**Peak RSS**, sampled live (it reads 0 after exit): L16 at 1 MiB goes
**32.8 -> 28.7 MB, -12.5%**; L7/L9/L12 at 256 KiB fall 4.9-5.7%.

**REFUTED, and worth recording: this is NOT a speed win.** Cutting the table
33% moved total encode time by -1.8%, inside the noise, and the `EncodeTables`
share did not fall. The reason is mechanical -- `vec![0; n]` for a large `n`
takes zero pages from the OS rather than memsetting, so the cost scales with
pages TOUCHED, not pages allocated. Allocating less cuts committed memory and
RSS; it does not cut work. Anyone reading the earlier "tables are 30% of small-
input encode" line should not expect to win that 30% back by shrinking the
allocation.

**`BT_SPEC_PAIRS` regenerated.** Moving `hash_log` down one shifts the
reachable `(hash_log, chain_log)` set from `(h, h)` / `(h, h+1)` shapes to
`(h, h+1)` shapes. Left alone, every Bt level at every size would have fallen
through to the slow runtime body -- the exact regression
`bt_specialisation_covers_every_input_size` was written to catch, and it did.
Re-enumerated by `btpairs.rs` over every bt clevel x every input size x the
streaming case: still exactly TWENTY pairs, so the set moved without growing.

`RZSTD_HASH_TIGHT=0` / `set_hash_tight_arm(0)` restores C's sizing.
### Fixed -- the chain ladder walked incompressible data ONE BYTE AT A TIME

**GOLD 269F0EC2BA6B8550 -> D8F9B47AD5DDD2AB** (-491 bytes). Small on that
board because 14 of 18 corpora are byte-identical under the change; the win
here is SPEED, and it is large.

C's `ZSTD_compressBlock_lazy_generic` advances a failed position by
`((ip - anchor) >> kSearchStrength) + 1`, with the comment "jump faster over
incompressible sections". `find_greedy_impl`, `find_lazy_impl` and
`find_bt_lazy` all advanced by a bare `ip += 1`, so on content that cannot
match they walked every byte while C accelerated away. Same shape as the
repcode and back-extension defects: a capability present in one finder and
absent in its neighbour -- `find_fast`/`find_dfast` have had an accel shift
for levels.

**How it was found, and how it was separated from a noisy box.** The
head-to-head board read 11.5-12.2x slower than C on `incomp-32m` at L7/L9,
but that board's null arm was 10-17% because the machine was at 91% CPU. A
12x claim inside a 17% null is not a measurement -- so the question was
whether it was real at all. It was settled WITHOUT trusting the board, by an
internal comparison in ONE process where load cancels between the arms: L1
took 943 us with MatchFind at 11.3%, L9 took 26,918 us with MatchFind at
86.0%. **28.5x between our own two levels on the same bytes.** Load explains
+-17%, not +2750%.

The walk census read ZERO chain loads at L7/L9, which ruled out chain walking
and pointed at the per-position advance itself.

| level | before | after | speedup | MatchFind share |
|---|---:|---:|---:|---|
| L5 Greedy | 9,333 us | 2,830 us | **3.3x** | 87% -> 70% |
| L7 Lazy | 23,374 us | 3,256 us | **7.2x** | 88% -> 62% |
| L9 Lazy2 | 26,918 us | 4,864 us | **5.5x** | 86% -> 54% |
| L12 Lazy2 | 26,949 us | 5,015 us | **5.4x** | 85% -> 51% |

**Shift 12, not C's 8.** The shift trades size against skipped positions and
the two do not move together:

```text
  shift   L5 size   L7 size   L9 size   incomp speedup
    8      +2,383    +2,597    +2,990     3.2x .. 4.8x
   10        -254      -257      +155     3.1x .. 4.2x
   12        -235      -216       -15     2.8x .. 3.7x
```

12 is the only value SMALLER on every level at both caps tested, so it is a
strict win rather than a trade -- 4 MiB totals -655 / -365 / -193 / -42 at
L5/L7/L9/L12. 10 was rejected because `x-ray` regresses +1,581 there and is
byte-identical at 12. 144 cells round-tripped.

`RZSTD_LAZY_ACCEL` / `set_lazy_accel_arm(0)` restores the old step.

### Known -- table zeroing is now the top cost on small inputs

With the search fixed, the stage profile on 1 MiB of incompressible data puts
**`EncodeTables` at 30.1% (L9) and 30.9% (L12)** -- it was invisible under the
old search cost. `MatchTables::new` zeroes hash + chain sized from the level,
and for a small input that is far more memory than the input itself:

```text
  input   L7        L9        L12
   64K    12x       12x       16x     (of the input, zeroed before any work)
  256K    12x       12x       16x
    1M     6x       12x       12x
```

A 1 MiB input at L9 zeroes 12 MiB of table. Counted and left; the fix is to
size the tables from the source length rather than the level, and it wants its
own measurement pass.
### Changed -- the ROW match finder now defaults ON for small inputs
### (-147,812 bytes, -0.247% on the identity board)

SHIPPED. The row arm's own doc said it "ships on `examples/rowboard.rs` or not
at all" -- so this is that board, run across input SIZES instead of at one cap.

**GOLD EA4E12B951B48F4A -> F72C7074A2240AF7**, 59,852,335 -> 59,704,523 bytes.
A pure ratio gain: nothing was traded for it. `ROW_ARM` gains a third state,
AUTO (0), which is now the default; `set_row_arm` still FORCES either way and
`set_row_arm_auto` restores AUTO. Out of band the output is byte-identical to
the previous default, so only in-band cells of the identity table moved, and
all 174 tests pass -- including the C cross-compatibility suites, so the new
bitstream is still ordinary zstd.

The gate fires only for `Strategy::Lazy | Lazy2` with a KNOWN source length in
512 KiB..2 MiB. Streaming and the dictionary harvest pass `None` and keep the
chain, because the band is a source-length band and they do not know the
length. Verified cell by cell (`rowauto.rs`, every cell round-tripped):

```text
  L5  Greedy    off at every size          (not the lazy ladder)
  L7  Lazy      512K 0.9866  1M 0.9880  2M 0.9895   256K/3M/8M byte-identical
  L9  Lazy2     512K 0.9882  1M 0.9882  2M 0.9894   256K/3M/8M byte-identical
  L12 Lazy2     512K 0.9959  1M 0.9968  2M 0.9992   256K/3M/8M byte-identical
  L13 BtLazy2   off at every size
```

Every ON cell is a win and every OFF cell is unchanged, so the gate is
STRICTLY non-regressing on the board.

`rowboard` caps every corpus at 8 MiB and reports L9 aggregate **1.0005x** --
a wash, which is why the arm has stayed `Defaults OFF`. Swept across caps, the
verdict is monotone in input size and 8 MiB is just past the crossover:

```text
  L9   cap    size ratio   dependent loads saved
       256K     1.0059          6.84x
       512K     0.9882          2.47x
        1M      0.9882          3.08x
        2M      0.9894          3.32x
        4M      0.9944          3.42x
        6M      1.0018          3.73x
        8M      1.0005   <- the only cap rowboard measures
```

So in **512 KiB .. 4 MiB** the row finder is smaller AND does far less work --
both currencies moving the right way at once, which is precisely the case the
rowboard header says it cannot assume ("A row finder that costs size must EARN
it in speed"). It does not cost size there; it saves it.

| level | win band | size ratio | dependent loads saved |
|---|---|---:|---:|
| L7 Lazy | 512K..4M | 0.9866 .. 0.9936 | 1.96x .. 2.57x |
| L9 Lazy2 | 512K..4M | 0.9882 .. 0.9944 | 2.47x .. 3.42x |
| L12 Lazy2 | 512K..2M | 0.9959 .. 0.9992 | 5.63x .. 8.53x |

336 cells, every one round-tripped. Beyond the crossover L12 still saves 9-11x
loads while costing 0.5-1.6% size -- the size-for-speed case rowboard was
written for, and a separate decision.

**The mechanism, and why it has a crossover.** A row holds the last 16
positions for its bucket where the chain held all of them linked. That trades
DEPTH for RECENCY -- and recent means SMALL OFFSETS, which cost fewer bits.
While the chain is shallow the row gives up almost no depth and banks the
offset saving; once the window fills, the chain's extra depth finds matches the
row cannot. Same offset-cost mechanism as the `nl_dispatch` result above: a
finder can win MATCH LENGTH and still lose COMPRESSED BYTES, and any signal
built on length alone is blind to it.

**The dispatch signal is size, not content.** Three content signals were tested
against the per-corpus win/loss split and ALL THREE overlap -- literal share
wins [0.029, 0.474] vs losses [0.136, 0.568]; mean match length wins
[8.80, 35.34] vs losses [5.30, 20.89]; sequences/KiB wins [27.5, 103.2] vs
losses [39.0, 138.5]. No empty interval, so no content gate. The separating
variable is how full the window is, which the encoder already knows.

Row memory is not the objection: `RowTable::reset` sizes `1 << hash_log`
entries and 16 buckets share a row, "which is what keeps this table the same
size as the chain".

### Also found on the same board

- **`lazy_gain` ON is -7,111 B at L9** (and +140 at L7, 0 at L13) -- level-
  dependent, so it is a dispatch question rather than a default flip.
- Defaults CONFIRMED correct for `lazy_fill` (off costs 101K-163K), `walk_cont`
  (off costs 5K-12K) and `wide_chain` (off costs 0.3K-0.8K). These had never
  been boarded at all -- `allgates` could not see their levels until today.

### Two more harness defects, both caught by disagreeing instruments

- **`take_row_census()` returns `(ROW_EXAM, ROW_LOADS)` -- candidates first.**
  Reading `.0` as "loads" compares chain LOADS against row CANDIDATES, two
  different units, and reports a flat 1.00x saving. `rowboard` destructures it
  correctly; a fresh harness did not, and the 3.81x claim looked refuted until
  the field order was checked.
- **These arms are THREE-state and have no public "unset".** `set_*(true|false)`
  FORCES; the untouched state resolves through an env knob or a dispatch. A
  board that sets arm A and then measures arm B has no valid baseline: the
  first attempt produced an identical **+10,462 B at L13 for thirteen
  unrelated arms** -- one stuck forced arm, read thirteen times as if it were
  each arm's own result. The fix is one arm per PROCESS (`armone.rs`).
### Changed -- the next-long OFFSET-TRADE dispatch now defaults ON (-0.36% at L3)

SHIPPED. **GOLD F72C7074A2240AF7 -> 269F0EC2BA6B8550**, 59,704,523 ->
59,686,173 bytes. `NL_DISPATCH_ON` now initialises to 2 and
`dfast_good_ml_raised()` returns 48 instead of 24; `set_nl_dispatch_arm(false)`
restores the old behaviour.

The bytegate delta (-18,350 B) is small because that board spans nine levels
and only one of them is DFast. On an 18-corpus L3-only board the same change
is **-82,653 bytes, -0.360%**, and -0.27% to -0.30% across 2/4/8 MiB caps at
both L3 and L4, with 72/72 round-trips.

It required widening the adjudicated L3->L5 ladder tie from 0.1% to 0.2%. That
is the same event the exception was written for: on full osdb L3 goes
3,517,111 -> 3,514,780 while **L5 stays at 3,519,696 -- the exact value the
test's own doc already recorded** -- so the cheaper level gained, Greedy did
not lose, and `L5_CEILING` (3,530,000) is untouched. The historical DEFECT
inversions on this pair were +1.25% and +0.33%, both still above the new bar.

**Why size and not speed.** This box's paired timing harness has a +-1.5% null
band (`eqlever.rs`, ABBA, pinned, min-of-15). Compressed bytes have no null
band at all -- same input, same arm, same number, any machine, any load. So
size is the only currency in which a sub-1% effect is decidable here.

`sizehunt.rs` swept the arms reaching the two Fast/DFast finders and found that
turning `next_long` OFF makes **`sao` 15,001 bytes SMALLER** at L3 while making
all fifteen other corpora larger. `nlhunt.rs` then showed why the obvious
signal cannot route it: the probe wins **468,072 match bytes** on `sao` and
still costs 15,001 compressed bytes, because it COMMITS at `ip + 1` and can
take a longer match at a worse OFFSET. `next_long_yield` does not separate that
case -- `x-ray`'s yield is 0.0002 against `sao`'s 0.0241 and the probe HELPS
`x-ray`.

The encoder already counts the offset trade (`band_worse / band_hits` ->
`tables.nl_off_worse`) and `nl_cut_for` already dispatches on it. **That
dispatch is off by default** (`NL_DISPATCH_ON != 2` returns the bare 8, so
`dfast_good_ml_raised()` is never consulted -- which is also why sweeping
`dfast_good_ml` with the dispatch off moves nothing at all).

| config (L3, 18 corpora, 4 MiB each) | bytes | vs shipped |
|---|---:|---:|
| shipped default | 22,934,391 | -- |
| `nl_dispatch` on | 22,865,392 | **-68,999 (-0.301%)** |
| + `dfast_good_ml` 48 | 22,851,738 | **-82,653 (-0.360%)** |

Eleven corpora improve, two regress (`reymont` -33,804, `webster` -18,406,
`samba` -5,136, `jsonlog` -4,852 against `dickens` +1,716, `mr` +517).

**Priced in work, not guessed.** The dispatch raises the "good enough, stop
searching" cut, so it must be bought with search -- but the longer matches it
finds leave less to scan and encode, and three of the four counters go DOWN:

| L3 | candidates | fills | positions | sequences |
|---|---:|---:|---:|---:|
| nl_dispatch | +9.00% | -1.28% | -0.45% | -1.18% |
| + good_ml 48 | +10.88% | -1.41% | -0.50% | -- |

**Stability and correctness.** 72/72 round-trips with checksums; the win holds
across a 4x range of input, so it is not a warm-up artefact:

```text
  cap    L3        L4
  2 MiB  -0.257%   -0.221%
  4 MiB  -0.254%   -0.258%
  8 MiB  -0.232%   -0.273%      (nl_dispatch alone)
  4 MiB  -0.304%   -0.284%
  8 MiB  -0.271%   -0.274%      (+ good_ml 48)
```

**48 is off the plateau, not the argmax.** The `mlgrid` 2-D sweep's best cell is
`good_ml=64, good_ml2=24` at -82,975 B; `48`/follow is -82,653, i.e. 322 bytes
(0.0014%) worse and not at an edge. Everything in 40..64 lands within 0.03%, so
the extreme cell is far more likely to be this corpus set than a real optimum.

Smaller size results from the same sweep, all at their true defaults:

- `dfast_good_ml2` = 48 with the dispatch OFF: **-10,808 B (-0.047%)** at L3 --
  independent of the above, because the second-candidate cut only ADDS a
  candidate at `ip` and cannot shorten the match.
- `pair_hi` = 4.0 at L1: **-7,702 B (-0.031%)**, flat from 4.0 to 9.0.
- `dfast_step` = 1 at L3: **-5,051 B (-0.022%)**.
- `search_log_d` = +1 at L9: **-86,421 B (-0.399%)** -- a depth increase, so a
  size-for-speed trade of a different character.

### Two harness defects caught in this sweep, recorded because both read as
### code defects

- **`set_pair_hi_arm(-1.0)` does not reset the arm, it PINS it to -1.0.** The
  f32 arms cache raw bits with `u32::MAX` as "unset" and the setter stores
  `v.to_bits()`, so there is no public reset. A sweep that used -1.0 as its
  baseline measured every delta from a changed config and reported
  `pair_hi=4.0` as **-47,965 B**; against the documented default (1.0) the same
  cell is **-7,702 B**. Set baselines explicitly; never assume a sentinel.
- **`accel_shift` is inert without `--features profile`.** `accel_shift_for` is
  consulted only under `cfg!(feature = "profile")`; release builds take the
  constant 7 (Fast) / 8 (DFast). Swept without it, all nine values read exactly
  +0 -- which is the signature of a dead knob and was a dead harness.
### Fixed -- two correctness gates were passing on SILENCE, not on evidence

Both gates covered a *level list* and believed they covered a *strategy set*.
They do not coincide, and nothing checked that they did.

**`tests/kreach_gate.rs` -- the gate that enforces >=95% kernel routing --
exercised three of the nine match finders.** Levels were `[1, 3, 9]` under a
comment claiming they "cover the distinct match-finder strategies"; they
resolve to Fast, DFast and Lazy2. Nothing reached `find_greedy`, `find_lazy`,
`find_bt_lazy` or `find_opt`'s BtOpt/BtUltra arms -- so a kernel reached only
from one of those scored `(0 kernel, 0 scalar)`, and the verdict loop **skips**
any slot with `h + m == 0` as "not exercised on this side". A dispatch that
never took its twin in five of nine finders would have passed.

Now `[(1,4M) (3,4M) (5,4M) (7,4M) (9,4M) (13,2M) (16,1M) (18,1M) (19,1M)]` --
one level per strategy, with a smaller prefix for the levels that are orders of
magnitude slower per byte, because reach is a RATIO and does not need the whole
corpus to be non-zero. `count_eq_len wide` traffic goes 9,316,212 -> 9,477,458
calls; all eleven slots read 100.00%; the run still takes 1.6 s; and the
`RZSTD_KREACH_POISON=1` self-check still fails, so the gate is proven live.

**`examples/simdparity.rs` never parity-checked BtLazy2.** Its doc lists the
families it claims to cover -- "fast, dfast, greedy, lazy, lazy2, btlazy2,
btopt, btultra" -- but the list said `12`, and **L12 resolves to Lazy2**;
BtLazy2 starts at L13. The gate that exists to catch a `simd.rs` defect in
every finder skipped `find_bt_lazy` entirely. `12 -> 13`, plus `18` for
BtUltra.

Both now carry an `assert!` that resolves each level's strategy and fails if
the set stops covering every finder, so the doc comment is enforceable instead
of aspirational.

### Fixed -- `allgates` reported a live ratio gate as dead

`LEVELS` was `[1, 3, 19, 22]` = Fast, DFast, BtUltra2, BtUltra2: **three of
nine strategies**, and none of levels 5-18. Every arm whose only call sites
live in an untested finder read SZ-DEAD for a reason that has nothing to do
with the arm -- the exact failure this tool exists to prevent.

`lazy_fill` is read only in `find_lazy_impl` and `find_bt_lazy`, so it read
SZ-DEAD at every prefix. Toggled at L9 it moves **266,695 compressed bytes
(1.53% of a 40 MiB board) and 4,133,134 probes**. Widening the list flips it
to LIVE with 28 moved cells (`mr` +23,482, `ooffice` +18,025). `pair_gain`'s
`0.0` arm flipped dead -> live too, and resolution improved throughout:
`search_log_d` 2/3 -> 61/52 moved cells, `strategy` 65 -> 111, `rep1_mode`
13/22 -> 42/71.

A `strategy_coverage` self-check now prints what the list exercises and names
anything missing. It earned its keep immediately: the first widened list
(`[1,3,5,9,13,16,19,22]`) still missed Lazy and BtUltra, and the check said so.

### Fixed -- two counters that do not mean what a reader assumes

- **`EncodeCounts::hash_probes` is not comparable across finders.**
  `find_fast_impl_inner` bumps it at the TOP of the scan loop (POSITIONS);
  `find_dfast_impl_inner` bumps it inside `if let Some(m8)`, after a tag filter
  (SURVIVORS). A `probe_hits / hash_probes` "hit rate" therefore reads ~12% at
  L1 and ~94% at L3 for reasons entirely about the denominator. Use
  `encode::take_mm`, bumped at the loop top in both. Documented at the field.
- **`EncodeCounts::hash_fills` reads a FALSE ZERO at L5 and above.**
  `note_hash_fill` is called only from the Fast/DFast fill helpers; the chain
  inserters Greedy/Lazy/Lazy2/BtLazy2 fill through never report. Deliberately
  not wired there -- `lazyfill.rs` measures 41,742,765 fill inserts at L9 and a
  `lock xaddq` on each would be the instrument dominating what it measures.

### Refuted, recorded so they are not retried

- **Removing the ten zero-caller functions is worth ZERO emitted bytes.** The
  release asm contains none of them; LLVM already eliminates every one, so the
  `#[allow(dead_code)]`-with-rationale convention costs nothing.
- **Packing the paired gate counters two-per-`u64`** (`nl_probes`/`nl_hits`,
  `band_hits`/`band_worse`, `spec_made`/`spec_dropped`) to relieve the DFast
  loop's register pressure measured **`find_dfast` 1448 -> 1469 instructions**,
  spills 184 -> 185, reloads 248 -> 251. The `1 << 32` constants and the unpack
  cost more than the register saved. Built, measured, reverted.
- **Hoisting the table bases out of the main loops** the way W9 did for the
  (default-off) fill stride buys nothing: the emitted innermost loops of both
  `find_fast_impl` and `find_dfast` contain **no repeated pointer-relative
  loads**. LLVM already hoists them; the repeated `%rbp`-relative loads are
  spill/reload of a live set that is simply too large.
- **No panic landing pads to remove**: `find_fast_impl`, `find_dfast`,
  `find_lazy` and `find_greedy` have zero guard branches already.
- **`find_fast`'s BMI2 twin converts 41 shifts across 1,817 instructions**
  (44 instrs/op) -- DENSER than every twin this crate retired (dfast 72,
  bt 97, lazy 111, greedy 123, chain 152), and retiring it would drop
  `K_FIND_FAST` to a miss against the >=95% reach gate. Left alone.
### Fixed -- a knob cache whose sentinel collided with its own default

`dfast_step_forced` cached its env knob in an `AtomicU32` and treated 0 as
"not yet read". But 0 is also what the UNSET knob resolves to -- the shipping
default -- so the store never took and every call re-read the environment: an
OS lookup and a `String` allocation, once per block, forever, for a value fixed
for the life of the process. Storing `v + 1` makes 0 mean "unread" and nothing
else. The public `set_dfast_step_arm` is biased to match, or a set value would
read back one low.

**-829 allocations per 88 MiB at L3.** This crate has been bitten by the
per-call `std::env::var` shape repeatedly -- one instance is recorded in-source
as having cost 60% of L19 encode -- and every previous fix was a cache. This one
HAD a cache; it just never engaged.

Two durable consequences:

- **All 33 direct `std::env::var` calls now route through the counted
  `env_knob` shim**, so one counter sees every knob read in the crate.
- **`tests/env_reads_gate.rs` makes it a gate**: a correctly cached knob costs a
  fixed number of reads no matter how much data is compressed, so the test
  compresses 128 KiB and 1 MiB and fails if the count SCALES. Verified by
  poisoning -- with the old sentinel restored it reads 2 vs 11 and fails with a
  diagnostic naming the cause.

### Changed -- encoder allocations halved

With the three pool leaks, the stack histogram, the knob cache and a pooled RLE
header:

| encode, 88 MiB | before | after |
|---|---:|---:|
| allocations per MiB @ L1 | 116.8 | **64.9** |
| allocations per MiB @ L3 | 143.7 | **71.1** |
| allocations per MiB @ L9 | 128.9 | **69.5** |
| scratch-pool hit rate | 78.2% | **99.8%** |
| copies per input byte | 0.430 | **0.371** |

`ct_pool` (which keeps its own free list) is now covered by the same census, so
one number answers for every pool in the crate. It measured ~100%, which is
only knowable by looking.

Byte-identical across 48 (level x corpus) pairs with exact round-trip.

### Known -- the finder scratch reallocates ~80 times per corpus

`dfast_finder_prologue` discards its pooled scratch and allocates a fresh one
whenever capacity falls short. `lit_scratch` is sized `block_len +
LIT_PUSH_WIDTH_MAX`, i.e. past the 128 KiB large-allocation threshold, so each
is a VirtualAlloc and a page-table edit. Measured: **80 reallocations, 7.89 MB**
over twelve corpora -- roughly per-frame rather than per-block, so mostly
amortised already (~0.15% priced). Counted and left, not fixed.
### Changed -- encoder allocations down 44%, and three scratch-pool leaks closed

An allocation-site census (backtrace-sampled, so it FINDS sites rather than
confirming suspected ones) put the encoder at 143.7 allocations per MiB against
the decoder's 1.6. Five landed changes, each byte-identical:

1. **`ncount_seq_table` copied a histogram to the heap to decrement one entry.**
   `counts.to_vec()`, three times per block (ll/of/ml), was **half of all
   encoder allocations**. The sequence tables' alphabets are fixed by the format
   (LL 36, OF 32, ML 53), so the copy is now a `[u32; 64]` on the stack with the
   heap path kept for any future wider caller. **-10,007 allocations (-30%).**
2. **`normalize_count` leaked its pooled buffer on the low-probability branch.**
   It takes `norm` from `SC_NORM` and returns `n2` instead; `norm` was dropped.
3. **`write_tree_fse` never returned its `norm` buffer.** `ncount_and_ctable`
   closes that loop for its own callers; a caller using `normalize_count`
   directly owns the give-back and this one did not do it.
4. **`write_tree_fse` never returned its `ncount` buffer either** -- same defect,
   different pool.
5. **`ct_pool` is now covered by the same census.** It keeps its own free list,
   so the `scratch` counters could not see it; it turned out healthy (~100%),
   which is itself worth knowing.

| encode, 88 MiB, L3 | before | after |
|---|---:|---:|
| allocations | 12,671 | **7,105** |
| allocations per MiB | 143.7 | **80.6** |
| scratch-pool hit rate | 78.2% | **99.8%** |
| pool misses (each an allocation) | 1,437 | **24** |

**REFUTED TWICE: raising the pool's free-list cap.** 6 -> 32 changed hits and
misses not at all, first at the 78.2% rate (5,167 / 1,437 both ways) and again
after the leaks were fixed (5,885 / 719 at caps 6, 12 and 32 alike). Only drops
moved. The cap stays at 6 with the refutation recorded beside it -- the misses
were leaks and first-use, never capacity.

The ordering matters and is the lesson: the cap experiment was run FIRST, came
back flat, and was correctly recorded as a refutation. It stayed a refutation
after the leaks were fixed -- but the leaks were only found by asking why the
hit rate was 78% when the cap plainly was not the reason.

Byte-identical across 36 (level x corpus) pairs with exact round-trip.
### Changed -- the literals section is written straight into the frame

`encode_literals_section_into` packed every candidate into a staging buffer and
copied the winner into `dst`. The staging was blamed on the header needing the
compressed size before the body can be placed -- but at PACK time the body is
already encoded, so `csize` is known and `write_lit_huff_header_into` takes it
as a parameter. The real constraint is only that candidates compete on size and
a loser must be discardable.

The new table is Huffman-OPTIMAL for the block's frequencies, so `body_new <=
body_prev` always and it is the candidate that usually wins. It is now appended
straight into `dst` and `dst` is truncated on the rare loss; only a
previous-table winner still pays a copy.

| one-shot encode, 208 MB corpus, L3 | before | after |
|---|---:|---:|
| section -> dst bytes | 17,730,842 | **5,515,309** (-69%) |
| sections needing the copy | 1,384 | **495** (64% now zero-copy) |
| all encode copies | 0.430 B/input | **0.371 B/input** |

Byte-identical across 48 (level x corpus) pairs with exact round-trip.
`pack_huff_section_append` shares its header arithmetic with the `Vec` form
through `lit_huff_header_bytes`, so the two cannot drift on a format-visible
layout.

### Known -- the encoder's copy surface, fully decomposed

Two rounds took one-shot encode from **0.633 to 0.371 copies per input byte**.
What remains is 77.3 MB, and every part of it is now categorised rather than
merely un-attacked:

| site | MB | status |
|---|---:|---|
| raw block -> dst | 33.6 | **irreducible** -- src must reach the output once |
| src -> lits | 24.0 | **architectural** -- literals are scattered src ranges and must be gathered before entropy coding |
| huffman body -> section | 14.2 | **blocked** -- the header precedes the body and needs the body's length; the only escapes change the bitstream |
| section -> dst (residual) | 5.5 | previous-table winners only, 36% of sections |

The `huffman body` entry is worth stating precisely because the neighbouring
`section -> dst` copy looked identical and was NOT blocked. The difference: at
pack time the body exists, so its length is known; at encode time it does not,
and the header that must precede it needs that length. Reserving a maximum
header and backfilling does not help either -- the header is 3, 4 or 5 bytes by
size class, so a shorter one leaves a gap that costs a memmove of the body,
which is the copy being removed.

### Known -- the scratch pool runs at 78% and is not capacity-starved

A new pool census (`take_pool_census`, profile-only) measures hit/miss/drop/give
on every `scratch` free list. Over four corpora at L3: **5,167 hits, 1,437
misses, 708 drops**, a 78.2% hit rate -- and every miss is an allocation, worth
~11% of all encoder allocations.

**REFUTED: raising the free-list cap from 6 to 32 changed hits and misses NOT AT
ALL** (5,167 / 1,437 both ways); only drops moved, 708 -> 682. Takes and gives
are near-balanced (webster: 2,304 takes against 2,048 capacity-carrying gives),
so it is not a leak either. 78% appears to be this design's structural rate, and
the cap is left at 6 with the refutation recorded next to it.
### Changed -- ten copy reductions, batched

Each is byte-identical and carries its own deterministic counter; the batch
carries the timing verdict, per the sub-1% brick discipline. Every one was
found with the emitted-assembly census rather than a source grep.

**Encode**

1. **The streaming compressor's `in_acc` staging buffer is gone.** Every input
   byte was copied twice before encoding began -- caller -> `in_acc` ->
   `hist` -- at exactly 1.000 + 1.000 B/input. `hist` was always the buffer the
   finders read, so a cursor into it does the same job. **-98.9 MB**; streaming
   encode 3.336 -> 2.336 copies/input. The slide now triggers on the ENCODED
   cursor (sliding on `hist.len()` would drop pending caller bytes) and the
   block checksum covers exactly `[block_start..block_end]`.
2. **The compressed block payload is emitted straight into the frame.** It was
   built into a scratch `Vec` purely to learn its length, then copied in whole.
   A zstd block header is FIXED at three bytes, so the payload is now written
   past a three-byte hole and the header patched in place; if raw wins, `out`
   truncates back. **-42.2 MB**, the largest reducible copy in the encoder.
3. **`MatchTables::payload_scratch` deleted** -- dead once (2) landed, so a
   block-sized buffer per table set goes with it.
4. **The MT concat reserves the exact total.** Job outputs were concatenated
   into a `Vec::new()`, so the buffer grew to the whole compressed stream by
   doubling -- ~1x the output again in realloc copies. The job lengths are
   known when the jobs finish. **32.1 MB concat, no longer paid twice.**
5. **The seekable concat reserves too**, extrapolated from the first frame's
   MEASURED compressed size rather than a guessed ratio; `entries` is now exact
   from the frame count.

**Decode**

6. **A frame with no declared content size now reserves.** `out.try_reserve`
   only ran when the header carried a size -- and our own streaming compressor
   omits it unless the caller pledges one, so unpledged streams grew the output
   by doubling. Extrapolated from the first block: measured 0.27-0.98x of the
   true size on frames that previously got nothing.
7. **The decoded-window compaction fires half as often** (`window + min(window,
   8 MiB)`). Unlike the encoder's slide this has NO ratio cost -- decode output
   is fixed by the bitstream. Compactions 4/10/16/15 -> 2/5/8/8, traffic
   **-53%** (0.941 -> 0.441 B/output on webster).
8. **The input compaction fires half as often again** (`3 * dead >= 2 * live`,
   so each reclaim is worth twice its move). **-50%**: 0.290 -> 0.145 B/output.

**Training**

9. **`fallback_content` and the segment concat reserve and trim in place.** Both
   grew unreserved and then copied the tail into a SECOND allocation. Only the
   last `max_dict` bytes are ever kept, so the buffer is now bounded at ~2x
   `max_dict` instead of the whole sample set.
10. **The dictionary tail trim is a `drain`, not a `to_vec`** -- was a fresh
    allocation and a full copy to discard a prefix.

Net, per the byte census: one-shot encode **0.633 -> 0.430** copies per input
byte, streaming encode **3.336 -> 2.336**, streaming decode **2.53 -> 1.75-1.99**
per output byte.

Gates: encode byte-identical and round-trip exact over 50 (level x corpus)
pairs including `--ultra -22`; 63 streaming round-trips byte-exact across
corpora x levels x chunk geometries; dictionary byte-identical across the
trainers; 141 lib tests plus the full suite.

**Two of these were found only because a census slot read ZERO.** `C_SEQ_TO_DST`
and `C_BLOCK_TO_FRAME` were declared and never wired, and an unwired slot reads
exactly like a site that costs nothing -- which is how the previous pass closed
on 0.36 copies/input when the true figure was 0.633.
### Changed -- the streaming compressor's `in_acc` staging buffer is gone

Every input byte was copied TWICE before encoding began: caller -> `in_acc`,
then `in_acc` -> `hist`. Measured at exactly **1.000 + 1.000 bytes per input
byte**. `hist` was always the buffer the match finders read; `in_acc` only held
a partial block until one was ready, which a cursor into `hist` does just as
well. `in_acc` and `compact_in` are deleted.

| streaming encode, 98.9 MB corpus, L3 | before | after |
|---|---:|---:|
| caller -> in_acc | 1.0000 B/input | (gone) |
| in_acc -> hist | 1.0000 B/input | **0** |
| total copies | 3.336 B/input | **2.336 B/input** |
| bytes moved | 330.0 MB | **231.1 MB** (-98.9 MB) |

Compressed output is bit-for-bit unchanged and one-shot encode is byte-identical
across 20 (level x corpus) pairs. Two details carry the correctness: the window
slide now triggers on the ENCODED cursor rather than `hist.len()` (sliding on the
buffer length would drop pending caller bytes), and the block checksum covers
exactly `[block_start..block_end]` rather than "to the end of hist", which now
has pending bytes past it. `encode_block_from_scratch` takes an explicit
`block_end` for the same reason.

Worth **0.25-0.55% of streaming encode** at measured memmove rates -- a counter
win, not a clock win, and recorded as such.

### Known -- the encoder's copy surface is now fully tapped, and it was under-counted

Two census slots were declared and never wired. Tapping `C_BLOCK_TO_FRAME` --
the `out.extend_from_slice(&payload)` that copies every compressed block into the
frame -- added **42.2 MB, 0.2026 B/input**, the largest reducible copy in the
encoder. One-shot encode is **0.633 copies per input byte, not the 0.36**
previously reported and closed on. A declared-but-unwired slot reads exactly like
a site that costs nothing.

Priced against measured (not peak) memmove rates, every remaining candidate is
an order of magnitude below the ~0.4-0.5% at which this workspace has previously
pruned copy work -- block->frame 0.040-0.089%, src->lits 0.023-0.051%,
section->dst 0.017-0.037%, huffman emit 0.014-0.030%, out_acc->caller
0.080-0.178%. They are pruned on arithmetic, not abandoned.

The one exception is `decoded -> caller` in the streaming decoder at
**0.454-1.008%** (98.9 MB). That is not a copy fix: removing it means decoding
directly into the caller's buffer when it is large enough, which requires the
caller's buffer to become the match window -- what libzstd does, and an
architecture change rather than a copy tweak.
### Fixed -- the streaming decoder's input compaction was an O(n^2) front-drain

`Decompressor::compact_input` reclaimed the consumed input prefix when that
prefix passed an ABSOLUTE 64 KiB -- but the drain memmoves the LIVE remainder,
and Brick A deliberately stops decoding the moment the caller's buffer can be
filled, so the remainder grows against a fixed trigger. The doc comment claimed
"total moved is bounded by the bytes fed"; measured, it was not:

| 32 MiB streamed, 64 KiB feed, L3 | before | after |
|---|---:|---:|
| webster in-compaction memmove | 7.633 B/output (244 MB) | 0.290 B/output |
| mozilla | 5.135 (164 MB) | 0.290 |
| samba | 2.923 | 0.220 |
| dickens | 1.874 | 0.333 |
| **all streaming-decode copies** | **3.87-9.87 B/output** | **2.33-2.56** |

Adding `2 * in_off >= input.len()` makes each compaction reclaim at least as
much as it moves, which is what makes the documented bound real -- `in_compact`
now sits at ~0.29 B/output, i.e. the compressed ratio, i.e. the bytes fed. It
also bounds the buffer: the live tail is never more than half, so `input` holds
at most ~2x what the caller has fed ahead.

**Kept on the deterministic counter, NOT on a clock.** Removing 244 MB of
memmove did not move the wall: streaming-vs-one-shot measured 0.568x before and
0.550x after on webster (null arm 0.91-0.98x, |z| <= 0.77). That is this
crate's own copy-handling law landing on its author -- A/B every obvious hop,
most measure ~0 -- and the honest label is *below instrument resolution*, not
*a speedup*. It is kept because the work removed is real and reproducible, the
memory bound is now genuine, and the comment is no longer false.

Decode output is identical to the pre-change binary across 24 (level x corpus)
round-trips.

### Known -- streaming decode is ~1.3-1.4x slower than one-shot; EIGHT causes ruled out

Measured admissibly (one process, ABBA-interleaved, null arm 0.95-1.08x):
one-shot -> streaming is **0.703-0.776x, z = -1.67 to -3.00**, on identical
bytes.

**This figure is REVISED DOWN from an earlier 1.5-1.8x, and the difference was
the harness.** The benchmark fed a chunk and read the output ONCE; the decoder
consumes all input it is handed but emits only what fits the caller's buffer,
so at a 3:1 ratio a 64 KiB chunk yields ~192 KiB and the read under-drained.
`decoded` accumulated a backlog no real consumer would build. Draining fully
before feeding again accounts for 5-8 points of the gap. An even earlier
attempt was wholly inadmissible -- sequential best-of-N blocks whose one-shot
arm drifted 29% between runs, producing a 1.20x "streaming is faster" row.

Eight causes have been eliminated BY MEASUREMENT, so none is worth re-testing
without new information:

1. **Copy traffic** -- cut 74% (9.87 -> 2.53 B/output); ratio unchanged.
2. **Structural re-entry** -- stage call counts are IDENTICAL (256/256).
3. **A single hot stage** -- scoped stages grow only 1.09x.
4. **The decoded-window compaction** -- ablated; no change.
5. **Per-call overhead** -- 32x fewer `stream` calls (128 KiB -> 4 MiB output
   buffer); no change.
6. **Buffer reallocation** -- a header-time reserve already exists.
7. **Buffer shape** -- made structurally one-shot-like (no compaction, no early
   exit, full reserve); the gap persists.
8. **The content checksum** -- disabled on both arms; no change.

The stage profiler CANNOT resolve this: its rdtsc tax is 27-29% of wall in both
arms and compresses the measured 1.4x into 1.04x. The next instrument is a
sampling profiler on the two binaries, not another hypothesis.

### Changed -- streaming window slide fires half as often (14-47% faster streaming encode)

A second copy pass, this time over the STREAMING encoder -- which the one-shot
census never exercises, and which turns out to move **1.80 bytes per input
byte against one-shot's 0.36**. 80% of that is the window slide.

Each slide memmoves the retained window, zeroes six match tables, and
re-primes every position of what is left. Section 20 already moved the trigger
from `hist > window` to `hist >= 2 * window`. This moves it again, to
`2 * window + min(window, 8 MiB)`, which halves how often all three costs are
paid:

| per 98.9 MB streamed, L3 | before | after |
|---|---:|---:|
| slides | 42 | 19 |
| window memmoved | 88.08 MB | 39.85 MB |
| tables zeroed | 55.05 MB | 24.90 MB |
| **prime inserts** | **88,080,048** | **39,845,736** |

**The prime inserts are the prize, not the copies.** The memmove and the memset
are sequential streaming and, priced against measured encode throughput, all of
the slide's copy traffic is under 1% of encode time -- which is why this looked
like a wash at first. The 48.2 million table inserts removed alongside them are
hashes followed by random-access stores into multi-megabyte tables, i.e. cache
misses, and they dominate. Pricing only the bytes moved was the analytical
error; the counter that mattered had to be added before the trade could be read
correctly.

Measured, one process, arms ABBA-interleaved, with a NULL arm to establish what
the box can resolve (null: 0.98-1.02x, |z| <= 1.81):

| corpus | median | min-of-N | paired | size |
|---|---:|---:|---:|---:|
| samba | 1.466x | 1.395x | 15/15, z=+3.87 | +0.137% |
| webster | 1.245x | 1.144x | 13/15, z=+2.84 | +0.109% |
| mozilla | 1.318x | 1.298x | 14/15, z=+3.36 | +0.058% |

**It is a trade, not a free win.** Compressed size grows 0.06-0.14%, and the
direction is counterintuitive on purpose: the re-prime inserts EVERY position
of the retained window, which is denser than the strided inserts normal
encoding does, so frequent sliding was accidentally acting as a table
densification pass. Sliding less often gives that up. The regression has the
same sign on all four corpora, so it is a uniform cost rather than a
content-dependent one -- a straight keep/revert decision, not a dispatch.

**The extra history is capped in ABSOLUTE bytes, and that matters more than the
multiplier.** The win scales with slide FREQUENCY (high when the window is
small) while the memory cost scales with window SIZE, so a flat 3x would spend
**+128 MiB at L22** to remove slides that a 128 MiB window mostly never
performs. Capping the extra at 8 MiB keeps the entire win at every level up to
L19 and bounds the worst case at +8 MiB. `slide_threshold_caps_the_extra_history`
pins that, including that `k = 2` reproduces the previous threshold exactly at
every window size.

One-shot encode is untouched and byte-identical across 42 (level x corpus)
pairs -- the slide is streaming-only. `RZSTD_ENC_SLIDE_MUL=2` restores the
previous behaviour exactly; `Compressor::set_slide_mul` overrides per instance.
### Changed -- encoder copy catalogue, one elimination, and a measured prune

Catalogued every byte-moving call on the encode path two ways, because neither
alone answers the question. `tools/copycat.py` reads the emitted asm and says
WHERE the `memcpy`/`memset`/`memmove` calls are (106/32/2 on the encode side,
flagging the ones inside loops); `rusty_zstd::copies` counts BYTES per site at
run time, which is what says whether a call matters. A call count is a bad
proxy: one call on the literal path moves a whole block, six in a table-setup
loop move a few hundred bytes between them.

Census over 9 corpora, 208 MB, L3 -- deterministic, same numbers anywhere:

| site | bytes | per input byte | verdict |
|---|---:|---:|---|
| `src -> lits` | 23.96 MB | 0.115 | architectural: literals are scattered `src` ranges and must be gathered |
| `section -> dst` | 17.73 MB | 0.085 | one copy of the Huffman winner; candidates compete on size so it needs staging |
| `raw block -> dst` | 33.55 MB | 0.161 | irreducible: `src` to output, the minimum possible |
| `lits -> raw section` | **4.67 MB -> 0** | 0.022 -> 0 | **removed** |

**The elimination.** The four literals arms that decide immediately -- empty,
RLE, tiny, not-worth-Huffman -- each built their section into a fresh `Vec`
which the caller then copied into `dst` and dropped. A raw literal byte was
therefore touched three times: staged out of `src`, materialised into a
section, copied to the output, for a byte the format says to store verbatim.
`encode_literals_section_into` writes through `dst` instead, so those arms emit
once. **4,669,394 bytes and 439 allocations per corpus pass, gone**; the
allocating twin now has no caller anywhere and was deleted. Output is
byte-identical across 60 (level x corpus) pairs.

It cost **+136 static instructions** in `write_literals` -- specialised emit
arms are new code. The byte count is the right instrument for a copy removal;
the instruction count is confounded here because the change adds paths rather
than shortening one.

**The prune, which is the more useful result.** Further copy elimination cannot
pay, and this is arithmetic rather than opinion:

* all 75.2 MB of remaining copy traffic, at 10-20 GB/s, is 3.8-7.5 ms;
* the encoder runs at 39 MiB/s, so that corpus is ~5.27 s of encode;
* every copy in the encoder is therefore **0.07%-0.14% of encode time**, and
  the one removable item left (`section -> dst`) is ~0.02%.

The stage profiler agrees from the other side: **match-finding is 76.3% of
encode and entropy coding 21.0%**. Copies do not register as a stage. Two
thirds of what remains is irreducible by construction anyway -- raw blocks must
move `src` to the output, and scattered literals must be gathered before they
can be entropy-coded.

So "zero copies" is not reachable and would not be worth reaching. The
`copies` census stays wired (it compiles to nothing without `profile`) so the
`lits -> raw section` row reading 0 is a standing check that the
materialise-then-copy path has not come back.
### Changed -- deterministic instruction/guard reductions (byte-identical)

A hunt for straight-line wins measured on DETERMINISTIC counters only -- static
instruction counts and guard-branch counts from the emitted release asm -- because
this box cannot resolve effects this size with a clock. Same toolchain and source
give the same number on any machine at any load, so a two-instruction change is a
verdict rather than noise.

Two instruments were built for it, and both are in `tools/`:

* `panic_census.py` -- ranks conditional branches whose target's FIRST control
  transfer is a panic call, and resolves each back to the line of our code that
  failed to prove its index (via the `Location` rustc emits beside every panic,
  since debug line tables just blame `core/src/slice/index.rs`). The detector has
  no tunable window on purpose: a line-budget version is a knob, and a knob has to
  be swept before it can be quoted.
* `loopscan.py` -- flags HALF-STORE / NARROW / INVARIANT / SELECT shapes in
  innermost loops.
* `icount.sh` -- per-symbol instruction counts, with a `--diff` mode.

**Result: guard branches 87 -> 28 (-68%), 309 net instructions removed.** Compress
output is byte-identical to the pre-campaign binary across 48 (level x corpus)
pairs, and trained dictionaries are byte-identical across four trainers.

| change | effect |
|---|---|
| `fse::normalize` -- zip `count` with `norm`/`n2` so one length bounds both | -112 instrs over 3 inlined sites, and deletes an `unsafe` |
| `fse::weights_into_body` -- narrow `dst` to its cap once | -71 / -64 (decode side) |
| `huffman::pop_min` -- carry the index out of `.get()` instead of re-indexing | -12 |
| `encode_block` -- `.last()` once instead of `len() - 1` indexed 9x | -6 |
| `mt` worker -- `.get()` replaces the `idx >= n` test AND its bounds check | -11 net |
| `train::hash_dmer` / `packed_dmer` -- iterate the tail, don't index it | -33 guards, -19 net |
| `build_coded_pass` -- no-op clamps on two LUT-derived indices | -1 instr, -2 guards |

Two results that were NOT wins are recorded rather than dropped, because a
refutation nobody writes down gets re-attempted:

* `prime_tables` routing its `chain[..]` write through the `chain_masked_set`
  accessor the hot finders use measured **guards -1, instructions +1**. Two
  deterministic counters disagreeing in sign is not a win. Reverted; the reason
  is in the source at the call site.
* `code_from_base` zipping `base` with `bits` measured **0 and 0** -- LLVM had
  already proven the second table's index. The zip is kept only because it also
  removes a `len() - 1` underflow on an empty table, and its comment says so.

`train::hash_dmer` is the one entry whose instruction count needs a caveat: it is
inlined into two callers and moved them in OPPOSITE directions (`train` -117,
`select_fastcover` +98). A static count cannot price a change across an inlining
boundary, which is exactly why the guard count -- which does not move when
inlining shifts -- is the primary number for this class.
### Added -- kernel-reach census and a standing gate against unwired twins

A `#[target_feature]` twin that exists but is never CALLED is invisible to
every gate this crate already had. Byte-identity passes, because the two paths
agree by design. The round-trip and conformance suites pass. And an arm-toggle
A/B reads FLAT -- which is indistinguishable from "this kernel does not help",
so it gets recorded as a refutation nobody revisits.

That defect shipped here. Before D8a/D8b the xxh64 AVX2 kernel was reachable
only from `xxh64_seed`, whose callers were one unit test and one benchmark; the
encoder, decoder and streaming API all used `Xxh64::update`, which was scalar.
The compress side reached the kernel on **50%** of checksummed bytes and the
**decode side on 0%**, for months, with every gate green.

The repair landed earlier. What was still missing was the instrument that would
have caught it, so this adds one:

* `rusty_zstd::kreach` -- a per-dispatch-site census of how many calls reach the
  kernel versus the scalar arm. Counters sit AT the dispatch, never at an
  eligibility test upstream of it: `simd`'s existing `wide_eligible` counter is
  not kernel reach, because most calls it counts are resolved by the 32-byte
  word ladder before any kernel runs. Thread-local `Cell` bumps, not atomics --
  `count_eq_len` runs ~247M times at L19 and a `lock xaddq` there would be the
  instrument dominating the measurement. Compiles to nothing without `profile`;
  compressed output is byte-identical between the two builds (verified over
  4 levels x 2 corpora).
* `tests/kreach_gate.rs` -- a standing gate asserting every exercised site
  routes >=95% of its calls to its kernel. Skips a slot whose ISA the host
  lacks; fails if fewer sites were exercised than the host can route, so it
  cannot pass on silence. That floor is HOST-DERIVED (6 with BMI2, 2 with only
  a vector ISA, else 1) -- it began as a flat 6 and that failed every aarch64
  runner, where the eight BMI2 slots are skipped by design and a correct build
  can only ever check two. `RZSTD_KREACH_POISON=1` forces the arms scalar and
  the gate must then FAIL -- CI runs both directions, because an assertion that
  has never fired is not evidence. The poison still bites on aarch64 because
  `set_xxh_avx2_arm(false)` gates the NEON stripe path too; `count_eq_len`'s
  NEON arm is compile-time and cannot be poisoned, and one poisonable slot is
  enough.
* `rusty_zstd-bench/examples/kreach.rs` -- the corpus-scale report, encode and
  decode censused separately (a combined number is exactly what hides
  "50% of encode and 0% of decode").

Measured over 16 corpora, 298.4 MiB, at levels 1/3/5/9/12/19: **every dispatch
site routes 100.00% of its calls to its kernel, on both sides.** L19 alone is
188,398,067 `count_eq_len` wide calls, all reaching AVX2. `scripts/isaudit.sh`
separately confirms none of the twins is a do-nothing thunk -- each carries the
BMI2 ops its baseline sibling carries in `%cl` (`decode_4x_x1_bmi2` 60,
`encode_stream_unrolled_bmi2_into` 118, `find_fast_impl_bmi2` 41).

Two twins are deliberately NOT routed, and are recorded here so they are not
mistaken for this defect: the `bt_*_spec_bmi2` chain (retired by D5/D11 on its
own measurement -- 291 instructions to convert three BMI2 ops) and
`simd::look_n_bits_bmi2` (`#[cfg(test)]`; production gets BMI2 through the
enclosing twins, which the asm confirms).

## [0.2.0] - 2026-08-27

### Changed — compressed output moves at levels 1 through 4

The per-match hash-table fill now writes only the match START position, not both
ends. Levels 1, 3 and 4 therefore emit **different bytes** than 0.1.0 for the
same input. Output remains valid RFC 8878 and is still accepted by
`zstd -t`/`-d`; only the byte sequence differs.

The trade, boarded per level over 12 corpora before the default was flipped:

| level | per-match table writes | compressed size |
|---|---|---|
| L1 | **0.50×** | +0.150% |
| L3 / L4 | **0.50×** | +0.371% / +0.482% |
| L5 and above | unchanged | unchanged (bit-identical) |

Whole-board effect: 59,760,356 → 59,841,188 bytes (**+0.135%**) across 18
corpora × 9 levels, for half the fill work at every level that fills through
this path. Levels 5+ use different finders and are untouched.

### Fixed

- **Portability, x86_64**: `HuffmanTable::decode_4x_bmi2` was compiled with
  `avx2` enabled while dispatched on `has_bmi2()` alone. Parts that ship BMI2
  with AVX2 fused off (some Skylake Pentium/Celeron) could have executed VEX
  instructions. The stray `#[target_feature]` came from a deleted sibling whose
  attribute block re-parented; no test could observe it, because an ISA twin is
  byte-identical to its baseline on any host that runs the suite.
- `simd::has_bmi2()` now tests **LZCNT** as well as BMI2, so the runtime guard
  covers every feature the twins it gates actually enable.
- The Huffman weights FSE decode had a BMI2 twin on the once-per-dictionary
  path (where it compiled to a single `jmp` and did nothing) and none on the
  per-block path. Retired the former; the latter now has a working twin.

### Added

- `RZSTD_DFAST_BEXT=1` enables backward match extension in the DFast finder
  (levels 3–4), which every other finder already performs and which C's
  `ZSTD_compressBlock_doubleFast` does. Boarded at **−1.0047%** size at L3 and
  −0.777% at L4 with no regression on any corpus. **Defaults off**: it changes
  the bitstream, and it is offered for evaluation rather than shipped.
- CI now runs `scripts/twinguard.py`, which checks the two ISA-twin invariants
  no test can see: orphaned function-only attributes, and a twin enabling a CPU
  feature its dispatch does not test.

### Removed

- `set_block_avx2_arm` (`#[doc(hidden)]`, no semver promise). The AVX2 block
  driver it selected was retired; the setter had no callers and its reader had
  no readers, so it stored a value nothing consulted.

## [0.1.0] - 2026-08-23

The first public release. `rusty_zstd` has been developed against facebook/zstd
v1.5.7 as a pinned external oracle since M1; this is the point at which the
product surface (M1–M6) is complete and the performance campaign (M7) has taken
compression **ratio** to parity. It is published now because everything a caller
depends on — the format, the API and the interop — is settled, and the remaining
work is speed, which changes no output byte.

### The state of the thing

- **Format: all of RFC 8878.** Raw / RLE / Compressed blocks, Huffman literals
  (1-stream, 4-stream, treeless), FSE sequences in all four modes, repeat
  offsets, skippable frames, multi-frame concatenation, content size, dictionary
  IDs, XXH64 content checksum.
- **Compress and decompress**, levels **−7…22**, with all **nine** libzstd
  strategies implemented — fast, dfast, greedy, lazy, lazy2, btlazy2, btopt,
  btultra, btultra2.
- **The libzstd job list:** streaming, dictionaries with `fastcover` / `COVER` /
  legacy trainers, `--patch-from` prefixes, long-distance matching, `--rsyncable`,
  the seekable frame format, multi-threading with job size and overlap, frame
  inspection.
- **A `zstd`-shaped CLI** — `rzstd` plus the `unzstd` / `zstdcat` / `zstdmt`
  aliases, `-l` / `-b` / `-r`, and the `ZSTD_CLEVEL` / `ZSTD_NBTHREADS` env vars.
- **Zero dependencies** in the published library. Builds on `no_std + alloc` and
  `wasm32-unknown-unknown`.

### Measured

Level 1, 18 corpora, against facebook/zstd v1.5.7 as a pinned external binary,
best-of-N on both arms with a **6.83%** session null arm
([`docs/plans/m7-anatomy.md`](docs/plans/m7-anatomy.md), 2026-08-22 board):

| | vs facebook/zstd v1.5.7 |
| --- | --- |
| ratio, mean `us/c size` at L1 | **0.975** — we emit fewer bytes than C |
| ratio, mean `us/c size` at L3 | **1.012**, worst cell `nci` 1.100 |
| compress speed, mean | 1.83× behind C (we lead on 2 of 18) |
| decompress speed, mean | 1.49× behind C (we lead on 4 of 18) |

The L3 ratio column is identical cell-for-cell across four consecutive boards,
which doubles as an end-to-end identity check. **No speed claim is made for the
optimization campaign**: every brick in it shipped on strictly-less-work plus
byte-identity, never on a wall-clock delta.

### Added — release engineering

- `crates/rusty_zstd/README.md`, wired in as the crate's module documentation via
  `#![doc = include_str!]`, so all three of its examples are compiled and run by
  `cargo test --doc` and cannot go stale.
- `CHANGELOG.md`, this file.
- Crate metadata for crates.io and docs.rs: `documentation`, expanded
  `description`, `no-std` in keywords and categories.

### Changed — the public API is now the codec, not the campaign

Roughly 200 A/B arms and counters (`set_*_arm`, `take_*`, the `prof_*` re-exports,
`BT_SPEC_PAIRS`, `Xxh64Pub`, …) were re-exported at the crate root and would have
rendered on docs.rs beside `compress` and `decompress`. They are now
`#[doc(hidden)]` and documented as carrying **no semver promise**. Nothing moved
and nothing was removed, so the benchmark harness is unchanged; the rendered
public surface is the codec.

A `--features bench-arms` gate was tried first and rejected: it made the arms
unreachable in the default build, which stranded ~228 items as dead code and
would have required blanket-allowing `dead_code` in the shipping configuration.

### Changed — the CLI is four shims over one entry point

`rusty_zstd-cli` declared four `[[bin]]` targets sharing a single `src/main.rs`,
which Cargo warns about on every invocation and has deprecated. It is now a
library exposing `entry()` plus four `src/bin/*.rs` shims. The argv[0] dispatch
that gives `unzstd` / `zstdcat` / `zstdmt` their behaviour is unchanged, and each
shim installs the `#[global_allocator]` itself — house law is that the allocator
lives in the deliverable, never in a library.

### Changed — `rusty_alloc-api` 0.x → `=1.1.0`

The process-wide allocator behind the `rzstd-alloc` seam. It reaches the two
binaries only; the published library still has an empty dependency tree.

### Fixed — `no_std + alloc` did not build

`ctable_from_nbits` called `huff_pool::take_w()` unconditionally, but the pool is
`thread_local!` and therefore `std`-only. Every other pool site in `huffman.rs`
already had the `cfg` fallback; this one did not, so the configuration the README
advertises — and that CI checks for `wasm32` — failed to compile. It now
allocates fresh under `no_std`, as its siblings do.

### Fixed — a shadowed `#[inline(never)]` meant W18 never took effect

`seq_table` in `compressed.rs` carried a documented `#[inline(never)]` (the W18
brick: outline the per-block FSE table build so the sequence loop keeps its
register budget). It was written *below* an `#[inline(always)]` that already sat
on the item, so rustc took the first and discarded the second — silently, until
`unused_attributes` was promoted to a warning. The dead attribute was removed
rather than the live one, so the shipped binary is the one that was measured, and
a note now records that the W18 A/B must be re-run before `seq_table` is promoted
to `inline(never)`. Two more duplicated-attribute pairs (`x2_from_x1_into`,
`FseTable::from_norm_into`) were resolved the same way, both no-ops on codegen.

### Fixed — six duplicated `if COUNT` guards, and dead work in the DP loop

`find_fast`'s instrumented arms contained `if COUNT { if COUNT { … } }` at six
sites. The optimal-parse DP also carried three `o_skip_*` counters that only the
`profile` census reads, and their increments ran in every build; they are now
`cfg`-gated with their reader. An `active: Vec<usize>` work list in the Huffman
merge — one allocation per block — had gone unread since the two-queue rewrite
landed and is removed.

### Changed — the lint gates are real now

The workspace builds and lints clean: zero rustc warnings in the default,
`profile`, and `no_std + alloc` configurations, and
`cargo clippy --all-targets -- -D warnings` passes on all three shipping crates.
`too_many_arguments` and `type_complexity` are allowed at the workspace level with
a stated reason — an entropy or match-find kernel takes its state by argument
precisely so it does *not* reach through a struct in the hot loop.

CI now scopes its strict gates to the shipping crates (`rusty_zstd`,
`rusty_zstd-cli`, `rzstd-alloc`) and checks the campaign harness separately: the
harness carries 455 one-off measurement instruments that need a pinned C binary
and multi-gigabyte corpora CI does not have, and holding them to `-D warnings`
gates nothing a consumer can see.
