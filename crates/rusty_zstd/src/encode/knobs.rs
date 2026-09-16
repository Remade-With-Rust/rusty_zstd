//! The tuning ARMS and census COUNTERS: bench hooks, env knobs, instruments.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a
//! moved item that was private is now `pub(crate)`, so the parent can
//! still name it. That changes who may reference a symbol, not what is
//! emitted for it -- checked, not assumed: the asm board was identical
//! in all thirty-two columns across the move, which matters because
//! this crate builds at the default `codegen-units = 16`, where rustc
//! partitions codegen units BY MODULE and layout can move inlining.

use super::*;
/// Store a fast-strategy sequence. C extends the match backwards, then fills
/// hash(found_ip+2) and hash(end-2) from the *search* position, not the new start.
/// Defect B1 arm selector: back-fill the hash chain / binary tree over the
/// span a match covers, in lazy / lazy2 / btlazy2. `RZSTD_LAZY_FILL=0`
/// restores the pre-fix behaviour (jump past the match, insert nothing).
/// Ratio is deterministic, so this A/B needs exact byte counts, not timing.
/// Defect B1 arm: back-fill the chain over the span a match covers.
pub(crate) static LAZY_FILL_ENABLED_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA; shipping default from `RZSTD_LAZY_FILL`.
pub fn set_lazy_fill_arm(on: bool) {
    LAZY_FILL_ENABLED_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn lazy_fill_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match LAZY_FILL_ENABLED_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = crate::env_knob_not0("RZSTD_LAZY_FILL", true);
            LAZY_FILL_ENABLED_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Dispatch threshold for the lazy back-fill, in search positions per byte.
/// Calibrated on the deployed estimator (`RZSTD_LAZY_FILL_T` to sweep).
/// GATE 3's threshold. Was a `OnceLock` -- the same latch that made Gate 12 read
/// DEAD at every level (see `lazy_fill_stride`). Pinned at 0.0, so
/// `last_search_per_byte >= 0.0` is always true and the dispatch has never
/// actually gated anything: the same fossil shape as
/// `rep_yield_min_for(DFast) = 0.0`, which was worth 26% of the repcode probe
/// work once unpinned.
pub(crate) fn lazy_fill_threshold() -> f32 {
    use core::sync::atomic::Ordering;
    let v = LAZY_FILL_T_ARM.load(Ordering::Relaxed);
    if v != u32::MAX {
        return f32::from_bits(v);
    }
    let t: f32 = crate::env_knob_parse("RZSTD_LAZY_FILL_T").unwrap_or(0.0);
    LAZY_FILL_T_ARM.store(t.to_bits(), Ordering::Relaxed);
    t
}

pub(crate) static LAZY_FILL_T_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set Gate 3's back-fill threshold in-process.
pub fn set_lazy_fill_threshold_arm(v: f32) {
    LAZY_FILL_T_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

/// Back-fill stride (1 = every covered position). `RZSTD_LAZY_FILL_S` sweeps.
/// Stride for the BtLazy2 back-fill. 1 = insert every position a match covers.
/// Cached: runs once per EMITTED MATCH in `find_btlazy2` (L13-L15) -- the same
/// per-call `std::env::var` shape that cost 60% of L19 encode. See
/// `bt_depth_target`.
pub(crate) static BT_FILL_S_C: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

#[inline(always)]
pub(crate) fn bt_fill_stride() -> usize {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_FILL_S_C.load(Relaxed);
    if c != usize::MAX && bt_depth_cached() {
        return c;
    }
    #[cfg(feature = "std")]
    {
        let v = crate::env_knob_parse("RZSTD_BT_FILL_S")
            .filter(|v| *v >= 1)
            .unwrap_or(1);
        BT_FILL_S_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static LF_FILLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static LF_NONEMPTY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static LF_INSERTS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(fill_sites_reached, sites_with_at_least_one_insert, total_inserts)`
pub fn take_lazy_fill() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        LF_FILLS.swap(0, Relaxed),
        LF_NONEMPTY.swap(0, Relaxed),
        LF_INSERTS.swap(0, Relaxed),
    )
}

/// GATE 12: the lazy back-fill stride.
///
/// This was a `OnceLock`, which latches the environment at the FIRST call and
/// caches it for the life of the process. Any in-process A/B that sets the
/// variable after the first compression therefore measures the OLD value on both
/// arms -- which is why Gate 12 read "0/18 sizes move, DEAD" at every level while
/// the loop it controls performs 17.4M inserts at L7. The same trap is documented
/// on `step0` a few hundred lines up. Now an atomic arm, like every other gate.
pub(crate) fn lazy_fill_stride() -> usize {
    use core::sync::atomic::Ordering;
    let v = LAZY_FILL_S_ARM.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let s: usize = crate::env_knob_parse("RZSTD_LAZY_FILL_S")
        .filter(|&v: &usize| v >= 1)
        .unwrap_or(1);
    LAZY_FILL_S_ARM.store(s, Ordering::Relaxed);
    s
}

pub(crate) static LAZY_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// SECTION 14.12: the ROW finder's back-fill stride. Default **2**.
///
/// The fill re-inserts every position a match covers (defect B1). At stride 1
/// that is 41,742,765 inserts at L9 -- 2.31x the finder's own probe count,
/// each a random write into an 80 MB table -- and `fillsweep.rs` shows the
/// marginal value falling off a cliff:
///
/// | stride | fill inserts | row size |
/// |---:|---:|---:|
/// | 1 | 41,742,765 | 1.0000x |
/// | **2** | **21,749,282 (0.52x)** | **1.0027x** |
/// | 4 | 11,713,837 (0.28x) | 1.0097x |
/// | 8 | 6,740,929 | 1.0167x |
///
/// **Half the fill removed for 0.27% of size** -- and stride 2 is the KNEE,
/// which the per-corpus spread is what shows. The aggregate is dominated by
/// the big poorly-compressing files; on TEXT, stride 4 costs ~1.9% (double
/// its own mean) while stride 2 costs ~0.5%, and `nci`/`xml` actually get
/// SMALLER at 2. Going 2 -> 4 buys 10M more inserts for another ~1.2% on
/// text: half the work for more than twice the price.
///
/// The chain keeps stride 1: it pays MORE for the same thinning (+1.16% at
/// stride 4) and it is the shipping default, so it moves only on its own
/// board.
pub(crate) static ROW_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook for the row back-fill stride; `RZSTD_ROW_FILL_S` overrides.
pub fn set_row_fill_stride_arm(v: usize) {
    ROW_FILL_S_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn row_fill_stride() -> usize {
    use core::sync::atomic::Ordering;
    let v = ROW_FILL_S_ARM.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let s: usize = crate::env_knob_parse("RZSTD_ROW_FILL_S")
        .filter(|&v: &usize| v >= 1)
        .unwrap_or(2);
    ROW_FILL_S_ARM.store(s, Ordering::Relaxed);
    s
}

/// Set the lazy back-fill stride in-process.
/// GATE 6 next-long probe outcomes, for GATE 14's dispatch study.
pub static NL_PROBES_G: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static NL_HITS_G: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Total match-length GAIN the next-long probe bought, across its hits.
pub static NL_GAIN_G: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Hits in the RAISED band (`best_ml >= 8`) -- what a higher cut newly enables.
pub static NL_BAND_HITS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static NL_BAND_GAIN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static NL_BAND_OLD: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Offsets the raised-band hits take, and the offsets they replace.
pub static NL_OFF_NEW: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static NL_OFF_OLD: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Raised-band hits whose new offset is LARGER than the one they replaced.
pub static NL_OFF_WORSE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(off_new_sum, off_old_sum, hits_with_worse_offset)`.
pub fn take_nl_off() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        NL_OFF_NEW.swap(0, Relaxed),
        NL_OFF_OLD.swap(0, Relaxed),
        NL_OFF_WORSE.swap(0, Relaxed),
    )
}

/// Read and clear `(band_hits, band_gain, band_old_ml)` for the raised band.
pub fn take_nl_band() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        NL_BAND_HITS.swap(0, Relaxed),
        NL_BAND_GAIN.swap(0, Relaxed),
        NL_BAND_OLD.swap(0, Relaxed),
    )
}

/// Read and clear `(next_long_probes, next_long_hits)`.
pub fn take_next_long() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        NL_PROBES_G.swap(0, Relaxed),
        NL_HITS_G.swap(0, Relaxed),
        NL_GAIN_G.swap(0, Relaxed),
    )
}

/// GATE 14 @ L3 DISPATCH -- the signal is what the change TRADES, not what the
/// content is.
///
/// Raising the next-long cut wins on 11 corpora and loses on two (`mr` +1.111%,
/// `osdb` +0.209%). Four content signals fail to separate them: mean match
/// length, `rep_yield`, GATE 6's `next_long_yield` (non-monotonic -- winners sit
/// both above and below the losers) and gain-per-hit (dickens 2.69 wins while
/// osdb 2.47 loses).
///
/// The reason they fail is that they all describe the CONTENT. The raise does
/// not merely lengthen a match: the probe commits at `ip + 1` to a DIFFERENT
/// match, at a different OFFSET. Measured over the band the raise actually opens
/// (`best_ml >= 8`), the share of hits taking a LARGER offset than the one they
/// replace separates cleanly:
///
/// ```text
///   winners (11)   33.8% .. 64.6%      offset ratio 0.59x .. 1.68x
///   osdb           76.8%               3.67x
///   mr             79.0%               2.78x
/// ```
///
/// A far match costs offset bits and resets `offset_1` to a distant value,
/// breaking the repcode chain the next positions would have used.
///
/// WARM-UP + RE-PROBE, for the reason GATES 6, 2 @ L3 and 10 @ L19 all needed
/// one: with the cut at 8 the raised band never fires, so the signal cannot be
/// measured and a naive gate latches shut on its first bad block forever.
pub(crate) const NL_BAND_WARMUP: u32 = 2;
pub(crate) const NL_BAND_PERIOD: u32 = 16;

pub(crate) static NL_OFF_WORSE_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the worse-offset share above which the cut stays at 8.
pub fn set_nl_off_worse_arm(v: f32) {
    NL_OFF_WORSE_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
/// WAS MEASURED INERT, AND IS NOW LIVE. Until 2026-09-08 this bar was
/// unreachable on the default path: its only reader is `nl_cut_for`, which
/// opens with `if NL_DISPATCH_ON != 2 { return 8; }`, and `NL_DISPATCH_ON`
/// initialised to 0 -- so the sweep that chose 0.60 was tuning a threshold
/// nothing consulted. `NL_DISPATCH_ON` now initialises to 2 and this bar
/// decides on every DFast block.
pub(crate) fn nl_off_worse_max() -> f32 {
    let v = NL_OFF_WORSE_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        // Swept: 0.70 gives the best aggregate (-0.1125%) but leaves mr at
        // +0.465%; 0.60 is the knee where the regression essentially vanishes
        // (mr +0.047%, worst corpus osdb +0.080%) for -0.0869%. The brief is
        // speed with MINIMAL quality cost, so the knee wins over the optimum.
        0.60
    } else {
        f32::from_bits(v)
    }
}

/// The next-long cut for THIS block: raised while the trade is paying, and
/// during warm-up and every re-probe so the signal can be refreshed.
/// DEFAULT OFF, and the reason is a work ledger I got wrong once already.
///
/// Raising this cut is a SIZE win (-0.0940% dispatched) and a SPEED LOSS. The
/// first ledger counted main-loop POSITIONS only and read -0.38%; but raising
/// the cut makes the next-long PROBE fire more often, and each firing is a hash
/// lookup plus `match_ok` plus `count_match` that the position counter never
/// sees. Both sides:
///
/// ```text
///   positions   -24,683
///   nl probes  +336,112
///   NET ops    +311,429      and a timed +3.43% against a 2.20% null
/// ```
///
/// Same half-ledger error as 4.40's back-fill. The brief is speed with minimal
/// quality cost, so the raise stays OFF; the dispatch, its signal and its arms
/// are kept because the SIGNAL is sound (it separates cleanly, see 4.51) and the
/// trade may be worth taking at a level where size dominates.
/// 0 = off, 2 = on. DEFAULTS ON since 2026-09-08 -- see the measurement in
/// `nl_cut_for`. `set_nl_dispatch_arm(false)` restores the old behaviour.
pub(crate) static NL_DISPATCH_ON: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(2);

/// Bench hook: enable the next-long raise + its offset-trade dispatch.
pub fn set_nl_dispatch_arm(on: bool) {
    NL_DISPATCH_ON.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub(crate) fn nl_cut_for(tables: &MatchTables) -> usize {
    if NL_DISPATCH_ON.load(core::sync::atomic::Ordering::Relaxed) != 2 {
        return 8;
    }
    if tables.nl_band_meas < NL_BAND_WARMUP
        || tables.nl_band_probe == 0
        || tables.nl_off_worse <= nl_off_worse_max()
    {
        dfast_good_ml_raised()
    } else {
        8
    }
}

/// The raised value the dispatch selects when the trade is paying.
#[inline(always)]
pub(crate) fn dfast_good_ml_raised() -> usize {
    let v = DFAST_GOOD_ML_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        // 24 -> 48 (2026-09-08). `mlgrid.rs` sweeps this against
        // `dfast_good_ml2` with the dispatch live: 24 is -68,999 B on the
        // 18-corpus L3 board, 48 is -82,653 B. The grid's best cell is
        // (64, 24) at -82,975 B -- 322 bytes better and at an EDGE, so 48
        // is taken off the plateau instead (everything in 40..64 lands
        // within 0.03% of each other, which is the shape of a corpus fit,
        // not an optimum).
        48
    } else {
        v
    }
}

/// GATE 14 @ L19 study: read the per-block signals the encoder already
/// maintains, so a dispatch can be tested WITHOUT adding instrumentation to a
/// 264M-probe path.
pub static SIG_REP_RATE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
pub static SIG_REP_PEAK: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
pub static SIG_SPB: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Read `(opt_rep_rate, opt_rep_peak, last_search_per_byte)` as last published.
pub fn take_opt_signals() -> (f32, f32, f32) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        f32::from_bits(SIG_REP_RATE.load(Relaxed)),
        f32::from_bits(SIG_REP_PEAK.load(Relaxed)),
        f32::from_bits(SIG_SPB.load(Relaxed)),
    )
}

/// GATE 14 @ L3 -- the DEPTH CUT DFast actually has.
///
/// GATE 14 proper (`bt_depth_apply`) is dead at L3 twice over: L3 makes ZERO
/// `bt_find_best` calls, and `bt_depth_cut` excludes non-opt strategies anyway.
/// But "stop searching once the match in hand is good enough" is exactly what a
/// depth cut IS, and DFast has one -- a bare `8` at two sites:
///
///   * gating the GATE 6 next-long probe at `ip + 1`
///   * gating the second (short-hash) candidate check at `ip`
///
/// Both were hardcoded and never gated, the same shape as the search-strength
/// shift of 4.43 (four sites, never gated, the biggest L1 speed lever found).
/// Lower = accept a shorter match and stop early; higher = keep looking.
pub(crate) static DFAST_GOOD_ML_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook: the "good enough, stop searching" match length for DFast.
/// 0 restores the shipped 8.
pub fn set_dfast_good_ml_arm(v: usize) {
    DFAST_GOOD_ML_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// The same cut, for the SECOND-CANDIDATE site only. The constant governed two
/// mechanisms with different characters -- the next-long probe COMMITS at
/// `ip + 1` (it changes the parse), while the short-hash check only adds a
/// candidate at `ip` (it cannot make the match shorter). They are swept apart.
pub(crate) static DFAST_GOOD_ML2_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook: the second-candidate cut. 0 follows `dfast_good_ml`.
pub fn set_dfast_good_ml2_arm(v: usize) {
    DFAST_GOOD_ML2_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// SHIPPED at 24, against the next-long site's 8.
///
/// Splitting the two sites is the whole finding. Raising the NEXT-LONG cut is
/// the bigger lever (-0.083% size, -0.66% probes) but regresses `mr` by 1.109%,
/// and FOUR signals fail to separate that: mean match length (webster 10.60
/// wins, mr 9.58 loses), `rep_yield`, GATE 6's own `next_long_yield` (losers sit
/// at 0.056-0.074 with winners both above AND below), and the probe's mean
/// length gain per hit (losers 16.3-17.9, inside the winners' 8.6-26.3). A
/// non-monotonic split with no separating signal is the doctrine's PRUNE case,
/// so the next-long cut stays at 8 and the arm stays settable.
///
/// The second-candidate site carries no such risk: it only ADDS a candidate at
/// `ip`, it cannot make the chosen match shorter or move the commit point, so it
/// cannot restructure the parse. Measured across all 18 at 24: L3 -0.0226%
/// size, L4 -0.0340%, worst corpus +0.0065% (osdb) -- at the noise floor.
#[inline(always)]
pub(crate) fn dfast_good_ml2() -> usize {
    let v = DFAST_GOOD_ML2_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        // REVERTED to 8. The size win was real (-0.0266%) but so was the cost:
        // raising this makes the short-hash candidate check run on every
        // position with `best_ml` in [8, 24) instead of [0, 8), and that work
        // was in NEITHER of the ledgers used to justify it. Timed in isolation
        // with every other arm pinned, the dose-response is monotonic --
        // cand2=16 +1.30%, cand2=24 +2.49% -- and three independent runs put
        // the whole gate at +2.47%, +2.49% and +3.43% SLOWER.
        //
        // The brief is speed with minimal quality cost. This is size at a speed
        // cost, which is the opposite trade.
        8
    } else {
        v
    }
}

/// GATE 12 @ L3. DFast's back-fill is not a span walk -- it inserts exactly two
/// positions per match (`match_ip+2` and `match_end-2`), mirroring C
/// `zstd_double_fast.c`. So `lazy_fill_stride` was never wired to it: that knob
/// controls `find_lazy`'s loop, which L3 never enters. Reading "DEAD at L3" off
/// it measured a loop with no caller, exactly as GATE 9 @ L3 did.
///
/// This is the density knob DFast actually lacks: `s != 0` also inserts the
/// interior positions of the match span on a stride. 0 = today (the two ends).
pub(crate) static DFAST_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Bench hook: interior back-fill stride for DFast. 0 restores today's two-ends fill.
pub fn set_dfast_fill_stride_arm(v: usize) {
    DFAST_FILL_S_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn dfast_fill_stride() -> usize {
    let v = DFAST_FILL_S_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v != usize::MAX {
        return v;
    }
    let s: usize = crate::env_knob_parse("RZSTD_DFAST_FILL_S").unwrap_or(0);
    DFAST_FILL_S_ARM.store(s, core::sync::atomic::Ordering::Relaxed);
    s
}

/// GATE 12 @ L3 work ledger: table WRITES performed by the two per-match end
/// fills (short and long counted separately). The sparse arm's saving is paid
/// in this unit; §4.39 priced only the main-loop positions it costs and so
/// called the arm "dominated" while ignoring the larger term.
pub static DF_ENDFILL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the per-match end-fill write count.
pub fn take_dfast_endfill() -> u64 {
    DF_ENDFILL.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// GATE 12 @ L3, the SPARSE direction. DFast writes four table entries per
/// match -- `match_ip+2` and `match_end-2`, into both the short and the long
/// hash -- unconditionally, and nothing has ever asked whether both earn it.
/// This is the only direction at L3 that REMOVES work.
///
/// 0 = unresolved, 1 = neither, 2 = start+2 only, 3 = both, 4 = end-2 only.
///
/// DEFAULT FLIPPED to 2 (start only), 2026-08-27, on the `fillcut.rs` board.
/// The two per-match positions are NOT worth the same: over 12 corpora the
/// whole fill buys 1.919% of ratio, of which the START half buys 1.548% and
/// the END half only 0.371% -- for exactly half the table writes. Boarded per
/// level (this arm is read by `find_fast` too, so it covers L1 as well as
/// DFast):
///
/// ```text
///   L1   1,872,359 -> 937,927 fills (0.50x)   size +0.150%
///   L3   6,448,402 -> 3,230,205 fills (0.50x) size +0.482% (2 MiB cap)
///   L9/L19  unaffected -- different finders, zero fills through here
/// ```
///
/// This CHANGES THE BITSTREAM. bytegate GOLD moved from BE0071FB0CB0CED9 to
/// the value recorded in `bytegate.rs`'s header; that is the deliberate
/// re-gold, not a regression.
pub(crate) static DFAST_FILL_N_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(2);

/// Bench hook: 0 = no end fills, 1 = start+2 only, 2 = both (today), 3 = end-2 only.
pub fn set_dfast_fill_n_arm(n: u8) {
    DFAST_FILL_N_ARM.store(n + 1, core::sync::atomic::Ordering::Relaxed);
}

/// `(fill_start, fill_end)` for the two per-match positions.
#[inline]
pub(crate) fn dfast_fill_ends() -> (bool, bool) {
    match DFAST_FILL_N_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => (false, false),
        2 => (true, false),
        4 => (false, true),
        _ => (true, true),
    }
}

/// GATE 12 @ L3, sibling finding. The short fill anchors on `best_ip`, the long
/// fill on `ip`. They differ by one whenever the next-long probe wins, so the two
/// halves of the DOUBLE hash record DIFFERENT positions for the same match
/// (short at `ip+3`, long at `ip+2`). C fills both tables at the same two
/// positions -- `curr+2` and `ip-2` -- so this is a divergence, not a design.
///
/// 0 = unresolved, 1 = today (`ip`), 2 = C-consistent (`best_ip`).
pub(crate) static DFAST_FILL_A_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `true` anchors BOTH DFast fills on the committed match start.
pub fn set_dfast_fill_anchor_arm(c: bool) {
    DFAST_FILL_A_ARM.store(u8::from(c) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn dfast_fill_anchor_c() -> bool {
    DFAST_FILL_A_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}

/// Interior back-fill positions inserted by GATE 12 @ L3's stride arm.
pub static DF_FILL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Bench hook: interior DFast back-fill inserts since the last call.
pub fn take_dfast_fill() -> u64 {
    DF_FILL.swap(0, core::sync::atomic::Ordering::Relaxed)
}

pub fn set_lazy_fill_stride_arm(v: usize) {
    LAZY_FILL_S_ARM.store(v.max(1), core::sync::atomic::Ordering::Relaxed);
}

/// Brick 40 arm selector: repcode-1 search in `find_fast`. **Default OFF --
/// NOT YET SHIPPABLE**, set `RZSTD_REP1=1` to enable.
///
/// Measured (exact bytes, L1, Silesia): a net ratio win on 9 of 12 files
/// (reymont -3.05%, x-ray -1.15%, samba -1.06%, webster -0.56%) but a LOSS on
/// xml (+3.39%), nci (+0.34%) and ooffice (+0.34%).
///
/// It also introduces a regression I have not root-caused: **Repeat FSE mode
/// (seq mode 3) stops being selected entirely** once every block carries
/// offset-code 0, so we pay three table headers on every block. Fixing that
/// interaction should lift the ratio win across the board; until then this
/// does not ship. The bit accountant showed our size gap vs C is ENTIRELY literals
/// (webster: our literals 14.3 MB vs C's 6.7 MB, while our sequences are
/// SMALLER) -- C finds more matches, so it has fewer literals to code. C's
/// `ZSTD_compressBlock_fast` tests the repeat offset at every position; we only
/// ever ENCODED a repcode when an offset happened to coincide, never SEARCHED
/// for one.
/// GATE 2 arm: the repcode-1 search, as a THREE-state choice so both constants
/// are reachable. The old `rep1_enabled()` arm (Gate 10) could only force ON,
/// which cannot answer "does any corpus lose under a constant" -- the OFF
/// constant was untestable. This replaced it and Gate 10 is now deleted.
/// 0 = unset (measured dispatch), 1 = force OFF, 2 = force ON.
pub(crate) static REP1_MODE_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook. `None` restores the measured dispatch.
pub fn set_rep1_mode(m: Option<bool>) {
    REP1_MODE_ARM.store(
        match m {
            None => 0,
            Some(false) => 1,
            Some(true) => 2,
        },
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// DEFECT (GATE 2 @ L1's second variable, found during GATE 12 @ L1).
///
/// `rep_len_ratio` starts at 1.0, the gate is `rep_len_ratio >= rep_len_min()`
/// with `rep_len_min()` == 1.0, and the ONLY code that lowers it sits after the
/// pipelined loop's early `return`. 42% of blocks take that return -- 93.8% on
/// eight of the eighteen corpora -- so on those the OR clause is pinned TRUE
/// from the first block and Gate 2's dispatch can never shut the repcode search
/// off, however low the measured yield.
///
/// Third instance of this exact early-return class in `find_fast`: `tag_yield`
/// and the GATE 6 re-probe countdown were both fixed here before it.
///
/// `false` restores the defect so the two can be A/B'd in one process.
pub(crate) static REPLEN_PIPE_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores the pre-fix behaviour (ratio never updated on
/// the pipelined path).
pub fn set_replen_pipe_arm(fixed: bool) {
    REPLEN_PIPE_ARM.store(u8::from(fixed) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn replen_pipe_fixed() -> bool {
    REPLEN_PIPE_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// The search-strength shift in `ip += step + ((ip - anchor) >> N)`.
///
/// After `2^N` positions without a match resetting `anchor`, the stride grows by
/// one; the growth is what makes match-poor content cheap. It is the knob that
/// PRODUCES the positions/byte spread across the corpus -- dickens 0.561 against
/// x-ray 0.028 and incomp 0.0014 -- and until now it was a hardcoded `8` at all
/// four sites (both loops of `find_fast`, both of `find_dfast`), never gated.
///
/// C `zstd` calls this `kSearchStrength` and also uses 8. Our compressed bytes
/// are not required to match C's, so it is ours to move. Unlike the back-fill
/// writes of 4.40, positions are DEPENDENT work on the critical path.
pub(crate) static ACCEL_SHIFT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: search-strength shift. 8 is the shipped default.
pub fn set_accel_shift_arm(n: u32) {
    ACCEL_SHIFT_ARM.store(n, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub(crate) fn accel_shift_base() -> u32 {
    let v = ACCEL_SHIFT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v != u32::MAX {
        return v;
    }
    let n: u32 = crate::env_knob_parse("RZSTD_ACCEL")
        .filter(|&n| (1..=24).contains(&n))
        .unwrap_or(0);
    ACCEL_SHIFT_ARM.store(n, core::sync::atomic::Ordering::Relaxed);
    n
}

/// DISPATCHED ON STRATEGY. Fast (L1-L2) accelerates one step harder.
///
/// The win is a POSITION count, and positions are dependent, latency-bound work
/// -- the opposite of 4.40's back-fill writes, where -25% of the count bought
/// exactly 0. Here -10.15% of positions buys -5.57% of L1 encode time against a
/// 0.36% null (sao -22.20%, mozilla -10.41%, mr -6.02%), for +0.2206% size,
/// worst corpus sao +0.796%.
///
/// It is dispatched rather than constant because the same shift is WORTH LESS
/// higher up: DFast removes only 2.41% of positions at shift 7 (against L1's
/// 10.15%) and times at -0.91% inside a 1.78% null. Fast has no second-chance
/// long hash, so its main loop is a larger share of total time and its skipped
/// positions are cheaper to give up.
///
/// `RZSTD_ACCEL` pins both arms for A/B.
#[inline(always)]
pub(crate) fn accel_shift_for(strategy: Strategy) -> u32 {
    let pinned = accel_shift_base();
    if pinned != 0 {
        return pinned;
    }
    if strategy == Strategy::Fast {
        7
    } else {
        8
    }
}

/// The Gate 2 decision for this block: forced constant, or the measured yield.
#[inline]
pub(crate) fn rep_search_on(rep_yield: f32, strategy: Strategy) -> bool {
    match REP1_MODE_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        // GATE 10 REMOVED. This was `rep1_enabled() || rep_yield >= min`, where
        // `rep1_enabled()` was a second arm that could only force ON -- exactly
        // what `REP1_MODE_ARM::Some(true)` above already does. It was DEAD at
        // L3/L19/L22 (the DFast threshold is 0.0 and find_opt prices reps
        // itself) and live only at L1, and its OR shape was a footgun: setting
        // `RZSTD_REP1=1` silently short-circuited the whole Gate 2 dispatch,
        // including the `rep_len_ratio` variable. Its default was `false`, so
        // deleting it is byte-identical at the shipped configuration.
        _ => rep_yield >= rep_yield_min_for(strategy),
    }
}

/// C `zstd_fast.c`: a repeat-offset match tested at `ip+1`. Returns its length.
#[inline(always)]
pub(crate) fn try_rep1(
    src: &[u8],
    ip: usize,
    rep1: usize,
    lowest: usize,
    block_end: usize,
    // W3: the bound as the CALLER states it. Every one of the seven call
    // sites sits inside `while ip <= ilimit` with
    // `ilimit = block_end.saturating_sub(8)` and the `block_start >= ilimit`
    // early-out above it, so `ip <= ilimit` -- and therefore `at + 4 <=
    // block_end` -- is already proven where this runs. Phrasing the guard as
    // the loop's OWN condition lets LLVM delete it outright at those sites
    // (it was `lea`, `cmp`, `ja` plus a `block_end` reload, per POSITION on
    // every ladder) while keeping a real guard for any caller that cannot
    // prove it.
    ilimit: usize,
) -> Option<usize> {
    let at = ip + 1;
    if rep1 == 0 || ip > ilimit || at < rep1 {
        return None;
    }
    debug_assert!(at + 4 <= block_end);
    let back = at - rep1;
    if back < lowest {
        return None;
    }
    // BRICK 91 (P22): the width from `ip < ilimit` -- `ilimit` is
    // `block_end - 8` at every caller (see above), so this is
    // `at + 8 <= block_end` on operands the caller's loop already holds.
    debug_assert_eq!(ip < ilimit, at + 8 <= block_end);
    rep1_len_w(src, at, back, block_end, ip < ilimit)
}

/// The compare half of `try_rep1` (BRICK 38): `at` and `back` are already
/// admissible (`at >= rep1 + lowest`, `at + 4 <= block_end`). `find_lazy_impl`
/// calls this directly behind its one-compare `rep_bar` gate.
#[inline(always)]
pub(crate) fn rep1_len(src: &[u8], at: usize, back: usize, block_end: usize) -> Option<usize> {
    rep1_len_w(src, at, back, block_end, at + 8 <= block_end)
}

/// `rep1_len` with the width decided by the caller (BRICK 86): inside
/// `while ip <= ilimit`, `at + 8 <= block_end` is `ip < ilimit`, a compare
/// the loop already has the operands for.
#[inline(always)]
pub(crate) fn rep1_len_w(
    src: &[u8],
    at: usize,
    back: usize,
    block_end: usize,
    wide: bool,
) -> Option<usize> {
    debug_assert!(at + 4 <= block_end && back < at);
    debug_assert_eq!(wide, at + 8 <= block_end);
    // Same fused head as `fast_probe`: one u64 pair gates AND answers 4..7.
    if wide {
        let x = load_u64le(src, back) ^ load_u64le(src, at);
        if x as u32 != 0 {
            return None;
        }
        return Some(if x != 0 {
            (x.trailing_zeros() as usize) >> 3
        } else {
            8 + count_match(src, back + 8, at + 8, block_end)
        });
    }
    if load_u32le(src, back) != load_u32le(src, at) {
        return None;
    }
    Some(4 + count_match_fast(src, back + 4, at + 4, block_end))
}

/// BRICK 38 (P2): the first position at which the rep-1 probe is admissible.
/// `try_rep1` admits `ip` iff `rep1 != 0`, `ip + 1 >= rep1` and
/// `ip + 1 - rep1 >= lowest` -- the last implies the second, so the three
/// are `ip >= rep1 + lowest - 1`; a disabled probe is `usize::MAX`, which no
/// `ip <= ilimit` reaches. Recomputed only when `rep1` changes.
#[inline(always)]
pub(crate) fn rep_bar_for(use_rep: bool, rep1: usize, lowest: usize) -> usize {
    if use_rep && rep1 != 0 {
        rep1 + lowest - 1
    } else {
        usize::MAX
    }
}

/// Base probe step for the Fast strategy when `target_length == 0`.
/// Probe-density arm (gg-matchfind Gate 9). Settable at RUNTIME so the harvest
/// can interleave both arms inside ONE process -- a `OnceLock` here made every
/// step0 measurement a separate process run, minutes apart, on a box that
/// drifts. 0 = not yet resolved, else `step0 + 1`.
pub(crate) static STEP0_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook for in-process ABBA; shipping default from `RZSTD_STEP0`.
pub fn set_step0_arm(step: usize) {
    STEP0_ARM.store(step.max(1) + 1, core::sync::atomic::Ordering::Relaxed);
}

/// GATE 18 @ L1: how many blocks to alternate before latching, and how often to
/// re-probe. Four blocks gives two samples per arm on adjacent content.
/// Probe blocks per decision, and blocks between re-probes.
///
/// A probe block runs the search TWICE, so the probe's own cost is
/// `2 * BLOCKS / PERIOD` of the total search. At 4 and 32 that is 12.5% against
/// a 15% saving -- measured at +2.62% SLOWER, the fifth time in this campaign
/// that an instrument outweighed what it measured. At 1 and 256 it is 0.4%.
pub(crate) const STEP_PROBE_BLOCKS: u32 = 1;
/// Bytes a sequence costs once entropy-coded, for the probe's size proxy.
/// Literal count plus this per sequence tracks emitted size closely enough to
/// rank two parses; coverage does not (4.70).
pub(crate) const SEQ_BYTES_EST: f64 = 3.0;
pub(crate) const STEP_REPROBE_PERIOD: u32 = 256;

pub(crate) static STEP_PROBE_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores the pinned step-1 behaviour on route 1.
pub fn set_step_probe_arm(on: bool) {
    STEP_PROBE_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

/// DEFAULT OFF. The complete ledger says this loses.
///
/// `pair_route == 2` does not SKIP the pair search -- it RUNS it, with step 2.
/// Route 1 is the cheap arm: no pair search, step 1. So routing 1 -> 2 halves
/// main-loop positions and DOUBLES pair probes:
///
/// ```text
///   positions    28,411,771 -> 22,941,198   -5,470,573  (-19.25%)
///   pair probes   8,323,627 -> 16,658,004   +8,334,377  (+100.13%)
///   NET                                     +2,863,804  (+7.80%)
/// ```
///
/// and the clock agrees at +2.21% against a 1.46% null. The -0.0131% size and
/// -19.25% positions that looked like a free win were a HALF LEDGER: pair probes
/// were never counted.
///
/// The machinery is kept because the probe itself is sound and reusable -- it
/// measures a counterfactual from identical state on a cloned table -- and
/// because the size result (-0.0131%) says route 2 genuinely parses better. What
/// it does not do is parse CHEAPER.
#[inline(always)]
pub(crate) fn step_probe_on() -> bool {
    STEP_PROBE_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}

/// GATE 18 study: the measured step-2 forfeit, per mille x10.
#[cfg(feature = "profile")]
pub static STEP_FORFEIT_SUM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STEP_FORFEIT_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STEP_SEQ_SUM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(sum_x10000, n)`.
#[cfg(feature = "profile")]
pub fn take_step_forfeit() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        STEP_FORFEIT_SUM.swap(0, Relaxed),
        STEP_FORFEIT_N.swap(0, Relaxed),
        STEP_SEQ_SUM.swap(0, Relaxed),
    )
}

/// Record one probe block: how much match coverage step 2 forfeits against
/// step 1, both measured from the same starting tables.
pub(crate) fn note_step_probe(
    tables: &mut MatchTables,
    seqs1: &[Seq],
    lits1: usize,
    seqs2: &[Seq],
    lits2: usize,
) {
    if seqs1.is_empty() {
        return;
    }
    // Judge on a SIZE PROXY, not on coverage. 4.70 measured coverage forfeit
    // ANTI-correlating with the true cost -- samba forfeits the least and pays
    // the most -- because a match the cheap route misses is usually re-found a
    // byte later as an extra short sequence. Literals plus a per-sequence
    // overhead tracks the emitted bytes; coverage does not.
    let est1 = lits1 as f64 + seqs1.len() as f64 * SEQ_BYTES_EST;
    let est2 = lits2 as f64 + seqs2.len() as f64 * SEQ_BYTES_EST;
    let forfeit = est2 / est1.max(1.0) - 1.0;
    let seq_ratio = 0.0;
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        STEP_SEQ_SUM.fetch_add((seq_ratio * 10000.0) as u64, Relaxed);
    }
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        STEP_FORFEIT_SUM.fetch_add((forfeit.max(0.0) * 10000.0) as u64, Relaxed);
        STEP_FORFEIT_N.fetch_add(1, Relaxed);
    }
    let _ = seq_ratio;
    tables.step_sum1 += forfeit;
    tables.step_sum2 += 1.0;
    tables.step_probed = tables.step_probed.saturating_add(1);
    if tables.step_probed >= STEP_PROBE_BLOCKS {
        let n = f64::from(tables.step_probed);
        let mean_forfeit = tables.step_sum1 / n;
        let mean_seq = 0.0f64;
        // TWO variables, each catching a case the other misses.
        //
        // `seq_ratio` -- the share of the sequence COUNT that survives step 2 --
        // is the size predictor. samba 0.9332 and mozilla 1.0144 keep nearly
        // every sequence and cost +9.1% and +13.6%: the matches step 2 skips are
        // re-found a byte later as extra short sequences, so the entropy bill
        // rises while the search saving is spent. mr 0.3916, sao 0.6575 and
        // dickens 0.7451 shed sequences instead, and are free.
        //
        // `forfeit` -- match bytes lost -- catches x-ray, whose seq_ratio is a
        // very low 0.1250 but which loses 89% of its coverage: there step 2 does
        // not restructure the parse, it destroys it (+25.1%).
        //
        // Coverage ALONE is anti-correlated with cost (samba forfeits the least
        // and costs the most), which is why four content signals and two earlier
        // probe designs failed here.
        let _ = mean_seq;
        tables.step_pick = if mean_forfeit < step_forfeit_max() {
            2
        } else {
            1
        };
        tables.step_reprobe = STEP_REPROBE_PERIOD;
        tables.step_probed = 0;
        tables.step_sum1 = 0.0;
        tables.step_sum2 = 0.0;
    }
}

/// Feed a probe block's measured counterfactual back to the step gate.
///
/// `step_sum1` carries the match bytes committed at positions step 2 would
/// SKIP, and `step_sum2` the total match bytes. Their ratio is what step 2 would
/// forfeit on this content, measured on a single step-1 pass with no double
/// search and no table pollution.
pub(crate) fn note_step_outcome(tables: &mut MatchTables, _payload: usize, _block_len: usize) {
    if tables.step_pick != 0 && tables.step_reprobe > 0 {
        tables.step_reprobe -= 1;
    }
}

pub(crate) static STEP_FORFEIT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the share of match bytes step 2 may forfeit before it is refused.
pub fn set_step_forfeit_arm(v: f32) {
    STEP_FORFEIT_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
/// MEASURED INERT (`dispatchaudit.rs`, every level): unreachable on the
/// default path. Its only reader is `note_step_probe`, which runs only inside
/// `encode_block`'s `probing` branch -- gated on `step_probe_on()`, i.e.
/// `STEP_PROBE_ARM == 2`, and that arm initialises to 0. The whole GATE 18
/// step-probe cluster (the `tables.clone()`, the second `find_sequences`, the
/// `step_pick`/`step_reprobe` route at `find_fast`) is off unless a bench
/// enables it. Its sibling knob `set_step_seq_arm` was removed outright: it had
/// no reader at all.
pub(crate) fn step_forfeit_max() -> f64 {
    let v = STEP_FORFEIT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        0.002
    } else {
        f64::from(f32::from_bits(v))
    }
}

// STEP_SEQ_ARM REMOVED. It was a `pub` setter storing into a static that
// nothing read: the sequence-ratio signal it tuned was abandoned (see the
// `let _ = mean_seq` note at the `step_pick` site), and the knob outlived it.
// `dispatchaudit.rs` reports it INERT at every level for the same reason
// `set_block_avx2_arm` was: there is no reader to be inert against.

pub(crate) fn step0_default() -> usize {
    use core::sync::atomic::Ordering;
    let v = STEP0_ARM.load(Ordering::Relaxed);
    if v != 0 {
        return v - 1;
    }
    let on = crate::env_knob_parse("RZSTD_STEP0")
        .filter(|&v: &usize| v >= 1)
        .unwrap_or(2);
    STEP0_ARM.store(on + 1, Ordering::Relaxed);
    on
}

/// ffanat hash-width: the per-block spec for the Fast ladder's table hash.
/// Legacy = 4 bytes at every `min_match` (this codebase's historical choice);
/// wide = `mls` bytes, C's `ZSTD_hashPtr` design. The census that motivated
/// this (`ffwaste`): with the 4-byte hash, **82.9% of candidates whose four
/// bytes match die below `min_match` at L1** (sao 96.1%, mr 95.7%, x-ray
/// 98.9%) -- ~14M wasted random loads + compares + `count_match` calls per
/// 12-corpus pass. Keying the table on the bytes acceptance actually needs
/// removes the waste at its source.
#[derive(Clone, Copy)]
pub(crate) struct FastHash {
    pub(crate) wide: bool,
    pub(crate) mask: u64,
    pub(crate) shift: u32,
}

pub(crate) const FAST_HASH_PRIME64: u64 = 0x9E37_79B1_85EB_CA87;

#[inline(always)]
pub(crate) fn fast_hash_spec(mls: usize, hash_log: u32) -> FastHash {
    if fast_hash_wide_enabled() && (5..=8).contains(&mls) {
        FastHash {
            wide: true,
            mask: if mls == 8 {
                u64::MAX
            } else {
                (1u64 << (8 * mls)) - 1
            },
            shift: 64u32.saturating_sub(hash_log),
        }
    } else {
        FastHash {
            wide: false,
            mask: 0,
            shift: 32u32.saturating_sub(hash_log),
        }
    }
}

/// Wide load with a zero-extended tail: several call sites are only 4-byte
/// safe (`ip + 1`, fill ends), so inside the last 8 bytes the missing bytes
/// read as zero. Deterministic and CONSISTENT between store and load -- a
/// tail-keyed slot can only ever be matched against the same tail key, and
/// every candidate is verified by compare + `count_match` regardless.
#[inline(always)]
pub(crate) fn load_u64le_tail(src: &[u8], pos: usize) -> u64 {
    if pos + 8 <= src.len() {
        return crate::simd::load_u64_le(src, pos);
    }
    // BACKOFF LOAD: one aligned-window load at len - 8, shifted right by the
    // overhang, replaces the up-to-7-iteration byte-assembly loop (a load,
    // an or and a variable shift PER BYTE). Value-exact: little-endian, the
    // shift discards exactly the bytes below `pos` and zero-fills the high
    // end, which is what the loop produced. The loop survives only for
    // sub-8-byte inputs.
    let len = src.len();
    if len >= 8 && pos < len {
        let over = (pos + 8 - len) as u32;
        return crate::simd::load_u64_le(src, len - 8) >> (8 * over);
    }
    let mut v = 0u64;
    let mut i = 0;
    while pos + i < len {
        v |= u64::from(src[pos + i]) << (8 * i);
        i += 1;
    }
    v
}

/// The Fast ladder's hash+tag, SCALARIZED. The struct form kept `FastHash` on
/// the stack and the live wide copy reloaded mask, shift, AND the wide flag
/// per position -- with the table base re-spilled beside them (`296(%rbp)`
/// twice, `shrq %cl` from `64(%rbp)` in an HLOG=14 copy that should emit
/// `shrq $50`). Scalars stay in registers, and `SAFE = true` sites (proven
/// `pos <= ilimit`, i.e. `pos + 8 <= block_end`) skip the tail branch and its
/// inline byte-loop entirely. The tag remains sound in both modes: it is a
/// function of bytes the accepted match must reproduce.
#[inline(always)]
pub(crate) fn fast_hash_tag<const SAFE: bool>(
    src: &[u8],
    pos: usize,
    wide: bool,
    mask: u64,
    shift: u32,
) -> (usize, u8) {
    if wide {
        let v = if SAFE {
            debug_assert!(pos + 8 <= src.len());
            crate::simd::load_u64_le(src, pos)
        } else {
            load_u64le_tail(src, pos)
        } & mask;
        let hv = v.wrapping_mul(FAST_HASH_PRIME64);
        // BRICK 52: the byte under the bucket (the top bits ARE the bucket).
        ((hv >> shift) as usize, ((hv << 8) >> shift) as u8)
    } else {
        let hv = load_u32le(src, pos).wrapping_mul(HASH4_PRIME);
        ((hv >> shift) as usize, (hv ^ (hv >> 15)) as u8)
    }
}

/// hash4 index AND its 8-bit tag, from one multiply.
///
/// The tag is a pure function of the 4 bytes at `pos`, and `fast_probe`
/// requires those 4 bytes to be EQUAL -- so a tag mismatch implies the bytes
/// differ, i.e. the tag can only reject candidates the probe would reject
/// anyway. That is what makes the whole scheme byte-identical by construction.
#[inline(always)]
/// DFast's short-slot hasher with the MLS-WIDTH tag. The INDEX is bit-exact
/// `hash4_tag`'s (the u32 gram times HASH4_PRIME, shifted) -- same slots,
/// byte-identity by construction. The TAG sees `min(mls, 8)` bytes via
/// `smask`, because the short consume-site census found the 4-byte tag's
/// blind spot: survivors share the tag's whole 4 bytes and die at byte 5
/// against mls = 5 -- 8,453,099 wasted random loads per board pass (32.2%
/// of the unfiltered waste; the long table's same class measured 0.42%).
/// SAFETY: every caller is bounded by `ilimit = block_end - 8` (or primes
/// with `p + 8 <= len`), so the u64 load is in bounds.
/// Soundness: acceptance verifies `mls` leading bytes, and the tag is a
/// function of `min(mls, 8)` of them -- a mismatch cannot hide a match.
pub(crate) fn hash4_tag_mls(src: &[u8], pos: usize, hash_shift: u32, smask: u64) -> (usize, u8) {
    hash4_tag_from(load_u64le(src, pos), hash_shift, smask)
}

/// The mixing half of `hash4_tag_mls`, split from the LOAD.
///
/// `hash4_tag_mls` and `hash8_shift` both begin `load_u64le(src, pos)`, and
/// DFast's fill calls both at the SAME position (`match_end - 2`, which both
/// tables index identically). Splitting the load out lets one `load_u64le`
/// feed both mixes instead of two.
#[inline(always)]
pub(crate) fn hash4_tag_from(v: u64, hash_shift: u32, smask: u64) -> (usize, u8) {
    let hv = (v as u32).wrapping_mul(HASH4_PRIME);
    let tv = (v & smask).wrapping_mul(FAST_HASH_PRIME64);
    // BRICK 66 (F4, brick 52 retried inlined): the product's TOP byte is the tag -- as well mixed as
    // the xor-fold it replaces (every bucket here is a product's top bits)
    // and two instructions to seat in the head word instead of four. Every
    // producer of a chain/row/short-table tag takes it from here or writes
    // the same expression; the wide-chain producers take the byte under
    // their bucket instead.
    ((hv >> hash_shift) as usize, (tv >> 56) as u8)
}

/// Brick 39 arm state: 2-way pipelined probe. Runtime-settable so the
/// in-process ABBA harness can flip it between adjacent measurements.
/// GATE 8 @ L1 reachability + speculation ledger for `find_fast`'s pipelined
/// loop, the same deterministic instrument that decided Gate 8 at L3.
/// How much of `find_fast`'s NON-pipelined (pair-route) loop would a
/// speculation serve? `MM_MISS / MM_TOTAL` is the share of positions that reach
/// the miss-advance, i.e. where a speculated next-position load is CONSUMED.
/// Gate 7 audit: tag rejections that `fast_probe` would have ACCEPTED.
pub static TAG_FALSE_REJECT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static TAG_REJECT_TOTAL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(false_rejects, total_rejects)`.
pub fn take_tag_rejects() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        TAG_FALSE_REJECT.swap(0, Relaxed),
        TAG_REJECT_TOTAL.swap(0, Relaxed),
    )
}

/// GATE 2 candidate signal: rep match BYTES per rep PROBE.
pub static REP_PROBES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static REP_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

pub static REP_HITS_G: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static ALL_MATCH_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static ALL_SEQS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// `(rep_probes, rep_bytes, rep_hits, all_match_bytes, all_seqs)`
pub fn take_rep_rate() -> (u64, u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        REP_PROBES.swap(0, Relaxed),
        REP_BYTES.swap(0, Relaxed),
        REP_HITS_G.swap(0, Relaxed),
        ALL_MATCH_BYTES.swap(0, Relaxed),
        ALL_SEQS.swap(0, Relaxed),
    )
}

pub static MM_TOTAL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static MM_MISS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(main_loop_positions, positions_reaching_the_advance)`.
pub fn take_mm() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (MM_TOTAL.swap(0, Relaxed), MM_MISS.swap(0, Relaxed))
}

/// ffanat: dispatch-arm census. 0 = specialised (false,false,pipe),
/// 1 = tag arm (generic), 2 = rep arms (generic), 3 = rest.
#[cfg(feature = "profile")]
pub static FF_ARM: [crate::census64::AtomicU64; 4] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];

/// Read and clear the dispatch-arm census.
#[cfg(feature = "profile")]
pub fn take_ff_arms() -> [u64; 4] {
    let mut o = [0u64; 4];
    for i in 0..4 {
        o[i] = FF_ARM[i].swap(0, core::sync::atomic::Ordering::Relaxed);
    }
    o
}

pub static FF_PIPE_BLOCKS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static FF_SPEC_MADE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static FF_SPEC_USED: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(pipelined_blocks, speculations_made, speculations_used)`.
pub fn take_ff_pipe() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        FF_PIPE_BLOCKS.swap(0, Relaxed),
        FF_SPEC_MADE.swap(0, Relaxed),
        FF_SPEC_USED.swap(0, Relaxed),
    )
}

pub(crate) static PIPE_REP1_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// A/B the pipelined loop's `rep1` maintenance. OFF reproduces the pre-fix
/// "sticky repcode" behaviour, which was an accident but is not obviously worse.
pub fn set_pipe_rep1_arm(on: bool) {
    PIPE_REP1_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn pipe_rep1_enabled() -> bool {
    PIPE_REP1_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

pub(crate) static PIPE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Override the pipeline arm for the rest of the process. Bench hook.
pub fn set_pipe_arm(on: bool) {
    PIPE_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn pipe_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match PIPE_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = crate::env_knob_not0("RZSTD_MF_PIPE", true);
            PIPE_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Phase C batch arm: Huffman literal-emit bricks 16 / 29 / 32.
/// `RZSTD_HUFF_FAST=0` selects the scalar twin (the byte-identity oracle).
pub(crate) static HUFF_FAST_ENABLED_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA; shipping default from `RZSTD_HUFF_FAST`.
pub fn set_huff_fast_arm(on: bool) {
    HUFF_FAST_ENABLED_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn huff_fast_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match HUFF_FAST_ENABLED_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = crate::env_knob_not0("RZSTD_HUFF_FAST", true);
            HUFF_FAST_ENABLED_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Brick 44 arm: reserved block payload buffer.
pub(crate) static PAYLOAD_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook; shipping default comes from `RZSTD_PAYLOAD_RES`.
pub fn set_payload_arm(on: bool) {
    PAYLOAD_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn payload_reserve_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match PAYLOAD_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = crate::env_knob_not0("RZSTD_PAYLOAD_RES", true);
            PAYLOAD_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Brick 38 arm: reserved `seqs`/`lits` scratch + fixed-width literal push.
///
/// Runtime-settable so the in-process ABBA harness can re-adjudicate it. Its
/// original verdict (+5%, z=1.0) was taken with the cross-PROCESS method and
/// sits in the 3-7% band that drift demonstrably destroys.
pub(crate) static LITPUSH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook; shipping default comes from `RZSTD_LIT_PUSH`.
/// Brick 77 A/B arm: hoisted flag (A) vs per-call read (B).
///
/// Brick 77 replaced `lit_push_enabled()` in `push_literals`' guard -- executed
/// 15,687,334 times across the corpus -- with a value threaded from the caller.
/// The two paths must be alternated IN-PROCESS to be measurable: a cross-process
/// comparison of two builds put C's own throughput 14-17% apart and left
/// `cyc/byte` and `C/us` disagreeing, because both were measuring the box.
pub(crate) static LITPUSH_HOIST_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `true` = use the hoisted parameter, `false` = re-read per call.
pub fn set_litpush_hoist_arm(on: bool) {
    LITPUSH_HOIST_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn litpush_hoist_enabled() -> bool {
    LITPUSH_HOIST_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

pub fn set_litpush_arm(on: bool) {
    LITPUSH_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn lit_push_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match LITPUSH_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            // DEFAULT ON: re-adjudicated on the in-process ABBA instrument --
            // compress 6/6, z=+2.45 (sao +3.4%, dickens +4.1%, webster +3.1%,
            // nci +2.0%, ooffice +1.8%, mr +0.7%), decompress correctly null.
            // Its original +5%/z=1.0 was taken cross-process and could not be
            // resolved; the effect was real all along.
            let on = crate::env_knob_not0("RZSTD_LIT_PUSH", true);
            LITPUSH_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Width of the fixed-width literal push. Also the slack reserved past a
/// block's worth of literals so the fast path is always eligible.
pub(crate) const LIT_PUSH_WIDTH: usize = 16;
/// The widened arm. 32 is the last width that stays inlined as register moves;
/// 64 lowers to a `memcpy` call (138 instructions vs 9).
pub(crate) const LIT_PUSH_WIDTH_WIDE: usize = 32;
/// `(fast32 - fast16) / (slow - fast32)` = 2/38, from the emitted asm.
pub(crate) const WIDEN_RATIO: f32 = 0.0526;
/// Widest value any tier may copy. Reservations use THIS so the capacity guard
/// stays valid for every tier.
pub(crate) const LIT_PUSH_WIDTH_MAX: usize = 64;

/// GATE 13: the second and third TIERS.
///
/// The dispatch here is per-CALL, not per-block. A dispatched WIDTH has to
/// predict the next block's run-length distribution and then serves every run
/// with one constant; a tier reads `n` and picks among constants -- no signal,
/// no threshold, no warm-up, and no misprediction. Both tiers stay compile-time
/// constants, so both still lower to fixed-width moves.
///
/// Priced deterministically as `bytes stored + F x slow calls`, swept over F so
/// the answer's dependence on the one unknown is visible (totals, 18 corpora):
///
/// ```text
///            w16      w8      w32   tier16/32  tier16/32/64
///  L1 F=4   1.40M   1.40M    2.14M    1.24M       1.24M
///  L1 F=32  4.94M   8.49M    3.46M    2.56M       1.81M
///  L3 F=32  3.98M   7.08M    4.22M    2.32M       2.14M
/// ```
///
/// The tier wins at every realistic F and on 13 of 14 corpora individually, so
/// no single dispatched width can match it.
pub(crate) const LIT_PUSH_TIER2: usize = 32;
pub(crate) const LIT_PUSH_TIER3: usize = 64;

/// Bench arm for the tiers. 0 = all tiers (shipped), 1 = tier 1 only (the
/// pre-tier behaviour), 2 = tiers 1 and 2.
pub(crate) static LIT_PUSH_TIERS_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: 0 all tiers, 1 tier-1 only, 2 tiers 1+2.
pub fn set_lit_push_tiers_arm(t: u8) {
    LIT_PUSH_TIERS_ARM.store(t, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub(crate) fn lit_push_tiers() -> u8 {
    LIT_PUSH_TIERS_ARM.load(core::sync::atomic::Ordering::Relaxed)
}

/// GATE 13 @ L1: the copy width, as a measurement arm.
///
/// The width is a CONSTANT 16 today, chosen from L3's run-length histogram. At
/// L1 the distribution is not the same shape and, more importantly, is not the
/// same shape ACROSS CORPORA: `smallmsg-8m` puts 95.5% of runs in 5-8 while
/// `sao` puts 50.4% in 65+ and engages the fast path on only 6.1% of calls.
/// A constant cannot serve both.
/// GATE 13 @ L3. `push_literals` had exactly ONE call site -- `find_fast`'s
/// match commit -- so the gate was DEAD everywhere but L1, and dead by SCOPE
/// rather than by measurement: `find_dfast` called `lits.extend_from_slice`
/// directly and allocated both output vectors unreserved.
///
/// The gate is two things, and DFast had neither:
///   1. reserve `lits`/`seqs` up front, so neither grows by repeated realloc
///   2. a fixed-width 16-byte `copy_nonoverlapping` for short literal runs,
///      which the compiler CAN lower to a constant-width move where
///      `extend_from_slice`'s runtime length cannot be
///
/// L3 emits 1,973,548 sequences over the corpus at a mean of 3.75 literal bytes
/// each, and 17 of 18 corpora sit under the 16-byte width -- the same shape that
/// measured +2-4% at L1. Byte-identical by construction.
pub(crate) static DFAST_LITPUSH_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores DFast's unreserved vectors and runtime-length
/// literal copies.
pub fn set_dfast_litpush_arm(on: bool) {
    DFAST_LITPUSH_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn dfast_litpush_enabled() -> bool {
    DFAST_LITPUSH_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// GATE 13 @ L1 signal: share of this block's literal runs short enough for the
/// fixed-width copy to catch.
///
/// Read off the emitted sequences rather than counted in the probe loop -- the
/// litlens are already there, so the signal costs one pass per BLOCK and nothing
/// per position. An empty block reports 1.0 so the gate stays open.
#[inline]
pub(crate) fn lit_shares(seqs: &[Seq]) -> (f32, f32) {
    if seqs.is_empty() {
        return (1.0, 0.0);
    }
    let (mut short, mut mid) = (0usize, 0usize);
    for q in seqs {
        let l = q.litlen as usize;
        if l <= LIT_PUSH_WIDTH {
            short += 1;
        } else if l <= LIT_PUSH_WIDTH_WIDE {
            mid += 1;
        }
    }
    let n = seqs.len() as f32;
    // ONE division, not two: both shares divide by the same `n`.
    let inv = 1.0 / n;
    (short as f32 * inv, mid as f32 * inv)
}

/// GATE 13 WIDTH DISPATCH, derived from the emitted asm rather than fitted.
///
/// The fast path is SEVEN instructions at width 8 AND at width 16 (one `movq`,
/// one `movups`) -- so the byte-based model that preferred 8 by 33% was pricing
/// a quantity the machine does not charge for. Width 32 is NINE (two `movups`).
/// Width 64 is 138: LLVM stops inlining and emits a `memcpy` CALL, a cliff.
///
/// Widening 16 -> 32 therefore costs 2 instructions on every fast call and saves
/// `slow - fast32` on every run in (16, 32] it newly catches. With the measured
/// slow path at ~47 instructions that breaks even at
///
/// ```text
/// mid_share * (47 - 9)  >  short_share * (9 - 7)
/// mid_share             >  short_share * 0.0526
/// ```
///
/// which predicts every corpus in the set, including both marginal ones
/// (`mozilla` 4.2% vs 4.85% -> stay 16; `samba` 5.2% vs 4.91% -> widen).
#[inline]
pub(crate) fn lit_width_for(tables: &MatchTables) -> usize {
    if tables.blocks_done == 0 {
        return LIT_PUSH_WIDTH;
    }
    if tables.lit_mid_share > tables.lit_short_share * WIDEN_RATIO {
        LIT_PUSH_WIDTH_WIDE
    } else {
        LIT_PUSH_WIDTH
    }
}

/// GATE 13 @ L1 threshold: the share of literal runs the fixed-width copy must
/// CATCH for its guard to be worth evaluating.
///
/// Below it the four-condition guard runs and FAILS on nearly every call -- pure
/// overhead, since those runs go to `extend_from_slice` anyway. The population
/// separates with nothing in between (share of runs <= 16 bytes, L1):
///
///   sao 6.1%   x-ray 7.2%   |   mr 55.9%   dickens 79.5% ... smallmsg 100.0%
///
/// A 7.7x gap with no corpus inside it, so this is a single-sided latch on a wide
/// natural gap (great-gate.md par.4), not a fitted constant. 0.25 sits in the
/// middle of the empty band.
pub(crate) const LIT_SHORT_MIN: f32 = 0.25;

/// Bench hook for the Gate 13 dispatch. Negative disables the gate (constant ON,
/// the pre-dispatch behaviour and the byte-identical fallback).
pub(crate) static LIT_SHORT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the Gate 13 share threshold. Negative = gate off (always take the guard).
pub fn set_lit_short_arm(v: f32) {
    LIT_SHORT_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline]
pub(crate) fn lit_short_min() -> f32 {
    let b = LIT_SHORT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        LIT_SHORT_MIN
    } else {
        f32::from_bits(b)
    }
}

/// Deterministic instrument: guard evaluations that FAILED, i.e. wasted work.
pub static LP_GUARD_FAIL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Guard evaluations SKIPPED by the Gate 13 dispatch.
pub static LP_GUARD_SKIP: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the Gate 13 guard instruments.
pub fn take_lp_guard() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        LP_GUARD_FAIL.swap(0, Ordering::Relaxed),
        LP_GUARD_SKIP.swap(0, Ordering::Relaxed),
    )
}

const _: () = assert!(LIT_PUSH_WIDTH == 16 && LIT_PUSH_WIDTH_WIDE == 32);

/// The runtime-width arm of `push_literals`' fast path, outlined COLD: no
/// shipping caller passes a width other than 16 or 32, and keeping the
/// `memcpy` here rather than in the finders' loops is the whole point.
///
/// # Safety
/// `w` readable bytes at `sp`, `w` writable at `dp`, non-overlapping.
#[cold]
#[inline(never)]
#[allow(unsafe_code)]
pub(crate) unsafe fn lit_copy_runtime(sp: *const u8, dp: *mut u8, w: usize) {
    unsafe { core::ptr::copy_nonoverlapping(sp, dp, w) }
}

/// Append `src[from..to]` to the literal buffer.
///
/// The measured literal run between matches is tiny -- 1.9 bytes/sequence on
/// nci, 3.6 on xml, 7.8 on samba, 8.9 on webster -- across ~1-1.8M sequences
/// per file, and `extend_from_slice` is a **runtime-length** memcpy the
/// compiler cannot lower to a constant-width move. Same class as the decoder's
/// literal and match copies. Falls back to the checked path whenever the
/// fixed-width read or write would not fit.
#[allow(unsafe_code)]
#[inline]
/// BRICK 77: the arm is a PARAMETER, not a per-call read.
///
/// This called `lit_push_enabled()` inside its guard -- an env/OnceLock read
/// executed **15,687,334 times** across the corpus (measured: it is the hot
/// plumbing site in the match finder, ~2800x more often than any other).
/// `find_fast_impl` already computes the same value once per block as
/// `reserve`; it is frame-constant, so it is threaded in instead.
///
/// `arm` is now AUTHORITATIVE: the hoist escape hatch is resolved by the caller,
/// per block, so this function performs no atomic load at all.
///
/// GATE 13 @ L1: `find_fast`'s two REPCODE sites append here too. They were
/// raw `extend_from_slice` while the match commit next to them went through
/// this function -- 8.0% of L1 sequences corpus-wide, but 35.4% on nci, 35.1%
/// on sao and 33.0% on ooffice and versions.
///
/// Same disease as brick 49 (`use_rep`) and brick 64 (`seqcheck_hoisted`):
/// a fixed-for-the-block flag re-read in the hottest loop.
pub(crate) fn push_literals(lits: &mut Vec<u8>, src: &[u8], from: usize, to: usize, w: usize) {
    let n = to - from;
    // Counted HERE, at the top, not at the `extend_from_slice` below: the
    // tier-1 fast path returns early and serves ~96.7% of appends at L3, so
    // a tap further down measures the 3.3% remainder and reads as though
    // the encoder barely touches literals.
    crate::copies::add(crate::copies::C_LIT_PUSH, n);
    // REFUTED 2026-09-09 (brick 15): making `arm` a const generic for the three
    // chain finders (whose width is never 0) measured greedy +20 and lazy +6
    // static instructions for bt -3 -- folding the one test re-laid the armed
    // monomorphisation. The runtime test stays.
    let arm = w != 0;
    #[cfg(feature = "profile")]
    {
        let b = match n {
            0..=4 => 0,
            5..=8 => 1,
            9..=16 => 2,
            17..=32 => 3,
            33..=64 => 4,
            _ => 5,
        };
        LP_HIST[b].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    if n <= w && from + w <= src.len() && lits.capacity() - lits.len() >= w && arm {
        #[cfg(feature = "profile")]
        LP_FAST.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let len = lits.len();
        // SAFETY: `from + 16 <= src.len()` gives 16 readable source bytes;
        // `capacity - len >= 16` gives 16 writable destination bytes inside
        // the allocation. `src` (the input) and `lits` (a fresh scratch Vec)
        // are distinct buffers, so the regions cannot overlap. Exactly
        // `n <= 16` bytes are published by `set_len`.
        unsafe {
            let sp = src.as_ptr().add(from);
            let dp = lits.as_mut_ptr().add(len);
            // The whole point of this arm is a FIXED-width copy that inlines to
            // one or two vector moves. It was written that way when `w` was the
            // constant `LIT_PUSH_WIDTH`; `lit_width_for` then made `w` a per-block
            // choice between 16 and 32, and a `copy_nonoverlapping` whose length
            // is a runtime value is a `call memcpy` -- the census read one at
            // every push site in every finder, with the length coming off the
            // stack. Dispatching on the two legal widths restores the constant
            // (the branch is on a block constant, predicted); the runtime arm
            // remains for any other `w` so the function keeps its contract.
            //
            // And NOT as three `copy_nonoverlapping` arms with constant lengths:
            // LLVM's sink-common pass merged those back into ONE `memcpy` with
            // a phi'd length in `find_fast` (the census read 2 -> 2 there while
            // the other finders went 2 -> 0). Typed 16-byte loads and stores
            // have different SHAPES per arm and cannot be merged into a call;
            // the runtime arm is outlined cold so no `memcpy` stays in the loop.
            if w == LIT_PUSH_WIDTH_WIDE {
                let a = core::ptr::read_unaligned(sp.cast::<[u8; 16]>());
                let b = core::ptr::read_unaligned(sp.add(16).cast::<[u8; 16]>());
                core::ptr::write_unaligned(dp.cast::<[u8; 16]>(), a);
                core::ptr::write_unaligned(dp.add(16).cast::<[u8; 16]>(), b);
            } else if w == LIT_PUSH_WIDTH {
                let a = core::ptr::read_unaligned(sp.cast::<[u8; 16]>());
                core::ptr::write_unaligned(dp.cast::<[u8; 16]>(), a);
            } else {
                lit_copy_runtime(sp, dp, w);
            }
            lits.set_len(len + n);
        }
        return;
    }
    // The tiers live in an OUTLINED cold helper. Inlining them here pushed
    // `push_literals` past LLVM's inlining threshold and it stopped being
    // inlined AT ALL -- it became a standalone symbol with 19 call sites,
    // turning ~1M literal appends at L1 into real function calls. That is the
    // linkage trap: making an inlined function bigger can cost more than the
    // work it adds saves. Tier 1 stays small so it keeps its inlining.
    push_literals_tiers(lits, src, from, to, n, arm);
}

/// GATE 13 tiers 2 and 3, plus the fallback. Outlined and cold: reached only
/// when tier 1 missed -- 12.4% of appends at L1, 3.3% at L3 -- so the call
/// costs the common path nothing, while INLINING it cost the common path its
/// own inlining.
#[allow(unsafe_code)]
#[inline(never)]
#[cold]
pub(crate) fn push_literals_tiers(
    lits: &mut Vec<u8>,
    src: &[u8],
    from: usize,
    to: usize,
    n: usize,
    arm: bool,
) {
    // TIER 2 and TIER 3. Reached only when tier 1 missed, so the 87.6% of
    // appends tier 1 already serves pay nothing for these.
    let tiers = lit_push_tiers();
    if tiers != 1 && arm {
        if n <= LIT_PUSH_TIER2
            && from + LIT_PUSH_TIER2 <= src.len()
            && lits.capacity() - lits.len() >= LIT_PUSH_TIER2
        {
            #[cfg(feature = "profile")]
            LP_FAST2.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let len = lits.len();
            // SAFETY: identical to tier 1 at a wider constant -- `from + 32 <=
            // src.len()` gives 32 readable source bytes, `capacity - len >= 32`
            // gives 32 writable destination bytes inside the allocation, and
            // `src` and `lits` are distinct buffers. Exactly `n <= 32` bytes are
            // published by `set_len`.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src.as_ptr().add(from),
                    lits.as_mut_ptr().add(len),
                    LIT_PUSH_TIER2,
                );
                lits.set_len(len + n);
            }
            return;
        }
        if tiers == 0
            && n <= LIT_PUSH_TIER3
            && from + LIT_PUSH_TIER3 <= src.len()
            && lits.capacity() - lits.len() >= LIT_PUSH_TIER3
        {
            #[cfg(feature = "profile")]
            LP_FAST3.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            let len = lits.len();
            // SAFETY: as tier 2, at 64 bytes.
            unsafe {
                core::ptr::copy_nonoverlapping(
                    src.as_ptr().add(from),
                    lits.as_mut_ptr().add(len),
                    LIT_PUSH_TIER3,
                );
                lits.set_len(len + n);
            }
            return;
        }
    }
    #[cfg(feature = "profile")]
    {
        LP_SLOW.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        // The guard was EVALUATED and failed only if the arm let us reach it.
        if arm {
            LP_GUARD_FAIL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        } else {
            LP_GUARD_SKIP.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
    }
    lits.extend_from_slice(&src[from..to]);
}

/// Literal run-length histogram: 0-4, 5-8, 9-16, 17-32, 33-64, 65+.
pub static LP_HIST: [crate::census64::AtomicU64; 6] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];

/// Read and clear the literal run-length histogram.
pub fn take_lit_hist() -> [u64; 6] {
    use core::sync::atomic::Ordering::Relaxed;
    let mut o = [0u64; 6];
    for (i, v) in LP_HIST.iter().enumerate() {
        o[i] = v.swap(0, Relaxed);
    }
    o
}

/// Literal appends served by tier 2 (32 bytes) and tier 3 (64 bytes).
pub static LP_FAST2: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static LP_FAST3: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the tier-2 and tier-3 counts.
pub fn take_lit_tiers() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (LP_FAST2.swap(0, Relaxed), LP_FAST3.swap(0, Relaxed))
}

/// Literal appends served by the fixed-width copy, and by the fallback.
/// Read and clear the literal-push instruments: the six run-length buckets
/// (0-4, 5-8, 9-16, 17-32, 33-64, 65+) plus fast/slow path counts.
pub fn take_lp_stats() -> ([u64; 6], u64, u64) {
    use core::sync::atomic::Ordering;
    let mut h = [0u64; 6];
    for (i, c) in LP_HIST.iter().enumerate() {
        h[i] = c.swap(0, Ordering::Relaxed);
    }
    (
        h,
        LP_FAST.swap(0, Ordering::Relaxed),
        LP_SLOW.swap(0, Ordering::Relaxed),
    )
}

pub static LP_FAST: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static LP_SLOW: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(fixed_width, fallback)` literal-append counts.
pub fn take_lit_push() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (LP_FAST.swap(0, Relaxed), LP_SLOW.swap(0, Relaxed))
}
/// ffanat: the Fast loop's slot primitives, operating on LOCALS taken out of
/// `MatchTables` so the table base pointer lives in a REGISTER for the whole
/// loop. The asm receipt that motivated this: the specialised copy reloaded the
/// hash base from the stack (`movq 96(%rbp), ..`) THREE times per iteration --
/// before the probe load, the store, and the speculation load -- while `src`
/// sat in a register, because brick 48's fix was never given to the table
/// itself. `fast_slot_store` is the ONE write-rule site (190ad8b) shared by the
/// loop and `fill_fast_after_match`; the bodies mirror
/// `store_fast`/`load_fast`/`raw_fast` exactly, receipt counters included.
#[inline(always)]
#[allow(unsafe_code)]
pub(crate) fn fast_slot_store(
    hash: &mut [u32],
    tags: &mut [u8],
    pack: bool,
    // W10: hoisted `!tags.is_empty()` -- see `fast_slot_swap`.
    tags_live: bool,
    h: usize,
    pos: usize,
    tag: u8,
) {
    debug_assert_eq!(tags_live, !tags.is_empty());
    debug_assert!(h < hash.len());
    if pack {
        *unsafe { hash.get_unchecked_mut(h) } =
            (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
        return;
    }
    // The array route's `tags` is allocated at EXACTLY `hash.len()`, and `h`
    // has already indexed `hash` above -- the bounds test and its branch were
    // dead on every unpacked store.
    if tags_live {
        debug_assert!(tags.len() == hash.len());
        *unsafe { tags.get_unchecked_mut(h) } = tag;
    }
    *unsafe { hash.get_unchecked_mut(h) } = (pos as u32).wrapping_add(1);
}

/// W7: LOAD THEN STORE OF THE SAME SLOT, fused. The main loop reads a slot
/// and immediately overwrites it with the current position, and each half
/// tested `pack` for itself -- the asm shows the flag spilled and re-tested
/// TWICE per position. One branch now serves both, and the packed arm builds
/// its stored word from the same registers it just decoded.
#[inline(always)]
#[allow(unsafe_code)]
pub(crate) fn fast_slot_swap(
    packed: bool,
    hash: &mut [u32],
    tags: &mut [u8],
    pack: bool,
    tags_live: bool,
    h: usize,
    pos: usize,
    tag: u8,
) -> u32 {
    debug_assert_eq!(tags_live, !tags.is_empty());
    debug_assert!(h < hash.len());
    // SAFETY: `h` is masked by `hash.len() - 1` at every caller (brick 50).
    let slot = unsafe { hash.get_unchecked_mut(h) };
    let e = *slot;
    if pack {
        *slot = (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
        if e == 0 {
            return 0;
        }
        #[cfg(feature = "profile")]
        PACKED_TAG_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if packed && (e >> 24) as u8 != tag {
            return 0;
        }
        return e & 0x00FF_FFFF;
    }
    *slot = (pos as u32).wrapping_add(1);
    // W8: `tags_live` is the caller's hoisted `!tags.is_empty()` -- a
    // per-BLOCK fact that was a length load and a test on every slot touch.
    if tags_live {
        debug_assert!(!tags.is_empty() && tags.len() == hash.len());
        let t = unsafe { *tags.get_unchecked(h) };
        // SAFETY-neutral: the array route writes the tag unconditionally
        // whenever the array exists (the 190ad8b rule).
        unsafe { *tags.get_unchecked_mut(h) = tag };
        if e == 0 {
            return 0;
        }
        if packed {
            #[cfg(feature = "profile")]
            TAGARR_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if t != tag {
                return 0;
            }
        }
        return e;
    }
    if e == 0 {
        0
    } else {
        e
    }
}

#[inline(always)]
#[allow(unsafe_code)]
pub(crate) fn fast_slot_load(
    packed: bool,
    hash: &[u32],
    tags: &[u8],
    pack: bool,
    // W9: the caller's hoisted `!tags.is_empty()` -- see `fast_slot_swap`.
    // This load is the pipelined loop's FORWARD probe, so the length test it
    // replaces ran once per position on that path.
    tags_live: bool,
    h: usize,
    tag: u8,
) -> u32 {
    debug_assert_eq!(tags_live, !tags.is_empty());
    debug_assert!(h < hash.len());
    let e = *unsafe { hash.get_unchecked(h) };
    if e == 0 {
        return 0;
    }
    if pack {
        #[cfg(feature = "profile")]
        PACKED_TAG_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if packed && (e >> 24) as u8 != tag {
            return 0;
        }
        return e & 0x00FF_FFFF;
    }
    if !packed {
        return e;
    }
    // Same provable bound as the store's -- per PROBE, on the hottest loop
    // in the encoder.
    if tags_live {
        debug_assert!(tags.len() == hash.len());
        #[allow(unsafe_code)]
        let t = *unsafe { tags.get_unchecked(h) };
        #[cfg(feature = "profile")]
        TAGARR_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if t != tag {
            return 0;
        }
    }
    e
}

/// Near bar for the wide hash on rep-dominated blocks. The segment experiment
/// proved wide == legacy LOCALLY on versions (+0.34% over independent 512K
/// chunks, swings both ways); the whole-file loss is cross-block STATE: `reps`
/// learn from emitted offsets, and the wide key's exact-gram matches point at
/// the PREVIOUS VERSION -- huge offsets that poison the rep triplet for the
/// stride content between. The legacy key's collision survivors were NEAR
/// matches (stride-family offsets) feeding the reps the right flavor, by
/// accident. This makes the accident policy: on rep-dominated blocks, consume
/// a hash match only when it is near enough to keep the rep state coherent.
#[allow(dead_code)] // the recorded bar for a refuted arm; kept as the record.
pub(crate) const FF_NEAR_MAX: usize = 1 << 16;

/// Length bar (the surviving design): profile builds may override via
/// RZSTD_FF_ML for the sweep.
pub(crate) fn ff_anchor_ml() -> usize {
    #[cfg(feature = "profile")]
    {
        if let Some(n) = crate::env_knob_parse("RZSTD_FF_ML") {
            return n;
        }
    }
    16
}

/// Latch a wide-keyed frame to the legacy 4-byte key: RE-SEED the heads over
/// the lookback window (clearing was proven byte-identical to doing nothing --
/// lazy treats wide-keyed and empty alike; the legacy arm's advantage is REAL
/// inherited heads), then stay legacy for the frame. Called from both triggers:
/// the rep_yield signal and the fast_lazy switch.
#[inline(always)]
pub(crate) fn fast_hash_relatch(
    tables: &mut MatchTables,
    src: &[u8],
    block_start: usize,
    window: usize,
) {
    let shift = 32u32.saturating_sub(tables.hash_log);
    let from = block_start.saturating_sub(window).max(tables.frame_start);
    let to = block_start.saturating_sub(8);
    let mut p = from;
    while p <= to && p + 8 <= src.len() {
        let h = (load_u32le(src, p).wrapping_mul(HASH4_PRIME) >> shift) as usize;
        tables.put_h(h, p);
        p += 1;
    }
    tables.pack_tags = false;
    tables.fast_hash_legacy = true;
    #[cfg(feature = "profile")]
    FF_LATCH.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

/// Diagnostic twin of `raw_fast` for the local-table loop (COUNT paths only).
#[inline(always)]
pub(crate) fn fast_slot_raw(hash: &[u32], pack: bool, h: usize) -> u32 {
    let e = hash[h];
    if pack {
        e & 0x00FF_FFFF
    } else {
        e
    }
}

/// W1: the Fast ladder's match emitter carried NINETEEN arguments, eleven of
/// them fixed for the whole block, and it was `#[inline(always)]` -- so the
/// whole emit path was stamped into all 140 `find_fast_impl` monomorphisations
/// AND their 140 BMI2 twins. 280 copies of one per-MATCH routine, in the
/// function that is already 73% of the library.
///
/// Same shape `BtCtx` and `ChainCtx` use: the per-block constants ride in a
/// context built once, and the emitter outlines to FOUR copies (plain/bmi2 x
/// packed) instead of 280. The ISA twin is mandatory, not optional -- an
/// outlined callee of a `#[target_feature]` twin compiles BASELINE (the
/// shim-trap rule), so outlining without a twin would silently downgrade the
/// end-fill's hashes from `shrx` back to `shr %cl`.
pub(crate) struct FastEmitCtx<'a> {
    pub(crate) src: &'a [u8],
    pub(crate) pack: bool,
    pub(crate) f_wide: bool,
    pub(crate) f_mask: u64,
    pub(crate) f_shift: u32,
    pub(crate) ilimit: usize,
    pub(crate) frame_start: usize,
    pub(crate) w: usize,
    pub(crate) tags_live: bool,
    pub(crate) ends: (bool, bool),
}
