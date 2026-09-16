//! The Fast ladder (L1-L2, and the Gate 1 dispatch): one hash table, one probe.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a
//! moved item that was private is now `pub(crate)`, so the parent can
//! still name it. That changes who may reference a symbol, not what is
//! emitted for it -- checked, not assumed: the asm board was identical
//! in all thirty-two columns across the move, which matters because
//! this crate builds at the default `codegen-units = 16`, where rustc
//! partitions codegen units BY MODULE and layout can move inlining.

use super::*;
pub(crate) fn find_fast(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BRICK 49: `use_rep` is default-OFF and measured SLOWER (brick 40: 0/6,
    // z=-2.45, sao -23.0%), yet the emitted probe loop tested it from the STACK
    // on every probe -- two reloads and two branches per probe for a question
    // whose answer is fixed for the whole block. Both flags are frame-constant,
    // so they dispatch ONCE here and vanish from the loop entirely.
    // The DEFAULT arm (no tag, no rep1) additionally specializes on hash_log so
    // the hash shift is an IMMEDIATE. Only Strategy::Fast rows reach here and
    // they use exactly {12,13,14,15,16}; anything else takes the HLOG=0 runtime
    // path, so correctness never depends on that list being complete.
    // The DEFAULT arm (no tag, no rep1) additionally specializes on hash_log and
    // on the step, so both the hash shift and the advance are immediates. Only
    // Strategy::Fast rows reach here and they use hash_log {12..16} with step 2;
    // anything else takes the 0/0 runtime path, so correctness never depends on
    // these lists being complete.
    let pipe_on = pipe_enabled();
    // GATE 6 SPEED DISPATCH. `pair_gain` is MATCH BYTES PER PROBE on the last
    // block the pair search actually ran -- benefit over cost, in the unit the
    // cost is paid in. Three routes, not two:
    //   rate <  PAIR_GAIN_MIN  the search finds nothing worth its probes (x-ray
    //                          0.018, sao 0.081) -- OFF, step 2, pipelined.
    //   rate <  PAIR_RATE_HI   it earns, but the step-1 loop earns the same size
    //                          more cheaply because it keeps the pipeline.
    //   rate >= PAIR_RATE_HI   only the pair path captures it (nci, 8.24).
    // Every `PAIR_PROBE_PERIOD` blocks the route is forced back to `pair` so the
    // rate is RE-MEASURED; without that the low routes never run the search and
    // the gate could never re-open.
    // GATE 18 @ L1 DISPATCH. `route_force` is the probe's own arm; `step_pick`
    // is its latched verdict. Route 2 skips the pair search, where 28.6% of L1's
    // positions live, and the probe decides per content whether that costs
    // bytes -- no static signal separates the free content from the costly
    // (4.70: four content signals and three probe designs failed first).
    #[allow(clippy::if_same_then_else)]
    let route = if tables.route_force != 0 {
        tables.route_force
    } else if tables.step_pick == 2 && tables.step_reprobe > 0 && step_probe_on() {
        2
    } else if !pair_enabled() {
        0
    } else if params.target_length != 0 {
        // Two rungs, one verdict, kept apart on purpose: `--fast` and an
        // unprobed block reach route 2 for different reasons, and the ladder is
        // the record of which reason fired. Collapsing them erases that.
        2
    } else if tables.pair_probe == 0 {
        2
    } else if tables.pair_gain < pair_gain_min() {
        0
    } else if tables.pair_gain >= pair_rate_hi() {
        2
    } else if tables.rep_yield > pair_rep_max() {
        // The STEP-1 route needs the SAME `rep_yield` veto the pair route has,
        // and for the same reason: on rep-dominated content the extra positions
        // find matches the repcode path already covers, and committing to them
        // breaks the chain. Gate 6 documented this for the pair search
        // (versions-16m +10.55%) but the veto was never applied to step-1.
        //
        // It only became visible once `step_rt` was honoured: while the tag and
        // rep arms were silently downgrading step-1 blocks to step 2, the route
        // was being ignored on exactly this content, which accidentally shielded
        // it. Fixing the plumbing exposed the missing veto as versions-16m
        // +12.75%.
        0
    } else if tables.pair_gain < pair_gain_lo() {
        // 4.72: cheap-pair band. See `pair_gain_lo`.
        2
    } else {
        1
    };
    tables.pair_route = route;
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering;
        ROUTE_HIST[tables.pair_route.min(2) as usize].fetch_add(1, Ordering::Relaxed);
        ROUTE_GAIN.fetch_add((tables.pair_gain * 1000.0) as u64, Ordering::Relaxed);
        ROUTE_REP.fetch_add((tables.rep_yield * 1000.0) as u64, Ordering::Relaxed);
        ROUTE_N.fetch_add(1, Ordering::Relaxed);
        // THE DISPATCH-VARIABLE DUMP. Every per-block content signal the encoder
        // already maintains, sampled at one point. 4.72's law: before inventing
        // a signal, dump the ones that already exist.
        // Every accumulator here is INDEPENDENT of the route histogram's, and has
        // its own block count: sharing ROUTE_N made `take_route_hist` drain this
        // dump to zero when called first, and the whole table read 0.0000.
        let q = |v: f32| -> u64 { (v.max(0.0).min(1.0e6) * 1000.0) as u64 };
        SIG_GAIN.fetch_add(q(tables.pair_gain), Ordering::Relaxed);
        SIG_REP.fetch_add(q(tables.rep_yield), Ordering::Relaxed);
        SIG_TAG.fetch_add(q(tables.tag_yield), Ordering::Relaxed);
        SIG_REPLEN.fetch_add(q(tables.rep_len_ratio), Ordering::Relaxed);
        SIG_NSEQ.fetch_add(tables.last_nseq as u64, Ordering::Relaxed);
        // `opt_rep_rate` initialises to f32::MAX; `* 1000.0` overflows to inf and
        // saturates the cast, which is what printed 1.8e19. Clamp at source.
        SIG_OPTREP.fetch_add(q(tables.opt_rep_rate), Ordering::Relaxed);
        SIG_N.fetch_add(1, Ordering::Relaxed);
    }
    let s0 = if params.target_length == 0 {
        if tables.pair_route == 1 {
            1
        } else {
            step0_default()
        }
    } else {
        tables.step_used = 0;
        params.target_length as usize + 1
    };
    // ffanat: WIDE is the sixth const. Runtime `fh.wide` inside the copies
    // forced a per-position branch, a register-resident mask, and `shrq %cl`
    // where specialised copies should emit an immediate -- the asm survey
    // showed shrq$50 x0 / shrq%cl x7 on EVERY wide copy. The latch decides
    // wide-vs-legacy per block BEFORE dispatch, so it is a dispatch input like
    // ut/rep. One branch here doubles the arms mechanically; the executed path
    // carries only its own mode.
    // ffanat guard unification: WIDE additionally requires `pack_tags`. Three
    // payoffs. (1) pack's < 16 MiB frame bound is exactly the proof WIDE never
    // had of its own; (2) inside WIDE copies `pack` becomes CONST-TRUE, so the
    // per-position pack tests and cmov chain fold away and the tags pointer
    // goes dead -- freeing the registers the wide mask and src base were
    // starving for; (3) frames >= 16 MiB run the legacy key, and the one
    // corpus that wanted that at full length is versions-16m itself. Board
    // bytes cannot move: every board runs < 16 MiB where pack is already true.
    let wide_block = fast_hash_wide_enabled()
        && (5..=8).contains(&(params.min_match.max(3) as usize))
        && !tables.fast_hash_legacy
        && tables.pack_tags;
    // W5: THE HLOG AXIS IS BMI2-REDUNDANT, and it was the single most
    // expensive thing in the library.
    //
    // BRICK 54 specialises `HLOG` so the hash shift folds to an immediate --
    // `shrl $n` instead of `mov %cl` + `shrl %cl`. That is a real win on a
    // baseline x86-64 shift, whose count MUST live in `%cl`. It buys exactly
    // NOTHING on the BMI2 twins: `shrx` takes its count from any GPR, with no
    // flag dependency and no fixed register -- which is precisely what the twin
    // campaign's own asm receipt records ("1,878 shrx, 0 CL" on the fast
    // twins). So the twins were paying a SIX-FOLD monomorphisation for a fold
    // their ISA had already made free.
    //
    // Six `hash_log` values x 2 WIDE x ~10 arms = 140 copies of a ~1,450
    // instruction function, TWICE (plain + twin). Routing the twins to the
    // generic-HLOG copy takes their tree from 140 to 40 and leaves brick 54
    // intact on the baseline path that still needs it.
    //
    // BYTE-IDENTICAL, and for the reason this file already gives for the
    // dispatch arms: the const takes the value the runtime variable already
    // held. `HLOG` chooses how the shift is ENCODED, never what it computes.
    //
    // The ISA choice also moves HERE, out of the 140 `find_fast_impl` bodies
    // that each re-asked `has_bmi2()` per block.
    // W4: THE HLOG AXIS IS COLLAPSED ON THE BASELINE PATH TOO -- 48 copies
    // become 8, and `find_fast_impl` goes 46,327 -> 7,852 instructions.
    //
    // W5 (above) removed this axis from the BMI2 twins because `shrx` takes
    // its count from any GPR. The same audit on the baseline path shows the
    // axis was never worth its price there either. `HLOG` reaches EXACTLY TWO
    // lines of `find_fast_impl_inner` -- the `f_mask` and `f_shift`
    // computations -- and both results then travel as ORDINARY RUNTIME
    // ARGUMENTS to `fast_hash_tag`. So six-fold monomorphisation of a
    // ~965-instruction body, twice over PACKED x REP x WIDE, bought one shift
    // immediate in the hash.
    //
    // What the fold is worth, per target:
    //   x86_64 + std, BMI2      already HLOG-generic (W5). Unaffected.
    //   x86_64 baseline         `shr %cl` instead of `shr $imm`: 1 uop, 1
    //                           cycle on every microarchitecture since Core 2.
    //   aarch64 / wasm32        NOTHING. `LSR Rd, Rn, Rm` costs exactly what
    //                           the immediate form costs; there is no `%cl`
    //                           constraint to escape. These targets always
    //                           take this path, and they were paying the whole
    //                           48-copy tree for a fold their ISA does not
    //                           have a problem with.
    //
    // And the I-cache argument runs the other way from the fold: 48 copies of
    // a ~965-instruction body is ~46 KB of code for ONE function, against a
    // typical 32 KB L1i. The specialisation could not stay resident, so on
    // the very CPUs it was written for it is likely a net LOSS.
    //
    // BYTE-IDENTICAL, for the reason the arms below already state: the const
    // takes the value the runtime variable already held. `HLOG` chooses how
    // the shift is ENCODED, never what it computes. (144/144 sha256.)
    macro_rules! go {
        ($p:expr, $r:expr) => {{
            // NOTE: this must stay an EXPRESSION. An earlier revision used
            // `return` here and skipped the `pair_probe` countdown below the
            // match, which froze GATE 6's re-probe and moved L1/L2 bytes.
            //
            // REFUTED, recorded: routing this through a per-(PACKED, REP)
            // `#[inline(never)]` trampoline -- so the arms stamp one thin call
            // instead of two 10-arg unsafe setups -- measured NEUTRAL (+45
            // crate-wide: dispatcher -245, trampolines +200). LLVM was already
            // tail-merging the duplicate call setups across arms.
            //
            // W8: ONE call per ISA arm, with `wide_block` passed straight
            // through. The `if wide_block { f(true, ..) } else { f(false, ..) }`
            // that stood in each arm was selecting between two
            // monomorphisations that W7 merged -- so it had become a branch
            // plus a second 11-argument call setup to reach the same function.
            #[cfg(all(target_arch = "x86_64", feature = "std"))]
            #[allow(unsafe_code)]
            let out = if crate::simd::has_bmi2() {
                crate::kreach::hit(crate::kreach::K_FIND_FAST);
                // SAFETY: runtime CPUID guard, identical body.
                unsafe {
                    find_fast_impl_bmi2(
                        $p,
                        $r,
                        wide_block,
                        s0,
                        pipe_on,
                        src,
                        block_start,
                        block_end,
                        window,
                        params,
                        tables,
                        reps,
                    )
                }
            } else {
                crate::kreach::miss(crate::kreach::K_FIND_FAST);
                find_fast_impl(
                    $p,
                    $r,
                    wide_block,
                    s0,
                    pipe_on,
                    src,
                    block_start,
                    block_end,
                    window,
                    params,
                    tables,
                    reps,
                )
            };
            #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
            let out = find_fast_impl(
                $p,
                $r,
                wide_block,
                s0,
                pipe_on,
                src,
                block_start,
                block_end,
                window,
                params,
                tables,
                reps,
            );
            out
        }};
    }
    // BRICK 67: repcode-1 is DISPATCHED on its own yield, not globally on/off.
    //
    // It is a genuine sign-flip: a LOSS on Silesia (brick 40: 0/6, z=-2.45,
    // sao -23.0%) and a 10x RATIO WIN on constant-stride content
    // (versions-16m L1: 820,848 -> 81,206 bytes). A global default cannot serve
    // both, so each block inherits the previous block's measured repcode yield.
    // `rep_yield` starts at 1.0, so the first block of every frame always probes.
    // EIGHTH sighting of the un-gated per-block atomic class (959e0ae),
    // caught by the whole-binary lock census.
    #[cfg(feature = "profile")]
    FAST_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // GATE 2, SECOND VARIABLE. `rep_yield` alone leaves real wins on the table:
    // always-on is -0.171% overall, with xml -3.708% and mozilla -1.107%, but it
    // costs jsonlog +0.837%. Bytes-per-probe does NOT separate them (jsonlog
    // 0.4610 sits between samba 0.4241 and mozilla 0.6540, both winners).
    //
    // What does separate them is the rep match LENGTH relative to the block's
    // mean match length. Below 1 the search is swapping a longer hash match for
    // a shorter rep match; above 1 its matches ARE the long ones. Every material
    // loser sits at 0.83-0.87 (jsonlog 0.87, smallmsg 0.83) and every material
    // winner at >= 1.14 (mozilla 1.14, xml 1.52, samba 1.87).
    let rep_on = rep_search_on(tables.rep_yield, params.strategy)
        || tables.rep_probe == 0
        || tables.rep_len_ratio >= rep_len_min();
    tables.rep_probe = if tables.rep_probe == 0 {
        REP_PROBE_PERIOD
    } else {
        tables.rep_probe - 1
    };
    let ut = (tables.pack_tags || !tables.tags.is_empty())
        && tag_enabled()
        && tables.tag_yield >= tag_min();
    // The GATE 6 re-probe countdown ticks HERE, not in `find_fast_impl`'s tail:
    // the pipelined loop returns early, so a countdown in the tail stops
    // advancing exactly when the gain term has the gate shut -- a one-way latch
    // that no threshold can open. `mozilla` and `samba` lost their -2.85% and
    // -6.03% to this, identically at every threshold, which is what gave it
    // away: a real threshold effect moves when the threshold moves.
    // ffanat census: WHICH monomorphisation serves the traffic?
    //
    // REWRITTEN FOR W4/W5. This used to mirror a 70-arm tree and classify on
    // `(fast_spec_enabled, ut, rep_on, pipe_on, s0)`. That tree is gone: the
    // dispatch is now `(ut, rep_on)` and nothing else, because those are the
    // only two keys that still choose a monomorphisation. The census MUST
    // mirror the dispatch -- the comment the old version carried says exactly
    // why ("the first version kept its labels when the dispatch gained arms,
    // and read stale"), and that failure mode is symmetric: it reads stale
    // when the dispatch LOSES arms too.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        let idx = match (ut, rep_on) {
            (false, false) => 0usize,
            (true, false) => 1,
            (false, true) => 2,
            (true, true) => 3,
        };
        FF_ARM[idx].fetch_add(1, Relaxed);
    }
    // W5: THE 70-ARM DISPATCH TREE COLLAPSES TO FOUR CALLS.
    //
    // With W4 above, `go!`'s `$h`, `$s` and `$pi` parameters appear NOWHERE in
    // its body -- only in its parameter list. `pipe_on` and `s0` were already
    // passed as ordinary runtime arguments to `find_fast_impl`, and `$h` died
    // with the HLOG axis. So every arm sharing a `(PACKED, REP)` pair expanded
    // to the identical expression, and the tree was 70 arms selecting between
    // four distinct calls -- a `hash_log` match nested inside a four-way tuple
    // match, run once per block, to pick something neither key influenced.
    //
    // Verified mechanically before the cut: across all 20 outer arms, every
    // `go!` consts pair agrees with its arm's `(ut, rep_on)` pattern -- zero
    // mismatches -- so this dispatches to exactly the same monomorphisation
    // the tree did, for every input.
    // W14: and now the match itself goes. W9 and W10 made `REP` and `PACKED`
    // runtime parameters, so these four arms were handing four different
    // literal bools to the SAME monomorphisation -- four inlined call setups
    // and a two-way branch to choose between calls that differ only in two
    // argument registers. The tuple that started this round at 70 arms is one
    // call.
    let r = go!(ut, rep_on);
    // AFTER the call: `find_fast_impl` reads `pair_probe == 0` to force a probe,
    // so ticking beforehand would consume the very first one.
    tables.pair_probe = if tables.pair_probe == 0 {
        PAIR_PROBE_PERIOD
    } else {
        tables.pair_probe - 1
    };
    r
}

/// BRICK 48: keep this finder OUT of `find_sequences_strategy`.
///
/// With every strategy inlined into one body, that function compiled to 4143
/// instructions over a 584-byte frame with **26.2% of instructions touching
/// stack memory** -- the 16 GPRs are exhausted, so the probe loop reloads its
/// invariants (src base, `ilimit`, `hash_shift`, `hash_mask`, table pointer)
/// from the stack on EVERY probe. Neighbouring standalone functions in the same
/// object (`count_match`, `bt_find_best`) spill 0%.
///
/// C's equivalent is a small standalone function that keeps those in registers,
/// which is where our ~3x per-probe cost was going. Splitting restores that.
#[inline(never)]
pub(crate) fn find_fast_impl(
    packed: bool,
    rep: bool,
    wide: bool,
    step_rt: usize,
    pipe_rt: bool,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // W5: the BMI2 branch used to live here, once per monomorphisation. It is
    // now made ONCE at the dispatch (see the `go!` macro), which is what lets
    // the twin tree drop the HLOG axis. This wrapper is the baseline arm only.
    find_fast_impl_inner::<false>(
        packed,
        rep,
        wide,
        step_rt,
        pipe_rt,
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        reps,
    )
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
#[inline(never)]
pub(crate) unsafe fn find_fast_impl_bmi2(
    packed: bool,
    rep: bool,
    wide: bool,
    step_rt: usize,
    pipe_rt: bool,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_fast_impl_inner::<true>(
        packed,
        rep,
        wide,
        step_rt,
        pipe_rt,
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        reps,
    )
}

/// Scratch acquisition + the too-short-block exit of `find_fast_impl_inner`,
/// factored out of the 48x (+8 bmi2) monomorphisation family -- the prologue
/// half of `fast_finder_epilogue`. Runs once per BLOCK.
#[inline(never)]
#[allow(clippy::type_complexity)]
pub(crate) fn fast_finder_prologue(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    block_len: usize,
    mls: usize,
    tables: &mut MatchTables,
) -> Result<(Vec<Seq>, Vec<u8>, usize, bool, bool, bool), (Vec<Seq>, Vec<u8>)> {
    // GATE reads folded from the caller (see the call site): both are pure
    // per-block signals, independent of anything this helper mutates.
    let g_rep1 = pipe_rep1_enabled() && tables.rep_yield <= fast_lazy_threshold();
    let g_pair = tables.rep_yield <= pair_rep_max();
    let reserve = lit_push_enabled();
    // GATE 13 @ L1 DISPATCH. `reserve` still governs the RESERVATION (a separate
    // win worth 1,648 reallocations at L3); this governs only whether the
    // fixed-width copy's guard is worth EVALUATING.
    //
    // Below the threshold the four-condition guard runs and fails on nearly
    // every call: those runs go to `extend_from_slice` either way, so the guard
    // is pure overhead. Turning it off cannot change output -- both paths append
    // the same bytes -- and cannot write more, because the calls it declines
    // were already taking the slow path.
    //
    // Seeded optimistic (`lit_short_share` starts at 1.0) so block 0 always
    // takes the fast path and the gate cannot suppress its own evidence.
    // GATE 13: resolve BOTH decisions once per block -- whether the guard is
    // worth evaluating at all, and how wide the copy should be. 0 = slow path.
    let lp_copy =
        if reserve && (tables.blocks_done == 0 || tables.lit_short_share >= lit_short_min()) {
            lit_width_for(tables)
        } else {
            0
        };
    let seq_guess = (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16);
    // GATE 6 @ L1. These were built FRESH every block, and `lits` asks for
    // `block_len + LIT_PUSH_WIDTH_MAX` = 131,136 B -- above the 128 KiB
    // large-allocation threshold, so it was one VirtualAlloc-class request per
    // block: 64 of them and 8,392,704 B on an 8 MiB frame, named by backtrace.
    // Exactly the defect Gate 6 fixed on `payload`, sitting on the L1 path.
    //
    // Take them from the frame and hand them back in `encode_block`. Replace
    // rather than grow when they are too small: they are cleared here, so a
    // `realloc` would memcpy an allocation that holds nothing live.
    let keep = finder_scratch_enabled();
    let mut seqs = if keep {
        core::mem::take(&mut tables.seq_scratch)
    } else {
        Vec::new()
    };
    seqs.clear();
    if reserve && seqs.capacity() < seq_guess {
        seqs = Vec::with_capacity(seq_guess);
    }
    let mut lits = if keep {
        core::mem::take(&mut tables.lit_scratch)
    } else {
        Vec::new()
    };
    lits.clear();
    if reserve && lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        crate::prof::note_huff_path(10);
        // W20: the SAME range copy the three other exits make, but this one
        // went through a CHECKED slice -- so every monomorphisation carried a
        // bounds test and a panic landing pad for it. `push_lits_range` is the
        // de-checked helper the other three already use, and W12's clamp above
        // is exactly its `from <= to && to <= src.len()` precondition.
        push_lits_range(&mut lits, src, block_start, block_end);
        crate::prof::note_search(0, 0, 0, 0, lits.len() as u64);
        tables.last_nseq = 0;
        return Err((seqs, lits));
    }
    Ok((seqs, lits, lp_copy, reserve, g_rep1, g_pair))
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn find_fast_impl_inner<
    // W1: which ISA twin is running, so the outlined emitter can be selected
    // at compile time instead of re-deciding per match.
    const BMI2: bool,
>(
    // W7: wide arrives at RUNTIME -- see the note in `find_fast`.
    // W9: rep joins WIDE at runtime. It gated the `try_rep1` block -- ~25
    // lines -- and charged a full second copy of the ~1,500-instruction body
    // for it. The test it replaces, `if rep`, is loop-invariant and therefore
    // perfectly predicted, exactly like the `pair` and `maintain_rep1` bools
    // this same loop has always tested per position.
    // W10: packed joins REP and WIDE at runtime. Its whole reach was ONE
    // compare -- `if packed && (e >> 24) as u8 != tag` inside the slot
    // helpers, which already took the layout choice as the runtime `pack`
    // beside it -- and it charged a second copy of the ~1,700-instruction
    // body to fold it.
    packed: bool,
    rep: bool,
    wide: bool,
    step_rt: usize,
    // W16: `PIPE` was a const-generic axis used by EXACTLY ONE per-BLOCK test
    // (`if PIPE && !pair`) -- and it doubled the whole monomorphisation tree to
    // do it. A per-block bool costs one branch per 128 KiB; the axis cost half
    // the copies of the largest function in the library.
    pipe_rt: bool,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = params.min_match.max(3) as usize;
    // W12: state the block invariant ONCE, per block, so the body does not
    // pay for re-proving it per copy.
    //
    // `block_start <= block_end <= src.len()` holds for every caller, but
    // nothing in the signature says so, so LLVM re-derived it nowhere and kept
    // a bounds test AND a panic landing pad on the early-return slice below --
    // one per monomorphisation, in the function with the most of them. The two
    // clamps are no-ops on every real call and cost two cmovs per BLOCK; they
    // buy the range facts for the whole body.
    debug_assert!(block_start <= block_end && block_end <= src.len());
    let block_end = block_end.max(block_start).min(src.len());
    crate::prof::note_scratch(2);
    // Reserve both scratch buffers up front. They were `Vec::new()` and grew by
    // doubling from zero every block: nci runs ~4k sequences per 128 KiB block,
    // so `seqs` alone re-copied ~95 KiB per block. The `lits` slack also makes
    // the fixed-width literal push in `emit_fast_seq` always eligible.
    let block_len = block_end.saturating_sub(block_start);
    // Size `seqs` from what the previous block actually produced (+25% slack),
    // capped by the structural maximum of one sequence per `mls` bytes. A flat
    // fraction over-reserves badly on sparse-match content.
    // `RZSTD_LIT_PUSH=0` restores the pre-brick-38 shape so both arms can be
    // measured in ONE interleaved session (codec-measurement 3); the flag is
    // resolved once per block, never inside the probe loop.
    // SCRATCH ACQUISITION AND THE EMPTY-TAIL EXIT, as ONE non-generic call.
    // Same reasoning as `fast_finder_epilogue` one screen down: none of this
    // depends on the const generics, and it was stamped into every one of the
    // 48+8 copies. `Err` carries the degenerate tail-block result.
    let (mut seqs, mut lits, lp_copy, _reserve, g_rep1, g_pair) =
        match fast_finder_prologue(src, block_start, block_end, block_len, mls, tables) {
            Ok(t) => t,
            Err(out) => return out,
        };
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    let mut ip = block_start;
    // ffanat: take the table out of `MatchTables` so its data pointer is a
    // LOCAL for the whole loop. The asm showed it spilled and reloaded from the
    // stack three times per iteration; `src` was already register-resident
    // (brick 48) and the table never got the same fix. Handed back at every
    // exit below.
    let mut hash_v = core::mem::take(&mut tables.hash);
    let mut tags_v = core::mem::take(&mut tables.tags);
    let pack = tables.pack_tags;
    // Guard unification (see the dispatch): wide implies pack, so in wide
    // copies this is const-true -- the slot helpers' pack branches fold and
    // `tags_v` is provably untouched.
    debug_assert!(!wide || pack);
    let pack_eff = if wide { true } else { pack };
    // W8: hoisted for the slot primitives -- see `fast_slot_swap`.
    // W8 -- CORRECTNESS, found by the debug suite. `tags_v` is `mem::take`n
    // out of `tables.tags` ABOVE, so `tables.tags` is empty from that line on
    // and this read answered `false` unconditionally. The slot helpers then
    // skipped the tag array on every frame that uses it -- the >= 16 MiB and
    // STREAMING route -- silently disabling Gate 7's filter there. The 8 MiB
    // identity boards could not see it: every frame they compress is under
    // the packed bound, where the array is legitimately absent.
    //
    // Ask the array that is actually in play.
    let tags_live = !tags_v.is_empty();
    // BRICK 51: `probes`/`hits` feed ONLY `note_search`, which is a no-op
    // without the `profile` feature (their other consumers, `last_hit_rate` and
    // `tag_latch`, were write-only dead state left by the brick-41 revert).
    // Register pressure had spilled `probes` to the stack, so the shipping build
    // was paying a read-modify-write to MEMORY on every probe to feed nothing.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut hits = 0u64;
    let mut rep_hits = 0u64;
    // GATE 2 re-denomination: the benefit is rep MATCH BYTES, the cost is one
    // `try_rep1` per POSITION. `rep_yield` prices hits per SEQUENCE, which has
    // nothing to do with the cost -- the same error `pair_gain` had before Gate
    // 6 was re-denominated into bytes-per-probe.
    let mut rep_probes = 0u64;
    let mut rep_bytes = 0u64;
    // GATE 7 feedback, as LOCALS. These were two unconditional atomic fetch_adds
    // inside `fast_probe`, i.e. two read-modify-writes on shared cache lines in
    // the hottest loop in the encoder, on EVERY probe -- not gated behind COUNT,
    // because `tag_yield` is a shipped dispatch input and genuinely needs them.
    // As locals they cost a register add and are summarised once per block.
    let mut cand = (0u64, 0u64);
    // Probe density. The bit accountant showed our size gap vs C is entirely
    // LITERALS, because C finds more matches -- and we probe only ~0.259
    // positions/byte against C's ~1.0. `step0 = 1` matches C's density.
    // Brick 39 made each probe substantially cheaper, so this trade is worth
    // re-testing. `RZSTD_STEP0` overrides (default 2 = pre-existing).
    // BRICK 55: `step0` is live in the hot advance (`ip + step0 + ..`). The
    // pipelined loop only runs when `!pair`, i.e. `step0 <= 2`, so the default
    // (2) is worth specializing -- it folds into the address arithmetic and
    // frees the register it was holding. `STEP == 0` keeps the runtime path.
    // STEP == 0 is the runtime arm, and its value MUST come from the caller.
    // `find_fast` already derives the step from Gate 6's route (route 1 asks for
    // step 1) and from `target_length`; recomputing it here from
    // `step0_default()` threw the route away on every arm that passes STEP = 0
    // -- which is ALL of Gate 7's tag arms.
    //
    // That is the whole of Gate 7's non-byte-identity. The tag COMPARE is exact
    // (a tag is a function of the same 4 bytes `fast_probe` compares, so a
    // mismatch implies no 4-byte match -- measured 0 false rejections in
    // 2,111,991 on sao, 1,428,044 on mozilla). What differed was that switching
    // the filter on switched the ARM, and the arm silently downgraded a
    // step-1-routed block to step 2. Pinning the filter on cost dickens +7.3%,
    // samba +5.7%, mr +2.4% -- exactly the corpora Gate 6 routes to step 1.
    // W15: `HLOG` and `STEP` are gone from the signature. Both were
    // instantiated as 0 at EVERY site once W4/W5 landed, so `if HLOG != 0` and
    // `if STEP != 0` were dead branches the compiler folded away -- zero
    // instructions, but two const parameters that read as live specialisation
    // axes to anyone auditing this signature next.
    let step0 = step_rt;
    // Pair-search ip+1 only when step skips it (`--fast=4`, step 5). At step 2
    // that doubles incomp probes for no ratio. Do not grow step without the pair
    // (that blew --fast=4 ratio 0.845 -> 1.272).
    //
    // Gate 6 (gg-matchfind): forceable so the pair search can be given its own
    // truth table INDEPENDENTLY of step0, which is the only way to tell the two
    // apart -- they are the same physical decision reached by two switches.
    // GATE 6 @ L1 -- DISPATCH. The pair search probes `ip+1` as well as `ip`.
    //
    // `step0 > 2` never fires at L1: target_length is 0 there, so step0 is 2 and
    // the preset variable cannot activate. The capability was therefore dead at
    // the level it helps most. Forced on, all 18 at L1:
    //
    //   nci -13.243%  mozilla -9.665%  reymont -9.029%  samba -8.013%
    //   xml -7.813%   webster -7.747%  dickens -7.440%  ooffice -7.427%
    //   osdb -5.132%  mr -3.024%   ... TOTAL -4.809%
    //   versions-16m +10.553%   jsonlog-16m +0.178%
    //
    // A sign flip, so it is dispatched rather than constant. `versions-16m` is
    // the corpus Gate 1 already routes to Lazy for being near-copy content, and
    // `rep_yield` separates it: the pair search re-finds matches the repcode
    // path already has, so on rep-dominated content it spends probes to emit a
    // worse parse.
    // A STACKED SECOND VARIABLE WAS TESTED AND REFUTED: `pair_gain`, the share
    // of the previous block covered by pair matches. On corpus MEANS it looked
    // separable -- jsonlog 0.3203 above every winner, mozilla highest at 0.3105
    // -- so a threshold at 0.315 should have excluded only the loser. Per BLOCK
    // the distributions overlap, and it gated mozilla off across much of its
    // input: -9.648% collapsed to -0.133% and the total halved from -4.778% to
    // -2.456% while recovering jsonlog's 0.177%.
    //
    // Third occurrence of this error in the campaign (Gate 1's rep_yield
    // threshold, offset_concentration, and this): A MEAN-LEVEL GAP BETWEEN TWO
    // CORPORA IS NOT EVIDENCE THAT A PER-BLOCK THRESHOLD SEPARATES THEM.
    // GATE 6 DISPATCH, two variables:
    //   rep_yield <= 0.7   -- repcodes do not already cover this content
    //   pair_gain >= T     -- the pair search is actually EARNING its probes
    // The first alone shipped a +28.9% mean time cost for -5.85% size, with
    // x-ray paying 19.8% for 0.02%. The second is what prices the trade.
    // W17: `let probe = tables.pair_probe == 0;` was computed here and
    // discarded below (`let _ = probe;`). The route decision moved to
    // `find_fast` and this read never followed it.
    let route = tables.pair_route;
    // Frame-constant, so it is decided ONCE here rather than tested per match.
    // `maintain_rep1` and the pair gate arrive from the PROLOGUE call now:
    // three knob atomics and their float compares were stamped per copy for
    // per-block decisions.
    let maintain_rep1 = g_rep1;
    // The route is decided in `find_fast` (it also selects the step, which must
    // be known before the specialised body is chosen). `rep_yield` still vetoes:
    // on rep-dominated content the pair search re-finds what the repcode path
    // already has and emits a worse parse.
    let pair = step0 > 2 || (route == 2 && g_pair);
    let mut pair_bytes = 0u64;
    let mut pair_probes = 0u64;
    let lowest = block_start.saturating_sub(window).max(tables.frame_start);
    let frame_start = tables.frame_start;
    // Local repeat-offset state, mirroring C's `offset_1`/`offset_2`. A repcode
    // match leaves them unchanged; a normal match shifts them.
    let mut rep1 = reps[0] as usize;
    // Shift from the table's OWN clamped hash_log -- never from `params`.
    //
    // BRICK 54: when `HLOG` is specialized (non-zero) this folds to a compile-
    // time immediate, so the variable shift `shrl %cl, %edx` becomes `shrl $n`
    // -- no register held for the shift amount, no `mov` into `%cl`, and one
    // fewer value competing for the 16 GPRs.
    // W13: `hash_shift` was computed here and immediately discarded
    // (`let _ = hash_shift;`) -- every consumer takes `f_shift` below. It read
    // `tables.hash_log` through the `&mut` to do it.
    // ffanat hash-width: one spec, hoisted per block, consumed by EVERY hash
    // site in this function and by the end-fill it calls -- the writers move
    // together or priming poisons (190ad8b).
    // ffanat hash-width: ONE spec per block, consumed by every hash site in
    // this function and the end-fill it calls. PROTECTION FOR versions-16m IS
    // AN OPEN GATE CELL, and two designs are already REFUTED -- record them so
    // they are not retried: (1) a per-BLOCK rep_yield dispatch made it WORSE
    // (+14.8% -> +34.6%; mixed keys poison the shared table); (2) a one-way
    // per-frame latch with a table clear ALSO made it worse (+17.2% at L1, and
    // it degraded L2's versions from +0.5% to +4.4%). The corpus's hash path
    // sees only ~2K candidates on 8 MiB -- the loss is DISPATCH COUPLING
    // (different early matches shift rep_yield/rep_run and break the repcode
    // chain), not the key itself, which is why key-side protection fails.
    // W14: of the three `FastHash` fields this built, release code consumed
    // exactly ONE -- `mask`, and only in wide copies. `shift` was dead (see
    // `f_shift`) and `wide` fed a `debug_assert!` alone.
    //
    // The legacy arm is dead in wide copies too: `wide_block` requires
    // `!fast_hash_legacy`, so a wide copy can never be on the legacy key. That
    // makes the whole conditional collapse to the spec call in the only copies
    // that read it.
    debug_assert!(!wide || !tables.fast_hash_legacy);
    // Scalarized AND const-moded: wide is a monomorphisation axis, so the
    // per-position mode branch is gone and specialised copies emit the shift
    // as an immediate. Only the mask (a function of runtime `mls`) stays in a
    // register.
    // W7 -- A LATENT SHIFT-OVERFLOW, found by running the DEBUG suite.
    //
    // `fh` is built from `fast_hash_spec`, whose `wide` is
    // `enabled && (5..=8).contains(&mls)`. `wide` -- the monomorphisation
    // axis -- is that AND `!fast_hash_legacy` AND `tables.pack_tags`. The
    // `pack_tags` term is missing from `fh`, so on any frame without a
    // pledged length (streaming, prefix) with `mls` in 5..=8, `fh.wide` is
    // TRUE while `wide` is FALSE.
    //
    // `f_mask` and `f_wide` already take `wide`, so they were fine. `f_shift`
    // did not: on the runtime-`hash_log` arm it took `fh.shift`, which in
    // that state is `64 - hash_log`. The non-wide hash path then evaluates
    // `u32 >> (64 - hash_log)` -- a shift of 32 or more on a 32-bit value,
    // which is UB in Rust and poison in LLVM IR. It has been invisible
    // because x86 masks shift counts to 5 bits, and
    // `(64 - hash_log) & 31 == 32 - hash_log` for every reachable
    // `hash_log`, so the hardware silently computed the right index.
    //
    // Taking the shift from `wide`, like its two siblings, is byte-identical
    // on x86 by that same identity -- and defined everywhere.
    let f_wide = wide;
    let f_mask = if wide {
        fast_hash_spec(mls, tables.hash_log).mask
    } else {
        0
    };
    let hlog_eff = tables.hash_log;
    let f_shift = if wide {
        64u32.saturating_sub(hlog_eff)
    } else {
        32u32.saturating_sub(hlog_eff)
    };
    // THE versions PROTECTION, found where Gate 6 found it for the pair search
    // and step-1: on rep-dominated content, hash matches do not add coverage --
    // they PREEMPT free repcode matches with full-offset ones and break the
    // chain. The legacy 4-byte key self-vetoed there by accident (its
    // promiscuity meant ~389 accepted candidates on the whole of versions);
    // the wide key is precise enough to find 1,838, and that is the entire
    // +14.8% loss. So the veto is on the PROBE, not the key: rep-dominated
    // blocks still STORE every position (the table stays warm and the keys
    // frame-stable -- both key-side designs are refuted in the comment above)
    // and still run the repcode search; they just stop consuming hash
    // candidates. Wide frames only, so the off arm stays byte-identical.
    // Detector: the same signal family as `maintain_rep1` above.
    // THREE refuted designs now, each sharpening the mechanism:
    //   1. per-block key switch (+34.6%): mixed keys poison the shared table.
    //   2. frame latch + clear (+17.2%): key-side protection cannot work,
    //      because the loss is not the key.
    //   3. FULL probe veto (58,178 bytes, 2.4x worse than either pure mode):
    //      the legacy key's ~389 accepted candidates were load-bearing ANCHORS.
    //      And per-position rep-cold hysteresis (27,631) barely moved it,
    //      because the harmful accepts live INSIDE the miss runs where any
    //      hysteresis re-enables.
    // Design #7 (2026-08-20, REMOVED after census): rep-SUBSTITUTION -- swap an
    // accepted far match for the same gram at rep1 distance (one masked
    // compare; offset_value_for encodes offset==reps[0] as repcode 1). Census:
    // of the veto-block accepts on versions, ALL 80 that reached the check had
    // NO gram at rep1 -- zero declined on length -- and every adjudication
    // total was identical to four decimals on both levels. The anchors sit at
    // genuine change points where the far match is the ONLY match; 311 of 391
    // accepts happen BEFORE rep dominance is established. Removed per the
    // OPT_SKIP_FLOOR precedent: built, measured inert, removed.
    //
    // What survives all seven: the harm is RATE-DISTORTION, not chain-breaking.
    // Rep re-locks by CONTENT (src[at] == src[at - rep1]), not alignment, so a
    // consumed match cannot derail it -- but ~1,800 short cross-version matches
    // each pay a FULL offset where literals + rep re-lock were cheaper. The
    // legacy key's promiscuity suppressed exactly those by accident. So the
    // protection is an anchor-length bar on rep-dominated blocks: a hash match
    // is consumed only when it is long enough to pay for its offset.
    //
    // versions went to 58,178 bytes (2.4x worse than either pure mode). The
    // legacy key's ~389 accepted candidates were not noise -- they were the
    // ANCHORS the sticky-rep chain re-synchronised on (Gate 8's sticky mode
    // assumes hash matches punctuate the stream). Remove every anchor and a
    // wrong sticky offset has nothing to heal it; whole blocks fall to
    // literals. So the dispatch is per-POSITION hysteresis: while the rep
    // chain is hitting, hash candidates are not consumed (they would preempt
    // free rep matches with full-offset ones); after FF_REP_COLD consecutive
    // rep misses the probe re-enables and provides the anchor, exactly where
    // the chain needs one.
    // EXPERIMENT KNOB (profile builds only): bar every block, to test whether
    // the pre-rep prefix loss is "marginal matches beating cheaper literals".
    #[cfg(feature = "profile")]
    let bar_all = crate::env_knob_is1("RZSTD_FFBAR_ALL");
    #[cfg(not(feature = "profile"))]
    let bar_all = false;
    // The bar also covers POST-LATCH fast blocks (refutation #5: the re-seed
    // that heals lazy hands fast a dense table whose short matches are the
    // very harm -- 1,824 accepts, broken rep_runs. Lazy keeps the heads; fast
    // is barred from the shorties). `fast_hash_legacy` is only ever set under
    // the wide arm, so the off arm stays byte-identical.
    let veto_block = (wide || tables.fast_hash_legacy)
        && (bar_all || (tables.blocks_done > 0 && tables.rep_yield > fast_lazy_threshold()));
    // W6: acceptance was `ml >= mls` INSIDE the probe plus a `.filter` for
    // `!veto_block || ml >= ff_anchor_ml()` OUTSIDE it -- six instructions per
    // accepted candidate (two compares, two setcc, an and and an or) for two
    // PER-BLOCK constants. They are both lower bounds on the same value, so
    // they compose into one bar tested once. `ff_anchor_ml()` is the constant
    // 16 in release, so this is byte-identical by construction.
    let accept_ml = if veto_block {
        mls.max(ff_anchor_ml())
    } else {
        mls
    };
    // 2-WAY SOFTWARE PIPELINE (brick 39, `RZSTD_MF_PIPE=0` disables).
    //
    // Measured: 26 cycles per probe on webster, while we probe 0.259/byte
    // against C's ~1.0/byte -- the per-probe COST is the gap, not the probe
    // count. Each probe is two dependent random loads (the 256 KiB hash table,
    // then `src[m]` for the u32 compare) with no independent work between them,
    // so the loop is latency-bound.
    //
    // Fix: issue the NEXT probe's hash-table load before consuming the current
    // probe's result, so the two miss latencies overlap. Byte-identical: same
    // probe order, same stores, same results -- only the issue order moves.
    // The store `hash[h0] = ip+1` still precedes the next read, so when the
    // next slot aliases the current one (`h1 == h0`) the just-stored value is
    // forwarded by hand rather than re-read.
    //
    // Only the non-`pair` path is pipelined (`step0 == 2`, i.e. every level
    // whose `target_length` is 0). `--fast=N` keeps the original loop.
    // BRICK 59: `pipe_enabled()` was a RUNTIME check, so every monomorphization
    // carried BOTH the pipelined and the non-pipelined loop. That doubles the
    // function, and a function this large is why LLVM spills the src base in
    // the prologue and rematerializes it on every probe even with six
    // callee-saved registers idle. As a const, the shipping copy contains only
    // the loop it actually runs.
    // Read ONCE per block, never per position -- see the -37% that an env
    // lookup inside the DP loop cost at L19.
    // ffanat release-asm read: `accel` is the constant 7 for Fast unless the
    // RZSTD_ACCEL bench pin is set, yet it was computed, spilled, reloaded from
    // the stack, and `shrq %cl`-shifted PER POSITION. Release builds take the
    // constant (immediate shift, no CL, no slot); the pin stays available under
    // `profile`, the same split EQLEN_ARM documents ("present ONLY under
    // --features profile").
    let accel = if cfg!(feature = "profile") {
        accel_shift_for(params.strategy)
    } else {
        7
    };
    // W1: hoisted for `fill_fast_after_match` -- see its `ends` parameter.
    let f_ends = dfast_fill_ends();
    // W1: every per-block invariant the emitter needs, gathered once. `pack_eff`
    // (not `pack`) so wide copies keep the const-true fold -- see W4.
    let ectx = FastEmitCtx {
        src,
        pack: pack_eff,
        f_wide,
        f_mask,
        f_shift,
        ilimit,
        frame_start,
        w: lp_copy,
        tags_live,
        ends: f_ends,
    };
    if pipe_rt && !pair && ip <= ilimit {
        if COUNT {
            FF_PIPE_BLOCKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        // INSTRUMENT DEFECT: `mm_total` is declared AFTER this block's
        // `return`, so MM_TOTAL only ever counted the NON-pipelined loop --
        // 58% of blocks, and 6.2% of them on the seven corpora that run this
        // path 93.8% of the time. 4.41's position ledger was an undercount.
        let mut pipe_pos = 0u64;
        let (mut ff_made, mut ff_used) = (0u64, 0u64);
        let (mut h0, mut g0) = fast_hash_tag::<true>(src, ip, wide, f_mask, f_shift);
        let mut m0 = fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h0, g0);
        loop {
            if COUNT {
                pipe_pos += 1;
            }
            if COUNT {
                probes += 1;
            }
            if COUNT && packed {
                let raw = fast_slot_raw(&hash_v, pack_eff, h0);
                if m0 == 0 && raw != 0 {
                    if fast_probe(&mut (0, 0), src, raw, ip, window, lowest, mls, block_end)
                        .is_some()
                    {
                        TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
            }
            fast_slot_store(&mut hash_v, &mut tags_v, pack_eff, tags_live, h0, ip, g0);
            if rep {
                // ffanat release-asm read: this unconditional per-position
                // increment was one of six spilled u64 locals -- `incq (%rbp)`
                // per miss in SHIPPING builds -- and its only consumers are the
                // COUNT-gated REP_PROBES publishes. Instrument, so gated.
                if COUNT {
                    rep_probes += 1;
                }
                if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end, ilimit) {
                    rep_hits += 1;
                    rep_bytes += ml as u64;
                    if COUNT {
                        hits += 1;
                    }
                    let mstart = ip + 1;
                    crate::prof::note_huff_path(11);
                    push_literals(&mut lits, src, anchor, mstart, lp_copy);
                    crate::prof::note_huff_path(13);
                    seqs.push(Seq {
                        litlen: (mstart - anchor) as u32,
                        matchlen: ml as u32,
                        offset: rep1 as u32,
                    });
                    ip = mstart + ml;
                    anchor = ip;
                    if ip > ilimit {
                        break;
                    }
                    // Brick 52: shift alone bounds the index (see `hash4_tag`).
                    //
                    // GATE 7 DEFECT: this used to recompute `h0` INLINE and read
                    // `tables.hash[h0]` directly, leaving `g0` holding the tag of
                    // the PREVIOUS position and bypassing `load_fast`. Harmless
                    // while no tag exists -- `g0` is always 0 then, so the stale
                    // value is never compared -- but it silently pairs a fresh
                    // hash with a stale tag the moment one does, rejecting VALID
                    // candidates on the repcode path.
                    //
                    // That single asymmetry cost versions-16m a CONSTANT 2,475
                    // bytes and made the tag filter look non-byte-identical,
                    // which is why the packed representation was blamed and
                    // removed. The representation was fine; this caller was not.
                    let (nh, ng) = fast_hash_tag::<true>(src, ip, wide, f_mask, f_shift);
                    h0 = nh;
                    g0 = ng;
                    m0 = fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h0, g0);
                    continue;
                }
            }
            // Next position, and its table load issued NOW -- this is the whole
            // point of the brick.
            let nip = ip + step0 + ((ip - anchor) >> accel);
            if COUNT && nip <= ilimit {
                ff_made += 1;
            }
            let (h1, g1, m1) = if nip <= ilimit {
                let (h, g) = fast_hash_tag::<true>(src, nip, wide, f_mask, f_shift);
                // The store above may have just overwritten this slot, so the
                // value is forwarded by hand rather than re-read.
                //
                // IT MUST MIRROR `load_fast` EXACTLY. `load_fast::<packed>`
                // consults the tag ONLY when packed; with packed = false -- the
                // SHIPPING Fast configuration -- it returns the raw slot and the
                // tag is irrelevant. This forward compared tags unconditionally,
                // so whenever the next position's hash aliased the current one
                // (`h == h0`) with a different tag it returned 0 and DISCARDED a
                // candidate the non-pipelined loop finds. Pure ratio loss, worst
                // on the highest hash-reuse content: nci -11.93%, xml -0.84%.
                //
                // Same defect class as 190ad8b, mirrored: there the STORE was
                // gated differently from the compare; here the FORWARD applies a
                // compare the LOAD does not.
                let v = if h == h0 {
                    if !packed || g == g0 {
                        (ip as u32).wrapping_add(1)
                    } else {
                        0
                    }
                } else {
                    fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h, g)
                };
                (h, g, v)
            } else {
                (0usize, 0u8, 0u32)
            };
            if let Some((m, ml)) = if wide {
                fast_probe_wide::<true>(
                    &mut cand, src, m0, ip, window, lowest, accept_ml, f_mask, block_end,
                )
            } else {
                fast_probe(&mut cand, src, m0, ip, window, lowest, accept_ml, block_end)
            } {
                if COUNT {
                    hits += 1;
                }
                // W3, pipelined twin -- see the note in the main loop.
                let found = ip;
                ip = emit_fast_seq::<BMI2>(
                    &ectx,
                    &mut hash_v,
                    &mut tags_v,
                    &mut seqs,
                    &mut lits,
                    anchor,
                    found,
                    m,
                    ml,
                );
                anchor = ip;
                // The non-pipelined loop does this after EVERY emitted match;
                // this loop did not, so `rep1` stayed frozen at its block-entry
                // value and every `try_rep1` tested a STALE offset for the whole
                // block. The pipeline is documented as byte-identical to the
                // main loop -- it was not, and the gap was pure ratio: with both
                // loops doing identical work, nci -11.93%, xml -0.84%,
                // jsonlog -0.74%, sao -0.14%.
                // GATE 8 DISPATCH -- `rep1` maintenance in the pipelined loop.
                //
                // This loop never maintained `rep1` at all, so it silently ran a
                // STICKY REPCODE: the block-entry offset held for the whole
                // block. That broke the loop's documented byte-identity with the
                // non-pipelined loop (nci -11.93% before the fix), but on
                // constant-stride content the stale offset is the RIGHT one and
                // committing to each match's offset breaks the chain.
                //
                // Priced across all 18 at L1, maintain vs sticky:
                //   size  +0.098% total, and ALL of it is versions-16m +20.54%
                //   time  -0.80% mean (ooffice -9.33%, mr -5.47%)
                // A sign flip on one axis, so it is dispatched -- on `rep_yield`,
                // the signal Gate 1 already maintains for exactly this content
                // class (versions 0.9778 against a real maximum of mr 0.4949).
                if maintain_rep1 {
                    rep1 = found - m;
                }
                if ip > ilimit {
                    break;
                }
                let (nh, ng) = fast_hash_tag::<true>(src, ip, wide, f_mask, f_shift);
                h0 = nh;
                g0 = ng;
                m0 = fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h0, g0);
                continue;
            }
            if nip > ilimit {
                break;
            }
            if COUNT {
                ff_used += 1;
            }
            ip = nip;
            h0 = h1;
            g0 = g1;
            m0 = m1;
        }
        // THE PIPELINED ARM'S PER-BLOCK TAIL, as one non-generic call -- the
        // same treatment as `fast_finder_epilogue` on the main tail, and worth
        // the same multiplier: this return was stamped into every one of the
        // 48+8 copies, and its `push_lits_range` was the `memcpy` call the asm
        // attribution kept finding twice per copy.
        fast_pipe_epilogue(
            tables, src, &seqs, &mut lits, anchor, block_end, rep, rep_hits, rep_bytes, rep_probes,
            cand, probes, hits, pipe_pos, ff_made, ff_used, hash_v, tags_v,
        );
        return (seqs, lits);
    }
    let (mut mm_total, mut mm_miss) = (0u64, 0u64);
    while ip <= ilimit {
        if COUNT {
            mm_total += 1;
        }
        if COUNT {
            probes += 1;
        }
        let (h0, g0) = fast_hash_tag::<true>(src, ip, wide, f_mask, f_shift);
        // W7: one slot touch instead of a load and a store that each branch
        // on `pack`. The store's value and position are unchanged, and it
        // still precedes the pair probe -- only the two `pack` tests merge.
        let m0 = fast_slot_swap(
            packed,
            &mut hash_v,
            &mut tags_v,
            pack_eff,
            tags_live,
            h0,
            ip,
            g0,
        );
        if COUNT && packed {
            // Gate 7 is recorded byte-identical: a tag mismatch should imply the
            // 4 bytes differ, so `fast_probe` would have rejected the candidate
            // anyway. Count the cases where it would NOT have.
            let raw = fast_slot_raw(&hash_v, pack_eff, h0);
            if m0 == 0 && raw != 0 {
                if fast_probe(&mut (0, 0), src, raw, ip, window, lowest, mls, block_end).is_some() {
                    TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
        // GATE 6 SPEED: issue the PAIR probe's load HERE, next to the main
        // probe's, instead of after `fast_probe` has consumed `m0`.
        //
        // The two loads hit different slots and are independent, but in program
        // order the second was issued only once the first had been consumed, so
        // the two cache misses SERIALIZED -- the pair search paid two full miss
        // latencies per position instead of two overlapped ones. That is the
        // same latency problem the pipelined loop exists to solve; it was just
        // never applied to this path.
        //
        // BYTE-IDENTICAL: `store_fast(h0, ip, g0)` still precedes it (so an
        // aliasing `h1 == h0` observes the same value it did before), and
        // nothing between here and the pair branch writes the table -- the rep
        // and match paths both `continue`. Only the issue order moves.
        let pair_pre = if pair && ip < ilimit {
            let (h1, g1) = fast_hash_tag::<false>(src, ip + 1, wide, f_mask, f_shift);
            Some((
                h1,
                g1,
                fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h1, g1),
            ))
        } else {
            None
        };
        if rep {
            if COUNT {
                rep_probes += 1;
            }
            if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end, ilimit) {
                rep_hits += 1;
                rep_bytes += ml as u64;
                if COUNT {
                    hits += 1;
                }
                let mstart = ip + 1;
                crate::prof::note_huff_path(11);
                push_literals(&mut lits, src, anchor, mstart, lp_copy);
                crate::prof::note_huff_path(13);
                seqs.push(Seq {
                    litlen: (mstart - anchor) as u32,
                    matchlen: ml as u32,
                    offset: rep1 as u32,
                });
                ip = mstart + ml;
                anchor = ip;
                continue;
            }
        }
        if let Some((m, ml)) = if wide {
            fast_probe_wide::<true>(
                &mut cand, src, m0, ip, window, lowest, accept_ml, f_mask, block_end,
            )
        } else {
            fast_probe(&mut cand, src, m0, ip, window, lowest, accept_ml, block_end)
        } {
            if COUNT {
                hits += 1;
            }
            // W3: `rep1` came back through `seqs.last()` -- a length load, an
            // emptiness branch and a `u32` reload of a value this frame already
            // holds. The emitter's back-extension walks `ip` and `mm` DOWN
            // TOGETHER, so the offset it pushes (`ip - mm`) is invariant under
            // that walk and equals `found - m` at entry. Same value, no reload,
            // and it drops the only reason this arm touched `seqs` at all.
            let found = ip;
            ip = emit_fast_seq::<BMI2>(
                &ectx,
                &mut hash_v,
                &mut tags_v,
                &mut seqs,
                &mut lits,
                anchor,
                found,
                m,
                ml,
            );
            anchor = ip;
            // Same decision as the pipelined loop -- see GATE 8 above. Guarding
            // only ONE loop would make the heuristic a property of which loop
            // ran, which is exactly the byte-identity break this gate exposed.
            if maintain_rep1 {
                rep1 = found - m;
            }
            continue;
        }
        if pair {
            let ip1 = ip + 1;
            if ip1 <= ilimit {
                if COUNT {
                    probes += 1;
                }
                pair_probes += 1;
                if COUNT {
                    use core::sync::atomic::Ordering::Relaxed;
                    if m0 == 0 {
                        PAIR_M0_EMPTY.fetch_add(1, Relaxed);
                    } else {
                        PAIR_M0_LIVE.fetch_add(1, Relaxed);
                    }
                }
                if COUNT {
                    PAIR_PROBES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                // Already issued above, next to the main probe's load.
                let (h1, g1, m1) = match pair_pre {
                    Some(v) => v,
                    None => {
                        let (h, g) = fast_hash_tag::<false>(src, ip1, wide, f_mask, f_shift);
                        (
                            h,
                            g,
                            fast_slot_load(packed, &hash_v, &tags_v, pack_eff, tags_live, h, g),
                        )
                    }
                };
                if COUNT && packed {
                    let raw = fast_slot_raw(&hash_v, pack_eff, h1);
                    if m1 == 0 && raw != 0 {
                        if fast_probe(&mut (0, 0), src, raw, ip1, window, lowest, mls, block_end)
                            .is_some()
                        {
                            TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                        TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
                fast_slot_store(&mut hash_v, &mut tags_v, pack_eff, tags_live, h1, ip1, g1);
                // A THIRD VARIABLE WAS TESTED AND REJECTED: the pair match's
                // LENGTH. `versions-16m` sits at exactly +10.55% for every
                // minimum from 0 to 24, while the winners degrade badly (total
                // -4.809% -> -1.748% at 24). Its pair matches are all LONG, so
                // they are not marginal candidates taken cheaply -- they are
                // genuine long matches whose commitment breaks the repcode
                // chain. A length filter cannot separate a good long match from
                // a harmful one, because the harm is a property of the CONTENT
                // (repcode already covers that span) and not of the candidate.
                // That is why `rep_yield` is the right and sufficient variable.
                if let Some((m, ml)) = (if wide {
                    fast_probe_wide::<false>(
                        &mut cand, src, m1, ip1, window, lowest, accept_ml, f_mask, block_end,
                    )
                } else {
                    fast_probe(
                        &mut cand, src, m1, ip1, window, lowest, accept_ml, block_end,
                    )
                })
                .filter(|&(_, ml)| !veto_block || ml >= ff_anchor_ml())
                {
                    if COUNT {
                        use core::sync::atomic::Ordering::Relaxed;
                        if m0 == 0 {
                            PAIR_HIT_EMPTY.fetch_add(1, Relaxed);
                            PAIR_BYTES_EMPTY.fetch_add(ml as u64, Relaxed);
                        } else {
                            PAIR_HIT_LIVE.fetch_add(1, Relaxed);
                            PAIR_BYTES_LIVE.fetch_add(ml as u64, Relaxed);
                        }
                        PAIR_HITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        PAIR_BYTES.fetch_add(ml as u64, core::sync::atomic::Ordering::Relaxed);
                    }
                    if COUNT {
                        hits += 1;
                    }
                    pair_bytes += ml as u64;
                    ip = emit_fast_seq::<BMI2>(
                        &ectx,
                        &mut hash_v,
                        &mut tags_v,
                        &mut seqs,
                        &mut lits,
                        anchor,
                        ip1,
                        m,
                        ml,
                    );
                    anchor = ip;
                    continue;
                }
            }
        }
        if COUNT {
            mm_miss += 1;
        }
        ip += step0 + ((ip - anchor) >> accel);
    }
    // THE WHOLE PER-BLOCK EPILOGUE IS ONE NON-GENERIC CALL. `find_fast_impl`
    // is monomorphised 48x (plus 8 bmi2 twins), and every copy carried its own
    // stamp of this 87-line tail: the census flushes, the rep/pair EWMAs, the
    // GATE 7/13 signal updates, the tail-literal push and the board hand-back.
    // None of it depends on the const generics except rep and COUNT -- rep is
    // now a runtime bool (per block, free), COUNT is a cfg constant the helper
    // shares. Pure code motion: byte-identical by construction.
    fast_finder_epilogue(
        tables,
        src,
        &seqs,
        &mut lits,
        anchor,
        block_end,
        rep,
        rep_hits,
        rep_bytes,
        rep_probes,
        pair,
        pair_bytes,
        pair_probes,
        cand,
        probes,
        hits,
        mm_total,
        mm_miss,
        hash_v,
        tags_v,
    );
    (seqs, lits)
}

/// The pipelined arm's per-block tail of `find_fast_impl_inner` -- the second
/// of its two exits, factored out of the 48x (+8) family exactly like
/// `fast_finder_epilogue` (the main tail). Runs once per block.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn fast_pipe_epilogue(
    tables: &mut MatchTables,
    src: &[u8],
    seqs: &[Seq],
    lits: &mut Vec<u8>,
    anchor: usize,
    block_end: usize,
    rep: bool,
    rep_hits: u64,
    rep_bytes: u64,
    rep_probes: u64,
    cand: (u64, u64),
    probes: u64,
    hits: u64,
    pipe_pos: u64,
    ff_made: u64,
    ff_used: u64,
    hash_v: Vec<u32>,
    tags_v: Vec<u8>,
) {
    const COUNT: bool = cfg!(feature = "profile");
    crate::prof::note_huff_path(12);
    push_lits_range(lits, src, anchor, block_end);
    let match_bytes: u64 = if cfg!(feature = "profile") {
        seqs.iter().map(|s| u64::from(s.matchlen)).sum()
    } else {
        0
    };
    crate::prof::note_search(
        probes,
        hits,
        seqs.len() as u64,
        match_bytes,
        lits.len() as u64,
    );
    // Decay rather than replace: the FIRST block of a frame has no
    // history to repeat against, so its yield is unrepresentative and a
    // straight assignment latched the search off for the whole frame.
    // Halving gives a ~4-block probe window before it can fall below
    // REP_YIELD_MIN, and one good block restores it immediately.
    let y = if seqs.is_empty() {
        0.0
    } else {
        rep_hits as f32 / seqs.len() as f32
    };
    tables.rep_yield = y.max(tables.rep_yield * 0.5);
    // The pipelined loop returns HERE, before the main tail -- so before this
    // it never refreshed `tag_yield` at all and the old global counters just
    // accumulated across blocks.
    tables.tag_yield = cand_yield(cand);
    let (ls, lm) = lit_shares(seqs);
    tables.lit_short_share = ls;
    tables.lit_mid_share = lm;
    tables.last_nseq = seqs.len();
    // DEFECT FIX: the main tail's `rep_len_ratio` update is BELOW this
    // return, so every pipelined block left Gate 2's second dispatch
    // variable pinned at its 1.0 initial value -- and the gate is `>= 1.0`.
    // See `replen_pipe_fixed`.
    if rep && replen_pipe_fixed() && rep_hits > 0 && !seqs.is_empty() {
        let all_bytes: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        // THREE divisions collapsed to ONE. `rl / al` expands to
        // `(rep_bytes/rep_hits) / (all_bytes/seqs.len())`, which is
        // `(rep_bytes * seqs.len()) / (rep_hits * all_bytes)` -- two
        // multiplies and one `divss` instead of three. The guard moves
        // from `al > 0.0` to the denominator it actually protects.
        let num = rep_bytes as f32 * seqs.len() as f32;
        let den = rep_hits as f32 * all_bytes as f32;
        if den > 0.0 {
            tables.rep_len_ratio = 0.75 * tables.rep_len_ratio + 0.25 * (num / den);
        }
    }
    if COUNT {
        use core::sync::atomic::Ordering::Relaxed;
        MM_TOTAL.fetch_add(pipe_pos, Relaxed);
        REP_PROBES.fetch_add(rep_probes, Relaxed);
        REP_BYTES.fetch_add(rep_bytes, Relaxed);
        REP_HITS_G.fetch_add(rep_hits, Relaxed);
        // The DENOMINATORS must be published on the same path as the
        // numerator. 4.44 added the rep counters here and left these in the
        // main tail only, so `rep_hits / all_seqs` counted two paths over
        // one and read as high as 11,516% -- an impossible ratio that
        // indicted the instrument, not the encoder.
        let mb: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        ALL_MATCH_BYTES.fetch_add(mb, Relaxed);
        ALL_SEQS.fetch_add(seqs.len() as u64, Relaxed);
        FF_SPEC_MADE.fetch_add(ff_made, Relaxed);
        FF_SPEC_USED.fetch_add(ff_used, Relaxed);
    }
    tables.hash = hash_v;
    tables.tags = tags_v;
}

/// The per-block epilogue of `find_fast_impl_inner`, factored out of the 48x
/// (+8 bmi2) monomorphisation family. `#[inline(never)]`: it runs once per
/// BLOCK, and inlined it existed once per COPY.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn fast_finder_epilogue(
    tables: &mut MatchTables,
    src: &[u8],
    seqs: &[Seq],
    lits: &mut Vec<u8>,
    anchor: usize,
    block_end: usize,
    rep: bool,
    rep_hits: u64,
    rep_bytes: u64,
    rep_probes: u64,
    pair: bool,
    pair_bytes: u64,
    pair_probes: u64,
    cand: (u64, u64),
    probes: u64,
    hits: u64,
    mm_total: u64,
    mm_miss: u64,
    hash_v: Vec<u32>,
    tags_v: Vec<u8>,
) {
    const COUNT: bool = cfg!(feature = "profile");
    if COUNT {
        use core::sync::atomic::Ordering::Relaxed;
        MM_TOTAL.fetch_add(mm_total, Relaxed);
        MM_MISS.fetch_add(mm_miss, Relaxed);
    }
    // ffanat full-read: this block was UNGUARDED -- five atomic RMWs plus a
    // full O(nseq) walk of `seqs`, per block, in SHIPPING builds, feeding
    // statics whose only consumers are the take_* bench APIs. The pipelined
    // tail has the same counters correctly inside `if COUNT`; the pair tail
    // never got the guard (ninth neighbour instance). The walk was also
    // DUPLICATED two lines later for `rep_len_ratio` -- now computed once and
    // shared.
    if COUNT {
        use core::sync::atomic::Ordering::Relaxed;
        REP_PROBES.fetch_add(rep_probes, Relaxed);
        REP_BYTES.fetch_add(rep_bytes, Relaxed);
        REP_HITS_G.fetch_add(rep_hits, Relaxed);
        let mb: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        ALL_MATCH_BYTES.fetch_add(mb, Relaxed);
        ALL_SEQS.fetch_add(seqs.len() as u64, Relaxed);
    }
    if rep && rep_hits > 0 && !seqs.is_empty() {
        let all_bytes: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        // Same three-into-one as the pipelined arm above.
        let num = rep_bytes as f32 * seqs.len() as f32;
        let den = rep_hits as f32 * all_bytes as f32;
        if den > 0.0 {
            tables.rep_len_ratio = 0.75 * tables.rep_len_ratio + 0.25 * (num / den);
        }
    }
    // GATE 7: feed this block's measured reject share to the next block's gate.
    tables.tag_yield = cand_yield(cand);
    // GATE 13: and this block's share of literal runs the fixed-width copy can catch.
    let (ls, lm) = lit_shares(seqs);
    tables.lit_short_share = ls;
    tables.lit_mid_share = lm;
    // feed this block's pair coverage to the next block's gate
    // Attribute only when the search actually RAN -- a rejected block measures
    // nothing, and zeroing it there is what would latch the gate shut.
    if pair {
        // BYTES PER PROBE, not bytes per input byte. The cost of this search is
        // one probe; the benefit is the match bytes it covers. Denominating the
        // gain in input bytes prices the benefit against a quantity that has
        // nothing to do with the cost, which is why a 0.05 threshold in those
        // units gated off mozilla and samba (real -2.85%/-6.03% wins) while
        // still admitting content the search does no good on.
        // EWMA, not last-block. Two things make a single block a bad decider:
        // the FIRST block of a frame probes against an EMPTY table and always
        // measures ~0 (a cold reading that would shut the gate for the whole
        // rest of the file), and per-block rates straddle any threshold set
        // from a corpus mean -- `nci` aggregates 8.24 B/probe but individual
        // blocks fall below it, which is the same mean-vs-per-block error this
        // campaign has now made four times.
        let now = pair_bytes as f32 / pair_probes.max(1) as f32;
        tables.pair_gain = 0.75 * tables.pair_gain + 0.25 * now;
    }

    crate::prof::note_huff_path(12);
    push_lits_range(lits, src, anchor, block_end);
    let match_bytes: u64 = if cfg!(feature = "profile") {
        seqs.iter().map(|s| u64::from(s.matchlen)).sum()
    } else {
        0
    };
    crate::prof::note_search(
        probes,
        hits,
        seqs.len() as u64,
        match_bytes,
        lits.len() as u64,
    );
    // Decay rather than replace: the FIRST block of a frame has no
    // history to repeat against, so its yield is unrepresentative and a
    // straight assignment latched the search off for the whole frame.
    // Halving gives a ~4-block probe window before it can fall below
    // REP_YIELD_MIN, and one good block restores it immediately.
    let y = if seqs.is_empty() {
        0.0
    } else {
        rep_hits as f32 / seqs.len() as f32
    };
    tables.rep_yield = y.max(tables.rep_yield * 0.5);
    tables.last_nseq = seqs.len();
    tables.hash = hash_v;
    tables.tags = tags_v;
}

/// C zstd_fast: 4-byte probe then ZSTD_count from +4. `ilimit` keeps ip+4 in-bounds.
/// `match_slot` is the hash-table value (`pos+1`, or 0 = empty).
#[inline(always)]
/// The WIDE probe -- the last piece the mls-hash ship missed. The legacy probe
/// reloads `src[ip]` as a u32 (the wide hash loaded those exact bytes as a u64
/// in the SAME iteration; different widths, so LLVM cannot CSE them), passes a
/// 4-byte compare the mls-keyed table satisfies almost by construction, and
/// then `count_match` re-walks bytes 4..mls. One masked u64 compare settles the
/// whole gram: identical accepted set, identical `ml` (the count walks the same
/// equality run from `mls` instead of 4), and the `ml >= mls` test disappears
/// because the compare IS the proof. `cand` counts move from the 4-byte to the
/// gram compare -- `tag_yield`'s only shipped consumer is `ut` at
/// `tag_min = 0.0`, where the value gates nothing (bench arms that raise
/// RZSTD_TAG_T see the new denomination).
///
/// SAFE mirrors `fast_hash_tag`: callers with `ip <= ilimit` prove `ip + 8 <=
/// block_end`, and `m < ip` carries the same bound for the candidate side.
pub(crate) fn fast_probe_wide<const SAFE: bool>(
    cand: &mut (u64, u64),
    src: &[u8],
    match_slot: u32,
    ip: usize,
    window: usize,
    lowest: usize,
    // W15: dead parameter -- see `fast_probe`.
    accept_ml: usize,
    mask: u64,
    block_end: usize,
) -> Option<(usize, usize)> {
    if match_slot == 0 {
        return None;
    }
    let m = (match_slot as usize) - 1;
    if m < lowest || m >= ip || ip - m > window {
        return None;
    }
    let a = if SAFE {
        debug_assert!(ip + 8 <= src.len());
        crate::simd::load_u64_le(src, ip)
    } else {
        load_u64le_tail(src, ip)
    };
    let b = if SAFE {
        debug_assert!(m + 8 <= src.len());
        crate::simd::load_u64_le(src, m)
    } else {
        load_u64le_tail(src, m)
    };
    let x = a ^ b;
    if x & mask != 0 {
        // Prometheus adjudication (m7-optimize-anatomy §3): `tag_yield`'s only
        // shipped consumer is `ut`'s compare against `tag_min`, which ships
        // 0.0 -- the value gates nothing. Maintained under `profile` only,
        // where the content-signal dump and the RZSTD_TAG_T sweep arms (both
        // profile machinery) still see it. Two register-held u64 adds leave
        // the hottest loop in the encoder.
        if cfg!(feature = "profile") {
            cand.0 += 1;
        }
        return None;
    }
    if cfg!(feature = "profile") {
        cand.1 += 1;
    }
    #[cfg(feature = "profile")]
    FF_CAND4.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "profile")]
    FF_ACCEPT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // W2: the FUSED HEAD, which `fast_probe` has had since the first-word
    // pass and this twin never got. The acceptance test already computed
    // `a ^ b`, and the mask proves bytes 0..mls are equal -- so when the xor
    // is non-zero the first differing byte IS the match length, in a register,
    // with no second load and no call. `mask` covers at most 8 bytes (WIDE
    // requires mls in 5..=8) and `ip <= ilimit = block_end - 8`, so the length
    // is bounded by the block without a clamp. Identical to
    // `mls + count_match_fast(m + mls, ip + mls)` on both arms: a non-zero xor
    // puts the difference at index >= mls, and a zero xor means all eight
    // bytes matched, which is exactly where the continuation starts.
    let ml = if x != 0 {
        (x.trailing_zeros() as usize) >> 3
    } else {
        8 + count_match_fast(src, m + 8, ip + 8, block_end)
    };
    // W6: the wide probe's mask already proves `ml >= mls`, so its only
    // acceptance question is the veto bar -- now the same single compare.
    if ml >= accept_ml {
        Some((m, ml))
    } else {
        None
    }
}

pub(crate) fn fast_probe(
    cand: &mut (u64, u64),
    src: &[u8],
    match_slot: u32,
    ip: usize,
    window: usize,
    lowest: usize,
    // W15: `mls` was a dead parameter here -- the bar is `accept_ml`, which
    // already IS `mls` raised to the veto anchor on blocks that carry it. It
    // was set up at every per-POSITION call site.
    accept_ml: usize,
    block_end: usize,
) -> Option<(usize, usize)> {
    if match_slot == 0 {
        return None;
    }
    let m = (match_slot as usize) - 1;
    if m < lowest || m >= ip || ip - m > window {
        return None;
    }
    // FUSED FIRST-WORD HEAD: with 8-byte room under block_end, ONE u64 pair
    // both gates the candidate (low 32 bits -- the same test the u32 pair
    // made) and answers lengths 4..7 from its high bits, so the commonest
    // accept class never calls out at all. `m + 8` is in bounds because
    // `m < ip` and `block_end <= src.len()`. Byte-identical: the gate is the
    // same equality, and a high-bits length equals 4 + the old tail count.
    let ml = if ip + 8 <= block_end {
        let x = load_u64le(src, m) ^ load_u64le(src, ip);
        if x as u32 != 0 {
            // Profile-only for the same reason as `fast_probe_wide`'s counts:
            // `tag_yield` gates nothing at the shipped `tag_min = 0.0`.
            if cfg!(feature = "profile") {
                cand.0 += 1;
            }
            return None;
        }
        if cfg!(feature = "profile") {
            cand.1 += 1;
        }
        #[cfg(feature = "profile")]
        FF_CAND4.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if x != 0 {
            (x.trailing_zeros() as usize) >> 3
        } else {
            8 + count_match(src, m + 8, ip + 8, block_end)
        }
    } else {
        if load_u32le(src, m) != load_u32le(src, ip) {
            if cfg!(feature = "profile") {
                cand.0 += 1;
            }
            return None;
        }
        if cfg!(feature = "profile") {
            cand.1 += 1;
        }
        #[cfg(feature = "profile")]
        FF_CAND4.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        4 + count_match(src, m + 4, ip + 4, block_end)
    };
    if ml >= accept_ml {
        #[cfg(feature = "profile")]
        FF_ACCEPT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Some((m, ml))
    } else {
        None
    }
}
