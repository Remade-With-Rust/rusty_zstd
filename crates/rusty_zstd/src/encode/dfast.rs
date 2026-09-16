//! The DFast ladder (L3-L4): two hash tables, the short one tag-packed.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a
//! moved item that was private is now `pub(crate)`, so the parent can
//! still name it. That changes who may reference a symbol, not what is
//! emitted for it -- checked, not assumed: the asm board was identical
//! in all thirty-two columns across the move, which matters because
//! this crate builds at the default `codegen-units = 16`, where rustc
//! partitions codegen units BY MODULE and layout can move inlining.

use super::*;
/// Split out for register allocation -- see brick 48 on `find_fast_impl`.
#[inline(never)]
pub(crate) fn find_dfast(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // Fold the hash shift to an immediate for the values the level table
    // actually produces: L3 uses 17, L4 uses 18, smaller inputs pick lower rows.
    // 12..=20 covers every reachable case; the runtime arm is a safety net, not
    // a hot path. `tables.hash_log` is the AUTHORITATIVE clamped value (brick
    // 52) -- `find_dfast` had been reading `params.hash_log` instead, which is
    // the same today only because `compression_params` clamps to the same range.
    // W8: the same BMI2 redundancy `find_fast` carried (see W5). `shrx` takes
    // its shift count from any GPR, so on the twins the HLOG immediate buys
    // nothing -- and DFast pays it TWICE per position (4-byte + 8-byte hash)
    // across five specialised copies. Route the twins to the generic copy and
    // keep brick 54's fold on the baseline arm that still needs it.
    //
    // The ISA choice moves here too, out of the five `find_dfast_impl` bodies.
    // REFUTED HERE, recorded so it is not retried: collapsing this HLOG axis
    // the way W4 collapsed `find_fast`'s measured **+81 crate-wide** (this
    // function -245... -178, and ~+259 elsewhere). `find_fast`'s axis was
    // six-fold over a ~965-instruction body; DFast's five copies had already
    // been largely merged by LLVM, so there was no duplication left to
    // recover and the generic form only cost spills. The same lever is not
    // the same win at a different multiplier -- price the copies, not the
    // pattern.
    macro_rules! go {
        ($h:expr, $p:expr) => {{
            // D6: TWIN RETIRED on its ISA density -- 1,287 instructions of
            // duplicated body converting EIGHTEEN BMI2 ops, 72 per op. Same
            // test that retired the greedy (123/op), lazy (111/op), chain
            // (152/op) and bt-runtime (97/op) twins, and the same precedent:
            // W4/W5/W6 retired the HLOG and (hash_log, chain_log) trees on the
            // argument that `shr %cl` and `shrx` are both one uop on any CPU
            // that HAS BMI2. DFast pays that shift twice per position, which is
            // exactly why 18 conversions is all a whole duplicate body bought.
            let out = find_dfast_impl::<$h, $p>(
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
    // W11: SIX CALL SITES BECOME ONE.
    //
    // A FIRST ATTEMPT AT THIS WAS REFUTED AND THE REFUTATION WAS WRONG; the
    // correction is worth recording because the failure mode is subtle. That
    // attempt changed only the CONST (`find_dfast_impl::<$h>` -> `::<0>`) and
    // measured -178, which read as "LLVM had already merged the copies". It
    // had not. `find_dfast_impl` is `#[inline(always)]`, and inlining happens
    // PER CALL SITE -- six sites inline six bodies even when all six name the
    // identical monomorphisation. Collapsing the const made the six copies
    // identical without making them one.
    //
    // So the `find_fast` lesson (W4) needed its other half (W5): kill the
    // AXIS *and* the DISPATCH. `HLOG` here reaches only
    // `let hlog = if HLOG == 0 { tables.hash_log } else { HLOG };`, after
    // which `hlog` and the `dtag_shift` derived from it travel as ordinary
    // runtime arguments into `dfast_hash_pair`, `hash8` and the fill walk --
    // so six bodies bought two shift immediates, and DFast pays them twice
    // per position (4-byte + 8-byte hash).
    //
    // GATE 5's census (below, kept for the record) established that only
    // hash_log {14..18} are reachable and culled 12/13/19/20. That was the
    // right cut for the wrong axis: reachability decides which SPECIALISATIONS
    // are dead, never whether the specialisation is worth its I-cache.
    //
    // `dfast_spec_enabled()` selected between the specialised arms and the
    // generic one; with no specialised arms left there is nothing to select.
    //
    // BRICK 21: the one axis that IS worth a body -- the tag representation.
    // `packed` reaches the probe (`get_h_tag`/`get_hl_tag` per position), the
    // insert (`put_h_tag`/`put_hl_tag` per position), `dtag_on`, and the
    // after-match fill; the per-position path tested it and kept the
    // array-form state live for nothing. Two bodies, chosen once per block.
    if tables.pack_tags {
        go!(0, true)
    } else {
        go!(0, false)
    }
}

/// GATE 4/5 EXTENDED TO L3 -- the DEFAULT level's finder.
///
/// `find_fast` has been specialised since bricks 46/48/59 into
/// `find_fast_impl<PACKED, REP, HLOG, STEP, PIPE>`, 13 monomorphizations, so its
/// hash shift folds to an IMMEDIATE. `find_dfast` never got that treatment, and
/// it is the finder the shipping DEFAULT (L3/L4) runs: the shift amount was a
/// runtime value feeding TWO hashes on EVERY probe (4-byte + 8-byte), i.e. a
/// variable-count shift twice per position, plus a third in the post-match fill.
///
/// Nine levels were specialised (-7..-1, 1, 2); the one carrying most real
/// traffic was not.
///
/// Byte-identical by construction: `HLOG` takes the value the runtime variable
/// already held, so every hash index is unchanged.
pub(crate) fn find_dfast_impl<const HLOG: u32, const PACKED: bool>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // W8: the ISA branch now lives once in `find_dfast`'s dispatch, which is
    // what lets the twin tree drop the HLOG axis. Baseline arm only.
    find_dfast_impl_inner::<HLOG, PACKED>(src, block_start, block_end, window, params, tables, reps)
}

/// The shipping per-block epilogue of `find_dfast_impl_inner`, factored out of
/// its five inlined HLOG copies. Runs once per block; `#[inline(never)]`.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dfast_finder_epilogue(
    tables: &mut MatchTables,
    src: &[u8],
    seqs: &[Seq],
    lits: &mut Vec<u8>,
    anchor: usize,
    block_end: usize,
    rep_hits: u64,
    spec_used: u64,
    spec_made: u64,
    dpipe: bool,
    nl_probes: u64,
    nl_hits: u64,
    band_worse: u64,
    band_hits: u64,
    probes: u64,
    hits: u64,
) {
    const COUNT: bool = cfg!(feature = "profile");
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * rep_decay())
    };
    tables.rep_probe = if tables.rep_probe == 0 {
        REP_PROBE_PERIOD
    } else {
        tables.rep_probe - 1
    };
    // Optimistic when the probe never fired, so a quiet block cannot latch it
    // off permanently; otherwise the measured hit share, floored at half the
    // previous value so one bad block does not kill it outright.
    // GATE 8 signal: share of speculated loads that were actually CONSUMED. A
    // speculation is discarded whenever the position ends in a match or a rep
    // hit, so match-dense content pays for loads it never uses.
    // T2: these three are DIAGNOSTICS, and leaving them ungated kept `mm_total`
    // live across the whole search loop for no shipping purpose. The gate signal
    // below is computed from `spec_used`/`spec_made` directly, not from the
    // atomics, so gating the atomics costs no dispatch anything. The `nl_probes`
    // block immediately after this one was already gated exactly this way.
    //
    // The DFast hot loop is only 151 instructions but carries 27 stack reloads,
    // 23 of them loop-invariant across 12 slots -- it is short of registers, and
    // what it is spending them on is the gates' own telemetry.
    // Attribute only when the pipeline actually RAN: a block that speculated
    // nothing measures nothing, and scoring it 1.0 would make the gate
    // oscillate on/off every block. EWMA for the same reason Gate 6 needs one --
    // one cold or atypical block must not decide the whole frame.
    if dpipe && spec_made > 0 {
        let now = spec_used as f32 / spec_made as f32;
        tables.dfast_spec_yield = 0.75 * tables.dfast_spec_yield + 0.25 * now;
    }
    // Periodic re-probe, so a block that scores low cannot latch the gate shut
    // for the rest of the frame. This epilogue always runs (the only early
    // return is the empty-block case), unlike `find_fast`'s, where putting the
    // tick in the tail is exactly what latched Gate 6.
    tables.dfast_probe = if tables.dfast_probe == 0 {
        DFAST_PROBE_PERIOD
    } else {
        tables.dfast_probe - 1
    };
    // The mean-ml EWMA below is the SHIPPING consumer of this sum; the atomic
    // publishes are gate-harness diagnostics (`take_dfast_match_stats`,
    // `take_dfast_rep_blocks`) and shipped as EIGHT lock-prefixed RMWs plus a
    // SECOND O(nseq) walk per block. Sum once, publish under `profile` only.
    let mb: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
    {
        let now = if seqs.is_empty() {
            0.0
        } else {
            mb as f32 / seqs.len() as f32
        };
        tables.dfast_mean_ml = if tables.dfast_mean_ml == 0.0 && now == 0.0 {
            0.0
        } else {
            0.75 * tables.dfast_mean_ml + 0.25 * now
        };
    }
    if band_hits > 0 {
        let now = band_worse as f32 / band_hits as f32;
        tables.nl_off_worse = if tables.nl_band_meas == 0 {
            now
        } else {
            0.75 * tables.nl_off_worse + 0.25 * now
        };
        tables.nl_band_meas = tables.nl_band_meas.saturating_add(1);
    }
    tables.nl_band_probe = if tables.nl_band_probe == 0 {
        NL_BAND_PERIOD
    } else {
        tables.nl_band_probe - 1
    };
    tables.next_long_yield = if nl_probes == 0 {
        1.0
    } else {
        (nl_hits as f32 / nl_probes as f32).max(tables.next_long_yield * 0.5)
    };
    push_lits_range(lits, src, anchor, block_end);
    // GATE 13 @ L3 FOLLOW-UP: `find_dfast` READ `last_nseq` to size its `seqs`
    // reservation but never WROTE it -- only `find_fast` did, and L3 never calls
    // `find_fast`. So the field sat at its initial 0 for the whole frame and the
    // guess collapsed to the `+ 64` floor, while DFast emits 5,685-13,763
    // sequences per block: the reservation was ~100x short and `seqs` still grew
    // by realloc (1,648 growths across the corpus).
    //
    // A capacity hint cannot affect output, so this is byte-identical.
    tables.last_nseq = seqs.len();
    note_finder_work(COUNT, probes, hits, seqs, lits);
}

/// The per-block gate decisions of `find_dfast_impl_inner`, resolved ONCE in
/// `dfast_finder_prologue` instead of once per HLOG monomorphisation.
pub(crate) struct DfastGates {
    lp: bool,
    good_ml: usize,
    good_ml2: usize,
    nl_on: bool,
    dstep: usize,
    dpipe: bool,
    lt_on: bool,
}

/// Scratch acquisition + the too-short-block exit of `find_dfast_impl_inner`,
/// factored out of its five HLOG copies. Runs once per block.
#[inline(never)]
#[allow(clippy::type_complexity)]
pub(crate) fn dfast_finder_prologue(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    block_len: usize,
    mls: usize,
    tables: &mut MatchTables,
) -> Result<(Vec<Seq>, Vec<u8>, DfastGates), (Vec<Seq>, Vec<u8>)> {
    let good_ml = nl_cut_for(tables);
    let good_ml2 = dfast_good_ml2();
    let lp = if litpush_hoist_enabled() {
        dfast_litpush_enabled()
    } else {
        lit_push_enabled()
    };
    let nl_on = next_long_enabled() && tables.next_long_yield >= next_long_min();
    let ml = tables.dfast_mean_ml;
    let dstep = if dfast_step_forced() != 0 {
        dfast_step_forced()
    } else if ml == 0.0 || ml >= dfast_ml_min() {
        2
    } else {
        1
    };
    let dpipe = dfast_pipe_enabled()
        && (tables.dfast_probe == 0 || tables.dfast_spec_yield >= dfast_spec_min());
    let lt_on = long_tag_enabled() && (tables.pack_tags || !tables.ltags.is_empty());
    let gates = DfastGates {
        lp,
        good_ml,
        good_ml2,
        nl_on,
        dstep,
        dpipe,
        lt_on,
    };
    let seq_guess = (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16);
    // GATE 6 family: DFast reserved its buffers but still built them fresh every
    // block. `find_fast_impl` takes them from the frame; this never did, so the
    // reservation was paid per block instead of once. Same scratch, same
    // hand-back in `encode_block`.
    let keep = finder_scratch_enabled();
    let mut seqs = if keep {
        let mut v = core::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    if lp && seqs.capacity() < seq_guess {
        crate::copies::add(
            crate::copies::C_SCRATCH_REALLOC,
            seq_guess * core::mem::size_of::<Seq>(),
        );
        seqs = Vec::with_capacity(seq_guess);
    }
    let mut lits = if keep {
        let mut v = core::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    if lp && lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        crate::copies::add(
            crate::copies::C_SCRATCH_REALLOC,
            block_len + LIT_PUSH_WIDTH_MAX,
        );
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        crate::copies::add(crate::copies::C_LIT_PUSH, block_end - block_start);
        lits.extend_from_slice(&src[block_start..block_end]);
        return Err((seqs, lits));
    }
    // BRICK 70: repcode-1 search in DFast.
    //
    // C checks `offset_1` at every position in `_doubleFast` exactly as it does
    // in `_fast`; we had it ONLY in `find_fast`, so L3 -- the SHIPPING DEFAULT --
    // had no repcode search at all. That is the whole of the 4.3x versions-16m
    // hole at L2-L4 (L1/L2 collapse to 0.07x/0.62x with it on, L3/L4 do not move).
    //
    // Dispatched on the same measured yield as brick 67, so content without a
    // constant stride does not pay for a search that cannot hit.
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    Ok((seqs, lits, gates))
}

/// The dfast STRIDE fill (`RZSTD_DFAST_FILL_S`), outlined -- see the call site
/// in `find_dfast_impl_inner` for why it left the loop. Body is the former
/// inline loop verbatim: `put_h_tag`/`put_hl_tag` inlined, same representation.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn dfast_stride_fill(
    tables: &mut MatchTables,
    src: &[u8],
    from: usize,
    stop: usize,
    dfs: usize,
    hash_shift: u32,
    dlong_shift: u32,
    smask: u64,
    flags: (bool, bool, bool),
) {
    let (packed, stag_live, ltag_live) = flags;
    let mut p = from;
    if p >= stop {
        return;
    }
    let hp = tables.hash.as_mut_ptr();
    let hlp = tables.hash_long.as_mut_ptr();
    let tp = tables.tags.as_mut_ptr();
    let ltp = tables.ltags.as_mut_ptr();
    while p < stop {
        let (h, g) = hash4_tag_mls(src, p, hash_shift, smask);
        let h8 = hash8_shift(src, p, dlong_shift);
        debug_assert!(h < tables.hash.len() && h8 < tables.hash_long.len());
        // SAFETY: `h` and `h8` are the hash shifts' own outputs, bounded by the
        // table lengths exactly as the accessors assert; the bases are those
        // tables', taken once above and not resized inside the loop.
        #[allow(unsafe_code)]
        unsafe {
            let v = (p as u32) + 1;
            if packed {
                let w = (v & 0x00FF_FFFF) | (u32::from(g) << 24);
                *hp.add(h) = w;
                *hlp.add(h8) = w;
            } else {
                if stag_live {
                    *tp.add(h) = g;
                }
                *hp.add(h) = v;
                if ltag_live {
                    *ltp.add(h8) = g;
                }
                *hlp.add(h8) = v;
            }
        }
        #[cfg(feature = "profile")]
        DF_FILL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        p += dfs;
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn find_dfast_impl_inner<const HLOG: u32, const PACKED: bool>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // Counted only under `profile`: one atomic per block is small, but it is
    // the same class the pair tail shipped until 959e0ae, and it has no
    // shipping consumer -- `take_dfast_calls` feeds gate harnesses only.
    #[cfg(feature = "profile")]
    if HLOG != 0 {
        DFAST_SPEC_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    // HLOG == 0 is the RUNTIME arm, served by THIS body rather than a separate
    // one. There used to be a hand-written `find_dfast_runtime` here; because it
    // was a second copy of the algorithm it silently DRIFTED -- Gate 6 added
    // `_search_next_long` to the specialised body only, so `dfast_spec` stopped
    // being a codegen A/B and became an A/B between two different algorithms
    // (15/18 corpora moved, versions-16m by 24.71%). Serving both from one body
    // makes byte-identity structural instead of a claim that has to be re-checked
    // every time the algorithm changes.
    let hlog = if HLOG == 0 { tables.hash_log } else { HLOG };
    #[cfg(feature = "profile")]
    if HLOG == 0 {
        DFAST_RUNTIME_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    // Read ONCE per block -- see the -37% an env lookup inside the DP loop cost
    // at L19, and the 60% the depth gate's four per-call reads cost at L19/L22.
    // GATE 14 @ L3 DISPATCH: raised while the offset trade is paying, held at 8
    // when it is not. See `nl_cut_for`.
    // Block-local band accumulators -- never atomics in the loop.
    let mut band_hits = 0u64;
    let mut band_worse = 0u64;
    let mls = params.min_match.max(3) as usize;
    // GATE 13 @ L3: reserve both outputs, as `find_fast` has since brick 38.
    // Sized from what the PREVIOUS block actually produced, so sparse-match
    // content does not over-reserve.
    // The hoist escape hatch is resolved HERE, once per block. It used to sit
    // inside `push_literals`' guard as an atomic load on EVERY call -- 1.97M
    // times at L3 and 15.7M at L1 -- selecting between two operands that are
    // IDENTICAL for `find_fast` (its `arm` is already `lit_push_enabled()`).
    // Brick 77 hoisted the env read out of that guard and left an atomic in its
    // place; this finishes the job.
    let block_len = block_end - block_start;
    // Scratch + the too-short-block exit, shared across the five HLOG copies
    // (the `fast_finder_prologue` treatment, dfast's variant: `lp` gates the
    // reservations here where fast's `reserve` does).
    // The knob reads (`good_ml`, `good_ml2`, the litpush arm) fold into the
    // prologue call too: four atomic loads and their branches were stamped
    // per HLOG copy for values decided once per block.
    // ALL seven per-block knob atomics arrive from the prologue now (see
    // `DfastGates`): each was an `Ordering::Relaxed` load plus its branch,
    // stamped into every HLOG copy to decide something that changes once per
    // block. None of them can move across the prologue -- it takes only the
    // two scratch Vecs and touches no signal any of these read.
    let (mut seqs, mut lits, g) =
        match dfast_finder_prologue(src, block_start, block_end, block_len, mls, tables) {
            Ok(t) => t,
            Err(out) => return out,
        };
    let DfastGates {
        lp,
        good_ml,
        good_ml2,
        nl_on,
        dstep,
        dpipe,
        lt_on,
    } = g;
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut hits = 0u64;
    // GATE 2 @ L3 -- shut IMMEDIATELY on a dry block and re-probe on a schedule,
    // instead of decaying 0.5 per block. The decay was written when the DFast
    // threshold was 0.0 and could never fire; with the gate live at 0.005 it
    // costs an 8-block warm-up in which every position is probed for nothing.
    //
    // Decay 0.0 alone would save that (12.5% of the remaining probe work) but is
    // the one-way LATCH from Gate 6: with the search off, `rep_hits` stays 0, so
    // `rep_yield` stays 0 and the gate can never reopen. The re-probe is what
    // makes an immediate shut safe.
    let use_rep = rep_search_on(tables.rep_yield, params.strategy) || tables.rep_probe == 0;
    let mut rep1 = reps[0] as usize;
    let mut rep_hits = 0u64;
    // W5: hoisted for the back-extension loop -- see its use.
    let fstart_c = tables.frame_start;
    let lowest_rep = block_start.saturating_sub(window).max(fstart_c);
    // W1/W2/W3: three PER-BLOCK values the loop recomputed per CANDIDATE.
    // `mlx` is a min/max over `mls` (four sites); `frame_start` was a struct
    // load through `&mut MatchTables`, which LLVM must re-prove after every
    // table write in between (six sites); and `lowest` is literally
    // `lowest_rep`'s expression, recomputed at two more.
    let frame_start_c = tables.frame_start;
    let mlx_c = 8.min(mls).max(4);
    // Block-hoisted, like every other arm here: an atomic load per MATCH is
    // exactly the cost this finder cannot afford (see section 7 of m7-anatomy).
    let bext_c = dfast_bext_enabled();
    // W19: `lowest_c` was an unread alias of `lowest_rep`.
    // W4: the literal-copy width, re-selected from a per-block flag on every
    // emitted match.
    let lp_w = if lp { LIT_PUSH_WIDTH } else { 0 };
    // GATE 6 @ L3 DISPATCH: run C's next-long probe only while it is EARNING.
    // `accel_shift_for(DFast)` is the constant 8 unless the RZSTD_ACCEL bench
    // pin is set; the pin stays available under `profile` (same treatment as
    // `find_fast_impl`'s loop, and `find_dfast` is dispatched for
    // Strategy::DFast only).
    let accel = if cfg!(feature = "profile") {
        accel_shift_for(params.strategy)
    } else {
        8
    };
    #[cfg(feature = "profile")]
    let mut mm_total = 0u64;
    let mut nl_probes = 0u64;
    let mut nl_hits = 0u64;
    // GATE 2 @ L3: repcode match bytes, for the same length-ratio signal that
    // dispatches Gate 2 at L1.
    let mut d_rep_bytes = 0u64;
    let mut ip = block_start;
    // Speculated (short hash, long hash, short slot, long slot) for the NEXT
    // position, produced by the previous iteration -- see GATE 8 below.
    // GATE 8 @ L3 DISPATCH -- decided DETERMINISTICALLY, on a work count.
    //
    // The pipeline changes issue ORDER, not work, so it is byte-identical and
    // cannot be priced by probe counts -- and at L3 the timing instrument's own
    // NULL ARM reads +-3.71% worst / +0.49% mean, which is larger than the whole
    // effect. Every per-corpus stopwatch verdict here was noise, and two runs
    // disagreed on the SIGN for versions (+6.89% then -8.48%) and ooffice
    // (-2.42% then +3.75%).
    //
    // The speculation ledger prices it exactly instead. A speculated load that
    // is CONSUMED replaces one the next iteration would have issued anyway --
    // pure latency overlap at zero added work. A speculated load that is
    // DISCARDED (the position ended in a match or a rep hit, so `ip` jumped past
    // it) is added work, full stop. So `spec_made - spec_used` is an exact count
    // of wasted loads, and the yield is the deterministic dispatch variable.
    //
    // Measured yields split the corpora nearly two to one:
    //   incomp 100.0%  text 98.2%  sao 91.9%  mozilla 87.7%  ooffice 83.7%
    //   ... against nci 32.3%  reymont 23.6%  dickens 38.6%  webster 41.4%
    // GATE 9 @ L3 -- the gate is DEAD here (step0 in {1,2,4} moves 0/18 sizes at
    // L3 against 16/18 at L1): DFast's advance was the literal `1`, so the
    // density knob had no caller. Read ONCE per block, never per position --
    // see the -37% that an env lookup inside the DP loop cost at L19.
    // GATE 9 @ L3 DISPATCH. Step 2 halves the hash work; measured across all 18
    // it is -9.56% time for +1.58% size, but the size cost is entirely content
    // dependent -- x-ray +12.67% and ooffice +6.09% against jsonlog -0.93% and
    // osdb -2.04%, which get SMALLER and faster. A sign flip, so it is routed.
    //
    // Mean match length is the axis, and it follows from the mechanism rather
    // than from fitting: skipping odd positions shifts a LONG match by one byte
    // (negligible), loses a SHORT match entirely, and loses nothing at all where
    // there are no matches.
    //   ml == 0   zeros, incomp-32m        step2 size 0.00%
    //   ml <  8   x-ray 5.05, sao 6.28     +12.67%, +3.26%
    //   ml >= 14  osdb .. text-32m         -2.04% .. +1.36%
    // W8: `spec_used` used to be incremented on EVERY position that consumed
    // a speculation -- a memory read-modify-write per position, visible as
    // `incq <slot>` in the emitted loop, feeding only the ratio
    // `spec_used / spec_made`. Every speculation issued is either CONSUMED,
    // DROPPED (the position ended in a match or a rep hit, which clears the
    // carry) or still live at block end, so counting the far rarer drops
    // gives the same ratio exactly:
    //     spec_used = spec_made - spec_dropped - (carried.live at exit)
    let (mut spec_made, mut spec_dropped) = (0u64, 0u64);
    // T1: the speculation now carries the short tag beside the short index.
    // W6: the carried speculation was
    // `Option<(usize, u8, usize, Option<usize>, Option<usize>)>` -- an outer
    // discriminant, two 8-byte hash indices and TWO nested `Option<usize>`
    // (16 bytes each), about 56 bytes that the loop head wrote to the stack
    // on every position (six spills per iteration in the emitted spec copy).
    //
    // Hash indices are `< 1 << hlog <= 2^24` and the candidates are carried in
    // the TABLE's OWN encoding -- `pos + 1`, with 0 meaning "no candidate",
    // exactly what the slots hold -- so nothing is lost, including position 0,
    // which the decoded `Option` form could not have expressed either. The
    // outer `Option` becomes the `live` flag. 56 bytes -> 20.
    #[derive(Clone, Copy)]
    struct Carried {
        h4: u32,
        h8: u32,
        v4: u32,
        v8: u32,
        g4: u8,
        live: bool,
    }
    let mut carried = Carried {
        h4: 0,
        h8: 0,
        v4: 0,
        v8: 0,
        g4: 0,
        live: false,
    };
    // Decode a carried slot value back to the `Option<usize>` the match logic
    // expects: identical to `get_h_tag`/`get_hl_tag`'s own tail.
    #[inline(always)]
    fn dec(v: u32) -> Option<usize> {
        if v == 0 {
            None
        } else {
            Some((v as usize) - 1)
        }
    }
    #[inline(always)]
    fn enc(m: Option<usize>) -> u32 {
        match m {
            Some(p) => (p as u32) + 1,
            None => 0,
        }
    }
    // Read ONCE per block. `hash4_tag`'s index is `(v * HASH4_PRIME) >> shift`,
    // which is exactly what `hash4` computes, so the tagged path indexes the
    // same slots as `hash_mls(src, ip, 4, hlog)` did.
    // BRICK 21: `PACKED` is this body's representation (see `find_dfast`).
    debug_assert_eq!(tables.pack_tags, PACKED);
    let dtag_on = PACKED || !tables.tags.is_empty();
    let dtag_shift = 32u32.saturating_sub(hlog.min(32));
    // 1a: the long-table tag filter. Packed frames only (the representation
    // needs the 24-bit position proof); the arm gates the compare.
    //
    // REFUTED (2026-08-21): an mls-width tag ("1a-strong" -- index and tag
    // from ONE u64 load, tag masked to the mlx bytes acceptance verifies).
    // Built in full and measured on the consume-site ledger with clean
    // counters: unfiltered 31,874,138 wasted loads/board; the shipped 4-byte
    // tag leaves 126,529 (0.40%); the mls-width tag leaves 124,718 (0.39%).
    // BOTH sit at the 8-bit collision floor (1/256 = 0.39%), because the
    // unfiltered waste is almost entirely FIRST-FOUR-BYTES-DIFFER bucket
    // collisions of the 8-byte hash -- the byte-5-differs class the wider
    // tag targets barely exists. It bought 1,811 loads/board for one AND and
    // one MUL per position in the hot loop.
    //
    // EPILOGUE (same day): the refutation stands for DEDICATED long-side
    // arithmetic -- but the SHORT table's consume census then found ITS
    // byte-5 class is 8.45M loads/board, the short tag went mls-width
    // (`hash4_tag_mls`), and since the long tag reuses the short tag it
    // became mls-width for free, landing the long boards on the exact 0.39%
    // floor anyway. Refuted work, delivered as a side effect at zero
    // marginal cost.
    //
    // Instrument lesson that found this (the "32M" false lead): the residual
    // statics ran during the arm-OFF pass too -- read counters out between
    // arms or the baseline contaminates the treatment 4:1.
    // W1: `pack_tags` is a per-FRAME constant that every tag accessor was
    // re-reading from the struct -- the asm showed offset 523 loaded and
    // tested ELEVEN times per position in one dfast twin. `find_fast_impl`
    // has hoisted it since ffanat (`let pack = tables.pack_tags`); the whole
    // dfast path, both fill helpers and the priming pass never did.
    let packed = PACKED;
    let stag_live = !tables.tags.is_empty();
    let ltag_live = !tables.ltags.is_empty();
    // The mls-width short tag's byte mask (min(mls, 8) bytes).
    let sk = 8.min(mls);
    let smask = if sk == 8 {
        u64::MAX
    } else {
        (1u64 << (8 * sk)) - 1
    };
    // Loop-invariant arm reads, hoisted from the MATCH path to once per block.
    let fill_anchor_c = dfast_fill_anchor_c();
    let fill_stride = dfast_fill_stride();
    let fill_ends = dfast_fill_ends();
    // Six `&mut MatchTables` field reads per match, hoisted to one each per
    // block. See `fill_dfast_after_match`.
    let fill_packed = PACKED;
    let fill_stag_live = !tables.tags.is_empty();
    let fill_ltag_live = !tables.ltags.is_empty();
    // W40: the LONG hash shift, once per block. See `dfast_hash_pair`.
    let dlong_shift = 64u32.saturating_sub(hlog.min(32));
    // REFUTED (2026-08-20): the Fast loop's mem::take table surgery, applied
    // here -- take hash/hash_long/tags into locals, slice-based slot twins,
    // slice-signature fill helpers. Byte-identical (dfid L1-L4 exact) but
    // strictly MORE work on every deterministic axis: family 8,656 -> 8,862
    // instrs, rbp-relative operands 2,001 -> 2,305, ALL memory operands
    // 3,309 -> 3,559. The mechanism is not Fast's: LLVM already keeps ONE
    // register on `tables` and folds the field offsets into addressing modes,
    // so the struct costs no per-access reload here, while THREE taken Vec
    // triples plus their restores add three competing base pointers to a loop
    // whose live set (spec tuple, two tables, rep, nl, band counters) is
    // already past sixteen GPRs. Fast won because it took TWO vecs into a
    // smaller live set. Do not redo without first shrinking the live set.
    while ip <= ilimit {
        #[cfg(feature = "profile")]
        if COUNT {
            mm_total += 1;
        }
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end, ilimit) {
                rep_hits += 1;
                // W5: profile-only -- its one reader is a `#[cfg(profile)]`
                // publish, and the shipping build already says
                // `let _ = d_rep_bytes`. A u64 add per rep HIT for nothing.
                if COUNT {
                    d_rep_bytes += ml as u64;
                }
                let mstart = ip + 1;
                push_literals(&mut lits, src, anchor, mstart, lp_w);
                seqs.push(Seq {
                    litlen: (mstart - anchor) as u32,
                    matchlen: ml as u32,
                    offset: rep1 as u32,
                });
                ip = mstart + ml;
                anchor = ip;
                // W8: see `spec_dropped`.
                spec_dropped += u64::from(carried.live);
                carried.live = false;
                continue;
            }
        }
        // GATE 8 @ L3 -- 2-WAY SOFTWARE PIPELINE FOR DFast.
        //
        // Gate 8 was DEAD at L3: `pipe_enabled()` is consumed only by
        // `find_fast`, and L3 runs `find_dfast` (measured: 0 find_fast calls,
        // 1027 find_dfast calls). The gate had no caller because the capability
        // did not exist here. This builds it.
        //
        // DFast is a BETTER pipelining candidate than `find_fast`: it issues TWO
        // independent table loads per position (short `hash` + long `hash_long`)
        // and consumes neither until after the match logic, so both miss
        // latencies serialise behind that logic instead of overlapping with it.
        //
        // The speculation is carried inside ONE loop body rather than duplicated
        // into a second pipelined loop. A second body is exactly how
        // `find_dfast_runtime` drifted until Gate 6 silently broke Gate 4's
        // byte-identity: an issue-order change must not be able to become an
        // algorithm change.
        let (h4, g4, h8, m4, m8) = if carried.live {
            carried.live = false;
            (
                carried.h4 as usize,
                carried.g4,
                carried.h8 as usize,
                dec(carried.v4),
                dec(carried.v8),
            )
        } else {
            {
                let (a, ga, b) = dfast_hash_pair(src, ip, dtag_shift, smask, dlong_shift);
                let m = tables.get_h_tag(a, ga, dtag_on, packed);
                // T1 ledger: a rejection is a candidate load AVOIDED. Counted
                // only under `profile`, so the shipping loop is untouched.
                // SECTION 15 INSTRUMENT FIX. `TAG_FALSE_REJECT` is documented
                // as "tag rejections that `fast_probe` would have ACCEPTED",
                // but it only ever tested `m.is_none()` -- i.e. it counted
                // EVERY rejection of a non-empty slot and called it false.
                // Read that way it says 59% of short-table rejections are
                // lost matches; verified against the BYTES, as the long-table
                // audit ten lines below always did, it says something else
                // entirely. A counter whose name asserts more than its code
                // checks is worse than no counter.
                #[cfg(feature = "profile")]
                if COUNT && dtag_on && tables.raw_fast(a) != 0 {
                    use core::sync::atomic::Ordering::Relaxed;
                    TAG_REJECT_TOTAL.fetch_add(1, Relaxed);
                    if m.is_none() {
                        let mr = (tables.raw_fast(a) as usize) - 1;
                        if match_ok(src, mr, ip, window, block_start, mlx_c, frame_start_c)
                            && count_match(src, mr, ip, block_end) >= mls
                        {
                            TAG_FALSE_REJECT.fetch_add(1, Relaxed);
                        }
                    }
                }
                let ml8 = tables.get_hl_tag(b, ga, lt_on, packed);
                // 1a ledger, THREE counters so the long table never inherits
                // the short counters' split personality (see the tag audit):
                // nonempty / rejected / FALSE (provably lost -- must be 0).
                #[cfg(feature = "profile")]
                if COUNT && lt_on {
                    use core::sync::atomic::Ordering::Relaxed;
                    let raw = tables.raw_hl(b);
                    if raw != 0 {
                        LTAG_NONEMPTY.fetch_add(1, Relaxed);
                        if ml8.is_none() {
                            LTAG_REJECT.fetch_add(1, Relaxed);
                            let mr = (raw as usize) - 1;
                            if match_ok(src, mr, ip, window, block_start, mlx_c, frame_start_c)
                                && count_match(src, mr, ip, block_end) >= mls
                            {
                                LTAG_FALSE.fetch_add(1, Relaxed);
                            }
                        }
                    }
                }
                (a, ga, b, m, ml8)
            }
        };
        tables.put_h_tag(h4, ip, g4, packed, stag_live);
        tables.put_hl_tag(h8, ip, g4, packed, ltag_live);
        // Issue the NEXT position's two loads NOW, so they are in flight while
        // this position's match logic runs. The miss-advance does not depend on
        // the match result, so `nip` is knowable here; a match or a rep hit
        // simply discards the speculation.
        //
        // BYTE-IDENTICAL: both stores above have already happened, exactly as
        // they had before the next iteration's loads in the original order, and
        // an aliasing slot is forwarded by hand -- `put_h` writes `ip+1`, so
        // `get_h` on that slot would return `Some(ip)`. The two tables are
        // distinct, so `h4` can only alias `h4` and `h8` only `h8`.
        if dpipe {
            let nip = ip + dstep + ((ip - anchor) >> accel);
            if nip <= ilimit {
                let (a, ga, b) = dfast_hash_pair(src, nip, dtag_shift, smask, dlong_shift);
                // The hand-forward has to respect the filter: `put_h_tag` just
                // wrote `g4` at slot `h4`, so a speculation landing on that slot
                // sees `ip` only when its own tag matches what is now stored.
                let va = if a == h4 {
                    if !dtag_on || ga == g4 {
                        Some(ip)
                    } else {
                        None
                    }
                } else {
                    tables.get_h_tag(a, ga, dtag_on, packed)
                };
                // The long hand-forward mirrors `get_hl_tag`: the store
                // above wrote tag `g4` at `h8`, so a speculation landing on
                // that slot sees `ip` only when its own short tag matches.
                let vb = if b == h8 {
                    if !lt_on || ga == g4 {
                        Some(ip)
                    } else {
                        None
                    }
                } else {
                    tables.get_hl_tag(b, ga, lt_on, packed)
                };
                spec_made += 1;
                carried = Carried {
                    h4: a as u32,
                    h8: b as u32,
                    v4: enc(va),
                    v8: enc(vb),
                    g4: ga,
                    live: true,
                };
            }
        }

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(m8) = m8 {
            if COUNT {
                probes += 1;
            }
            let mlx = mlx_c;
            if match_ok(src, m8, ip, window, block_start, mlx, frame_start_c) {
                // Count past match_ok's verified prefix (fast_probe_wide rule).
                let ml = mlx + count_match_fast(src, m8 + mlx, ip + mlx, block_end);
                if ml >= mls {
                    best_m = m8;
                    best_ml = ml;
                }
            }
            // 1a residual census: a survivor that fails acceptance is waste
            // the 4-byte tag could not see. SPLIT by failure class, because
            // the costs differ completely: a window/bounds fail is pure ALU
            // (match_ok tests them FIRST, no memory touched), while a bytes
            // fail paid the random src[m] load the filter exists to prevent.
            // Only the bytes class is a stronger tag's budget. The guard
            // below mirrors match_ok's cheap tests -- COUNT-only, drift risk
            // accepted for an instrument.
            #[cfg(feature = "profile")]
            if COUNT {
                use core::sync::atomic::Ordering::Relaxed;
                if best_ml == 0 {
                    let mlx = mlx_c;
                    let lowest = lowest_rep;
                    let cheap = m8 >= ip
                        || ip - m8 > window
                        || m8 < lowest
                        || ip + mlx > src.len()
                        || m8 + mlx > src.len();
                    if cheap {
                        LTAG_SURV_WFAIL.fetch_add(1, Relaxed);
                    } else {
                        LTAG_SURV_FAIL.fetch_add(1, Relaxed);
                    }
                } else {
                    LTAG_SURV_ACC.fetch_add(1, Relaxed);
                }
            }
        }
        // GATE 6 EXTENDED TO DFast -- C's `_search_next_long`.
        //
        // Gate 6's pair search at `ip+1` lives in `find_fast` only, so it is
        // dead at L3 where `find_dfast` runs. But C's doubleFast has an ip+1
        // probe we lack: when the LONG hash misses and only the short one hit,
        // it checks `hashLong` at `ip+1` BEFORE settling for the short match and
        // prefers that long match if it lands.
        //
        // Without it `find_dfast` commits to a 4-byte-hash match whenever the
        // 8-byte hash misses at exactly `ip`, even when a long match starts one
        // byte later -- the same "capability present in one finder, absent in
        // its neighbour" shape as the repcode and back-extension defects.
        let mut best_ip = ip;
        if best_ml < good_ml && nl_on && ip < ilimit {
            nl_probes += 1;
            // W42: resolved shift, like W40 one branch away.
            let h8b = hash8_shift(src, ip + 1, dlong_shift);
            // The only long consumer without a free tag: `ip + 1` never
            // computed a short hash. One mul+xor on a path already gated by
            // `best_ml < good_ml && nl_on`.
            let g8b = if lt_on {
                hash4_tag_mls(src, ip + 1, dtag_shift, smask).1
            } else {
                0
            };
            if let Some(m8b) = tables.get_hl_tag(h8b, g8b, lt_on, packed) {
                if COUNT {
                    probes += 1;
                }
                let mlx = mlx_c;
                if match_ok(src, m8b, ip + 1, window, block_start, mlx, frame_start_c) {
                    // Count past match_ok's verified prefix (fast_probe_wide rule).
                    let ml = mlx + count_match_fast(src, m8b + mlx, ip + 1 + mlx, block_end);
                    if ml >= mls && ml > best_ml {
                        // GATE 14 signal, measured only in the band the raise
                        // opens. Two adds on a path that fires a few thousand
                        // times per block -- not per position.
                        if best_ml >= 8 {
                            band_hits += 1;
                            if ip + 1 - m8b > ip - best_m {
                                band_worse += 1;
                            }
                        }
                        // GATE 14 study: the probe COMMITS at `ip + 1`, spending
                        // a literal. What it buys is `ml - best_ml` bytes, so
                        // that gain -- not the raw hit rate -- is what the cut
                        // actually stresses.
                        #[cfg(feature = "profile")]
                        {
                            use core::sync::atomic::Ordering::Relaxed;
                            NL_GAIN_G.fetch_add((ml - best_ml) as u64, Relaxed);
                            // The RAISED BAND only: hits that a cut above 8
                            // newly enables. Measuring the gain over ALL hits
                            // mixes in the baseline band and washes the signal
                            // out -- which is why the first attempt read flat.
                            if best_ml >= 8 {
                                NL_BAND_HITS.fetch_add(1, Relaxed);
                                NL_BAND_GAIN.fetch_add((ml - best_ml) as u64, Relaxed);
                                NL_BAND_OLD.fetch_add(best_ml as u64, Relaxed);
                                // The probe does not only lengthen the match --
                                // it takes a DIFFERENT one, at a different
                                // OFFSET. Offset bits are what the gain has to
                                // pay for, so record both offsets.
                                let off_new = (ip + 1 - m8b) as u64;
                                let off_old = (ip - best_m) as u64;
                                NL_OFF_NEW.fetch_add(off_new, Relaxed);
                                NL_OFF_OLD.fetch_add(off_old, Relaxed);
                                if off_new > off_old {
                                    NL_OFF_WORSE.fetch_add(1, Relaxed);
                                }
                            }
                        }
                        best_m = m8b;
                        best_ml = ml;
                        best_ip = ip + 1;
                        nl_hits += 1;
                    }
                }
            }
        }
        if best_ml < good_ml2 && best_ip == ip {
            if let Some(m4) = m4 {
                if COUNT {
                    probes += 1;
                }
                let mut _acc = false;
                if match_ok(src, m4, ip, window, block_start, mls, frame_start_c) {
                    // Count past match_ok's verified prefix (fast_probe_wide rule).
                    let ml = mls + count_match_fast(src, m4 + mls, ip + mls, block_end);
                    _acc = ml >= mls;
                    if ml >= mls && ml > best_ml {
                        best_m = m4;
                        best_ml = ml;
                    }
                }
                // SHORT-table consume-site census, the mirror of the long
                // table's (which found 60.8M invisible wasted loads across
                // two boards). Survivors here share only FOUR guaranteed
                // bytes against an mls of 5+, so the byte-5 class that
                // barely existed for the long table is structurally real
                // here. Classes: window/bounds fail (ALU only), bytes fail
                // (paid the random src[m] load), produced a valid match.
                #[cfg(feature = "profile")]
                if COUNT {
                    use core::sync::atomic::Ordering::Relaxed;
                    if _acc {
                        STAG_SURV_ACC.fetch_add(1, Relaxed);
                    } else {
                        let lowest = lowest_rep;
                        let cheap = m4 >= ip
                            || ip - m4 > window
                            || m4 < lowest
                            || ip + mls > src.len()
                            || m4 + mls > src.len();
                        if cheap {
                            STAG_SURV_WFAIL.fetch_add(1, Relaxed);
                        } else {
                            STAG_SURV_FAIL.fetch_add(1, Relaxed);
                        }
                    }
                }
            }
        }
        if best_ml >= mls {
            // BACK-EXTENSION PROBE (profile only, no behaviour change). Mirrors
            // `emit_fast_seq_body`'s walk exactly -- same guards, same
            // `back_eq` -- so the number it reports is the number applying it
            // would recover. See `take_dfast_bext`.
            #[cfg(feature = "profile")]
            {
                use core::sync::atomic::Ordering::Relaxed;
                let (mut bi, mut bm, mut got) = (best_ip, best_m, 0u64);
                while bi > anchor && bm > frame_start_c && back_eq(src, bi, bm) {
                    bi -= 1;
                    bm -= 1;
                    got += 1;
                }
                DFAST_BEXT_SEQS.fetch_add(1, Relaxed);
                if got > 0 {
                    DFAST_BEXT_BYTES.fetch_add(got, Relaxed);
                    DFAST_BEXT_MATCHES.fetch_add(1, Relaxed);
                }
            }
            // BACK-EXTENSION. Mirrors `emit_fast_seq_body`'s walk: same guards,
            // same `back_eq`. `best_ip` and `best_m` fall together so the OFFSET
            // is unchanged, and `best_ip + best_ml` is unchanged -- only the
            // literal/match split moves, which is the whole point.
            //
            // `found_ip` is the PRE-extension position and the fills below use
            // it, exactly as L1's emitter passes `found_ip` to
            // `fill_fast_after_match` rather than the walked-back `ip`. Filling
            // from the extended position would change which slots the table
            // holds, which is a different change with a different verdict.
            let found_ip = best_ip;
            if bext_c {
                while best_ip > anchor && best_m > frame_start_c && back_eq(src, best_ip, best_m) {
                    best_ip -= 1;
                    best_m -= 1;
                    best_ml += 1;
                }
            }
            // commit at `best_ip`, which is `ip+1` when the next-long probe won
            push_literals(&mut lits, src, anchor, best_ip, lp_w);
            seqs.push(Seq {
                litlen: (best_ip - anchor) as u32,
                matchlen: best_ml as u32,
                offset: (best_ip - best_m) as u32,
            });
            rep1 = best_ip - best_m;
            if COUNT {
                hits += 1;
            }
            let end = best_ip + best_ml;
            // DFast never sets `packed` (it is gated on Strategy::Fast).
            // GATE 12 @ L3: `ip` here is the PRE-probe position; when the
            // next-long probe won, `best_ip == ip + 1` and the two tables index
            // different positions for one match. See `dfast_fill_anchor_c`.
            let long_anchor = if fill_anchor_c { found_ip } else { ip };
            // FUSED: one walk over both tables. See `fill_dfast_after_match`.
            fill_dfast_after_match(
                tables,
                src,
                found_ip,
                long_anchor,
                end,
                fill_ends,
                smask,
                dtag_shift,
                dlong_shift,
                ilimit,
                fill_packed,
                fill_stag_live,
                fill_ltag_live,
            );
            // GATE 12 @ L3: the density knob DFast never had. Off by default.
            let dfs = fill_stride;
            if dfs != 0 {
                // BRICK 7: OUTLINED. This stride fill is OFF by default (`dfast_fill_stride`
                // resolves to 0), so the loop never ran in production -- yet it sat inline
                // in the L3 per-position loop holding four table bases, two shifts, the
                // mask and three flags LIVE across the hottest code in the crate. The
                // census read that loop at 865 instructions with 201 stack reloads per
                // position, the two table bases alone reloaded 12x and 10x. Moving the
                // loop into its own frame is pressure relief for the caller; the loop's
                // own cost is unchanged and only ever paid when the knob is on.
                dfast_stride_fill(
                    tables,
                    src,
                    best_ip + 2 + dfs,
                    end.saturating_sub(2).min(ilimit + 1),
                    dfs,
                    dtag_shift,
                    dlong_shift,
                    smask,
                    (packed, stag_live, ltag_live),
                );
            }
            ip = end;
            anchor = ip;
            // The two fills rewrite many entries, so anything speculated before
            // them is stale.
            spec_dropped += u64::from(carried.live);
            carried.live = false;
        } else {
            ip += dstep + ((ip - anchor) >> accel);
        }
    }
    // THE SHIPPING EPILOGUE IS ONE NON-GENERIC CALL, stamped once instead of
    // once per HLOG copy (five in this symbol) -- the `fast_finder_epilogue`
    // treatment. The `#[cfg(profile)]` census flushes stay here: they are
    // cfg'd out of shipping builds and reference profile-only locals.
    let spec_used = spec_made
        .saturating_sub(spec_dropped)
        .saturating_sub(u64::from(carried.live));
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        MM_TOTAL.fetch_add(mm_total, Relaxed);
        DFAST_SPEC_MADE.fetch_add(spec_made, Relaxed);
        DFAST_SPEC_USED.fetch_add(spec_used, Relaxed);
    }
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        let mb: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        DFAST_MATCH_BYTES.fetch_add(mb, Relaxed);
        DFAST_SEQS.fetch_add(seqs.len() as u64, Relaxed);
        DFAST_BLOCK_BYTES.fetch_add((block_end - block_start) as u64, Relaxed);
        DFAST_REP_BYTES.fetch_add(d_rep_bytes, Relaxed);
        DFAST_REP_HITS.fetch_add(rep_hits, Relaxed);
        DFAST_BLOCKS.fetch_add(1, Relaxed);
        if use_rep {
            DFAST_REP_BLOCKS.fetch_add(1, Relaxed);
            DFAST_REP_POS.fetch_add((block_end - block_start) as u64, Relaxed);
        }
    }
    #[cfg(not(feature = "profile"))]
    let _ = d_rep_bytes;
    // EWMA so one atypical block cannot flip the route -- the Gate 6 lesson.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        NL_PROBES_G.fetch_add(nl_probes, Relaxed);
        NL_HITS_G.fetch_add(nl_hits, Relaxed);
    }
    // GATE 14 @ L3: feed this block's measured offset trade to the next block.
    // Attribute ONLY when the band actually fired -- a block that measured
    // nothing must not move the EWMA, which is what would latch the gate.
    dfast_finder_epilogue(
        tables, src, &seqs, &mut lits, anchor, block_end, rep_hits, spec_used, spec_made, dpipe,
        nl_probes, nl_hits, band_worse, band_hits, probes, hits,
    );
    (seqs, lits)
}

/// GATE 9: DFast probe density. C's `_doubleFast` probes every position; 2 halves
/// the hash work at some ratio cost. Swept via `RZSTD_DFAST_STEP`.
/// Mean match length at or above which DFast may probe every OTHER position.
pub(crate) fn dfast_ml_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[5].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        // Was a raw env::var per block; cached like `dfast_spec_min`.
        use core::sync::atomic::Ordering;
        let c = DFAST_ML_MIN_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = crate::env_knob_parse("RZSTD_DFAST_ML").unwrap_or(14.0);
        DFAST_ML_MIN_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    14.0
}
#[cfg(feature = "std")]
pub(crate) static DFAST_ML_MIN_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Non-zero forces a fixed density (measurement arm); 0 = dispatch.
pub(crate) fn dfast_step_forced() -> usize {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        // OFFSET SENTINEL. This stored the value RAW and treated 0 as "not
        // cached" -- but 0 is also the value an UNSET knob resolves to, which is
        // the shipping default. So the cache never took, and every call re-read
        // the environment: a `std::env::var` allocation and an OS lookup once
        // per block, forever, to answer a question fixed for the life of the
        // process. Storing `v + 1` makes 0 mean "unread" and nothing else.
        let c = DFAST_STEP_ARM.load(Ordering::Relaxed);
        if c != 0 {
            return (c - 1) as usize;
        }
        let v: usize = crate::env_knob_parse("RZSTD_DFAST_STEP")
            .filter(|v| *v >= 1)
            .unwrap_or(0);
        DFAST_STEP_ARM.store(v as u32 + 1, Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static DFAST_MATCH_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_SEQS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_BLOCK_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

pub static DFAST_BLOCKS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_REP_BLOCKS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Positions over which `try_rep1` is live -- the work a rep dispatch removes.
pub static DFAST_REP_POS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(blocks, rep_blocks, rep_positions)`
pub fn take_dfast_rep_blocks() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DFAST_BLOCKS.swap(0, Relaxed),
        DFAST_REP_BLOCKS.swap(0, Relaxed),
        DFAST_REP_POS.swap(0, Relaxed),
    )
}

pub static DFAST_REP_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_REP_HITS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(match_bytes, seqs, block_bytes, rep_bytes, rep_hits)` for DFast.
pub fn take_dfast_match_stats() -> (u64, u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DFAST_MATCH_BYTES.swap(0, Relaxed),
        DFAST_SEQS.swap(0, Relaxed),
        DFAST_BLOCK_BYTES.swap(0, Relaxed),
        DFAST_REP_BYTES.swap(0, Relaxed),
        DFAST_REP_HITS.swap(0, Relaxed),
    )
}

pub(crate) static DFAST_STEP_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Set the DFast probe density in-process.
pub fn set_dfast_step_arm(v: usize) {
    // `+ 1` to match the reader's offset sentinel: 0 means "never read",
    // so a stored value must be biased the same way or it reads back one low.
    DFAST_STEP_ARM.store(v as u32 + 1, core::sync::atomic::Ordering::Relaxed);
}

/// Blocks between forced DFast-pipeline re-probes.
pub(crate) const DFAST_PROBE_PERIOD: u32 = 16;

/// Minimum share of speculated loads that must be CONSUMED for the DFast
/// pipeline to run. Below it the speculation is net added work.
/// MEASURED INERT (`dispatchaudit.rs`, every level INCLUDING L3/L4, its own
/// strategy). Unlike the two above this bar IS reached -- `dpipe` evaluates it
/// per block -- but `dfast_spec_yield` never falls to the 0.70 bar on the
/// board, so the guard is one-sided and the branch always goes the same way.
/// Same shape as `walk_rep_max`, whose signal also never approaches its bar,
/// and the opposite of `walk_first_max`, whose signal sits right on top of
/// its own. A bar is only a dispatch if its signal crosses it.
pub(crate) fn dfast_spec_min() -> f32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = DFAST_SPEC_MIN_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = crate::env_knob_parse("RZSTD_DFAST_SPECMIN").unwrap_or(0.70);
        DFAST_SPEC_MIN_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.70
}

pub(crate) static DFAST_SPEC_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the Gate 8 speculation-yield threshold in-process.
pub fn set_dfast_spec_min_arm(v: f32) {
    DFAST_SPEC_MIN_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

pub(crate) static DFAST_PIPE_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// A/B the DFast 2-way software pipeline in-process -- both shapes, one binary,
/// so the comparison is immune to cross-binary drift.
pub static DFAST_SPEC_MADE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_SPEC_USED: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(speculations_made, speculations_consumed)`.
pub fn take_dfast_spec() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DFAST_SPEC_MADE.swap(0, Relaxed),
        DFAST_SPEC_USED.swap(0, Relaxed),
    )
}

pub fn set_dfast_pipe_arm(on: bool) {
    DFAST_PIPE_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn dfast_pipe_enabled() -> bool {
    DFAST_PIPE_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

pub(crate) fn note_finder_work(count: bool, probes: u64, hits: u64, seqs: &[Seq], lits: &[u8]) {
    let match_bytes: u64 = if count {
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
}
