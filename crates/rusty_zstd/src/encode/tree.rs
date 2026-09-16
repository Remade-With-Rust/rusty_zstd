//! The TREE ladder: `find_bt_lazy` (L13-L15) and `find_opt` (L16-L22),
//! with the knobs and census counters that only they read.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a moved
//! item that was private is now `pub(crate)` so the parent can still name
//! it. That changes who may reference a symbol, not what is emitted for it,
//! and the asm board was identical in all thirty-two columns across the
//! move -- which is the check that matters, because this crate builds at
//! the default `codegen-units = 16` where module layout CAN move inlining.
//!
//! `use super::*` is how a child reaches the parent's private items; the
//! parent reaches back through the `pub use` beside its `mod tree;`.

use super::*;
// Outlined for the same reason as `find_opt` above (same sibling-parity gap,
// same nil ISA trade -- one `%cl` shift, zero `shrx`).
#[inline(never)]
pub(crate) fn find_bt_lazy(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BRICK 99 (K18): the contract's bound (brick 35) -- `wide_hash` below is
    // then provably false and the tree kernel's 8-byte-hash arm is gone.
    let mls = params.min_match.clamp(3, 7) as usize;
    // GATE 6 family, fourth instance: take the finder buffers from the FRAME.
    //
    // `find_fast_impl` was wired to `MatchTables::seq_scratch`/`lit_scratch`
    // and `find_opt` to its own scratch, but Greedy/Lazy/BtLazy still built
    // both from bare `Vec::new()` -- no reserve at all, growing by doubling
    // with LIVE contents, so every growth is a real memcpy. Measured on the
    // 18-corpus 8 MiB board: **172 MB** through `realloc` at L5, 164 MB at L9,
    // 154 MB at L13, against 9.2 MB at L3 and 1.3 MB at L19.
    //
    // `encode_block` already hands these back at all four of its exits, so the
    // plumbing was in place and only these finders were missing from it.
    // W6: `finder_scratch_enabled()` is an arm read, and it was read TWICE --
    // once per buffer -- for one per-block answer.
    // Scratch + the too-short-block exit, ONE copy for Greedy/Lazy/BtLazy and
    // their bmi2 twins -- six stamps of the identical idiom become one call
    // (the `fast_finder_prologue` treatment, chain-finder variant).
    let (mut seqs, mut lits) = match chain_finder_prologue(src, block_start, block_end, tables, mls)
    {
        Ok(t) => t,
        Err(out) => return out,
    };
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    // W8: GATE 6 for BtLazy2 -- every other finder takes its output buffers
    // from the frame WITH A RESERVE; this one grew them by repeated `realloc`
    // with LIVE contents, so every growth is a real memcpy.
    // (reserve moved into `chain_finder_prologue`)
    // W9: GATE 13 for BtLazy2. `push_lits_range` appends through a
    // runtime-length `extend_from_slice`; `push_literals` takes the
    // fixed-width `copy_nonoverlapping` path when the run fits and the spare
    // capacity proves the wide store is in bounds -- which W8's reserve now
    // guarantees. The capability has been in find_fast since brick 38 and in
    // find_dfast since 4.46; the Bt ladder never got it. Byte-identical: only
    // `n` bytes are ever published.
    // GATE 13's literal-width guard, FOLDED -- it has never fired on this path.
    // `lit_short_share` has exactly TWO writes in the crate,
    // `fast_pipe_epilogue` and `fast_finder_epilogue`, both on the L1 Fast
    // ladder. Greedy, Lazy and BtLazy2 never write it, so it holds its initial
    // 1.0 and `>= LIT_SHORT_MIN` (0.25) is permanently true here.
    // `dispatchaudit.rs` shows the same thing from outside: sweeping the
    // `lit_short` bar from 0.0 to 1.0 moves neither the compressed bytes nor
    // the probe count at ANY level, and bytegate holds across 18 corpora x 9
    // levels with the guard folded.
    //
    // Folded rather than left alone because as written it is a TRAP: if this
    // path ever starts maintaining that field, the branch begins firing and
    // moves the bitstream with no edit here to explain why.
    let lp_copy = lit_width_for(tables);
    // BRICK 73: repcode-1 in BtLazy2 (L13-L14) -- the last finder without it.
    // Per-call arm reads hoisted to once per block (the chain_find_best rule):
    // attempts (an arm atomic inside bt_depth_apply/search_attempts) ran per
    // position, per look-ahead AND per fill insert; the fill arms ran per
    // match.
    let attempts = bt_depth_apply(search_attempts(params), params, tables.opt_rep_rate);
    let clog = params.chain_log.min(24);
    let btf = bt_resolve::<true>(tables.hash_log, clog);
    let btf_ins = bt_resolve_ins(tables.hash_log, clog);
    // W7: `block_start.saturating_sub(window).max(frame_start)` was built
    // TWICE per block -- once for the tree's lower bound, once for the
    // repcode's. One value now, and the two cannot drift apart.
    let fstart_c = tables.frame_start;
    let lowest_rep = block_start.saturating_sub(window).max(fstart_c);
    // BRICK 34: the block's tree geometry, once.
    let (bt_mask, bt_ok) = bt_geom(clog, tables.chain.len());
    let bt_ctx = BtCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts,
        chain_log: clog,
        bt_lowest: lowest_rep,
        chain_len: tables.chain.len(),
        wide_hash: mls >= 8,
        bt_mask,
        bt_shift32: 32u32.saturating_sub(tables.hash_log.min(32)),
        bt_shift64: 64u32.saturating_sub(tables.hash_log.min(32)),
        bt_ok,
    };
    let gain_cmp = lazy_gain_enabled_bt();
    let fill_on = lazy_fill_enabled();
    let bt_stride = bt_fill_stride();
    let use_rep = rep_search_on(tables.rep_yield, params.strategy);
    let mut rep1 = reps[0] as usize;
    let mut rep_hits = 0u64;
    let mut ip = block_start;
    // Hoisted per BLOCK: an atomic load per position would cost more
    // than the positions it skips.
    let accel_sh = lazy_step_shift(lazy_accel());
    while ip <= ilimit {
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end, ilimit) {
                rep_hits += 1;
                let mstart = ip + 1;
                push_literals(&mut lits, src, anchor, mstart, lp_copy);
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
        let (mut best_m, mut best_ml) = btf(&bt_ctx, ip, tables);
        let mut best_ip = ip;
        let mut look_hi = ip;
        // W3: `bt_find_best` returns either `(0, 0)` or a length that already
        // cleared its own `>= mls` bar, so `best_ml >= mls` IS `best_ml != 0`
        // -- a test against zero instead of against a value that has to stay
        // live across the whole look-ahead.
        debug_assert!(best_ml == 0 || best_ml >= mls);
        if best_ml != 0 {
            // W5: the in-hand gain is only meaningful once there IS a match to
            // describe, and only the look-ahead reads it -- computing it per
            // POSITION spent a multiply and a `leading_zeros` on every miss,
            // which is most positions.
            let mut best_gain = if gain_cmp {
                lazy_gain(best_ml, ip - best_m)
            } else {
                0
            };
            for d in 1..=depth {
                let ip2 = ip + d;
                if ip2 > ilimit {
                    break;
                }
                look_hi = ip2;
                let (m, ml) = btf(&bt_ctx, ip2, tables);
                // C's offset-priced look-ahead (`set_lazy_gain_arm`), wired
                // here for its own board: refuted at L7-L12, untested at
                // L13-L15 where BtLazy2's economics differ.
                // W1: `lazy_gain(best_ml, best_ip - best_m)` describes the
                // match ALREADY IN HAND, so it changes only when that match
                // does -- it was rebuilt on every look-ahead step (a multiply,
                // a `leading_zeros` and two subs).
                //
                // W2: and the gain this test computes for the CANDIDATE is
                // exactly the new best's gain when the test passes; it was
                // thrown away and recomputed.
                //
                // W4: `ml >= mls` is `ml != 0` -- see W3 above.
                let cand_gain = if gain_cmp { lazy_gain(ml, ip2 - m) } else { 0 };
                let take = if gain_cmp {
                    ml != 0 && cand_gain > best_gain + 4
                } else {
                    ml > best_ml
                };
                if take {
                    best_ml = ml;
                    best_m = m;
                    best_ip = ip2;
                    best_gain = cand_gain;
                }
            }
        }
        // W3: same identity -- `best_ml` is 0 or already past `mls`.
        if best_ml != 0 {
            // DEFECT B3 FIX (btlazy2): back-extend -- see `find_greedy`.
            let mut s = best_ip;
            let mut mm = best_m;
            let mut n = best_ml;
            #[cfg(feature = "profile")]
            let bext_from = s;
            // W5: `frame_start` is a per-FRAME constant, and this is the
            // back-extension loop -- the struct load ran on every extended
            // BYTE, through `&mut MatchTables`, so LLVM had to re-prove it
            // after each table write the match path performs.
            while s > anchor && mm > fstart_c && back_eq(src, s, mm) {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            #[cfg(feature = "profile")]
            note_bext((bext_from - s) as u64);
            push_literals(&mut lits, src, anchor, s, lp_copy);
            seqs.push(Seq {
                litlen: (s - anchor) as u32,
                matchlen: n as u32,
                offset: (s - mm) as u32,
            });
            // Commits at the look-ahead winner (brick 71b).
            rep1 = best_ip - best_m;
            // DEFECT B1 FIX (btlazy2): same missing back-fill as find_lazy.
            // `bt_find_best` inserts `ip` into the tree as a side effect, so
            // walking the covered span re-uses it rather than duplicating the
            // insertion logic.
            let end = best_ip + best_ml;
            if fill_on {
                // GATE 11/12 @ L13-L15: this loop inserts EVERY position a match
                // covers, and it is 61.9% of all binary-tree work at these levels
                // (28,776,361 calls with it, 10,977,025 without). `find_lazy`'s
                // equivalent has had a stride knob all along; this one never did.
                //
                // It EARNS its place -- removing it entirely costs +2.41% size
                // (reymont +8.48%, webster +7.83%, nci +7.15%) -- so the question
                // is not whether to fill but how densely.
                let stride = bt_stride;
                // B2: the look-ahead already inserted up to `look_hi`.
                // W10: `end` and `ilimit` are both fixed for this fill, so the
                // two bounds it tested on EVERY inserted position fold to one
                // stop value -- and this loop is 61.9% of all tree work here.
                let stop = end.min(ilimit + 1);
                let mut p = (best_ip + 1).max(look_hi + 1);
                while p < stop {
                    btf_ins(&bt_ctx, p, tables);
                    p += stride;
                }
            }
            ip = end;
            anchor = ip;
        } else {
            // C: `ip += ((ip-anchor) >> kSearchStrength) + 1`.
            ip += lazy_step(ip, anchor, accel_sh);
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    push_lits_range(&mut lits, src, anchor, block_end);
    // Probes reported by `bt_find_best`.
    note_finder_work(
        cfg!(feature = "profile"),
        0,
        seqs.len() as u64,
        &seqs,
        &lits,
    );
    (seqs, lits)
}

/// The DP's LITERAL price in bits. Flat 6 since the parser was written, against
/// a real cost of ~8 bits raw and ~4-7 after Huffman -- so it UNDER-prices
/// literals on high-entropy content, which makes the "optimal" parse prefer
/// literals over matches and lose to plain lazy. Swept via `RZSTD_OPT_LIT`.
/// The MEASURED cost of the literals just emitted, for the next block's DP.
#[inline]
pub(crate) fn measured_lit_bits(section_bytes: usize, literal_count: usize) -> u32 {
    // WHOLE-SECTION cost per literal, deliberately -- see 4.20: the marginal
    // variant is theoretically righter and measured worse, because the DP's
    // MATCH price is itself an approximation and pricing only the literal side
    // exactly unbalances the pair.
    let bits = (section_bytes as u64 * 8) / literal_count.max(1) as u64;
    // Clamp to the range the price model is meaningful over.
    bits.clamp(3, 10) as u32
}

pub(crate) fn opt_lit_cost(tables: &MatchTables) -> u32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        // The env override is resolved ONCE. The first version fell through to
        // `std::env::var` on every call whenever no override was set -- a string
        // allocation and environment scan PER DP POSITION, which measured -37%
        // throughput at L19 across all twelve corpora. Same defect class as the
        // per-probe atomics in `fast_probe`.
        const UNCHECKED: u32 = u32::MAX;
        const NO_OVERRIDE: u32 = u32::MAX - 1;
        let mut e = OPT_LIT_ARM.load(Ordering::Relaxed);
        if e == UNCHECKED {
            e = crate::env_knob_parse("RZSTD_OPT_LIT").unwrap_or(NO_OVERRIDE);
            OPT_LIT_ARM.store(e, Ordering::Relaxed);
        }
        if e != NO_OVERRIDE {
            return e;
        }
        // ONE-SIDED: only ever RAISE the price above the historical constant, so
        // blocks whose literals are cheap keep exactly today's parse.
        match tables.opt_lit_price {
            0 => 6,
            m => m.max(6),
        }
    }
    #[cfg(not(feature = "std"))]
    6
}

pub(crate) static OPT_LIT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// The DP's match-length extra-bits pricing (Gate 19's other half).
pub(crate) static OPT_MLBITS_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook for ML-bits pricing.
pub fn set_opt_mlbits_arm(on: bool) {
    OPT_MLBITS_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn opt_mlbits_enabled() -> bool {
    // DEFAULT ON -- adjudicated: L16 -0.007% / L19 -0.014% / L22 -0.014%
    // totals, best nci -0.302%, worst jsonlog +0.097%. Small and real.
    !matches!(
        OPT_MLBITS_ARM.load(core::sync::atomic::Ordering::Relaxed),
        1
    )
}

/// Set the DP literal price in-process.
pub fn set_opt_lit_arm(v: u32) {
    OPT_LIT_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Blocks between forced re-probes of the opt repcode candidate.
pub(crate) const OPT_REP_PERIOD: u32 = 16;

/// Blocks the candidate must RUN before the gate may shut it.
pub(crate) const OPT_REP_WARMUP: u32 = 4;

/// Minimum bytes-per-probe for the opt DP's repcode candidate to run. A NEGATIVE
/// value is the escape hatch: constant ON, i.e. the pre-dispatch behaviour, which
/// is what the ledger's "fallback proven" column requires.
///
/// The term is NOT decoration -- disabling it entirely (schedule only) removes
/// 91.0% of the probes instead of 85.8%, but costs +0.1179% size against
/// +0.0195%. Those 6M extra probes buy back 0.098 percentage points.
pub(crate) fn opt_rep_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[6].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_REP_MIN_C.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = crate::env_knob_parse("RZSTD_OPT_REP_MIN").unwrap_or(50.0);
        OPT_REP_MIN_C.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    50.0
}
#[cfg(feature = "std")]
pub(crate) static OPT_REP_MIN_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Measurement arm for the opt DP's repcode candidate.
pub(crate) static OPT_REP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// A/B the opt DP's repcode candidate in-process.
pub fn set_opt_rep_arm(on: bool) {
    OPT_REP_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn opt_rep_enabled() -> bool {
    !matches!(OPT_REP_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// GATE 10 @ L19: what the DP's repcode candidate earns. `try_rep1` runs at
/// every position of every opt block, unconditionally.
pub static OPT_REP_PROBES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_REP_HITS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_REP_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

pub static OPT_POS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_SKIP_INF: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_SKIP_JUMP: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_SKIP_JUMPS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(positions, skipped_price_inf, bytes_jumped, jumps)`
pub fn take_opt_skips() -> (u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        OPT_POS.swap(0, Relaxed),
        OPT_SKIP_INF.swap(0, Relaxed),
        OPT_SKIP_JUMP.swap(0, Relaxed),
        OPT_SKIP_JUMPS.swap(0, Relaxed),
    )
}

pub static OPT_BT_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_BT_DRY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_BT_LEN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static OPT_SEQS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(bt_calls, bt_calls_returning_nothing, total_match_len, emitted_seqs)`
pub fn take_opt_bt() -> (u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        OPT_BT_CALLS.swap(0, Relaxed),
        OPT_BT_DRY.swap(0, Relaxed),
        OPT_BT_LEN.swap(0, Relaxed),
        OPT_SEQS.swap(0, Relaxed),
    )
}

/// `(probes, hits, hit_bytes)` for the opt DP's repcode candidate.
pub fn take_opt_rep() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        OPT_REP_PROBES.swap(0, Relaxed),
        OPT_REP_HITS.swap(0, Relaxed),
        OPT_REP_BYTES.swap(0, Relaxed),
    )
}

/// GATE 11 @ L19: back-fill the span the `sufficient_len` jump skips.
///
/// SHIPPED ON, dispatched on the frame's PEAK bytes-per-rep-probe. Ungated it
/// cost +27.2% of bt probes for -342 bytes with versions-16m regressing +54;
/// dispatched it costs +1.11% for -361 bytes with NO corpus regressing -- 115
/// bytes per million probes against 4.4, a 26x better exchange rate.
pub(crate) fn opt_fill_enabled() -> bool {
    #[cfg(feature = "profile")]
    ENVHIT[7].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_FILL_C.load(Ordering::Relaxed);
        if c != 0 {
            return c == 2;
        }
        let v = crate::env_knob_not0("RZSTD_OPT_FILL", true);
        OPT_FILL_C.store(if v { 2 } else { 1 }, Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    false
}
#[cfg(feature = "std")]
pub(crate) static OPT_FILL_C: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Above this bytes-per-rep-probe the content is rep-dominated and the jumped
/// span's interior is not worth inserting.
pub(crate) fn opt_fill_rep_max() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[8].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_FILL_REP_C.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = crate::env_knob_parse("RZSTD_OPT_FILL_REP").unwrap_or(50.0);
        OPT_FILL_REP_C.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    50.0
}
#[cfg(feature = "std")]
pub(crate) static OPT_FILL_REP_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Longest span the back-fill will walk. Beyond this the jump is a single huge
/// repeat and its interior is not worth inserting.
pub(crate) fn opt_fill_max() -> usize {
    #[cfg(feature = "profile")]
    ENVHIT[9].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let a = OPT_FILL_MAX_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if a != 0 {
        return a;
    }
    #[cfg(feature = "std")]
    {
        crate::env_knob_parse("RZSTD_OPT_FILL_MAX").unwrap_or(usize::MAX)
    }
    #[cfg(not(feature = "std"))]
    usize::MAX
}

/// Stride for that back-fill; 1 inserts every skipped position.
/// GATE 12 @ L19 arms: the opt back-fill's stride and span cap, as atomics so
/// they can be swept in one process. 0 = unset (use the env/default path).
pub(crate) static OPT_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
pub(crate) static OPT_FILL_MAX_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook: opt back-fill stride (0 restores the default of 1).
pub fn set_opt_fill_stride_arm(v: usize) {
    OPT_FILL_S_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Bench hook: opt back-fill span cap (0 restores the uncapped default).
pub fn set_opt_fill_max_arm(v: usize) {
    OPT_FILL_MAX_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Positions inserted by the opt back-fill -- the work GATE 12 controls at L19.
pub static OPT_FILL_INS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the opt back-fill insert count.
pub fn take_opt_fill_ins() -> u64 {
    OPT_FILL_INS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// GATE 12 @ L19 defect arm: `false` restores the per-jump `std::env::var`
/// lookups the back-fill guard used to perform inside the DP loop, so the fix
/// can be A/B'd in one process instead of across two binaries.
/// Bench hook, now a NO-OP (BRICK 17): the per-jump re-read arm it selected
/// was retired from `find_opt`'s DP loop. Kept so the arm tables link.
pub fn set_opt_hoist_arm(_hoisted: bool) {}

pub(crate) fn opt_fill_stride() -> usize {
    #[cfg(feature = "profile")]
    ENVHIT[10].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let a = OPT_FILL_S_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if a != 0 {
        return a;
    }
    #[cfg(feature = "std")]
    {
        crate::env_knob_parse("RZSTD_OPT_FILL_S")
            .filter(|v| *v >= 1)
            .unwrap_or(1)
    }
    #[cfg(not(feature = "std"))]
    1
}

// OUTLINED, and it is a SIBLING-PARITY fix: `find_dfast`, `find_greedy` and
// `find_lazy` all carry their own symbol (plus a bmi2 twin); `find_opt` and
// `find_bt_lazy` alone stayed `#[inline(always)]` and were therefore stamped
// into BOTH `find_sequences_strategy` twins -- so every Fast/DFast block paid
// the optimal parse's stack frame and spills just to reach the dispatcher.
// The two twins fell 2,707+2,644 -> 311+254.
//
// The ISA trade is nil, and was checked rather than assumed: the emitted body
// contains ONE `%cl` shift and zero `shrx`, so the bmi2 context it used to
// inherit had nothing to fold -- while the parse's actual ISA-sensitive work,
// `bt_find_best`, still arrives through `bt_resolve`'s per-block function
// pointers, whose bmi2 twins are unchanged and still linked.
#[inline(never)]
pub(crate) fn find_opt(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // Sixth sighting of the un-gated per-block atomic class (959e0ae).
    #[cfg(feature = "profile")]
    OPT_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let n = block_end - block_start;
    // BRICK 99 (K18): the contract's bound (brick 35) -- `wide_hash` below is
    // then provably false and the tree kernel's 8-byte-hash arm is gone.
    let mls = params.min_match.clamp(3, 7) as usize;
    if n < 8 {
        return (Vec::new(), src[block_start..block_end].to_vec());
    }
    let inf = u32::MAX / 4;
    // T2: take the DP arrays from the frame instead of building 2.63 MiB of
    // them per block. See `MatchTables::opt_price`.
    let mut price = core::mem::take(&mut tables.opt_price);
    let mut prev = core::mem::take(&mut tables.opt_prev);
    // off and ml live in ONE u64 (off | ml << 32): one store per edge
    // improvement and one load per parse step instead of two of each, and
    // one scratch array fewer.
    let mut match_om = core::mem::take(&mut tables.opt_om);
    reset_to(&mut price, n + 1, inf);
    // The other four arrays are NEVER read before written: every position j
    // in 1..=n is reachable through the literal chain (price[0] = 0 and the
    // literal edge runs first at every i), and the FIRST improvement at j --
    // from price[j] == inf -- writes prev/is_match (and match_off/match_ml
    // together under is_match). The backtrace only visits priced positions.
    // Their per-block resets were ~17 bytes of memset PER INPUT BYTE doing
    // nothing; only the LENGTH must be ensured (stale contents are dead).
    // `prev` shrank from usize (8 B/position of DP write+backtrace traffic)
    // to u32 -- positions are < 2^24 -- and `is_match` PACKED into its spare
    // bit 31, deleting that whole array (alloc, sizing, one store per edge
    // improvement, one load per parse step).
    const OPT_MATCH_BIT: u32 = 1 << 31;
    ensure_len(&mut prev, n + 1, 0u32);
    ensure_len(&mut match_om, n + 1, 0u64);
    debug_assert!(!price.is_empty());
    #[allow(unsafe_code)]
    unsafe {
        *price.get_unchecked_mut(0) = 0;
    }
    // BRICK 75: offer the REPCODE as a DP candidate (find_opt was the last
    // finder without repcode search).
    //
    // Correctness is the emit path's job: we record a candidate at byte
    // DISTANCE `rep1`, and `offset_value_for` turns that into a repcode code
    // using the real rep state at emit time. What the DP must get right is
    // WHERE the match starts and what it costs.
    let rep1 = reps[0] as usize;
    // W5: hoisted for the back-extension loop -- see its use.
    let fstart_c = tables.frame_start;
    let lowest_rep = block_start.saturating_sub(window).max(fstart_c);
    // W6: the DP's own `i + 8 > n` continue gives `ip + 8 <= block_end`, which
    // is exactly the finders' `ip <= ilimit`. The bound is per BLOCK; it was
    // rebuilt from `block_end` on every position that probed the repcode.
    let rep_ilimit = block_end.saturating_sub(8);
    // Hoisted once per block (the chain_find_best rule): bt_find_best runs
    // per DP position here.
    let bt_attempts = bt_depth_apply(search_attempts(params), params, tables.opt_rep_rate);
    let clog = params.chain_log.min(24);
    let btf = bt_resolve::<true>(tables.hash_log, clog);
    let btf_ins = bt_resolve_ins(tables.hash_log, clog);
    // BRICK 34: the block's tree geometry, once.
    let (bt_mask, bt_ok) = bt_geom(clog, tables.chain.len());
    let bt_ctx = BtCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts: bt_attempts,
        chain_log: clog,
        bt_lowest: block_start.saturating_sub(window).max(tables.frame_start),
        chain_len: tables.chain.len(),
        wide_hash: mls >= 8,
        bt_mask,
        bt_shift32: 32u32.saturating_sub(tables.hash_log.min(32)),
        bt_shift64: 64u32.saturating_sub(tables.hash_log.min(32)),
        bt_ok,
    };
    let extra = match params.strategy {
        Strategy::BtUltra2 => 2u32,
        Strategy::BtUltra => 1,
        _ => 0,
    };
    let rep_cost = 12u32.saturating_sub(extra).saturating_add(2);
    // C's `sufficient_len` (`ZSTD_compressBlock_opt_generic`): a match longer
    // than `targetLength` is taken IMMEDIATELY -- "large match -> immediate
    // encoding" -- and the DP is skipped for the span it covers. `find_opt`
    // never read `target_length` at all, so on content whose matches always
    // exceed it we ran a full per-byte optimal parse where C commits and jumps.
    // That is the structural half of the L16+ pathology (the other half was the
    // length enumeration below).
    //
    // ABSOLUTE FLOOR on top of `target_length`. C's opt parse runs its DP inside
    // a bounded WINDOW and jumps to the end of the committed path; ours prices
    // the whole block per byte, so a bare `target_length` skip fires on ordinary
    // matches and forces the parse through them. At L16 `target_length` is 48,
    // and skipping on 48-byte matches cost osdb 3,141,787 -> 3,156,514 bytes and
    // broke level monotonicity (L16 > L13).
    //
    // The pathology is driven by matches in the tens of THOUSANDS of bytes, so
    // the floor keeps the whole speed win while leaving ordinary matches to the
    // DP. `higher_level_never_larger_osdb` is the gate that caught this.
    // NOTE (gg-matchfind Gate 9 @ L22): this floor DOMINATES every `target_length`
    // the level table produces for the opt strategies -- L16 48, L18 64, L19 256,
    // L21 512, L22 999 all collapse to 1024, so `target_length` is inert across
    // the whole optimal ladder.
    //
    // That sounds like a defect and MEASURED as a non-event: sweeping the floor
    // 1024 -> 512 -> 256 -> 64 moves nothing on 11 of 14 corpora and makes nci,
    // samba and xml slightly WORSE. `bml` simply does not reach these lengths
    // often enough for the skip to fire. The knob was built, measured inert, and
    // REMOVED rather than left as dead configuration surface.
    const OPT_SKIP_FLOOR: usize = 1024;
    let sufficient_len = if params.target_length == 0 {
        usize::MAX
    } else {
        (params.target_length as usize).max(OPT_SKIP_FLOOR)
    };
    // Block-constant: read ONCE, never inside the DP loop.
    let lit_cost = opt_lit_cost(tables);
    let (mut o_rep_probes, mut o_rep_hits, mut o_rep_bytes) = (0u64, 0u64, 0u64);
    // GATE 10 @ L22 curiosity: what does the DP's per-position bt search return?
    let (mut o_bt_calls, mut o_bt_dry, mut o_bt_len) = (0u64, 0u64, 0u64);
    // GATE 11 @ L19: are there positions the DP never inserts? Two paths skip
    // without calling bt_find_best.
    // Read only by the `profile` census at the end of this function; without
    // that feature the three adds below are not compiled at all.
    #[cfg(feature = "profile")]
    let (mut o_skip_inf, mut o_skip_jump, mut o_skip_jumps) = (0u64, 0u64, 0u64);
    #[cfg(feature = "profile")]
    let o_positions = n as u64;
    // GATE 10 @ L19 DISPATCH. The candidate costs a `try_rep1` at every position
    // and EARNS almost nowhere: 12 of 18 corpora are SMALLER without it, and
    // only versions-16m (+51.654% if removed) and text-32m (+5.376%) need it.
    // Bytes-per-probe separates them absolutely -- 434 and 26,932 against a
    // maximum of 35.6 for everything else.
    //
    // Re-probed on a schedule rather than decayed: with the candidate off no
    // hits are recorded, so any decay converges to zero and latches the gate
    // shut permanently. That is the Gate 6 defect, and the Gate 2 @ L3 one.
    let rep_min = opt_rep_min();
    // `rep1 == 0` makes every try_rep1 return None; testing it per position
    // inside the helper was a block-constant branch in the DP loop.
    let opt_rep_on = rep1 != 0
        && opt_rep_enabled()
        && (rep_min < 0.0 // sentinel: constant ON, the pre-dispatch behaviour
            || tables.opt_rep_seen < OPT_REP_WARMUP
            || tables.opt_rep_probe == 0
            || tables.opt_rep_rate >= rep_min);
    let mut i = 0usize;
    // DEFECT (GATE 12 @ L19). These four were read INSIDE the DP loop, so every
    // jumped position performed `std::env::var` -- a `GetEnvironmentVariableW`
    // plus a `String` allocation, up to four per jump, across 3.85M jumped
    // positions. The file already carried the warning that produced this rule
    // ("the -37% that an env lookup inside the DP loop cost at L19"); GATE 11's
    // back-fill reintroduced it. Read ONCE per block.
    let fill_on = opt_fill_enabled();
    let fill_rep_max = opt_fill_rep_max();
    let fill_step = opt_fill_stride();
    let fill_span_max = opt_fill_max();
    // W9: the jump-fill gate, resolved once per block for the shipped arm.
    let fill_gate_hoisted =
        fill_on && tables.opt_rep_meas >= 2 && tables.opt_rep_peak < fill_rep_max;
    let mlb_on = opt_mlbits_enabled();
    // W8: the DP's length loop compared `params.strategy == BtUltra2` on every
    // LENGTH STEP of every priced match -- a per-BLOCK constant read from a
    // by-value struct field inside the innermost loop in the encoder.
    let ultra2 = params.strategy == Strategy::BtUltra2;
    // W11: `mlb_on && len > 34` is a flag test AND a bound test on every
    // length step. The flag is per-BLOCK, so fold it into the bound: with the
    // arm off, no length can exceed the sentinel and the whole ML-bits term
    // (an lzcnt, a sub and two cmovs) never enters the loop's dependency
    // chain. One compare replaces compare + compare + cmov.
    let mlb_over = if mlb_on { 34usize } else { usize::MAX };
    // (The per-jump re-read arm and its selector were retired in BRICK 17.)
    while i < n {
        // T2/T4 SAFETY, for the literal edge below -- the ONLY part of this loop
        // that runs at EVERY position.
        //
        // `price`, `prev`, `is_match`, `match_off` and `match_ml` are all reset
        // to exactly `n + 1` entries at the top of `find_opt`, and the loop
        // condition is `i < n`, so `i` and `i + 1` are both `<= n` and therefore
        // in range. LLVM cannot carry that through the `saturating_add` and the
        // early-continue, so it bounds-checked a per-position access. Every
        // other index in this DP is already guarded by an explicit `if j <= n`.
        debug_assert!(i + 1 < price.len() && price.len() == n + 1);
        #[allow(unsafe_code)]
        let pi = *unsafe { price.get_unchecked(i) };
        if pi >= inf {
            #[cfg(feature = "profile")]
            {
                o_skip_inf += 1;
            }
            i += 1;
            continue;
        }
        // Range-proven plain add: pi < inf = MAX/4 and lit_cost is a small
        // constant, so saturation is unreachable -- the saturating form paid
        // a cmov per position for nothing.
        let np = pi + lit_cost;
        // W13: keep what the literal edge already read. `price[i + 1]` is
        // loaded here for the compare and was loaded AGAIN a few lines down
        // as the rep edge's base -- the same slot, with only this edge's own
        // store in between, so its post-state is known without a reload.
        #[allow(unsafe_code)]
        let p_next = unsafe {
            let q = price.as_mut_ptr().add(i + 1);
            let cur = *q;
            if np < cur {
                *q = np;
                *prev.get_unchecked_mut(i + 1) = i as u32;
                np
            } else {
                cur
            }
        };
        if i + 8 > n {
            i += 1;
            continue;
        }
        let ip = block_start + i;
        // `try_rep1` matches at ip+1: a rep0 code requires litlen >= 1. So the DP
        // edge must ORIGINATE AT i+1 (after that literal), not at i. Basing it on
        // `price[i]` was the first attempt and it emitted every sequence with
        // litlen off by one -- an invalid stream that 36 conformance cases caught.
        // `price[i + 1]` is final here: the literal edge above already set it.
        // GATE 10 @ L19 -- the L3 question, transferred. `try_rep1` runs at EVERY
        // position here too, unconditionally. Count what it earns before gating
        // it: probes issued, hits, and the bytes those hits cover.
        // DP arrays are len n + 1 and every index below is guarded <= n;
        // the checked ops compiled to a bounds test + panic branch PER DP
        // EDGE (and per length step in the loop below).
        // `i + 1 <= n` was the loop condition restated, and
        // `price[i + 1] < inf` is ALWAYS true here: pi < inf (checked above)
        // and the literal edge just wrote price[i+1] <= pi + lit_cost < inf.
        // Both tests were dead.
        debug_assert!(price[i + 1] < inf);
        if opt_rep_on {
            o_rep_probes += 1;
            // The DP's own `i + 8 > n` continue above gives `ip + 8 <=
            // block_end`, which is exactly the finders' `ip <= ilimit`.
            if let Some(rml) = try_rep1(src, ip, rep1, lowest_rep, block_end, rep_ilimit) {
                o_rep_hits += 1;
                o_rep_bytes += rml as u64;
                let j = i + 1 + rml;
                if j <= n {
                    #[allow(unsafe_code)]
                    unsafe {
                        let np = p_next
                            + rep_cost
                            + if rml > mlb_over {
                                27 - ((rml - 3) as u32).leading_zeros()
                            } else {
                                0
                            };
                        if np < *price.get_unchecked(j) {
                            *price.get_unchecked_mut(j) = np;
                            *prev.get_unchecked_mut(j) = (i + 1) as u32 | OPT_MATCH_BIT;
                            *match_om.get_unchecked_mut(j) = rep1 as u64 | ((rml as u64) << 32);
                        }
                    }
                }
            }
        }
        let (bm, bml) = btf(&bt_ctx, ip, tables);
        o_bt_calls += 1;
        if bml < mls {
            o_bt_dry += 1;
            i += 1;
            continue;
        }
        o_bt_len += bml as u64;
        // BRICK 72: price a sequence by its OFFSET, not a flat constant.
        //
        // This was a flat `24 - extra` for EVERY match, so the dynamic program
        // could not distinguish a match 100 bytes back from one 2 MB back --
        // it optimised a cost function that does not describe the bitstream.
        // C prices offsets through `ZSTD_getMatchPrice` / the offset code,
        // which is ~log2(offset) bits.
        //
        // `of_code` is the RFC's offset code = floor(log2(offset_value)), and
        // the encoder then writes that many extra bits, so the true cost grows
        // with the offset's magnitude. A near match is genuinely cheaper.
        let off_bits = 32 - ((ip - bm) as u32 | 1).leading_zeros();
        // W7: `extra` is 0, 1 or 2 (BtUltra2 / BtUltra / else), and the left
        // side is at least 12 -- the saturating form's cmov guards an
        // underflow no strategy can produce, once per priced match.
        debug_assert!(extra <= 2);
        let seq_cost = (12u32 + off_bits) - extra;
        // GATE 19 DEFECT FIX -- the DP enumerated LENGTHS; C enumerates MATCHES.
        //
        // `np` below does not depend on `len`: this price model charges a match
        // `12 + off_bits - extra` whatever its length. So this loop writes the
        // SAME value into every `price[j]` for `j` in `i+mls ..= i+bml`. On
        // content whose matches run long, `bml` reaches the block size, and the
        // DP becomes O(n * bml).
        //
        // Measured on an 8 MiB `text-32m` prefix, matched levels:
        //     L13 BtLazy2   C   107 ms   us     334 ms      3x
        //     L16 BtOpt     C    71 ms   us 198,441 ms  2,795x
        //     L22 BtUltra2  C    79 ms   us 409,475 ms  5,183x
        // The cliff is exactly the BtLazy2 -> BtOpt boundary, i.e. entry to
        // `find_opt`. C never pays it because `ZSTD_BtGetAllMatches` hands its
        // DP a BOUNDED list of candidate matches rather than a length range.
        //
        // Cap the exploration at `OPT_MAX_LENGTHS` evenly spaced probes, always
        // including `bml` itself. This is a NO-OP wherever `bml - mls` is
        // already below the cap -- which is all normal content; only inputs
        // with very long matches take a different path.
        const OPT_MAX_LENGTHS: usize = 64;
        // W8: the `if bml < mls { continue }` above proves `bml >= mls`, so
        // this saturating sub is another dead cmov, once per priced match.
        debug_assert!(bml >= mls);
        let floor_step = ((bml - mls) / OPT_MAX_LENGTHS).max(1);
        // price[i] and seq_cost are PER-MATCH constants: the sum was
        // reloaded and re-added on every length step.
        #[allow(unsafe_code)]
        let np_base = unsafe { *price.get_unchecked(i) } + seq_cost;
        // ADJUDICATED (the Gate 19 note's other half): the bitstream charges
        // MATCH-LENGTH extra bits, but the DP priced every length of a match
        // identically -- so the "optimal" parse over-preferred long matches
        // whose tails cost real bits. RFC shape: lengths 3..=34 pay 0 extra
        // bits; beyond that the extra bits grow ~log2(len - 3) - 4.
        // W9: the `i + len > n` exit is a bound on `len`, and both terms are
        // loop-invariant -- so it is one `min` before the loop instead of an
        // add and a compare on every length step. The set of lengths priced is
        // identical: the old loop ran while `len <= bml` AND `i + len <= n`.
        let lmax = bml.min(n - i);
        // W12: the length loop reloaded the price base from the stack for the
        // compare and AGAIN for the store -- LLVM cannot prove a `Vec`'s data
        // pointer survives the writes next door. The three DP arrays are
        // frame-scratch and nothing resizes them inside the parse, so take
        // their bases once per match.
        #[allow(unsafe_code)]
        let (pp, pv, pm) = (price.as_mut_ptr(), prev.as_mut_ptr(), match_om.as_mut_ptr());
        let mut len = mls;
        if len <= lmax {
            loop {
                let j = i + len;
                let np = if len > mlb_over {
                    np_base + (27 - ((len - 3) as u32).leading_zeros())
                } else {
                    np_base
                };
                // SAFETY: `j = i + len <= i + lmax <= n` and every array is
                // `n + 1` long; the bases are the ones taken above.
                #[allow(unsafe_code)]
                unsafe {
                    let pj = pp.add(j);
                    if np < *pj {
                        *pj = np;
                        *pv.add(j) = i as u32 | OPT_MATCH_BIT;
                        *pm.add(j) = (ip - bm) as u64 | ((len as u64) << 32);
                    }
                }
                if len == lmax {
                    break;
                }
                // W10: BtUltra2's step is the constant 1, so `step.max(
                // floor_step)` is just `floor_step` (which is >= 1) -- the
                // clamp chain and the max collapse to an add for the levels
                // that run every length.
                len = if ultra2 {
                    (len + floor_step).min(lmax)
                } else {
                    (len + (bml - len).clamp(1, 4).max(floor_step)).min(lmax)
                };
            }
        }
        // C's immediate encoding: this match already exceeds `targetLength`, so
        // commit it and jump the DP past the span it covers instead of pricing
        // every interior position. Positions inside keep `price == inf`, so no
        // path can route through them -- exactly the greedy commitment C makes.
        if bml >= sufficient_len && i + bml <= n {
            #[cfg(feature = "profile")]
            {
                o_skip_jump += bml as u64;
                o_skip_jumps += 1;
            }
            // GATE 11 BROUGHT TO LIFE AT L19. The DP inserts a position by
            // searching it, so the `sufficient_len` jump leaves the whole span
            // OUT of the tree -- measured, 3,853,451 positions (11.4%) over 675
            // jumps. Those positions can never afterwards be the START of a
            // match, which is exactly the hole `find_bt_lazy`'s back-fill exists
            // to close. This is the same capability, at the level where it was
            // dead for want of a caller.
            // GATE 11 @ L19 DISPATCH. The span-length CAP was the wrong axis:
            // dickens' jumps average 5,335 positions and GAIN, versions' average
            // 2,812 and LOSE, so no cap separates them -- and a partial fill is
            // worse for versions than filling none or all (non-monotonic).
            //
            // `opt_rep_rate` does separate them, and it is the same signal Gate
            // 10 maintains and Gate 14's depth cut uses: versions-16m 434
            // bytes/probe and text-32m 26,932 against a maximum of 35.6 for
            // everything else. Those two hold 93% of ALL jumped positions
            // (3.58M of 3.85M) and contribute -15 and +54 bytes; the other five
            // hold 5.8% and contribute -381.
            //
            // Rep-dominated content does not need the interior of a huge repeat
            // in the tree -- it is reachable through the repeat itself.
            // The PEAK, not the last block. A single block in which the rep
            // candidate probed and never hit drives `opt_rep_rate` to 0 --
            // measured on versions-16m, whose jumps read the sentinel, then 0,
            // then 131,041 -- and that one block was enough to fill part of its
            // spans. A partial fill is worse for versions than filling none or
            // all, which is the whole +134 bytes.
            //
            // Two real measurements are also required: with one, versions has
            // only seen the 0.
            // The OFF arm re-reads the environment here, per jumped position,
            // exactly as the shipped code did before the hoist.
            // BRICK 17: the four knobs are block constants, read once above.
            // The OFF arm that re-read them per jumped position (an A/B hook
            // for GATE 12's hoist) is retired: with `hoisted_arm` a runtime
            // bool the DP loop carried BOTH arms -- nine rip-relative static
            // loads and four selector tests per jump, on the shipping path
            // that never took them. `set_opt_hoist_arm` is a no-op now.
            // W9: `opt_rep_meas` and `opt_rep_peak` are per-BLOCK signals, but
            // they were read from the struct on every JUMP -- and on
            // match-dense content the DP jumps constantly. Hoisted for the
            // shipped (hoisted) arm; the measurement arm keeps its deliberate
            // per-jump re-reads.
            if fill_gate_hoisted {
                let step = fill_step;
                // Cap the span. text-32m and versions-16m hold 93% of ALL jumped
                // positions (3.58M of 3.85M) and contribute -15 and +54 bytes;
                // dickens, samba, nci, ooffice and xml hold 6% and contribute
                // -381. An enormous jump means one huge repeat, and filling its
                // interior buys nothing -- those positions are reachable through
                // the repeat itself.
                let span = bml.min(fill_span_max);
                // W14: the fill walked POSITIONS but addressed BYTES, so each
                // inserted position paid `block_start + q` and `qp + 8 >
                // block_end` -- two adds and a compare for a walk whose stride
                // is constant. Both become induction: `qp` advances by the
                // stride and the bound is subtracted once. (`block_end >= 8`
                // wherever a match was priced, so the bound cannot wrap.)
                let qp_end = block_end - 8;
                let mut qp = block_start + i + 1;
                let mut q = i + 1;
                while q < i + span && qp <= qp_end {
                    btf_ins(&bt_ctx, qp, tables);
                    #[cfg(feature = "profile")]
                    OPT_FILL_INS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    q += step;
                    qp += step;
                }
            }
            i += bml;
            continue;
        }
        i += 1;
    }
    // price[n] < inf is PROVEN: every position is reachable through the
    // literal chain (price[0] = 0, the literal edge runs first at every
    // priced i), and a sufficient_len jump prices its own endpoint (the
    // length loop always includes len == bml). The old fallback to
    // find_bt_lazy was unreachable.
    debug_assert!(price[n] < inf);
    // GATE 6, one layer under the payload buffer: `ops` had the SAME defect,
    // and a much larger one.
    //
    // Bucketing every `realloc` by the size it grows TO shows a doubling ladder
    // repeated once per block -- `sao` at L19 takes 31 reallocs at EVERY rung
    // from 128 KiB to 2 MiB, one full climb per block, because this vector was
    // rebuilt from zero each time. A single buffer that doubles hits each rung
    // ONCE; 31 hits per rung is 31 buffers each climbing from scratch.
    //
    // The entry is a 32-byte tuple pushed once per PARSE STEP, and a literal
    // step advances one byte, so incompressible content pushes one per input
    // byte -- 4 MiB of tuples for a 128 KiB block. That is why L19 memcpy'd
    // 340 MB on a 2 MiB board where L3 moved 10 MB.
    //
    // Same remedy as the payload: the vector never escapes `find_opt`, so keep
    // it on the frame and let it converge on its own high-water mark.
    // (start, off, ml, matched): 16 bytes -- start fits u32 (positions
    // < 2^24), and the bool packs into the 4-aligned layout. Was 24.
    let mut ops: Vec<(u32, u32, u32)> = core::mem::take(&mut tables.opt_ops);
    ops.clear();
    // The reuse above leaves exactly ONE growth ladder per frame: the first
    // block still climbs from nothing to its high-water mark. It is removable,
    // and the two obvious constants both lose:
    //
    //   * reserve `n + 1` always -- exact upper bound (a literal step advances
    //     one byte, so the chain cannot be longer than the block), but it asks
    //     for 4 MiB per 128 KiB block even on content whose parse is 30 steps.
    //   * reserve nothing -- pays the ladder, which copies ~2x the final size.
    //
    // The chain length is COUNTABLE before it is pushed, though: walking `prev`
    // is a pointer chase with no allocation and no writes. So take the exact
    // size when the buffer could overflow, and skip the walk entirely when it
    // provably cannot -- `k <= n`, so a capacity of `n + 1` is proof.
    // THE COPY WAS COPYING DEAD BYTES.
    //
    // Two sizing arms looked like a dispatch -- exact-fit (pre-walk the chain)
    // versus the `n + 1` upper bound -- and they did split on content: blanket
    // beat exact on `mr`/`mozilla`/`nci`/`samba`/`xml`, tied on the other 13,
    // and cost up to +4.19 MB of address space for zero copy benefit on
    // `text` and `versions`. Escalating between them made it WORSE (6.0 MB of
    // copying became 21.5 MB), which is what exposed the real defect.
    //
    // `Vec::reserve` grows through `realloc`, and `realloc` preserves the old
    // ALLOCATION -- the allocator has no idea the Vec's `len` is 0. This buffer
    // is cleared at the top of every block, so every byte `realloc` carried was
    // already dead. Replacing the buffer instead of growing it copies nothing,
    // and it does so whatever sizing policy sits on top: the split between the
    // two arms was never about content, it was both of them paying for a memcpy
    // neither of them needed.
    //
    // Exact-fit is then strictly better than the upper bound -- same zero
    // copies, and it asks for what the parse actually uses.
    // `k <= n`, so a capacity of n + 1 is PROOF the buffer cannot grow --
    // and the frame-kept scratch converges there after the first blocks. The
    // pre-walk (an O(steps) pointer chase over `prev`) then sizes nothing:
    // it ran on EVERY block anyway. Skip it when capacity is the proof.
    // W5: the pre-walk is an O(steps) DEPENDENT pointer chase over `prev`,
    // run only to size `ops`. Its old proof was `k <= n`, which a converged
    // buffer rarely satisfied. After W1 the buffer holds MATCHES, and every
    // match advances at least `mls` positions, so `n / mls + 1` bounds it --
    // a proof a converged buffer meets immediately, and the chase is skipped
    // entirely from the second block on.
    let ops_bound = n / mls.max(1) + 1;
    if opt_ops_exact() && ops.capacity() < ops_bound {
        let mut k = 0usize;
        let mut j = n;
        // j <= n along the whole chain (prev entries are indices the DP
        // wrote, all <= n); the checked op was a bounds branch per parse
        // step.
        while j > 0 {
            debug_assert!(j < prev.len());
            #[allow(unsafe_code)]
            let pr = *unsafe { prev.get_unchecked(j) };
            // W1: count only what will be PUSHED -- the matched steps.
            k += usize::from(pr & OPT_MATCH_BIT != 0);
            j = (pr & !OPT_MATCH_BIT) as usize;
        }
        if ops.capacity() < k {
            ops = Vec::with_capacity(k);
        }
    } else if opt_ops_blanket() && ops.capacity() < n + 1 {
        // The blanket arm keeps its `n + 1` upper bound: it deliberately does
        // not pre-walk, so it cannot know the match count. It is still an
        // upper bound after W1 (matches <= steps).
        ops = Vec::with_capacity(n + 1);
    }
    let mut i = n;
    // opt_w's literal-run histogram is ALSO computed here (the pending-start
    // trick: walking backward, the run before match k is start_k minus the
    // end of the match seen NEXT in this walk), removing what was a separate
    // full pass over `ops`.
    let (mut w_short, mut w_mid) = (0usize, 0usize);
    let mut pending_start = usize::MAX;
    let count_run = |run: usize, w_short: &mut usize, w_mid: &mut usize| {
        if run <= LIT_PUSH_WIDTH {
            *w_short += 1;
        } else if run <= LIT_PUSH_WIDTH_WIDE {
            *w_mid += 1;
        }
    };
    while i > 0 {
        debug_assert!(i < prev.len());
        #[allow(unsafe_code)]
        let pr = unsafe { *prev.get_unchecked(i) };
        let p = (pr & !OPT_MATCH_BIT) as usize;
        let m = pr & OPT_MATCH_BIT != 0;
        if m {
            // W3: `match_om` is only read on MATCHED steps. It used to be
            // loaded -- and split into `off`/`ml` -- on every step of the
            // walk, and after W1 the walk is ~20 literal steps per match, so
            // that load and its two extracts were wasted 19 times out of 20.
            #[allow(unsafe_code)]
            let om = unsafe { *match_om.get_unchecked(i) };
            let (off, ml) = (om as u32, (om >> 32) as u32);
            if pending_start != usize::MAX {
                count_run(pending_start - (p + ml as usize), &mut w_short, &mut w_mid);
            }
            pending_start = p;
            // W1: ONLY matched steps are pushed. The emit loop below reads
            // `ops` and skips every entry whose flag is false, so a literal
            // step contributed a 16-byte tuple that nothing ever used -- and a
            // literal step advances ONE BYTE, so incompressible content pushed
            // one per input byte (the 4 MiB-per-128 KiB the comment above
            // describes). The literal runs are not lost: each match's own
            // `start` minus the running `anchor` is exactly the run before it,
            // which is how the emit loop already reconstructs them.
            ops.push((p as u32, off, ml));
        }
        i = p;
    }
    if pending_start != usize::MAX {
        count_run(pending_start, &mut w_short, &mut w_mid);
    }
    // `ops` is in REVERSE parse order; consumers iterate `.rev()` instead of
    // paying an O(steps) reversal pass.
    // GATE 13 @ L22 -- the capability find_fast has had since brick 38 and
    // find_dfast since 4.46, absent from the whole Bt ladder. `find_opt` grew
    // both vectors by repeated realloc and appended every literal run through a
    // runtime-length `extend_from_slice`.
    //
    // Unlike the other finders this one can be EXACT rather than estimated:
    // `ops` is already built, so the sequence count and the literal-run shares
    // are known before a single byte is appended -- no `last_nseq` guess, no
    // previous-block signal, and therefore no warm-up block.
    let block_len = block_end - block_start;
    // GATE 6/13 for find_opt, at last: every other finder takes its output
    // buffers from the frame; this one allocated BOTH fresh per block.
    // Capacity stays EXACT (ops is built, so the counts are known).
    let mut seqs = core::mem::take(&mut tables.seq_scratch);
    seqs.clear();
    // W4: `nmatched` was a counter incremented once per match beside the
    // push that already records exactly those steps -- `ops.len()` IS the
    // match count now that W1 stores nothing else.
    let nmatched = ops.len();
    if seqs.capacity() < nmatched + 1 {
        seqs = Vec::with_capacity(nmatched + 1);
    }
    let mut lits = core::mem::take(&mut tables.lit_scratch);
    lits.clear();
    if lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    // Width chosen from THIS block's own runs, by the asm-derived rule:
    // widen when mid_share > short_share * (fast32 - fast16) / (slow - fast32).
    let opt_w = {
        let n = nmatched.max(1) as f32;
        // ONE division, not two -- same denominator.
        let inv = 1.0 / n;
        let (sh, md) = (w_short as f32 * inv, w_mid as f32 * inv);
        if sh < lit_short_min() {
            0
        } else if md > sh * WIDEN_RATIO {
            LIT_PUSH_WIDTH_WIDE
        } else {
            LIT_PUSH_WIDTH
        }
    };
    let mut anchor = 0usize;
    for &(start, off, ml) in ops.iter().rev() {
        let start = start as usize;
        {
            push_literals(
                &mut lits,
                src,
                block_start + anchor,
                block_start + start,
                opt_w,
            );
            // W10: `seqs` was reserved to `nmatched + 1` immediately above and
            // this loop pushes exactly `nmatched` times (one per `ops` entry,
            // and after W1 every entry is a match) -- so `push`'s grow branch
            // is provably dead, yet it re-read len and capacity per sequence.
            debug_assert!(seqs.len() < seqs.capacity());
            #[allow(unsafe_code)]
            unsafe {
                let l = seqs.len();
                seqs.as_mut_ptr().add(l).write(Seq {
                    litlen: (start - anchor) as u32,
                    matchlen: ml,
                    offset: off,
                });
                seqs.set_len(l + 1);
            }
            anchor = start + ml as usize;
        }
    }
    push_lits_range(&mut lits, src, block_start + anchor, block_end);
    // Consumers are take_opt_rep / take_opt_bt gate harnesses only --
    // SEVEN un-cfg'd lock-prefixed RMWs per block in shipping, the 959e0ae
    // class, fifth sighting.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        OPT_REP_PROBES.fetch_add(o_rep_probes, Relaxed);
        OPT_REP_HITS.fetch_add(o_rep_hits, Relaxed);
        OPT_REP_BYTES.fetch_add(o_rep_bytes, Relaxed);
        OPT_BT_CALLS.fetch_add(o_bt_calls, Relaxed);
        OPT_BT_DRY.fetch_add(o_bt_dry, Relaxed);
        OPT_BT_LEN.fetch_add(o_bt_len, Relaxed);
        OPT_SEQS.fetch_add(seqs.len() as u64, Relaxed);
        OPT_POS.fetch_add(o_positions, Relaxed);
        OPT_SKIP_INF.fetch_add(o_skip_inf, Relaxed);
        OPT_SKIP_JUMP.fetch_add(o_skip_jump, Relaxed);
        OPT_SKIP_JUMPS.fetch_add(o_skip_jumps, Relaxed);
    }
    #[cfg(not(feature = "profile"))]
    let _ = (
        o_rep_probes,
        o_rep_hits,
        o_rep_bytes,
        o_bt_calls,
        o_bt_dry,
        o_bt_len,
    );
    if opt_rep_on && o_rep_probes > 0 {
        let now = o_rep_bytes as f32 / o_rep_probes as f32;
        tables.opt_rep_peak = tables.opt_rep_peak.max(now);
        #[cfg(feature = "profile")]
        {
            use core::sync::atomic::Ordering::Relaxed;
            SIG_REP_RATE.store(tables.opt_rep_rate.to_bits(), Relaxed);
            SIG_REP_PEAK.store(tables.opt_rep_peak.to_bits(), Relaxed);
            SIG_SPB.store(tables.last_search_per_byte.to_bits(), Relaxed);
        }
        tables.opt_rep_meas = tables.opt_rep_meas.saturating_add(1);
        tables.opt_rep_seen = tables.opt_rep_seen.saturating_add(1);
        // Take the MAX over the warm-up rather than an average: the question is
        // whether this content EVER repays the candidate, and a frame's first
        // blocks systematically understate it (no history to repeat against).
        tables.opt_rep_rate = if tables.opt_rep_rate == f32::MAX {
            now
        } else if tables.opt_rep_seen <= OPT_REP_WARMUP {
            tables.opt_rep_rate.max(now)
        } else {
            0.75 * tables.opt_rep_rate + 0.25 * now
        };
    }
    tables.opt_rep_probe = if tables.opt_rep_probe == 0 {
        OPT_REP_PERIOD
    } else {
        tables.opt_rep_probe - 1
    };
    // Probes reported by `bt_find_best`, which the DP calls per position.
    note_finder_work(
        cfg!(feature = "profile"),
        0,
        seqs.len() as u64,
        &seqs,
        &lits,
    );
    tables.opt_ops = ops;
    tables.opt_price = price;
    tables.opt_prev = prev;
    tables.opt_om = match_om;
    (seqs, lits)
}
