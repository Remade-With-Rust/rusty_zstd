//! RFC 8878 encoder: all literals types and sequence compression modes.
//!
//! Compressed bytes are not required to match C. Dual gate: our decoder and
//! C `zstd -d` reconstruct the source bit-exact.

use crate::bit::BitCStream;
use crate::block::BlockType;
use crate::compressed::{ll_code, ml_code, of_code, offset_value_for, resolve_offset};
use crate::dict::Dictionary;
use crate::error::Error;
use crate::frame::{BLOCKSIZE_MAX, MAGIC};
use crate::fse::{self, FseCTable};
use crate::huffman::{self, HuffCTable, HuffUpdate};
use crate::params::{compression_params, CompressionParameters, Strategy};
use crate::xxh64::{content_checksum, Xxh64};
use alloc::vec;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// One-shot compress at `level` (-7..=22). Checksum on, content size in the header.
#[cfg(feature = "alloc")]
pub fn compress(src: &[u8], level: i32) -> Result<Vec<u8>, Error> {
    compress_with(
        src,
        CompressOptions {
            level,
            checksum: true,
        },
    )
}

/// Knobs for [`compress_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CompressOptions {
    /// Compression level (-7..=22).
    pub level: i32,
    /// Write the XXH64 content checksum.
    ///
    /// **The LIBRARY default is OFF** (`ZSTD_c_checksumFlag = 0`); it is the
    /// zstd CLI that turns it on for files. We match the CLI, because that is
    /// what a user of this crate expects. Do not "correct" this to match
    /// libzstd -- but DO remember which default you are comparing against:
    /// benchmarking us against `zstd -b` (no `--check`) with this on charges
    /// us a full xxh64 pass over every byte that C never runs. That mistake
    /// cost this campaign a phantom 2.2x. See docs/plans/m7-encoder-whys.md.
    pub checksum: bool,
}

impl Default for CompressOptions {
    fn default() -> Self {
        Self {
            level: crate::DEFAULT_CLEVEL,
            checksum: true,
        }
    }
}

/// Extra compressor knobs: LDM (`--long`), `--rsyncable`, target cblock size, MT.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct AdvancedOptions {
    /// Long-distance matching.
    pub ldm: crate::ldm::LdmParams,
    /// Periodic hash-table-friendly block cuts.
    pub rsyncable: bool,
    /// Aim for compressed blocks near this size (`0` = off).
    pub target_cblock_size: u32,
    /// Worker threads (`ZSTD_c_nbWorkers`). `0` = single-thread oneshot (not `-T0`).
    pub nb_workers: u32,
    /// MT job size in bytes. `0` = `4 * window` (then the 512 KiB floor).
    pub job_size: usize,
    /// `overlapLog` (`0` = default by strategy, `1` = independent jobs, `9` = full window).
    pub overlap_log: u32,
    /// Prime match tables from `prefix` but never emit offsets into it (MT overlap).
    pub prime_only: bool,
}

/// One-shot compress with explicit options.
#[cfg(feature = "alloc")]
pub fn compress_with(src: &[u8], opts: CompressOptions) -> Result<Vec<u8>, Error> {
    let params = compression_params(opts.level, Some(src.len() as u64))?;
    encode_oneshot(
        src,
        params,
        opts.checksum,
        Some(src.len() as u64),
        None,
        &[],
        true,
        AdvancedOptions::default(),
    )
}

/// FINDING 1 (Gate 2 @ L19): size the window from payload + prefix, as C does.
///
/// libzstd's `ZSTD_adjustCParams(cPar, srcSize, dictSize)` clamps `windowLog`
/// against `srcSize + dictSize`. We clamped against the PAYLOAD alone, so a
/// 4 MiB reference behind a 1 MiB payload produced windowLog 20 and three of the
/// four reference megabytes were unreachable by construction.
///
/// Measured at L19 over 15 corpora, once FINDING 2 built the tree over the
/// prefix: **-2.047%, 12 smaller / 2 larger** (nci -9.87%, reymont -6.95%,
/// webster -6.89%). The two are COUPLED -- measured alone, before the tree
/// existed, this same change was only -0.389% with 6 corpora larger, because a
/// wider window cannot pay when there is no tree to search in it.
///
/// L3 is unchanged (+0.009%): DFast has no tree, so the extra window has nothing
/// to exploit. That asymmetry is the confirmation.
///
/// The cost is real and is recorded: the advertised `windowLog` rises 20 -> 23,
/// so a decoder must allocate 8 MiB for that frame instead of 1 MiB. C makes the
/// same trade.
fn params_with_history(level: i32, src_len: usize, hist_len: usize) -> Result<CompressionParameters, Error> {
    let hint = if prefix_window_enabled() {
        (src_len as u64).saturating_add(hist_len as u64)
    } else {
        src_len as u64
    };
    compression_params(level, Some(hint))
}

/// FINDING 1 -- **DEFAULT ON. This is a CONTRACT fix, not a speed trade.**
///
/// `compress_using_dict` / `compress_using_prefix` used to size the window from
/// `src.len()` ALONE, where libzstd's `ZSTD_adjustCParams(cPar, srcSize,
/// dictSize)` clamps `windowLog` against `srcSize + dictSize`. Because every
/// finder rejects a candidate at `ip - m > window`, everything in the supplied
/// dictionary beyond a payload-sized window was UNREACHABLE.
///
/// PROVEN, not argued. Compressing one payload against dictionaries built from
/// the same bytes truncated to 4 MiB / 2 MiB / 1 MiB produced BYTE-IDENTICAL
/// output on 8 of 8 corpora at L19 -- the caller's dictionary was silently
/// truncated to the window and three quarters of it did nothing.
///
/// That is why the earlier "-0.402% size for +15.4% time, 6 corpora larger"
/// verdict was the wrong test: it compared two arms that do DIFFERENT AMOUNTS OF
/// WORK. The fast arm was fast because it ignored most of the input it was given.
/// The worst-corpus rule governs equivalent arms; it does not license silently
/// discarding a caller's data to save time.
///
/// The real cost is honest and belongs to the caller: the advertised `windowLog`
/// rises (20 -> 23 in the measured shape), so a decoder allocates 8 MiB for that
/// frame instead of 1 MiB. libzstd obliges its decoders identically. A caller who
/// does not want that should pass a smaller dictionary -- which now actually
/// means what it says.
///
/// Only the dict/prefix path is affected. `compress()` has no prefix, so the
/// 60-cell size table and every speed board are untouched.
static PREFIX_WINDOW_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `true` sizes the window from payload + prefix, as libzstd does.
pub fn set_prefix_window_arm(on: bool) {
    PREFIX_WINDOW_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prefix_window_enabled() -> bool {
    // DEFAULT ON: 0 (unresolved) and 2 both mean on; only an explicit
    // `set_prefix_window_arm(false)` (stored as 1) restores the old behaviour,
    // which is the byte-identical fallback the ledger requires.
    !matches!(
        PREFIX_WINDOW_ARM.load(core::sync::atomic::Ordering::Relaxed),
        1
    )
}

/// Compress `src` using a dictionary (raw or trained).
pub fn compress_using_dict(src: &[u8], dict: &Dictionary, level: i32) -> Result<Vec<u8>, Error> {
    compress_using_dict_with(
        src,
        dict,
        CompressOptions {
            level,
            checksum: true,
        },
        true,
    )
}

/// Compress with a dictionary and explicit checksum / Dictionary_ID knobs.
pub fn compress_using_dict_with(
    src: &[u8],
    dict: &Dictionary,
    opts: CompressOptions,
    write_dict_id: bool,
) -> Result<Vec<u8>, Error> {
    let params = params_with_history(opts.level, src.len(), dict.content().len())?;
    encode_oneshot(
        src,
        params,
        opts.checksum,
        Some(src.len() as u64),
        Some(dict),
        &[],
        write_dict_id,
        AdvancedOptions::default(),
    )
}

/// Compress `src` with an external prefix (`--patch-from` / `ZSTD_CCtx_refPrefix`).
/// No Dictionary_ID is written.
pub fn compress_using_prefix(src: &[u8], prefix: &[u8], level: i32) -> Result<Vec<u8>, Error> {
    let params = params_with_history(level, src.len(), prefix.len())?;
    encode_oneshot(
        src,
        params,
        true,
        Some(src.len() as u64),
        None,
        prefix,
        false,
        AdvancedOptions::default(),
    )
}

/// One-shot compress with already-resolved compression parameters (`--zstd=`).
#[cfg(feature = "alloc")]
pub fn compress_with_params(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
) -> Result<Vec<u8>, Error> {
    encode_oneshot(
        src,
        params,
        checksum,
        Some(src.len() as u64),
        None,
        &[],
        true,
        AdvancedOptions::default(),
    )
}

/// One-shot compress with an optional dictionary or prefix (`-D` / `--patch-from`).
pub fn compress_with_history(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    dict: Option<&Dictionary>,
    prefix: &[u8],
    write_dict_id: bool,
) -> Result<Vec<u8>, Error> {
    compress_with_advanced(
        src,
        params,
        checksum,
        dict,
        prefix,
        write_dict_id,
        AdvancedOptions::default(),
    )
}

/// [`compress_with_history`] plus LDM / rsyncable / target-cblock.
#[allow(clippy::too_many_arguments)]
pub fn compress_with_advanced(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    dict: Option<&Dictionary>,
    prefix: &[u8],
    write_dict_id: bool,
    adv: AdvancedOptions,
) -> Result<Vec<u8>, Error> {
    if adv.nb_workers > 0 {
        #[cfg(feature = "std")]
        {
            return crate::mt::compress_mt(src, params, checksum, dict, prefix, write_dict_id, adv);
        }
    }
    encode_oneshot(
        src,
        params,
        checksum,
        Some(src.len() as u64),
        dict,
        prefix,
        write_dict_id,
        adv,
    )
}

#[derive(Clone, Copy, Debug)]
struct Seq {
    litlen: u32,
    matchlen: u32,
    offset: u32,
}

#[derive(Clone)]
pub(crate) struct MatchTables {
    hash: Vec<u32>,
    hash_long: Vec<u32>,
    /// 1a array route: the long table's tag byte array, for frames where the
    /// packed form is refused (>= 16 MiB, streaming). Mirrors `tags` exactly:
    /// empty on packed frames, allocated at frame init when the arms say so.
    ltags: Vec<u8>,
    /// BRICK 67: repcode yield of the PREVIOUS block -- the dispatch signal
    /// for the repcode-1 search. Optimistic start so the first block always
    /// probes; a block finding no repcodes turns it off for the next.
    ///
    /// Safe to flip per block: repcode search only changes WHICH matches the
    /// encoder finds, leaving no stale table state (unlike the tag latch).
    rep_yield: f32,
    /// BRICK 52: the AUTHORITATIVE hash_log, clamped once here. The table size
    /// and the hash SHIFT must be derived from the same value or they disagree:
    /// `params.hash_log` can reach 25 (a level-table row) while the table is
    /// capped at `1 << 24`, and `hv >> (32 - 25)` would then index 25 bits into
    /// a 24-bit table. Holding it here makes `h < hash.len()` true by
    /// construction, which is what lets the mask go.
    hash_log: u32,
    chain: Vec<u32>,
    /// Workspace index: matches must start at or after this (MT overlap prime).
    frame_start: usize,
    /// Sequences the previous block produced, used to size the next block's
    /// `seqs` reservation. A fixed fraction of the block length cannot work:
    /// nci needs ~4k slots per 128 KiB block while sao needs ~357, and
    /// over-reserving measurably regressed sao (+3.9% cyc/byte) -- the same
    /// holdout sign-flip that reverted bricks 31 and 34.
    last_nseq: usize,
    /// GATE 18 @ L1 step probe. `pair_route == 1` pins the search step at 1, and
    /// on sao/mr/dickens step 2 is SMALLER and saves 25-45% of positions. No
    /// content signal separates those from samba/mozilla/x-ray, so the step is
    /// MEASURED instead of predicted: alternate 1,2,1,2 over the first blocks,
    /// compare compressed bytes per input byte, and latch the winner.
    ///
    /// 0 = still probing, 1 = latched on step 1, 2 = latched on step 2.
    step_pick: u8,
    /// Step used by the block currently being encoded, so the outcome can be
    /// attributed to the arm that produced it.
    step_used: u8,
    /// Forces the GATE 6 pair route during a probe run. 0 = normal dispatch.
    route_force: u8,
    /// Blocks probed, and accumulated (compressed / input) per arm.
    step_probed: u32,
    step_sum1: f64,
    step_sum2: f64,
    /// Countdown to a forced re-probe, so content that changes character is
    /// picked up -- the warm-up + re-probe shape GATES 2, 6, 10 and 14 all need.
    step_reprobe: u32,
    /// Search positions per byte from the PREVIOUS block -- the dispatch signal
    /// for the lazy back-fill (defect B1). Measured, not assumed: see the
    /// truth table in `m7-benchmark-repair.md`. High = the search is working
    /// hard to find matches, so a richer chain pays; low = matches come easily
    /// (dense repetitive content) and extra chain density is pure walk cost.
    last_search_per_byte: f32,
    /// WALK-CONTINUE dispatch: EWMA share of walk-continue accepts that were
    /// FIRST-FINDS past a collision (legacy would have emitted a literal),
    /// and the Gate-2-style re-probe countdown. First-find-dominated content
    /// (jsonlog 67%, smallmsg 74%) LOSES under the C-parity walk -- the found
    /// matches displace cheaper literal+rep economies -- while upgrade-rich
    /// content (dickens 45%, reymont 41%) wins big.
    walk_first_share: f32,
    walk_probe: u32,
    /// True once `walk_first_share` has been fed at least one measured block.
    walk_share_meas: bool,
    /// Consecutive measured blocks with walk_first_share under the wide bar.
    wide_ok_blocks: u32,
    /// See `set_chain_tag_arm`: lazy-ladder heads and links carry the hash4
    /// tag in their high 8 bits this frame.
    chain_pack: bool,
    /// Array route for the same filter where the packed form is refused
    /// (>= 16 MiB, streaming): link tags beside `chain`, head tags in
    /// `tags`. Mirrors the ltags story exactly.
    ctags: Vec<u8>,
    /// Frame scratch for the per-block sequence coding (the GATE 6 family):
    /// `coded` and the bitstream buffer were fresh allocations per block.
    coded_scratch: Vec<CodedSeq>,
    bits_scratch: Vec<u8>,
    /// See `set_wide_chain_arm`.
    chain_wide: bool,
    /// Blocks whose finder has actually RUN and written back its signals.
    ///
    /// GATE 1 @ L1 needs this because `rep_yield` starts OPTIMISTIC at 1.0 so
    /// the first block of every frame probes for repcodes. A dispatch reading
    /// `rep_yield` directly would therefore fire on block 0 of EVERY file,
    /// changing output everywhere. Gating on `blocks_done > 0` makes the
    /// dispatch fire only on measured evidence.
    blocks_done: u32,
    /// GATE 6 @ L3: the block payload buffer, REUSED across every block of the
    /// frame instead of being built fresh each time.
    ///
    /// The gate as written chose between `Vec::new()` (grow by doubling, which
    /// memcpy'd 40.2 MB across the 18-corpus board) and
    /// `Vec::with_capacity(block.len())` (one 128 KiB request per block). The
    /// clock cannot separate them -- a null arm measuring the reserve against
    /// ITSELF reads up to +-24.15% -- but a counting allocator can, and it shows
    /// the reserve was trading one cost for another: -77.18% bytes copied, but
    /// +812 allocations at or above 128 KiB. `block.len()` IS `BLOCKSIZE_MAX`,
    /// so the reserve landed exactly on the large-allocation threshold and
    /// bought a fresh VirtualAlloc, and its page-table edit, for every block.
    ///
    /// Keeping the buffer sidesteps the choice: it reaches its steady-state
    /// capacity once per frame and then neither grows nor is freed.
    payload_scratch: Vec<u8>,
    /// GATE 6 @ L1: the finder's sequence and literal buffers, kept on the
    /// frame for the same reason as `payload_scratch`. `lit_scratch` is the
    /// expensive one -- sized `block_len + LIT_PUSH_WIDTH_MAX`, it cleared the
    /// 128 KiB large-allocation threshold on every single block.
    seq_scratch: Vec<Seq>,
    lit_scratch: Vec<u8>,
    /// GATE 6, deeper: `find_opt`'s parse-backtrace buffer, kept for the same
    /// reason as `payload_scratch` and worth far more -- this one carried the
    /// bulk of the 340 MB L19 was pushing through `realloc` on a 2 MiB board.
    opt_ops: Vec<(u32, u32, u32, bool)>,
    /// T2: `find_opt`'s DP arrays, kept on the frame.
    ///
    /// Sized `n + 1` for a block of `n`, they were built fresh EVERY block:
    /// `price` 0.50 MiB, `prev` **1.00 MiB**, `is_match` 0.13, `match_off` 0.50,
    /// `match_ml` 0.50 -- **2.63 MiB allocated and freed per block** at a 128 KiB
    /// block size.
    ///
    /// This is the one allocation site on the board where the size class
    /// actually matters. `allocost` measured no cliff at 128-512 KiB (a fresh
    /// buffer costs what a kept one costs, to within noise) but a large one at
    /// 1 MiB: 392 us fresh against 33 us reused, +1078%, the OS zero-filling
    /// pages the heap has stopped recycling. `prev` sits exactly there.
    opt_price: Vec<u32>,
    opt_prev: Vec<u32>,

    opt_om: Vec<u64>,
    /// GATE 6 @ L3: share of DFast positions where C's `_search_next_long`
    /// probe at `ip+1` actually BEAT the short-hash candidate, measured on the
    /// PREVIOUS block. Same self-calibrating shape as `rep_yield`: the probe
    /// cannot lose locally (it is taken only when strictly longer), so its
    /// losses are downstream parse-cascade effects, and the corpora it hurts are
    /// the ones where it fires often and buys little.
    next_long_yield: f32,
    /// GATE 14 @ L3 dispatch: EWMA of the share of raised-band next-long hits
    /// that take a LARGER offset than the match they replace.
    nl_off_worse: f32,
    /// Blocks in which the raised band was actually measured.
    nl_band_meas: u32,
    /// Re-probe countdown: the band cannot be measured while the cut is low, so
    /// the gate must periodically raise it again or it latches shut forever.
    nl_band_probe: u32,
    /// GATE 13 @ L1 -- share of the PREVIOUS block's literal runs short enough
    /// for the fixed-width copy to catch. Seeded optimistically so block 0
    /// always takes the fast path.
    lit_short_share: f32,
    /// GATE 13 WIDTH: share of the previous block's literal runs in (16, 32] --
    /// the runs a 32-byte copy catches that a 16-byte one does not.
    lit_mid_share: f32,
    /// GATE 2 second variable: mean rep match length divided by mean match
    /// length on the previous block. Below 1 the repcode search is trading a
    /// LONGER hash match for a shorter rep match; above 1 its matches are the
    /// long ones and taking them is free.
    rep_len_ratio: f32,
    /// Countdown to the next forced rep re-probe (the ratio can only be measured
    /// on a block where the search actually ran).
    rep_probe: u32,
    /// Consecutive blocks emitted RAW. Incompressible content otherwise pays the
    /// full match search before anything discovers it is incompressible.
    raw_run: u32,
    /// Countdown to the next forced re-probe of the raw short circuit.
    raw_probe: u32,
    /// GATE 10 @ L19: bytes the opt DP's repcode candidate covers per probe,
    /// EWMA'd. It runs at EVERY position and is worth keeping on almost nothing:
    /// versions-16m 434 B/probe and text-32m 26,932 need it, everything else is
    /// at most 35.6 and is SMALLER without it.
    opt_rep_rate: f32,
    /// Countdown to the next forced re-probe, so an off block can be re-measured.
    opt_rep_probe: u32,
    /// Highest bytes-per-rep-probe seen this frame, and how many real
    /// measurements it is built from. The PEAK is what characterises the
    /// content; a single dry block sends the instantaneous rate to 0.
    opt_rep_peak: f32,
    opt_rep_meas: u32,
    /// Blocks in which the candidate has actually RUN. The gate may not shut
    /// until it has real evidence: block 0 of a frame has no history, so its
    /// rate is unrepresentative -- and because an OFF block records no hits, a
    /// gate that shuts on block 0 suppresses its own measurement and can never
    /// reopen. Same cold-start defect as Gate 6's `pair_gain` (4.17).
    opt_rep_seen: u32,
    /// GATE 19: measured literal price in bits, fed to the next block's opt DP.
    /// 0 = not yet measured this frame.
    ///
    /// PER-FRAME, like every other feedback signal here. It first shipped as a
    /// process-global static, which made compression depend on CALL HISTORY:
    /// the same input at L19 gave a different result on the first call than on
    /// later ones (8/12 corpora), because the frame inherited whatever the
    /// PREVIOUS compression had left behind.
    opt_lit_price: u32,
    /// GATE 9 @ L3: mean MATCH LENGTH on the previous DFast block, EWMA'd.
    /// Skipping odd positions shifts a LONG match by a byte (free) but loses a
    /// SHORT match outright, and loses nothing where there are no matches.
    dfast_mean_ml: f32,
    /// GATE 8 @ L3: share of DFast's speculated loads that the next iteration
    /// actually consumed. Low = the pipeline is prefetching for positions the
    /// match logic then jumps past.
    dfast_spec_yield: f32,
    /// Countdown to the next forced DFast-pipeline re-probe.
    dfast_probe: u32,
    /// GATE 6 second variable: MATCH BYTES PER PROBE on the previous block --
    /// the pair search's exchange rate, benefit over cost in the units the cost
    /// is actually paid in. The pair search costs real probe time (+28.9% mean at L1), so it
    /// must be shown to be EARNING. Low gain = the extra probes find nothing:
    /// x-ray 0.0010 pays 19.8% time for 0.02% size; sao 0.0358 pays 61.8% for
    /// 1.82%. Winners sit an order of magnitude higher (nci 0.1875, mozilla
    /// 0.3105).
    pair_gain: f32,
    /// Countdown to the next forced pair RE-PROBE. Without it the gain term is a
    /// one-way latch: pair off => 0 bytes attributed => gain 0 => off forever,
    /// so content that changes phase mid-stream could never recover. On a
    /// rejected block the gain is RETAINED (not zeroed) and re-measured every
    /// `PAIR_PROBE_PERIOD` blocks.
    pair_probe: u32,
    /// GATE 6 ROUTE for this block: 0 = off, 1 = pipelined step-1, 2 = pair.
    /// The pair search and the step-1 loop probe the SAME positions, but step-1
    /// has a pipelined, HLOG-specialised body while the pair path forfeits
    /// pipelining entirely (`if PIPE && !pair`). On 13 of 16 corpora they tie on
    /// size and step-1 is ~7 points cheaper; on `nci` pair is twice as good
    /// (-12.97% vs -6.44%). Same work, opposite verdicts -- so it is routed.
    pair_route: u8,
    /// GATE 7: one tag byte per hash slot, in a SEPARATE array.
    ///
    /// The tag derives from the same 4 bytes as the hash, so equal words give
    /// equal tags. A mismatch therefore PROVES the words differ, and
    /// `fast_probe` would have rejected the candidate anyway -- making this
    /// filter byte-identical while skipping the random load of `src[m]`.
    ///
    /// Deterministically priced at L1: 22,056,552 of 42,109,297 candidates
    /// (52.4%) are rejectable this way, from 83.4% (x-ray) to 0.2% (versions).
    ///
    /// A separate array, NOT packed into the slot. The packed form truncated the
    /// position to 24 bits; it is gone (3a25bc7).
    tags: alloc::vec::Vec<u8>,
    /// T1: hold DFast's tag in the TOP 8 BITS of the short slot instead of in a
    /// second array.
    ///
    /// The separate-array form works -- byte-identical, and it rejects 29.8% of
    /// non-empty short slots -- but it does not PAY: it adds one tag store per
    /// position to avoid ~0.3 candidate loads per position, a second cache line
    /// touched every time round the loop. Packed, the tag costs nothing at all:
    /// same word, same load, same store.
    ///
    /// Sound only while `pos + 1` fits in 24 bits, so it is enabled per frame
    /// against the actual buffer length and never guessed.
    pack_tags: bool,
    /// ffanat hash-width: latched by the fast_lazy SWITCH, not by rep_yield.
    /// `find_lazy` reads this table with 4-byte `hash_mls` keys, so a wide
    /// frame that routes blocks to lazy would hand it a key-blind table (the
    /// residual +10.7% on versions after every probe-side protection). The
    /// switch is ground truth for rep-dominated frames: on its first fire the
    /// table is cleared once and the frame's key latches legacy, coherent for
    /// lazy and for every later Fast block.
    fast_hash_legacy: bool,
    /// Share of the PREVIOUS block's candidates the tag would have rejected --
    /// i.e. loads of `src[m]` it saves. Winners sit at 51-100%, the two losers
    /// at 34.3% (mr) and 12.5% (reymont), so the filter is worth its compare
    /// only above roughly half.
    tag_yield: f32,
    /// Consecutive blocks whose `rep_yield` cleared the Gate 1 @ L1 threshold.
    ///
    /// The bare threshold does NOT work, and the per-block data says so: `mr`
    /// has 6 blocks over 0.7 (max 0.809) and `x-ray` has 2 at exactly 1.000, so
    /// the corpus-MEAN gap [0.4949, 0.9778] was an averaging artefact and the
    /// per-block distributions overlap completely. Deployed on the mean, the
    /// gate regressed `mr` by +0.15%.
    ///
    /// The property actually wanted is "this FILE is repetitive", not "this
    /// block was", and RUN LENGTH separates them cleanly:
    ///     versions-16m 107   text-32m 255   zeros-32m 256
    ///     mr 1   x-ray 1   every other corpus 0
    /// A gap of [1, 107] -- a 100x margin against the threshold's zero.
    rep_run: u32,
}

impl MatchTables {
    pub(crate) fn new(params: CompressionParameters) -> Self {
        let hash_log = params.hash_log.clamp(6, 24);
        let hsz = 1usize << hash_log;
        let csz = 1usize << params.chain_log.min(24);
        // Report what is ACTUALLY allocated, not what the level table implies.
        // This reported all three tables at full size regardless of brick 47, so
        // `unused_long_chain=98304` kept appearing for allocations that no
        // longer exist -- an instrument describing the code as it was two
        // bricks ago.
        let use_long = matches!(params.strategy, Strategy::DFast);
        // A/B: does the tag array EARN its per-probe store? It is a SECOND
        // array, so every probe writes two cache lines instead of one, and the
        // write happens even on blocks where Gate 7's filter is off and nothing
        // reads it. Gate 7 is byte-identical, so this is purely a speed
        // question.
        // T1 note: DFast does NOT want this array. It carries its tag packed in
        // the slot it already loads, so allocating a second array here would
        // reintroduce exactly the per-position store that made the unpacked form
        // fail to pay.
        // ffanat: `new` no longer allocates the tag array. Packed frames (the
        // L1/L2 default, every frame < 16 MiB) never need it, and allocating
        // 1 << hash_log bytes of ZEROED memory per frame only to drop it at the
        // enable site was a pure memset tax. The array is now allocated at the
        // one place that knows whether packing applies (`encode_oneshot`) and,
        // for the streaming compressor, right after construction.
        let use_tags = false;
        let _ = tag_alloc_enabled;
        let use_chain = !matches!(params.strategy, Strategy::Fast | Strategy::DFast);
        let hash_b = (hsz as u64).saturating_mul(4);
        let long_b = if use_long { hash_b } else { 0 };
        let chain_b = if use_chain {
            (csz as u64).saturating_mul(4)
        } else {
            0
        };
        crate::prof::note_tables(hash_b, long_b, chain_b);
        // Only the Fast strategy reads these slots through `store_fast` /
        // `load_fast`; the chain strategies keep plain positions. The window
        // guard is what makes the modulo-2^24 reconstruction unambiguous.

        // BRICK 47: allocate ONLY the tables this strategy reads.
        //
        // `find_fast` touches neither `hash_long` nor `chain`; `find_dfast`
        // touches `hash_long` but not `chain`. We were allocating and zeroing
        // all three unconditionally, so L1 carried a 160 KiB table footprint
        // against C's 64 KiB -- 96 KiB of it never read. The profiler had been
        // printing the evidence as `unused_long_chain=98304` all along.
        //
        // Dead tables cannot affect the bitstream, so this is byte-identical by
        // construction. It pays where tables are built often rather than once:
        // per-entry CRDT blobs (a table set per small payload) and streaming,
        // where `reset()` memsets the whole set on every window slide.
        Self {
            rep_yield: 1.0,
            hash_log,
            hash: vec![0; hsz],
            hash_long: if use_long { vec![0; hsz] } else { Vec::new() },
            ltags: Vec::new(),
            chain: if use_chain { vec![0; csz] } else { Vec::new() },
            frame_start: 0,
            last_nseq: 0,
            step_pick: 0,
            step_used: 0,
            route_force: 0,
            step_probed: 0,
            step_sum1: 0.0,
            step_sum2: 0.0,
            step_reprobe: 0,
            // Start optimistic: the first block back-fills, then measures.
            last_search_per_byte: 1.0,
            walk_first_share: 0.0,
            walk_probe: 0,
            walk_share_meas: false,
            wide_ok_blocks: 0,
            chain_pack: false,
            ctags: Vec::new(),
            chain_wide: false,
            coded_scratch: Vec::new(),
            bits_scratch: Vec::new(),
            blocks_done: 0,
            payload_scratch: Vec::new(),
            seq_scratch: Vec::new(),
            lit_scratch: Vec::new(),
            opt_ops: Vec::new(),
            opt_price: Vec::new(),
            opt_prev: Vec::new(),
            opt_om: Vec::new(),
            rep_run: 0,
            next_long_yield: 1.0,
            nl_off_worse: 0.0,
            nl_band_meas: 0,
            nl_band_probe: 0,
            lit_short_share: 1.0,
            lit_mid_share: 0.0,
            rep_len_ratio: 1.0,
            rep_probe: 0,
            raw_run: 0,
            raw_probe: 0,
            opt_rep_rate: f32::MAX,
            opt_rep_probe: 0,
            opt_rep_peak: 0.0,
            opt_rep_meas: 0,
            opt_rep_seen: 0,
            opt_lit_price: 0,
            dfast_mean_ml: 0.0,
            dfast_spec_yield: 1.0,
            dfast_probe: 0,
            pair_gain: 1.0,
            pair_probe: 0,
            pair_route: 2,
            tags: if use_tags { alloc::vec![0u8; hsz] } else { alloc::vec::Vec::new() },
            pack_tags: false,
            fast_hash_legacy: false,
            tag_yield: 1.0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.tags.fill(0);
        self.hash.fill(0);
        self.hash_long.fill(0);
        self.ltags.fill(0);
        self.chain.fill(0);
        self.ctags.fill(0);
    }

    /// Store `pos + 1` so slot 0 stays "empty" (C window index never uses 0).
    /// Store a Fast-strategy slot (packed with its tag, or plain).
    #[inline(always)]
    #[allow(unsafe_code)]
    fn store_fast(&mut self, h: usize, pos: usize, tag: u8) {
        // BRICK 50 -- SAFETY: `h` always arrives from `hash4_tag`, which returns
        // `(hv >> hash_shift) as usize & hash_mask`, and `hash_mask` is
        // `self.hash.len() - 1` where the length is `1 << hash_log` (a non-zero
        // power of two, allocated in `new`). A value masked by `len - 1` is
        // therefore always `< len`. LLVM cannot see this because `hash_mask`
        // spills to the stack, so it emitted a bounds check AND a reload of
        // `hash.len()` on EVERY probe -- 2 of the 6 stack accesses left in the
        // hot loop. The debug build still checks it.
        debug_assert!(h < self.hash.len());
        // Written UNCONDITIONALLY whenever the array exists. Gating the STORE
        // on the same flag as the compare is what lets tags go stale, which is
        // the defect class that cost this gate a day (190ad8b).
        // ffanat 5a: the packed form is LIVE when `pack_tags` is set for a Fast
        // frame (< 16 MiB, proven by `enable_packed_tags`). The historical
        // refutation of this representation was real but misattributed -- see
        // the forward-mirror comment in the pipelined loop -- and its one
        // structural hazard, the mid-frame Fast->Lazy shared table, is handled
        // by unpacking at the switch. With it on, the separate `tags` array is
        // dropped entirely: one random line loaded and one stored per probe
        // instead of two of each.
        //
        // The packed branch RETURNS EARLY so the `tags.get_mut` length check
        // below never runs on packed frames -- the array is empty there, and a
        // per-position len-load + compare + branch against an empty Vec is pure
        // waste in the hottest store in the encoder. The 190ad8b rule ("written
        // unconditionally whenever the array exists") still holds in the
        // else-path: packing removes the array, it does not gate the store.
        if self.pack_tags {
            *unsafe { self.hash.get_unchecked_mut(h) } =
                (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        if let Some(t) = self.tags.get_mut(h) {
            *t = tag;
        }
        *unsafe { self.hash.get_unchecked_mut(h) } = {
            // BRICK 57: `wrapping_add`, not `saturating_add`. The slot holds
            // `pos + 1` with 0 meaning "empty". The only input that differs is
            // `pos as u32 == u32::MAX`, where saturating stored `u32::MAX` --
            // which `fast_probe` then turns into the BOGUS candidate
            // `m = u32::MAX - 1`, relying on `match_ok` to reject it. Wrapping
            // stores 0 instead, i.e. "empty" = a cleanly missed match, so this
            // is the safer semantic as well as the cheaper one.
            //
            // Cheaper because saturating needs a `cmovel` against a register
            // held at -1 for the whole loop; dropping it frees that register
            // for the src base pointer, the last stack reload in the probe.
            (pos as u32).wrapping_add(1)
        };
    }


    /// Raw slot, bypassing the tag filter -- diagnostic only.
    #[inline(always)]
    fn raw_fast(&self, h: usize) -> u32 {
        let e = self.hash[h];
        if self.pack_tags { e & 0x00FF_FFFF } else { e }
    }

    /// HAZARD (recorded 2026-08-18, not yet fixed): this writes `hash[h]` and
    /// leaves `tags[h]` UNTOUCHED, so on the Fast ladder it installs a new
    /// position under the PREVIOUS position's tag. Gate 7's filter would then
    /// reject a valid candidate at that slot -- the 190ad8b defect class again,
    /// one level up.
    ///
    /// It is not reachable today: the only Fast-path caller is the
    /// dictionary/prefix prefill, which returns early when `payload_off == 0`
    /// (every benchmark and test here). It also hashes with `hash_mls`, not
    /// `hash4_tag`, so its slots may not even correspond. Fixing it needs a
    /// dictionary/prefix test first -- do not "fix" it blind.
    #[inline(always)]
    // T2 -- SAFETY, and it is the SAME invariant brick 50 proved for the Fast
    // path. Every index into `hash`/`hash_long` is produced by `hash4`/`hash8`
    // /`hash4_tag` shifting down to `tables.hash_log` bits, and the tables are
    // allocated `1 << tables.hash_log`. That the log is the TABLE's and never
    // `params.hash_log` is already load-bearing here: `params.hash_log` is
    // user-settable with no upper bound, which is why `prime_tables` and every
    // finder bind `let hash_log = tables.hash_log`.
    //
    // LLVM cannot see it, so it emitted a bounds check and a branch on EVERY
    // table access -- twice per position, short table and long. The Fast finder
    // carries 0 panic sites for this reason; DFast carried 13.
    #[allow(unsafe_code)]
    fn put_h(&mut self, h: usize, pos: usize) {
        debug_assert!(h < self.hash.len());
        *unsafe { self.hash.get_unchecked_mut(h) } = (pos as u32).saturating_add(1);
    }

    /// T1: `put_h` that also writes the tag, so the short table obeys the same
    /// rule `store_fast` does -- the tag is written UNCONDITIONALLY whenever the
    /// array exists. Gating the store on the same flag as the compare is what
    /// lets tags go stale (190ad8b, and again in `prime_tables`).
    #[inline(always)]
    #[allow(unsafe_code)]
    fn put_h_tag(&mut self, h: usize, pos: usize, tag: u8) {
        if self.pack_tags {
            // `pos + 1` is guaranteed < 2^24 by `enable_packed_tags`, so the
            // mask cannot truncate a live position, and the low bits are never
            // 0 -- an all-zero word still means "empty".
            debug_assert!(h < self.hash.len());
            *unsafe { self.hash.get_unchecked_mut(h) } =
                (((pos as u32).saturating_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        if let Some(t) = self.tags.get_mut(h) {
            *t = tag;
        }
        debug_assert!(h < self.hash.len());
        *unsafe { self.hash.get_unchecked_mut(h) } = (pos as u32).saturating_add(1);
    }

    /// Allocate the array form of the Fast tag filter (for callers with no
    /// frame length to prove the packed bound -- the streaming compressor).
    pub(crate) fn alloc_fast_tags(&mut self, params: CompressionParameters) {
        // TAG AUDIT hole #2 closed: streaming DFast now gets the array form
        // too (this was Fast-only, leaving `dtag_on` false for every
        // streaming DFast frame).
        if ((params.strategy == Strategy::Fast && tag_alloc_enabled())
            || (params.strategy == Strategy::DFast && dfast_tag_enabled()))
            && self.tags.is_empty()
        {
            self.tags = alloc::vec![0u8; self.hash.len()];
        }
        // 1a array route, streaming leg.
        if params.strategy == Strategy::DFast
            && dfast_tag_enabled()
            && long_tag_enabled()
            && !self.hash_long.is_empty()
            && self.ltags.is_empty()
        {
            self.ltags = alloc::vec![0u8; self.hash_long.len()];
        }
        // Chain-link tag, streaming leg (no length proof -> array route).
        if matches!(
            params.strategy,
            Strategy::Greedy | Strategy::Lazy | Strategy::Lazy2
        ) && chain_tag_enabled()
            && (params.min_match.max(3) as usize) < 8
            && !self.chain.is_empty()
        {
            if self.ctags.is_empty() {
                self.ctags = alloc::vec![0u8; self.chain.len()];
            }
            if self.tags.is_empty() {
                self.tags = alloc::vec![0u8; self.hash.len()];
            }
        }
    }

    /// Enable packed tags for this frame, but only when every position the
    /// finder can store fits in the 24 bits the representation leaves.
    /// `len` must be the length of the buffer the finder indexes into.
    #[inline]
    fn enable_packed_tags(&mut self, on: bool, len: usize) {
        self.pack_tags = on && len < 0x00FF_FFFF;
    }

    /// T1: short-table load with the DFast rejection filter.
    ///
    /// The tag derives from the same 4 bytes as the index, and DFast's
    /// `min_match` is 5, so any match it could accept implies 4 equal bytes and
    /// therefore an equal tag. A mismatch provably cannot hide a match, which is
    /// why this is byte-identical rather than a size-for-speed trade.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn get_h_tag(&self, h: usize, tag: u8, on: bool) -> Option<usize> {
        debug_assert!(h < self.hash.len());
        let v = *unsafe { self.hash.get_unchecked(h) };
        if v == 0 {
            return None;
        }
        if self.pack_tags {
            if (v >> 24) as u8 != tag {
                return None;
            }
            return Some(((v & 0x00FF_FFFF) as usize) - 1);
        }
        if on {
            if let Some(&t) = self.tags.get(h) {
                if t != tag {
                    return None;
                }
            }
        }
        Some((v as usize) - 1)
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn get_h(&self, h: usize) -> Option<usize> {
        debug_assert!(h < self.hash.len());
        let v = *unsafe { self.hash.get_unchecked(h) };
        if v == 0 {
            None
        } else {
            Some((v as usize) - 1)
        }
    }

    /// Chain-link tag helpers (see `set_chain_tag_arm`). Head format under
    /// `chain_pack`: `(pos+1) | tag << 24` (0 = empty, pos+1 < 2^24 by the
    /// frame guard); link format: `pos | tag << 24` (low 24 = 0 keeps the
    /// historical none-sentinel semantics).
    #[inline(always)]
    #[allow(unsafe_code)]
    fn lz_head_raw(&self, h: usize) -> u32 {
        debug_assert!(h < self.hash.len());
        *unsafe { self.hash.get_unchecked(h) }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn lz_head_put(&mut self, h: usize, pos: usize, tag: u8, cp: bool) {
        debug_assert!(h < self.hash.len());
        let v = if cp {
            (((pos as u32).saturating_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24)
        } else {
            (pos as u32).saturating_add(1)
        };
        *unsafe { self.hash.get_unchecked_mut(h) } = v;
    }

    #[inline(always)]
    fn lz_head_pos(raw: u32, cp: bool) -> Option<usize> {
        let p = if cp { raw & 0x00FF_FFFF } else { raw };
        if p == 0 { None } else { Some((p as usize) - 1) }
    }

    #[inline(always)]
    fn lz_head_tag(raw: u32) -> u8 {
        (raw >> 24) as u8
    }

    /// The link stored for a new position is the OLD head, re-encoded from
    /// `(pos+1) | tag<<24` to `pos | tag<<24` (empty stays 0).
    #[inline(always)]
    fn lz_link_from_head(raw: u32, cp: bool) -> u32 {
        if cp {
            let p = raw & 0x00FF_FFFF;
            if p == 0 { 0 } else { (p - 1) | (raw & 0xFF00_0000) }
        } else if raw == 0 {
            0
        } else {
            raw - 1
        }
    }

    /// Brick 50 for the chain arrays: `i` always arrives masked by
    /// `chain.len() - 1` (a power of two), so it is provably in bounds --
    /// LLVM cannot see it because the mask spills, and emitted a bounds
    /// check plus a panic branch on EVERY walk step and insert.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn chain_masked(&self, i: usize) -> u32 {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked(i) }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn chain_masked_set(&mut self, i: usize, v: u32) {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked_mut(i) } = v;
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    fn ctags_masked(&self, i: usize) -> u8 {
        debug_assert!(i < self.ctags.len());
        *unsafe { self.ctags.get_unchecked(i) }
    }

    /// Lazy-ladder insert, all representations: writes the chain link (and
    /// its tag, packed or array), the head (and its tag), and returns the
    /// OLD head's (pos, tag) for the walk. `ca` = array route.
    #[inline(always)]
    fn lz_insert(
        &mut self,
        h: usize,
        ip: usize,
        gtag: u8,
        cp: bool,
        ca: bool,
        chain_mask: usize,
    ) -> (Option<usize>, u8) {
        let raw = self.lz_head_raw(h);
        // `h < hash.len()` by the hash shift (brick 50/52); `tags` and
        // `ctags`, when allocated, share `hash`/`chain` lengths by
        // construction.
        let old_tag = if cp {
            Self::lz_head_tag(raw)
        } else if ca {
            debug_assert!(h < self.tags.len());
            #[allow(unsafe_code)]
            *unsafe { self.tags.get_unchecked(h) }
        } else {
            0
        };
        self.chain_masked_set(ip & chain_mask, Self::lz_link_from_head(raw, cp));
        if ca {
            debug_assert!((ip & chain_mask) < self.ctags.len() && h < self.tags.len());
            #[allow(unsafe_code)]
            unsafe {
                *self.ctags.get_unchecked_mut(ip & chain_mask) = old_tag;
                *self.tags.get_unchecked_mut(h) = gtag;
            }
        }
        self.lz_head_put(h, ip, gtag, cp);
        (Self::lz_head_pos(raw, cp), old_tag)
    }

    /// T2: binary-tree slot read.
    ///
    /// SAFETY: every caller indexes `(x & bt_mask) << 1` or that `+ 1`, and
    /// `bt_find_best` returns early unless `(bt_mask << 1) | 1 < chain.len()`,
    /// which bounds the LARGEST index the tree can form. That worst-case guard
    /// replaced a per-`ip` one that could not prove the loop's own accesses --
    /// `bt_idx` is formed from `m`, not `ip`.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn chain_at(&self, i: usize) -> u32 {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked(i) }
    }

    /// T2: binary-tree slot write. Same invariant as `chain_at`.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn chain_set(&mut self, i: usize, v: u32) {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked_mut(i) } = v;
    }

    #[allow(unsafe_code)]
    fn put_hl(&mut self, h: usize, pos: usize) {
        debug_assert!(h < self.hash_long.len());
        *unsafe { self.hash_long.get_unchecked_mut(h) } = (pos as u32).saturating_add(1);
    }


    /// 1a: long-table stores carry the SHORT tag (`hash4_tag`'s byte) in the
    /// high 8 bits on packed frames -- the same representation and < 16 MiB
    /// bound as `put_h_tag`, and the tag costs NOTHING new: it is a function
    /// of the first 4 bytes and is already computed at every store site for
    /// the short table. Every long-candidate acceptance verifies at least 4
    /// leading bytes (`match_ok` with `max(4, ..)`), so a mismatch provably
    /// cannot hide a match. Representation follows `pack_tags`
    /// unconditionally (the 190ad8b rule); the arm gates only the COMPARE.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn put_hl_tag(&mut self, h: usize, pos: usize, tag: u8) {
        debug_assert!(h < self.hash_long.len());
        if self.pack_tags {
            *unsafe { self.hash_long.get_unchecked_mut(h) } =
                (((pos as u32).saturating_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        // Array route (>= 16 MiB / streaming): written UNCONDITIONALLY
        // whenever the array exists -- gating the store on the compare's flag
        // is what lets tags go stale (190ad8b).
        if let Some(t) = self.ltags.get_mut(h) {
            *t = tag;
        }
        *unsafe { self.hash_long.get_unchecked_mut(h) } = (pos as u32).saturating_add(1);
    }

    /// 1a: tag-filtered long-table load. `on` gates the compare only; the
    /// unmask under `pack_tags` is unconditional, because the slot holds the
    /// packed form whenever the frame does.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn get_hl_tag(&self, h: usize, tag: u8, on: bool) -> Option<usize> {
        debug_assert!(h < self.hash_long.len());
        let v = *unsafe { self.hash_long.get_unchecked(h) };
        if v == 0 {
            return None;
        }
        if self.pack_tags {
            if on && (v >> 24) as u8 != tag {
                return None;
            }
            return Some(((v & 0x00FF_FFFF) as usize) - 1);
        }
        if on {
            if let Some(&t) = self.ltags.get(h) {
                if t != tag {
                    return None;
                }
            }
        }
        Some((v as usize) - 1)
    }

    /// Diagnostic twin: the raw long-slot position with the mask honored
    /// (COUNT paths only -- the false-reject re-probe).
    #[cfg(feature = "profile")]
    #[inline(always)]
    fn raw_hl(&self, h: usize) -> u32 {
        let e = self.hash_long[h];
        if self.pack_tags { e & 0x00FF_FFFF } else { e }
    }
}

/// Huffman / FSE tables carried across compressed blocks (Repeat / Treeless).
#[derive(Clone, Default)]
pub(crate) struct EntropyState {
    huff: Option<HuffCTable>,
    ll: Option<FseCTable>,
    of: Option<FseCTable>,
    ml: Option<FseCTable>,
}

impl EntropyState {
    pub(crate) fn seed_from_dict(&mut self, e: &crate::dict::DictEntropy) {
        self.huff = Some(e.huff_c.clone());
        self.ll = Some(e.ll_c.clone());
        self.of = Some(e.of_c.clone());
        self.ml = Some(e.ml_c.clone());
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_oneshot(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    pledged: Option<u64>,
    dict: Option<&Dictionary>,
    prefix: &[u8],
    write_dict_id: bool,
    adv: AdvancedOptions,
) -> Result<Vec<u8>, Error> {
    let _enc = crate::prof::scope(crate::prof::Stage::EncodeTotal);
    let hist_prefix = dict.map(Dictionary::content).unwrap_or(prefix);
    let dict_id = if write_dict_id {
        dict.map(Dictionary::id).filter(|&id| id != 0)
    } else {
        None
    };
    let mut tables = {
        let _t = crate::prof::scope(crate::prof::Stage::EncodeTables);
        MatchTables::new(params)
    };
    // T1: DFast's short-table rejection tag, packed into the slot it already
    // loads. Decided against the real buffer length, so the 24-bit bound is
    // proven per frame rather than assumed.
    tables.enable_packed_tags(
        (params.strategy == Strategy::DFast && dfast_tag_enabled())
            || (params.strategy == Strategy::Fast
                && tag_alloc_enabled()
                && fast_pack_enabled()),
        hist_prefix.len() + src.len(),
    );
    if !tables.pack_tags
        && ((params.strategy == Strategy::Fast && tag_alloc_enabled())
            || (params.strategy == Strategy::DFast && dfast_tag_enabled()))
    {
        // Non-packed frames (>= 16 MiB) still carry the array form of the
        // tag filter; `new` no longer allocates it, so this is the one site
        // that does. Packed frames never allocate it at all -- previously it
        // was built zeroed here-ish and dropped, a per-frame memset for
        // nothing.
        //
        // TAG AUDIT hole #1 closed: this fallback was Fast-only, so DFast
        // frames >= 16 MiB ran with dfast_tag ON and NO filter at all --
        // `dtag_on` silently false. The writers already honor the array
        // representation unconditionally (190ad8b), so routing the
        // allocation is the whole fix; byte-identity follows from the T1
        // proof (the tag derives from the same 4 bytes as the index, and a
        // real match implies an equal tag). Priced on the T1 instrument by
        // `tagbig`: see the commit.
        tables.tags = alloc::vec![0u8; tables.hash.len()];
    }
    // Chain-link tag: lazy strategies only (Bt shares the chain array as
    // TREE NODES and must never see tag bits), same < 16 MiB bound.
    tables.chain_pack = matches!(
        params.strategy,
        Strategy::Greedy | Strategy::Lazy | Strategy::Lazy2
    ) && chain_tag_enabled()
        && (params.min_match.max(3) as usize) < 8
        && (hist_prefix.len() + src.len()) < 0x00FF_FFFF;
    // chain_wide is decided by the MID-FRAME LATCH in the walk finders (see
    // `maybe_latch_wide_chain`): frames start narrow, and only content whose
    // measured walk_first_share says the deeper effective search PAYS gets
    // the wide key -- smallmsg-class content (first-find dominated, prefers
    // its literal+rep economy) never latches. Frame init only resets it.
    tables.chain_wide = false;
    // Array route where the 24-bit proof fails (>= 16 MiB): link tags in
    // `ctags`, head tags in `tags` (same hash index). Priced by `linkbig`.
    if matches!(
        params.strategy,
        Strategy::Greedy | Strategy::Lazy | Strategy::Lazy2
    ) && chain_tag_enabled()
        && (params.min_match.max(3) as usize) < 8
        && !tables.chain_pack
        && !tables.chain.is_empty()
    {
        if tables.ctags.is_empty() {
            tables.ctags = alloc::vec![0u8; tables.chain.len()];
        }
        if tables.tags.is_empty() {
            tables.tags = alloc::vec![0u8; tables.hash.len()];
        }
    }
    // 1a array route: the LONG table's filter for the same frames. Priced by
    // `ltagbig` on the same instrument.
    if params.strategy == Strategy::DFast
        && dfast_tag_enabled()
        && long_tag_enabled()
        && !tables.pack_tags
        && !tables.hash_long.is_empty()
        && tables.ltags.is_empty()
    {
        tables.ltags = alloc::vec![0u8; tables.hash_long.len()];
    }
    let mut reps = [1u32, 4, 8];
    let mut entropy = EntropyState::default();
    if let Some(d) = dict {
        if let Some(e) = d.entropy() {
            entropy.seed_from_dict(e);
            reps = e.reps;
        }
    }
    let mut out = Vec::with_capacity(crate::compress_bound(src.len()));
    write_frame_header(
        &mut out,
        src.len() as u64,
        params.window_log,
        checksum,
        pledged,
        dict_id,
        !hist_prefix.is_empty() && !adv.prime_only,
    );
    if src.is_empty() {
        write_block_header(&mut out, true, BlockType::Raw, 0);
        if checksum {
            out.extend_from_slice(&content_checksum(src).to_le_bytes());
        }
        return Ok(out);
    }
    let window = 1usize << params.window_log.min(31);
    let mut block_max = (window.min(BLOCKSIZE_MAX as usize)).max(1);
    // EXPERIMENT ONLY (RZSTD_BLOCK_KB): C emits ~84 KiB regen blocks on mozilla
    // where we emit 128 KiB, so it re-adapts its entropy tables ~1.56x more
    // often. This knob tests whether that explains our literals gap. Ratio is
    // deterministic, so the answer needs no quiet box.
    if let Ok(v) = std::env::var("RZSTD_BLOCK_KB") {
        if let Ok(kb) = v.trim().parse::<usize>() {
            if kb > 0 {
                block_max = block_max.min(kb * 1024);
            }
        }
    }
    if adv.target_cblock_size > 0 {
        let t = adv.target_cblock_size as usize;
        block_max = block_max.min(t.saturating_mul(4).max(256));
    }
    let ldm_res = if adv.ldm.enable {
        Some(adv.ldm.resolved(params.window_log))
    } else {
        None
    };
    let mut ldm_tables = ldm_res.map(crate::ldm::LdmTables::new);
    let mut owned = Vec::new();
    let (workspace, payload_off): (&[u8], usize) = if hist_prefix.is_empty() {
        (src, 0)
    } else {
        // GATE 2 @ L3: copy only the reachable tail of the prefix.
        //
        // This used to copy the WHOLE prefix, however large. Nothing below
        // `window + BLOCKSIZE_MAX` can ever be referenced, and the bound is
        // provable rather than fitted:
        //   * every finder rejects a candidate at `ip - m > window`, and
        //     `lowest` is floored at `block_start - window`; and
        //   * `back_extend` walks down at most `ip - anchor`, and `anchor` never
        //     precedes `block_start`, so the walk cannot reach further than one
        //     block below that floor.
        // So the deepest byte any match can touch is `window + BLOCKSIZE_MAX`
        // before the payload, and everything under it is copied for nothing.
        //
        // `--patch-from` against a large reference is the case that pays: with a
        // 4 MiB reference and a 1 MiB payload at L3 this is BYTE-IDENTICAL on
        // 18/18 corpora and measurably faster on 17/18 (up to -21.9%).
        // `prime_tables` was ALREADY window-bounded -- the deterministic
        // `take_prime_iters()` counter is unchanged across the two arms -- so the
        // win is the `memcpy` alone, not the priming.
        let keep = window.saturating_add(BLOCKSIZE_MAX as usize);
        let cut = if prefix_bound_enabled() {
            hist_prefix.len().saturating_sub(keep)
        } else {
            0
        };
        let hp = &hist_prefix[cut..];
        owned.reserve(hp.len() + src.len());
        owned.extend_from_slice(hp);
        owned.extend_from_slice(src);
        (owned.as_slice(), hp.len())
    };
    if adv.prime_only {
        tables.frame_start = payload_off;
    }
    prime_tables(&mut tables, workspace, payload_off, window, params);
    if let (Some(lt), Some(rp)) = (ldm_tables.as_mut(), ldm_res) {
        crate::ldm::prime_ldm(lt, workspace, payload_off, window, rp);
    }
    let rbits = if adv.rsyncable {
        crate::ldm::rsync_bits(params.window_log)
    } else {
        0
    };
    let mut off = payload_off;
    let mut xxh = if checksum { Some(Xxh64::new()) } else { None };
    {
        let _b = crate::prof::scope(crate::prof::Stage::EncodeBlocks);
        let mut r_prev: f32 = -1.0;
        let mut r_prev2: f32 = -1.0;
        while off < workspace.len() {
            let bmax = adaptive_block_max(
                block_max,
                r_prev,
                r_prev2,
                tables.rep_yield,
                params.strategy,
                workspace.len(),
            );
            let mut end = (off + bmax).min(workspace.len());
            if adv.rsyncable && end > off + 64 {
                if let Some(cut) = crate::ldm::rsync_cut(&workspace[off..end], rbits) {
                    if cut > 32 && off + cut < workspace.len() {
                        end = off + cut;
                    }
                }
            }
            let last = end == workspace.len();
            let before_block = out.len();
            encode_block(
                &mut out,
                workspace,
                off,
                end,
                window,
                params,
                &mut tables,
                &mut reps,
                &mut entropy,
                last,
                ldm_tables.as_mut(),
                adv.ldm,
            )?;
            if let Some(h) = xxh.as_mut() {
                h.update(&workspace[off..end]);
            }
            // feed this block's own outcome forward -- free, it is already known
            let produced = out.len() - before_block;
            r_prev2 = r_prev;
            r_prev = produced as f32 / (end - off).max(1) as f32;
            off = end;
        }
    }
    if let Some(h) = xxh {
        let _c = crate::prof::scope(crate::prof::Stage::EncodeChecksum);
        crate::prof::note_checksum_bytes(src.len() as u64);
        out.extend_from_slice(&(h.digest() as u32).to_le_bytes());
    }
    Ok(out)
}

/// GATE 1 @ L19 -- the Bt tree is primed in the WRONG LAYOUT.
///
/// `prime_tables` writes `chain[p & chain_mask]`, i.e. the linked-chain form:
/// one slot per position, "previous position with this hash". That is correct
/// for `Greedy`/`Lazy`/`Lazy2` (L5-L12), which read it back through
/// `chain_find_best`.
///
/// From `BtLazy2` up (L13-L22) the SAME array is a BINARY TREE. `bt_find_best`
/// addresses it as `(m & bt_mask) << 1` with `bt_log = chain_log - 1`, i.e. TWO
/// slots per position holding that node's smaller/larger children. Priming a
/// prefix at those levels therefore scatters chain-format links across tree
/// nodes at unrelated indices.
///
/// It is not a correctness bug -- a bogus candidate either fails the
/// `m < bt_lowest` / `ip - m > window` guards or is rejected by `count_match`,
/// so the output stays valid. It is a QUALITY and SPEED bug: the descent starts
/// from garbage instead of from an empty tree.
///
/// This is reached whenever a prefix or dictionary is present, which for MT is
/// EVERY job after the first -- and at L19 the overlap is the whole 8 MiB
/// window, so it is 8 MiB of per-byte work per job, seeding noise.
///
/// MEASURED: skipping it is BYTE-IDENTICAL on 18 corpora x L13/L19/L22 (54
/// cells, 0 changed), so the write is provably DEAD on the Bt ladder -- the
/// values land at indices the tree never reads as links. It is also strictly
/// less work: 12 of 18 faster by >1% at L13, 7 of 18 at L22, and up to -28.8%
/// where the priming loop dominates (`zeros-32m`, `text-32m`).
///
/// Default is now SKIP. `RZSTD_PRIME_BT=1` (or `set_prime_bt_arm(true)`) restores
/// the old write -- that is the byte-identical fallback the ledger requires.
///
/// 0 = unresolved, 1 = skip the chain write on Bt strategies, 2 = keep it.
static PRIME_BT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the Gate 1 @ L19 A/B. `true` keeps the current (chain-format)
/// write on the Bt ladder; `false` skips it.
pub fn set_prime_bt_arm(keep: bool) {
    PRIME_BT_ARM.store(u8::from(keep) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prime_bt_chain_write() -> bool {
    match PRIME_BT_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            #[cfg(feature = "std")]
            {
                let keep = std::env::var("RZSTD_PRIME_BT")
                    .map(|v| v.trim() == "1")
                    .unwrap_or(false);
                PRIME_BT_ARM.store(u8::from(keep) + 1, core::sync::atomic::Ordering::Relaxed);
                keep
            }
            #[cfg(not(feature = "std"))]
            false
        }
    }
}

/// GATE 2 @ L3 -- how DENSELY a dictionary/prefix is primed into the tables.
///
/// `prime_tables` inserts EVERY position of the last `window` bytes of the
/// prefix, one at a time, with both the short and (for DFast) the long hash.
/// libzstd has a sparse counterpart for exactly this: `ZSTD_dtlm_fast` vs
/// `ZSTD_dtlm_full` in `ZSTD_fillDoubleHashTable`. We only implement the full
/// walk, so a `--patch-from` against a large reference pays a dense insert over
/// the whole window before a single byte of payload is searched.
///
/// Striding is NOT byte-identical -- it changes which positions are findable --
/// so it is a size-for-speed dispatch, not a free win. 0 = unresolved, else
/// stride + 1.
static PRIME_STRIDE_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Bench hook for the Gate 2 @ L3 stride sweep. 1 = every position (shipped).
pub fn set_prime_stride_arm(n: usize) {
    PRIME_STRIDE_ARM.store(n.max(1) as u32 + 1, core::sync::atomic::Ordering::Relaxed);
}

/// Deterministic work counter for the priming loop: positions inserted.
/// Accumulated LOCALLY and published once per call -- an atomic inside the loop
/// is the bricks 49/64/77 defect this campaign keeps finding.
pub static PRIME_ITERS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the priming work counter.
pub fn take_prime_iters() -> u64 {
    PRIME_ITERS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

#[inline]
fn prime_stride() -> usize {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let v = PRIME_STRIDE_ARM.load(Ordering::Relaxed);
        if v != 0 {
            return (v - 1) as usize;
        }
        let n: usize = std::env::var("RZSTD_PRIME_STRIDE")
            .ok()
            .and_then(|x| x.trim().parse().ok())
            .filter(|x| *x >= 1)
            .unwrap_or(1);
        PRIME_STRIDE_ARM.store(n as u32 + 1, Ordering::Relaxed);
        n
    }
    #[cfg(not(feature = "std"))]
    1
}

/// GATE 2 fallback arm: the window-bounded prefix copy.
///
/// The Great Gate form requires every shipped constant to have a proven
/// byte-identical OFF. `false` restores the old behaviour -- copy the WHOLE
/// prefix however large -- so the two can be A/B'd in one process instead of
/// across two binaries.
///
/// 0 = unresolved, 1 = copy everything (old), 2 = bound it (shipped).
static PREFIX_BOUND_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` copies the entire prefix, as the encoder used to.
pub fn set_prefix_bound_arm(bound: bool) {
    PREFIX_BOUND_ARM.store(u8::from(bound) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prefix_bound_enabled() -> bool {
    match PREFIX_BOUND_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        _ => true,
    }
}

/// FINDING 2 (Gate 2 @ L19): build the BINARY TREE over the prefix.
///
/// `prime_tables` wrote hash HEADS only. From `BtLazy2` up the finder descends a
/// binary tree held in `chain`, and priming never wrote a node -- so the first
/// descent from a primed head read an unseeded child and stopped. The prefix
/// contributed at most one candidate per bucket, with no tree behind it.
///
/// libzstd does the opposite: `ZSTD_loadDictionaryContent` calls
/// `ZSTD_updateTree` for btlazy2/btopt/btultra/btultra2, under the comment
/// "we want the dictionary table fully sorted".
///
/// `bt_find_best` inserts `ip` into the tree as a side effect (and maintains the
/// hash head itself), so walking the prefix through it is our `ZSTD_updateTree`.
///
/// **DEFAULT OFF.** It is a SIZE capability bought with TIME, and this campaign's
/// objective is the reverse: we are at size parity and hunting speed. The whole
/// curve was measured at L19 over a 4 MiB reference (15 corpora), and **no point
/// on it is both smaller and faster**:
///
///   arm         size      time
///   both OFF    0.000%    0.0%     <- shipped
///   s1/d5      -3.784%  +246.0%
///   s2/d5      -2.149%  +172.3%
///   s4/d5      -1.213%   +86.4%
///   s8/d5      -0.545%   +68.0%
///   s8/d3      -0.110%   +46.3%
///
/// Enable with `RZSTD_PRIME_BT_TREE=1` / `set_prime_bt_tree_arm(true)` when
/// dictionary RATIO matters more than dictionary load time -- with FINDING 1 it
/// is -3.78% on 15 of 15 corpora and moves `us/c` from 1.0880 to 1.0468.
///
/// GATE 2 FINDING 2, third cost axis: how MUCH of the prefix gets a tree.
///
/// Stride and depth were swept; EXTENT was not. Matches favour recent history,
/// so the tree's value is not uniform over the window: the bytes nearest the
/// payload are searched first and matched most. This builds the tree only over
/// the last `range / extent` bytes and leaves hash heads below it.
///
/// MEASURED at L19, per corpus, best-of-5, against heads-only priming:
///
///   extent   size      time     bigger   slower
///   1/1     -3.78%    +241%       0        15
///   1/16    -1.53%     +40%       0        14
///   1/32    -1.13%     +34%       0        15
///   1/64    -0.83%     +19%       0        15
///
/// NO POINT IS FREE -- every extent buys size with time, and an aggregate run
/// that appeared to show 1/16 both smaller AND faster was an artifact of arm
/// ordering; per corpus at best-of-5 it is slower on 14 of 15.
///
/// Extent is nonetheless the best of the three cost dials: it keeps 40% of the
/// full win for ~17% of the cost, where stride 4 kept only 0.045% of 1.78%.
/// So the capability DEFAULTS to 1/16 when it is switched on, and the tree
/// itself stays off.
///
/// 1 = the whole primed range; N = the last 1/N.
static PRIME_BT_EXTENT_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(16);

/// Bench hook for the extent sweep. 1 = tree over the whole primed range.
pub fn set_prime_bt_extent_arm(n: u32) {
    PRIME_BT_EXTENT_ARM.store(n.max(1), core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prime_bt_extent() -> usize {
    PRIME_BT_EXTENT_ARM.load(core::sync::atomic::Ordering::Relaxed).max(1) as usize
}

/// 0 = unresolved, 1 = heads only (shipped), 2 = build the tree.
static PRIME_BT_TREE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores heads-only priming.
pub fn set_prime_bt_tree_arm(build: bool) {
    PRIME_BT_TREE_ARM.store(u8::from(build) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prime_bt_tree_enabled() -> bool {
    match PRIME_BT_TREE_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            #[cfg(feature = "std")]
            {
                let on = std::env::var("RZSTD_PRIME_BT_TREE")
                    .map(|v| v.trim() == "1")
                    .unwrap_or(false);
                PRIME_BT_TREE_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
                on
            }
            #[cfg(not(feature = "std"))]
            true
        }
    }
}

/// FINDING 2 cost dial: how deep the PRIMING tree-insert descends. `0` = use the
/// level's own `search_log` (full depth, what a real search does).
///
/// DEFAULT 5, measured. The cost is linear in depth; the benefit saturates. At
/// L19 over a 4 MiB reference, against heads-only priming:
///
///   depth   size      time
///   full   -1.7779%  +60.3%
///   d5     -1.7732%  +35.9%   <- 99.7% of the win for 60% of the cost
///   d4     -1.6533%  +44.7%
///   d3     -1.3535%  +43.8%
///   d1     -0.5283%  +33.4%
///
/// Striding the insert was tried FIRST and refused: it moves along the same
/// line instead of off it (stride 4 keeps 0.045% of the 1.78%, stride 8 is
/// WORSE than not building the tree at all).
static PRIME_BT_DEPTH_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// See the table above. `set_prime_bt_depth_arm(0)` restores full depth.
const PRIME_BT_DEPTH_DEFAULT: u32 = 5;

/// Bench hook for the priming-depth sweep.
pub fn set_prime_bt_depth_arm(d: u32) {
    PRIME_BT_DEPTH_ARM.store(d, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn prime_bt_depth() -> u32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let v = PRIME_BT_DEPTH_ARM.load(Ordering::Relaxed);
        if v != u32::MAX {
            return v;
        }
        let d: u32 = std::env::var("RZSTD_PRIME_BT_DEPTH")
            .ok()
            .and_then(|x| x.trim().parse().ok())
            .unwrap_or(PRIME_BT_DEPTH_DEFAULT);
        PRIME_BT_DEPTH_ARM.store(d, Ordering::Relaxed);
        d
    }
    #[cfg(not(feature = "std"))]
    PRIME_BT_DEPTH_DEFAULT
}

// REFUTED AND REVERTED -- priming prefetch.
//
// Priming occupies 12.5% of the prefix path at L1, 16.2% at L3 and 3.8% at L19,
// and the loop runs at ~1.5 ns per primed position (about four cycles) doing a
// multiply, a shift and a RANDOM store into a 1-4 MiB table. That store misses,
// so prefetching its slot 16 positions ahead looked free.
//
// It is not. Measured byte-identical on 15/15 (as a prefetch must be) and
// SLOWER: +3.54% at L3 (10 of 15 corpora slower) and +2.20% at L1 (11 of 15).
// The extra hash needed to compute the future slot costs more than the miss it
// hides -- at four cycles a position the loop is ALU-bound, not stalled on
// stores, and the store buffer already covers the latency.
//
// Reverted rather than left switchable: a brick that measures worse does not
// earn an arm. Recorded so it is not re-attempted.

/// GATE 5 @ L3 -- adaptive `block_max`, decided PER BLOCK from the previous
/// blocks' own outcomes.
///
/// The sweep found 11 of 18 corpora prefer a block smaller than 128 KiB, with the
/// optimum landing on six different sizes -- a dispatch. One variable could not
/// carry it (chunk-drift alone reads r = -0.358) because THREE mechanisms drive
/// the choice, and they disagree:
///
///   1 ENTROPY DRIFT   mozilla, samba, xml, mr -- statistics move along the file,
///                     so a smaller block re-adapts its tables sooner.
///   2 RAW ESCAPE      sao (ratio 0.85), x-ray (0.80) -- barely compressible, so a
///                     smaller block lets an incompressible region go RAW on its
///                     own instead of dragging a bad Huffman table across 128 KiB.
///   3 MATCH REACH     versions (ratio 0.047) -- the ratio comes from long-range
///                     near-copies that CROSS block boundaries, so splitting
///                     destroys the matches that pay. Keep the block big.
///
/// Plus the degenerate case: an RLE block costs 1 byte, so splitting one is pure
/// header. `zeros` and `text` are +32.6% and +35% under a constant 96 KiB purely
/// through that, which is what stops a constant from shipping.
///
/// All three signals are FREE -- the previous blocks' own compressed ratios and
/// `rep_yield`, already carried.
#[inline]
fn adaptive_block_max(
    base: usize,
    r_prev: f32,
    r_prev2: f32,
    rep_yield: f32,
    strategy: Strategy,
    input_len: usize,
) -> usize {
    // 4.77 -- THE FAST LADDER IS SIZE-DISPATCHED. Its own fitting grid, re-run:
    //
    // ```text
    //   input     TOTAL       sao        mozilla
    //   1 MiB   -0.1118%    -0.341%     -0.230%
    //   2 MiB   -0.0274%    +0.077%     -0.139%
    //   4 MiB   +0.0692%    +0.376%     +0.236%
    //   8 MiB   +0.0658%       --       +0.641%
    // ```
    //
    // The fit was real when it was made (1 MiB still reads -0.1118% against its
    // claimed -0.1140%) and has since INVERTED: `sao` and `mozilla` both
    // sign-flip, and the recorded "worst +0.000%" is now `mozilla` +0.641%.
    //
    // It costs TIME on exactly the content it no longer earns on -- `x-ray`
    // +26.40% and `sao` +8.63% against a 2.31% null, for +0.000% and +0.126%
    // size. Above the crossover the ladder is pure loss on both axes.
    if strategy == Strategy::Fast && input_len > g5_fast_max_len() {
        return base;
    }
    // LEVEL-AWARE. The thresholds below were fitted at L3 and they do NOT
    // transfer to L1: there they regressed `mozilla` +0.208% and `samba` +0.153%
    // at 8 MiB, while blocking `versions-16m` from a -3.935% win because the
    // match-reach guard that protects it at L3 is wrong on the Fast ladder.
    //
    // The mechanisms are the same; their thresholds are not. `Fast` emits a very
    // different sequence distribution, so both the repcode yield and the drift
    // it produces live on different scales.
    // THREE ladders, because the match-reach guard means something different on
    // each. On `Fast` it protects nothing (Fast never finds the long-range
    // matches it exists to preserve) and on the OPT ladder it fires on
    // everything, taking the whole gate to 0.000% on 18 of 18 corpora. Only the
    // middle ladder needs it.
    let (rep_min, ratio_min, drift_min) = match strategy {
        Strategy::Fast => (g5_rep_min_fast(), g5_ratio_min_fast(), g5_drift_min_fast()),
        Strategy::BtOpt | Strategy::BtUltra | Strategy::BtUltra2 => {
            (g5_rep_min_opt(), g5_ratio_min_opt(), g5_drift_min_opt())
        }
        _ => (g5_rep_min(), g5_ratio_min(), g5_drift_min()),
    };
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        G5_CALLS.fetch_add(1, Relaxed);
        // WHY a block is not reduced: record the two inputs the live mechanisms
        // test, so "0% reduced" can be attributed to a value rather than guessed.
        if r_prev >= 0.0 {
            G5_RPREV.fetch_add((r_prev.clamp(0.0, 10.0) * 10000.0) as u64, Relaxed);
            G5_RPREV_N.fetch_add(1, Relaxed);
            if r_prev2 >= 0.0 {
                let d = (r_prev - r_prev2).abs() / r_prev.max(1e-6);
                G5_DRIFTSUM.fetch_add((d.clamp(0.0, 100.0) * 10000.0) as u64, Relaxed);
                G5_DRIFT_N.fetch_add(1, Relaxed);
            }
        }
    }
    // block 0 has no history: always take the full size
    if r_prev < 0.0 {
        return base;
    }
    // degenerate: TRUE RLE, one byte per block, so splitting is pure header.
    if r_prev < g5_tiny_max() {
        return base;
    }
    // 4.76 -- the VERY-COMPRESSIBLE band, [tiny, rle). Not RLE: compressible by
    // long-range MATCHES. At L1 `Fast` never finds those matches, so splitting
    // costs nothing it was earning and lets the entropy tables re-adapt.
    // `versions-16m` sits alone in this band at r_prev 0.0028 and wants -3.935%.
    if r_prev < G5_RLE_MAX {
        return base.min(g5_band());
    }
    // mechanism 3 -- long-range matches cross boundaries; splitting breaks them
    if rep_yield >= rep_min {
        return base;
    }
    // mechanism 2 -- barely compressible: let bad regions escape to RAW sooner
    if r_prev >= ratio_min {
        #[cfg(feature = "profile")]
        G5_HIT_RATIO.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return base.min(G5_SMALL);
    }
    // mechanism 1 -- entropy drift between the last two blocks
    if r_prev2 >= 0.0 {
        let drift = (r_prev - r_prev2).abs() / r_prev.max(1e-6);
        if drift >= drift_min {
            #[cfg(feature = "profile")]
            G5_HIT_DRIFT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            return base.min(G5_SMALL);
        }
    }
    base
}

pub static G5_RPREV: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static G5_RPREV_N: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static G5_DRIFTSUM: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static G5_DRIFT_N: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// GATE 5 inputs, as block means: `(mean r_prev, mean drift)`.
pub fn take_g5_inputs() -> (f64, f64) {
    use core::sync::atomic::Ordering::Relaxed;
    let a = G5_RPREV_N.swap(0, Relaxed).max(1) as f64;
    let b = G5_DRIFT_N.swap(0, Relaxed).max(1) as f64;
    (
        G5_RPREV.swap(0, Relaxed) as f64 / 10000.0 / a,
        G5_DRIFTSUM.swap(0, Relaxed) as f64 / 10000.0 / b,
    )
}

pub static G5_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static G5_HIT_RATIO: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static G5_HIT_DRIFT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// GATE 5 coverage: `(calls, raw-escape fires, drift fires)`.
pub fn take_g5() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (G5_CALLS.swap(0, Relaxed), G5_HIT_RATIO.swap(0, Relaxed), G5_HIT_DRIFT.swap(0, Relaxed))
}

/// 4.77: the Fast ladder is OFF above this input length. Crossover measured
/// between 2 and 4 MiB (total -0.0274% -> +0.0692%); 2 MiB keeps every cell that
/// still earns and drops every cell that regressed.
const G5_FAST_MAX_LEN: usize = 2 << 20;

static G5_FAST_LEN_A: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline(always)]
fn g5_fast_max_len() -> usize {
    let v = G5_FAST_LEN_A.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 { G5_FAST_MAX_LEN } else { v }
}

/// Bench arm. `usize::MAX` restores the pre-4.77 behaviour (ladder always on).
pub fn set_g5_fast_len_arm(v: usize) {
    G5_FAST_LEN_A.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// 4.76. Below this a block is TRUE RLE and must never be split.
///
/// `G5_RLE_MAX` was 0.01 and returned `base` for everything under it, which
/// intercepted `versions-16m` (r_prev **0.0028**) before any other mechanism ran.
/// The Fast ladder's `G5_REP_MIN_FAST = 2.00` was set to an OFF switch
/// specifically to release `versions` for a **-3.935%** win -- and the win was
/// never delivered, because this guard sits EARLIER in the chain and the comment
/// recording the fix does not mention it.
///
/// The separation is clean over two orders of magnitude:
///
/// ```text
///   zeros-32m     r_prev 0.000000   +32.593% if split   MUST NOT
///   text-32m      r_prev 0.000013   +30.159% if split   MUST NOT
///   versions-16m  r_prev 0.002817    -3.935% if split   WANTS SPLIT
/// ```
const G5_TINY_MAX: f32 = 0.0005;

/// DEFAULT OFF (`usize::MAX` never binds). The band was BUILT and MEASURED and it
/// LOSES: versions-16m **+1.685%** at L1 where the sweep promised -3.935%, and
/// text-32m +1.270% despite sitting below the tiny guard on its mean.
///
/// The reason is the finding. The sweep's -3.935% comes from a UNIFORM 96 KiB
/// grid over the whole frame. GATE 5 is PER BLOCK and block 0 always takes
/// `base`, so every later boundary is offset from that grid. `versions-16m` is a
/// versioned-file corpus whose ratio comes from long-range near-copies, and its
/// block-size curve is non-monotonic (+21.2% at 16 KiB, -0.516% at 64 KiB,
/// **-3.935%** at 96 KiB, 0 at 128 KiB) -- an ALIGNMENT signature, not a
/// "smaller blocks re-adapt sooner" one. A per-block mechanism cannot produce an
/// aligned uniform grid, so this win is structurally GATE 19's (per frame), not
/// GATE 5's (per block).
///
/// Kept, default off, because the band itself is correct machinery and the
/// separation it keys on is real (two orders of magnitude, see `G5_TINY_MAX`).
const G5_BAND: usize = usize::MAX;

static G5_TINY_A: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_BAND_A: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline(always)]
fn g5_tiny_max() -> f32 {
    let v = G5_TINY_A.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX { G5_TINY_MAX } else { f32::from_bits(v) }
}

#[inline(always)]
fn g5_band() -> usize {
    let v = G5_BAND_A.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 { G5_BAND } else { v }
}

/// Bench arms for the 4.76 band. `set_g5_band_arm(usize::MAX)` disables the band
/// (it can then never bind), restoring the pre-4.76 behaviour exactly.
pub fn set_g5_tiny_arm(v: f32) {
    G5_TINY_A.store(if v.is_nan() { u32::MAX } else { v.to_bits() },
        core::sync::atomic::Ordering::Relaxed);
}
pub fn set_g5_band_arm(v: usize) {
    G5_BAND_A.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Below this ratio a block is RLE or near-RLE: splitting only adds headers.
const G5_RLE_MAX: f32 = 0.01;
/// The smaller arm. 64 KiB is the best single small size in the sweep
/// (-0.171% aggregate); 16/32 win more on individual corpora but cost far more
/// time (+46.7% / +16.1% against +6.6%).
const G5_SMALL: usize = 64 << 10;

/// FITTED ON TRAIN (dickens, mozilla, nci, samba, xml, x-ray), judged ONCE on
/// HOLDOUT (mr, ooffice, osdb, reymont, sao, webster). Grid over
/// rep {0.30, 0.50, 0.70} x ratio {0.60, 0.70, 0.80} x drift {0.05, 0.10, 0.20},
/// objective = total train size, REFUSED if any train corpus regressed > 0.05%.
///
/// The FIRST fit used one input size and did not survive: `samba` flipped sign
/// with SIZE (+0.459% at 4 MiB, -0.151% at 8 MiB). A threshold that generalises
/// across CONTENT but not across SIZE is not fitted. Re-fitted across four caps
/// (1/2/4/8 MiB) at once, 68 (corpus, size) cells, and the drift term swept until
/// the worst case cleared:
///
///   drift >= 0.5   total -0.1111%   worst +0.300% (samba)
///   drift >= 1.0   total -0.1109%   worst +0.037% (xml)
///   drift >= 1.5   total -0.1101%   worst +0.008% (samba)   <- shipped
///
/// The total is flat across that sweep while the worst case falls 37x, so 1.5
/// costs nothing and buys the finish line. A drift of 1.5 means the block ratio
/// changed by 150% between neighbours -- only a dramatic transition re-adapts.
const G5_REP_MIN: f32 = 0.30;
const G5_RATIO_MIN: f32 = 0.70;
const G5_DRIFT_MIN: f32 = 1.50;

/// FAST-ladder thresholds (L1/L2), fitted separately -- see `adaptive_block_max`.
/// Fitted on TRAIN at L1 across 1/2/4/8 MiB (68 cells), judged once on HOLDOUT,
/// with the L3+ thresholds left exactly as shipped:
///
///   train    -0.1140%   worst +0.000%   best -0.799% (samba)
///   HOLDOUT  -0.0766%   worst +0.000%   best -0.408% (sao)
///
/// `rep >= 2.0` is not a threshold, it is an OFF switch: `rep_yield` cannot
/// exceed 1.0, so the match-reach branch never fires on the Fast ladder. That is
/// the finding. At L3 that guard protects `versions-16m`, whose ratio comes from
/// long-range near-copies that splitting would break. `Fast` does not find those
/// matches in the first place, so the guard protects nothing there and merely
/// blocked `versions` from a **-3.935%** win. A mechanism that is real at one
/// level can be pure cost at another.
const G5_REP_MIN_FAST: f32 = 2.00;
const G5_RATIO_MIN_FAST: f32 = 0.70;
const G5_DRIFT_MIN_FAST: f32 = 2.00;

/// OPT-ladder thresholds (L16-L22). `rep >= 2.0` is again an OFF switch: at L19
/// the shipped `rep >= 0.30` fired on every corpus and the gate did nothing at
/// all -- 0.000% on 18 of 18. Fitted separately below.
const G5_REP_MIN_OPT: f32 = 2.00;
const G5_RATIO_MIN_OPT: f32 = 0.50;
const G5_DRIFT_MIN_OPT: f32 = 1.50;

static G5_REP_O: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_RATIO_O: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_DRIFT_O: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook for the opt-ladder fit. Leaves Fast and the middle ladder alone.
pub fn set_g5_opt_arms(rep: f32, ratio: f32, drift: f32) {
    use core::sync::atomic::Ordering::Relaxed;
    G5_REP_O.store(rep.to_bits(), Relaxed);
    G5_RATIO_O.store(ratio.to_bits(), Relaxed);
    G5_DRIFT_O.store(drift.to_bits(), Relaxed);
}
#[inline]
fn g5_rep_min_opt() -> f32 {
    let b = G5_REP_O.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_REP_MIN_OPT } else { f32::from_bits(b) }
}
#[inline]
fn g5_ratio_min_opt() -> f32 {
    let b = G5_RATIO_O.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_RATIO_MIN_OPT } else { f32::from_bits(b) }
}
#[inline]
fn g5_drift_min_opt() -> f32 {
    let b = G5_DRIFT_O.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_DRIFT_MIN_OPT } else { f32::from_bits(b) }
}

static G5_REP_F: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_RATIO_F: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_DRIFT_F: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook for the Fast-ladder fit. Leaves the L3+ thresholds untouched.
pub fn set_g5_fast_arms(rep: f32, ratio: f32, drift: f32) {
    use core::sync::atomic::Ordering::Relaxed;
    G5_REP_F.store(rep.to_bits(), Relaxed);
    G5_RATIO_F.store(ratio.to_bits(), Relaxed);
    G5_DRIFT_F.store(drift.to_bits(), Relaxed);
}
#[inline]
fn g5_rep_min_fast() -> f32 {
    let b = G5_REP_F.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_REP_MIN_FAST } else { f32::from_bits(b) }
}
#[inline]
fn g5_ratio_min_fast() -> f32 {
    let b = G5_RATIO_F.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_RATIO_MIN_FAST } else { f32::from_bits(b) }
}
#[inline]
fn g5_drift_min_fast() -> f32 {
    let b = G5_DRIFT_F.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_DRIFT_MIN_FAST } else { f32::from_bits(b) }
}

static G5_REP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_RATIO: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
static G5_DRIFT: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hooks for the Gate 5 threshold fit. Negative disables that term.
pub fn set_g5_arms(rep: f32, ratio: f32, drift: f32) {
    use core::sync::atomic::Ordering::Relaxed;
    G5_REP.store(rep.to_bits(), Relaxed);
    G5_RATIO.store(ratio.to_bits(), Relaxed);
    G5_DRIFT.store(drift.to_bits(), Relaxed);
}
#[inline]
fn g5_rep_min() -> f32 {
    let b = G5_REP.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_REP_MIN } else { f32::from_bits(b) }
}
#[inline]
fn g5_ratio_min() -> f32 {
    let b = G5_RATIO.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_RATIO_MIN } else { f32::from_bits(b) }
}
#[inline]
fn g5_drift_min() -> f32 {
    let b = G5_DRIFT.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { G5_DRIFT_MIN } else { f32::from_bits(b) }
}

#[inline(always)]
pub(crate) fn prime_tables(
    tables: &mut MatchTables,
    src: &[u8],
    payload_off: usize,
    window: usize,
    params: CompressionParameters,
) {
    if payload_off == 0 {
        return;
    }
    let mls = params.min_match.max(3) as usize;
    let from = payload_off.saturating_sub(window);
    let ilimit = payload_off.saturating_sub(8);
    if from >= ilimit || src.len() < mls {
        return;
    }
    // BRICK 52, COMPLETED: the AUTHORITATIVE clamped value, never `params`.
    // `params.hash_log` is USER-SETTABLE with no upper bound (`hlog` in the
    // advanced-parameter setter does only `value.max(6)`), while the table is
    // allocated at `params.hash_log.clamp(6, 24)`. Indexing with the raw value
    // therefore ran off the end of a 2^24 table: `hlog >= 25` at L9 panicked
    // with `index out of bounds: the len is 16777216 but the index is
    // 28488790`. Brick 52 fixed `find_fast` and `find_dfast` and left the
    // chain-walking finders on the raw value.
    let hash_log = tables.hash_log;
    let chain_mask = tables.chain.len().saturating_sub(1);
    // See `PRIME_BT_ARM`: from BtLazy2 up, `chain` is a binary tree, not a chain.
    let uses_bt = matches!(
        params.strategy,
        Strategy::BtLazy2 | Strategy::BtOpt | Strategy::BtUltra | Strategy::BtUltra2
    );
    let write_chain = (!uses_bt || prime_bt_chain_write()) && !tables.chain.is_empty();
    // Hoisted: both were re-tested on EVERY primed position.
    let do_long = !tables.hash_long.is_empty();
    let stride = prime_stride();
    // GATE 2 @ L1 -- prime the TAG as well, or the priming is thrown away.
    //
    // `put_h` writes `hash` and nothing else. `store_fast` writes `hash` AND
    // `tags`, "UNCONDITIONALLY whenever the array exists", because gating the
    // store on the same flag as the compare is what lets tags go stale -- the
    // defect class that already cost this gate a day (190ad8b).
    //
    // `prime_tables` was the one remaining writer that broke that rule. At L1
    // the tag array is allocated, `tag_yield` SEEDS TO 1.0 and `tag_min` is
    // 0.50, so the filter is ON for block 0 -- exactly the block that consumes
    // the primed prefix. Every primed slot carried `tags[h] == 0`, mismatched,
    // and `load_fast` returned 0: the candidate was rejected without ever
    // reading `src[m]`.
    //
    // Measured before the fix, prefix-primed at L1 with the filter forced OFF:
    // smaller on 14 of 18 corpora, -0.4114% overall, `versions-16m` -59.3%
    // (13,164 -> 5,355 bytes). With NO prefix the same A/B is 0.000% on all 18,
    // which is the control proving the effect is priming-specific.
    //
    // Derived exactly like `hash4_tag` rather than via `hash_mls`: `find_fast`
    // always hashes 4 bytes whatever `min_match` says, so a `mls >= 8` Fast row
    // would otherwise prime hash8 slots the finder never reads.
    let is_fast =
        params.strategy == Strategy::Fast && (tables.pack_tags || !tables.tags.is_empty());
    let mut iters = 0u64;
    // FINDING 2: on the Bt ladder, INSERT each position into the tree rather
    // than only writing its hash head. `bt_find_best` performs the insertion as
    // a side effect and maintains the head itself, so this is the whole change.
    // `block_start = block_end = payload_off` keeps the load self-contained: the
    // descent floor becomes `payload_off - window` (exactly the priming range)
    // and comparisons stop at the end of the prefix, never running into payload
    // the caller has not asked us to look at yet.
    if uses_bt && prime_bt_tree_enabled() && !tables.chain.is_empty() {
        // COST CONTROL. `bt_find_best` runs a full SEARCH at each position --
        // it tracks the best match and calls `count_match` -- when priming only
        // needs the INSERT. libzstd separates the two: `ZSTD_insertBt1` is
        // insert-only. We cannot cheaply drop the comparison (it decides
        // left/right), but we CAN bound how deep the insert descends, and depth
        // is the term the cost is linear in.
        //
        // Injected through `params.search_log` rather than new plumbing, so the
        // real search path keeps its exact code and pays nothing for this.
        // Striding the insert was measured first and REFUSED: the size win
        // collapses faster than the cost (stride 4 keeps 0.045% of 1.78%).
        let d = prime_bt_depth();
        let pparams = if d == 0 {
            params
        } else {
            CompressionParameters { search_log: d, ..params }
        };
        let prime_attempts =
            bt_depth_apply(search_attempts(pparams), pparams, tables.opt_rep_rate);
        let btf = bt_resolve::<false>(tables.hash_log, pparams.chain_log.min(24));
        let prime_ctx = BtCtx {
            src,
            block_start: payload_off,
            block_end: payload_off,
            window,
            mls,
            attempts: prime_attempts,
            chain_log: pparams.chain_log.min(24),
        };
        // EXTENT: the tree only over the most recent slice; heads below it.
        let ext = prime_bt_extent();
        let range = ilimit.saturating_sub(from);
        let tree_from = if ext <= 1 {
            from
        } else {
            ilimit.saturating_sub(range / ext).max(from)
        };
        let mut p = from;
        while p < tree_from && p + 8 <= src.len() {
            let h = hash_mls(src, p, mls, hash_log);
            tables.put_h(h, p);
            if do_long {
                let hl = hash8(src, p, hash_log);
                tables.put_hl(hl, p);
            }
            iters += 1;
            p += stride;
        }
        while p <= ilimit && p + 8 <= src.len() {
            let _ = btf(&prime_ctx, p, tables);
            iters += 1;
            p += stride;
        }
        #[cfg(feature = "profile")]
        PRIME_ITERS.fetch_add(iters, core::sync::atomic::Ordering::Relaxed);
        #[cfg(not(feature = "profile"))]
        let _ = iters;
        return;
    }
    let mut p = from;
    while p <= ilimit && p + 8 <= src.len() {
        if is_fast {
            // ffanat hash-width: MUST mirror `fast_hash_tag` exactly, or every
            // primed slot mismatches the finder's keys -- the -59.3% priming
            // poison this function has already been bitten by once.
            let fhp = fast_hash_spec(mls, hash_log);
            let (h, tag) =
                fast_hash_tag::<true>(src, p, fhp.wide, fhp.mask, fhp.shift);
            tables.store_fast(h, p, tag);
        } else {
            // Chain-tag frames prime in the finder's own format (packed or
            // array) -- the -59.3% priming-poison rule, third application.
            if tables.chain_pack || !tables.ctags.is_empty() {
                let cp = tables.chain_pack;
                let ca = !tables.ctags.is_empty();
                let smask = if mls >= 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 };
                let (hh, gt) = if tables.chain_wide {
                    hash_wide_link_tag(src, p, hash_log, smask)
                } else {
                    hash4_link_tag(src, p, hash_log, smask)
                };
                if write_chain {
                    let _ = tables.lz_insert(hh, p, gt, cp, ca, chain_mask);
                } else {
                    let raw = tables.lz_head_raw(hh);
                    let _ = raw;
                    if ca {
                        tables.tags[hh] = gt;
                    }
                    tables.lz_head_put(hh, p, gt, cp);
                }
                if do_long {
                    let hl = hash8(src, p, hash_log);
                    tables.put_hl(hl, p);
                }
                iters += 1;
                p += stride;
                continue;
            }
            let h = hash_mls(src, p, mls, hash_log);
            if write_chain {
                tables.chain[p & chain_mask] = tables.get_h(h).map(|x| x as u32).unwrap_or(0);
            }
            // T1: the Fast branch above learned this the hard way -- prime the
            // TAG or the filter rejects every primed slot and the priming is
            // thrown away (-0.4114% overall at L1, `versions-16m` -59.3%). DFast
            // reaches this branch, so it needs the same treatment. `mls` is 5
            // there, so `hash_mls` took its hash4 path and the tag comes from
            // the same 4 bytes as the index.
            if tables.tags.is_empty() && !tables.pack_tags {
                tables.put_h(h, p);
                if do_long {
                    let hl = hash8(src, p, hash_log);
                    tables.put_hl(hl, p);
                }
            } else {
                // MUST mirror `hash4_tag_mls` exactly -- the -59.3% priming
                // poison. `p + 8 <= len` is this loop's own guard.
                let sk = 8.min(mls);
                let smask = if sk == 8 { u64::MAX } else { (1u64 << (8 * sk)) - 1 };
                let tv = (load_u64le(src, p) & smask).wrapping_mul(FAST_HASH_PRIME64);
                let g = (tv ^ (tv >> 29)) as u8;
                tables.put_h_tag(h, p, g);
                // 1a: prime the LONG tag too, or the filter rejects every
                // primed long slot -- the exact -59.3% priming-poison class
                // the short table was bitten by.
                if do_long {
                    let hl = hash8(src, p, hash_log);
                    tables.put_hl_tag(hl, p, g);
                }
            }
        }
        iters += 1;
        p += stride;
    }
    #[cfg(feature = "profile")]
    PRIME_ITERS.fetch_add(iters, core::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "profile"))]
    let _ = iters;
}

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
    // Wholesale BMI2 twin: the per-block section packing carried 34
    // variable shifts of its own, outside every finer-grained twin.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            encode_block_bmi2(
                out, src, block_start, block_end, window, params, tables, reps, entropy, last,
                ldm, ldm_p,
            )
        };
    }
    encode_block_inner(
        out, src, block_start, block_end, window, params, tables, reps, entropy, last, ldm, ldm_p,
    )
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn encode_block_bmi2(
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
    encode_block_inner(
        out, src, block_start, block_end, window, params, tables, reps, entropy, last, ldm, ldm_p,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn encode_block_inner(
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
    let skip_search =
        raw_skip_on() && tables.raw_run >= raw_run_min() && tables.raw_probe != 0;
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
        let mut probe = tables.clone();
        probe.route_force = 2;
        let (s2, l2) = find_sequences(
            src, block_start, block_end, window, params, &mut probe, None, ldm_p, *reps,
        );
        let _m = crate::prof::scope(crate::prof::Stage::EncodeMatchFind);
        let r = find_sequences(
            src, block_start, block_end, window, params, tables, ldm, ldm_p, *reps,
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
        out.extend_from_slice(block);
        // GATE 6 @ L1 -- hand the finder's buffers back to the frame.
        if finder_scratch_enabled() {
            tables.seq_scratch = seqs;
            tables.lit_scratch = literals;
        }
        return Ok(());
    }
    let saved_reps = *reps;
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
    let mut payload = std::mem::take(&mut tables.payload_scratch);
    payload.clear();
    if payload_reserve_enabled() && payload.capacity() < block.len() {
        // REPLACE, do not grow. `payload` was just cleared, so `realloc` would
        // memcpy an allocation that holds nothing live. See `opt_ops`.
        payload = Vec::with_capacity(block.len());
    }
    {
        let _e = crate::prof::scope(crate::prof::Stage::EncodeEntropy);
        if seqs.is_empty() {
            let _ = write_literals(&mut payload, block, entropy)?;
            crate::prof::note_emit_lit(payload.len() as u64);
            payload.push(0);
        } else {
            let lit_reused = write_literals(&mut payload, &literals, entropy)?;
            let lit_end = payload.len();
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
            write_sequences(&mut payload, &seqs, reps, entropy, params.strategy, tables)?;
            crate::prof::note_emit_seq((payload.len() - lit_end) as u64);
        }
    }
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
        let ratio = (payload.len() as f64 / raw_limit.max(1) as f64 * 1000.0) as u64;
        if payload.len() >= raw_limit {
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
    if payload.len() >= raw_limit {
        #[cfg(feature = "profile")]
        RAW_EXIT[2].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        *reps = saved_reps;
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
        out.extend_from_slice(block);
        tables.payload_scratch = payload;
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
        payload.len(),
        tables.rep_yield,
        off_coll,
        off_bkt,
    );
    note_raw_outcome(tables, false);
    note_step_outcome(tables, payload.len(), block.len());
    write_block_header(out, last, BlockType::Compressed, payload.len() as u32);
    out.extend_from_slice(&payload);
    tables.payload_scratch = payload;
    if finder_scratch_enabled() {
        tables.seq_scratch = seqs;
        tables.lit_scratch = literals;
    }
    Ok(())
}

/// Streaming block: `src` is history || current block; sequences only from `block_start`.
#[allow(clippy::too_many_arguments)]
pub(crate) fn encode_block_from_scratch(
    out: &mut Vec<u8>,
    src: &[u8],
    block_start: usize,
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
        src.len(),
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

fn rle_byte(block: &[u8]) -> Option<u8> {
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
    static SKIP_OVERRIDE: core::cell::Cell<Option<bool>> =
        const { core::cell::Cell::new(None) };
}





/// Gate 16 arm: the incompressible early-raw skip. Also RETIRES the uncached
/// `std::env::var` read that `incomp_skip_on` performed on EVERY BLOCK -- the
/// last uncached env read on a hot path (m7-anatomy section 3 addendum).
/// 0 = unresolved, 1 = off, 2 = on, 3 = follow the level rule.
static INCOMP_SKIP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

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
static RAW_SKIP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` always searches, restoring the pre-4.30 behaviour.
pub fn set_raw_skip_arm(on: bool) {
    RAW_SKIP_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn raw_skip_on() -> bool {
    RAW_SKIP_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// GATE 16 @ L3: the two constants the short circuit runs on, never swept.
///
/// `RAW_RUN_MIN` is how many consecutive raw blocks it takes before the search
/// is skipped; `RAW_PROBE_PERIOD` is how often it re-probes so content that
/// starts compressing is picked up. Both were chosen when 4.30 shipped and
/// neither has been moved since -- the same shape as the search-strength shift
/// of 4.43, which sat unexamined and turned out to be the biggest L1 lever.
static RAW_RUN_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);
static RAW_PROBE_ARM: core::sync::atomic::AtomicU32 =
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
fn raw_run_min() -> u32 {
    let v = RAW_RUN_MIN_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        RAW_RUN_MIN
    } else {
        v
    }
}

#[inline(always)]
fn raw_probe_period() -> u32 {
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
pub static RAW_EXIT: [core::sync::atomic::AtomicU64; 3] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
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
pub static RAW_MARGIN_SUM: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static RAW_MARGIN_N: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static RAW_MARGIN_HIST: [core::sync::atomic::AtomicU64; 4] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
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

const RAW_RUN_MIN: u32 = 2;
/// Blocks between forced re-probes of the raw short circuit.
const RAW_PROBE_PERIOD: u32 = 16;

/// Record whether this block ended up RAW, and tick the re-probe countdown.
/// Called on every exit so the run length is never stale.
fn note_raw_outcome(tables: &mut MatchTables, raw: bool) {
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

fn incomp_skip_on(params: CompressionParameters) -> bool {
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
            v = match std::env::var("RZSTD_INCOMP_SKIP") {
                Ok(x) if x.trim() == "0" || x.trim().eq_ignore_ascii_case("off") => 1,
                Ok(x) if x.trim() == "1" || x.trim().eq_ignore_ascii_case("on") => 2,
                _ => 3,
            };
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
fn early_raw_skip(match_bytes: usize, block_len: usize, params: CompressionParameters) -> bool {
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
fn offset_stats(seqs: &[Seq]) -> (u32, u8) {
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

fn tap_block(
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

fn write_block_header(out: &mut Vec<u8>, last: bool, ty: BlockType, size: u32) {
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
fn write_literals(
    dst: &mut Vec<u8>,
    lits: &[u8],
    entropy: &mut EntropyState,
) -> Result<bool, Error> {
    // The literal-section table builders (histogram, ctable, tree write,
    // normalize, ncount) all carry variable shifts; the BMI2 twin compiles
    // the whole section in its own ISA context.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe { write_literals_bmi2(dst, lits, entropy) };
    }
    write_literals_inner(dst, lits, entropy)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn write_literals_bmi2(
    dst: &mut Vec<u8>,
    lits: &[u8],
    entropy: &mut EntropyState,
) -> Result<bool, Error> {
    write_literals_inner(dst, lits, entropy)
}

#[inline(always)]
fn write_literals_inner(
    dst: &mut Vec<u8>,
    lits: &[u8],
    entropy: &mut EntropyState,
) -> Result<bool, Error> {
    let _h = crate::prof::scope(crate::prof::Stage::EncodeHuff);
    let (sec, upd) = huffman::encode_literals_section(lits, entropy.huff.as_ref())?;
    let reused = matches!(upd, HuffUpdate::Unchanged);
    match upd {
        HuffUpdate::New(ct) => entropy.huff = Some(ct),
        HuffUpdate::Unchanged => {}
    }
    dst.extend_from_slice(&sec);
    Ok(reused)
}

fn write_nseq(dst: &mut Vec<u8>, n: u32) {
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

fn write_sequences(
    dst: &mut Vec<u8>,
    seqs: &[Seq],
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    strategy: Strategy,
    tables: &mut MatchTables,
) -> Result<(), Error> {
    // The encode-side mirror of 621a140: the FSE flush loop and add_bits are
    // variable-shift chains; the BMI2 twin compiles the same body with
    // shrx/shlx available. Byte-identity by construction.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: guarded by runtime CPUID; the body is identical.
        #[allow(unsafe_code)]
        return unsafe { write_sequences_bmi2(dst, seqs, reps, entropy, strategy, tables) };
    }
    write_sequences_inner(dst, seqs, reps, entropy, strategy, tables)
}

/// The BMI2-compiled twin.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn write_sequences_bmi2(
    dst: &mut Vec<u8>,
    seqs: &[Seq],
    reps: &mut [u32; 3],
    entropy: &mut EntropyState,
    strategy: Strategy,
    tables: &mut MatchTables,
) -> Result<(), Error> {
    write_sequences_inner(dst, seqs, reps, entropy, strategy, tables)
}

#[inline(always)]
fn write_sequences_inner(
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

    let (coded, ll_count, of_count, ml_count, of_max) = {
        let _sc = crate::prof::scope(crate::prof::Stage::EncodeSeqCode);
        // T4/brick-79: hoist the LUT arm out of the per-sequence loop. It was
        // read inside `ll_code` AND `ml_code`, i.e. two atomic loads per
        // sequence, while the two copy arms beside it are both resolved once
        // per block.
        let lut_arm = crate::compressed::lut_on();
        let mut coded: Vec<CodedSeq> = std::mem::take(&mut tables.coded_scratch);
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
            ll_count[llc as usize] += 1;
            of_count[ofc as usize] += 1;
            ml_count[mlc as usize] += 1;
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
    let use_low = strategy.id() >= Strategy::Lazy.id();
    let last_i = coded.len() - 1;
    let (ll_mode, ll_t, ll_hdr, of_mode, of_t, of_hdr, ml_mode, ml_t, ml_hdr) = {
        let _t = crate::prof::scope(crate::prof::Stage::EncodeTableSelect);
        let (ll_mode, ll_t, ll_hdr) = select_seq_table(
            &ll_count,
            36,
            9,
            &fse::DEFAULT_LL_NORM,
            6,
            entropy.ll.as_ref(),
            use_low,
            false,
            coded[last_i].llc as usize,
        )?;
        let of_needs_comp = of_max as usize >= fse::DEFAULT_OF_NORM.len();
        let (of_mode, of_t, of_hdr) = select_seq_table(
            &of_count,
            32,
            8,
            &fse::DEFAULT_OF_NORM,
            5,
            entropy.of.as_ref(),
            use_low,
            of_needs_comp,
            coded[last_i].ofc as usize,
        )?;
        let (ml_mode, ml_t, ml_hdr) = select_seq_table(
            &ml_count,
            53,
            9,
            &fse::DEFAULT_ML_NORM,
            6,
            entropy.ml.as_ref(),
            use_low,
            false,
            coded[last_i].mlc as usize,
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

    let last = last_i;
    let _fs = crate::prof::scope(crate::prof::Stage::EncodeFseSeq);
    let mut ml_s = ml_t.init_state2(coded[last].mlc as usize);
    let mut of_s = of_t.init_state2(coded[last].ofc as usize);
    let mut ll_s = ll_t.init_state2(coded[last].llc as usize);

    let mut bits = BitCStream::from_vec(
        std::mem::take(&mut tables.bits_scratch),
        coded.len() * 4 + 16,
    );
    bits.add_bits(u64::from(coded[last].llx), u32::from(coded[last].llb));
    bits.add_bits(u64::from(coded[last].mlx), u32::from(coded[last].mlb));
    bits.add_bits(u64::from(coded[last].ofx), u32::from(coded[last].ofc));
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
    entropy.ll = Some(ll_t);
    entropy.of = Some(of_t);
    entropy.ml = Some(ml_t);
    Ok(())
}

/// libzstd `ZSTD_buildCTable`: last sequence is `FSE_initCState2` only.
#[inline(always)]
fn ncount_seq_table(
    counts: &[u32],
    last_sym: usize,
    max_log: u8,
    use_low_prob: bool,
) -> Result<(Vec<u8>, FseCTable), Error> {
    // libzstd ZSTD_buildCTable: last sequence is FSE_initCState2 only, so drop
    // it from the normalized counts when it still leaves a usable distribution.
    let mut buf = counts.to_vec();
    if last_sym < buf.len() && buf[last_sym] > 1 {
        buf[last_sym] -= 1;
    }
    fse::ncount_and_ctable(&buf, max_log, use_low_prob)
}

/// Select Predefined / RLE / FSE-compressed / Repeat. Returns (mode, table, header bytes).
#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn select_seq_table(
    counts: &[u32],
    _alphabet: usize,
    max_log: u8,
    default_norm: &[i16],
    default_log: u8,
    prev: Option<&FseCTable>,
    use_low_prob: bool,
    force_compressed: bool,
    last_sym: usize,
) -> Result<(u8, FseCTable, Vec<u8>), Error> {
    let total: u32 = counts.iter().sum();
    let most = counts.iter().copied().max().unwrap_or(0);
    // libzstd ZSTD_selectEncodingType: a single symbol is always RLE.
    if total > 0 && most == total {
        let sym = counts.iter().position(|&c| c == total).unwrap_or(0) as u8;
        return Ok((1, FseCTable::rle(u16::from(sym)), vec![sym]));
    }

    let basic = fse::FseCTable::from_norm(default_norm, default_log)?;
    let mut best_mode = 0u8;
    let mut best_table: Option<FseCTable> = None;
    let mut best_hdr = Vec::new();
    let mut best_cost = basic.bit_cost(counts);

    if let Some(p) = prev {
        let c = p.bit_cost(counts);
        if c <= best_cost {
            best_mode = 3;
            best_table = Some(p.clone());
            best_hdr = Vec::new();
            best_cost = c;
        }
    }

    if total >= 8 {
        if let Ok((hdr, ct)) = ncount_seq_table(counts, last_sym, max_log, use_low_prob) {
            let c = ct.bit_cost(counts) + (hdr.len() as u64) * 8;
            if c < best_cost || force_compressed {
                best_mode = 2;
                best_table = Some(ct);
                best_hdr = hdr;
                best_cost = c;
            }
        }
    }

    if force_compressed && best_mode == 0 {
        if let Ok((hdr, ct)) = ncount_seq_table(counts, last_sym, max_log, use_low_prob) {
            return Ok((2, ct, hdr));
        }
        return Err(Error::Corruption);
    }
    let _ = best_cost;
    Ok((best_mode, best_table.unwrap_or(basic), best_hdr))
}

#[derive(Clone, Copy)]
struct CodedSeq {
    llc: u8,
    mlc: u8,
    ofc: u8,
    llx: u32,
    mlx: u32,
    ofx: u32,
    llb: u8,
    mlb: u8,
}

#[allow(clippy::too_many_arguments)]
fn find_sequences(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    ldm: Option<&mut crate::ldm::LdmTables>,
    ldm_p: crate::ldm::LdmParams,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_sequences_bmi2(src, block_start, block_end, window, params, tables, ldm, ldm_p, reps)
        };
    }
    find_sequences_inner(src, block_start, block_end, window, params, tables, ldm, ldm_p, reps)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn find_sequences_bmi2(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    ldm: Option<&mut crate::ldm::LdmTables>,
    ldm_p: crate::ldm::LdmParams,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_sequences_inner(src, block_start, block_end, window, params, tables, ldm, ldm_p, reps)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_sequences_inner(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    ldm: Option<&mut crate::ldm::LdmTables>,
    ldm_p: crate::ldm::LdmParams,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let hits = if let Some(lt) = ldm {
        if ldm_p.enable {
            let rp = ldm_p.resolved(params.window_log);
            crate::ldm::collect_ldm(
                lt,
                src,
                block_start,
                block_end,
                window,
                rp,
                tables.frame_start,
            )
        } else {
            Vec::new()
        }
    } else {
        Vec::new()
    };
    if hits.is_empty() {
        return find_sequences_strategy(src, block_start, block_end, window, params, tables, reps);
    }
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
    let mut pos = block_start;
    for h in hits {
        if h.ip < pos || h.ip >= block_end {
            continue;
        }
        let (s, lit) = find_sequences_strategy(src, pos, h.ip, window, params, tables, reps);
        seqs.extend(s.iter().copied());
        lits.extend_from_slice(&lit);
        let consumed: u32 = s.iter().map(|x| x.litlen).sum();
        let leftover = (lit.len() as u32).saturating_sub(consumed);
        seqs.push(Seq {
            litlen: leftover,
            matchlen: h.matchlen,
            offset: h.offset,
        });
        pos = h.ip + h.matchlen as usize;
        if pos > block_end {
            pos = block_end;
        }
    }
    let (s, lit) = find_sequences_strategy(src, pos, block_end, window, params, tables, reps);
    seqs.extend(s);
    lits.extend_from_slice(&lit);
    (seqs, lits)
}

fn find_sequences_strategy(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // The DP drivers (find_opt / find_bt_lazy) inline HERE, so this twin is
    // what puts the L13-L22 pricing loops under BMI2.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_sequences_strategy_bmi2(src, block_start, block_end, window, params, tables, reps)
        };
    }
    find_sequences_strategy_sel(src, block_start, block_end, window, params, tables, reps)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn find_sequences_strategy_bmi2(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_sequences_strategy_sel(src, block_start, block_end, window, params, tables, reps)
}

#[inline(always)]
fn find_sequences_strategy_sel(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    match params.strategy {
        Strategy::DFast => find_dfast(src, block_start, block_end, window, params, tables, reps),
        Strategy::Greedy => find_greedy(src, block_start, block_end, window, params, tables, reps),
        Strategy::Lazy => find_lazy(src, block_start, block_end, window, params, tables, 1, reps),
        Strategy::Lazy2 => find_lazy(src, block_start, block_end, window, params, tables, 2, reps),
        Strategy::BtLazy2 => find_bt_lazy(src, block_start, block_end, window, params, tables, 2, reps),
        Strategy::BtOpt | Strategy::BtUltra | Strategy::BtUltra2 => {
            find_opt(src, block_start, block_end, window, params, tables, reps)
        }
        Strategy::Fast => {
            // GATE 1 @ L1 -- DEPLOYED DISPATCH.
            //
            // Measured (best-of-41, ABBA, both arms in one process, whole file):
            //   versions-16m  fast 81,206 B / 3.39 ms -> lazy 49,697 B / 2.72 ms
            //                 38.8% SMALLER and 19.6% FASTER -- dominated, not a trade
            //   text-32m      1.19% smaller at +0.2% time (noise)
            //   nci           19.17% smaller but +77.0% time  <- must NOT fire
            //
            // SIGNAL: `rep_yield`, not `hit_rate`. Both separate the corpora, but
            // `rep_yield` is ALREADY maintained in shipping builds for the
            // repcode dispatch, so this costs one compare and no new counter.
            //
            // THRESHOLD sits in a MEASURED EMPTY INTERVAL:
            //   highest real corpus   mr        0.4949
            //   lowest firing corpus  versions  0.9778   (~2x margin)
            // `nci` (0.0039) is nowhere near it, which is what makes this safe.
            //
            // At `hit_rate` the same two corpora sit at 0.9846/0.9962 against a
            // real maximum of 0.491 -- the same shape, but it would need a new
            // per-block counter in the shipping build.
            // Advance the run counter on the PREVIOUS block's measured yield.
            if tables.blocks_done > 0 && tables.rep_yield > fast_lazy_threshold() {
                tables.rep_run = tables.rep_run.saturating_add(1);
            } else {
                tables.rep_run = 0;
            }
            if fast_lazy_enabled() && tables.rep_run >= FAST_LAZY_RUN {
                // ffanat 5a: the ONE hazard of the packed Fast table, handled at
                // its one site. Lazy reads this table through `get_h`, which
                // must see plain `pos + 1` -- the historical refutation of the
                // packed form was exactly this shared read. Strip the tag bytes
                // once (16K entries, on a dispatch that fires rarely) and stay
                // unpacked for the rest of the frame; the tag filter is a pure
                // filter, so later Fast blocks running without it are
                // byte-identical by T1's argument.
                #[cfg(feature = "profile")]
                FF_LAZY_FIRES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                if !tables.fast_hash_legacy
                    && fast_hash_wide_enabled()
                    && (5..=8).contains(&(params.min_match.max(3) as usize))
                {
                    // See `fast_hash_legacy` -- and the refutation ladder that
                    // led here. Clearing was byte-IDENTICAL to leaving the wide
                    // keys in place (latches=1, output unchanged), which proved
                    // lazy treats wide-keyed and empty alike: both give it
                    // nothing. The legacy arm's lazy blocks inherit REAL 4-byte
                    // heads, and that inheritance is the residual delta. So:
                    // RE-SEED, don't clear. One stride-1 pass over the lookback
                    // window rebuilds the heads lazy actually reads
                    // (`hash_mls` == hash4 at these mls), then the frame
                    // latches legacy so every later block stays coherent.
                    fast_hash_relatch(tables, src, block_start, window);
                }
                // TAG AUDIT 2026-08-20: when the relatch above ran, it
                // already set `pack_tags = false`, so this unpack loop is
                // SKIPPED and slots outside the re-seeded window keep their
                // packed (and wide-keyed) bits while the flag says unpacked.
                // That is SAFE, not sloppy, and deliberately so: every
                // consumer downstream of the switch (lazy heads, chain walk,
                // fills) validates candidates through `match_ok`, whose FIRST
                // test rejects `m >= ip`, so a stale tag byte decoding as a
                // huge position costs one dead probe and can never underflow,
                // read out of bounds, or change output -- and the clear-vs-not
                // experiment in the relatch comment measured byte-identity
                // directly. The invariant to preserve: `pack_tags == false`
                // does NOT promise the slots are tag-free; only `match_ok`
                // discipline makes that irrelevant. Do not add a consumer
                // that trusts positions without it.
                if tables.pack_tags {
                    for e in tables.hash.iter_mut() {
                        *e &= 0x00FF_FFFF;
                    }
                    tables.pack_tags = false;
                }
                // `Fast` does not allocate a chain (brick 47), so materialise it
                // on FIRST FIRE only -- files that never trip the dispatch keep
                // brick 47's smaller L1 footprint.
                if tables.chain.is_empty() {
                    tables.chain = alloc::vec![0u32; 1usize << params.chain_log.min(24)];
                }
                let r = find_lazy(src, block_start, block_end, window, params, tables, 1, reps);
                tables.blocks_done += 1;
                return r;
            }
            let r = find_fast(src, block_start, block_end, window, params, tables, reps);
            tables.blocks_done += 1;
            r
        }
    }
}

/// GATE 1 @ L1 dispatch: route highly-repetitive content to the lazy finder.
/// `RZSTD_FASTLAZY=0` disables (reproducing pre-gate bytes exactly).
static FAST_LAZY_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_fast_lazy_arm(on: bool) {
    FAST_LAZY_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn fast_lazy_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match FAST_LAZY_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            #[cfg(feature = "std")]
            {
                let on = std::env::var("RZSTD_FASTLAZY")
                    .map(|v| v.trim() != "0")
                    .unwrap_or(true);
                FAST_LAZY_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
                on
            }
            #[cfg(not(feature = "std"))]
            true
        }
    }
}

/// Consecutive qualifying blocks required before the dispatch engages. Sits in
/// the measured empty interval [1, 107] -- see `MatchTables::rep_run`.
const FAST_LAZY_RUN: u32 = 4;

/// Per-block yield threshold feeding the run counter. `RZSTD_FASTLAZY_T` sweeps.
fn fast_lazy_threshold() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // ffanat: cached (the tag_min pattern). This is read per BLOCK on the
    // find_fast path -- an uncached `std::env::var` is 115.6 ns and a String
    // allocation per read, for a process constant.
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = FASTLAZY_T_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_FASTLAZY_T")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.7);
        FASTLAZY_T_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.7
}

static FASTLAZY_T_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Dispatch the Fast match finder on the frame-latched packed flag ONCE per
/// block, so neither the flag load nor the dead tag computation appears inside
/// the probe loop (brick 46). `packed` is fixed at table construction, so this
/// is a pure hoist -- both arms are byte-identical to the pre-brick code.
/// Repcode-1 stays on while at least this fraction of a block's sequences
/// were repcode hits. Below it the search is pure per-probe cost.
const REP_YIELD_MIN_DEFAULT: f32 = 0.125;

/// GATE 2 threshold, BY STRATEGY. Swept via `RZSTD_REPMIN` (overrides both).
///
/// The right constant is not the same across the ladder. Silesia totals,
/// shipped 0.125 vs always-on 0.0 (`text` fence so rustdoc does not run it):
///
/// ```text
/// L3  DFast     -0.472%   always-on WINS   (xml -3.390%, mozilla -2.117%)
/// L5  Greedy    -0.342%   always-on wins   -- NOT deployed, L5 not yet gated
/// L7  Lazy      +0.060%   always-on loses
/// L9  Lazy2     +0.092%   always-on loses
/// L13 BtLazy2   +0.225%   always-on loses  (xml +1.345%)
/// L19 BtUltra2   0.000%   no effect        (find_opt prices reps itself)
/// ```
///
/// DEPLOYED FOR `DFast` ONLY -- i.e. L3/L4, the level this gate was evaluated
/// at. `Greedy` shows the same sign but belongs to L5's own gate and is left
/// alone until that level is measured on its own terms.
///
/// The mechanism is the look-ahead. `try_rep1` commits a match at `ip+1`, which
/// in a LAZY finder PREEMPTS the deferred search that might have found a better
/// one at `ip+1` or `ip+2`. Fast/DFast/Greedy have no look-ahead to preempt, so
/// there the repcode probe is pure gain and gating it only loses bytes.
///
/// Always-on is also 0.6% FASTER on Silesia at L3, so the dispatch it replaces
/// was costing ratio and buying no speed.
/// Blocks between forced rep re-probes, so the ratio can be re-measured.
const REP_PROBE_PERIOD: u32 = 16;

/// GATE 2 second threshold: minimum rep-to-mean match length ratio.
fn rep_len_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // ffanat: cached (the tag_min pattern). This is read per BLOCK on the
    // find_fast path -- an uncached `std::env::var` is 115.6 ns and a String
    // allocation per read, for a process constant.
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = REPLEN_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_REPLEN")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(1.0);
        REPLEN_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1.0
}

static REPLEN_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Decay floor on `rep_yield` for DFast. 0.0 = shut on the first dry block.
///
/// It was 0.5, written when the DFast threshold was 0.0 and the gate could never
/// fire. Once the gate fires at 0.005 that schedule IS a warm-up cost: eight
/// blocks probing every position for nothing before it shuts.
///
/// And 0.5 was not actually protecting anything. With the search off `rep_hits`
/// is 0, so `rep_yield` keeps halving and never recovers -- it is the SAME
/// one-way latch as Gate 6's, merely eight blocks slower to engage. What makes
/// an immediate shut safe is the RE-PROBE (`rep_probe`), not the decay.
fn rep_decay() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[2].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = REP_DECAY_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_REP_DECAY")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.0);
        REP_DECAY_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.0
}
#[cfg(feature = "std")]
static REP_DECAY_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

#[cfg(feature = "std")]
static REPMIN_OVR: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

fn rep_yield_min_for(strategy: Strategy) -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[3].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // The RZSTD_REPMIN override resolved ONCE (it was an env::var -- a
    // GetEnvironmentVariableW plus a String -- per BLOCK in every finder's
    // rep_search_on). u32::MAX = unchecked, MAX-1 = no override.
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let mut c = REPMIN_OVR.load(Ordering::Relaxed);
        if c == u32::MAX {
            c = std::env::var("RZSTD_REPMIN")
                .ok()
                .and_then(|v| v.trim().parse::<f32>().ok())
                .map(f32::to_bits)
                .unwrap_or(u32::MAX - 1);
            REPMIN_OVR.store(c, Ordering::Relaxed);
        }
        if c != u32::MAX - 1 {
            return f32::from_bits(c);
        }
    }
    match strategy {
        // GATE 2 @ L3 -- was a flat 0.0, i.e. the repcode search CONSTANT ON and
        // never dispatched. It loses that way: forcing it off is smaller on 7 of
        // 18 (reymont +0.167%, dickens +0.099%, mr +0.083%).
        //
        // The size opportunity alone is negligible (-0.0106% at best, and every
        // threshold >= 0.01 loses because per-block yields straddle it -- xml
        // +3.089% at 0.03). The WORK is the point: `try_rep1` runs at EVERY
        // position, and x-ray yields 0.000, smallmsg 0.001, dickens 0.002 -- a
        // probe per position that essentially never hits.
        //
        // At 0.005, deterministically: 27.3% of all repcode probe positions
        // removed (134,428,522 -> 97,728,362) AND total size -0.0106%. Five
        // corpora shed 87.5% of their rep probes, which is exactly the decay
        // schedule: `rep_yield` falls as max(new, prev/2) from 1.0, so it takes
        // 8 blocks to drop below 0.005 -- 8 of 64 blocks left on.
        //
        // The speed of this is NOT claimed from the clock: the L3 null arm on
        // this box reaches -8.74%, far larger than the effect. The work count is
        // exact and needs no quiet machine.
        Strategy::DFast => 0.005,
        // GATE 2 RE-VALIDATION (all 18 @ L1, deterministic sizes): the shipped
        // 0.125 is not the optimum for the Fast ladder. Sweeping the threshold
        // against 0.125 as baseline:
        //   0.15/0.20/0.25 all give TOTAL -0.090%
        //   mr -0.783%  sao -0.152%  dickens -0.151%  ooffice -0.144%
        //   samba -0.061%  jsonlog -0.015%  xml -0.013%   vs mozilla +0.017%
        // Seven corpora smaller, one trivially larger. Scoped to Fast because
        // REP_YIELD_MIN_DEFAULT is shared with Lazy/Lazy2/BtLazy2 at L5-L15,
        // which this sweep did not cover.
        //
        // NOT FIXED by this, and recorded as an open gap: `xml` is 3.72% smaller
        // with rep1 forced ALWAYS ON, but always-on costs dickens +7.18% and
        // samba +4.51%. No threshold separates them -- their per-block rep_yield
        // distributions overlap -- so capturing xml needs a SECOND variable.
        Strategy::Fast => 0.20,
        _ => REP_YIELD_MIN_DEFAULT,
    }
}

#[allow(dead_code)]
fn rep_yield_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[4].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        std::env::var("RZSTD_REPMIN")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(REP_YIELD_MIN_DEFAULT)
    }
    #[cfg(not(feature = "std"))]
    REP_YIELD_MIN_DEFAULT
}

fn find_fast(
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
    tables.pair_route = if tables.route_force != 0 {
        tables.route_force
    } else if tables.step_pick == 2 && tables.step_reprobe > 0 && step_probe_on() {
        2
    } else if !pair_enabled() {
        0
    } else if params.target_length != 0 {
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
    macro_rules! go {
        ($p:expr, $r:expr, $h:expr, $s:expr, $pi:expr) => {
            if wide_block {
                find_fast_impl::<$p, $r, $h, $s, $pi, true>(
                    s0,
                    src,
                    block_start,
                    block_end,
                    window,
                    params,
                    tables,
                    reps,
                )
            } else {
                find_fast_impl::<$p, $r, $h, $s, $pi, false>(
                    s0,
                    src,
                    block_start,
                    block_end,
                    window,
                    params,
                    tables,
                    reps,
                )
            }
        };
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
    // ffanat census: WHICH monomorphisation class serves the traffic? The
    // comment on the (false,..) arms calls them "the shipping configuration",
    // but `ut` is tag_enabled() && tag_yield >= tag_min, which defaults ON --
    // if that is what usually runs, the shipped path is the GENERIC body.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        // MIRRORS THE DISPATCH BELOW exactly -- classify by which arm will
        // match, not by the inputs alone (the first version of this census
        // kept its labels when the dispatch gained arms, and read stale).
        let spec = fast_spec_enabled()
            && pipe_on
            && ((!ut && !rep_on && (1..=4).contains(&s0))
                || ((ut || rep_on) && (1..=2).contains(&s0)));
        let idx = if spec {
            0usize
        } else {
            match (ut, rep_on, pipe_on) {
                (true, false, true) => 1,
                (_, true, true) => 2,
                _ => 3,
            }
        };
        FF_ARM[idx].fetch_add(1, Relaxed);
    }
    let r = match (ut, rep_on, pipe_on, s0) {
        // The shipping configuration: no tag, no rep1, pipelined, default step.
        // Specialized on hash_log so the shift is an immediate too.
        (false, false, true, 2) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, false, 12, 2, true),
            13 => go!(false, false, 13, 2, true),
            14 => go!(false, false, 14, 2, true),
            15 => go!(false, false, 15, 2, true),
            16 => go!(false, false, 16, 2, true),
            _ => go!(false, false, 0, 2, true),
        },
        // Step 1 (probe EVERY position, C's density) gets the same treatment.
        // Without this it fell through to the runtime-STEP/runtime-HLOG arm,
        // so any step-1 measurement was comparing a generic loop against a
        // fully specialized one -- a work-parity break in the instrument, not
        // a property of the density.
        (false, false, true, 1) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, false, 12, 1, true),
            13 => go!(false, false, 13, 1, true),
            14 => go!(false, false, 14, 1, true),
            15 => go!(false, false, 15, 1, true),
            16 => go!(false, false, 16, 1, true),
            _ => go!(false, false, 0, 1, true),
        },
        // Step 3 and 4 get the SAME specialisation as 1 and 2. Without these
        // arms a step-3 measurement compares a fully generic body (runtime
        // shift AND runtime step) against a fully specialised step-2 one -- the
        // work-parity break this file already documents for step-1, and one I
        // reproduced: it made step 3 read -2.31% when the density's real effect
        // was masked by the generic arm's own cost.
        (false, false, true, 3) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, false, 12, 3, true),
            13 => go!(false, false, 13, 3, true),
            14 => go!(false, false, 14, 3, true),
            15 => go!(false, false, 15, 3, true),
            16 => go!(false, false, 16, 3, true),
            _ => go!(false, false, 0, 3, true),
        },
        (false, false, true, 4) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, false, 12, 4, true),
            13 => go!(false, false, 13, 4, true),
            14 => go!(false, false, 14, 4, true),
            15 => go!(false, false, 15, 4, true),
            16 => go!(false, false, 16, 4, true),
            _ => go!(false, false, 0, 4, true),
        },
        // ffanat 2026-08-20: the census that added FF_ARM found the arms above
        // serve ZERO blocks in the shipped configuration. `ut` defaults ON
        // (tag_enabled() && tag_yield >= tag_min, and tag_min ships 0.0) and
        // rep_on fires on most of the rest, so 100% of L1 traffic was running
        // the HLOG=0/STEP=0 GENERIC bodies -- the exact work-parity cost this
        // file documents for the step arms ("a fully generic body (runtime
        // shift AND runtime step)"). The live combinations get the same
        // specialisation the dead ones always had. Byte-identical by the same
        // argument as `find_dfast_impl`: the consts take the values the runtime
        // variables already held.
        (true, false, true, 2) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(true, false, 12, 2, true),
            13 => go!(true, false, 13, 2, true),
            14 => go!(true, false, 14, 2, true),
            15 => go!(true, false, 15, 2, true),
            16 => go!(true, false, 16, 2, true),
            _ => go!(true, false, 0, 2, true),
        },
        (true, false, true, 1) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(true, false, 12, 1, true),
            13 => go!(true, false, 13, 1, true),
            14 => go!(true, false, 14, 1, true),
            15 => go!(true, false, 15, 1, true),
            16 => go!(true, false, 16, 1, true),
            _ => go!(true, false, 0, 1, true),
        },
        (false, true, true, 2) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, true, 12, 2, true),
            13 => go!(false, true, 13, 2, true),
            14 => go!(false, true, 14, 2, true),
            15 => go!(false, true, 15, 2, true),
            16 => go!(false, true, 16, 2, true),
            _ => go!(false, true, 0, 2, true),
        },
        (false, true, true, 1) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(false, true, 12, 1, true),
            13 => go!(false, true, 13, 1, true),
            14 => go!(false, true, 14, 1, true),
            15 => go!(false, true, 15, 1, true),
            16 => go!(false, true, 16, 1, true),
            _ => go!(false, true, 0, 1, true),
        },
        (true, true, true, 2) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(true, true, 12, 2, true),
            13 => go!(true, true, 13, 2, true),
            14 => go!(true, true, 14, 2, true),
            15 => go!(true, true, 15, 2, true),
            16 => go!(true, true, 16, 2, true),
            _ => go!(true, true, 0, 2, true),
        },
        (true, true, true, 1) if fast_spec_enabled() => match tables.hash_log {
            12 => go!(true, true, 12, 1, true),
            13 => go!(true, true, 13, 1, true),
            14 => go!(true, true, 14, 1, true),
            15 => go!(true, true, 15, 1, true),
            16 => go!(true, true, 16, 1, true),
            _ => go!(true, true, 0, 1, true),
        },
        (false, false, true, _) => go!(false, false, 0, 0, true),
        (false, false, false, 2) => go!(false, false, 0, 2, false),
        (false, false, false, 1) => go!(false, false, 0, 1, false),
        (false, false, false, _) => go!(false, false, 0, 0, false),
        (false, true, true, _) => go!(false, true, 0, 0, true),
        (true, true, true, _) => go!(true, true, 0, 0, true),
        (true, true, false, _) => go!(true, true, 0, 0, false),
        (true, false, true, _) => go!(true, false, 0, 0, true),
        (true, false, false, _) => go!(true, false, 0, 0, false),
        (false, true, false, _) => go!(false, true, 0, 0, false),
    };
    // AFTER the call: `find_fast_impl` reads `pair_probe == 0` to force a probe,
    // so ticking beforehand would consume the very first one.
    tables.pair_probe =
        if tables.pair_probe == 0 { PAIR_PROBE_PERIOD } else { tables.pair_probe - 1 };
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
fn find_fast_impl<
    const PACKED: bool,
    const REP: bool,
    const HLOG: u32,
    const STEP: usize,
    const PIPE: bool,
    const WIDE: bool,
>(
    step_rt: usize,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BMI2 twin per monomorphisation: the probe loop's runtime shifts (wide
    // hash, tag mask, tail loads) become flag-free shrx/shlx. One branch per
    // BLOCK; both wrappers stay separate frames, preserving brick 48's
    // register isolation.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_fast_impl_bmi2::<PACKED, REP, HLOG, STEP, PIPE, WIDE>(
                step_rt, src, block_start, block_end, window, params, tables, reps,
            )
        };
    }
    find_fast_impl_inner::<PACKED, REP, HLOG, STEP, PIPE, WIDE>(
        step_rt, src, block_start, block_end, window, params, tables, reps,
    )
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
#[inline(never)]
unsafe fn find_fast_impl_bmi2<
    const PACKED: bool,
    const REP: bool,
    const HLOG: u32,
    const STEP: usize,
    const PIPE: bool,
    const WIDE: bool,
>(
    step_rt: usize,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_fast_impl_inner::<PACKED, REP, HLOG, STEP, PIPE, WIDE>(
        step_rt, src, block_start, block_end, window, params, tables, reps,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_fast_impl_inner<
    const PACKED: bool,
    const REP: bool,
    const HLOG: u32,
    const STEP: usize,
    const PIPE: bool,
    const WIDE: bool,
>(
    step_rt: usize,
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = params.min_match.max(3) as usize;
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
    let lp_copy = if reserve
        && (tables.blocks_done == 0 || tables.lit_short_share >= lit_short_min())
    {
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
    let mut seqs = if keep { std::mem::take(&mut tables.seq_scratch) } else { Vec::new() };
    seqs.clear();
    if reserve && seqs.capacity() < seq_guess {
        seqs = Vec::with_capacity(seq_guess);
    }
    let mut lits = if keep { std::mem::take(&mut tables.lit_scratch) } else { Vec::new() };
    lits.clear();
    if reserve && lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        crate::prof::note_huff_path(10);
        lits.extend_from_slice(&src[block_start..block_end]);
        crate::prof::note_search(0, 0, 0, 0, lits.len() as u64);
        tables.last_nseq = 0;
        return (seqs, lits);
    }
    let mut ip = block_start;
    // ffanat: take the table out of `MatchTables` so its data pointer is a
    // LOCAL for the whole loop. The asm showed it spilled and reloaded from the
    // stack three times per iteration; `src` was already register-resident
    // (brick 48) and the table never got the same fix. Handed back at every
    // exit below.
    let mut hash_v = core::mem::take(&mut tables.hash);
    let mut tags_v = core::mem::take(&mut tables.tags);
    let pack = tables.pack_tags;
    // Guard unification (see the dispatch): WIDE implies pack, so in WIDE
    // copies this is const-true -- the slot helpers' pack branches fold and
    // `tags_v` is provably untouched.
    debug_assert!(!WIDE || pack);
    let pack_eff = if WIDE { true } else { pack };
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
    let step0 = if STEP != 0 { STEP } else { step_rt };
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
    let probe = tables.pair_probe == 0;
    let route = tables.pair_route;
    // Frame-constant, so it is decided ONCE here rather than tested per match.
    let maintain_rep1 = pipe_rep1_enabled() && tables.rep_yield <= fast_lazy_threshold();
    // The route is decided in `find_fast` (it also selects the step, which must
    // be known before the specialised body is chosen). `rep_yield` still vetoes:
    // on rep-dominated content the pair search re-finds what the repcode path
    // already has and emits a worse parse.
    let _ = probe;
    let pair = step0 > 2 || (route == 2 && tables.rep_yield <= pair_rep_max());
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
    let hash_shift = if HLOG != 0 {
        32u32.saturating_sub(HLOG)
    } else {
        32u32.saturating_sub(tables.hash_log)
    };
    let _ = hash_shift;
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
    let fh = if tables.fast_hash_legacy {
        FastHash {
            wide: false,
            mask: 0,
            shift: 32u32.saturating_sub(if HLOG != 0 { HLOG } else { tables.hash_log }),
        }
    } else {
        fast_hash_spec(mls, if HLOG != 0 { HLOG } else { tables.hash_log })
    };
    // Scalarized AND const-moded: WIDE is a monomorphisation axis, so the
    // per-position mode branch is gone and specialised copies emit the shift
    // as an immediate. Only the mask (a function of runtime `mls`) stays in a
    // register.
    debug_assert!(fh.wide == WIDE);
    let f_wide = WIDE;
    let f_mask = if WIDE { fh.mask } else { 0 };
    let f_shift = if HLOG != 0 {
        if WIDE { 64 - HLOG } else { 32 - HLOG }
    } else {
        fh.shift
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
    // Design #7 (2026-08-20, REMOVED after census): REP-SUBSTITUTION -- swap an
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
    let bar_all = std::env::var("RZSTD_FFBAR_ALL").map(|v| v == "1").unwrap_or(false);
    #[cfg(not(feature = "profile"))]
    let bar_all = false;
    // The bar also covers POST-LATCH fast blocks (refutation #5: the re-seed
    // that heals lazy hands fast a dense table whose short matches are the
    // very harm -- 1,824 accepts, broken rep_runs. Lazy keeps the heads; fast
    // is barred from the shorties). `fast_hash_legacy` is only ever set under
    // the wide arm, so the off arm stays byte-identical.
    let veto_block = (WIDE || tables.fast_hash_legacy)
        && (bar_all || (tables.blocks_done > 0 && tables.rep_yield > fast_lazy_threshold()));
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
    if PIPE && !pair && ip <= ilimit {
        if COUNT {
            FF_PIPE_BLOCKS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        // INSTRUMENT DEFECT: `mm_total` is declared AFTER this block's
        // `return`, so MM_TOTAL only ever counted the NON-pipelined loop --
        // 58% of blocks, and 6.2% of them on the seven corpora that run this
        // path 93.8% of the time. 4.41's position ledger was an undercount.
        let mut pipe_pos = 0u64;
        let (mut ff_made, mut ff_used) = (0u64, 0u64);
        let (mut h0, mut g0) = fast_hash_tag::<true>(src, ip, WIDE, f_mask, f_shift);
        let mut m0 = fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h0, g0);
        loop {
            if COUNT {
                pipe_pos += 1;
            }
            if COUNT {
                if COUNT {
                    probes += 1;
                }
            }
            if COUNT && PACKED {
                let raw = fast_slot_raw(&hash_v, pack_eff, h0);
                if m0 == 0 && raw != 0 {
                    if fast_probe(&mut (0, 0), src, raw, ip, window, lowest, mls, block_end).is_some() {
                        TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
            }
            fast_slot_store(&mut hash_v, &mut tags_v, pack_eff, h0, ip, g0);
            if REP {
                // ffanat release-asm read: this unconditional per-position
                // increment was one of six spilled u64 locals -- `incq (%rbp)`
                // per miss in SHIPPING builds -- and its only consumers are the
                // COUNT-gated REP_PROBES publishes. Instrument, so gated.
                if COUNT {
                    rep_probes += 1;
                }
                if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end) {
                    rep_hits += 1;
                    rep_bytes += ml as u64;
                    if COUNT {
                        if COUNT {
                            hits += 1;
                        }
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
                    let (nh, ng) = fast_hash_tag::<true>(src, ip, WIDE, f_mask, f_shift);
                    h0 = nh;
                    g0 = ng;
                    m0 = fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h0, g0);
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
                let (h, g) = fast_hash_tag::<true>(src, nip, WIDE, f_mask, f_shift);
                // The store above may have just overwritten this slot, so the
                // value is forwarded by hand rather than re-read.
                //
                // IT MUST MIRROR `load_fast` EXACTLY. `load_fast::<PACKED>`
                // consults the tag ONLY when PACKED; with PACKED = false -- the
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
                    if !PACKED || g == g0 {
                        (ip as u32).wrapping_add(1)
                    } else {
                        0
                    }
                } else {
                    fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h, g)
                };
                (h, g, v)
            } else {
                (0usize, 0u8, 0u32)
            };
            if let Some((m, ml)) = (if WIDE {
                fast_probe_wide::<true>(&mut cand, src, m0, ip, window, lowest, mls, f_mask, block_end)
            } else {
                fast_probe(&mut cand, src, m0, ip, window, lowest, mls, block_end)
            })
                .filter(|&(_, ml)| !veto_block || ml >= ff_anchor_ml())
            {
                if COUNT {
                    if COUNT {
                        hits += 1;
                    }
                }
                ip = emit_fast_seq::<PACKED>(
                    src,
                    &mut hash_v,
                    &mut tags_v,
                    pack_eff,
                    f_wide,
                    f_mask,
                    f_shift,
                    &mut seqs,
                    &mut lits,
                    anchor,
                    ip,
                    m,
                    ml,
                    mls,
                    ilimit,
                    frame_start,
                    lp_copy,
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
                    if let Some(sq) = seqs.last() {
                        rep1 = sq.offset as usize;
                    }
                }
                if ip > ilimit {
                    break;
                }
                let (nh, ng) = fast_hash_tag::<true>(src, ip, WIDE, f_mask, f_shift);
                h0 = nh;
                g0 = ng;
                m0 = fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h0, g0);
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
        crate::prof::note_huff_path(12);
        push_lits_range(&mut lits, src, anchor, block_end);
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
        let (ls, lm) = lit_shares(&seqs);
        tables.lit_short_share = ls;
        tables.lit_mid_share = lm;
        tables.last_nseq = seqs.len();
        // DEFECT FIX: the main tail's `rep_len_ratio` update is BELOW this
        // return, so every pipelined block left Gate 2's second dispatch
        // variable pinned at its 1.0 initial value -- and the gate is `>= 1.0`.
        // See `replen_pipe_fixed`.
        if REP && replen_pipe_fixed() && rep_hits > 0 && !seqs.is_empty() {
            let all_bytes: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
            let rl = rep_bytes as f32 / rep_hits as f32;
            let al = all_bytes as f32 / seqs.len() as f32;
            if al > 0.0 {
                tables.rep_len_ratio = 0.75 * tables.rep_len_ratio + 0.25 * (rl / al);
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
        return (seqs, lits);
    }
    let (mut mm_total, mut mm_miss) = (0u64, 0u64);
    while ip <= ilimit {
        if COUNT {
            mm_total += 1;
        }
        if COUNT {
            if COUNT {
                probes += 1;
            }
        }
        let (h0, g0) = fast_hash_tag::<true>(src, ip, WIDE, f_mask, f_shift);
        let m0 = fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h0, g0);
        if COUNT && PACKED {
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
        fast_slot_store(&mut hash_v, &mut tags_v, pack_eff, h0, ip, g0);
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
        let pair_pre = if pair && ip + 1 <= ilimit {
            let (h1, g1) = fast_hash_tag::<false>(src, ip + 1, WIDE, f_mask, f_shift);
            Some((h1, g1, fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h1, g1)))
        } else {
            None
        };
        if REP {
            if COUNT {
                rep_probes += 1;
            }
            if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end) {
                rep_hits += 1;
                rep_bytes += ml as u64;
                if COUNT {
                    if COUNT {
                        hits += 1;
                    }
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
        if let Some((m, ml)) = (if WIDE {
                fast_probe_wide::<true>(&mut cand, src, m0, ip, window, lowest, mls, f_mask, block_end)
            } else {
                fast_probe(&mut cand, src, m0, ip, window, lowest, mls, block_end)
            })
                .filter(|&(_, ml)| !veto_block || ml >= ff_anchor_ml())
            {
            if COUNT {
                hits += 1;
            }
            ip = emit_fast_seq::<PACKED>(
                src,
                &mut hash_v,
                &mut tags_v,
                pack,
                f_wide,
                f_mask,
                f_shift,
                &mut seqs,
                &mut lits,
                anchor,
                ip,
                m,
                ml,
                mls,
                ilimit,
                frame_start,
                lp_copy,
            );
            anchor = ip;
            // Same decision as the pipelined loop -- see GATE 8 above. Guarding
            // only ONE loop would make the heuristic a property of which loop
            // ran, which is exactly the byte-identity break this gate exposed.
            if maintain_rep1 {
                if let Some(sq) = seqs.last() {
                    rep1 = sq.offset as usize;
                }
            }
            continue;
        }
        if pair {
            let ip1 = ip + 1;
            if ip1 <= ilimit {
                if COUNT {
                    if COUNT {
                        probes += 1;
                    }
                }
                pair_probes += 1;
                if COUNT {
                    use core::sync::atomic::Ordering::Relaxed;
                    if m0 == 0 { PAIR_M0_EMPTY.fetch_add(1, Relaxed); }
                    else { PAIR_M0_LIVE.fetch_add(1, Relaxed); }
                }
                if COUNT {
                    PAIR_PROBES.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                // Already issued above, next to the main probe's load.
                let (h1, g1, m1) = match pair_pre {
                    Some(v) => v,
                    None => {
                        let (h, g) = fast_hash_tag::<false>(src, ip1, WIDE, f_mask, f_shift);
                        (h, g, fast_slot_load::<PACKED>(&hash_v, &tags_v, pack_eff, h, g))
                    }
                };
                if COUNT && PACKED {
                    let raw = fast_slot_raw(&hash_v, pack_eff, h1);
                    if m1 == 0 && raw != 0 {
                        if fast_probe(&mut (0, 0), src, raw, ip1, window, lowest, mls, block_end).is_some() {
                            TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                        TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
                fast_slot_store(&mut hash_v, &mut tags_v, pack_eff, h1, ip1, g1);
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
                if let Some((m, ml)) = (if WIDE {
                    fast_probe_wide::<false>(&mut cand, src, m1, ip1, window, lowest, mls, f_mask, block_end)
                } else {
                    fast_probe(&mut cand, src, m1, ip1, window, lowest, mls, block_end)
                })
                    .filter(|&(_, ml)| !veto_block || ml >= ff_anchor_ml())
                {
                    if COUNT {
                        use core::sync::atomic::Ordering::Relaxed;
                        if m0 == 0 { PAIR_HIT_EMPTY.fetch_add(1, Relaxed);
                                     PAIR_BYTES_EMPTY.fetch_add(ml as u64, Relaxed); }
                        else { PAIR_HIT_LIVE.fetch_add(1, Relaxed);
                               PAIR_BYTES_LIVE.fetch_add(ml as u64, Relaxed); }
                        PAIR_HITS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        PAIR_BYTES.fetch_add(ml as u64, core::sync::atomic::Ordering::Relaxed);
                    }
                    if COUNT {
                        if COUNT {
                            hits += 1;
                        }
                    }
                    pair_bytes += ml as u64;
                    ip = emit_fast_seq::<PACKED>(
                        src,
                        &mut hash_v,
                        &mut tags_v,
                        pack_eff,
                        f_wide,
                        f_mask,
                        f_shift,
                        &mut seqs,
                        &mut lits,
                        anchor,
                        ip1,
                        m,
                        ml,
                        mls,
                        ilimit,
                        frame_start,
                        lp_copy,
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
    if REP && rep_hits > 0 && !seqs.is_empty() {
        let all_bytes: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        let rl = rep_bytes as f32 / rep_hits as f32;
        let al = all_bytes as f32 / seqs.len() as f32;
        if al > 0.0 {
            tables.rep_len_ratio = 0.75 * tables.rep_len_ratio + 0.25 * (rl / al);
        }
    }
    // GATE 7: feed this block's measured reject share to the next block's gate.
    tables.tag_yield = cand_yield(cand);
    // GATE 13: and this block's share of literal runs the fixed-width copy can catch.
    let (ls, lm) = lit_shares(&seqs);
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
    push_lits_range(&mut lits, src, anchor, block_end);
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
    (seqs, lits)
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
fn fast_probe_wide<const SAFE: bool>(
    cand: &mut (u64, u64),
    src: &[u8],
    match_slot: u32,
    ip: usize,
    window: usize,
    lowest: usize,
    mls: usize,
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
    if (a ^ b) & mask != 0 {
        cand.0 += 1;
        return None;
    }
    cand.1 += 1;
    #[cfg(feature = "profile")]
    FF_CAND4.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "profile")]
    FF_ACCEPT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    Some((m, mls + count_match(src, m + mls, ip + mls, block_end)))
}

fn fast_probe(
    cand: &mut (u64, u64),
    src: &[u8],
    match_slot: u32,
    ip: usize,
    window: usize,
    lowest: usize,
    mls: usize,
    block_end: usize,
) -> Option<(usize, usize)> {
    if match_slot == 0 {
        return None;
    }
    let m = (match_slot as usize) - 1;
    if m < lowest || m >= ip || ip - m > window {
        return None;
    }
    if load_u32le(src, m) != load_u32le(src, ip) {
        cand.0 += 1;
        return None;
    }
    cand.1 += 1;
    #[cfg(feature = "profile")]
    FF_CAND4.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let ml = 4 + count_match(src, m + 4, ip + 4, block_end);
    if ml >= mls {
        #[cfg(feature = "profile")]
        FF_ACCEPT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        Some((m, ml))
    } else {
        None
    }
}

/// Store a fast-strategy sequence. C extends the match backwards, then fills
/// hash(found_ip+2) and hash(end-2) from the *search* position, not the new start.
/// Defect B1 arm selector: back-fill the hash chain / binary tree over the
/// span a match covers, in lazy / lazy2 / btlazy2. `RZSTD_LAZY_FILL=0`
/// restores the pre-fix behaviour (jump past the match, insert nothing).
/// Ratio is deterministic, so this A/B needs exact byte counts, not timing.
/// Defect B1 arm: back-fill the chain over the span a match covers.
static LAZY_FILL_ENABLED_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA; shipping default from `RZSTD_LAZY_FILL`.
pub fn set_lazy_fill_arm(on: bool) {
    LAZY_FILL_ENABLED_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn lazy_fill_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match LAZY_FILL_ENABLED_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_LAZY_FILL")
                .map(|v| v != "0")
                .unwrap_or(true);
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
fn lazy_fill_threshold() -> f32 {
    use core::sync::atomic::Ordering;
    let v = LAZY_FILL_T_ARM.load(Ordering::Relaxed);
    if v != u32::MAX {
        return f32::from_bits(v);
    }
    let t: f32 = std::env::var("RZSTD_LAZY_FILL_T")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0.0);
    LAZY_FILL_T_ARM.store(t.to_bits(), Ordering::Relaxed);
    t
}

static LAZY_FILL_T_ARM: core::sync::atomic::AtomicU32 =
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
static BT_FILL_S_C: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

#[inline(always)]
fn bt_fill_stride() -> usize {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_FILL_S_C.load(Relaxed);
    if c != usize::MAX && bt_depth_cached() {
        return c;
    }
    #[cfg(feature = "std")]
    {
        let v = std::env::var("RZSTD_BT_FILL_S")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(1);
        BT_FILL_S_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static LF_FILLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static LF_NONEMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static LF_INSERTS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
fn lazy_fill_stride() -> usize {
    use core::sync::atomic::Ordering;
    let v = LAZY_FILL_S_ARM.load(Ordering::Relaxed);
    if v != 0 {
        return v;
    }
    let s: usize = std::env::var("RZSTD_LAZY_FILL_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .filter(|&v: &usize| v >= 1)
        .unwrap_or(1);
    LAZY_FILL_S_ARM.store(s, Ordering::Relaxed);
    s
}

static LAZY_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Set the lazy back-fill stride in-process.
/// GATE 6 next-long probe outcomes, for GATE 14's dispatch study.
pub static NL_PROBES_G: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static NL_HITS_G: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Total match-length GAIN the next-long probe bought, across its hits.
pub static NL_GAIN_G: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Hits in the RAISED band (`best_ml >= 8`) -- what a higher cut newly enables.
pub static NL_BAND_HITS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static NL_BAND_GAIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static NL_BAND_OLD: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Offsets the raised-band hits take, and the offsets they replace.
pub static NL_OFF_NEW: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static NL_OFF_OLD: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Raised-band hits whose new offset is LARGER than the one they replaced.
pub static NL_OFF_WORSE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
const NL_BAND_WARMUP: u32 = 2;
const NL_BAND_PERIOD: u32 = 16;

static NL_OFF_WORSE_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the worse-offset share above which the cut stays at 8.
pub fn set_nl_off_worse_arm(v: f32) {
    NL_OFF_WORSE_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn nl_off_worse_max() -> f32 {
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
static NL_DISPATCH_ON: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: enable the next-long raise + its offset-trade dispatch.
pub fn set_nl_dispatch_arm(on: bool) {
    NL_DISPATCH_ON.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn nl_cut_for(tables: &MatchTables) -> usize {
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
fn dfast_good_ml_raised() -> usize {
    let v = DFAST_GOOD_ML_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        24
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
static DFAST_GOOD_ML_ARM: core::sync::atomic::AtomicUsize =
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
static DFAST_GOOD_ML2_ARM: core::sync::atomic::AtomicUsize =
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
fn dfast_good_ml2() -> usize {
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
static DFAST_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Bench hook: interior back-fill stride for DFast. 0 restores today's two-ends fill.
pub fn set_dfast_fill_stride_arm(v: usize) {
    DFAST_FILL_S_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn dfast_fill_stride() -> usize {
    let v = DFAST_FILL_S_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v != usize::MAX {
        return v;
    }
    let s: usize = std::env::var("RZSTD_DFAST_FILL_S")
        .ok()
        .and_then(|v| v.parse().ok())
        .unwrap_or(0);
    DFAST_FILL_S_ARM.store(s, core::sync::atomic::Ordering::Relaxed);
    s
}

/// GATE 12 @ L3 work ledger: table WRITES performed by the two per-match end
/// fills (short and long counted separately). The sparse arm's saving is paid
/// in this unit; §4.39 priced only the main-loop positions it costs and so
/// called the arm "dominated" while ignoring the larger term.
pub static DF_ENDFILL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the per-match end-fill write count.
pub fn take_dfast_endfill() -> u64 {
    DF_ENDFILL.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// GATE 12 @ L3, the SPARSE direction. DFast writes four table entries per
/// match -- `match_ip+2` and `match_end-2`, into both the short and the long
/// hash -- unconditionally, and nothing has ever asked whether both earn it.
/// This is the only direction at L3 that REMOVES work.
///
/// 0 = unresolved, 1 = neither, 2 = start+2 only, 3 = today (both), 4 = end-2 only.
static DFAST_FILL_N_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: 0 = no end fills, 1 = start+2 only, 2 = both (today), 3 = end-2 only.
pub fn set_dfast_fill_n_arm(n: u8) {
    DFAST_FILL_N_ARM.store(n + 1, core::sync::atomic::Ordering::Relaxed);
}

/// `(fill_start, fill_end)` for the two per-match positions.
#[inline]
fn dfast_fill_ends() -> (bool, bool) {
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
static DFAST_FILL_A_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `true` anchors BOTH DFast fills on the committed match start.
pub fn set_dfast_fill_anchor_arm(c: bool) {
    DFAST_FILL_A_ARM.store(u8::from(c) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn dfast_fill_anchor_c() -> bool {
    DFAST_FILL_A_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}

/// Interior back-fill positions inserted by GATE 12 @ L3's stride arm.
pub static DF_FILL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
static REP1_MODE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

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
static REPLEN_PIPE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores the pre-fix behaviour (ratio never updated on
/// the pipelined path).
pub fn set_replen_pipe_arm(fixed: bool) {
    REPLEN_PIPE_ARM.store(u8::from(fixed) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn replen_pipe_fixed() -> bool {
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
static ACCEL_SHIFT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: search-strength shift. 8 is the shipped default.
pub fn set_accel_shift_arm(n: u32) {
    ACCEL_SHIFT_ARM.store(n, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn accel_shift_base() -> u32 {
    let v = ACCEL_SHIFT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v != u32::MAX {
        return v;
    }
    let n: u32 = std::env::var("RZSTD_ACCEL")
        .ok()
        .and_then(|v| v.trim().parse().ok())
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
fn accel_shift_for(strategy: Strategy) -> u32 {
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
fn rep_search_on(rep_yield: f32, strategy: Strategy) -> bool {
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
fn try_rep1(src: &[u8], ip: usize, rep1: usize, lowest: usize, block_end: usize) -> Option<usize> {
    let at = ip + 1;
    if rep1 == 0 || at + 4 > block_end || at < rep1 {
        return None;
    }
    let back = at - rep1;
    if back < lowest {
        return None;
    }
    if load_u32le(src, back) != load_u32le(src, at) {
        return None;
    }
    Some(4 + count_match_fast(src, back + 4, at + 4, block_end))
}

/// Base probe step for the Fast strategy when `target_length == 0`.
/// Probe-density arm (gg-matchfind Gate 9). Settable at RUNTIME so the harvest
/// can interleave both arms inside ONE process -- a `OnceLock` here made every
/// step0 measurement a separate process run, minutes apart, on a box that
/// drifts. 0 = not yet resolved, else `step0 + 1`.
static STEP0_ARM: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

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
const STEP_PROBE_BLOCKS: u32 = 1;
/// Bytes a sequence costs once entropy-coded, for the probe's size proxy.
/// Literal count plus this per sequence tracks emitted size closely enough to
/// rank two parses; coverage does not (4.70).
const SEQ_BYTES_EST: f64 = 3.0;
const STEP_REPROBE_PERIOD: u32 = 256;

static STEP_PROBE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

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
fn step_probe_on() -> bool {
    STEP_PROBE_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}

/// GATE 18 study: the measured step-2 forfeit, per mille x10.
#[cfg(feature = "profile")]
pub static STEP_FORFEIT_SUM: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STEP_FORFEIT_N: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STEP_SEQ_SUM: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

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
fn note_step_probe(
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
        tables.step_pick = if mean_forfeit < step_forfeit_max() { 2 } else { 1 };
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
fn note_step_outcome(tables: &mut MatchTables, _payload: usize, _block_len: usize) {
    if tables.step_pick != 0 && tables.step_reprobe > 0 {
        tables.step_reprobe -= 1;
    }
}

static STEP_FORFEIT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the share of match bytes step 2 may forfeit before it is refused.
pub fn set_step_forfeit_arm(v: f32) {
    STEP_FORFEIT_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn step_forfeit_max() -> f64 {
    let v = STEP_FORFEIT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        0.002
    } else {
        f64::from(f32::from_bits(v))
    }
}

static STEP_SEQ_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the share of sequences that may SURVIVE step 2 before it is
/// refused.
pub fn set_step_seq_arm(v: f32) {
    STEP_SEQ_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

fn step0_default() -> usize {
    use core::sync::atomic::Ordering;
    let v = STEP0_ARM.load(Ordering::Relaxed);
    if v != 0 {
        return v - 1;
    }
    let on = std::env::var("RZSTD_STEP0")
        .ok()
        .and_then(|v| v.parse().ok())
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
struct FastHash {
    wide: bool,
    mask: u64,
    shift: u32,
}

const FAST_HASH_PRIME64: u64 = 0x9E37_79B1_85EB_CA87;

#[inline(always)]
fn fast_hash_spec(mls: usize, hash_log: u32) -> FastHash {
    if fast_hash_wide_enabled() && (5..=8).contains(&mls) {
        FastHash {
            wide: true,
            mask: if mls == 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 },
            shift: 64u32.saturating_sub(hash_log),
        }
    } else {
        FastHash { wide: false, mask: 0, shift: 32u32.saturating_sub(hash_log) }
    }
}

/// Wide load with a zero-extended tail: several call sites are only 4-byte
/// safe (`ip + 1`, fill ends), so inside the last 8 bytes the missing bytes
/// read as zero. Deterministic and CONSISTENT between store and load -- a
/// tail-keyed slot can only ever be matched against the same tail key, and
/// every candidate is verified by compare + `count_match` regardless.
#[inline(always)]
fn load_u64le_tail(src: &[u8], pos: usize) -> u64 {
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
fn fast_hash_tag<const SAFE: bool>(
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
        ((hv >> shift) as usize, (hv ^ (hv >> 29)) as u8)
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
fn hash4_tag_mls(src: &[u8], pos: usize, hash_shift: u32, smask: u64) -> (usize, u8) {
    let v = load_u64le(src, pos);
    let hv = (v as u32).wrapping_mul(HASH4_PRIME);
    let tv = (v & smask).wrapping_mul(FAST_HASH_PRIME64);
    ((hv >> hash_shift) as usize, (tv ^ (tv >> 29)) as u8)
}


/// Brick 39 arm state: 2-way pipelined probe. Runtime-settable so the
/// in-process ABBA harness can flip it between adjacent measurements.
/// GATE 8 @ L1 reachability + speculation ledger for `find_fast`'s pipelined
/// loop, the same deterministic instrument that decided Gate 8 at L3.
/// How much of `find_fast`'s NON-pipelined (pair-route) loop would a
/// speculation serve? `MM_MISS / MM_TOTAL` is the share of positions that reach
/// the miss-advance, i.e. where a speculated next-position load is CONSUMED.
/// Gate 7 audit: tag rejections that `fast_probe` would have ACCEPTED.
pub static TAG_FALSE_REJECT: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static TAG_REJECT_TOTAL: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(false_rejects, total_rejects)`.
pub fn take_tag_rejects() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (TAG_FALSE_REJECT.swap(0, Relaxed), TAG_REJECT_TOTAL.swap(0, Relaxed))
}

/// GATE 2 candidate signal: rep match BYTES per rep PROBE.
pub static REP_PROBES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static REP_BYTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

pub static REP_HITS_G: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static ALL_MATCH_BYTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static ALL_SEQS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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

pub static MM_TOTAL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static MM_MISS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(main_loop_positions, positions_reaching_the_advance)`.
pub fn take_mm() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (MM_TOTAL.swap(0, Relaxed), MM_MISS.swap(0, Relaxed))
}

/// ffanat: dispatch-arm census. 0 = specialised (false,false,pipe),
/// 1 = tag arm (generic), 2 = rep arms (generic), 3 = rest.
#[cfg(feature = "profile")]
pub static FF_ARM: [core::sync::atomic::AtomicU64; 4] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Read and clear the dispatch-arm census.
#[cfg(feature = "profile")]
pub fn take_ff_arms() -> [u64; 4] {
    let mut o = [0u64; 4];
    for i in 0..4 { o[i] = FF_ARM[i].swap(0, core::sync::atomic::Ordering::Relaxed); }
    o
}

pub static FF_PIPE_BLOCKS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static FF_SPEC_MADE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static FF_SPEC_USED: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(pipelined_blocks, speculations_made, speculations_used)`.
pub fn take_ff_pipe() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        FF_PIPE_BLOCKS.swap(0, Relaxed),
        FF_SPEC_MADE.swap(0, Relaxed),
        FF_SPEC_USED.swap(0, Relaxed),
    )
}

static PIPE_REP1_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// A/B the pipelined loop's `rep1` maintenance. OFF reproduces the pre-fix
/// "sticky repcode" behaviour, which was an accident but is not obviously worse.
pub fn set_pipe_rep1_arm(on: bool) {
    PIPE_REP1_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn pipe_rep1_enabled() -> bool {
    match PIPE_REP1_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        _ => true,
    }
}

static PIPE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Override the pipeline arm for the rest of the process. Bench hook.
pub fn set_pipe_arm(on: bool) {
    PIPE_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn pipe_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match PIPE_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_MF_PIPE")
                .map(|v| v != "0")
                .unwrap_or(true);
            PIPE_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Phase C batch arm: Huffman literal-emit bricks 16 / 29 / 32.
/// `RZSTD_HUFF_FAST=0` selects the scalar twin (the byte-identity oracle).
static HUFF_FAST_ENABLED_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

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
            let on = std::env::var("RZSTD_HUFF_FAST")
                .map(|v| v != "0")
                .unwrap_or(true);
            HUFF_FAST_ENABLED_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Brick 44 arm: reserved block payload buffer.
static PAYLOAD_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook; shipping default comes from `RZSTD_PAYLOAD_RES`.
pub fn set_payload_arm(on: bool) {
    PAYLOAD_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn payload_reserve_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match PAYLOAD_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_PAYLOAD_RES")
                .map(|v| v != "0")
                .unwrap_or(true);
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
static LITPUSH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook; shipping default comes from `RZSTD_LIT_PUSH`.
/// Brick 77 A/B arm: hoisted flag (A) vs per-call read (B).
///
/// Brick 77 replaced `lit_push_enabled()` in `push_literals`' guard -- executed
/// 15,687,334 times across the corpus -- with a value threaded from the caller.
/// The two paths must be alternated IN-PROCESS to be measurable: a cross-process
/// comparison of two builds put C's own throughput 14-17% apart and left
/// `cyc/byte` and `C/us` disagreeing, because both were measuring the box.
static LITPUSH_HOIST_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `true` = use the hoisted parameter, `false` = re-read per call.
pub fn set_litpush_hoist_arm(on: bool) {
    LITPUSH_HOIST_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn litpush_hoist_enabled() -> bool {
    match LITPUSH_HOIST_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        _ => true,
    }
}

pub fn set_litpush_arm(on: bool) {
    LITPUSH_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn lit_push_enabled() -> bool {
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
            let on = std::env::var("RZSTD_LIT_PUSH")
                .map(|v| v != "0")
                .unwrap_or(true);
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
const WIDEN_RATIO: f32 = 0.0526;
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
static LIT_PUSH_TIERS_ARM: core::sync::atomic::AtomicU8 =
    core::sync::atomic::AtomicU8::new(0);

/// Bench hook: 0 all tiers, 1 tier-1 only, 2 tiers 1+2.
pub fn set_lit_push_tiers_arm(t: u8) {
    LIT_PUSH_TIERS_ARM.store(t, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn lit_push_tiers() -> u8 {
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
static DFAST_LITPUSH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` restores DFast's unreserved vectors and runtime-length
/// literal copies.
pub fn set_dfast_litpush_arm(on: bool) {
    DFAST_LITPUSH_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn dfast_litpush_enabled() -> bool {
    DFAST_LITPUSH_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// GATE 13 @ L1 signal: share of this block's literal runs short enough for the
/// fixed-width copy to catch.
///
/// Read off the emitted sequences rather than counted in the probe loop -- the
/// litlens are already there, so the signal costs one pass per BLOCK and nothing
/// per position. An empty block reports 1.0 so the gate stays open.
#[inline]
fn lit_shares(seqs: &[Seq]) -> (f32, f32) {
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
    (short as f32 / n, mid as f32 / n)
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
fn lit_width_for(tables: &MatchTables) -> usize {
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
const LIT_SHORT_MIN: f32 = 0.25;

/// Bench hook for the Gate 13 dispatch. Negative disables the gate (constant ON,
/// the pre-dispatch behaviour and the byte-identical fallback).
static LIT_SHORT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the Gate 13 share threshold. Negative = gate off (always take the guard).
pub fn set_lit_short_arm(v: f32) {
    LIT_SHORT_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn lit_short_min() -> f32 {
    let b = LIT_SHORT_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX { LIT_SHORT_MIN } else { f32::from_bits(b) }
}

/// Deterministic instrument: guard evaluations that FAILED, i.e. wasted work.
pub static LP_GUARD_FAIL: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Guard evaluations SKIPPED by the Gate 13 dispatch.
pub static LP_GUARD_SKIP: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Read and clear the Gate 13 guard instruments.
pub fn take_lp_guard() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        LP_GUARD_FAIL.swap(0, Ordering::Relaxed),
        LP_GUARD_SKIP.swap(0, Ordering::Relaxed),
    )
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
fn push_literals(lits: &mut Vec<u8>, src: &[u8], from: usize, to: usize, w: usize) {
    let n = to - from;
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
    if n <= w
        && from + w <= src.len()
        && lits.capacity() - lits.len() >= w
        && arm
    {
        #[cfg(feature = "profile")]
        LP_FAST.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        let len = lits.len();
        // SAFETY: `from + 16 <= src.len()` gives 16 readable source bytes;
        // `capacity - len >= 16` gives 16 writable destination bytes inside
        // the allocation. `src` (the input) and `lits` (a fresh scratch Vec)
        // are distinct buffers, so the regions cannot overlap. Exactly
        // `n <= 16` bytes are published by `set_len`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                src.as_ptr().add(from),
                lits.as_mut_ptr().add(len),
                w,
            );
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
fn push_literals_tiers(
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
pub static LP_HIST: [core::sync::atomic::AtomicU64; 6] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
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
pub static LP_FAST2: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static LP_FAST3: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
    (h, LP_FAST.swap(0, Ordering::Relaxed), LP_SLOW.swap(0, Ordering::Relaxed))
}

pub static LP_FAST: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static LP_SLOW: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
fn fast_slot_store(hash: &mut [u32], tags: &mut [u8], pack: bool, h: usize, pos: usize, tag: u8) {
    debug_assert!(h < hash.len());
    if pack {
        *unsafe { hash.get_unchecked_mut(h) } =
            (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
        return;
    }
    if let Some(t) = tags.get_mut(h) {
        *t = tag;
    }
    *unsafe { hash.get_unchecked_mut(h) } = (pos as u32).wrapping_add(1);
}

#[inline(always)]
#[allow(unsafe_code)]
fn fast_slot_load<const PACKED: bool>(
    hash: &[u32],
    tags: &[u8],
    pack: bool,
    h: usize,
    tag: u8,
) -> u32 {
    debug_assert!(h < hash.len());
    let e = *unsafe { hash.get_unchecked(h) };
    if e == 0 {
        return 0;
    }
    if pack {
        #[cfg(feature = "profile")]
        PACKED_TAG_READS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if PACKED && (e >> 24) as u8 != tag {
            return 0;
        }
        return e & 0x00FF_FFFF;
    }
    if !PACKED {
        return e;
    }
    if let Some(&t) = tags.get(h) {
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
const FF_NEAR_MAX: usize = 1 << 16;

/// Length bar (the surviving design): profile builds may override via
/// RZSTD_FF_ML for the sweep.
fn ff_anchor_ml() -> usize {
    #[cfg(feature = "profile")]
    {
        if let Ok(v) = std::env::var("RZSTD_FF_ML") {
            if let Ok(n) = v.parse() {
                return n;
            }
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
fn fast_hash_relatch(
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
fn fast_slot_raw(hash: &[u32], pack: bool, h: usize) -> u32 {
    let e = hash[h];
    if pack {
        e & 0x00FF_FFFF
    } else {
        e
    }
}

/// The Fast ladder's after-match end-fill on the LOCAL table -- same semantics
/// and same instruments as `fill_hash_after_match`, writing through the shared
/// `fast_slot_store` rule.
#[inline]
#[inline(always)]
fn fill_fast_after_match<const PACKED: bool>(
    hash: &mut [u32],
    tags: &mut [u8],
    pack: bool,
    f_wide: bool,
    f_mask: u64,
    f_shift: u32,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    ilimit: usize,
) {
    let (do_a, do_b) = dfast_fill_ends();
    let mut n = 0u64;
    let a = match_ip.saturating_add(2);
    if do_a && a <= ilimit {
        let (h, g) = fast_hash_tag::<true>(src, a, f_wide, f_mask, f_shift);
        fast_slot_store(hash, tags, pack, h, a, g);
        n += 1;
    }
    if do_b && match_end >= 2 {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let (h, g) = fast_hash_tag::<true>(src, b, f_wide, f_mask, f_shift);
            fast_slot_store(hash, tags, pack, h, b, g);
            n += 1;
        }
    }
    #[cfg(feature = "profile")]
    DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
    crate::prof::note_hash_fill(n);
}


#[inline(always)]
fn emit_fast_seq<const PACKED: bool>(
    src: &[u8],
    hash: &mut [u32],
    tags: &mut [u8],
    pack: bool,
    f_wide: bool,
    f_mask: u64,
    f_shift: u32,
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
    mls: usize,
    ilimit: usize,
    frame_start: usize,
    w: usize,
) -> usize {
    let _ = mls;
    let mut ip = found_ip;
    let mut mm = m;
    let mut n = ml;
    let back_from = ip;
    // T2's `back_eq`, finally applied HERE too. This is the Fast ladder's copy
    // of the exact back-extension walk that find_greedy/find_lazy/find_bt_lazy
    // had de-checked -- a PER-BYTE loop paying two bounds checks per extended
    // byte, running on EVERY match at L1/L2. Same proof: `ip > anchor` gives
    // `ip >= 1`, `mm > frame_start` gives `mm >= 1`, both start below the block
    // end and only decrease. Seventh instance of a capability present in one
    // path and absent in its neighbour.
    #[cfg(feature = "profile")]
    let bext_from = ip;
    while ip > anchor && mm > frame_start && back_eq(src, ip, mm) {
        ip -= 1;
        mm -= 1;
        n += 1;
    }
    #[cfg(feature = "profile")]
    note_bext((bext_from - ip) as u64);
    crate::prof::note_back_ext((back_from - ip) as u64);
    push_literals(lits, src, anchor, ip, w);
    seqs.push(Seq {
        litlen: (ip - anchor) as u32,
        matchlen: n as u32,
        offset: (ip - mm) as u32,
    });
    let end = ip + n;
    fill_fast_after_match::<PACKED>(hash, tags, pack, f_wide, f_mask, f_shift, src, found_ip, end, ilimit);
    end
}

/// C `zstd_fast.c` after a match: insert hash(start+2) and hash(end-2) only.
/// Filling every byte of a long match was ~src_len hash writes on repeating text.
#[inline]
fn fill_hash_after_match(
    tables: &mut MatchTables,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    // Block-hoisted: the arm atomic ran per call, twice per match across
    // both DFast fill helpers.
    ends: (bool, bool),
    smask: u64,
    // Shift from the table's OWN clamped hash_log -- never from `params`.
    // Passed IN rather than recomputed from the struct field: the caller's
    // spec copies hold it as a CONSTANT (dtag_shift from const hlog), and
    // whether LLVM re-proved the field unchanged here turned out to be
    // build-to-build unstable -- one emit folded these two shifts to
    // immediates, the next left them variable.
    hash_shift: u32,
    ilimit: usize,
) {
    let (do_a, do_b) = ends;
    let mut n = 0u64;
    let a = match_ip.saturating_add(2);
    if do_a && a <= ilimit {
        let (h, g) = hash4_tag_mls(src, a, hash_shift, smask);
        // T1: this helper runs after EVERY match on the DFast path too, so it
        // must write the short table in whatever representation the frame is
        // using. Writing it unpacked while the reader is packed decodes the tag
        // bits as part of the position -- which is exactly what it did, and it
        // moved output on 12 of 18 corpora.
        if tables.pack_tags {
            tables.put_h_tag(h, a, g);
        } else {
            tables.store_fast(h, a, g);
        }
        n += 1;
    }
    if do_b && match_end >= 2 {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let (h, g) = hash4_tag_mls(src, b, hash_shift, smask);
            if tables.pack_tags {
                tables.put_h_tag(h, b, g);
            } else {
                tables.store_fast(h, b, g);
            }
            n += 1;
        }
    }
    // Counted only under `--features profile`: this helper runs once per match,
    // so an unconditional atomic here is ~2M lock-prefixed ops per corpus pass
    // -- the same per-position atomic tax GATE 9 @ L1 removed from the Bt ladder.
    #[cfg(feature = "profile")]
    DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
    crate::prof::note_hash_fill(n);
}

#[inline]
fn fill_hash_long_after_match(
    tables: &mut MatchTables,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    hash_log: u32,
    ends: (bool, bool),
    smask: u64,
    // 1a: the short-tag shift, for the packed long store. Passed in like
    // `fill_hash_after_match`'s -- never re-derived from the struct field
    // (see 4a30eb4: that fold was build-to-build unstable).
    hash_shift: u32,
    ilimit: usize,
) {
    let (do_a, do_b) = ends;
    let mut n = 0u64;
    let a = match_ip.saturating_add(2);
    let ltag_live = tables.pack_tags || !tables.ltags.is_empty();
    if do_a && a <= ilimit {
        let g = if ltag_live { hash4_tag_mls(src, a, hash_shift, smask).1 } else { 0 };
        tables.put_hl_tag(hash8(src, a, hash_log), a, g);
        n += 1;
    }
    if do_b && match_end >= 2 {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let g = if ltag_live { hash4_tag_mls(src, b, hash_shift, smask).1 } else { 0 };
            tables.put_hl_tag(hash8(src, b, hash_log), b, g);
            n += 1;
        }
    }
    #[cfg(feature = "profile")]
    DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
    #[cfg(not(feature = "profile"))]
    let _ = n;
}

/// Split out for register allocation -- see brick 48 on `find_fast_impl`.
#[inline(never)]
fn find_dfast(
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
    macro_rules! go {
        ($h:expr) => {
            find_dfast_impl::<$h>(src, block_start, block_end, window, params, tables, reps)
        };
    }
    if !dfast_spec_enabled() {
        return go!(0);
    }
    // GATE 5: the MINIMAL COMPLETE set. Enumerated exhaustively over every
    // input size 0..2^28 plus the unknown-size (streaming) case, DFast reaches
    // exactly hash_log {14, 15, 16, 17, 18}. The first cut specialised 12..=20,
    // so 12/13/19/20 were dead monomorphizations -- roughly 2,100 instructions
    // of code that no input can execute, paid for in I-cache.
    match tables.hash_log {
        14 => go!(14),
        15 => go!(15),
        16 => go!(16),
        17 => go!(17),
        18 => go!(18),
        _ => go!(0),
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
fn find_dfast_impl<const HLOG: u32>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BMI2 twin per monomorphisation -- the DEFAULT level's finder. One
    // branch per block.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_dfast_impl_bmi2::<HLOG>(src, block_start, block_end, window, params, tables, reps)
        };
    }
    find_dfast_impl_inner::<HLOG>(src, block_start, block_end, window, params, tables, reps)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn find_dfast_impl_bmi2<const HLOG: u32>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_dfast_impl_inner::<HLOG>(src, block_start, block_end, window, params, tables, reps)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_dfast_impl_inner<const HLOG: u32>(
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
    let good_ml = nl_cut_for(tables);
    let good_ml2 = dfast_good_ml2();
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
    let lp = if litpush_hoist_enabled() {
        dfast_litpush_enabled()
    } else {
        lit_push_enabled()
    };
    let block_len = block_end - block_start;
    let seq_guess = (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16);
    // GATE 6 family: DFast reserved its buffers but still built them fresh every
    // block. `find_fast_impl` takes them from the frame; this never did, so the
    // reservation was paid per block instead of once. Same scratch, same
    // hand-back in `encode_block`.
    let keep = finder_scratch_enabled();
    let mut seqs = if keep {
        let mut v = std::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    if lp && seqs.capacity() < seq_guess {
        seqs = Vec::with_capacity(seq_guess);
    }
    let mut lits = if keep {
        let mut v = std::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    if lp && lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
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
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
    // GATE 6 @ L3 DISPATCH: run C's next-long probe only while it is EARNING.
    let nl_on = next_long_enabled() && tables.next_long_yield >= next_long_min();
    // `accel_shift_for(DFast)` is the constant 8 unless the RZSTD_ACCEL bench
    // pin is set; the pin stays available under `profile` (same treatment as
    // `find_fast_impl`'s loop, and `find_dfast` is dispatched for
    // Strategy::DFast only).
    let accel = if cfg!(feature = "profile") { accel_shift_for(params.strategy) } else { 8 };
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
    let (mut spec_made, mut spec_used) = (0u64, 0u64);
    // T1: the speculation now carries the short tag beside the short index.
    let mut carried: Option<(usize, u8, usize, Option<usize>, Option<usize>)> = None;
    // Read ONCE per block. `hash4_tag`'s index is `(v * HASH4_PRIME) >> shift`,
    // which is exactly what `hash4` computes, so the tagged path indexes the
    // same slots as `hash_mls(src, ip, 4, hlog)` did.
    let dtag_on = tables.pack_tags || !tables.tags.is_empty();
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
    let lt_on = long_tag_enabled() && (tables.pack_tags || !tables.ltags.is_empty());
    // The mls-width short tag's byte mask (min(mls, 8) bytes).
    let sk = 8.min(mls);
    let smask = if sk == 8 { u64::MAX } else { (1u64 << (8 * sk)) - 1 };
    // Loop-invariant arm reads, hoisted from the MATCH path to once per block.
    let fill_anchor_c = dfast_fill_anchor_c();
    let fill_stride = dfast_fill_stride();
    let fill_ends = dfast_fill_ends();
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
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                rep_hits += 1;
                d_rep_bytes += ml as u64;
                let mstart = ip + 1;
                push_literals(&mut lits, src, anchor, mstart, if lp { LIT_PUSH_WIDTH } else { 0 });
                seqs.push(Seq {
                    litlen: (mstart - anchor) as u32,
                    matchlen: ml as u32,
                    offset: rep1 as u32,
                });
                ip = mstart + ml;
                anchor = ip;
                carried = None;
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
        let (h4, g4, h8, m4, m8) = match carried.take() {
            Some(v) => {
                spec_used += 1;
                v
            }
            None => {
                let (a, ga, b) = dfast_hash_pair(src, ip, dtag_shift, smask, hlog);
                let m = tables.get_h_tag(a, ga, dtag_on);
                // T1 ledger: a rejection is a candidate load AVOIDED. Counted
                // only under `profile`, so the shipping loop is untouched.
                if COUNT && dtag_on {
                    if tables.raw_fast(a) != 0 {
                        TAG_REJECT_TOTAL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        if m.is_none() {
                            TAG_FALSE_REJECT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                    }
                }
                let ml8 = tables.get_hl_tag(b, ga, lt_on);
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
                            if match_ok(
                                src,
                                mr,
                                ip,
                                window,
                                block_start,
                                8.min(mls).max(4),
                                tables.frame_start,
                            ) && count_match(src, mr, ip, block_end) >= mls
                            {
                                LTAG_FALSE.fetch_add(1, Relaxed);
                            }
                        }
                    }
                }
                (a, ga, b, m, ml8)
            }
        };
        tables.put_h_tag(h4, ip, g4);
        tables.put_hl_tag(h8, ip, g4);
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
                let (a, ga, b) = dfast_hash_pair(src, nip, dtag_shift, smask, hlog);
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
                    tables.get_h_tag(a, ga, dtag_on)
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
                    tables.get_hl_tag(b, ga, lt_on)
                };
                spec_made += 1;
                carried = Some((a, ga, b, va, vb));
            }
        }

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(m8) = m8 {
            if COUNT {
                probes += 1;
            }
            let mlx = 8.min(mls).max(4);
            if match_ok(src, m8, ip, window, block_start, mlx, tables.frame_start) {
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
                    let mlx = 8.min(mls).max(4);
                    let lowest = block_start.saturating_sub(window).max(tables.frame_start);
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
        if best_ml < good_ml && nl_on && ip + 1 <= ilimit {
            nl_probes += 1;
            let h8b = hash8(src, ip + 1, hlog);
            // The only long consumer without a free tag: `ip + 1` never
            // computed a short hash. One mul+xor on a path already gated by
            // `best_ml < good_ml && nl_on`.
            let g8b = if lt_on { hash4_tag_mls(src, ip + 1, dtag_shift, smask).1 } else { 0 };
            if let Some(m8b) = tables.get_hl_tag(h8b, g8b, lt_on) {
                if COUNT {
                    probes += 1;
                }
                let mlx = 8.min(mls).max(4);
                if match_ok(src, m8b, ip + 1, window, block_start, mlx, tables.frame_start) {
                    // Count past match_ok's verified prefix.
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
                if match_ok(src, m4, ip, window, block_start, mls, tables.frame_start) {
                    // Count past match_ok's verified prefix.
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
                        let lowest =
                            block_start.saturating_sub(window).max(tables.frame_start);
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
            // commit at `best_ip`, which is `ip+1` when the next-long probe won
            push_literals(&mut lits, src, anchor, best_ip, if lp { LIT_PUSH_WIDTH } else { 0 });
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
            fill_hash_after_match(tables, src, best_ip, end, fill_ends, smask, dtag_shift, ilimit);
            // GATE 12 @ L3: `ip` here is the PRE-probe position; when the
            // next-long probe won, `best_ip == ip + 1` and the two tables index
            // different positions for one match. See `dfast_fill_anchor_c`.
            let long_anchor = if fill_anchor_c { best_ip } else { ip };
            fill_hash_long_after_match(tables, src, long_anchor, end, hlog, fill_ends, smask, dtag_shift, ilimit);
            // GATE 12 @ L3: the density knob DFast never had. Off by default.
            let dfs = fill_stride;
            if dfs != 0 {
                // `dtag_shift` IS `32 - tables.hash_log` (hlog mirrors the
                // struct field in both dispatch arms); recomputing it here
                // from the field kept a variable CL-shift alive in this arm
                // while every other hash4 site in the spec copies folded to
                // an immediate.
                let hash_shift = dtag_shift;
                let stop = end.saturating_sub(2).min(ilimit + 1);
                let mut p = best_ip + 2 + dfs;
                while p < stop {
                    let (h, g) = hash4_tag_mls(src, p, hash_shift, smask);
                    // T1: same table, same representation. `store_fast` writes
                    // the slot unpacked, which a packed reader would decode as a
                    // bogus position.
                    tables.put_h_tag(h, p, g);
                    tables.put_hl_tag(hash8(src, p, hlog), p, g);
                    #[cfg(feature = "profile")]
                    DF_FILL.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    p += dfs;
                }
            }
            ip = end;
            anchor = ip;
            // The two fills rewrite many entries, so anything speculated before
            // them is stale.
            carried = None;
        } else {
            ip += dstep + ((ip - anchor) >> accel);
        }
    }
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
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        MM_TOTAL.fetch_add(mm_total, Relaxed);
        DFAST_SPEC_MADE.fetch_add(spec_made, Relaxed);
        DFAST_SPEC_USED.fetch_add(spec_used, Relaxed);
    }
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
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
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
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        NL_PROBES_G.fetch_add(nl_probes, Relaxed);
        NL_HITS_G.fetch_add(nl_hits, Relaxed);
    }
    // GATE 14 @ L3: feed this block's measured offset trade to the next block.
    // Attribute ONLY when the band actually fired -- a block that measured
    // nothing must not move the EWMA, which is what would latch the gate.
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
    push_lits_range(&mut lits, src, anchor, block_end);
    // GATE 13 @ L3 FOLLOW-UP: `find_dfast` READ `last_nseq` to size its `seqs`
    // reservation but never WROTE it -- only `find_fast` did, and L3 never calls
    // `find_fast`. So the field sat at its initial 0 for the whole frame and the
    // guess collapsed to the `+ 64` floor, while DFast emits 5,685-13,763
    // sequences per block: the reservation was ~100x short and `seqs` still grew
    // by realloc (1,648 growths across the corpus).
    //
    // A capacity hint cannot affect output, so this is byte-identical.
    tables.last_nseq = seqs.len();
    note_finder_work(COUNT, probes, hits, &seqs, &lits);
    (seqs, lits)
}

/// GATE 9: DFast probe density. C's `_doubleFast` probes every position; 2 halves
/// the hash work at some ratio cost. Swept via `RZSTD_DFAST_STEP`.
/// Mean match length at or above which DFast may probe every OTHER position.
fn dfast_ml_min() -> f32 {
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
        let v: f32 = std::env::var("RZSTD_DFAST_ML")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(14.0);
        DFAST_ML_MIN_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    14.0
}
#[cfg(feature = "std")]
static DFAST_ML_MIN_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Non-zero forces a fixed density (measurement arm); 0 = dispatch.
fn dfast_step_forced() -> usize {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = DFAST_STEP_ARM.load(Ordering::Relaxed);
        if c != 0 {
            return c as usize;
        }
        let v: usize = std::env::var("RZSTD_DFAST_STEP")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(0);
        DFAST_STEP_ARM.store(v as u32, Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static DFAST_MATCH_BYTES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static DFAST_SEQS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static DFAST_BLOCK_BYTES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

pub static DFAST_BLOCKS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static DFAST_REP_BLOCKS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
/// Positions over which `try_rep1` is live -- the work a rep dispatch removes.
pub static DFAST_REP_POS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// `(blocks, rep_blocks, rep_positions)`
pub fn take_dfast_rep_blocks() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DFAST_BLOCKS.swap(0, Relaxed),
        DFAST_REP_BLOCKS.swap(0, Relaxed),
        DFAST_REP_POS.swap(0, Relaxed),
    )
}

pub static DFAST_REP_BYTES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static DFAST_REP_HITS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

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

static DFAST_STEP_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(0);

/// Set the DFast probe density in-process.
pub fn set_dfast_step_arm(v: usize) {
    DFAST_STEP_ARM.store(v as u32, core::sync::atomic::Ordering::Relaxed);
}

/// Blocks between forced DFast-pipeline re-probes.
const DFAST_PROBE_PERIOD: u32 = 16;

/// Minimum share of speculated loads that must be CONSUMED for the DFast
/// pipeline to run. Below it the speculation is net added work.
fn dfast_spec_min() -> f32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = DFAST_SPEC_MIN_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_DFAST_SPECMIN")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.70);
        DFAST_SPEC_MIN_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.70
}

static DFAST_SPEC_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the Gate 8 speculation-yield threshold in-process.
pub fn set_dfast_spec_min_arm(v: f32) {
    DFAST_SPEC_MIN_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

static DFAST_PIPE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// A/B the DFast 2-way software pipeline in-process -- both shapes, one binary,
/// so the comparison is immune to cross-binary drift.
pub static DFAST_SPEC_MADE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static DFAST_SPEC_USED: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(speculations_made, speculations_consumed)`.
pub fn take_dfast_spec() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (DFAST_SPEC_MADE.swap(0, Relaxed), DFAST_SPEC_USED.swap(0, Relaxed))
}

pub fn set_dfast_pipe_arm(on: bool) {
    DFAST_PIPE_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn dfast_pipe_enabled() -> bool {
    match DFAST_PIPE_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        _ => true,
    }
}

fn note_finder_work(count: bool, probes: u64, hits: u64, seqs: &[Seq], lits: &[u8]) {
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

/// Split out for register allocation -- see brick 48 on `find_fast_impl`.
#[inline(never)]
fn find_greedy(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // The chain-ladder hot loops hash with RUNTIME hash_log -- per-position
    // CL-shifts. The twin compiles the same selector and impls with BMI2.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_greedy_bmi2(src, block_start, block_end, window, params, tables, reps)
        };
    }
    find_greedy_sel(src, block_start, block_end, window, params, tables, reps)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn find_greedy_bmi2(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_greedy_sel(src, block_start, block_end, window, params, tables, reps)
}

#[inline(always)]
fn find_greedy_sel(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // GATE 4/5 for the chain ladder, NARROW: mls = 5 serves every default
    // row L5-L12 (clevels.h min_match), so ONE spec copy folds smask, the
    // mls_eq mask, the mls-branches and the hash path to constants. MLS = 0
    // is the runtime arm, served by the SAME body (the find_dfast_runtime
    // drift lesson).
    if params.min_match.max(3) == 5 {
        find_greedy_impl::<5>(src, block_start, block_end, window, params, tables, reps)
    } else {
        find_greedy_impl::<0>(src, block_start, block_end, window, params, tables, reps)
    }
}

#[inline(always)]
fn find_greedy_impl<const MLS: usize>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = if MLS == 0 { params.min_match.max(3) as usize } else { MLS };
    // BRICK 52, COMPLETED: the AUTHORITATIVE clamped value, never `params`.
    // `params.hash_log` is USER-SETTABLE with no upper bound (`hlog` in the
    // advanced-parameter setter does only `value.max(6)`), while the table is
    // allocated at `params.hash_log.clamp(6, 24)`. Indexing with the raw value
    // therefore ran off the end of a 2^24 table: `hlog >= 25` at L9 panicked
    // with `index out of bounds: the len is 16777216 but the index is
    // 28488790`. Brick 52 fixed `find_fast` and `find_dfast` and left the
    // chain-walking finders on the raw value.
    let hash_log = tables.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let attempts = search_attempts(params);
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut hits = 0u64;
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
    let mut seqs = if finder_scratch_enabled() {
        let mut v = std::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut lits = if finder_scratch_enabled() {
        let mut v = std::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 71: repcode-1 search in find_greedy -- L5-L6 had none
    // C checks `offset_1` at every position in `_greedy`/`_lazy` exactly as in
    // `_fast`/`_doubleFast`. Same dispatch on measured yield as bricks 67/70.
    let use_rep = rep_search_on(tables.rep_yield, params.strategy)
        || (rep_reprobe_enabled() && tables.rep_probe == 0);
    if rep_reprobe_enabled() {
        tables.rep_probe = if tables.rep_probe == 0 {
            REP_PROBE_PERIOD
        } else {
            tables.rep_probe - 1
        };
    }
    let mut rep1 = reps[0] as usize;
    let mut rep_hits = 0u64;
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
    // WALK-CONTINUE dispatch: see `walk_rep_max`.
    let walk_cont = walk_cont_enabled()
        && tables.rep_yield <= walk_rep_max()
        && (tables.walk_first_share <= walk_first_max(attempts) || tables.walk_probe == 0);
    tables.walk_probe = if tables.walk_probe == 0 {
        WALK_PROBE_PERIOD
    } else {
        tables.walk_probe - 1
    };
    let mut wcls = (0u32, 0u32);
    maybe_latch_wide_chain(tables, src, block_start, window, mls);
    let cp = tables.chain_pack;
    let ca = !tables.ctags.is_empty();
    let wchain = tables.chain_wide;
    let smask = if mls >= 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 };
    // The searches/byte signal feeds the wide latch's second route; greedy
    // never maintained it, so at L5 the field held its 1.0 INIT and the
    // route always passed (smallmsg +1.62% leak).
    let mut searches = 0u64;
    let mut ip = block_start;
    while ip <= ilimit {
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                rep_hits += 1;
                let mstart = ip + 1;
                push_lits_range(&mut lits, src, anchor, mstart);
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
        searches += 1;
        let (h, gtag) = if mls >= 8 && ip + 8 <= src.len() {
            (hash8(src, ip, hash_log), 0u8)
        } else if wchain {
            hash_wide_link_tag(src, ip, hash_log, smask)
        } else {
            hash4_link_tag(src, ip, hash_log, smask)
        };
        let (prev, head_tag) = tables.lz_insert(h, ip, gtag, cp, ca, chain_mask);

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(mut m) = prev {
            let mut mtag = head_tag;
            // See `chain_find_best`: the three per-step validity tests fold
            // to one monotone bound; `m >= ip` is entry-only.
            let low = lowest_rep.max(ip.saturating_sub(window));
            if m < ip && ip + mls <= src.len() {
                let mut missed_before = false;
                for _ in 0..attempts {
                    if m < low {
                        break;
                    }
                    // Link-tag reject: the tag rode in on the load that
                    // produced `m`, so a collision skips `mls_eq`'s src[m]
                    // load entirely. Sound: mls_eq true => 4 bytes equal =>
                    // tags equal.
                    // `m == 0` is ambiguous with the none-sentinel (whose
                    // fabricated tag is 0), and legacy walks probe position 0
                    // through it -- never tag-filter it (the 2-FALSE-skips
                    // catch on mozilla L5).
                    if (cp || ca) && m != 0 && mtag != gtag {
                        #[cfg(feature = "profile")]
                        if COUNT {
                            use core::sync::atomic::Ordering::Relaxed;
                            LINK_SKIPS.fetch_add(1, Relaxed);
                            if mls_eq(src, m, ip, mls, smask) {
                                LINK_FALSE.fetch_add(1, Relaxed);
                            }
                        }
                        missed_before = true;
                        if !walk_cont {
                            break;
                        }
                        let link = tables.chain_masked(m & chain_mask);
                        let next = if cp { (link & 0x00FF_FFFF) as usize } else { link as usize };
                        if next >= m {
                            break;
                        }
                        mtag = if cp { (link >> 24) as u8 } else { tables.ctags_masked(m & chain_mask) };
                        m = next;
                        continue;
                    }
                    if COUNT {
                        probes += 1;
                        #[cfg(feature = "profile")]
                        WALK_EXAM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    if mls_eq(src, m, ip, mls, smask) {
                        // C's `match[ml] == ip[ml]` prefilter
                        // (`ZSTD_HcFindBestMatch`): a candidate that DIFFERS at
                        // the current best length cannot exceed it, so the full
                        // `count_match` is provably wasted. The same candidate
                        // still wins, so this is byte-identical.
                        if best_ml == 0 || pre_eq(src, m, ip, best_ml) {
                            // Count past mls_eq's verified prefix (see
                            // `chain_find_best`).
                            let ml = mls + count_match_fast(src, m + mls, ip + mls, block_end);
                            if ml >= mls && ml > best_ml {
                                if missed_before {
                                    if best_ml == 0 {
                                        wcls.0 += 1;
                                    } else {
                                        wcls.1 += 1;
                                    }
                                }
                                best_ml = ml;
                                best_m = m;
                                // Reaches the block end -- nothing can be longer.
                                if ip + best_ml >= block_end {
                                    break;
                                }
                            }
                        }
                    } else {
                        missed_before = true;
                        #[cfg(feature = "profile")]
                        if COUNT {
                            WALK_BYTEMISS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                        if !walk_cont {
                            break;
                        }
                    }
                    let link = tables.chain_masked(m & chain_mask);
                    let next = if cp { (link & 0x00FF_FFFF) as usize } else { link as usize };
                    if next >= m {
                        break;
                    }
                    mtag = if cp { (link >> 24) as u8 } else if ca { tables.ctags_masked(m & chain_mask) } else { 0 };
                    m = next;
                }
            }
        }
        if best_ml >= mls {
            if COUNT {
                hits += 1;
            }
            // DEFECT B3 FIX: back-extend the match (C's "catch up" loop in
            // `ZSTD_compressBlock_lazy_generic`). `emit_fast_seq` -- i.e.
            // fast/dfast -- has always done this; greedy and lazy never did,
            // so every literal that also sat just before the match stayed a
            // literal. The offset is unchanged, so validity is preserved: only
            // `litlen` shrinks and `matchlen` grows by the same amount.
            let mut s = ip;
            let mut mm = best_m;
            let mut n = best_ml;
            #[cfg(feature = "profile")]
            let bext_from = s;
            while s > anchor && mm > tables.frame_start && back_eq(src, s, mm) {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            #[cfg(feature = "profile")]
            note_bext((bext_from - s) as u64);
            push_lits_range(&mut lits, src, anchor, s);
            seqs.push(Seq {
                litlen: (s - anchor) as u32,
                matchlen: n as u32,
                offset: (s - mm) as u32,
            });
            rep1 = ip - best_m;
            let end = ip + best_ml;
            // Positions `s..=ip` were ALREADY inserted as the loop walked to
            // `ip`; re-inserting them would self-loop the chain (see B2).
            let mut p = ip + 1;
            while p < end && p <= ilimit {
                let (hh, gt) = if mls >= 8 && p + 8 <= src.len() {
                    (hash8(src, p, hash_log), 0u8)
                } else if wchain {
                    hash_wide_link_tag(src, p, hash_log, smask)
                } else {
                    hash4_link_tag(src, p, hash_log, smask)
                };
                let _ = tables.lz_insert(hh, p, gt, cp, ca, chain_mask);
                p += 1;
            }
            ip = end;
            anchor = ip;
        } else {
            ip += 1;
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    update_walk_first_share(tables, walk_cont, wcls, attempts);
    let span = (block_end - block_start).max(1) as f32;
    tables.last_search_per_byte = searches as f32 / span;
    push_lits_range(&mut lits, src, anchor, block_end);
    note_finder_work(COUNT, probes, hits, &seqs, &lits);
    (seqs, lits)
}

#[allow(clippy::too_many_arguments)]
// REFUTED (2026-08-22): #[inline(always)] into find_lazy. Static size 1,000
// -> 1,753 (two inlined copies) with unknowable spill delta -- there is no
// deterministic executed-instruction receipt for an inlining decision, and
// brick 48 chose OUTLINING for exactly this shape. The call overhead stays.
// Brick 48 REVISITED under the twin architecture: outlining is PRESERVED
// (both arms carry #[inline(never)]), and the ISA choice moves to the caller
// as a per-block `ChainFn` pointer -- the BtFn precedent. The plain arm's
// work is unchanged; the twin arm compiles the same body with BMI2.
type ChainFn = fn(
    &[u8],
    usize,
    usize,
    usize,
    usize,
    usize,
    usize,
    bool,
    &mut (u32, u32),
    &mut MatchTables,
) -> (usize, usize);

#[inline(never)]
fn chain_find_best<const MLS: usize>(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    walk_cont: bool,
    cls: &mut (u32, u32),
    tables: &mut MatchTables,
) -> (usize, usize) {
    chain_find_best_inner::<MLS>(
        src, ip, block_start, block_end, window, mls, attempts, walk_cont, cls, tables,
    )
}

/// Safe `ChainFn`-shaped wrapper; handed out only behind `has_bmi2()`.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
fn chain_find_best_bmi2_ptr<const MLS: usize>(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    walk_cont: bool,
    cls: &mut (u32, u32),
    tables: &mut MatchTables,
) -> (usize, usize) {
    // SAFETY: only selected under the caller's CPUID guard.
    #[allow(unsafe_code)]
    unsafe {
        chain_find_best_bmi2::<MLS>(
            src, ip, block_start, block_end, window, mls, attempts, walk_cont, cls, tables,
        )
    }
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
#[inline(never)]
unsafe fn chain_find_best_bmi2<const MLS: usize>(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    walk_cont: bool,
    cls: &mut (u32, u32),
    tables: &mut MatchTables,
) -> (usize, usize) {
    chain_find_best_inner::<MLS>(
        src, ip, block_start, block_end, window, mls, attempts, walk_cont, cls, tables,
    )
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn chain_find_best_inner<const MLS: usize>(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    // Hoisted by the caller once per block: `search_attempts` reads its arm
    // atomic, and this function runs per position plus per look-ahead step.
    attempts: usize,
    // Also caller-hoisted (arm atomic + rep_yield dispatch), once per block.
    walk_cont: bool,
    // Block-local (first-find, upgrade) accept counters past a collision --
    // the walk gate's own dispatch signal. Plain integers, never atomics.
    cls: &mut (u32, u32),
    tables: &mut MatchTables,
) -> (usize, usize) {
    // BRICK 52, COMPLETED: the AUTHORITATIVE clamped value, never `params`.
    // `params.hash_log` is USER-SETTABLE with no upper bound (`hlog` in the
    // advanced-parameter setter does only `value.max(6)`), while the table is
    // allocated at `params.hash_log.clamp(6, 24)`. Indexing with the raw value
    // therefore ran off the end of a 2^24 table: `hlog >= 25` at L9 panicked
    // with `index out of bounds: the len is 16777216 but the index is
    // 28488790`. Brick 52 fixed `find_fast` and `find_dfast` and left the
    // chain-walking finders on the raw value.
    let mls = if MLS == 0 { mls } else { MLS };
    let hash_log = tables.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let cp = tables.chain_pack;
    let ca = !tables.ctags.is_empty();
    let wchain = tables.chain_wide;
    let smask = if mls >= 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 };
    let (h, gtag) = if mls >= 8 && ip + 8 <= src.len() {
        (hash8(src, ip, hash_log), 0u8)
    } else if wchain {
        hash_wide_link_tag(src, ip, hash_log, smask)
    } else {
        hash4_link_tag(src, ip, hash_log, smask)
    };
    let (prev, head_tag) = tables.lz_insert(h, ip, gtag, cp, ca, chain_mask);
    // P0/gg-matchfind: candidate examinations are the WORK COUNTER, the primary
    // evidence under the Great Gate 2026-08-06 law. Compiled out entirely when
    // the profile feature is off.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut best_m = 0usize;
    let mut best_ml = 0usize;
    let Some(mut m) = prev else {
        return (0, 0);
    };
    let mut mtag = head_tag;
    let lowest = block_start.saturating_sub(window).max(tables.frame_start);
    // The walk's THREE per-step validity tests fold to ONE: `m >= ip` can
    // only fire on ENTRY (afterwards m strictly decreases below ip), and the
    // window and lowest checks are both lower bounds on m, merged into a
    // per-walk constant. `ip - m > window  <=>  m < ip - window` for m < ip.
    let low = lowest.max(ip.saturating_sub(window));
    let mut missed_before = false;
    if m < ip && ip + mls <= src.len() {
        for _ in 0..attempts {
            // Monotone: m only decreases, so one bound test per step.
            if m < low {
                break;
            }
            // Link-tag reject: skip `mls_eq`'s src[m] load on a tag byte the
            // link load already delivered. Sound: mls_eq true => 4 bytes
            // equal => tags equal.
            // See the greedy walk: position 0 is sentinel-ambiguous, never
            // tag-filtered.
            if (cp || ca) && m != 0 && mtag != gtag {
                #[cfg(feature = "profile")]
                if COUNT {
                    use core::sync::atomic::Ordering::Relaxed;
                    LINK_SKIPS.fetch_add(1, Relaxed);
                    if mls_eq(src, m, ip, mls, smask) {
                        LINK_FALSE.fetch_add(1, Relaxed);
                    }
                }
                missed_before = true;
                if !walk_cont {
                    break;
                }
                let link = tables.chain_masked(m & chain_mask);
                let next = if cp { (link & 0x00FF_FFFF) as usize } else { link as usize };
                if next >= m {
                    break;
                }
                mtag = if cp { (link >> 24) as u8 } else { tables.ctags_masked(m & chain_mask) };
                m = next;
                continue;
            }
            if COUNT {
                probes += 1;
                #[cfg(feature = "profile")]
                WALK_EXAM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            if mls_eq(src, m, ip, mls, smask) {
                // C's `match[ml] == ip[ml]` prefilter -- see `find_greedy`.
                if best_ml == 0 || pre_eq(src, m, ip, best_ml) {
                    // Count from the byte AFTER what mls_eq just verified --
                    // restarting at 0 re-compared the first word of every
                    // candidate (the fast_probe_wide rule, applied here).
                    let ml = mls + count_match_fast(src, m + mls, ip + mls, block_end);
                    // offset_ok and the frame_start floor are GUARANTEED by
                    // the walk bound (m >= low >= lowest >= frame_start,
                    // m >= ip - window, m < ip); re-checking per accept was
                    // pure redundancy.
                    if ml >= mls && ml > best_ml {
                        if missed_before {
                            if best_ml == 0 {
                                cls.0 += 1;
                            } else {
                                cls.1 += 1;
                            }
                            #[cfg(feature = "profile")]
                            if COUNT {
                                use core::sync::atomic::Ordering::Relaxed;
                                if best_ml == 0 {
                                    WALK_CONT_FIRST.fetch_add(1, Relaxed);
                                } else {
                                    WALK_CONT_UPGRADE.fetch_add(1, Relaxed);
                                }
                            }
                        }
                        best_ml = ml;
                        best_m = m;
                        if ip + best_ml >= block_end {
                            break;
                        }
                    }
                }
            } else {
                missed_before = true;
                #[cfg(feature = "profile")]
                if COUNT {
                    WALK_BYTEMISS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                // A byte mismatch is a hash collision, not a wall: C steps to
                // the next link. Legacy arm preserves the historical break.
                if !walk_cont {
                    break;
                }
            }
            let link = tables.chain_masked(m & chain_mask);
            let next = if cp { (link & 0x00FF_FFFF) as usize } else { link as usize };
            if next >= m {
                break;
            }
            mtag = if cp { (link >> 24) as u8 } else if ca { tables.ctags_masked(m & chain_mask) } else { 0 };
            m = next;
        }
    }
    if COUNT {
        crate::prof::note_probes(probes);
    }
    (best_m, best_ml)
}

/// Split out for register allocation -- see brick 48 on `find_fast_impl`.
#[inline(never)]
fn find_lazy(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // See `find_greedy`: the twin covers the runtime-hash_log CL-shifts.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            find_lazy_bmi2(src, block_start, block_end, window, params, tables, depth, reps)
        };
    }
    find_lazy_sel(src, block_start, block_end, window, params, tables, depth, reps)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
unsafe fn find_lazy_bmi2(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    find_lazy_sel(src, block_start, block_end, window, params, tables, depth, reps)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_lazy_sel(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // See `find_greedy`: narrow MLS spec, runtime arm from the same body.
    if params.min_match.max(3) == 5 {
        find_lazy_impl::<5>(src, block_start, block_end, window, params, tables, depth, reps)
    } else {
        find_lazy_impl::<0>(src, block_start, block_end, window, params, tables, depth, reps)
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_lazy_impl<const MLS: usize>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = if MLS == 0 { params.min_match.max(3) as usize } else { MLS };
    // BRICK 52, COMPLETED: the AUTHORITATIVE clamped value, never `params`.
    // `params.hash_log` is USER-SETTABLE with no upper bound (`hlog` in the
    // advanced-parameter setter does only `value.max(6)`), while the table is
    // allocated at `params.hash_log.clamp(6, 24)`. Indexing with the raw value
    // therefore ran off the end of a 2^24 table: `hlog >= 25` at L9 panicked
    // with `index out of bounds: the len is 16777216 but the index is
    // 28488790`. Brick 52 fixed `find_fast` and `find_dfast` and left the
    // chain-walking finders on the raw value.
    let hash_log = tables.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let attempts = search_attempts(params);
    // Per-block ISA selection for the outlined walk (brick 48 + twin).
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    let cfb: ChainFn = if crate::simd::has_bmi2() {
        chain_find_best_bmi2_ptr::<MLS>
    } else {
        chain_find_best::<MLS>
    };
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    let cfb: ChainFn = chain_find_best::<MLS>;
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
    let scratch = finder_scratch_enabled();
    let mut seqs = if scratch {
        let mut v = std::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut lits = if scratch {
        let mut v = std::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 71: repcode-1 search in find_lazy -- L7-L12 had none
    // C checks `offset_1` at every position in `_greedy`/`_lazy` exactly as in
    // `_fast`/`_doubleFast`. Same dispatch on measured yield as bricks 67/70.
    let use_rep = rep_search_on(tables.rep_yield, params.strategy)
        || (rep_reprobe_enabled() && tables.rep_probe == 0);
    if rep_reprobe_enabled() {
        tables.rep_probe = if tables.rep_probe == 0 {
            REP_PROBE_PERIOD
        } else {
            tables.rep_probe - 1
        };
    }
    let mut rep1 = reps[0] as usize;
    let mut rep_hits = 0u64;
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
    let mut ip = block_start;
    let mut searches = 0u64;
    // GATE 3 @ L1 -- CONSTANT OFF when the caller is the Fast ladder.
    //
    // `find_lazy` is reachable at L1 ONLY through the Gate 1 dispatch, which
    // leaves `params.strategy == Fast`, so that flag identifies the routed case
    // exactly. There the back-fill is a REGRESSION:
    //
    //   L1 (routed)  versions-16m  OFF 46,025  ON 47,037   ON is +2.199% WORSE
    //   L7 (native)  every corpus wins with ON: mr -6.237%, webster -5.580%,
    //                xml -3.505%, nci -2.358%, jsonlog -1.384%, sao -0.901%
    //
    // Not a content split -- at L7 no corpus loses. It is a PARAMETER split:
    // L1 has `chain_log` 13 (8,192 entries) against L7's 19 (524,288), 64x
    // smaller. Filling every position a long match covers floods a chain that
    // size and evicts the entries the next search needs. The fill's value is
    // conditional on there being room for it.
    let fill = lazy_fill_enabled()
        && params.strategy != Strategy::Fast
        && tables.last_search_per_byte >= lazy_fill_threshold();
    // Per-match arm read hoisted to once per block.
    let fill_stride = lazy_fill_stride();
    // WALK-CONTINUE dispatch: see `walk_rep_max`.
    let walk_cont = walk_cont_enabled()
        // GATE 3's rule for the L1-routed case: `find_lazy` reachable with
        // `strategy == Fast` is the Gate 1 dispatch, and the C-parity walk
        // must not change the Fast ladder's bytes.
        && params.strategy != Strategy::Fast
        && tables.rep_yield <= walk_rep_max()
        && (tables.walk_first_share <= walk_first_max(attempts) || tables.walk_probe == 0);
    tables.walk_probe = if tables.walk_probe == 0 {
        WALK_PROBE_PERIOD
    } else {
        tables.walk_probe - 1
    };
    let mut wcls = (0u32, 0u32);
    maybe_latch_wide_chain(tables, src, block_start, window, mls);
    let cp = tables.chain_pack;
    let ca = !tables.ctags.is_empty();
    let wchain = tables.chain_wide;
    let smask = if mls >= 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 };
    let gain_cmp = lazy_gain_enabled();
    while ip <= ilimit {
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                rep_hits += 1;
                let mstart = ip + 1;
                push_lits_range(&mut lits, src, anchor, mstart);
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
        searches += 1;
        let (mut best_m, mut best_ml) =
            cfb(src, ip, block_start, block_end, window, mls, attempts, walk_cont, &mut wcls, tables);
        let mut best_ip = ip;
        let mut look_hi = ip; // PROBE: highest position the look-ahead inserted
        if best_ml >= mls {
            for d in 1..=depth {
                let ip2 = ip + d;
                if ip2 > ilimit {
                    break;
                }
                look_hi = ip2;
                let (m, ml) = cfb(
                    src,
                    ip2,
                    block_start,
                    block_end,
                    window,
                    mls,
                    attempts,
                    walk_cont,
                    &mut wcls,
                    tables,
                );
                let take = if gain_cmp {
                    // C parity: the +4 favors the match already in hand.
                    ml >= mls
                        && lazy_gain(ml, ip2 - m)
                            > lazy_gain(best_ml, best_ip - best_m) + 4
                } else {
                    ml > best_ml
                };
                if take {
                    best_ml = ml;
                    best_m = m;
                    best_ip = ip2;
                }
            }
        }
        if best_ml >= mls {
            // DEFECT B3 FIX: back-extend the match -- see `find_greedy`.
            let mut s = best_ip;
            let mut mm = best_m;
            let mut n = best_ml;
            #[cfg(feature = "profile")]
            let bext_from = s;
            while s > anchor && mm > tables.frame_start && back_eq(src, s, mm) {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            #[cfg(feature = "profile")]
            note_bext((bext_from - s) as u64);
            push_lits_range(&mut lits, src, anchor, s);
            seqs.push(Seq {
                litlen: (s - anchor) as u32,
                matchlen: n as u32,
                offset: (s - mm) as u32,
            });
            // The repcode must track the offset ACTUALLY EMITTED. Lazy
            // commits at `best_ip` (the look-ahead winner), not `ip`.
            rep1 = best_ip - best_m;
            // DEFECT B1 FIX: back-fill every position the match covers.
            // `find_greedy` already did this; lazy/lazy2 jumped straight to
            // `best_ip + best_ml`, so every byte inside a match was absent
            // from the chain. On matchy content that is most of the file, so
            // later searches saw a nearly empty chain and found worse matches
            // -- which is why ratio DEGRADED as the level rose. C achieves the
            // same thing via `nextToUpdate` back-filling inside
            // `ZSTD_insertAndFindFirstIndex`.
            let end = best_ip + best_ml;
            if fill {
                // Stride the back-fill. `1` = every position (C's behaviour).
                // Larger strides thin the chain: the cost of the back-fill is
                // the chain DENSITY it creates, not the inserts themselves.
                let stride = fill_stride;
                // DEFECT B2 FIX: never insert a position TWICE. The look-ahead
                // already inserted `ip+1 ..= look_hi` via `chain_find_best`, and
                // re-inserting `p` stores `chain[p] = get_h(h)` when the head IS
                // already `p` -- i.e. `chain[p] = p`, a self-loop. The walk's
                // `next >= m` guard then breaks on it, so the whole bucket's
                // history below `p` is unreachable FOREVER. Measured on osdb:
                // 501,705 such amputations at L7 and 791,088 at L9 (10.9% of all
                // back-fill inserts) -- which is why lazy/lazy2 emitted MORE bytes
                // than the cheaper dfast below them. C cannot hit this: its
                // `nextToUpdate` cursor is monotone, so every position is inserted
                // exactly once.
                let mut p = (best_ip + 1).max(look_hi + 1);
                // Consumers are `take_lazy_fill` gate harnesses only; in
                // shipping LF_INSERTS was one lock-prefixed RMW PER COVERED
                // POSITION -- the pair-tail class (959e0ae), on the matchiest
                // content the heaviest.
                #[cfg(feature = "profile")]
                {
                    LF_FILLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    if p < end && p <= ilimit {
                        LF_NONEMPTY.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
                while p < end && p <= ilimit {
                    #[cfg(feature = "profile")]
                    LF_INSERTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    let (hh, gt) = if mls >= 8 && p + 8 <= src.len() {
                        (hash8(src, p, hash_log), 0u8)
                    } else if wchain {
                        hash_wide_link_tag(src, p, hash_log, smask)
                    } else {
                        hash4_link_tag(src, p, hash_log, smask)
                    };
                    let _ = tables.lz_insert(hh, p, gt, cp, ca, chain_mask);
                    p += stride;
                }
            }
            ip = end;
            anchor = ip;
        } else {
            ip += 1;
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    update_walk_first_share(tables, walk_cont, wcls, attempts);
    push_lits_range(&mut lits, src, anchor, block_end);
    let span = (block_end - block_start).max(1) as f32;
    tables.last_search_per_byte = searches as f32 / span;
    // Signal probe for the wide-chain latch design (profile only): expose
    // the block-signal EWMAs so a harness can see what separates the
    // first-heavy winners (sao) from the first-heavy losers (smallmsg).
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        WALK_SIG_FIRST.store(tables.walk_first_share.to_bits(), Relaxed);
        WALK_SIG_REP.store(tables.rep_yield.to_bits(), Relaxed);
        WALK_SIG_SPB.store(tables.last_search_per_byte.to_bits(), Relaxed);
        let mb: u64 = seqs.iter().map(|q| q.matchlen as u64).sum();
        let ob: u64 = seqs.iter().map(|q| 64 - u64::from(q.offset.max(1)).leading_zeros() as u64).sum();
        WALK_SIG_MB.store(mb, Relaxed);
        WALK_SIG_NS.store(seqs.len() as u64, Relaxed);
        WALK_SIG_OB.store(ob, Relaxed);
    }
    // `searches` is SEARCH POSITIONS, not candidate examinations -- reporting it
    // as `probes` was a work-count parity break against `find_fast`. The real
    // probe count comes from `chain_find_best` via `note_probes`, so pass 0.
    note_finder_work(cfg!(feature = "profile"), 0, seqs.len() as u64, &seqs, &lits);
    (seqs, lits)
}

/// Gate 14 (gg-matchfind) arm: chain-walk depth. `attempts = 1 << search_log`
/// is a pure LEVEL constant today, and section 6 of m7-anatomy found our ratio
/// gains LESS per level than C's -- so the marginal return on this exact
/// constant is the campaign's top open question. Settable at runtime so the
/// harvest can A/B it in-process. 0 = unset (delta 0); else `delta + 8`.
static SEARCH_LOG_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);

/// Bench hook: shift the chain-walk depth exponent by `delta` (clamped -4..=4).
pub fn set_search_log_delta(delta: i32) {
    SEARCH_LOG_ARM.store(
        (delta.clamp(-4, 4) + 8) as u32,
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// Candidate examinations the chain walk is allowed, for this level and arm.
/// How many halvings to take off the chain-walk depth for this content.
/// Rep-dominated content keeps the full depth; everything else gives up one step
/// for -9.2% of all bt probes at +0.001% size.
///
/// The signal is `opt_rep_rate`, NOT `rep_yield`: `find_opt` never updates
/// `rep_yield`, so at L16+ it sits at its initial 1.0 forever and a gate keyed on
/// it is dead. `opt_rep_rate` is maintained by `find_opt` itself (Gate 10) and
/// separates the content that needs the depth -- versions-16m 434 bytes/probe and
/// text-32m 26,932 against a maximum of 35.6 for everything else.
///
/// Restricted to the opt strategies: at L13 (BtLazy2) the same cut removes 29.8%
/// of probes but costs +1.60% size (reymont +7.94%), so it is not applied there.
/// Clamp the walk budget to `bt_depth_target()` where the gate allows.
///
/// A TARGET, not a shift, because the shift that works is level-dependent while
/// the target is not: L19 (128 attempts) wants -2 and L22 (512) wants -4, and
/// both land on 32 with the SAME worst corpus (nci +0.132%). Mean walk depth is
/// 10.6 at L19 and 12.4 at L22, so 32 is about 3x the mean and still covers the
/// tail.
#[inline]
fn bt_depth_apply(attempts: usize, params: CompressionParameters, opt_rep_rate: f32) -> usize {
    if bt_depth_cut(params, opt_rep_rate) == 0 {
        attempts
    } else {
        attempts.min(bt_depth_target_for(opt_rep_rate))
    }
}

/// GATE 14 @ L19 DISPATCH: a DEEPER cut where the tree walk is not paying for
/// its depth, on a signal the encoder already maintains.
///
/// Cutting 32 -> 24 is free on three corpora and costs 0.257% on nci. What
/// separates them is `opt_rep_rate`, already computed per block for GATE 10:
///
/// ```text
///   mr       36.25    probes -11.81%   size +0.003%   time  -9.70%
///   mozilla  29.35    probes  -6.52%   size -0.015%   time  -0.85%
///   samba     4.39    probes  -6.22%   size +0.011%   time  -3.90%
///   ---------------- threshold 2.0 ----------------
///   nci       0.97    probes  -5.02%   size +0.257%   NOT CUT
///   all others <=0.65 probes  ~0%      size  ~0%
/// ```
///
/// A 4.5x gap, and zero instrumentation cost -- the alternative signal
/// (no-gain probe share) would have needed a counter on a 264M-probe path to
/// separate samba 78.5% from nci 78.1%, which it does not do anyway.
///
/// Content with a high repcode rate has many equal-prefix candidates in the
/// tree; walking past 24 of them re-finds matches the repcode already covers.
/// `versions-16m` (rate 6028) is excluded a level up by `bt_depth_rep_max`.
#[inline(always)]
fn bt_depth_target_for(opt_rep_rate: f32) -> usize {
    let base = bt_depth_target();
    if opt_rep_rate >= bt_depth_deep_min() {
        base.min(bt_depth_deep())
    } else {
        base
    }
}

static BT_DEEP_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);
static BT_DEEP_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);

/// Bench hook: the `opt_rep_rate` above which the deeper cut applies.
pub fn set_bt_deep_min_arm(v: f32) {
    BT_DEEP_MIN_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

/// Bench hook: the deeper target itself. 0 restores 24.
pub fn set_bt_deep_arm(v: usize) {
    BT_DEEP_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn bt_depth_deep_min() -> f32 {
    let v = BT_DEEP_MIN_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == u32::MAX {
        2.0
    } else {
        f32::from_bits(v)
    }
}

#[inline(always)]
fn bt_depth_deep() -> usize {
    let v = BT_DEEP_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        24
    } else {
        v
    }
}

/// GATE 12 @ L22 DEFECT. These four knobs feed `bt_depth_apply`, which runs ONCE
/// PER `bt_find_best` CALL -- 25,094,086 calls over the corpus at 2 MiB. Each was
/// an uncached `std::env::var`, so the depth gate performed up to FOUR
/// `GetEnvironmentVariableW` calls plus a `String` allocation per tree walk.
///
/// Measured: 4 lookups x 124.8 ns x 25.09M calls = 12,526 ms, against a 21,003 ms
/// L19 encode and 24,833 ms at L22 -- 60% and 50% of total encode time, spent
/// reading environment variables that never change.
///
/// Cached in atomics, read once. `RZSTD_BT_DEPTH_ENV=1` restores the per-call
/// lookups so the fix can be A/B'd in one process.
static BT_DEPTH_ENV_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
static BT_DEPTH_T_C: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);
static BT_DEPTH_SLOG_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);
static BT_DEPTH_REP_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);
static BT_DEPTH_STEPS_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: `false` restores the uncached per-call `std::env::var` reads.
/// Bench hook: set the depth target directly, bypassing the env cache. 0
/// restores the shipped 32. Needed because the value is cached on first read --
/// setting the env var after that reads the STALE cache, which is exactly how
/// an earlier harness measured the default on every arm of its sweep.
pub fn set_bt_depth_target_arm(v: usize) {
    BT_DEPTH_T_C.store(
        if v == 0 { 32 } else { v },
        core::sync::atomic::Ordering::Relaxed,
    );
}

pub fn set_bt_depth_cached_arm(cached: bool) {
    BT_DEPTH_ENV_ARM.store(u8::from(cached) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn bt_depth_cached() -> bool {
    BT_DEPTH_ENV_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

#[inline(always)]
fn bt_depth_target() -> usize {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_DEPTH_T_C.load(Relaxed);
    if c != usize::MAX && bt_depth_cached() {
        return c;
    }
    #[cfg(feature = "std")]
    {
        let v = std::env::var("RZSTD_BT_DEPTH_TARGET")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(32);
        BT_DEPTH_T_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    32
}

#[inline]
fn bt_depth_cut(params: CompressionParameters, opt_rep_rate: f32) -> u32 {
    let opt = matches!(
        params.strategy,
        Strategy::BtOpt | Strategy::BtUltra | Strategy::BtUltra2
    );
    // Applied only in the depth band that MEASURED a win, at both ends:
    //   searchLog 5-6 (L16-L18, 32-64 attempts)  -0.2928% size for 9.8% probes
    //                                            (jsonlog +2.348%) -- too costly
    //   searchLog 7   (L19-L21, 128 attempts)    +0.0010% for 8.6% -- shipped
    //   searchLog 9   (L22, 512 attempts)        NO probe saving at all: probes
    //                                            rose 0.26% and size +0.0022%,
    //                                            because the shallower parse
    //                                            emits more sequences and the DP
    //                                            then visits more positions.
    // L22 was excluded on a measurement taken before Gate 11's fill shipped AND
    // through a harness that discarded the depth setting. Re-measured, L22 gives
    // 22.4% of probes at +0.0120%; the band now has no upper bound.
    if !opt || params.search_log < bt_depth_min_slog() || opt_rep_rate > bt_depth_rep_max() {
        0
    } else {
        bt_depth_steps()
    }
}

#[inline(always)]
fn bt_depth_rep_max() -> f32 {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_DEPTH_REP_C.load(Relaxed);
    if c != u32::MAX && bt_depth_cached() {
        return f32::from_bits(c);
    }
    #[cfg(feature = "std")]
    {
        let v: f32 = std::env::var("RZSTD_BT_DEPTH_REP")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(50.0);
        BT_DEPTH_REP_C.store(v.to_bits(), Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    50.0
}

/// Lowest `search_log` at which the depth cut applies. Swept via
/// `RZSTD_BT_DEPTH_SLOG`; 10 disables it entirely.
#[inline(always)]
fn bt_depth_min_slog() -> u32 {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_DEPTH_SLOG_C.load(Relaxed);
    if c != u32::MAX && bt_depth_cached() {
        return c;
    }
    #[cfg(feature = "std")]
    {
        let v = std::env::var("RZSTD_BT_DEPTH_SLOG")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(7);
        BT_DEPTH_SLOG_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    7
}

#[inline(always)]
fn bt_depth_steps() -> u32 {
    use core::sync::atomic::Ordering::Relaxed;
    let c = BT_DEPTH_STEPS_C.load(Relaxed);
    if c != u32::MAX && bt_depth_cached() {
        return c;
    }
    #[cfg(feature = "std")]
    {
        let v = std::env::var("RZSTD_BT_DEPTH")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(1);
        BT_DEPTH_STEPS_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static BT_WALKS2: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static BT_ITERS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static BT_FULL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// `(walks, total_iterations, walks_that_used_ALL attempts)`
pub fn take_bt_iters() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        BT_WALKS2.swap(0, Relaxed),
        BT_ITERS.swap(0, Relaxed),
        BT_FULL.swap(0, Relaxed),
    )
}

fn search_attempts(params: CompressionParameters) -> usize {
    let v = SEARCH_LOG_ARM.load(core::sync::atomic::Ordering::Relaxed);
    let base = params.search_log.min(12) as i32;
    let d = if v == 0 { 0 } else { v as i32 - 8 };
    1usize << base.saturating_add(d).clamp(0, 12)
}

/// The `(hash_log, chain_log)` pairs the binary-tree specialisation covers.
///
/// ONE list, two consumers: the dispatch arms in `bt_find_best` and the public
/// `BT_SPEC_PAIRS` the coverage test asserts against. They were previously
/// independent, so a pair could be dropped from the dispatch while every test
/// still passed -- which is exactly how 24 of 64 (size, level) cells came to run
/// the slow runtime body unnoticed.
macro_rules! bt_spec_list {
    ($cb:ident) => {
        $cb! {
            (11, 11) (12, 12) (13, 13) (14, 14) (14, 15) (15, 15) (16, 16)
            (17, 17) (17, 18) (18, 18) (19, 18) (19, 19) (20, 20) (21, 21)
            (22, 22) (22, 23) (22, 24) (23, 22) (23, 23) (23, 24) (24, 24)
        }
    };
}

macro_rules! bt_spec_pairs_const {
    ($( ($h:literal, $c:literal) )*) => {
        /// Every `(hash_log, chain_log)` pair served by the specialised body.
        /// Anything else falls to `bt_find_best_runtime`.
        pub const BT_SPEC_PAIRS: &[(u32, u32)] = &[$( ($h, $c) ),*];
    };
}
bt_spec_list!(bt_spec_pairs_const);


/// The dispatch, RESOLVED ONCE PER BLOCK: `(hash_log, chain_log)` is
/// loop-invariant in every caller, yet `bt_find_best` re-ran a jump-table
/// dispatch (plus re-reading both fields) on every call -- per position,
/// per look-ahead, per fill insert and per DP edge. Callers hoist a fn
/// pointer instead; one predictable indirect call replaces the dance.
/// Same arms, same runtime fallback, same bt_spec parity gate.
/// The per-block-constant arguments of every bt call, packed: the fn
/// pointer previously re-marshaled NINE scalars per position, per
/// look-ahead step, per fill insert and per DP edge.
pub(crate) struct BtCtx<'a> {
    src: &'a [u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    chain_log: u32,
}

type BtFn = for<'a> fn(&BtCtx<'a>, usize, &mut MatchTables) -> (usize, usize);

fn bt_rt_search(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    bt_find_best_runtime(true, ctx, ip, t)
}

fn bt_rt_insert(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    bt_find_best_runtime(false, ctx, ip, t)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
fn bt_rt_search_bmi2(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    // SAFETY: only reachable through `bt_resolve`'s CPUID guard.
    #[allow(unsafe_code)]
    unsafe {
        bt_find_best_runtime_bmi2(true, ctx, ip, t)
    }
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
fn bt_rt_insert_bmi2(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    // SAFETY: only reachable through `bt_resolve`'s CPUID guard.
    #[allow(unsafe_code)]
    unsafe {
        bt_find_best_runtime_bmi2(false, ctx, ip, t)
    }
}

fn bt_resolve<const SEARCH: bool>(hash_log: u32, chain_log: u32) -> BtFn {
    // ISA selection happens HERE, once per block, so the per-position bt
    // calls carry no dispatch of their own.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    let bmi2 = crate::simd::has_bmi2();
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    let bmi2 = false;
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    let rt: BtFn = match (bmi2, SEARCH) {
        (true, true) => bt_rt_search_bmi2,
        (true, false) => bt_rt_insert_bmi2,
        (false, true) => bt_rt_search,
        (false, false) => bt_rt_insert,
    };
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    let rt: BtFn = if SEARCH { bt_rt_search } else { bt_rt_insert };
    if !bt_spec_enabled() {
        return rt;
    }
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if bmi2 {
        macro_rules! bt_spec_resolve_bmi2 {
            ($( ($h:literal, $c:literal) )*) => {
                match (hash_log, chain_log) {
                    $( ($h, $c) => bt_find_best_spec_bmi2::<$h, $c, SEARCH>, )*
                    _ => rt,
                }
            };
        }
        return bt_spec_list!(bt_spec_resolve_bmi2);
    }
    macro_rules! bt_spec_resolve {
        ($( ($h:literal, $c:literal) )*) => {
            match (hash_log, chain_log) {
                $( ($h, $c) => bt_find_best_impl::<$h, $c, SEARCH>, )*
                _ => rt,
            }
        };
    }
    bt_spec_list!(bt_spec_resolve)
}

/// GATE 4/5 EXTENDED TO THE BT PATH (L13-L22).
///
/// Same gap `find_dfast` had: `hash_log` and `chain_log` were read from `params`
/// at run time, so the hash shift and the binary-tree mask were computed with
/// variable-count shifts on EVERY position. Measured in the emitted assembly,
/// `bt_find_best` carried 5 variable shifts in 234 instructions.
///
/// The payoff here is structurally SMALLER than at L3 and that is worth stating:
/// this function is called once per position but then drives ~27 tree probes
/// (xml at L19: 55,988,704 probes over a 2 MiB prefix), each doing a
/// `count_match`. The shift is amortised over that walk, where `find_dfast` had
/// only ~2 candidate checks to amortise against.
///
/// Byte-identical by construction: HLOG and CLOG take the values the runtime
/// variables already held.
#[inline(never)]
fn bt_find_best_impl<const HLOG: u32, const CLOG: u32, const SEARCH: bool>(
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    bt_find_best_impl_inner::<HLOG, CLOG, SEARCH>(ctx, ip, tables)
}

/// Safe `BtFn`-shaped wrapper for the BMI2 twin; `bt_resolve` hands this out
/// only after its own `has_bmi2()` check, once per block.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
fn bt_find_best_spec_bmi2<const HLOG: u32, const CLOG: u32, const SEARCH: bool>(
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    // SAFETY: only reachable through `bt_resolve`'s CPUID guard.
    #[allow(unsafe_code)]
    unsafe {
        bt_find_best_impl_bmi2::<HLOG, CLOG, SEARCH>(ctx, ip, tables)
    }
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
#[inline(never)]
unsafe fn bt_find_best_impl_bmi2<const HLOG: u32, const CLOG: u32, const SEARCH: bool>(
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    bt_find_best_impl_inner::<HLOG, CLOG, SEARCH>(ctx, ip, tables)
}

#[inline(always)]
fn bt_find_best_impl_inner<const HLOG: u32, const CLOG: u32, const SEARCH: bool>(
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let BtCtx { src, block_start, block_end, window, mls, attempts, chain_log } = *ctx;
    // Diagnostic ONLY -- gated. Unguarded this was one atomic read-modify-write
    // per `bt_find_best` CALL, i.e. per POSITION across the whole L13-L22
    // ladder (~15.7M per level per corpus set). Same defect class as the two
    // per-probe atomics removed from `fast_probe`, which were worth +6.97%.
    // `take_bt_calls` therefore needs `--features rusty_zstd/profile`.
    if cfg!(feature = "profile") {
        BT_SPEC_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    const fn btlog(c: u32) -> u32 { let c = if c > 24 { 24 } else { c }; let c = c.saturating_sub(1); if c < 1 { 1 } else { c } }
    let _ = chain_log;
    let bt_log = btlog(CLOG);
    let bt_mask = (1usize << bt_log) - 1;
    // T2: guard the WORST CASE, not this `ip`.
    //
    // The tree addresses `(x & bt_mask) << 1` and that `+ 1`, so the largest
    // index it can ever form is `(bt_mask << 1) | 1` -- and `x` is `m`, a match
    // position, not `ip`. The old pair of guards (`len < 2`, then `larger >=
    // len` for this one `ip`) therefore bounded nothing inside the walk, which
    // is why every `chain[..]` access needed its own bounds check.
    //
    // It also closes a real edge. `bt_log` comes from `CLOG`/`params.chain_log`
    // rather than from the table, and `btlog` floors at 1, so `bt_mask >= 1` and
    // the tree needs `chain.len() >= 4` -- with `chain_log = 1`, reachable
    // through the advanced API, it addressed index 3 of a 2-entry table.
    if (bt_mask << 1) | 1 >= tables.chain.len() {
        return (0, 0);
    }
    let h = hash_mls(src, ip, mls, HLOG);
    // SPEC arm: `h < 2^HLOG` by the hash shift, and the resolve dispatch
    // guarantees tables.hash_log == HLOG, so hash.len() == 1 << HLOG.
    // `larger <= (bt_mask << 1) | 1` is the T2 entry guard's bound. Both
    // per-call checks were provably dead here (the runtime arm keeps its
    // own).
    debug_assert!(h < tables.hash.len());
    let mut match_idx = tables.get_h(h);
    tables.put_h(h, ip);
    let mut smaller = (ip & bt_mask) << 1;
    let mut larger = smaller + 1;
    debug_assert!(larger < tables.chain.len());
    // Loop-INVARIANT, recomputed on every node of every walk: a saturating_sub,
    // a max and a field load through `&mut MatchTables`, on a loop that runs
    // ~30M times per level across the corpus. The `tables.chain[..]` writes in
    // this same loop are what stop LLVM proving `frame_start` cannot change.
    let bt_lowest = block_start.saturating_sub(window).max(tables.frame_start);
    // Hoisted: the per-node window test `ip - m > window` is `m < ip - window`
    // (m < ip is tested first), one cmp against a per-call constant instead
    // of sub+cmp per node.
    let win_low = ip.saturating_sub(window);
    // GATE 14 DISPATCH -- the chain-walk depth.
    //
    // 4.33's "82-84% of walks end by exhausting `attempts`" is REFUTED and this
    // comment used to repeat it. That flag was set at the BOTTOM of the loop, so
    // it measured "did at least one iteration", not "used all attempts".
    //
    // The walk is NOT depth-bound. Measured with `take_bt_iters` (walks,
    // iterations, walks that consumed ALL attempts), 15 corpora at 512 KiB:
    //
    //   L13   13.5% full depth, mean  6.8 iterations
    //   L19    2.9% full depth, mean  8.4
    //   L22    2.6% full depth, mean  8.6
    //
    // 97-98% of walks at L19/L22 end on their own guards, an order of magnitude
    // under a 128- or 512-attempt budget. That is why raising the depth arm by
    // +1 or +2 moves output on 0 of 18 corpora at L22: nothing wants more depth,
    // and the probes live in the TAIL rather than at the cap.
    //
    // Priced at L19 (deterministic probe counts, 18 corpora):
    //   searchLog +1   +8.6% probes   -0.002% size   -- deeper buys nothing
    //   searchLog -1   -9.2% probes   +0.001% size   -- one step is nearly free
    //   searchLog -2  -16.9% probes   +0.014% size
    //
    // One step shallower is free in aggregate and loses on exactly ONE corpus:
    // versions-16m, +4.00%. That is the constant-stride content Gates 1, 2 and 6
    // all veto on `rep_yield`, and the same veto serves here -- a near-copy file
    // needs the depth to walk past its many equal-prefix candidates.
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut best_ml = 0usize;
    let mut best_m = 0usize;
    let mut iters = 0u32;
    for _ in 0..attempts {
        iters += 1;
        let Some(m) = match_idx else {
            tables.chain_set(smaller, 0);
            tables.chain_set(larger, 0);
            break;
        };
        if m >= ip || m < win_low {
            tables.chain_set(smaller, 0);
            tables.chain_set(larger, 0);
            break;
        }
        if m < bt_lowest {
            break;
        }
        // The T2 ENTRY guard already proves the worst case:
        // bt_idx + 1 <= (bt_mask << 1) | 1 < chain.len(). The per-node
        // re-check it replaced had survived it as a dead branch.
        let bt_idx = (m & bt_mask) << 1;
        debug_assert!(bt_idx + 1 < tables.chain.len());
        if COUNT {
            probes += 1;
        }
        // GATE 8 ON THE Bt LADDER -- the gate is DEAD at L13-L22 (`pipe_enabled`
        // has no caller there: find_fast 0 calls, find_opt 272), so this BUILDS
        // the capability rather than tuning it.
        //
        // Both children of this node live at `bt_idx` and `bt_idx + 1` -- one
        // cache line -- and NEITHER depends on `count_match`. In program order
        // the descent load was issued only after `count_match` had walked `src`,
        // so the chain miss serialised behind the src misses instead of
        // overlapping them. `chain` is far larger than LLC at these levels, so
        // that load misses on essentially every node.
        //
        // Applied to BOTH bt bodies -- keeping two hand-written copies in step
        // is exactly what `find_dfast_runtime` failed to do until Gate 6
        // silently broke Gate 4's byte-identity.
        let c_lo = tables.chain_at(bt_idx);
        let c_hi = tables.chain_at(bt_idx + 1);
        // REFUTED (2026-08-21): C's commonLengthSmaller/Larger floor
        // (count from the BST-invariant shared prefix instead of 0).
        // Corrupted the ROUNDTRIP on the first board: our tree tolerates
        // stale and aliased structure (bt slots alias at chain_log-1, and
        // the early breaks leave dangling subtree links) PRECISELY BECAUSE
        // this count re-verifies every byte from 0. The floor inherits C's
        // sort invariant only with C's full insert discipline; counting
        // from it here emitted matches longer than the data. The from-zero
        // count is load-bearing -- it is the tree's validity check.
        // The count head OPEN-CODED (count_match_fast's shape) because the
        // descent bytes ride in it: on a first-word mismatch, mb and ib are
        // bytes OF the two words already in registers -- the separate
        // `src.get(m + ml)` / `src.get(ip + ml)` loads and their two bounds
        // branches vanish for that (majority) case. Value-exact: in the head
        // case m + ml < m + 8 <= src.len(), so get() returns exactly the
        // byte the word holds; the long path keeps the get()-based loads
        // (bytes BEYOND block_end legitimately participate in routing).
        let (ml, mb, ib) = if ip + 8 <= block_end && ip + 8 <= src.len() && m + 8 <= src.len() {
            let a = load_u64le(src, m);
            let b = load_u64le(src, ip);
            if a != b {
                let ml = ((a ^ b).trailing_zeros() as usize) >> 3;
                (ml, (a >> (8 * ml)) as u8, (b >> (8 * ml)) as u8)
            } else {
                let ml = 8 + count_match(src, m + 8, ip + 8, block_end);
                (
                    ml,
                    src.get(m + ml).copied().unwrap_or(0),
                    src.get(ip + ml).copied().unwrap_or(0),
                )
            }
        } else {
            let ml = count_match(src, m, ip, block_end);
            (
                ml,
                src.get(m + ml).copied().unwrap_or(0),
                src.get(ip + ml).copied().unwrap_or(0),
            )
        };
        #[cfg(feature = "profile")]
        {
            BT_PROBE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if ml < mls {
                BT_SHORT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            if ml <= best_ml {
                BT_NOGAIN.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
        // offset_ok and the frame_start floor are GUARANTEED by the node
        // validity above (m >= win_low => ip - m <= window; m >= bt_lowest >=
        // frame_start); re-checking per node was pure redundancy.
        //
        // INSERT-ONLY copies (SEARCH = false) serve the three callers that
        // DISCARD the return -- both fills (61.9% of all tree work at
        // L13-L15) and the priming pass. The descent and every tree write
        // are identical (the bt walk has NO best_ml-dependent break), so
        // skipping the tracking is byte-identical for a discarded result.
        if SEARCH && ml >= mls && ml > best_ml {
            best_ml = ml;
            best_m = m;
        }
        if mb < ib {
            tables.chain_set(smaller, m as u32);
            // BYTE-IDENTICAL: if the store above targeted the slot we
            // pre-loaded, forward the stored value by hand -- the original read
            // happened AFTER the write and would have observed it.
            let v = if smaller == bt_idx + 1 { m as u32 } else { c_hi };
            smaller = bt_idx + 1;
            match_idx = if v == 0 { None } else { Some(v as usize) };
        } else {
            tables.chain_set(larger, m as u32);
            let v = if larger == bt_idx { m as u32 } else { c_lo };
            larger = bt_idx;
            match_idx = if v == 0 { None } else { Some(v as usize) };
        }
        // smaller/larger are bt_idx or bt_idx + 1: covered by the entry
        // guard, same as above.
        debug_assert!(smaller < tables.chain.len() && larger < tables.chain.len());
    }
    // Consumers are the g14/btdepth gate harnesses only; unguarded this was
    // THREE lock-prefixed RMWs per walk -- per POSITION across L13-L22 (the
    // 959e0ae class, fourth sighting, in both bt bodies).
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        BT_WALKS2.fetch_add(1, Relaxed);
        BT_ITERS.fetch_add(iters as u64, Relaxed);
        if iters as usize >= attempts {
            BT_FULL.fetch_add(1, Relaxed);
        }
    }
    #[cfg(not(feature = "profile"))]
    let _ = iters;
    if COUNT {
        crate::prof::note_probes(probes);
    }
    (best_m, best_ml)
}

#[inline(never)]
fn bt_find_best_runtime(
    search: bool,
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    bt_find_best_runtime_inner(search, ctx, ip, tables)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
#[inline(never)]
unsafe fn bt_find_best_runtime_bmi2(
    search: bool,
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    bt_find_best_runtime_inner(search, ctx, ip, tables)
}

#[inline(always)]
fn bt_find_best_runtime_inner(
    search: bool,
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let BtCtx { src, block_start, block_end, window, mls, attempts, chain_log } = *ctx;
    // Diagnostic ONLY -- gated. Unguarded this was one atomic read-modify-write
    // per `bt_find_best` CALL, i.e. per POSITION across the whole L13-L22
    // ladder (~15.7M per level per corpus set). Same defect class as the two
    // per-probe atomics removed from `fast_probe`, which were worth +6.97%.
    // `take_bt_calls` therefore needs `--features rusty_zstd/profile`.
    if cfg!(feature = "profile") {
        BT_RUNTIME_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    let hash_log = tables.hash_log;
    let bt_log = chain_log.min(24).saturating_sub(1).max(1);
    let bt_mask = (1usize << bt_log) - 1;
    // T2: guard the WORST CASE, not this `ip`.
    //
    // The tree addresses `(x & bt_mask) << 1` and that `+ 1`, so the largest
    // index it can ever form is `(bt_mask << 1) | 1` -- and `x` is `m`, a match
    // position, not `ip`. The old pair of guards (`len < 2`, then `larger >=
    // len` for this one `ip`) therefore bounded nothing inside the walk, which
    // is why every `chain[..]` access needed its own bounds check.
    //
    // It also closes a real edge. `bt_log` comes from `CLOG`/`params.chain_log`
    // rather than from the table, and `btlog` floors at 1, so `bt_mask >= 1` and
    // the tree needs `chain.len() >= 4` -- with `chain_log = 1`, reachable
    // through the advanced API, it addressed index 3 of a 2-entry table.
    if (bt_mask << 1) | 1 >= tables.chain.len() {
        return (0, 0);
    }
    let h = hash_mls(src, ip, mls, hash_log);
    if h >= tables.hash.len() {
        return (0, 0);
    }
    let mut match_idx = tables.get_h(h);
    tables.put_h(h, ip);
    let mut smaller = (ip & bt_mask) << 1;
    let mut larger = smaller + 1;
    if larger >= tables.chain.len() {
        return (0, 0);
    }
    // Loop-INVARIANT, recomputed on every node of every walk: a saturating_sub,
    // a max and a field load through `&mut MatchTables`, on a loop that runs
    // ~30M times per level across the corpus. The `tables.chain[..]` writes in
    // this same loop are what stop LLVM proving `frame_start` cannot change.
    let bt_lowest = block_start.saturating_sub(window).max(tables.frame_start);
    // Hoisted: the per-node window test `ip - m > window` is `m < ip - window`
    // (m < ip is tested first), one cmp against a per-call constant instead
    // of sub+cmp per node.
    let win_low = ip.saturating_sub(window);
    // GATE 14 DISPATCH -- the chain-walk depth.
    //
    // 4.33's "82-84% of walks end by exhausting `attempts`" is REFUTED and this
    // comment used to repeat it. That flag was set at the BOTTOM of the loop, so
    // it measured "did at least one iteration", not "used all attempts".
    //
    // The walk is NOT depth-bound. Measured with `take_bt_iters` (walks,
    // iterations, walks that consumed ALL attempts), 15 corpora at 512 KiB:
    //
    //   L13   13.5% full depth, mean  6.8 iterations
    //   L19    2.9% full depth, mean  8.4
    //   L22    2.6% full depth, mean  8.6
    //
    // 97-98% of walks at L19/L22 end on their own guards, an order of magnitude
    // under a 128- or 512-attempt budget. That is why raising the depth arm by
    // +1 or +2 moves output on 0 of 18 corpora at L22: nothing wants more depth,
    // and the probes live in the TAIL rather than at the cap.
    //
    // Priced at L19 (deterministic probe counts, 18 corpora):
    //   searchLog +1   +8.6% probes   -0.002% size   -- deeper buys nothing
    //   searchLog -1   -9.2% probes   +0.001% size   -- one step is nearly free
    //   searchLog -2  -16.9% probes   +0.014% size
    //
    // One step shallower is free in aggregate and loses on exactly ONE corpus:
    // versions-16m, +4.00%. That is the constant-stride content Gates 1, 2 and 6
    // all veto on `rep_yield`, and the same veto serves here -- a near-copy file
    // needs the depth to walk past its many equal-prefix candidates.
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut best_ml = 0usize;
    let mut best_m = 0usize;
    let mut iters = 0u32;
    for _ in 0..attempts {
        iters += 1;
        let Some(m) = match_idx else {
            tables.chain_set(smaller, 0);
            tables.chain_set(larger, 0);
            break;
        };
        if m >= ip || m < win_low {
            tables.chain_set(smaller, 0);
            tables.chain_set(larger, 0);
            break;
        }
        if m < bt_lowest {
            break;
        }
        // The T2 ENTRY guard already proves the worst case:
        // bt_idx + 1 <= (bt_mask << 1) | 1 < chain.len(). The per-node
        // re-check it replaced had survived it as a dead branch.
        let bt_idx = (m & bt_mask) << 1;
        debug_assert!(bt_idx + 1 < tables.chain.len());
        if COUNT {
            probes += 1;
        }
        // GATE 8 ON THE Bt LADDER -- the gate is DEAD at L13-L22 (`pipe_enabled`
        // has no caller there: find_fast 0 calls, find_opt 272), so this BUILDS
        // the capability rather than tuning it.
        //
        // Both children of this node live at `bt_idx` and `bt_idx + 1` -- one
        // cache line -- and NEITHER depends on `count_match`. In program order
        // the descent load was issued only after `count_match` had walked `src`,
        // so the chain miss serialised behind the src misses instead of
        // overlapping them. `chain` is far larger than LLC at these levels, so
        // that load misses on essentially every node.
        //
        // Applied to BOTH bt bodies -- keeping two hand-written copies in step
        // is exactly what `find_dfast_runtime` failed to do until Gate 6
        // silently broke Gate 4's byte-identity.
        let c_lo = tables.chain_at(bt_idx);
        let c_hi = tables.chain_at(bt_idx + 1);
        // REFUTED (2026-08-21): C's commonLengthSmaller/Larger floor
        // (count from the BST-invariant shared prefix instead of 0).
        // Corrupted the ROUNDTRIP on the first board: our tree tolerates
        // stale and aliased structure (bt slots alias at chain_log-1, and
        // the early breaks leave dangling subtree links) PRECISELY BECAUSE
        // this count re-verifies every byte from 0. The floor inherits C's
        // sort invariant only with C's full insert discipline; counting
        // from it here emitted matches longer than the data. The from-zero
        // count is load-bearing -- it is the tree's validity check.
        // The count head OPEN-CODED (count_match_fast's shape) because the
        // descent bytes ride in it: on a first-word mismatch, mb and ib are
        // bytes OF the two words already in registers -- the separate
        // `src.get(m + ml)` / `src.get(ip + ml)` loads and their two bounds
        // branches vanish for that (majority) case. Value-exact: in the head
        // case m + ml < m + 8 <= src.len(), so get() returns exactly the
        // byte the word holds; the long path keeps the get()-based loads
        // (bytes BEYOND block_end legitimately participate in routing).
        let (ml, mb, ib) = if ip + 8 <= block_end && ip + 8 <= src.len() && m + 8 <= src.len() {
            let a = load_u64le(src, m);
            let b = load_u64le(src, ip);
            if a != b {
                let ml = ((a ^ b).trailing_zeros() as usize) >> 3;
                (ml, (a >> (8 * ml)) as u8, (b >> (8 * ml)) as u8)
            } else {
                let ml = 8 + count_match(src, m + 8, ip + 8, block_end);
                (
                    ml,
                    src.get(m + ml).copied().unwrap_or(0),
                    src.get(ip + ml).copied().unwrap_or(0),
                )
            }
        } else {
            let ml = count_match(src, m, ip, block_end);
            (
                ml,
                src.get(m + ml).copied().unwrap_or(0),
                src.get(ip + ml).copied().unwrap_or(0),
            )
        };
        #[cfg(feature = "profile")]
        {
            BT_PROBE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if ml < mls {
                BT_SHORT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
            if ml <= best_ml {
                BT_NOGAIN.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
        // offset_ok and the frame_start floor are GUARANTEED by the node
        // validity above (m >= win_low => ip - m <= window; m >= bt_lowest >=
        // frame_start); re-checking per node was pure redundancy.
        if search && ml >= mls && ml > best_ml {
            best_ml = ml;
            best_m = m;
        }
        if mb < ib {
            tables.chain_set(smaller, m as u32);
            // BYTE-IDENTICAL: if the store above targeted the slot we
            // pre-loaded, forward the stored value by hand -- the original read
            // happened AFTER the write and would have observed it.
            let v = if smaller == bt_idx + 1 { m as u32 } else { c_hi };
            smaller = bt_idx + 1;
            match_idx = if v == 0 { None } else { Some(v as usize) };
        } else {
            tables.chain_set(larger, m as u32);
            let v = if larger == bt_idx { m as u32 } else { c_lo };
            larger = bt_idx;
            match_idx = if v == 0 { None } else { Some(v as usize) };
        }
        // smaller/larger are bt_idx or bt_idx + 1: covered by the entry
        // guard, same as above.
        debug_assert!(smaller < tables.chain.len() && larger < tables.chain.len());
    }
    // Consumers are the g14/btdepth gate harnesses only; unguarded this was
    // THREE lock-prefixed RMWs per walk -- per POSITION across L13-L22 (the
    // 959e0ae class, fourth sighting, in both bt bodies).
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        BT_WALKS2.fetch_add(1, Relaxed);
        BT_ITERS.fetch_add(iters as u64, Relaxed);
        if iters as usize >= attempts {
            BT_FULL.fetch_add(1, Relaxed);
        }
    }
    #[cfg(not(feature = "profile"))]
    let _ = iters;
    if COUNT {
        crate::prof::note_probes(probes);
    }
    (best_m, best_ml)
}

#[inline(always)]
fn find_bt_lazy(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = params.min_match.max(3) as usize;
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
    let mut seqs = if finder_scratch_enabled() {
        let mut v = std::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut lits = if finder_scratch_enabled() {
        let mut v = std::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 73: repcode-1 in BtLazy2 (L13-L14) -- the last finder without it.
    // Per-call arm reads hoisted to once per block (the chain_find_best rule):
    // attempts (an arm atomic inside bt_depth_apply/search_attempts) ran per
    // position, per look-ahead AND per fill insert; the fill arms ran per
    // match.
    let attempts = bt_depth_apply(search_attempts(params), params, tables.opt_rep_rate);
    let clog = params.chain_log.min(24);
    let btf = bt_resolve::<true>(tables.hash_log, clog);
    let btf_ins = bt_resolve::<false>(tables.hash_log, clog);
    let bt_ctx = BtCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts,
        chain_log: clog,
    };
    let gain_cmp = lazy_gain_enabled_bt();
    let fill_on = lazy_fill_enabled();
    let bt_stride = bt_fill_stride();
    let use_rep = rep_search_on(tables.rep_yield, params.strategy);
    let mut rep1 = reps[0] as usize;
    let mut rep_hits = 0u64;
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
    let mut ip = block_start;
    while ip <= ilimit {
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                rep_hits += 1;
                let mstart = ip + 1;
                push_lits_range(&mut lits, src, anchor, mstart);
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
        if best_ml >= mls {
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
                let take = if gain_cmp {
                    ml >= mls
                        && lazy_gain(ml, ip2 - m) > lazy_gain(best_ml, best_ip - best_m) + 4
                } else {
                    ml > best_ml
                };
                if take {
                    best_ml = ml;
                    best_m = m;
                    best_ip = ip2;
                }
            }
        }
        if best_ml >= mls {
            // DEFECT B3 FIX (btlazy2): back-extend -- see `find_greedy`.
            let mut s = best_ip;
            let mut mm = best_m;
            let mut n = best_ml;
            #[cfg(feature = "profile")]
            let bext_from = s;
            while s > anchor && mm > tables.frame_start && back_eq(src, s, mm) {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            #[cfg(feature = "profile")]
            note_bext((bext_from - s) as u64);
            push_lits_range(&mut lits, src, anchor, s);
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
                let mut p = (best_ip + 1).max(look_hi + 1);
                while p < end && p <= ilimit {
                    let _ = btf_ins(&bt_ctx, p, tables);
                    p += stride;
                }
            }
            ip = end;
            anchor = ip;
        } else {
            ip += 1;
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    push_lits_range(&mut lits, src, anchor, block_end);
    // Probes reported by `bt_find_best`.
    note_finder_work(cfg!(feature = "profile"), 0, seqs.len() as u64, &seqs, &lits);
    (seqs, lits)
}

/// The DP's LITERAL price in bits. Flat 6 since the parser was written, against
/// a real cost of ~8 bits raw and ~4-7 after Huffman -- so it UNDER-prices
/// literals on high-entropy content, which makes the "optimal" parse prefer
/// literals over matches and lose to plain lazy. Swept via `RZSTD_OPT_LIT`.
/// The MEASURED cost of the literals just emitted, for the next block's DP.
#[inline]
fn measured_lit_bits(section_bytes: usize, literal_count: usize) -> u32 {
    // WHOLE-SECTION cost per literal, deliberately -- see 4.20: the marginal
    // variant is theoretically righter and measured worse, because the DP's
    // MATCH price is itself an approximation and pricing only the literal side
    // exactly unbalances the pair.
    let bits = (section_bytes as u64 * 8) / literal_count.max(1) as u64;
    // Clamp to the range the price model is meaningful over.
    bits.clamp(3, 10) as u32
}

fn opt_lit_cost(tables: &MatchTables) -> u32 {
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
            e = std::env::var("RZSTD_OPT_LIT")
                .ok()
                .and_then(|v| v.trim().parse().ok())
                .unwrap_or(NO_OVERRIDE);
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

static OPT_LIT_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// The DP's match-length extra-bits pricing (Gate 19's other half).
static OPT_MLBITS_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for ML-bits pricing.
pub fn set_opt_mlbits_arm(on: bool) {
    OPT_MLBITS_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn opt_mlbits_enabled() -> bool {
    // DEFAULT ON -- adjudicated: L16 -0.007% / L19 -0.014% / L22 -0.014%
    // totals, best nci -0.302%, worst jsonlog +0.097%. Small and real.
    !matches!(OPT_MLBITS_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// Set the DP literal price in-process.
pub fn set_opt_lit_arm(v: u32) {
    OPT_LIT_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

/// Blocks between forced re-probes of the opt repcode candidate.
const OPT_REP_PERIOD: u32 = 16;

/// Blocks the candidate must RUN before the gate may shut it.
const OPT_REP_WARMUP: u32 = 4;

/// Minimum bytes-per-probe for the opt DP's repcode candidate to run. A NEGATIVE
/// value is the escape hatch: constant ON, i.e. the pre-dispatch behaviour, which
/// is what the ledger's "fallback proven" column requires.
///
/// The term is NOT decoration -- disabling it entirely (schedule only) removes
/// 91.0% of the probes instead of 85.8%, but costs +0.1179% size against
/// +0.0195%. Those 6M extra probes buy back 0.098 percentage points.
fn opt_rep_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[6].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_REP_MIN_C.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_OPT_REP_MIN")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(50.0);
        OPT_REP_MIN_C.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    50.0
}
#[cfg(feature = "std")]
static OPT_REP_MIN_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Measurement arm for the opt DP's repcode candidate.
static OPT_REP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// A/B the opt DP's repcode candidate in-process.
pub fn set_opt_rep_arm(on: bool) {
    OPT_REP_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn opt_rep_enabled() -> bool {
    !matches!(
        OPT_REP_ARM.load(core::sync::atomic::Ordering::Relaxed),
        1
    )
}

/// GATE 10 @ L19: what the DP's repcode candidate earns. `try_rep1` runs at
/// every position of every opt block, unconditionally.
pub static OPT_REP_PROBES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static OPT_REP_HITS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
pub static OPT_REP_BYTES: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

pub static OPT_POS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_SKIP_INF: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_SKIP_JUMP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_SKIP_JUMPS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

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

pub static OPT_BT_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_BT_DRY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_BT_LEN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static OPT_SEQS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

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
fn opt_fill_enabled() -> bool {
    #[cfg(feature = "profile")]
    ENVHIT[7].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_FILL_C.load(Ordering::Relaxed);
        if c != 0 {
            return c == 2;
        }
        let v = std::env::var("RZSTD_OPT_FILL").map(|v| v.trim() != "0").unwrap_or(true);
        OPT_FILL_C.store(if v { 2 } else { 1 }, Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    false
}
#[cfg(feature = "std")]
static OPT_FILL_C: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Above this bytes-per-rep-probe the content is rep-dominated and the jumped
/// span's interior is not worth inserting.
fn opt_fill_rep_max() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[8].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = OPT_FILL_REP_C.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_OPT_FILL_REP")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(50.0);
        OPT_FILL_REP_C.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    50.0
}
#[cfg(feature = "std")]
static OPT_FILL_REP_C: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Longest span the back-fill will walk. Beyond this the jump is a single huge
/// repeat and its interior is not worth inserting.
fn opt_fill_max() -> usize {
    #[cfg(feature = "profile")]
    ENVHIT[9].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let a = OPT_FILL_MAX_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if a != 0 {
        return a;
    }
    #[cfg(feature = "std")]
    {
        std::env::var("RZSTD_OPT_FILL_MAX")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(usize::MAX)
    }
    #[cfg(not(feature = "std"))]
    usize::MAX
}

/// Stride for that back-fill; 1 inserts every skipped position.
/// GATE 12 @ L19 arms: the opt back-fill's stride and span cap, as atomics so
/// they can be swept in one process. 0 = unset (use the env/default path).
static OPT_FILL_S_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(0);
static OPT_FILL_MAX_ARM: core::sync::atomic::AtomicUsize =
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
pub static OPT_FILL_INS: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);

/// Read and clear the opt back-fill insert count.
pub fn take_opt_fill_ins() -> u64 {
    OPT_FILL_INS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// GATE 12 @ L19 defect arm: `false` restores the per-jump `std::env::var`
/// lookups the back-fill guard used to perform inside the DP loop, so the fix
/// can be A/B'd in one process instead of across two binaries.
static OPT_HOIST_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: `false` reads the four back-fill knobs per jumped position again.
pub fn set_opt_hoist_arm(hoisted: bool) {
    OPT_HOIST_ARM.store(u8::from(hoisted) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn opt_hoisted() -> bool {
    OPT_HOIST_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

fn opt_fill_stride() -> usize {
    #[cfg(feature = "profile")]
    ENVHIT[10].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let a = OPT_FILL_S_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if a != 0 {
        return a;
    }
    #[cfg(feature = "std")]
    {
        std::env::var("RZSTD_OPT_FILL_S")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .filter(|v| *v >= 1)
            .unwrap_or(1)
    }
    #[cfg(not(feature = "std"))]
    1
}

#[inline(always)]
fn find_opt(
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
    let mls = params.min_match.max(3) as usize;
    if n < 8 {
        return (Vec::new(), src[block_start..block_end].to_vec());
    }
    let inf = u32::MAX / 4;
    // T2: take the DP arrays from the frame instead of building 2.63 MiB of
    // them per block. See `MatchTables::opt_price`.
    let mut price = std::mem::take(&mut tables.opt_price);
    let mut prev = std::mem::take(&mut tables.opt_prev);
    // off and ml live in ONE u64 (off | ml << 32): one store per edge
    // improvement and one load per parse step instead of two of each, and
    // one scratch array fewer.
    let mut match_om = std::mem::take(&mut tables.opt_om);
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
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
    // Hoisted once per block (the chain_find_best rule): bt_find_best runs
    // per DP position here.
    let bt_attempts = bt_depth_apply(search_attempts(params), params, tables.opt_rep_rate);
    let clog = params.chain_log.min(24);
    let btf = bt_resolve::<true>(tables.hash_log, clog);
    let btf_ins = bt_resolve::<false>(tables.hash_log, clog);
    let bt_ctx = BtCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts: bt_attempts,
        chain_log: clog,
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
    let mlb_on = opt_mlbits_enabled();
    // Per-JUMP arm read hoisted (the OFF arm's deliberate env re-reads stay
    // inside; only the selector atomic moves).
    let hoisted_arm = opt_hoisted();
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
            o_skip_inf += 1;
            i += 1;
            continue;
        }
        // Range-proven plain add: pi < inf = MAX/4 and lit_cost is a small
        // constant, so saturation is unreachable -- the saturating form paid
        // a cmov per position for nothing.
        let np = pi + lit_cost;
        #[allow(unsafe_code)]
        unsafe {
            if np < *price.get_unchecked(i + 1) {
                *price.get_unchecked_mut(i + 1) = np;
                *prev.get_unchecked_mut(i + 1) = i as u32;
            }
        }
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
            if let Some(rml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                o_rep_hits += 1;
                o_rep_bytes += rml as u64;
                let j = i + 1 + rml;
                if j <= n {
                    #[allow(unsafe_code)]
                    unsafe {
                        let np = *price.get_unchecked(i + 1)
                            + rep_cost
                            + if mlb_on && rml > 34 {
                                27 - ((rml - 3) as u32).leading_zeros()
                            } else {
                                0
                            };
                        if np < *price.get_unchecked(j) {
                            *price.get_unchecked_mut(j) = np;
                            *prev.get_unchecked_mut(j) = (i + 1) as u32 | OPT_MATCH_BIT;
                            *match_om.get_unchecked_mut(j) =
                                rep1 as u64 | ((rml as u64) << 32);
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
        let seq_cost = (12u32 + off_bits).saturating_sub(extra);
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
        let floor_step = (bml.saturating_sub(mls) / OPT_MAX_LENGTHS).max(1);
        // price[i] and seq_cost are PER-MATCH constants: the sum was
        // reloaded and re-added on every length step.
        #[allow(unsafe_code)]
        let np_base = unsafe { *price.get_unchecked(i) } + seq_cost;
        // ADJUDICATED (the Gate 19 note's other half): the bitstream charges
        // MATCH-LENGTH extra bits, but the DP priced every length of a match
        // identically -- so the "optimal" parse over-preferred long matches
        // whose tails cost real bits. RFC shape: lengths 3..=34 pay 0 extra
        // bits; beyond that the extra bits grow ~log2(len - 3) - 4.
        let mut len = mls;
        loop {
            let j = i + len;
            if j > n {
                break;
            }
            let np = if mlb_on && len > 34 {
                np_base + (27 - ((len - 3) as u32).leading_zeros())
            } else {
                np_base
            };
            #[allow(unsafe_code)]
            unsafe {
                if np < *price.get_unchecked(j) {
                    *price.get_unchecked_mut(j) = np;
                    *prev.get_unchecked_mut(j) = i as u32 | OPT_MATCH_BIT;
                    *match_om.get_unchecked_mut(j) =
                        (ip - bm) as u64 | ((len as u64) << 32);
                }
            }
            if len == bml {
                break;
            }
            let step = if params.strategy == Strategy::BtUltra2 {
                1
            } else {
                (bml - len).clamp(1, 4)
            };
            // `.min(bml)` guarantees the longest match is always priced, which
            // a larger step would otherwise skip past.
            len = (len + step.max(floor_step)).min(bml);
        }
        // C's immediate encoding: this match already exceeds `targetLength`, so
        // commit it and jump the DP past the span it covers instead of pricing
        // every interior position. Positions inside keep `price == inf`, so no
        // path can route through them -- exactly the greedy commitment C makes.
        if bml >= sufficient_len && i + bml <= n {
            o_skip_jump += bml as u64;
            o_skip_jumps += 1;
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
            let hoisted = hoisted_arm;
            let (g_on, g_rep) = if hoisted {
                (fill_on, fill_rep_max)
            } else {
                (opt_fill_enabled(), opt_fill_rep_max())
            };
            if g_on && tables.opt_rep_meas >= 2 && tables.opt_rep_peak < g_rep {
                let step = if hoisted { fill_step } else { opt_fill_stride() };
                // Cap the span. text-32m and versions-16m hold 93% of ALL jumped
                // positions (3.58M of 3.85M) and contribute -15 and +54 bytes;
                // dickens, samba, nci, ooffice and xml hold 6% and contribute
                // -381. An enormous jump means one huge repeat, and filling its
                // interior buys nothing -- those positions are reachable through
                // the repeat itself.
                let span = bml.min(if hoisted { fill_span_max } else { opt_fill_max() });
                let mut q = i + 1;
                while q < i + span {
                    let qp = block_start + q;
                    if qp + 8 > block_end {
                        break;
                    }
                    let _ = btf_ins(&bt_ctx, qp, tables);
                    #[cfg(feature = "profile")]
                    OPT_FILL_INS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    q += step;
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
    let mut ops: Vec<(u32, u32, u32, bool)> = std::mem::take(&mut tables.opt_ops);
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
    if opt_ops_exact() && ops.capacity() <= n {
        let mut k = 0usize;
        let mut j = n;
        // j <= n along the whole chain (prev entries are indices the DP
        // wrote, all <= n); the checked op was a bounds branch per parse
        // step.
        while j > 0 {
            k += 1;
            debug_assert!(j < prev.len());
            #[allow(unsafe_code)]
            {
                j = (*unsafe { prev.get_unchecked(j) } & !OPT_MATCH_BIT) as usize;
            }
        }
        if ops.capacity() < k {
            ops = Vec::with_capacity(k);
        }
    } else if opt_ops_blanket() && ops.capacity() < n + 1 {
        ops = Vec::with_capacity(n + 1);
    }
    let mut i = n;
    let mut nmatched = 0usize;
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
        let (pr, om) = unsafe { (*prev.get_unchecked(i), *match_om.get_unchecked(i)) };
        let (off, ml) = (om as u32, (om >> 32) as u32);
        let p = (pr & !OPT_MATCH_BIT) as usize;
        let m = pr & OPT_MATCH_BIT != 0;
        if m {
            nmatched += 1;
            if pending_start != usize::MAX {
                count_run(pending_start - (p + ml as usize), &mut w_short, &mut w_mid);
            }
            pending_start = p;
        }
        ops.push((p as u32, off, ml, m));
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
    let mut seqs = std::mem::take(&mut tables.seq_scratch);
    seqs.clear();
    if seqs.capacity() < nmatched + 1 {
        seqs = Vec::with_capacity(nmatched + 1);
    }
    let mut lits = std::mem::take(&mut tables.lit_scratch);
    lits.clear();
    if lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    // Width chosen from THIS block's own runs, by the asm-derived rule:
    // widen when mid_share > short_share * (fast32 - fast16) / (slow - fast32).
    let opt_w = {
        let n = nmatched.max(1) as f32;
        let (sh, md) = (w_short as f32 / n, w_mid as f32 / n);
        if sh < lit_short_min() {
            0
        } else if md > sh * WIDEN_RATIO {
            LIT_PUSH_WIDTH_WIDE
        } else {
            LIT_PUSH_WIDTH
        }
    };
    let mut anchor = 0usize;
    for &(start, off, ml, matched) in ops.iter().rev() {
        let start = start as usize;
        if matched {
            push_literals(
                &mut lits,
                src,
                block_start + anchor,
                block_start + start,
                opt_w,
            );
            seqs.push(Seq {
                litlen: (start - anchor) as u32,
                matchlen: ml,
                offset: off,
            });
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
    let _ = (o_rep_probes, o_rep_hits, o_rep_bytes, o_bt_calls, o_bt_dry, o_bt_len);
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
    note_finder_work(cfg!(feature = "profile"), 0, seqs.len() as u64, &seqs, &lits);
    tables.opt_ops = ops;
    tables.opt_price = price;
    tables.opt_prev = prev;
    tables.opt_om = match_om;
    (seqs, lits)
}

/// The BYTE half of `match_ok`, exactly (u32 head + tail slice to `mls`).
/// Split out for the chain walk: validity is MONOTONE along a chain (positions
/// strictly decrease), so a validity failure correctly ends the walk -- but a
/// byte mismatch is just a hash collision, and C's `ZSTD_HcFindBestMatch`
/// steps past it to the next link. Our walk broke on it, amputating the
/// remaining chain at the first collision.
#[inline(always)]
fn mls_eq(src: &[u8], m: usize, ip: usize, mls: usize, smask: u64) -> bool {
    // The census found the tail slice-eq compiled to a LIBC MEMCMP CALL per
    // candidate -- for mls = 5, a memcmp of ONE byte. Every caller sits in a
    // walk that has proven `m < ip <= len - 8` (ip <= ilimit and validity),
    // so for mls <= 8 the whole test is one masked u64 xor -- fewer loads
    // than the old u32-head + tail, and no call. `smask` is the caller's
    // block-hoisted byte mask (recomputing it here was a shift PER
    // CANDIDATE).
    if mls <= 8 {
        debug_assert!(m < ip && ip + 8 <= src.len());
        debug_assert!(smask == if mls == 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 });
        return (load_u64le(src, m) ^ load_u64le(src, ip)) & smask == 0;
    }
    if load_u32le(src, m) != load_u32le(src, ip) {
        return false;
    }
    src[m + 4..m + mls] == src[ip + 4..ip + mls]
}

/// WALK-CONTINUE arm: C-parity chain walk (step past byte mismatches).
/// Byte-CHANGING (finds matches the amputated walk missed), so it ships on
/// the adjudication board in `chainwalk`, not on byte-identity.
static WALK_CONT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the walk-continue arm.
pub fn set_walk_cont_arm(on: bool) {
    WALK_CONT_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn walk_cont_enabled() -> bool {
    // DEFAULT ON: adjudicated on the chainwalk board with the first-share
    // gate at 0.55 -- worst corpus jsonlog +0.54% at L12 against dickens
    // -8.99%, reymont -7.62%, webster -6.03%.
    !matches!(WALK_CONT_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// WALK-CONTINUE DISPATCH: the C-parity walk wins big on ordinary content
/// (dickens -9.60%, reymont -8.10%, webster -6.33% at L12) and LOSES on
/// rep-dominated content (smallmsg +4.30%, jsonlog +3.89%) -- the deeper
/// walk finds longer matches at offsets that displace the repcode economy.
/// Identical shape to the wide hash's versions dispatch at L1, and the
/// signal is the same one these finders already maintain per block:
/// `rep_yield`. Continue only where reps are NOT carrying the block.
static WALK_REP_MAX_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the `rep_yield` bar under which the C-parity walk applies.
pub fn set_walk_rep_max_arm(v: f32) {
    WALK_REP_MAX_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

fn walk_rep_max() -> f32 {
    let c = WALK_REP_MAX_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if c != u32::MAX {
        return f32::from_bits(c);
    }
    0.10
}

/// LAZY GAIN ARM: C's offset-priced look-ahead comparison
/// (`ZSTD_compressBlock_lazy_generic`): a later match displaces the current
/// one only when `4*ml2 - log2(off2)` beats `4*ml1 - log2(off1) + 4`. Our
/// look-ahead compared RAW LENGTHS, which is exactly what lets a deeper
/// chain walk trade a cheap repeated offset for a long-but-expensive one on
/// record-periodic content (jsonlog +3.9%, smallmsg +4.3% under
/// walk-continue). Byte-CHANGING; ships on the `chainwalk` board.
static LAZY_GAIN_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the offset-priced look-ahead.
pub fn set_lazy_gain_arm(on: bool) {
    LAZY_GAIN_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn lazy_gain_enabled() -> bool {
    // find_lazy's default: OFF (refuted at L7-L12 -- not the loser
    // mechanism there, and mixed-small on its own).
    matches!(LAZY_GAIN_ARM.load(core::sync::atomic::Ordering::Relaxed), 2)
}

fn lazy_gain_enabled_bt() -> bool {
    // find_bt_lazy's default: ON -- adjudicated at L13-L15: totals -0.20%,
    // best dickens -1.01%, worst smallmsg +0.48%. Same arm value overrides
    // both ladders for A/Bs.
    !matches!(LAZY_GAIN_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// C's lazy gain: `4*ml - highbit(offset + 1)`.
#[inline(always)]
fn lazy_gain(ml: usize, off: usize) -> i64 {
    (ml as i64) * 4 - (63 - ((off as u64 + 1).leading_zeros() as i64))
}

/// REFUTED dispatch signals for the walk, so nobody re-tries them:
/// `rep_yield <= 0.02` left jsonlog at +2.47% (its blocks are not
/// rep-dominated), and adjacent-offset repetition (`off_rep_ratio`) never
/// fired on it at any threshold (its seq stream interleaves offsets). The
/// signal that separates losers from winners is the walk's own accept mix --
/// see `walk_first_share` on `MatchTables`.
static WALK_FIRST_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the first-find share above which the C-parity walk latches off.
pub fn set_walk_first_max_arm(v: f32) {
    WALK_FIRST_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

fn walk_first_max(attempts: usize) -> f32 {
    let c = WALK_FIRST_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if c != u32::MAX {
        return f32::from_bits(c);
    }
    // The first-find share of BOTH classes falls as the walk deepens, so the
    // bar scales with `attempts` (swept: L5 wants 0.80 -- greedy is
    // first-heavy by construction -- L7/L9 want 0.70, L12 wants 0.55; a
    // static bar leaks jsonlog at one level or over-shuts dickens at
    // another).
    // Actual ladder attempts (clevels.h search_log): L5=8, L7/L9=16, L12=64.
    if attempts <= 8 {
        0.80
    } else if attempts <= 16 {
        0.70
    } else {
        0.55
    }
}

/// Re-probe period for the walk gate (the Gate-2 shut-and-re-probe rule: an
/// immediate shut needs a scheduled reopen, or it is a one-way latch).
const WALK_PROBE_PERIOD: u32 = 16;

/// GREEDY/LAZY REP RE-PROBE arm: `rep_yield` halves on every rep-less block
/// and `rep_search_on` has no reopen on this ladder (DFast got Gate 2's
/// re-probe; greedy/lazy never did), so rep-quiet openings latch the rep
/// search off for the whole frame. Byte-CHANGING; ships on its board.
static REP_REPROBE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the greedy/lazy rep re-probe.
pub fn set_rep_reprobe_arm(on: bool) {
    REP_REPROBE_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn rep_reprobe_enabled() -> bool {
    // DEFAULT OFF -- REFUTED on its board (repro, 18 corpora x L5-L12):
    // totals -0.04% / +0.01% / +0.00% / +0.03%, worst xml +1.57% at L12.
    // The latch DFast paid for costs nothing here: the chain walk finds the
    // same matches the reopened rep search would, and reopening on
    // rep-hostile blocks trades offset economy for nothing. Arm kept for
    // study.
    matches!(REP_REPROBE_ARM.load(core::sync::atomic::Ordering::Relaxed), 2)
}

/// CHAIN-LINK TAG (win 5 of the chain-walk arc): pack the hash4 rejection
/// tag into the lazy ladder's hash HEADS ((pos+1) | tag << 24) and CHAIN
/// LINKS (pos | tag << 24), under the same < 16 MiB position proof as
/// `enable_packed_tags` -- a SEPARATE frame flag (`chain_pack`), so the
/// audited `pack_tags` contract is untouched. Every walk step then rejects
/// a colliding candidate from the tag byte ALREADY IN the link it just
/// loaded, skipping the random src[m] load that `mls_eq` would pay -- and
/// the walk-continue fix made those steps 31M-249M per board level.
/// Soundness (the T1 proof): mls >= 4 on this ladder, `mls_eq` true implies
/// the first 4 bytes equal implies tags equal -- a mismatch cannot hide a
/// match. The tag is the hash4 formula, computed from the u32 the hasher
/// already loads, and the PRIME path mirrors it exactly (the -59.3% rule).
static CHAIN_TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the chain-link tag.
pub fn set_chain_tag_arm(on: bool) {
    CHAIN_TAG_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn chain_tag_enabled() -> bool {
    !matches!(CHAIN_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// The DFast position hash pair from ONE u64 load: short index (bit-exact
/// hash4), mls-width short tag, and long index (bit-exact hash8) all derive
/// from the same 8 bytes -- `hash4_tag_mls` and `hash8` each loaded them
/// separately, an optimizer-mood CSE (the 4a30eb4 rule: own the fold).
#[inline(always)]
fn dfast_hash_pair(src: &[u8], pos: usize, dtag_shift: u32, smask: u64, hlog: u32) -> (usize, u8, usize) {
    let v = load_u64le(src, pos);
    let hv4 = (v as u32).wrapping_mul(HASH4_PRIME);
    let tv = (v & smask).wrapping_mul(FAST_HASH_PRIME64);
    let h8 = (v.wrapping_mul(0xCF1B_BCDC_B7A5_6463) >> (64u32.saturating_sub(hlog.min(32)))) as usize;
    ((hv4 >> dtag_shift) as usize, (tv ^ (tv >> 29)) as u8, h8)
}

/// hash4 index + the lazy ladder's link tag. The tag is MLS-WIDTH
/// (`hash4_tag_mls`), not 4-byte: chain buckets are keyed by the 4-byte
/// gram, so colliding candidates mostly SHARE those 4 bytes and die at byte
/// 5 -- the short table's structure, not the long table's. Measured with the
/// 4-byte tag first: only 2.4M of L12's 169M bytemiss steps caught (1.4%);
/// the byte-5 class is the whole game here. Index bit-identical to `hash4`.
#[inline(always)]
fn hash4_link_tag(src: &[u8], pos: usize, hash_log: u32, smask: u64) -> (usize, u8) {
    hash4_tag_mls(src, pos, 32u32.saturating_sub(hash_log.min(32)), smask)
}

/// WIDE-CHAIN arm: key the lazy ladder's buckets on the mls-byte gram
/// instead of 4 bytes -- the L1 wide-hash cure applied to the chain. The
/// census that motivates it: ~48% of all walk steps at L12 are collision
/// link-chases (candidates sharing the 4-byte key but not the gram); a
/// wide key never puts them in the same bucket. Byte-CHANGING (different
/// buckets, different candidates); ships on the `chainwide` board or not
/// at all.
static WCHAIN_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the wide chain key.
pub fn set_wide_chain_arm(on: bool) {
    WCHAIN_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn wide_chain_enabled() -> bool {
    // DEFAULT ON: adjudicated on the chainwide board with the hold-3 latch
    // and the attempts-scaled bar -- L5 -0.07% / L7 -0.46% / L9 -0.52% /
    // L12 -0.32% totals with ZERO losing corpora at any level.
    !matches!(WCHAIN_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

static WIDE_FIRST_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the first-find-share bar for the wide-chain latch.
pub fn set_wide_first_max_arm(v: f32) {
    WIDE_FIRST_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

static WIDE_SPB_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the searches-per-byte floor for the latch's second route.
pub fn set_wide_spb_min_arm(v: f32) {
    WIDE_SPB_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

fn wide_spb_min() -> f32 {
    let c = WIDE_SPB_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if c != u32::MAX {
        return f32::from_bits(c);
    }
    0.50
}

fn wide_first_max(attempts: usize) -> f32 {
    let c = WIDE_FIRST_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if c != u32::MAX {
        return f32::from_bits(c);
    }
    // Level-scaled like walk_first_max: jsonlog's sustained share at deep
    // attempts sits in (0.60, 0.65) and must stay excluded.
    if attempts <= 16 { 0.65 } else { 0.60 }
}

/// The sao class is CAPTURED by the latch's second route (searches/byte
/// >= 0.50: sao 0.66 against every first-heavy loser <= 0.36 -- content
/// where nearly every position searches has almost no literal+rep economy
/// for the wide key to disturb). Still unlatched, deliberately: ooffice
/// (-0.43), osdb (-0.77), mr (-0.69) -- first-heavy winners whose spb sits
/// AMONG the losers'; no maintained signal separates them, recorded as the
/// residual.
///
/// The wide-chain LATCH: at a block boundary, once walk_first_share has
/// been measured (on narrow blocks) and says upgrade-rich, re-seed the
/// HEADS over the lookback window with the wide key and latch the frame
/// wide. Chains below stale heads are miss-safe, not corrupt-safe-needing:
/// every candidate is verified by mls_eq (the relatch precedent). The
/// isolation experiment behind the bar: smallmsg loses ~+4.9% under the
/// wide key with walk-continue ON OR OFF -- the key itself is the loser
/// there -- while dickens wins ~-4% both ways.
#[inline(always)]
fn maybe_latch_wide_chain(
    tables: &mut MatchTables,
    src: &[u8],
    block_start: usize,
    window: usize,
    mls: usize,
) {
    if tables.chain_wide
        || !wide_chain_enabled()
        || mls >= 8
        || !tables.walk_share_meas
        // The wide key gets its OWN bar, and the signal must HOLD for three
        // measured blocks (see update_walk_first_share): smallmsg
        // (share ~0.74) loses ~+4.9% under the wide key and must never
        // latch; a transient dip must not latch jsonlog.
        || tables.wide_ok_blocks < 3
    {
        return;
    }
    let hash_log = tables.hash_log;
    let smask = (1u64 << (8 * mls)) - 1;
    let cp = tables.chain_pack;
    let ca = !tables.ctags.is_empty();
    let from = block_start.saturating_sub(window).max(tables.frame_start);
    let to = block_start.saturating_sub(8);
    let chain_mask = tables.chain.len() - 1;
    let mut p = from;
    while p <= to && p + 8 <= src.len() {
        let (h, g) = hash_wide_link_tag(src, p, hash_log, smask);
        // FULL insert, not heads-only: heads-only reseeding left every
        // wide bucket one deep with stale narrow-epoch links below it --
        // the latched frame walked chains of length ~1 over its whole
        // lookback. lz_insert rebuilds the links in wide keying, so the
        // latch inherits real history. Same O(window) pass.
        let _ = tables.lz_insert(h, p, g, cp, ca, chain_mask);
        p += 1;
    }
    tables.chain_wide = true;
}

/// Wide bucket key + tag from one u64 load and ONE multiply (tag and index
/// take disjoint bit ranges of the same product, the fast-hash shape).
#[inline(always)]
fn hash_wide_link_tag(src: &[u8], pos: usize, hash_log: u32, smask: u64) -> (usize, u8) {
    let v = load_u64le(src, pos) & smask;
    let hv = v.wrapping_mul(FAST_HASH_PRIME64);
    (
        (hv >> (64u32.saturating_sub(hash_log.min(32)))) as usize,
        (hv ^ (hv >> 29)) as u8,
    )
}

/// Chain-walk census: src loads the link tag skipped, and (COUNT) the
/// FALSE-skip re-probe -- a skipped candidate whose bytes would have matched
/// must never exist.
#[cfg(feature = "profile")]
pub static LINK_SKIPS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LINK_FALSE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_link_tag() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (LINK_SKIPS.swap(0, Relaxed), LINK_FALSE.swap(0, Relaxed))
}

/// Append `src[from..to]` to `lits` with the range proof supplied by the
/// caller: every finder emit site maintains anchor <= from <= to <=
/// block_end <= src.len(). The checked slice op compiled to a bounds test
/// plus a panic branch per SEQUENCE in each finder.
#[inline(always)]
#[allow(unsafe_code)]
fn push_lits_range(lits: &mut Vec<u8>, src: &[u8], from: usize, to: usize) {
    debug_assert!(from <= to && to <= src.len());
    lits.extend_from_slice(unsafe { src.get_unchecked(from..to) });
}

/// Attribute only when the walk RAN and produced enough samples -- a block
/// that measured nothing must not move the EWMA (the Gate 14 rule).
fn update_walk_first_share(tables: &mut MatchTables, walked: bool, cls: (u32, u32), attempts: usize) {
    let n = cls.0 + cls.1;
    if !walked || n < 64 {
        return;
    }
    let now = cls.0 as f32 / n as f32;
    // The FIRST measurement SEEDS the EWMA. Blending it with the 0.0 init
    // made every frame read as upgrade-rich for its first ~4 measured
    // blocks (smallmsg's true 0.74 entered the wide-chain latch reading
    // 0.185), which is a warmup artifact, not a signal.
    tables.walk_first_share = if tables.walk_share_meas {
        0.75 * tables.walk_first_share + 0.25 * now
    } else {
        now
    };
    tables.walk_share_meas = true;
    // The wide-chain latch is ONE-WAY per frame, so a TRANSIENT dip must not
    // fire it (jsonlog's EWMA dips under any bar at L12 and latched wide for
    // +3.3%). Require the signal to HOLD.
    // Second admission route (the sao capture): among first-heavy content
    // the census separates the wide-key winners' king by SEARCH DENSITY --
    // sao runs 0.66 searches/byte, twice any first-heavy loser (smallmsg
    // 0.29, jsonlog 0.18, x-ray 0.36). Nearly every position searching
    // means the literal+rep economy the wide key would disturb barely
    // exists.
    if tables.walk_first_share <= wide_first_max(attempts)
        || tables.last_search_per_byte >= wide_spb_min()
    {
        tables.wide_ok_blocks = tables.wide_ok_blocks.saturating_add(1);
    } else {
        tables.wide_ok_blocks = 0;
    }
}

/// Signal probe statics (see the find_lazy epilogue).
#[cfg(feature = "profile")]
pub static WALK_SIG_FIRST: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_REP: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_SPB: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_MB: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_NS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_OB: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_walk_signals() -> (f32, f32, f32, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        f32::from_bits(WALK_SIG_FIRST.load(Relaxed)),
        f32::from_bits(WALK_SIG_REP.load(Relaxed)),
        f32::from_bits(WALK_SIG_SPB.load(Relaxed)),
        WALK_SIG_MB.load(Relaxed),
        WALK_SIG_NS.load(Relaxed),
        WALK_SIG_OB.load(Relaxed),
    )
}

/// Back-extension census for the SIMD question: (extensions > 0, total
/// bytes, extensions >= 8 -- the class a u64 backward step would win).
#[cfg(feature = "profile")]
pub static BEXT_N: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_BYTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_GE8: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_MATCHES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_bext() -> (u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        BEXT_MATCHES.swap(0, Relaxed),
        BEXT_N.swap(0, Relaxed),
        BEXT_BYTES.swap(0, Relaxed),
        BEXT_GE8.swap(0, Relaxed),
    )
}

#[cfg(feature = "profile")]
fn note_bext(ext: u64) {
    use core::sync::atomic::Ordering::Relaxed;
    BEXT_MATCHES.fetch_add(1, Relaxed);
    if ext > 0 {
        BEXT_N.fetch_add(1, Relaxed);
        BEXT_BYTES.fetch_add(ext, Relaxed);
        if ext >= 8 {
            BEXT_GE8.fetch_add(1, Relaxed);
        }
    }
}

/// Chain-walk census: (candidates examined, byte-mismatch steps).
#[cfg(feature = "profile")]
pub static WALK_EXAM: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_BYTEMISS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Walk-continue accept classes: (first-find past a collision -- legacy would
/// have emitted a literal; upgrade past a collision -- legacy had a match).
#[cfg(feature = "profile")]
pub static WALK_CONT_FIRST: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_CONT_UPGRADE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_walk_classes() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (WALK_CONT_FIRST.swap(0, Relaxed), WALK_CONT_UPGRADE.swap(0, Relaxed))
}
#[cfg(feature = "profile")]
pub fn take_walk_census() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (WALK_EXAM.swap(0, Relaxed), WALK_BYTEMISS.swap(0, Relaxed))
}

#[inline(always)]
fn match_ok(
    src: &[u8],
    m: usize,
    ip: usize,
    window: usize,
    block_start: usize,
    mls: usize,
    frame_start: usize,
) -> bool {
    if m >= ip || ip - m > window {
        return false;
    }
    let lowest = block_start.saturating_sub(window).max(frame_start);
    if m < lowest {
        return false;
    }
    if ip + mls > src.len() || m + mls > src.len() {
        return false;
    }
    // The tail slice-eq compiled to a LIBC MEMCMP CALL per candidate (for
    // mls = 5, comparing ONE byte) -- the mls_eq lesson, applied to the
    // shared validity helper. Self-proving: the u64 path runs only when its
    // own 8-byte reads are in bounds (m < ip from the order check above).
    if mls <= 8 && ip + 8 <= src.len() {
        debug_assert!(m + 8 <= src.len());
        let mask = if mls == 8 { u64::MAX } else { (1u64 << (8 * mls)) - 1 };
        return (load_u64le(src, m) ^ load_u64le(src, ip)) & mask == 0;
    }
    match_ok_cold_tail(src, m, ip, mls)
}

/// The mls > 8 arm, outlined and cold: inlining match_ok replicated this
/// slice-eq (a static memcmp site) at every caller for a branch no real
/// level reaches.
#[cold]
#[inline(never)]
fn match_ok_cold_tail(src: &[u8], m: usize, ip: usize, mls: usize) -> bool {
    if mls >= 4 {
        if load_u32le(src, m) != load_u32le(src, ip) {
            return false;
        }
        return mls == 4 || src[m + 4..m + mls] == src[ip + 4..ip + mls];
    }
    src[m..m + mls] == src[ip..ip + mls]
}

/// The per-candidate fast head of `count_match`: the first-word peek fully
/// INLINE -- no call, no `has_avx2` atomic, no slice construction -- with
/// the outlined routine only for the (rare) long tail. Value-identical to
/// `count_match` at every input: the head fires only when all three
/// 8-byte reads are in bounds and within `limit`, a first-word mismatch
/// answers <= 7 <= max, and an equal first word makes the total exactly
/// `8 + count_match(m+8, ip+8)`. The eqlen histogram says ~79% of calls
/// end in the head.
#[inline(always)]
fn count_match_fast(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    // CALL-SITE INVARIANTS (audited, all 17 sites): `limit` is block_end,
    // which is a position in `src`, and `m` is a candidate strictly below
    // `ip`. So `ip + 8 <= limit` implies both slice tests that used to sit
    // beside it -- three compares collapse to one, per candidate.
    debug_assert!(limit <= src.len() && m < ip);
    if ip + 8 <= limit {
        let a = load_u64le(src, m);
        let b = load_u64le(src, ip);
        if a != b {
            return ((a ^ b).trailing_zeros() as usize) >> 3;
        }
        8 + count_match(src, m + 8, ip + 8, limit)
    } else {
        count_match(src, m, ip, limit)
    }
}

fn count_match(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    // Same invariants as `count_match_fast`. They make every min redundant:
    // `len - m > len - ip >= limit - ip`, so max IS `limit - ip`, and the
    // three range guards collapse to `ip >= limit`. The slice constructions
    // keep memory safety: a violated invariant panics, it cannot read wild.
    debug_assert!(limit <= src.len() && m < ip);
    if ip >= limit {
        return 0;
    }
    let max = limit - ip;
    let a = &src[m..m + max];
    let b = &src[ip..limit];
    // Sub-8 boundary tails answer HERE, without the dispatch or the call.
    if max < 8 {
        let mut n = 0usize;
        while n < max && a[n] == b[n] {
            n += 1;
        }
        #[cfg(feature = "profile")]
        crate::simd::note_eqlen(n);
        return n;
    }
    // The slices have PROVEN equal length `max >= 8`; the known-length inner
    // skips the re-min / zero-test / sub-8 re-branch the public entry does.
    let n = crate::simd::count_eq_len_ge8(a, b, max);
    #[cfg(feature = "profile")]
    crate::simd::note_eqlen(n);
    n
}

const HASH4_PRIME: u32 = 2_654_435_761;

#[inline(always)]
fn load_u32le(src: &[u8], i: usize) -> u32 {
    crate::simd::load_u32_le(src, i)
}

#[inline(always)]
fn load_u64le(src: &[u8], i: usize) -> u64 {
    crate::simd::load_u64_le(src, i)
}

/// T2/GATE 6: reset a kept scratch vector to `n` copies of `val` without a
/// growth `realloc`.
///
/// The contents are dead at this point, so growing through `reserve` would
/// memcpy a buffer holding nothing live -- the defect that made the first
/// `opt_ops` attempt cost more than it saved. Replacing instead copies nothing.
#[inline(always)]
/// Grow-only sizing for write-before-read scratch: never refills.
fn ensure_len<T: Clone>(v: &mut Vec<T>, n: usize, val: T) {
    if v.len() < n {
        v.resize(n, val);
    }
}

fn reset_to<T: Clone>(v: &mut Vec<T>, n: usize, val: T) {
    if v.capacity() < n {
        *v = Vec::with_capacity(n);
    }
    v.clear();
    v.resize(n, val);
}

/// T2: C's `match[ml] == ip[ml]` prefilter, without its two bounds checks.
///
/// SAFETY: only reached with `best_ml > 0`, and the loop above `break`s the
/// moment `ip + best_ml >= block_end`. So any candidate that gets here has
/// `ip + off < block_end <= src.len()`, and `match_ok` has already established
/// that `m` is a past position (`m < ip`), giving `m + off < ip + off`.
#[inline(always)]
#[allow(unsafe_code)]
fn pre_eq(src: &[u8], m: usize, ip: usize, off: usize) -> bool {
    debug_assert!(m + off < src.len() && ip + off < src.len());
    unsafe { *src.get_unchecked(m + off) == *src.get_unchecked(ip + off) }
}

/// T2: one byte compare for the back-extension walk, without the two bounds
/// checks the indexed form pays on EVERY byte it extends.
///
/// SAFETY, and it is what the loop condition already establishes:
///   * the caller tests `s > anchor` before calling, so `s >= 1` and `s - 1`
///     cannot wrap; likewise `mm > tables.frame_start` gives `mm >= 1`.
///   * `s` starts at a scan position inside the block (`< block_end <=
///     src.len()`) and only ever decreases; `mm` starts at a match position
///     strictly below it. So both `s - 1` and `mm - 1` are `< src.len()`.
///
/// The three back-extension loops (`find_greedy`, `find_lazy`, `find_bt_lazy`)
/// carried 2 panic sites each -- 6 of the 10 left after the DFast and Bt
/// tranches -- and they sit in a PER-BYTE loop, which is the worst place in the
/// encoder to pay a bounds check.
#[inline(always)]
#[allow(unsafe_code)]
/// SIMD/u64-WIDENING REFUTED BY CENSUS (2026-08-21, `take_bext`, 18
/// corpora x L1..L13): only 7-13% of matches back-extend at all, the mean
/// extension among those is 1.1-1.4 BYTES, and the >= 8-byte class a u64
/// backward step would win is 0.39% of extensions at L1 and ~zero from L5
/// up. A widened step pays two 8-byte loads, xor, lzcnt and two boundary
/// guards to answer what is ~93% of the time a single byte compare --
/// while reading 14 unneeded bytes backward across a possible extra cache
/// line. The byte loop IS the right shape for this distribution. (The
/// same census machinery stays under profile for re-adjudication if match
/// geometry ever changes.)
fn back_eq(src: &[u8], s: usize, mm: usize) -> bool {
    debug_assert!(s >= 1 && mm >= 1 && s - 1 < src.len() && mm - 1 < src.len());
    unsafe { *src.get_unchecked(s - 1) == *src.get_unchecked(mm - 1) }
}

fn hash4(v: u32, hash_log: u32) -> usize {
    let shift = 32u32.saturating_sub(hash_log.min(32));
    (v.wrapping_mul(HASH4_PRIME) >> shift) as usize
}

#[inline(always)]
fn hash8(src: &[u8], ip: usize, hash_log: u32) -> usize {
    let v = load_u64le(src, ip);
    let shift = 64u32.saturating_sub(hash_log.min(32));
    (v.wrapping_mul(0xCF1B_BCDC_B7A5_6463) >> shift) as usize
}


#[inline(always)]
fn hash_mls(src: &[u8], ip: usize, mls: usize, hash_log: u32) -> usize {
    if mls >= 8 && ip + 8 <= src.len() {
        hash8(src, ip, hash_log)
    } else {
        hash4(load_u32le(src, ip), hash_log)
    }
}

/// Used by the streaming compressor to checksum incrementally.
pub(crate) fn checksum_u32(h: &Xxh64) -> u32 {
    h.digest() as u32
}

/// Huffman + FSE NCount headers + reps harvested from samples vs dict content.
#[cfg(feature = "std")]
pub(crate) struct HarvestedEntropy {
    pub huff: huffman::HuffCTable,
    pub of_nc: Vec<u8>,
    pub ml_nc: Vec<u8>,
    pub ll_nc: Vec<u8>,
    pub reps: [u32; 3],
}

/// Harvest Huffman + FSE NCount + reps from samples matched against `content`.
#[cfg(feature = "std")]
pub(crate) fn harvest_dict_entropy(
    content: &[u8],
    samples: &[&[u8]],
) -> Result<HarvestedEntropy, Error> {
    let hint = samples.iter().map(|s| s.len() as u64).sum::<u64>().max(1);
    let params = compression_params(3, Some(hint))?;
    let mut tables = MatchTables::new(params);
    let window = 1usize << params.window_log.min(31);
    let block_max = (window.min(BLOCKSIZE_MAX as usize)).max(1);
    let mut lit_freq = [0u32; 256];
    let mut ll_count = [0u32; 36];
    let mut of_count = [0u32; 32];
    let mut ml_count = [0u32; 53];
    let mut reps = [1u32, 4, 8];
    for sample in samples {
        if sample.is_empty() {
            continue;
        }
        let mut owned = Vec::with_capacity(content.len() + sample.len());
        owned.extend_from_slice(content);
        owned.extend_from_slice(sample);
        tables.reset();
        prime_tables(&mut tables, &owned, content.len(), window, params);
        let mut off = content.len();
        while off < owned.len() {
            let end = (off + block_max).min(owned.len());
            let (seqs, lits) = find_sequences(
                &owned,
                off,
                end,
                window,
                params,
                &mut tables,
                None,
                crate::ldm::LdmParams::default(),
                [1, 4, 8],
            );
            for &b in &lits {
                lit_freq[b as usize] = lit_freq[b as usize].saturating_add(1);
            }
            for s in &seqs {
                let ov = offset_value_for(s.offset, s.litlen, &reps);
                if resolve_offset(ov, s.litlen, &mut reps).is_err() {
                    continue;
                }
                let (llc, _, _) = ll_code(s.litlen, true);
                let (mlc, _, _) = ml_code(s.matchlen, true);
                let (ofc, _) = of_code(ov);
                if (llc as usize) < ll_count.len() {
                    ll_count[llc as usize] = ll_count[llc as usize].saturating_add(1);
                }
                if (ofc as usize) < of_count.len() {
                    of_count[ofc as usize] = of_count[ofc as usize].saturating_add(1);
                }
                if (mlc as usize) < ml_count.len() {
                    ml_count[mlc as usize] = ml_count[mlc as usize].saturating_add(1);
                }
            }
            off = end;
        }
    }
    let huff = huffman::build_ctable_from_freq(&pad_lit_freq(lit_freq))?;
    let of_nc = ncount_or_default(&of_count, 8, &fse::DEFAULT_OF_NORM, 5)?;
    let ml_nc = ncount_or_default(&ml_count, 9, &fse::DEFAULT_ML_NORM, 6)?;
    let ll_nc = ncount_or_default(&ll_count, 9, &fse::DEFAULT_LL_NORM, 6)?;
    let clen = content.len() as u32;
    let reps = clamp_reps(reps, clen);
    Ok(HarvestedEntropy {
        huff,
        of_nc,
        ml_nc,
        ll_nc,
        reps,
    })
}

#[cfg(feature = "std")]
fn pad_lit_freq(mut freq: [u32; 256]) -> [u32; 256] {
    let n = freq.iter().filter(|&&c| c > 0).count();
    if n < 2 {
        freq[0] = freq[0].saturating_add(1);
        freq[1] = freq[1].saturating_add(1);
        freq[255] = freq[255].saturating_add(1);
    }
    freq
}

#[cfg(feature = "std")]
fn ncount_or_default(
    count: &[u32],
    max_log: u8,
    default_norm: &[i16],
    default_log: u8,
) -> Result<Vec<u8>, Error> {
    let mut buf = count.to_vec();
    let total: u32 = buf.iter().sum();
    if total == 0 {
        return fse::write_ncount(default_norm, default_log);
    }
    let max_sv = buf.iter().rposition(|&c| c > 0).unwrap_or(0);
    if buf[max_sv] == total {
        let other = if max_sv == 0 { 1 } else { 0 };
        if other < buf.len() {
            buf[other] = buf[other].saturating_add(1);
        }
    }
    match fse::ncount_and_ctable(&buf, max_log, false) {
        Ok((hdr, _)) => Ok(hdr),
        Err(_) => fse::write_ncount(default_norm, default_log),
    }
}

#[cfg(feature = "std")]
fn clamp_reps(mut reps: [u32; 3], content_len: u32) -> [u32; 3] {
    let cap = content_len.max(1);
    for r in &mut reps {
        if *r == 0 || *r > cap {
            *r = ((*r) % cap).max(1);
        }
    }
    reps
}

#[cfg(test)]
mod tests {

    /// The fixed-width literal push must equal `extend_from_slice` for every
    /// run length and every capacity, including the fallback cases (run > 16,
    /// no spare capacity, source too close to the end of the input).
    #[test]
    fn push_literals_matches_extend_from_slice() {
        let src: Vec<u8> = (0..512u32).map(|i| (i % 251) as u8).collect();
        for from in [0usize, 1, 7, 100, 495, 500, 511] {
            for n in 0usize..=40 {
                if from + n > src.len() {
                    continue;
                }
                for spare in [0usize, 1, 15, 16, 31, 32, 1024] {
                    // GATE 13: every width the dispatch can select, plus the
                    // gate-off arm (0). All must equal `extend_from_slice` --
                    // the copy writes `w` bytes but publishes only `n`.
                    for w in [0usize, super::LIT_PUSH_WIDTH, super::LIT_PUSH_WIDTH_WIDE] {
                        let mut fast = Vec::with_capacity(4 + spare);
                        fast.extend_from_slice(b"HEAD");
                        let mut want = fast.clone();
                        want.extend_from_slice(&src[from..from + n]);
                        super::push_literals(&mut fast, &src, from, from + n, w);
                        assert_eq!(fast, want, "from={from} n={n} spare={spare} w={w}");
                    }
                }
            }
        }
    }
    use super::*;
    use crate::{decompress, frame_block_census};

    fn rt(src: &[u8], level: i32) {
        let zst = compress(src, level).expect("compress");
        let got = decompress(&zst).unwrap_or_else(|e| {
            panic!(
                "decompress our frame L{level} src={} zst={}: {e:?}",
                src.len(),
                zst.len()
            )
        });
        assert_eq!(got.len(), src.len(), "len level={level}");
        if got != src {
            let pos = got
                .iter()
                .zip(src.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len());
            panic!(
                "mismatch L{level} at {pos}/{} got={:02x} want={:02x} zst={}",
                src.len(),
                got.get(pos).copied().unwrap_or(0),
                src.get(pos).copied().unwrap_or(0),
                zst.len()
            );
        }
    }

    #[test]
    fn silesia_mr_prefix_finder_recon() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia/mr");
        if !path.is_file() {
            return;
        }
        let mut src = std::fs::read(&path).expect("read mr");
        src.truncate(277_521);
        let params = crate::compression_params(1, Some(src.len() as u64)).expect("params");
        let mut tables = MatchTables::new(params);
        let window = 1usize << params.window_log.min(31);
        let mut off = 0usize;
        let mut recon = Vec::new();
        // Mirror `encode_block`: the repeat offsets evolve across blocks, and
        // `find_fast` now SEARCHES them, so a stale [1,4,8] per block would
        // generate different sequences than the real encoder did.
        let mut oracle_reps = [1u32, 4, 8];
        while off < src.len() {
            let end = (off + crate::BLOCKSIZE_MAX as usize).min(src.len());
            let (seqs, lits) = find_fast(&src, off, end, window, params, &mut tables, oracle_reps);
            for sq in &seqs {
                let ov = crate::compressed::offset_value_for(sq.offset, sq.litlen, &oracle_reps);
                let _ = crate::compressed::resolve_offset(ov, sq.litlen, &mut oracle_reps);
            }
            let mut lit_at = 0usize;
            for s in &seqs {
                let n = s.litlen as usize;
                recon.extend_from_slice(&lits[lit_at..lit_at + n]);
                lit_at += n;
                let start = recon
                    .len()
                    .checked_sub(s.offset as usize)
                    .unwrap_or_else(|| {
                        panic!(
                            "offset {} > recon {} off={off} ml={} ll={}",
                            s.offset,
                            recon.len(),
                            s.matchlen,
                            s.litlen
                        )
                    });
                for k in 0..s.matchlen as usize {
                    recon.push(recon[start + k]);
                }
            }
            recon.extend_from_slice(&lits[lit_at..]);
            off = end;
        }
        assert_eq!(recon.as_slice(), src.as_slice(), "finder recon vs src");
    }

    /// Split Huffman literals vs FSE sequences on the 277521 `mr` prefix.
    #[test]
    fn silesia_mr_prefix_entropy_oracle() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia/mr");
        if !path.is_file() {
            return;
        }
        let mut src = std::fs::read(&path).expect("read mr");
        src.truncate(277_521);
        let params = crate::compression_params(1, Some(src.len() as u64)).expect("params");
        let mut tables = MatchTables::new(params);
        // Mirror the encode path's table init (encode_frame): `pack_tags` now
        // participates in WIDE dispatch, so an oracle that skips this runs a
        // different finder arm than the frame it is checking against.
        tables.enable_packed_tags(
            params.strategy == Strategy::Fast && tag_alloc_enabled() && fast_pack_enabled(),
            src.len(),
        );
        let window = 1usize << params.window_log.min(31);
        let zst = compress_with(
            &src,
            CompressOptions {
                level: 1,
                checksum: false,
            },
        )
        .expect("nocheck");

        use crate::block::{parse_block_header, BlockType};
        use crate::compressed::BlockState;
        use crate::frame::parse_kind;
        use crate::reader::Reader;
        let mut r = Reader::new(&zst);
        parse_kind(&mut r).expect("frame");
        let mut state = BlockState::new();
        let mut off = 0usize;
        let mut block_i = 0u32;
        let mut decoded = Vec::new();
        let mut oracle_reps = [1u32, 4, 8];
        loop {
            let bh = parse_block_header(&mut r).expect("bh");
            let payload = r.take(bh.payload_len() as usize).expect("payload");
            let end = (off + crate::BLOCKSIZE_MAX as usize).min(src.len());
            let (seqs, lits) = if bh.ty == BlockType::Rle {
                (Vec::new(), Vec::new())
            } else {
                find_fast(&src, off, end, window, params, &mut tables, oracle_reps)
            };
            for sq in &seqs {
                let ov = crate::compressed::offset_value_for(sq.offset, sq.litlen, &oracle_reps);
                let _ = crate::compressed::resolve_offset(ov, sq.litlen, &mut oracle_reps);
            }
            match bh.ty {
                BlockType::Compressed => {
                    let mut lr = Reader::new(payload);
                    let got_lits = crate::compressed::decode_literals(&mut lr, &mut state)
                        .unwrap_or_else(|e| panic!("block {block_i} literals: {e:?}"));
                    let lit_pos = got_lits
                        .iter()
                        .zip(lits.iter())
                        .position(|(a, b)| a != b)
                        .unwrap_or(got_lits.len().min(lits.len()));
                    assert_eq!(
                        got_lits.as_slice(),
                        lits.as_slice(),
                        "block {block_i} Huffman lits mismatch at {lit_pos}/enc={} dec={} nseq={}",
                        lits.len(),
                        got_lits.len(),
                        seqs.len()
                    );
                    let seq_bytes = lr.take(lr.remaining()).expect("seq bytes");
                    let (nseq_d, modes, got_codes) =
                        crate::compressed::debug_seq_codes(seq_bytes, &state)
                            .unwrap_or_else(|e| panic!("block {block_i} seq codes: {e:?}"));
                    let mut reps = state.reps;
                    let mut want_codes = Vec::new();
                    for s in &seqs {
                        let ov = offset_value_for(s.offset, s.litlen, &reps);
                        let _ = resolve_offset(ov, s.litlen, &mut reps).expect("ov");
                        let (llc, _, _) = ll_code(s.litlen, true);
                        let (mlc, _, _) = ml_code(s.matchlen, true);
                        let (ofc, _) = of_code(ov);
                        want_codes.push((s.litlen, s.matchlen, ov, llc, mlc, ofc));
                    }
                    if got_codes != want_codes {
                        let i = got_codes
                            .iter()
                            .zip(want_codes.iter())
                            .position(|(a, b)| a != b)
                            .unwrap_or(got_codes.len().min(want_codes.len()));
                        panic!(
                            "block {block_i} seq codes mismatch at {i}/enc={} dec={} nseq_d={nseq_d} modes={modes:#04x}\n  got={:?}\n want={:?}\n last_got={:?}\n last_want={:?}",
                            want_codes.len(),
                            got_codes.len(),
                            got_codes.get(i),
                            want_codes.get(i),
                            got_codes.last(),
                            want_codes.last()
                        );
                    }
                    crate::compressed::decode_sequences(
                        seq_bytes,
                        &got_lits,
                        &mut decoded,
                        1u64 << params.window_log.min(31),
                        crate::BLOCKSIZE_MAX,
                        &mut state,
                        &[],
                        0,
                        0,
                    )
                    .unwrap_or_else(|e| panic!("block {block_i} seqs: {e:?}"));
                    let got = &decoded[off..decoded.len().min(end)];
                    let want = &src[off..end];
                    if got != want {
                        let pos = got
                            .iter()
                            .zip(want.iter())
                            .position(|(a, b)| a != b)
                            .unwrap_or(got.len().min(want.len()));
                        panic!(
                            "block {block_i} FSE/exec mismatch at {pos}/{} nseq={} last={:?} trail={}",
                            end - off,
                            seqs.len(),
                            seqs.last(),
                            lits.len() as u32 - seqs.iter().map(|s| s.litlen).sum::<u32>()
                        );
                    }
                }
                BlockType::Raw => {
                    assert_eq!(&payload[..], &src[off..end], "block {block_i} raw");
                    decoded.extend_from_slice(payload);
                }
                BlockType::Rle => {
                    decoded.resize(decoded.len() + (end - off), payload[0]);
                }
            }
            off = end;
            block_i += 1;
            if bh.last {
                break;
            }
        }
        assert_eq!(off, src.len());
    }

    /// Real Silesia `mr` prefix: finder + entropy + us decode.
    #[test]
    fn silesia_mr_prefix_roundtrip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia/mr");
        if !path.is_file() {
            return;
        }
        let mut src = std::fs::read(&path).expect("read mr");
        src.truncate(277_521);
        let z_off = compress_with(
            &src,
            CompressOptions {
                level: 1,
                checksum: false,
            },
        )
        .expect("nocheck");
        let got = crate::decompress(&z_off).expect("decompress");
        if got.as_slice() != src.as_slice() {
            let pos = got
                .iter()
                .zip(src.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(src.len()));
            panic!(
                "first mismatch at {pos}/{} got_len={} zst={}",
                src.len(),
                got.len(),
                z_off.len()
            );
        }
    }

    #[test]
    fn silesia_all_oneshot_roundtrip_l1() {
        let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia");
        if !dir.is_dir() {
            return;
        }
        let mut files: Vec<_> = std::fs::read_dir(&dir)
            .expect("silesia dir")
            .filter_map(|e| e.ok())
            .map(|e| e.path())
            .filter(|p| p.is_file())
            .collect();
        files.sort();
        assert!(
            !files.is_empty(),
            "silesia dir exists but has no files: {}",
            dir.display()
        );
        for path in files {
            let src =
                std::fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
            let zst =
                compress(&src, 1).unwrap_or_else(|e| panic!("compress {}: {e:?}", path.display()));
            let got = crate::decompress(&zst)
                .unwrap_or_else(|e| panic!("decompress {}: {e:?}", path.display()));
            if got.as_slice() != src.as_slice() {
                let pos = got
                    .iter()
                    .zip(src.iter())
                    .position(|(a, b)| a != b)
                    .unwrap_or(got.len().min(src.len()));
                panic!(
                    "{} mismatch at {pos}/{} zst={}",
                    path.file_name().unwrap().to_string_lossy(),
                    src.len(),
                    zst.len()
                );
            }
        }
    }

    /// A HIGHER level must never produce a LARGER file. This FAILED on osdb --
    /// L5 (greedy) and L7 (lazy2) both emitted more than L3 (dfast) -- until
    /// defects B2 and B3 were fixed. Kept as a standing gate: it is the only
    /// thing that catches a match finder silently losing a capability its
    /// cheaper neighbour has.
    ///
    ///   level  strategy  was         now         C
    ///   3      dfast     3,613,320   3,613,320   3,501,634
    ///   5      greedy    3,658,497   3,520,086   3,431,228
    ///   7      lazy2     3,625,184   3,461,023   3,359,176
    ///   13     btlazy2   3,370,051   3,152,921
    ///
    /// B2: the lazy look-ahead inserted `ip+1 ..= ip+depth` into the chain, then
    /// the back-fill re-inserted them -- `chain[p] = p`, a self-loop the walk
    /// breaks on, amputating that hash bucket's whole history (501,705 of them
    /// at L7). B3: greedy/lazy/btlazy2 never back-extended a match, a capability
    /// `emit_fast_seq` (fast/dfast) has always had and C's lazy has as its
    /// "catch up" loop.
    #[test]
    fn higher_level_never_larger_osdb() {
        let Ok(src) = std::fs::read("../../corpora/data/silesia/osdb") else {
            return; // corpus absent
        };
        let mut prev = usize::MAX;
        for lvl in [1, 3, 5, 7, 9, 13, 16, 19] {
            let n = crate::compress(&src, lvl).unwrap().len();
            assert!(
                n <= prev,
                "level {lvl} emitted {n} bytes, more than the previous level's {prev}"
            );
            prev = n;
        }
    }

    /// Truth table for the probe-density (pair-search) dispatch: per file, the
    /// L1 size delta from `RZSTD_STEP0=1` joined to the finder's own counters
    /// measured at the shipping `step0=2`. Needs `--features profile`.
    #[ignore]
    #[test]
    fn probe_density_truth_table() {
        const FILES: &[&str] = &[
            "dickens", "mozilla", "mr", "nci", "ooffice", "osdb", "reymont", "samba", "sao",
            "webster", "x-ray", "xml",
        ];
        println!(
            "TT {:<9} {:>10} {:>9} {:>9} {:>10} {:>9}",
            "file", "probes/B", "hit_rate", "matchfrac", "lit_share", "seqs/B"
        );
        for f in FILES {
            let Ok(src) = std::fs::read(format!("../../corpora/data/silesia/{f}")) else {
                continue;
            };
            crate::prof::reset();
            let n = crate::compress(&src, 1).unwrap().len();
            let c = crate::prof::encode_counts();
            let b = src.len() as f64;
            println!(
                "TT {f:<9} {:>10.4} {:>9.4} {:>9.4} {:>10.4} {:>9.5}  size={n}",
                c.hash_probes as f64 / b,
                c.probe_hits as f64 / (c.hash_probes.max(1)) as f64,
                c.match_bytes as f64 / b,
                c.lit_bytes as f64 / b,
                c.seqs as f64 / b,
            );
        }
    }

    /// P0/gg-matchfind gate: the deterministic WORK COUNTER must be non-zero at
    /// EVERY level, for every strategy. It was reported only by `find_fast` and
    /// `find_lazy`, so `work` -- the PRIMARY evidence under the Great Gate
    /// 2026-08-06 law -- did not exist for levels 2-22 and no gate on them could
    /// be banked. Needs `--features profile`.
    #[cfg(feature = "profile")]
    #[test]
    fn work_counter_covers_every_strategy() {
        let Ok(src) = std::fs::read("../../corpora/data/silesia/xml") else {
            return; // corpus absent
        };
        // one level per strategy: fast, dfast, greedy, lazy, lazy2, btlazy2, btopt, btultra
        for (lvl, strat) in [
            (1, "fast"),
            (3, "dfast"),
            (5, "greedy"),
            (7, "lazy"),
            (9, "lazy2"),
            (13, "btlazy2"),
            (17, "btopt"),
            (19, "btultra"),
        ] {
            crate::prof::reset();
            let _ = crate::compress(&src, lvl).unwrap();
            let c = crate::prof::encode_counts();
            println!(
                "WORK L{lvl:<2} {strat:<8} probes={:<12} hits={:<10} seqs={:<10}",
                c.hash_probes, c.probe_hits, c.seqs
            );
            assert!(
                c.hash_probes > 0,
                "L{lvl} ({strat}) reported ZERO probes -- the work counter is \
                 missing for this strategy, so no gate on it is bankable"
            );
            assert!(c.seqs > 0, "L{lvl} ({strat}) reported zero sequences");
        }
    }

    /// Deterministic compressed-size table over Silesia -- run before and
    /// after a size-affecting change and diff the rows.
    #[ignore]
    #[test]
    fn size_table_silesia() {
        const FILES: &[&str] = &[
            "dickens", "mozilla", "mr", "nci", "ooffice", "osdb", "reymont", "samba", "sao",
            "webster", "x-ray", "xml",
        ];
        for f in FILES {
            let Ok(src) = std::fs::read(format!("../../corpora/data/silesia/{f}")) else {
                continue;
            };
            let mut row = format!("{f:<8}");
            for lvl in [5, 7, 9, 13, 19] {
                let n = crate::compress(&src, lvl).unwrap().len();
                row.push_str(&format!(" L{lvl}={n}"));
            }
            println!("SIZETABLE {row}");
        }
    }

    #[test]
    fn census_zeros_all_rle() {
        let src = vec![0u8; 128 * 1024 * 2];
        let zst = compress(&src, 1).expect("compress");
        let c = crate::frame_block_census(&zst).expect("census");
        assert_eq!(c.compressed, 0);
        assert_eq!(c.raw, 0);
        assert_eq!(c.rle, 2);
        assert_eq!(c.rle_regen, src.len() as u64);
    }

    #[test]
    fn frame_checksum_matches_oneshot_xxh64() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut text = Vec::new();
        while text.len() < 200_000 {
            text.extend_from_slice(fox);
        }
        for src in [&b""[..], b"a", &[0u8; 128 * 1024 + 7][..], text.as_slice()] {
            let zst = compress(src, 1).expect("compress");
            assert!(zst.len() >= 4);
            let got = u32::from_le_bytes(zst[zst.len() - 4..].try_into().unwrap());
            assert_eq!(got, content_checksum(src), "len {}", src.len());
        }
    }

    #[test]
    fn roundtrip_small_all_fast_levels() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut text = Vec::new();
        while text.len() < 8192 {
            text.extend_from_slice(fox);
        }
        for level in -7i32..=3 {
            rt(b"", level);
            rt(b"a", level);
            rt(b"hello", level);
            rt(&[0u8; 16], level);
            rt(&[0u8; 256], level);
            rt(&[0u8; 4096], level);
            rt(&text, level);
            rt(&xorshift(0xA5A5_5A5A, 1024), level);
            rt(&xorshift(0xA5A5_5A5A, 64 * 1024), level);
        }
    }

    #[test]
    fn roundtrip_mid_and_high_levels() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut text = Vec::new();
        while text.len() < 8192 {
            text.extend_from_slice(fox);
        }
        let noise = xorshift(0xA5A5_5A5A, 8192);
        for level in [4, 5, 6, 8, 9, 13, 16, 19] {
            rt(&text, level);
            rt(&noise, level);
            rt(&[0u8; 1024], level);
        }
    }

    #[test]
    fn huffman_literals_emitted_on_text() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut text = Vec::new();
        while text.len() < 224 {
            text.extend_from_slice(fox);
        }
        text.truncate(224);
        let (sec, upd) = crate::huffman::encode_literals_section(&text, None).unwrap();
        assert_eq!(sec[0] & 3, 2, "expected Huffman Compressed literals");
        match upd {
            crate::huffman::HuffUpdate::New(_) => {}
            crate::huffman::HuffUpdate::Unchanged => panic!("expected a new Huffman table"),
        }
        let zst = compress(&text, 1).unwrap();
        assert_eq!(decompress(&zst).unwrap(), text);
    }

    #[test]
    fn roundtrip_greedy_explicit() {
        let opts = CompressOptions {
            level: 5,
            ..CompressOptions::default()
        };
        let src = xorshift(0x1111_2222, 32 * 1024);
        let zst = compress_with(&src, opts).unwrap();
        assert_eq!(decompress(&zst).unwrap(), src);
    }

    #[test]
    fn zeros_and_text_shrink() {
        let zeros = vec![0u8; 4096];
        let zst = compress(&zeros, 1).unwrap();
        assert!(
            zst.len() < zeros.len(),
            "zeros L1 {} vs {}",
            zst.len(),
            zeros.len()
        );
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(64);
        let zst = compress(&fox, 3).unwrap();
        assert!(
            zst.len() < fox.len(),
            "text L3 {} vs {}",
            zst.len(),
            fox.len()
        );
    }

    #[test]
    fn rle_byte_word_matches_byte_all() {
        assert_eq!(rle_byte(&[7, 7, 7, 7, 7, 7, 7, 7, 7]), Some(7));
        assert_eq!(rle_byte(&[7, 7, 7, 7, 7, 7, 7, 8]), None);
        assert_eq!(rle_byte(&[1]), None);
        let mut v = vec![0xAAu8; 1024];
        assert_eq!(rle_byte(&v), Some(0xAA));
        v[1000] = 0xAB;
        assert_eq!(rle_byte(&v), None);
    }

    #[test]
    fn empty_frame_has_checksum() {
        let zst = compress(b"", 3).unwrap();
        assert!(zst.len() >= 13);
        assert_eq!(decompress(&zst).unwrap(), b"");
    }

    #[test]
    fn roundtrip_all_strategies() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(32);
        let noise = xorshift(0x3333_4444, 4096);
        for id in 1i32..=9 {
            let mut params = crate::compression_params(3, Some(fox.len() as u64)).unwrap();
            params.apply_zstd_kv("strategy", id).unwrap();
            let zeros = [0u8; 2048];
            for src in [fox.as_slice(), noise.as_slice(), zeros.as_slice()] {
                let zst = compress_with_params(src, params, true).expect("compress");
                let got = decompress(&zst)
                    .unwrap_or_else(|e| panic!("strategy {id} src={}: {e:?}", src.len()));
                assert_eq!(got, src, "strategy {id}");
            }
        }
    }

    #[test]
    fn roundtrip_ultra_levels() {
        let src = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(48);
        for level in [20, 21, 22] {
            rt(&src, level);
            rt(&xorshift(0xABCDu64, 2048), level);
        }
    }

    #[test]
    fn rle_fse_large_match() {
        let chunk: Vec<u8> = (0..32_768).map(|i| (i % 251) as u8).collect();
        let mut src = chunk.clone();
        src.extend_from_slice(&chunk);
        let zst = compress(&src, 1).expect("compress");
        let got = decompress(&zst).unwrap_or_else(|e| panic!("zst={} err={e:?}", zst.len()));
        assert_eq!(got, src);
    }

    #[test]
    fn literals_and_sequence_modes_coverage() {
        let mut seen_lit = [false; 4];
        let mut seen_seq = [false; 4];
        let mut seen_4stream = false;
        let mut seen_huff_direct = false;
        let mut seen_huff_fse = false;

        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut text_block = Vec::new();
        while text_block.len() < 64 * 1024 {
            text_block.extend_from_slice(fox);
        }
        let mut text_two_blocks = text_block.clone();
        while text_two_blocks.len() < 130 * 1024 {
            text_two_blocks.extend_from_slice(fox);
        }

        let mut skip = crate::compression_params(1, Some(400)).unwrap();
        skip.target_length = 1 << 16;
        skip.min_match = 7;
        skip.strategy = crate::Strategy::Fast;

        let mut huff_src = Vec::new();
        while huff_src.len() < 400 {
            huff_src.extend_from_slice(fox);
        }
        huff_src.truncate(400);
        let mut huff_two = Vec::new();
        while huff_two.len() < 130 * 1024 {
            huff_two.extend_from_slice(fox);
        }
        // Bytes 0 and 1: Huffman alphabet is tiny, so the tree is direct 4-bit
        // (FSE weight compression needs more than two weights).
        let mut two_sym = Vec::new();
        while two_sym.len() < 400 {
            two_sym.extend_from_slice(&[0u8, 0, 0, 1]);
        }
        two_sym.truncate(400);

        let mut rle_lits = fox.repeat(20);
        rle_lits.truncate(1024);
        for _ in 0..30 {
            rle_lits.push(0xA5);
            rle_lits.push(0xA5);
            rle_lits.extend_from_slice(&fox[..20]);
        }
        let mut rle_win = crate::compression_params(1, Some(rle_lits.len() as u64)).unwrap();
        rle_win.window_log = 10;

        let mut frames: Vec<Vec<u8>> = Vec::new();
        let mut blocks_log: Vec<(u8, Option<u8>)> = Vec::new();
        let zst_rl = compress_with_params(&rle_lits, rle_win, true).expect("rle lits");
        assert_eq!(decompress(&zst_rl).unwrap(), rle_lits);
        frames.push(zst_rl);
        for (src, p) in [
            (huff_src.as_slice(), skip),
            (huff_two.as_slice(), skip),
            (two_sym.as_slice(), skip),
        ] {
            let zst = compress_with_params(src, p, true).expect("huff params");
            assert_eq!(decompress(&zst).unwrap(), src);
            frames.push(zst);
        }

        let mut mixed = Vec::new();
        let mut n = 0u32;
        while mixed.len() < 4096 {
            mixed.extend_from_slice(b"block ");
            mixed.push(b'0' + (n % 10) as u8);
            mixed.extend_from_slice(b" extra words for matches ");
            mixed.extend_from_slice(&(n.to_le_bytes()));
            mixed.push(b'\n');
            n += 1;
        }
        frames.push({
            let z = compress(&mixed, 1).expect("mixed L1");
            assert_eq!(decompress(&z).unwrap(), mixed);
            z
        });
        let mut mix_win = crate::compression_params(1, Some(8192)).unwrap();
        mix_win.window_log = 12;
        let mixed2 = mixed.repeat(2);
        frames.push({
            let z = compress_with_params(&mixed2, mix_win, true).expect("mixed2");
            assert_eq!(decompress(&z).unwrap(), mixed2);
            z
        });
        let mut fox_win = crate::compression_params(3, Some(3000)).unwrap();
        fox_win.window_log = 10;
        let fox_multi = fox.repeat(60);
        let zst_rep = compress_with_params(&fox_multi, fox_win, true).expect("repeat seq");
        assert_eq!(decompress(&zst_rep).unwrap(), fox_multi);
        frames.push(zst_rep);
        let mut small_win = crate::compression_params(1, Some(2048)).unwrap();
        small_win.window_log = 10;
        let repeated = b"TheQuickBrownFox0123456789ABCD".repeat(80);
        let zst_rle = compress_with_params(&repeated, small_win, true).expect("rle seq");
        assert_eq!(decompress(&zst_rle).unwrap(), repeated);
        frames.push(zst_rle);

        // RLE-literals coverage: EVERY literal in the block must be the same
        // byte while sequences still exist. The patterns are primed via a
        // PREFIX so the block itself contains only matches plus single 'q'
        // separators; a 1-byte separator cannot be repcode-matched (that needs
        // 4 bytes), so brick 40 leaves it as a literal. The older corpus
        // reached this state via a matcher weakness that repcode-1 removed.
        let mut rle_prefix = Vec::new();
        let mut rle_body = Vec::new();
        for i in 0..24u8 {
            let pat: Vec<u8> = (0..24u8).map(|j| b'A' + ((i * 7 + j * 3) % 26)).collect();
            rle_prefix.extend_from_slice(&pat);
            rle_body.push(b'q');
            rle_body.extend_from_slice(&pat);
        }
        let zst_rle_lits =
            compress_using_prefix(&rle_body, &rle_prefix, 1).expect("rle lits prefix");
        assert_eq!(
            crate::decode::decompress_using_prefix(&zst_rle_lits, &rle_prefix).unwrap(),
            rle_body
        );
        frames.push(zst_rle_lits);

        // Repeat FSE mode (seq mode 3) needs CONSECUTIVE blocks whose sequence
        // statistics are close enough that reusing the previous table beats
        // rebuilding. That needs >1 block (128 KiB each) of STATIONARY content.
        // The small samples below cannot reach it, and repcode-1 search
        // (brick 40) shifted the distributions that used to hit it by luck.
        let mut stationary = Vec::with_capacity(400 * 1024);
        {
            let mut st = 0x2545_F491_4F6C_DD1Du64;
            while stationary.len() < 400 * 1024 {
                st ^= st << 13;
                st ^= st >> 7;
                st ^= st << 17;
                // 16-symbol alphabet: compressible, but no long matches, so
                // every block sees the same LL/ML/OF shape.
                for k in 0..8 {
                    stationary.push(b'a' + ((st >> (k * 8)) & 0x0F) as u8);
                }
            }
        }
        let zst_stat = compress(&stationary, 1).expect("stationary");
        assert_eq!(decompress(&zst_stat).unwrap(), stationary);
        frames.push(zst_stat);

        // FSE Repeat mode (seq mode 3) needs a distribution that is STATIONARY
        // across blocks but NOT degenerate. `text_two_blocks` used to supply it,
        // but once repcode-1 search shipped (brick 67) that content became a
        // single constant-offset repeat, so its blocks select RLE (one symbol)
        // instead of Repeat -- a coverage fixture hostage to matcher quality,
        // the same trap the RLE-literals case hit earlier.
        //
        // Built here by construction instead: fox fragments of VARYING length
        // give varied litlen/matchlen codes, while the length distribution stays
        // identical block to block, so block N+1's cheapest table is block N's.
        let mut stationary = Vec::new();
        let mut rng = 0x1234_5678_9abc_def0u64;
        while stationary.len() < 400 * 1024 {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            let take = 12 + (rng >> 40) as usize % (fox.len() - 12);
            stationary.extend_from_slice(&fox[..take]);
        }

        let samples: Vec<(Vec<u8>, i32)> = vec![
            (fox[..20].to_vec(), 1),
            (b"TheQuickBrownFox0123456789ABCD".repeat(2), 1),
            (fox.repeat(4), 1),
            (fox.repeat(8), 3),
            (text_block.clone(), 3),
            (text_two_blocks, 3),
            (stationary, 3),
            (xorshift(0xF00Du64, 4096), 1),
            (vec![0u8; 8192], 1),
            (vec![b'a'; 1024], 5),
            (xorshift(0xF00Du64, 8192), 9),
        ];
        for (src, level) in samples {
            let zst = compress(&src, level)
                .unwrap_or_else(|e| panic!("compress L{level} src={}: {e:?}", src.len()));
            let got = decompress(&zst).unwrap_or_else(|e| {
                panic!(
                    "decompress L{level} src={} zst={}: {e:?}",
                    src.len(),
                    zst.len()
                )
            });
            assert_eq!(got, src, "L{level} src={}", src.len());
            frames.push(zst);
        }
        for zst in &frames {
            for b in inspect_compressed_blocks(zst) {
                seen_lit[b.lit as usize] = true;
                if let Some(m) = b.seq {
                    seen_seq[((m >> 6) & 3) as usize] = true;
                    seen_seq[((m >> 4) & 3) as usize] = true;
                    seen_seq[((m >> 2) & 3) as usize] = true;
                }
                if b.four_stream {
                    seen_4stream = true;
                }
                match b.huff_tree {
                    Some(true) => seen_huff_fse = true,
                    Some(false) => seen_huff_direct = true,
                    None => {}
                }
                blocks_log.push((b.lit, b.seq));
            }
        }

        // RLE literals (type 1) are gated directly by
        // `huffman::tests::rle_literals_section_emits_type_1_and_round_trips`.
        // Requiring them to FALL OUT of the match finder here made this test a
        // hostage to matcher quality: repcode-1 search (brick 40) legitimately
        // consumes the single-byte runs that used to survive as literals, so
        // the mode became unreachable from this corpus while the emit path
        // itself is unchanged.
        assert!(
            seen_lit[0],
            "missing Raw literals (type 0); lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_lit[2],
            "missing Huffman Compressed literals (type 2); lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_lit[3],
            "missing Treeless Huffman literals (type 3); lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_seq[0],
            "missing Predefined FSE mode; lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_seq[1],
            "missing RLE FSE mode; lit={seen_lit:?} seq={seen_seq:?} blocks={blocks_log:?}"
        );
        assert!(
            seen_seq[2],
            "missing Compressed FSE mode; lit={seen_lit:?} seq={seen_seq:?} blocks={blocks_log:?}"
        );
        assert!(
            seen_seq[3],
            "missing Repeat FSE mode; lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_4stream,
            "missing 4-stream Huffman; lit={seen_lit:?} seq={seen_seq:?}"
        );
        assert!(
            seen_huff_direct,
            "missing direct Huffman tree (header>=128); lit={seen_lit:?}"
        );
        assert!(
            seen_huff_fse,
            "missing FSE Huffman tree (header<128); lit={seen_lit:?}"
        );
    }

    struct InspectedBlock {
        lit: u8,
        seq: Option<u8>,
        four_stream: bool,
        /// `Some(true)` = FSE-compressed weights; `Some(false)` = direct 4-bit.
        huff_tree: Option<bool>,
    }

    fn inspect_compressed_blocks(zst: &[u8]) -> Vec<InspectedBlock> {
        use crate::block::{parse_block_header, BlockType};
        use crate::frame::parse_kind;
        use crate::reader::Reader;
        let mut r = Reader::new(zst);
        parse_kind(&mut r).expect("frame header");
        let mut out = Vec::new();
        loop {
            let bh = parse_block_header(&mut r).expect("block header");
            let payload = r.take(bh.payload_len() as usize).expect("payload");
            if bh.ty == BlockType::Compressed {
                let lit = payload[0] & 3;
                let size_fmt = (payload[0] >> 2) & 3;
                let four_stream = matches!(lit, 2 | 3) && size_fmt != 0;
                let huff_tree = if lit == 2 {
                    let hlen = match size_fmt {
                        0 | 1 => 3usize,
                        2 => 4,
                        3 => 5,
                        _ => 0,
                    };
                    payload.get(hlen).map(|&b| b < 128)
                } else {
                    None
                };
                let after = skip_literals_section(payload).expect("literals");
                let (nseq, rest) = read_nseq(after);
                let mode = if nseq == 0 {
                    None
                } else {
                    rest.first().copied()
                };
                out.push(InspectedBlock {
                    lit,
                    seq: mode,
                    four_stream,
                    huff_tree,
                });
            }
            if bh.last {
                break;
            }
        }
        out
    }

    fn skip_literals_section(payload: &[u8]) -> Option<&[u8]> {
        let first = *payload.first()?;
        let lit_type = first & 3;
        let size_fmt = (first >> 2) & 3;
        match lit_type {
            0 | 1 => {
                let (regen, hdr) = match size_fmt {
                    0 | 2 => (u32::from(first >> 3), 1usize),
                    1 => {
                        let b1 = *payload.get(1)?;
                        (u32::from(first >> 4) + (u32::from(b1) << 4), 2)
                    }
                    3 => {
                        let b1 = *payload.get(1)?;
                        let b2 = *payload.get(2)?;
                        (
                            u32::from(first >> 4) + (u32::from(b1) << 4) + (u32::from(b2) << 12),
                            3,
                        )
                    }
                    _ => return None,
                };
                let body = if lit_type == 1 {
                    1usize
                } else {
                    regen as usize
                };
                payload.get(hdr + body..)
            }
            2 | 3 => {
                let (csize, hdr) = match size_fmt {
                    0 | 1 => {
                        let b1 = *payload.get(1)?;
                        let b2 = *payload.get(2)?;
                        let csize = ((u32::from(b1) >> 6) + (u32::from(b2) << 2)) & 0x3FF;
                        (csize as usize, 3usize)
                    }
                    2 => {
                        let b2 = *payload.get(2)?;
                        let b3 = *payload.get(3)?;
                        let csize = (u32::from(b2) >> 2) + (u32::from(b3) << 6);
                        ((csize as usize) & 0x3FFF, 4)
                    }
                    3 => {
                        let b2 = *payload.get(2)?;
                        let b3 = *payload.get(3)?;
                        let b4 = *payload.get(4)?;
                        let csize =
                            (u32::from(b2) >> 6) + (u32::from(b3) << 2) + (u32::from(b4) << 10);
                        ((csize as usize) & 0x3FFFF, 5)
                    }
                    _ => return None,
                };
                payload.get(hdr + csize..)
            }
            _ => None,
        }
    }

    fn read_nseq(src: &[u8]) -> (u32, &[u8]) {
        let Some(&b0) = src.first() else {
            return (0, src);
        };
        if b0 == 0 {
            (0, &src[1..])
        } else if b0 < 128 {
            (u32::from(b0), &src[1..])
        } else if b0 < 255 {
            let b1 = src.get(1).copied().unwrap_or(0);
            (
                ((u32::from(b0) - 128) << 8) + u32::from(b1),
                src.get(2..).unwrap_or(&[]),
            )
        } else {
            let b1 = src.get(1).copied().unwrap_or(0);
            let b2 = src.get(2).copied().unwrap_or(0);
            (
                0x7F00 + u32::from(b1) + (u32::from(b2) << 8),
                src.get(3..).unwrap_or(&[]),
            )
        }
    }

    fn xorshift(seed: u64, n: usize) -> Vec<u8> {
        let mut s = seed;
        let mut v = vec![0u8; n];
        for b in &mut v {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            *b = (s & 0xFF) as u8;
            if *b == 0 {
                *b = 1;
            }
        }
        v
    }

    fn count_zstd_blocks(zst: &[u8]) -> usize {
        let mut r = crate::reader::Reader::new(zst);
        crate::frame::parse_kind(&mut r).expect("frame header");
        let mut n = 0usize;
        loop {
            let h = crate::block::parse_block_header(&mut r).expect("block header");
            n += 1;
            r.take(h.payload_len() as usize).expect("payload");
            if h.last {
                break;
            }
        }
        n
    }

    #[test]
    fn long_forces_window_descriptor() {
        let src = xorshift(0x5E1A_B1E5, 300 * 1024);
        let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
        params.window_log = 18;
        let zst = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                ldm: crate::ldm::LdmParams::enabled(),
                ..AdvancedOptions::default()
            },
        )
        .expect("long compress");
        match crate::get_frame_header(&zst).expect("hdr") {
            crate::FrameKind::Zstd(h) => {
                assert!(
                    !h.single_segment,
                    "300 KiB > 256 KiB window must emit Window_Descriptor"
                );
                assert_eq!(h.window_size, 1u64 << 18);
            }
            other => panic!("expected zstd frame, got {other:?}"),
        }
        assert_eq!(decompress(&zst).expect("decode"), src);
    }

    #[test]
    fn enable_ldm_zstd_keys_roundtrip() {
        let src = xorshift(0x1D1D, 32 * 1024);
        let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
        params
            .apply_zstd_option_string("enableLdm=1,ldmHashLog=12,ldmMinMatch=64,ldmHashRateLog=7")
            .unwrap();
        let ldm = params.ldm_params();
        assert!(ldm.enable);
        assert_eq!(ldm.hash_log, 12);
        assert_eq!(ldm.min_match, 64);
        let zst = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                ldm,
                ..AdvancedOptions::default()
            },
        )
        .expect("enableLdm compress");
        assert_eq!(decompress(&zst).expect("decode"), src);
    }

    #[test]
    fn rsyncable_splits_blocks() {
        let src = xorshift(0xA11, 64 * 1024);
        let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
        params.window_log = 18;
        let plain = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions::default(),
        )
        .unwrap();
        let rsync = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                ldm: crate::ldm::LdmParams::enabled(),
                rsyncable: true,
                target_cblock_size: 0,
                ..AdvancedOptions::default()
            },
        )
        .unwrap();
        let n_plain = count_zstd_blocks(&plain);
        let n_rsync = count_zstd_blocks(&rsync);
        assert_eq!(decompress(&rsync).unwrap(), src);
        assert!(
            n_rsync > n_plain,
            "rsyncable should cut extra blocks (plain={n_plain} rsync={n_rsync})"
        );
    }

    #[test]
    fn target_cblock_caps_uncompressed_blocks() {
        let src = xorshift(0xC0B1, 32 * 1024);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let plain = compress_with_params(&src, params, true).unwrap();
        let capped = compress_with_advanced(
            &src,
            params,
            true,
            None,
            &[],
            true,
            AdvancedOptions {
                target_cblock_size: 256,
                ..AdvancedOptions::default()
            },
        )
        .unwrap();
        let n_plain = count_zstd_blocks(&plain);
        let n_capped = count_zstd_blocks(&capped);
        assert_eq!(decompress(&capped).unwrap(), src);
        assert!(
            n_capped > n_plain,
            "target cblock 256 => ~1 KiB raw blocks (plain={n_plain} capped={n_capped})"
        );
    }

    #[test]
    fn decompress_long_raises_window_cap() {
        let src = xorshift(0x5716, 128 * 1024 + 64);
        let mut params = compression_params(1, Some(src.len() as u64)).unwrap();
        params.window_log = 16;
        let zst = compress_with_params(&src, params, true).unwrap();
        match crate::get_frame_header(&zst).unwrap() {
            crate::FrameKind::Zstd(h) => {
                assert!(!h.single_segment);
                assert_eq!(h.window_size, 1u64 << 16);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(
            crate::decompress_with(
                &zst,
                crate::DecompressOptions {
                    window_max: 32 * 1024,
                    ..Default::default()
                }
            )
            .unwrap_err(),
            crate::Error::WindowTooLarge
        );
        assert_eq!(
            crate::decompress_with(
                &zst,
                crate::DecompressOptions {
                    window_max: 1u64 << 16,
                    ..Default::default()
                }
            )
            .unwrap(),
            src
        );
    }

    #[test]
    fn fast_sparse_match_fill_roundtrips_repeating_text() {
        let src = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(8000);
        for level in [1, -1, -4] {
            rt(&src, level);
            let zst = compress(&src, level).unwrap();
            assert!(
                zst.len() < src.len() / 20,
                "L{level}: repeating text should stay compact ({} vs {})",
                zst.len(),
                src.len()
            );
        }
    }

    #[test]
    fn count_match_words_match_byte_loop() {
        fn bytes(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
            let max = (limit - ip).min(src.len() - m).min(src.len() - ip);
            let mut n = 0usize;
            while n < max && src[m + n] == src[ip + n] {
                n += 1;
            }
            n
        }
        let mut src = vec![0u8; 4096];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let head: Vec<u8> = src[0..200].to_vec();
        src[200..400].copy_from_slice(&head);
        let mid: Vec<u8> = src[3..20].to_vec();
        src[800..800 + 17].copy_from_slice(&mid);
        for m in [0usize, 1, 3, 7, 8, 15, 200] {
            for ip in [200usize, 201, 400, 800, 801, 2000] {
                if m >= src.len() || ip >= src.len() {
                    continue;
                }
                for limit in [ip, ip + 1, ip + 7, ip + 8, ip + 9, ip + 64, src.len()] {
                    let limit = limit.min(src.len());
                    if ip > limit {
                        continue;
                    }
                    assert_eq!(
                        count_match(&src, m, ip, limit),
                        bytes(&src, m, ip, limit),
                        "m={m} ip={ip} limit={limit}"
                    );
                }
            }
        }
    }

    #[test]
    fn min_gain_matches_c_fast() {
        assert_eq!(min_gain(128 * 1024, Strategy::Fast), (128 * 1024 >> 6) + 2);
        assert_eq!(min_gain(100, Strategy::Greedy), (100 >> 6) + 2);
        assert_eq!(min_gain(100, Strategy::BtUltra), (100 >> 7) + 2);
    }

    #[test]
    fn early_raw_skip_fast_rung_low_matches() {
        SKIP_OVERRIDE.with(|c| c.set(None));
        let fast = compression_params(-1, Some(128 * 1024)).unwrap();
        assert!(fast.target_length >= 1);
        assert!(fast.target_length <= 7);
        let mg = min_gain(128 * 1024, fast.strategy);
        assert!(early_raw_skip(mg.saturating_sub(1), 128 * 1024, fast));
        assert!(!early_raw_skip(mg + 10, 128 * 1024, fast));
        let l1 = compression_params(1, Some(128 * 1024)).unwrap();
        assert_eq!(l1.target_length, 0);
        assert!(!early_raw_skip(0, 128 * 1024, l1));
        let l3 = compression_params(3, Some(128 * 1024)).unwrap();
        assert!(l3.strategy != Strategy::Fast);
        assert!(!early_raw_skip(0, 128 * 1024, l3));
    }

    #[test]
    fn skip_off_l1_bytes_match_unset() {
        let src = xorshift(0xBEEF, 32 * 1024);
        SKIP_OVERRIDE.with(|c| c.set(Some(false)));
        let off = compress(&src, 1).expect("off");
        SKIP_OVERRIDE.with(|c| c.set(None));
        let unset = compress(&src, 1).expect("unset");
        assert_eq!(off, unset, "knob-off at -1 must match default (tlen=0)");
        assert_eq!(decompress(&off).unwrap(), src);
    }

    #[test]
    fn skip_off_fast_roundtrip_and_on_skips_noise() {
        let src = xorshift(0xA11E, 64 * 1024);
        SKIP_OVERRIDE.with(|c| c.set(Some(false)));
        let off = compress(&src, -1).expect("off");
        SKIP_OVERRIDE.with(|c| c.set(Some(true)));
        let on = compress(&src, -1).expect("on");
        SKIP_OVERRIDE.with(|c| c.set(None));
        assert_eq!(decompress(&off).unwrap(), src);
        assert_eq!(decompress(&on).unwrap(), src);
        let l1 = compress(&src, 1).expect("l1");
        assert_eq!(decompress(&l1).unwrap(), src);
        let off_c = frame_block_census(&off).unwrap();
        let on_c = frame_block_census(&on).unwrap();
        assert!(
            on_c.raw >= off_c.raw,
            "skip-on should dump at least as many raw blocks"
        );
    }
}

/// Clear every cached env-var arm so a later `std::env::set_var` is observed.
///
/// Each arm caches its env read in an atomic on first use -- bricks 49/64/77
/// removed those reads from hot loops. That makes an IN-PROCESS A/B that flips
/// an env var read stale: the second arm silently re-measures the first. Only
/// needed by probes that set env vars mid-process; the shipped paths never do.
pub fn reset_env_arms() {
    use core::sync::atomic::Ordering;
    STEP0_ARM.store(0, Ordering::Relaxed);
    PIPE_ARM.store(0, Ordering::Relaxed);
    LAZY_FILL_ENABLED_ARM.store(0, Ordering::Relaxed);
    FAST_LAZY_ARM.store(0, Ordering::Relaxed);
    PAIR_GAIN_ARM.store(u32::MAX, Ordering::Relaxed);
    PAIR_HI_ARM.store(u32::MAX, Ordering::Relaxed);
}

/// Arm for the `find_dfast` HLOG specialisation, so it can be A/B'd IN-PROCESS
/// rather than across two binaries (a cross-binary compare buries the kernel
/// delta under process-start cost). `RZSTD_DFAST_SPEC=0` selects the old
/// runtime-shift path.
static DFAST_SPEC_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_dfast_spec_arm(on: bool) {
    DFAST_SPEC_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn dfast_spec_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match DFAST_SPEC_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_DFAST_SPEC")
                .map(|v| v.trim() != "0")
                .unwrap_or(true);
            DFAST_SPEC_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// GATE 4 arm: the `find_fast` HLOG/STEP specialisation itself.
///
/// With this OFF the shipping configuration falls through to the generic
/// `go!(false, false, 0, 0, true)` arm -- runtime HLOG, runtime STEP -- which is
/// the "constant" alternative to the 13-way dispatch. Default ON, so an A/B
/// setting it to 0 differs from the default and is not a null comparison.
static FAST_SPEC_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_fast_spec_arm(on: bool) {
    FAST_SPEC_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn fast_spec_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match FAST_SPEC_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_FAST_SPEC")
                .map(|v| v.trim() != "0")
                .unwrap_or(true);
            FAST_SPEC_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Which `find_dfast` body actually executed. Probe counts and output bytes are
/// IDENTICAL between the two, by design -- they examine the same candidates in
/// the same order -- so neither can show which one ran. These can.
pub static DFAST_SPEC_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static DFAST_RUNTIME_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear both call counters.
pub fn take_dfast_calls() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        DFAST_SPEC_CALLS.swap(0, Ordering::Relaxed),
        DFAST_RUNTIME_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// Calls into `find_fast`'s Gate-4 dispatcher, for reachability proofs.
pub static FAST_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Calls into `find_opt` (L16+).
pub static OPT_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the finder reachability counters: `(find_fast, find_opt)`.
pub fn take_finder_calls() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        FAST_CALLS.swap(0, Ordering::Relaxed),
        OPT_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// Which `bt_find_best` body ran: `(specialised, runtime_fallback)`.
pub static BT_SPEC_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static BT_RUNTIME_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the bt-path body counters.
pub fn take_bt_calls() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        BT_SPEC_CALLS.swap(0, Ordering::Relaxed),
        BT_RUNTIME_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// GATE 5 arm for the BT path (L13-L22).
///
/// **DEFAULT OFF — the specialisation MEASURED WORSE and was reverted here.**
///
/// It was shipped in 727e503 on deterministic instruction counts alone: 214
/// instructions per call against the runtime arm's 237, and 3 of 4 variable
/// shifts eliminated. Those numbers are correct and they were the wrong
/// measure. Twelve monomorphizations are 2,580 instructions of code where there
/// was 238, and at L19 the binary-tree walk is the hot loop, so I-cache
/// pressure decides rather than per-call instruction count.
///
/// Tested properly -- three independent ABBA runs per corpus, 18 corpora, a
/// stable sign in all three runs required to count:
///
/// ```text
/// L19   stable-generic 5   stable-spec 0   (nci +3.9..+5.6%, x-ray +5.1..+10.8%)
/// L13   stable-generic 2   stable-spec 3   -- a wash, not a case for a dispatch
/// ```
///
/// Loses on five corpora at L19 and wins on none, so CONSTANT OFF. Same
/// precedent as `tag_enabled`: the code stays so the arm can be re-tested, the
/// default ships the arm that measured better.
///
/// The `find_dfast` specialisation is NOT affected -- tested the same way it
/// came out 6 stable-spec / 0 stable-generic and remains on.
static BT_SPEC_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_bt_spec_arm(on: bool) {
    BT_SPEC_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn bt_spec_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match BT_SPEC_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_BT_SPEC")
                .map(|v| v.trim() != "0")
                .unwrap_or(true);
            BT_SPEC_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// GATE 6 @ L3 arm: C's `_search_next_long` ip+1 long-hash probe in DFast.
/// Default OFF until measured, so enabling it differs from the default.
static NEXT_LONG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_next_long_arm(on: bool) {
    NEXT_LONG_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn next_long_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match NEXT_LONG_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_NEXT_LONG").map(|v| v.trim() != "0").unwrap_or(true);
            NEXT_LONG_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// GATE 6 @ L3 dispatch threshold: minimum share of next-long probes that must
/// have WON on the previous block for the probe to run on this one.
/// `RZSTD_NEXT_LONG_T` sweeps it.
fn next_long_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[11].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = NEXT_LONG_MIN_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_NEXT_LONG_T")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.10);
        NEXT_LONG_MIN_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.10
}
#[cfg(feature = "std")]
static NEXT_LONG_MIN_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// GATE 6 @ L1: pair-search dispatch. Default ON; `RZSTD_PAIR=0` disables.
static PAIR_ON_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_pair_on_arm(on: bool) {
    PAIR_ON_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn pair_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match PAIR_ON_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_PAIR").map(|v| v.trim() != "0").unwrap_or(true);
            PAIR_ON_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Above this previous-block repcode yield the pair search is switched OFF: the
/// repcode path already holds those matches, so pairing spends probes to reach a
/// worse parse. `RZSTD_PAIR_T` sweeps it.
/// 4.72: below this `pair_gain` the PAIR route is net CHEAPER in total search ops.
///
/// Counter-intuitive until you read the probe COUNT rather than the rate.
/// `pair_gain` is bytes-per-probe, and across the corpus it runs INVERSELY to
/// how often the pair search fires. Forcing route 1 -> 2:
///
/// ```text
///   corpus     pair_gain   d positions      d pair     NET ops
///   x-ray         0.3674        -15608       11826       -3782   cheaper
///   sao           0.4404      -1592127      162900    -1429227   cheaper
///   mozilla       0.6835       -663818      394230     -269588   cheaper
///   ---------------------------------------------------- 0.71 --
///   ooffice       0.7406      -1491034     1713353     +222319   costs
///   incomp-32m    0.8056          -889        3740       +2851   costs
///   dickens       0.8735      -2193266     2193539        +273   costs
///   mr            0.9012      -1723724     2132991     +409267   costs
///   samba         1.5846       -447848      513447      +65599   costs
/// ```
///
/// At low gain the pair search barely fires, so the step-2 position saving is
/// nearly free; at high gain it fires millions of times and the saving is more
/// than repaid in probes. **8/8 on the work sign**, including the two corpora
/// that were not in the set that suggested the threshold.
///
/// This makes the gate non-monotonic in `pair_gain` (2 below 0.20 is route 0,
/// then 2, then 1, then 2 above 1.00) -- correct, because the two route-2
/// branches are selected for DIFFERENT reasons: this one for cheapness, the
/// `pair_rate_hi` one for the bytes the search returns.
#[inline(always)]
fn pair_gain_lo() -> f32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = PAIR_LO_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_PAIR_LO")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.71);
        PAIR_LO_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.71
}

static PAIR_LO_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Pin `pair_gain_lo`. `f32::NAN` restores the env/default path.
pub fn set_pair_lo_arm(v: f32) {
    use core::sync::atomic::Ordering;
    PAIR_LO_ARM.store(if v.is_nan() { u32::MAX } else { v.to_bits() }, Ordering::Relaxed);
}

fn pair_rep_max() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[12].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    // ffanat: cached (the tag_min pattern). This is read per BLOCK on the
    // find_fast path -- an uncached `std::env::var` is 115.6 ns and a String
    // allocation per read, for a process constant.
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = PAIR_T_CACHE.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_PAIR_T")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.7);
        PAIR_T_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.7
}

static PAIR_T_CACHE: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// PROMETHEUS PREREQ: how often is each fitted constant actually READ?
/// Each of these accessors calls `std::env::var` with no cache -- a
/// GetEnvironmentVariableW plus a String allocation for a process constant.
#[cfg(feature = "profile")]
pub static ENVHIT: [core::sync::atomic::AtomicU64; 14] = [
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0), core::sync::atomic::AtomicU64::new(0),
];

/// Read and clear the fitted-constant read counts.
#[cfg(feature = "profile")]
pub fn take_envhits() -> [u64; 14] {
    let mut o = [0u64; 14];
    for i in 0..14 { o[i] = ENVHIT[i].swap(0, core::sync::atomic::Ordering::Relaxed); }
    o
}

/// Diagnostic counters for Gate 6 candidate variables: how often the pair probe
/// fires, how often it HITS, and how many bytes those hits cover. Activity vs
/// outcome -- the campaign's law says the signal must predict the outcome.
pub static PAIR_PROBES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_HITS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_BYTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Split the pair probe by the MAIN probe's slot state -- `m0 == 0` means that
/// hash bucket has never been written, which is free information already in a
/// register at the probe site.
pub static PAIR_M0_EMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_M0_LIVE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_HIT_EMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_HIT_LIVE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_BYTES_EMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static PAIR_BYTES_LIVE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// `(probes_empty, probes_live, hits_empty, hits_live, bytes_empty, bytes_live)`
pub fn take_pair_split() -> (u64, u64, u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        PAIR_M0_EMPTY.swap(0, Relaxed),
        PAIR_M0_LIVE.swap(0, Relaxed),
        PAIR_HIT_EMPTY.swap(0, Relaxed),
        PAIR_HIT_LIVE.swap(0, Relaxed),
        PAIR_BYTES_EMPTY.swap(0, Relaxed),
        PAIR_BYTES_LIVE.swap(0, Relaxed),
    )
}

pub static MAIN_BYTES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear: `(probes, hits, pair_match_bytes, all_match_bytes)`.
static ROUTE_HIST: [core::sync::atomic::AtomicU64; 3] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];
static ROUTE_GAIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static ROUTE_REP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static ROUTE_N: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

static SIG_GAIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_REP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_N: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_TAG: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_REPLEN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_NSEQ: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
static SIG_OPTREP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// EVERY per-block content signal the encoder already maintains, as block means:
/// `(pair_gain, rep_yield, tag_yield, rep_len_ratio, last_nseq, opt_rep_rate)`.
///
/// The campaign's 4.72 law in tool form: before inventing a dispatch signal,
/// dump the ones already in `MatchTables`. Four invented signals were refuted in
/// 4.70 while the working one (`pair_gain`) sat in the struct the whole time.
///
/// SCOPE: `pair_gain` is maintained ONLY in `find_fast_impl` (L1/L2).
/// `rep_yield` is maintained in all five finders (L1-L15). Check a signal EXISTS
/// at the level you are dispatching before reading meaning into its value.
pub fn take_content_signals() -> (f64, f64, f64, f64, f64, f64) {
    use core::sync::atomic::Ordering;
    let n = SIG_N.swap(0, Ordering::Relaxed).max(1) as f64;
    let g = SIG_GAIN.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n;
    let y = SIG_REP.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n;
    let t = SIG_TAG.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n;
    let r = SIG_REPLEN.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n;
    let q = SIG_NSEQ.swap(0, Ordering::Relaxed) as f64 / n;
    let o = SIG_OPTREP.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n;
    (g, y, t, r, q, o)
}

/// Per-block route histogram and the mean state that decided it.
/// Returns `(route0, route1, route2, mean pair_gain, mean rep_yield)`.
pub fn take_route_hist() -> (u64, u64, u64, f64, f64) {
    use core::sync::atomic::Ordering;
    let n = ROUTE_N.swap(0, Ordering::Relaxed).max(1);
    (
        ROUTE_HIST[0].swap(0, Ordering::Relaxed),
        ROUTE_HIST[1].swap(0, Ordering::Relaxed),
        ROUTE_HIST[2].swap(0, Ordering::Relaxed),
        ROUTE_GAIN.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n as f64,
        ROUTE_REP.swap(0, Ordering::Relaxed) as f64 / 1000.0 / n as f64,
    )
}

pub fn take_pair_stats() -> (u64, u64, u64, u64) {
    use core::sync::atomic::Ordering;
    (
        PAIR_PROBES.swap(0, Ordering::Relaxed),
        PAIR_HITS.swap(0, Ordering::Relaxed),
        PAIR_BYTES.swap(0, Ordering::Relaxed),
        MAIN_BYTES.swap(0, Ordering::Relaxed),
    )
}

/// GATE 7 arm. Default OFF until measured; `RZSTD_TAG=1` enables.
static TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA.
pub fn set_tag_arm(on: bool) {
    TAG_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn tag_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match TAG_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_TAG").map(|v| v.trim() != "0").unwrap_or(true);
            TAG_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// GATE 7 dispatch threshold: minimum share of the previous block's candidates
/// the tag must have rejected for the filter to run. `RZSTD_TAG_T` sweeps.
/// PROMETHEUS ADJUDICATION: this was MIS-FITTED at 0.50, and cached besides.
///
/// The tag is a PURE FILTER -- it cannot hide a match, and 0 false rejects were
/// measured across the whole board -- so its only axis is WORK. Swept on that
/// axis at L1 (candidate loads avoided out of 8,248,621 probes):
///
///   tag_min 0.00 -> 4,538,058 avoided (55.0%)   <- best
///           0.25 -> 2,055,500 (24.9%)
///           0.50 -> 1,859,598 (22.5%)           <- was shipped
///           0.90 ->   356,859 (4.3%)
///           1.00 ->    29,487 (0.4%)
///
/// Lowering it to 0 more than DOUBLES the loads the filter avoids, for no size
/// change at all. The threshold was forfeiting benefit for nothing, because
/// `store_fast` writes the tag UNCONDITIONALLY whenever the array exists -- only
/// the COMPARE was gated. So a high `tag_min` pays the store and then declines
/// to use it. That asymmetry is the same one 190ad8b documents from the other
/// direction.
///
/// Also cached: this was one of 19 accessors calling `std::env::var` per read --
/// 115.6 ns each, ~1,875 reads per 32 MiB pass -- for a process constant.
static TAG_MIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

fn tag_min() -> f32 {
    #[cfg(feature = "profile")]
    ENVHIT[13].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = TAG_MIN_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_TAG_T")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.0);
        TAG_MIN_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.0
}

/// Candidates a tag could reject without loading `src[m]`, and those it cannot.

/// Consume the counters and return the reject share.
/// Share of candidates rejected by the 4-byte compare -- Gate 7's dispatch input.
#[inline]
fn cand_yield((f, t): (u64, u64)) -> f32 {
    if f + t == 0 {
        1.0
    } else {
        f as f32 / (f + t) as f32
    }
}

/// L19-native accounting: tree probes, those too SHORT to use, and those that
/// could not IMPROVE on the best so far.
pub static BT_PROBE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static BT_SHORT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
pub static BT_NOGAIN: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// `(probes, too_short, no_gain)`
pub fn take_bt_probe_stats() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering;
    (
        BT_PROBE.swap(0, Ordering::Relaxed),
        BT_SHORT.swap(0, Ordering::Relaxed),
        BT_NOGAIN.swap(0, Ordering::Relaxed),
    )
}

/// GATE 6 second threshold: minimum share of the previous block covered by pair
/// matches for the search to run. `RZSTD_PAIR_G` sweeps; 0 disables the term.
/// Blocks between forced pair re-probes when the gain term has the gate shut.
const PAIR_PROBE_PERIOD: u32 = 16;

/// Above this exchange rate the pair path is worth its lost pipelining.
fn pair_rate_hi() -> f32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        let c = PAIR_HI_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_PAIR_HI")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(1.0);
        PAIR_HI_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1.0
}

static PAIR_HI_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the pair-vs-step1 crossover in-process.
pub fn set_pair_hi_arm(v: f32) {
    PAIR_HI_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

fn pair_gain_min() -> f32 {
    #[cfg(feature = "std")]
    {
        use core::sync::atomic::Ordering;
        // Cached as raw bits: this is read once per BLOCK on the shipped path,
        // and an `env::var` there allocates a String per block for a constant.
        let c = PAIR_GAIN_ARM.load(Ordering::Relaxed);
        if c != u32::MAX {
            return f32::from_bits(c);
        }
        let v: f32 = std::env::var("RZSTD_PAIR_G")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.20);
        PAIR_GAIN_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.20
}

static TAG_ALLOC_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// A/B whether the Fast tag array is ALLOCATED at all (and therefore whether
/// the per-probe tag store happens). Distinct from `set_tag_arm`, which only
/// controls whether the filter READS it.
pub fn set_tag_alloc_arm(on: bool) {
    TAG_ALLOC_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn tag_alloc_enabled() -> bool {
    match TAG_ALLOC_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        _ => true,
    }
}

static PAIR_GAIN_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

/// Set the Gate 6 earning threshold in-process (A/B without a rebuild).
pub fn set_pair_gain_arm(v: f32) {
    PAIR_GAIN_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

/// GATE 6 deep arm: how `find_opt`'s parse-backtrace buffer is sized.
/// 0/2 = exact (pre-walk the chain), 1 = neither, 3 = blanket `n + 1`.
static OPT_OPS_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook. 0 = reuse only, 1 = exact pre-walk, 2 = blanket n+1.
pub fn set_opt_ops_arm(v: u8) {
    OPT_OPS_ARM.store(v + 1, core::sync::atomic::Ordering::Relaxed);
}

fn opt_ops_exact() -> bool {
    matches!(OPT_OPS_ARM.load(core::sync::atomic::Ordering::Relaxed), 0 | 2)
}

fn opt_ops_blanket() -> bool {
    OPT_OPS_ARM.load(core::sync::atomic::Ordering::Relaxed) == 3
}



/// GATE 6 @ L1 arm: keep the finder's sequence/literal buffers on the frame
/// instead of building them fresh per block. Default ON.
static FINDER_SCRATCH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for GATE 6 @ L1.
pub fn set_finder_scratch_arm(on: bool) {
    FINDER_SCRATCH_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn finder_scratch_enabled() -> bool {
    !matches!(FINDER_SCRATCH_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}


/// T1 arm: give DFast the packed rejection tag that the Fast ladder already
/// uses. DEFAULT ON -- byte-identical on 18/18 at L3 and 72/72 across the board,
/// and it strictly removes work: 2,938,472 candidate loads avoided per board
/// pass (29.8% of non-empty short slots) for no added load, store, or byte of
/// memory, because the tag rides in the word the finder already touches.
static DFAST_TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for T1.
pub fn set_dfast_tag_arm(on: bool) {
    DFAST_TAG_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn dfast_tag_enabled() -> bool {
    !matches!(DFAST_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// 1a arm: the LONG-table rejection tag (packed frames only). DEFAULT ON.
static LONG_TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for 1a.
pub fn set_long_tag_arm(on: bool) {
    LONG_TAG_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn long_tag_enabled() -> bool {
    !matches!(LONG_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// 1a ledger: (nonempty long probes, rejections, FALSE rejections). Three
/// counters with one meaning each -- see the tag audit's instrument trap.
#[cfg(feature = "profile")]
pub static LTAG_NONEMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_REJECT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_FALSE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the 1a ledger.
#[cfg(feature = "profile")]
pub fn take_long_tag() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        LTAG_NONEMPTY.swap(0, Relaxed),
        LTAG_REJECT.swap(0, Relaxed),
        LTAG_FALSE.swap(0, Relaxed),
    )
}

/// 1a residual: survivors of the 4-byte tag that (failed, passed) acceptance
/// at the MAIN long consume site. The fail share is the ceiling on what a
/// stronger tag could still remove.
#[cfg(feature = "profile")]
pub static LTAG_SURV_FAIL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_SURV_WFAIL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_SURV_ACC: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// SHORT-table consume-site residual, mirror of the long table's.
#[cfg(feature = "profile")]
pub static STAG_SURV_FAIL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STAG_SURV_WFAIL: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STAG_SURV_ACC: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// `(bytes_fail, window_fail, accepted)` for the SHORT consume site.
#[cfg(feature = "profile")]
pub fn take_short_tag_residual() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        STAG_SURV_FAIL.swap(0, Relaxed),
        STAG_SURV_WFAIL.swap(0, Relaxed),
        STAG_SURV_ACC.swap(0, Relaxed),
    )
}

/// `(bytes_fail, window_fail, accepted)` -- only `bytes_fail` paid a load.
#[cfg(feature = "profile")]
pub fn take_long_tag_residual() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        LTAG_SURV_FAIL.swap(0, Relaxed),
        LTAG_SURV_WFAIL.swap(0, Relaxed),
        LTAG_SURV_ACC.swap(0, Relaxed),
    )
}


/// ffanat 5a receipt counters: which representation served each tag compare.
/// `TAGARR_READS` is a load from a SECOND random cache line; `PACKED_TAG_READS`
/// reads the byte that arrived with the position.
#[cfg(feature = "profile")]
pub static TAGARR_READS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static PACKED_TAG_READS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(tag-array reads, packed reads)`.
#[cfg(feature = "profile")]
pub fn take_tag_reads() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (TAGARR_READS.swap(0, Relaxed), PACKED_TAG_READS.swap(0, Relaxed))
}

/// ffanat 5a arm: pack the Fast ladder's rejection tag into the hash slot
/// (dropping the separate `tags` array). DEFAULT ON; the guard in
/// `enable_packed_tags` still refuses frames >= 16 MiB.
static FAST_PACK_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the packed Fast tag.
pub fn set_fast_pack_arm(on: bool) {
    FAST_PACK_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn fast_pack_enabled() -> bool {
    !matches!(FAST_PACK_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}


/// ffanat hash-width census: candidates whose FOUR bytes matched (`cand.1`)
/// versus matches actually ACCEPTED (`ml >= mls`). The difference is work a
/// 4-byte hash creates that an `mls`-byte hash (C's `ZSTD_hashPtr`) would not:
/// every such candidate costs a random `src[m]` load, a compare, and a
/// `count_match` that dies below `mls`.
#[cfg(feature = "profile")]
pub static FF_LAZY_FIRES: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_LATCH: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_CAND4: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_ACCEPT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear `(four-byte passes, accepted matches)`.
#[cfg(feature = "profile")]
pub fn take_ff_waste() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (FF_CAND4.swap(0, Relaxed), FF_ACCEPT.swap(0, Relaxed))
}


/// ffanat hash-width arm -- **DEFAULT ON, by explicit campaign decision.**
/// OFF = the historical 4-byte hash; ON = key the Fast table on `mls` bytes
/// (C's `ZSTD_hashPtr` design) with the versions protections (switch-latch +
/// window re-seed + anchor bar on rep-dominated blocks).
///
/// Final adjudication, protected: L1 TOTAL -2.49%, HOLDOUT -4.92% (reymont
/// -10.2%, mr -7.2%, dickens -6.3%); L2 TOTAL -2.82%, versions itself a -8.1%
/// WIN at L2. The one standing exception: versions-16m at L1 **+6.33%** --
/// the floor of a six-design refutation ladder (per-block key switch, clear
/// latch, full probe veto, rep-cold hysteresis, dense re-seed for fast, near
/// bar), each recorded at its site. The waste receipt: 82.9% -> 0.1% of
/// candidate passes wasted. Worst-corpus law is waived HERE ONLY, explicitly,
/// by the campaign owner; `set_fast_hash_arm(false)` restores the old bytes
/// exactly.
static FAST_HASH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the mls-wide Fast hash.
pub fn set_fast_hash_arm(on: bool) {
    FAST_HASH_ARM.store(if on { 2 } else { 1 }, core::sync::atomic::Ordering::Relaxed);
}

fn fast_hash_wide_enabled() -> bool {
    !matches!(FAST_HASH_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}
