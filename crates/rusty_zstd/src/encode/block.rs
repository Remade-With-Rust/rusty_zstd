//! Block encoding: sequences and literals into a compressed block.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a
//! moved item that was private is now `pub(crate)`, so the parent can
//! still name it. That changes who may reference a symbol, not what is
//! emitted for it -- checked, not assumed: the asm board was identical
//! in all thirty-two columns across the move, which matters because
//! this crate builds at the default `codegen-units = 16`, where rustc
//! partitions codegen units BY MODULE and layout can move inlining.

use super::*;
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_block(
    out: &mut Vec<u8>,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    last: bool,
    ldm: Option<&mut crate::ldm::LdmTables>,
    ldm_p: crate::ldm::LdmParams,
) -> Result<(), Error> {
    // TWIN RETIRED. This carried a wholesale BMI2 twin justified by "the
    // per-block section packing carried 34 variable shifts of its own,
    // outside every finer-grained twin". That premise has expired: the
    // packing moved into `write_sequences` and `write_literals`, which grew
    // their OWN twins, and every finder now runs its own `has_bmi2()`
    // dispatch. Measured on the emitted asm before removing it -- the twin
    // contained 1 `shrx` against 1291 instructions of duplicated
    // body. It was buying single-digit shift encodings for four figures of
    // I-cache.
    encode_block_inner(
        out,
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        reps,
        entropy,
        last,
        ldm,
        ldm_p,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
pub(crate) fn encode_block_inner(
    out: &mut Vec<u8>,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    last: bool,
    ldm: Option<&mut crate::ldm::LdmTables>,
    ldm_p: crate::ldm::LdmParams,
) -> Result<(), Error> {
    let block = &src[block_start..block_end];
    if block.is_empty() {
        crate::prof::note_raw_block();
        write_block_header(out, last, BlockType::Raw, 0);
        return Ok(());
    }
    if let Some(b) = rle_byte(block) {
        crate::prof::note_rle_block();
        // P1/gg-matchfind: RLE blocks returned BEFORE `tap_block`, so
        // `zeros-32m` contributed 0 rows to the harvest and the corpus count was
        // 17, not 18. A degenerate class that emits no rows cannot be shown to
        // be unharmed by a gate -- which is exactly what the finish line
        // ("worst of the 18 <= 0") requires. One tap, 1 byte emitted.
        tap_block(
            block.len(),
            0,
            block.len(),
            0,
            1000,
            params.strategy,
            false,
            1,
            tables.rep_yield,
            0,
            0,
        );
        write_block_header(out, last, BlockType::Rle, block.len() as u32);
        out.push(b);
        return Ok(());
    }

    // GATE 16 SCOPE GAP -- incompressible content pays the FULL match search
    // before anything discovers it is incompressible.
    //
    // At L22, incomp-32m issues 2,097,040 binary-tree searches, 100.0% of which
    // return nothing, and then the block is emitted RAW anyway -- 4,177 bt calls
    // per emitted sequence. `early_raw_skip` cannot help: it is gated to
    // `Strategy::Fast` with `--fast=N`, so it never fires on the Bt ladder.
    //
    // The outcome of the PREVIOUS block is the signal, and it costs nothing to
    // read. After `RAW_RUN_MIN` consecutive raw blocks, skip the search and emit
    // the block as literals -- which is what it was going to become. Re-probed on
    // a schedule so content that starts compressing is picked up: without that
    // the gate would suppress its own evidence, the defect this campaign has now
    // hit in Gates 6, 2 and 10.
    let skip_search = raw_skip_on() && tables.raw_run >= raw_run_min() && tables.raw_probe != 0;
    // LDM is excluded: the probe would have to clone and then discard the LDM
    // state too, and a second pollution problem is not worth solving for a
    // feature that is off by default on this path.
    let probing = params.strategy == Strategy::Fast
        && params.target_length == 0
        && tables.pair_route == 1
        && ldm.is_none()
        && step_probe_on()
        && (tables.step_pick == 0 || tables.step_reprobe == 0);
    let (seqs, literals) = if skip_search {
        (Vec::new(), block.to_vec())
    } else if probing {
        // GATE 18 @ L1 DISPATCH. Measure what step 2 would forfeit, from an
        // IDENTICAL starting state, then keep step 1's output so a probe block
        // is never worse than the pinned behaviour.
        //
        // Two earlier designs failed and are recorded so they are not retried:
        // alternating the steps across blocks compares CONTENT (adjacent blocks
        // differ in compressibility, and it latched mozilla and samba onto step
        // 2 at +2.3% and +3.2%); counting match bytes at skipped positions
        // overestimates, because a match at a skipped position usually SHIFTS to
        // the next one rather than vanishing.
        // `Clone` for `MatchTables` is PROBE-SCOPED: it hands over only the
        // boards the Fast family reads (see the impl). This assert is the
        // contract's tripwire.
        debug_assert_eq!(params.strategy, Strategy::Fast);
        let mut probe = tables.clone();
        probe.route_force = 2;
        let (s2, l2) = find_sequences(
            src,
            block_start,
            block_end,
            window,
            params,
            &mut probe,
            None,
            ldm_p,
            *reps,
        );
        let _m = crate::prof::scope(crate::prof::Stage::EncodeMatchFind);
        let r = find_sequences(
            src,
            block_start,
            block_end,
            window,
            params,
            tables,
            ldm,
            ldm_p,
            *reps,
        );
        note_step_probe(tables, &r.0, r.1.len(), &s2, l2.len());
        r
    } else {
        let _m = crate::prof::scope(crate::prof::Stage::EncodeMatchFind);
        find_sequences(
            src,
            block_start,
            block_end,
            window,
            params,
            tables,
            ldm,
            ldm_p,
            *reps,
        )
    };
    // P1/gg-matchfind candidate signal, computed once for every tap below.
    let (off_coll, off_bkt) = if cfg!(feature = "profile") {
        offset_stats(&seqs)
    } else {
        (0, 0)
    };
    if seqs.is_empty() && !huffman::literals_worth_huffman(block) {
        #[cfg(feature = "profile")]
        RAW_EXIT[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        note_raw_outcome(tables, true);
        crate::prof::note_raw_block();
        tap_block(
            block.len(),
            0,
            0,
            block.len(),
            huffman::lit_sample_peak(block),
            params.strategy,
            false,
            block.len(),
            tables.rep_yield,
            off_coll,
            off_bkt,
        );
        write_block_header(out, last, BlockType::Raw, block.len() as u32);
        crate::copies::add(crate::copies::C_RAW_BLOCK_TO_DST, block.len());
        out.extend_from_slice(block);
        // GATE 6 @ L1 -- hand the finder's buffers back to the frame.
        if finder_scratch_enabled() {
            tables.seq_scratch = seqs;
            tables.lit_scratch = literals;
        }
        return Ok(());
    }
    let match_b: usize = seqs.iter().map(|s| s.matchlen as usize).sum();
    let lit_b = literals.len();
    let mg = min_gain(block.len(), params.strategy);
    let peak = huffman::lit_sample_peak(if seqs.is_empty() { block } else { &literals });
    if early_raw_skip(match_b, block.len(), params) {
        #[cfg(feature = "profile")]
        RAW_EXIT[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        note_raw_outcome(tables, true);
        crate::prof::note_raw_block();
        crate::prof::note_early_raw();
        tap_block(
            block.len(),
            seqs.len(),
            match_b,
            lit_b,
            peak,
            params.strategy,
            true,
            block.len(),
            tables.rep_yield,
            off_coll,
            off_bkt,
        );
        write_block_header(out, last, BlockType::Raw, block.len() as u32);
        crate::copies::add(crate::copies::C_RAW_BLOCK_TO_DST, block.len());
        out.extend_from_slice(block);
        // GATE 6 @ L1 -- hand the finder's buffers back to the frame.
        if finder_scratch_enabled() {
            tables.seq_scratch = seqs;
            tables.lit_scratch = literals;
        }
        return Ok(());
    }
    let saved_reps = *reps;
    #[cfg(feature = "profile")]
    ENT_SAVE[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let saved_ent = entropy.clone();
    crate::prof::note_scratch(1);
    // GATE 6 @ L3 -- reuse the payload buffer across blocks.
    //
    // `payload` is only ever written, measured, and copied out; it is never
    // moved or handed to a caller, so there is no reason to build a new one per
    // block. Taking the frame's scratch buffer and putting it back on the way
    // out makes the reserve a once-per-frame cost instead of a per-block one,
    // which is what removes BOTH failure modes the allocator counter found (see
    // `payload_scratch`). `block.len()` is still a hard upper bound -- a payload
    // that reaches it is rejected for Raw by `raw_limit` below -- so the first
    // block sizes the buffer correctly and no later block has to grow it.
    // The payload is emitted STRAIGHT INTO `out`, after a three-byte hole for
    // the header. `payload_len` below replaces what used to be `payload.len()`.
    // If raw wins, `out` is truncated back to `mark` and nothing was lost but
    // the writes; if compressed wins, the header is patched in place and the
    // whole staging copy is gone.
    let mark = out.len();
    out.extend_from_slice(&[0u8; 3]);
    let body_at = out.len();
    if payload_reserve_enabled() {
        out.reserve(block.len());
    }
    {
        let _e = crate::prof::scope(crate::prof::Stage::EncodeEntropy);
        if seqs.is_empty() {
            let _ = write_literals(&mut *out, block, entropy)?;
            crate::prof::note_emit_lit((out.len() - body_at) as u64);
            out.push(0);
        } else {
            let lit_reused = write_literals(&mut *out, &literals, entropy)?;
            let lit_end = out.len() - body_at;
            // GATE 19 -- feed the DP its literal price MEASURED, not guessed.
            //
            // `find_opt` priced a literal at a flat 6 bits. Real literals cost
            // ~8 raw and ~4-7 after Huffman, so 6 UNDER-prices them on
            // high-entropy content: the DP then prefers literals to matches and
            // the "optimal" parse LOSES to plain lazy -- x-ray +2.94% against
            // L15, and every BtOpt/BtUltra level worse than L14 (L16 +38,564).
            //
            // This is the real cost of the literals this encoder just emitted,
            // so the next block prices them at what they actually cost rather
            // than at a constant that can only suit one content class.
            let _ = lit_reused;
            if !literals.is_empty() {
                tables.opt_lit_price = measured_lit_bits(lit_end, literals.len());
            }
            crate::prof::note_emit_lit(lit_end as u64);
            write_sequences(&mut *out, &seqs, reps, entropy, params.strategy, tables)?;
            crate::prof::note_emit_seq(((out.len() - body_at) - lit_end) as u64);
        }
    }
    let payload_len = out.len() - body_at;
    let raw_limit = if incomp_skip_on(params) {
        block.len().saturating_sub(mg)
    } else {
        block.len()
    };
    // GATE 16 study: the gate's signal is BINARY ("was the last block raw?").
    // A continuous one is right here -- how badly the block missed. A block that
    // barely missed may compress next time; one that missed by a mile will not.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        let ratio = (payload_len as f64 / raw_limit.max(1) as f64 * 1000.0) as u64;
        if payload_len >= raw_limit {
            RAW_MARGIN_SUM.fetch_add(ratio.min(4000), Relaxed);
            RAW_MARGIN_N.fetch_add(1, Relaxed);
            // bucket: 1000-1010, 1010-1050, 1050-1200, 1200+
            let b = match ratio {
                0..=1010 => 0,
                1011..=1050 => 1,
                1051..=1200 => 2,
                _ => 3,
            };
            RAW_MARGIN_HIST[b].fetch_add(1, Relaxed);
        }
    }
    if payload_len >= raw_limit {
        // Rewind the speculative payload, header hole and all.
        out.truncate(mark);
        #[cfg(feature = "profile")]
        RAW_EXIT[2].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        *reps = saved_reps;
        #[cfg(feature = "profile")]
        ENT_SAVE[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        *entropy = saved_ent;
        note_raw_outcome(tables, true);
        crate::prof::note_raw_block();
        tap_block(
            block.len(),
            seqs.len(),
            match_b,
            lit_b,
            peak,
            params.strategy,
            false,
            block.len(),
            tables.rep_yield,
            off_coll,
            off_bkt,
        );
        write_block_header(out, last, BlockType::Raw, block.len() as u32);
        crate::copies::add(crate::copies::C_RAW_BLOCK_TO_DST, block.len());
        out.extend_from_slice(block);
        // GATE 6 @ L1 -- hand the finder's buffers back to the frame.
        if finder_scratch_enabled() {
            tables.seq_scratch = seqs;
            tables.lit_scratch = literals;
        }
        return Ok(());
    }
    crate::prof::note_comp_block();
    tap_block(
        block.len(),
        seqs.len(),
        match_b,
        lit_b,
        peak,
        params.strategy,
        false,
        payload_len,
        tables.rep_yield,
        off_coll,
        off_bkt,
    );
    note_raw_outcome(tables, false);
    note_step_outcome(tables, payload_len, block.len());
    patch_block_header(out, mark, last, BlockType::Compressed, payload_len as u32);
    if finder_scratch_enabled() {
        tables.seq_scratch = seqs;
        tables.lit_scratch = literals;
    }
    Ok(())
}

/// Streaming block: `src` is history || current block; sequences only from `block_start`.
#[allow(clippy::too_many_arguments)]
/// `block_end` is explicit because the streaming compressor appends caller
/// input straight into its history buffer, so `src` can hold PENDING bytes
/// past this block that must not be coded into it. This was `src.len()`,
/// which is still what the one-shot path passes.
pub(crate) fn encode_block_from_scratch(
    out: &mut Vec<u8>,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    last: bool,
) -> Result<(), Error> {
    let window = 1usize << params.window_log.min(31);
    encode_block(
        out,
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        reps,
        entropy,
        last,
        None,
        crate::ldm::LdmParams::default(),
    )
}

pub(crate) fn rle_byte(block: &[u8]) -> Option<u8> {
    let first = *block.first()?;
    if block.len() < 2 {
        return None;
    }
    let splat = u64::from(first) * 0x0101_0101_0101_0101;
    let mut i = 0usize;
    while i + 8 <= block.len() {
        if load_u64le(block, i) != splat {
            return None;
        }
        i += 8;
    }
    while i < block.len() {
        if block[i] != first {
            return None;
        }
        i += 1;
    }
    Some(first)
}

/// libzstd `ZSTD_minGain`: `(srcSize >> minlog) + 2`, minlog=6 except btultra+.
pub(crate) fn min_gain(src_size: usize, strategy: Strategy) -> usize {
    let minlog = if strategy.id() >= 8 {
        u32::from(strategy.id()) - 1
    } else {
        6
    };
    (src_size >> minlog) + 2
}

#[cfg(test)]
thread_local! {
    pub(crate) static SKIP_OVERRIDE: core::cell::Cell<Option<bool>> =
        const { core::cell::Cell::new(None) };
}

/// Gate 16 arm: the incompressible early-raw skip. Also RETIRES the uncached
/// `std::env::var` read that `incomp_skip_on` performed on EVERY BLOCK -- the
/// last uncached env read on a hot path (m7-anatomy section 3 addendum).
/// 0 = unresolved, 1 = off, 2 = on, 3 = follow the level rule.
pub(crate) static INCOMP_SKIP_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the Gate 16 truth table. `None` restores the level rule.
pub fn set_incomp_skip_arm(on: Option<bool>) {
    let v = match on {
        Some(false) => 1,
        Some(true) => 2,
        None => 3,
    };
    INCOMP_SKIP_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Consecutive raw blocks before the match search is short-circuited.
/// GATE 16 @ L3: the OFF arm the raw short circuit never had.
///
/// 4.30 shipped `skip_search` as an unconditional constant. Every other shipped
/// constant in this campaign carries a proven byte-identical OFF; this one did
/// not, so it could not be A/B'd at all -- and the arm that LOOKS like its
/// switch (`set_incomp_skip_arm`) actually gates a different mechanism, the
/// `raw_limit` tightening. Measuring the wrong one is exactly the mistake that
/// produced a "zero positions saved" reading for a gate that saves ENTROPY work.
pub(crate) static RAW_SKIP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` always searches, restoring the pre-4.30 behaviour.
pub fn set_raw_skip_arm(on: bool) {
    RAW_SKIP_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub(crate) fn raw_skip_on() -> bool {
    RAW_SKIP_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// GATE 16 @ L3: the two constants the short circuit runs on, never swept.
///
/// `RAW_RUN_MIN` is how many consecutive raw blocks it takes before the search
/// is skipped; `RAW_PROBE_PERIOD` is how often it re-probes so content that
/// starts compressing is picked up. Both were chosen when 4.30 shipped and
/// neither has been moved since -- the same shape as the search-strength shift
/// of 4.43, which sat unexamined and turned out to be the biggest L1 lever.
pub(crate) static RAW_RUN_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
pub(crate) static RAW_PROBE_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Bench hook: consecutive raw blocks before the search is skipped. 0 = shipped 2.
pub fn set_raw_run_min_arm(v: u32) {
    RAW_RUN_MIN_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Bench hook: blocks between forced re-probes. 0 = shipped 16.
pub fn set_raw_probe_arm(v: u32) {
    RAW_PROBE_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
pub(crate) fn raw_run_min() -> u32 {
    let v = RAW_RUN_MIN_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        RAW_RUN_MIN
    } else {
        v
    }
}

#[inline(always)]
pub(crate) fn raw_probe_period() -> u32 {
    let v = RAW_PROBE_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        RAW_PROBE_PERIOD
    } else {
        v
    }
}

/// GATE 16 study: WHICH of the three raw exits does each block take?
/// 0 = no sequences and literals not worth huffman (before any payload exists)
/// 1 = `early_raw_skip` (needs Fast + tlen 1..7)
/// 2 = payload did not beat `raw_limit`
#[cfg(feature = "profile")]
pub static RAW_EXIT: [crate::census64::AtomicU64; 3] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];

/// Read and clear the three raw-exit counts.
#[cfg(feature = "profile")]
pub fn take_raw_exits() -> [u64; 3] {
    use core::sync::atomic::Ordering::Relaxed;
    let mut o = [0u64; 3];
    for (i, v) in RAW_EXIT.iter().enumerate() {
        o[i] = v.swap(0, Relaxed);
    }
    o
}

/// GATE 16 study: how far past `raw_limit` did blocks that went RAW land?
#[cfg(feature = "profile")]
pub static RAW_MARGIN_SUM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static RAW_MARGIN_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static RAW_MARGIN_HIST: [crate::census64::AtomicU64; 4] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];

/// Read and clear `(sum_permille, n, [<=1.01, <=1.05, <=1.20, >1.20])`.
#[cfg(feature = "profile")]
pub fn take_raw_margin() -> (u64, u64, [u64; 4]) {
    use core::sync::atomic::Ordering::Relaxed;
    let mut h = [0u64; 4];
    for (i, v) in RAW_MARGIN_HIST.iter().enumerate() {
        h[i] = v.swap(0, Relaxed);
    }
    (
        RAW_MARGIN_SUM.swap(0, Relaxed),
        RAW_MARGIN_N.swap(0, Relaxed),
        h,
    )
}

pub(crate) const RAW_RUN_MIN: u32 = 2;
/// Blocks between forced re-probes of the raw short circuit.
pub(crate) const RAW_PROBE_PERIOD: u32 = 16;

/// Record whether this block ended up RAW, and tick the re-probe countdown.
/// Called on every exit so the run length is never stale.
pub(crate) fn note_raw_outcome(tables: &mut MatchTables, raw: bool) {
    if raw {
        tables.raw_run = tables.raw_run.saturating_add(1);
    } else {
        tables.raw_run = 0;
    }
    tables.raw_probe = if tables.raw_probe == 0 {
        raw_probe_period()
    } else {
        tables.raw_probe - 1
    };
}

/// The three-state env resolve of `incomp_skip_on`, cold and outlined -- the
/// same split every other knob got in brick 3, hand-written because this one
/// accepts `off`/`on` as well as `0`/`1`.
#[cfg(feature = "std")]
#[cold]
#[inline(never)]
pub(crate) fn incomp_skip_resolve() -> u8 {
    match crate::env_knob("RZSTD_INCOMP_SKIP") {
        Ok(x) if x.trim() == "0" || x.trim().eq_ignore_ascii_case("off") => 1,
        Ok(x) if x.trim() == "1" || x.trim().eq_ignore_ascii_case("on") => 2,
        _ => 3,
    }
}

pub(crate) fn incomp_skip_on(params: CompressionParameters) -> bool {
    #[cfg(test)]
    {
        if let Some(v) = SKIP_OVERRIDE.with(|c| c.get()) {
            return v;
        }
    }
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let mut v = INCOMP_SKIP_ARM.load(Ordering::Relaxed);
        if v == 0 {
            // Resolve ONCE, not once per block. This read used to be a raw
            // `std::env::var` inside `early_raw_skip`, i.e. an allocation and a
            // process-environment lookup on every block -- the same shape as
            // bricks 49/64/77.
            v = incomp_skip_resolve();
            INCOMP_SKIP_ARM.store(v, Ordering::Relaxed);
        }
        if v == 1 {
            return false;
        }
        if v == 2 {
            return true;
        }
    }
    // --fast=N (N=1..=7) is strategy Fast with targetLength = N.
    // Level 1 Fast has tlen 0. Huge tlen on Fast is a match-finder skip, not --fast.
    // Greedy+ tlen is a search knob, not a skip trigger.
    params.strategy == Strategy::Fast && params.target_length >= 1 && params.target_length <= 7
}

/// Depth-1 skip tree (Great Gate Z1 / brick 15): Fast AND
/// `1 <= target_length <= 7` AND `match_bytes < minGain` -> Raw, skip entropy.
pub(crate) fn early_raw_skip(
    match_bytes: usize,
    block_len: usize,
    params: CompressionParameters,
) -> bool {
    if !incomp_skip_on(params) {
        return false;
    }
    match_bytes < min_gain(block_len, params.strategy)
}

#[allow(clippy::too_many_arguments)]
/// P1/gg-matchfind candidate signal: how CONCENTRATED this block's match offsets
/// are, bucketed by `log2(offset)` into 32 bins.
///
/// Returns `(collision_probability * 1000, distinct_buckets)`. Collision
/// probability is `sum(p^2)` -- the Renyi-2 form already used by
/// `literals_worth_huffman` -- so it needs no logarithm and is monotone in
/// concentration: 1000 = every match at one offset scale, ~31 = perfectly spread
/// across all 32.
///
/// Physical premise being tested: record-structured content (fixed-width rows,
/// log lines) matches at a near-constant offset that the REPCODE path already
/// captures for free, so extra probe density re-discovers matches it already
/// had. If that is true, this separates such content from genuinely matchy text.
pub(crate) fn offset_stats(seqs: &[Seq]) -> (u32, u8) {
    if seqs.is_empty() {
        return (0, 0);
    }
    let mut bins = [0u32; 32];
    for s in seqs {
        let b = (31 - s.offset.max(1).leading_zeros()) as usize;
        bins[b.min(31)] += 1;
    }
    let n = seqs.len() as u64;
    let sum_sq: u64 = bins.iter().map(|&c| u64::from(c) * u64::from(c)).sum();
    let used = bins.iter().filter(|&&c| c != 0).count() as u8;
    (((sum_sq * 1000) / (n * n)) as u32, used)
}

pub(crate) fn tap_block(
    block_len: usize,
    nseq: usize,
    match_bytes: usize,
    lit_bytes: usize,
    lit_peak: u32,
    strategy: Strategy,
    early_raw: bool,
    csize: usize,
    rep_yield: f32,
    off_collision_x1000: u32,
    off_buckets: u8,
) {
    // Cumulative at block exit; the harvest differences consecutive rows.
    let c = crate::prof::encode_counts();
    crate::prof::note_block_tap(crate::prof::BlockTap {
        block_len: block_len as u32,
        nseq: nseq as u32,
        match_bytes: match_bytes as u32,
        lit_bytes: lit_bytes as u32,
        min_gain: min_gain(block_len, strategy) as u32,
        lit_peak,
        early_raw: u8::from(early_raw),
        csize: csize as u32,
        probes: c.hash_probes,
        hits: c.probe_hits,
        rep_yield_x1000: (rep_yield * 1000.0) as u32,
        off_collision_x1000,
        off_buckets,
        mf_ns: crate::prof::stage_ns(crate::prof::Stage::EncodeMatchFind),
    });
}

/// Overwrite a 3-byte block header already reserved at `at`.
///
/// The block header is FIXED at three bytes, which is what lets the payload be
/// built straight into the frame buffer: reserve the three bytes, emit the
/// payload after them, then come back and fill them in. The alternative -- and
/// what this replaced -- was to build the payload into a scratch `Vec` purely
/// to learn its length, then copy the whole thing into the frame. That copy was
/// 42.2 MB per 208 MB encoded, the largest reducible copy in the encoder.
pub(crate) fn patch_block_header(out: &mut [u8], at: usize, last: bool, ty: BlockType, size: u32) {
    let t = match ty {
        BlockType::Raw => 0u32,
        BlockType::Rle => 1,
        BlockType::Compressed => 2,
    };
    let n = u32::from(last) | (t << 1) | (size << 3);
    out[at] = n as u8;
    out[at + 1] = (n >> 8) as u8;
    out[at + 2] = (n >> 16) as u8;
}

pub(crate) fn write_block_header(out: &mut Vec<u8>, last: bool, ty: BlockType, size: u32) {
    let t = match ty {
        BlockType::Raw => 0u32,
        BlockType::Rle => 1,
        BlockType::Compressed => 2,
    };
    let n = u32::from(last) | (t << 1) | (size << 3);
    out.push(n as u8);
    out.push((n >> 8) as u8);
    out.push((n >> 16) as u8);
}

pub(crate) fn write_frame_header(
    out: &mut Vec<u8>,
    src_len: u64,
    window_log: u32,
    checksum: bool,
    pledged: Option<u64>,
    dict_id: Option<u32>,
    ext_hist: bool,
) {
    out.extend_from_slice(&MAGIC.to_le_bytes());
    let window = 1u64 << window_log.min(31);
    let size = pledged.unwrap_or(src_len);
    let known = pledged.is_some();
    // Single_Segment window is Frame_Content_Size. Dict/prefix offsets can exceed
    // FCS, so never SS when external history is attached.
    let single = known && size <= window && !ext_hist;
    let (fcs_flag, fcs_bytes) = if !known {
        (0u8, Vec::new())
    } else if single && size < 256 {
        (0, vec![size as u8])
    } else if size < 256 + 65536 {
        let v = (size as u16).wrapping_sub(256);
        (1, v.to_le_bytes().to_vec())
    } else if size < 1 << 32 {
        (2, (size as u32).to_le_bytes().to_vec())
    } else {
        (3, size.to_le_bytes().to_vec())
    };
    let (fcs_flag, fcs_bytes) = if known && !single && size < 256 {
        (2u8, (size as u32).to_le_bytes().to_vec())
    } else {
        (fcs_flag, fcs_bytes)
    };
    let (dict_flag, dict_bytes): (u8, Vec<u8>) = match dict_id.filter(|&id| id != 0) {
        None => (0, Vec::new()),
        Some(id) if id < 256 => (1, vec![id as u8]),
        Some(id) if id < 65536 => (2, (id as u16).to_le_bytes().to_vec()),
        Some(id) => (3, id.to_le_bytes().to_vec()),
    };
    let mut desc = fcs_flag << 6;
    if single {
        desc |= 0x20;
    }
    if checksum {
        desc |= 0x04;
    }
    desc |= dict_flag;
    out.push(desc);
    if !single {
        let exp = window_log.saturating_sub(10).min(31);
        out.push((exp << 3) as u8);
    }
    out.extend_from_slice(&dict_bytes);
    out.extend_from_slice(&fcs_bytes);
}

/// Returns whether the section REUSED the previous Huffman table. A reused
/// table costs only the small section header, so the coded stream is then a
/// clean measure of the MARGINAL bits per literal; a freshly emitted table adds
/// a large fixed cost that has nothing to do with what one more literal costs.
/// SIMD-3 arm: the AVX2+BMI2 entropy twins (`write_literals`, `write_sequences`).
///
/// The 17 existing twins in this crate all enable `bmi2,lzcnt` and NOT avx2, so
/// every loop LLVM auto-vectorises inside them is emitted as 128-bit legacy SSE:
/// 526 SSE ops in `write_sequences_bmi2`, 283 in `write_literals_bmi2`, 0 ymm in
/// either. Unlike the block driver (SIMD-2, per BLOCK, measured 0), these bodies
/// carry per-SEQUENCE and per-LITERAL-BYTE loops, so the count multiplies.
/// 1 = off, 2 = on (default).
pub(crate) static ENC_AVX2_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);

/// Bench hook: `false` routes the entropy twins back to the bmi2-only arm.
///
/// **INERT as of the SIMD-3 retirement.** Both arms this knob selected are
/// gone: `write_literals_bmi2` was retired (0 BMI2 ops, and its baseline had 0
/// `%cl` shifts), and `write_sequences_avx2` was retired for growing the body
/// while emitting 98 `vmovups` and zero vector arithmetic. Nothing reads the
/// flag any more, so setting it changes no behaviour.
///
/// Kept because it is `pub use`d from the crate root and shipped in v0.1.0 --
/// removing it would be a breaking change for a knob that now costs one
/// atomic store on a path nobody takes. `crates/rusty_zstd-bench/examples/
/// simd3ab.rs` drives this and will now A/B two identical arms.
pub fn set_enc_avx2_arm(on: bool) {
    ENC_AVX2_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub(crate) fn write_literals(
    dst: &mut Vec<u8>,
    lits: &[u8],
    entropy: &mut EntropyState,
) -> Result<bool, Error> {
    // TWIN RETIRED, on its own measurement. The note this replaces justified a
    // wholesale BMI2 twin because "the literal-section table builders
    // (histogram, ctable, tree write, normalize, ncount) all carry variable
    // shifts". Measured on the emitted asm: the twin contained **zero** BMI2
    // instructions across 1,661 of body, and this baseline function contains
    // **zero** `%cl` shifts -- there was nothing on either side for the twin
    // to improve. The builders it names are separate symbols now (`from_norm`
    // 4 `%cl` shifts, `write_ncount` 5), and buying those 9 shift encodings
    // back would cost ~960 instructions of twin -- the trade this file has
    // rejected everywhere else.
    //
    // SIMD-3 REMAINS REFUTED HERE and the reasoning is unchanged: enabling
    // avx2 grew the body 3,659 -> 4,404 and measured Huff **+5.0% SLOWER**
    // (14-corpus in-process ABBA x7). Contrast `write_sequences`, which shrank
    // 8,769 -> 8,324 and measured -1.8% -- and which KEEPS both its twins
    // (45 BMI2 ops, 98 ymm on the avx2 arm; they earn their place).
    // **Enable a twin where the ISA count is real; retire it where it is zero.**
    write_literals_inner(dst, lits, entropy)
}

#[inline(always)]
pub(crate) fn write_literals_inner(
    dst: &mut Vec<u8>,
    lits: &[u8],
    entropy: &mut EntropyState,
) -> Result<bool, Error> {
    let _h = crate::prof::scope(crate::prof::Stage::EncodeHuff);
    // Writes THROUGH `dst`. The old shape took a `Vec` back, copied it in and
    // handed it to the pool; the four arms that decide immediately (empty, RLE,
    // tiny, not-worth-Huffman) now emit their bytes once instead of twice, and
    // the pool hand-back for the Huffman arm moved inside with the copy that
    // still has to happen there. ALLOC-13's loop is still closed -- see the
    // `sec_pool_give` at the end of `encode_literals_section_into`.
    let upd = huffman::encode_literals_section_into(dst, lits, entropy.huff.as_deref())?;
    let reused = matches!(upd, HuffUpdate::Unchanged);
    match upd {
        HuffUpdate::New(ct) => entropy.huff = Some(alloc::sync::Arc::new(ct)),
        HuffUpdate::Unchanged => {}
    }
    Ok(reused)
}

pub(crate) fn write_nseq(dst: &mut Vec<u8>, n: u32) {
    if n == 0 {
        dst.push(0);
    } else if n < 128 {
        dst.push(n as u8);
    } else if n < 0x7F00 {
        dst.push(((n >> 8) + 128) as u8);
        dst.push(n as u8);
    } else {
        dst.push(255);
        let v = n - 0x7F00;
        dst.push(v as u8);
        dst.push((v >> 8) as u8);
    }
}

pub(crate) fn write_sequences(
    dst: &mut Vec<u8>,
    seqs: &[Seq],
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    strategy: Strategy,
    tables: &mut MatchTables,
) -> Result<(), Error> {
    // D7: TWIN RETIRED on its ISA density -- 2,385 instructions of duplicated
    // body converting FORTY-FIVE BMI2 ops, 56 per op. The avx2 arm above it
    // went in Trans XI for emitting 98 `vmovups` and zero vector arithmetic;
    // this one converts real shifts, just not enough of them to justify a
    // second copy of the sequence-section encoder.
    //
    // The line this round has drawn, in instructions per ISA op converted:
    //   RETIRED  chain 152, greedy 123, lazy 111, bt-rt 97, dfast 72, this 56
    //   KEPT     find_fast_impl 44, decode_sequences_avx2 37,
    //            emit_fast_seq 32, compress_using_ctable 26, decode_4x 20
    // The kept twins are the dense ones on the per-position and per-literal-byte
    // paths, which is where an ISA fold can actually pay for its I-cache.
    write_sequences_inner(dst, seqs, reps, entropy, strategy, tables)
}

/// The per-sequence coding pass of `write_sequences_inner`, shared by its
/// three ISA twins. Returns the coded sequences plus the three histograms.
#[inline(never)]
#[allow(clippy::type_complexity)]
pub(crate) fn build_coded_pass(
    seqs: &[Seq],
    reps: &mut [u32; 3],
    tables: &mut MatchTables,
) -> Result<(Vec<CodedSeq>, [u32; 36], [u32; 32], [u32; 53], u8), Error> {
    let out = {
        let _sc = crate::prof::scope(crate::prof::Stage::EncodeSeqCode);
        // T4/brick-79: hoist the LUT arm out of the per-sequence loop. It was
        // read inside `ll_code` AND `ml_code`, i.e. two atomic loads per
        // sequence, while the two copy arms beside it are both resolved once
        // per block.
        let lut_arm = crate::compressed::lut_on();
        let mut coded: Vec<CodedSeq> = core::mem::take(&mut tables.coded_scratch);
        coded.clear();
        if coded.capacity() < seqs.len() {
            coded = Vec::with_capacity(seqs.len());
        }
        // The code histograms and the of_needs_comp scan were SEPARATE full
        // passes over `coded`; both fold into this loop.
        let mut ll_count = [0u32; 36];
        let mut of_count = [0u32; 32];
        let mut ml_count = [0u32; 53];
        let mut of_max = 0u8;
        for s in seqs {
            let ov = offset_value_for(s.offset, s.litlen, reps);
            // BRICK 62: advance the repcodes directly instead of calling the
            // DECODER's `resolve_offset` and discarding its result.
            //
            // `resolve_offset` reconstructs the offset from `ov` through a
            // branchy match plus a `Result` -- but the encoder already HAS that
            // offset in `s.offset`, and it is provably the same value:
            //   * `ov > 3`  => `offset_value_for` produced `s.offset + 3`, so
            //     `ov - 3 == s.offset` (offsets are window-bounded, so the
            //     `saturating_add(3)` there never saturates);
            //   * `ov == 3 && litlen == 0` => that arm is only taken when
            //     `s.offset == reps[0] - 1`, which is what it reconstructs.
            // The repcode SHUFFLE below is `resolve_offset`'s verbatim.
            let is_new = ov > 3 || (ov == 3 && s.litlen == 0);
            if is_new {
                reps[2] = reps[1];
                reps[1] = reps[0];
                reps[0] = s.offset;
            } else {
                let which = if s.litlen == 0 { ov + 1 } else { ov };
                match which {
                    2 => reps.swap(0, 1),
                    3 => reps.rotate_right(1),
                    _ => {}
                }
            }
            let (llc, llx, llb) = ll_code(s.litlen, lut_arm);
            let (mlc, mlx, mlb) = ml_code(s.matchlen, lut_arm);
            let (ofc, ofx) = of_code(ov);
            if ofc > 31 {
                return Err(Error::Corruption);
            }
            // WIN: the two clamps are NO-OPS and exist only so LLVM can drop a
            // bounds check, the same idiom as `rtb[(proba as usize).min(7)]` in
            // `fse::normalize`. `llc` and `mlc` come out of a LUT, so their range
            // (0..=35 and 0..=52, the lengths of LL_BASE and ML_BASE) is true by
            // construction but invisible to the optimiser -- unlike `ofc`, which
            // the explicit `ofc > 31` check above already makes provable, which is
            // exactly why only these two lines carried a guard branch.
            ll_count[(llc as usize).min(ll_count.len() - 1)] += 1;
            of_count[ofc as usize] += 1;
            ml_count[(mlc as usize).min(ml_count.len() - 1)] += 1;
            of_max = of_max.max(ofc);
            coded.push(CodedSeq {
                llc,
                mlc,
                ofc,
                llx,
                mlx,
                ofx,
                llb,
                mlb,
            });
        }

        (coded, ll_count, of_count, ml_count, of_max)
    };
    Ok(out)
}

#[inline(always)]
pub(crate) fn write_sequences_inner(
    dst: &mut Vec<u8>,
    seqs: &[Seq],
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    strategy: Strategy,
    tables: &mut MatchTables,
) -> Result<(), Error> {
    write_nseq(dst, seqs.len() as u32);
    if seqs.is_empty() {
        return Ok(());
    }

    // THE CODED PASS IS ONE NON-GENERIC CALL. `write_sequences_inner` is
    // compiled into three ISA twins for the FSE BIT-WRITING loop below; this
    // pass -- rep resolution, ll/ml/of coding and the three histograms -- was
    // stamped into each. Its only ISA-sensitive ops are the `leading_zeros`
    // in `ll_code`/`ml_code` (lzcnt vs `bsr`+xor, both a couple of uops), so
    // one shared copy trades a marginal encoding for two fewer stamps of the
    // whole pass. Output is identical either way -- the codes are the codes.
    let (coded, ll_count, of_count, ml_count, of_max) = build_coded_pass(seqs, reps, tables)?;
    let use_low = strategy.id() >= Strategy::Lazy.id();
    // WIN: fetch the final sequence ONCE, by value (`CodedSeq: Copy`).
    // `coded.len() - 1` underflows to `usize::MAX` on an empty `coded`, so
    // every one of the NINE `coded[last]` reads below had to carry its own
    // bounds check -- LLVM cannot rule out the wrapped index. `.last()` states
    // the same intent without the underflow, so all nine checks go, and the
    // empty case becomes a clean error instead of a panic.
    let last_seq = *coded.last().ok_or(Error::Corruption)?;
    // REFUTED, recorded: extracting these three `select_seq_table` calls into a
    // shared non-ISA `select_seq_tables` -- the `build_coded_pass` treatment,
    // and structurally the same opportunity -- measured **+348**. Each twin did
    // shrink (bmi2 2,535 -> 2,385), but the helper returns NINE values: three
    // `SeqTable`s and three `Vec`s. That tuple goes through memory, and the
    // moves cost more than the duplication saved.
    //
    // The lesson generalises: an extraction's win is (copies - 1) x body, and
    // its cost is the CALLING CONVENTION. When the interface is a handful of
    // scalars, as in `build_coded_pass`, the trade is free; when it is six
    // owned heap values, it is not. Price the return, not just the body.
    let (ll_mode, ll_t, ll_hdr, of_mode, of_t, of_hdr, ml_mode, ml_t, ml_hdr) = {
        let _t = crate::prof::scope(crate::prof::Stage::EncodeTableSelect);
        let (ll_mode, ll_t, ll_hdr) = select_seq_table(
            &ll_count,
            36,
            9,
            &fse::DEFAULT_LL_NORM,
            6,
            entropy.ll.as_deref().map(|r| &**r),
            use_low,
            false,
            last_seq.llc as usize,
        )?;
        let of_needs_comp = of_max as usize >= fse::DEFAULT_OF_NORM.len();
        let (of_mode, of_t, of_hdr) = select_seq_table(
            &of_count,
            32,
            8,
            &fse::DEFAULT_OF_NORM,
            5,
            entropy.of.as_deref().map(|r| &**r),
            use_low,
            of_needs_comp,
            last_seq.ofc as usize,
        )?;
        let (ml_mode, ml_t, ml_hdr) = select_seq_table(
            &ml_count,
            53,
            9,
            &fse::DEFAULT_ML_NORM,
            6,
            entropy.ml.as_deref().map(|r| &**r),
            use_low,
            false,
            last_seq.mlc as usize,
        )?;
        (
            ll_mode, ll_t, ll_hdr, of_mode, of_t, of_hdr, ml_mode, ml_t, ml_hdr,
        )
    };

    crate::prof::note_seq_mode(ll_mode);
    crate::prof::note_seq_mode(of_mode);
    crate::prof::note_seq_mode(ml_mode);
    dst.push((ll_mode << 6) | (of_mode << 4) | (ml_mode << 2));
    dst.extend_from_slice(&ll_hdr);
    dst.extend_from_slice(&of_hdr);
    dst.extend_from_slice(&ml_hdr);
    // ALLOC-14: the three ncount headers die here -- copied into `dst` and
    // dropped. Close the loop, or the pool starves exactly as ALLOC-13's did.
    fse::give_ncount_buf(ll_hdr);
    fse::give_ncount_buf(of_hdr);
    fse::give_ncount_buf(ml_hdr);

    let _fs = crate::prof::scope(crate::prof::Stage::EncodeFseSeq);
    let mut ml_s = ml_t.init_state2(last_seq.mlc as usize);
    let mut of_s = of_t.init_state2(last_seq.ofc as usize);
    let mut ll_s = ll_t.init_state2(last_seq.llc as usize);

    let mut bits = BitCStream::from_vec(
        core::mem::take(&mut tables.bits_scratch),
        coded.len() * 4 + 16,
    );
    bits.add_bits(u64::from(last_seq.llx), u32::from(last_seq.llb));
    bits.add_bits(u64::from(last_seq.mlx), u32::from(last_seq.mlb));
    bits.add_bits(u64::from(last_seq.ofx), u32::from(last_seq.ofc));
    bits.flush();

    if coded.len() >= 2 {
        for n in (0..coded.len() - 1).rev() {
            let c = &coded[n];
            of_t.encode(&mut of_s, &mut bits, c.ofc as usize);
            ml_t.encode(&mut ml_s, &mut bits, c.mlc as usize);
            ll_t.encode(&mut ll_s, &mut bits, c.llc as usize);
            bits.add_bits(u64::from(c.llx), u32::from(c.llb));
            bits.add_bits(u64::from(c.mlx), u32::from(c.mlb));
            bits.add_bits(u64::from(c.ofx), u32::from(c.ofc));
        }
    }

    ml_t.flush(ml_s, &mut bits);
    of_t.flush(of_s, &mut bits);
    ll_t.flush(ll_s, &mut bits);
    let out = bits.close();
    dst.extend_from_slice(&out);
    tables.bits_scratch = out;
    tables.coded_scratch = coded;
    // ALLOC-5 (N11): write back ONLY a table that is actually new.
    //
    // `prev` for each of the three is `entropy.<x>.as_ref()`, so on Repeat mode
    // the old code cloned `entropy.ll` and then assigned the clone straight back
    // onto `entropy.ll` -- two or three heap allocations to replace a table with
    // a copy of itself. Borrowing (`SeqTable::Ref`) makes that visible, and the
    // match consumes the carrier so the borrow of `entropy` ends before the
    // write. Byte-identical: the retained table is the same table either way.
    let ll_new = match ll_t {
        SeqTable::Own(t) => Some(RetainedTable::Own(t)),
        SeqTable::Static(t) => Some(RetainedTable::Static(t)),
        SeqTable::Ref(_) => None,
    };
    let of_new = match of_t {
        SeqTable::Own(t) => Some(RetainedTable::Own(t)),
        SeqTable::Static(t) => Some(RetainedTable::Static(t)),
        SeqTable::Ref(_) => None,
    };
    let ml_new = match ml_t {
        SeqTable::Own(t) => Some(RetainedTable::Own(t)),
        SeqTable::Static(t) => Some(RetainedTable::Static(t)),
        SeqTable::Ref(_) => None,
    };
    if let Some(t) = ll_new {
        entropy.ll = Some(alloc::sync::Arc::new(t));
    }
    if let Some(t) = of_new {
        entropy.of = Some(alloc::sync::Arc::new(t));
    }
    if let Some(t) = ml_new {
        entropy.ml = Some(alloc::sync::Arc::new(t));
    }
    Ok(())
}

/// libzstd `ZSTD_buildCTable`: last sequence is `FSE_initCState2` only.
#[inline(always)]
pub(crate) fn ncount_seq_table(
    counts: &[u32],
    last_sym: usize,
    max_log: u8,
    use_low_prob: bool,
) -> Result<(Vec<u8>, FseCTable), Error> {
    // libzstd ZSTD_buildCTable: last sequence is FSE_initCState2 only, so drop
    // it from the normalized counts when it still leaves a usable distribution.
    //
    // This was `counts.to_vec()` -- a fresh heap allocation per call, to copy a
    // histogram and decrement ONE entry. It ran three times per block (ll/of/ml)
    // and the allocation-site census put `select_seq_table` at HALF of all
    // encoder allocations because of it. The sequence tables' symbol counts are
    // fixed by the format (LL 36, OF 32, ML 53), so the copy fits on the stack
    // and the allocation disappears entirely.
    const MAX_SEQ_SYMS: usize = 64;
    let n = counts.len();
    if n <= MAX_SEQ_SYMS {
        let mut buf = [0u32; MAX_SEQ_SYMS];
        buf[..n].copy_from_slice(counts);
        if last_sym < n && buf[last_sym] > 1 {
            buf[last_sym] -= 1;
        }
        return fse::ncount_and_ctable(&buf[..n], max_log, use_low_prob);
    }
    // Not reachable for the three sequence tables; kept so a future caller with
    // a wider alphabet cannot silently truncate.
    let mut buf = counts.to_vec();
    if last_sym < buf.len() && buf[last_sym] > 1 {
        buf[last_sym] -= 1;
    }
    fse::ncount_and_ctable(&buf, max_log, use_low_prob)
}

/// ALLOC-5 (N11): a seq table that may be BORROWED.
///
/// `select_seq_table` returned an owned `FseCTable`, so the two winning modes
/// that already have one -- Repeat (the caller's `prev`) and Predefined (the
/// process-constant cached table from N9) -- each paid a full clone: two or
/// three heap allocations for a table the caller could simply borrow. Only the
/// Compressed mode genuinely builds a new one.
///
/// Derefs to `FseCTable`, so every consumer reads unchanged.
#[cfg(feature = "alloc")]
pub(crate) enum SeqTable<'a> {
    /// Already the table `entropy` holds -- nothing to write back.
    Ref(&'a FseCTable),
    /// The process-constant Predefined table -- retain by reference.
    Static(&'static FseCTable),
    Own(FseCTable),
}

#[cfg(feature = "alloc")]
impl core::ops::Deref for SeqTable<'_> {
    type Target = FseCTable;
    #[inline(always)]
    fn deref(&self) -> &FseCTable {
        match self {
            SeqTable::Ref(t) => t,
            SeqTable::Static(t) => t,
            SeqTable::Own(t) => t,
        }
    }
}

/// Select Predefined / RLE / FSE-compressed / Repeat. Returns (mode, table, header bytes).
///
/// `#[inline(never)]`, NOT `always`. This runs THREE TIMES PER BLOCK -- once
/// each for litlen, offset and matchlen -- so a call is free at that
/// frequency. Inlined it was reproduced 3x in `write_sequences_inner`, and
/// that function has THREE twins (baseline / bmi2 / avx2), so the selector's
/// whole body existed NINE times. `write_sequences` was the largest lump left
/// in the crate at 12,413 x 3 = 36K instructions.
#[allow(clippy::too_many_arguments)]
#[inline(never)]
pub(crate) fn select_seq_table<'a>(
    counts: &[u32],
    _alphabet: usize,
    max_log: u8,
    default_norm: &[i16],
    default_log: u8,
    prev: Option<&'a FseCTable>,
    use_low_prob: bool,
    force_compressed: bool,
    last_sym: usize,
) -> Result<(u8, SeqTable<'a>, Vec<u8>), Error> {
    let total: u32 = counts.iter().sum();
    let most = counts.iter().copied().max().unwrap_or(0);
    // libzstd ZSTD_selectEncodingType: a single symbol is always RLE.
    if total > 0 && most == total {
        let sym = counts.iter().position(|&c| c == total).unwrap_or(0) as u8;
        // Pooled, not `vec![sym]`: that was a fresh heap allocation for ONE
        // byte, and the caller already returns this buffer through
        // `give_ncount_buf`, so the loop closes with no new plumbing.
        let mut hdr = fse::take_ncount_buf();
        hdr.push(sym);
        return Ok((1, SeqTable::Own(FseCTable::rle(u16::from(sym))), hdr));
    }

    // N9 probe: this rebuilds an RFC-CONSTANT ctable -- three heap allocations,
    // a cumul pass, the serial spread, a scatter into state_table and the delta
    // build -- for a value fixed for the life of the process.
    // N9: hand out the process-constant table by reference. Only the losing
    // path below needs to own it, and Predefined winning is the minority case.
    let basic_owned;
    #[cfg(all(feature = "std", feature = "alloc"))]
    let cached = fse::default_ctable_cached(default_norm, default_log);
    #[cfg(not(all(feature = "std", feature = "alloc")))]
    let cached: Option<&fse::FseCTable> = None;
    let basic: &fse::FseCTable = match cached {
        Some(t) => t,
        None => {
            #[cfg(feature = "profile")]
            N9_BASIC.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            basic_owned = fse::FseCTable::from_norm(default_norm, default_log)?;
            &basic_owned
        }
    };
    let mut best_mode = 0u8;
    let mut best_table: Option<SeqTable<'a>> = None;
    let mut best_hdr = Vec::new();
    let mut best_cost = basic.bit_cost(counts);

    if let Some(p) = prev {
        let c = p.bit_cost(counts);
        if c <= best_cost {
            best_mode = 3;
            // ALLOC-5: borrow the caller's table instead of cloning it.
            best_table = Some(SeqTable::Ref(p));
            best_hdr = Vec::new();
            best_cost = c;
        }
    }

    if total >= 8 {
        if let Ok((hdr, ct)) = ncount_seq_table(counts, last_sym, max_log, use_low_prob) {
            let c = ct.bit_cost(counts) + (hdr.len() as u64) * 8;
            if c < best_cost || force_compressed {
                best_mode = 2;
                best_table = Some(SeqTable::Own(ct));
                best_hdr = hdr;
                best_cost = c;
            } else {
                // ALLOC-16: the LOSING candidate's ncount header dies right
                // here. Without this it never reached the pool and
                // `write_ncount` stayed the top site after ALLOC-14.
                fse::give_ncount_buf(hdr);
            }
        }
    }

    if force_compressed && best_mode == 0 {
        if let Ok((hdr, ct)) = ncount_seq_table(counts, last_sym, max_log, use_low_prob) {
            return Ok((2, SeqTable::Own(ct), hdr));
        }
        return Err(Error::Corruption);
    }
    let _ = best_cost;
    // ALLOC-5, CORRECTED: the Predefined fallback must stay OWNED.
    //
    // Returning `Ref(cached)` here looked free and changed the bitstream --
    // `bytegate` caught it on mozilla. The caller writes the returned table into
    // `entropy.<x>`, which becomes the NEXT block's `prev`; on Predefined mode
    // the old code therefore replaced the retained table with the predefined
    // one. Skipping that write-back left the previous (compressed) table in
    // place, the next block's Repeat test saw a different `prev`, and the
    // decisions diverged.
    //
    // So `SeqTable::Ref` now means exactly one thing: "this IS the table
    // `entropy` already holds, so there is nothing to write back." It is
    // produced only on the Repeat path, from `prev`. Every other mode owns.
    // ALLOC-7: Predefined retains the cached `&'static` table by reference.
    // ALLOC-5's correction still holds -- this MUST still be written back, and
    // `SeqTable::Static` carries that instruction; what it no longer does is
    // clone a process constant to do it.
    Ok((
        best_mode,
        best_table.unwrap_or_else(|| match cached {
            Some(t) => SeqTable::Static(t),
            None => SeqTable::Own(basic.clone()),
        }),
        best_hdr,
    ))
}

#[derive(Clone, Copy)]
pub(crate) struct CodedSeq {
    pub(crate) llc: u8,
    pub(crate) mlc: u8,
    pub(crate) ofc: u8,
    pub(crate) llx: u32,
    pub(crate) mlx: u32,
    pub(crate) ofx: u32,
    pub(crate) llb: u8,
    pub(crate) mlb: u8,
}
