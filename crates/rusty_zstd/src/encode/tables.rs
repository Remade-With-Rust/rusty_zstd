//! The match tables: the hash/chain/tag arrays and every accessor over them.
//!
//! Split out of `encode.rs` verbatim. The only edit is visibility: a
//! moved item that was private is now `pub(crate)`, so the parent can
//! still name it. That changes who may reference a symbol, not what is
//! emitted for it -- checked, not assumed: the asm board was identical
//! in all thirty-two columns across the move, which matters because
//! this crate builds at the default `codegen-units = 16`, where rustc
//! partitions codegen units BY MODULE and layout can move inlining.

use super::*;
pub(crate) struct MatchTables {
    pub(crate) hash: Vec<u32>,
    /// BRICK 14b: the chain walk's (first-miss, later-miss) accept counts. It was
    /// a `&mut (u32, u32)` argument of the kernel -- a pointer marshalled per
    /// call, forcing the caller's copy into memory and making the kernel a
    /// five-argument function whose fifth rode the stack. The kernel already
    /// holds `&mut MatchTables`; the pair is two words inside it.
    pub(crate) wcls: (u32, u32),
    /// BRICK 74 (K14): the link written for an EMPTY head under the packed
    /// representation -- position 0's own tag under the block's producer,
    /// seated in the tag byte (0 when links carry no tag). See
    /// `set_null_tag`.
    pub(crate) null_link: u32,
    pub(crate) hash_long: Vec<u32>,
    /// 1a array route: the long table's tag byte array, for frames where the
    /// packed form is refused (>= 16 MiB, streaming). Mirrors `tags` exactly:
    /// empty on packed frames, allocated at frame init when the arms say so.
    pub(crate) ltags: Vec<u8>,
    /// BRICK 67: repcode yield of the PREVIOUS block -- the dispatch signal
    /// for the repcode-1 search. Optimistic start so the first block always
    /// probes; a block finding no repcodes turns it off for the next.
    ///
    /// Safe to flip per block: repcode search only changes WHICH matches the
    /// encoder finds, leaving no stale table state (unlike the tag latch).
    pub(crate) rep_yield: f32,
    /// BRICK 52: the AUTHORITATIVE hash_log, clamped once here. The table size
    /// and the hash SHIFT must be derived from the same value or they disagree:
    /// `params.hash_log` can reach 25 (a level-table row) while the table is
    /// capped at `1 << 24`, and `hv >> (32 - 25)` would then index 25 bits into
    /// a 24-bit table. Holding it here makes `h < hash.len()` true by
    /// construction, which is what lets the mask go.
    pub(crate) hash_log: u32,
    pub(crate) chain: Vec<u32>,
    /// E1: the ROW match finder's table -- our `ZSTD_row_match_finder`.
    ///
    /// Empty unless the row arm is on. Allocated lazily at the same place the
    /// chain is, and to the same total size (`1 << hash_log` entries), because
    /// it REPLACES the chain rather than supplementing it. See `rowfind`.
    pub(crate) rows: crate::rowfind::RowTable,
    /// Workspace index: matches must start at or after this (MT overlap prime).
    pub(crate) frame_start: usize,
    /// Sequences the previous block produced, used to size the next block's
    /// `seqs` reservation. A fixed fraction of the block length cannot work:
    /// nci needs ~4k slots per 128 KiB block while sao needs ~357, and
    /// over-reserving measurably regressed sao (+3.9% cyc/byte) -- the same
    /// holdout sign-flip that reverted bricks 31 and 34.
    pub(crate) last_nseq: usize,
    /// GATE 18 @ L1 step probe. `pair_route == 1` pins the search step at 1, and
    /// on sao/mr/dickens step 2 is SMALLER and saves 25-45% of positions. No
    /// content signal separates those from samba/mozilla/x-ray, so the step is
    /// MEASURED instead of predicted: alternate 1,2,1,2 over the first blocks,
    /// compare compressed bytes per input byte, and latch the winner.
    ///
    /// 0 = still probing, 1 = latched on step 1, 2 = latched on step 2.
    pub(crate) step_pick: u8,
    /// Step used by the block currently being encoded, so the outcome can be
    /// attributed to the arm that produced it.
    pub(crate) step_used: u8,
    /// Forces the GATE 6 pair route during a probe run. 0 = normal dispatch.
    pub(crate) route_force: u8,
    /// Blocks probed, and accumulated (compressed / input) per arm.
    pub(crate) step_probed: u32,
    pub(crate) step_sum1: f64,
    pub(crate) step_sum2: f64,
    /// Countdown to a forced re-probe, so content that changes character is
    /// picked up -- the warm-up + re-probe shape GATES 2, 6, 10 and 14 all need.
    pub(crate) step_reprobe: u32,
    /// Search positions per byte from the PREVIOUS block -- the dispatch signal
    /// for the lazy back-fill (defect B1). Measured, not assumed: see the
    /// truth table in `m7-benchmark-repair.md`. High = the search is working
    /// hard to find matches, so a richer chain pays; low = matches come easily
    /// (dense repetitive content) and extra chain density is pure walk cost.
    pub(crate) last_search_per_byte: f32,
    /// WALK-CONTINUE dispatch: EWMA share of walk-continue accepts that were
    /// FIRST-FINDS past a collision (legacy would have emitted a literal),
    /// and the Gate-2-style re-probe countdown. First-find-dominated content
    /// (jsonlog 67%, smallmsg 74%) LOSES under the C-parity walk -- the found
    /// matches displace cheaper literal+rep economies -- while upgrade-rich
    /// content (dickens 45%, reymont 41%) wins big.
    pub(crate) walk_first_share: f32,
    pub(crate) walk_probe: u32,
    /// True once `walk_first_share` has been fed at least one measured block.
    pub(crate) walk_share_meas: bool,
    /// Consecutive measured blocks with walk_first_share under the wide bar.
    pub(crate) wide_ok_blocks: u32,
    /// See `set_chain_tag_arm`: lazy-ladder heads and links carry the hash4
    /// tag in their high 8 bits this frame.
    pub(crate) chain_pack: bool,
    /// Array route for the same filter where the packed form is refused
    /// (>= 16 MiB, streaming): link tags beside `chain`, head tags in
    /// `tags`. Mirrors the ltags story exactly.
    pub(crate) ctags: Vec<u8>,
    /// Frame scratch for the per-block sequence coding (the GATE 6 family):
    /// `coded` and the bitstream buffer were fresh allocations per block.
    pub(crate) coded_scratch: Vec<CodedSeq>,
    pub(crate) bits_scratch: Vec<u8>,
    /// See `set_wide_chain_arm`.
    pub(crate) chain_wide: bool,
    /// Blocks whose finder has actually RUN and written back its signals.
    ///
    /// GATE 1 @ L1 needs this because `rep_yield` starts OPTIMISTIC at 1.0 so
    /// the first block of every frame probes for repcodes. A dispatch reading
    /// `rep_yield` directly would therefore fire on block 0 of EVERY file,
    /// changing output everywhere. Gating on `blocks_done > 0` makes the
    /// dispatch fire only on measured evidence.
    pub(crate) blocks_done: u32,
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
    /// GATE 6 @ L1: the finder's sequence and literal buffers, kept on the
    /// frame for the same reason as `payload_scratch`. `lit_scratch` is the
    /// expensive one -- sized `block_len + LIT_PUSH_WIDTH_MAX`, it cleared the
    /// 128 KiB large-allocation threshold on every single block.
    pub(crate) seq_scratch: Vec<Seq>,
    pub(crate) lit_scratch: Vec<u8>,
    /// GATE 6, deeper: `find_opt`'s parse-backtrace buffer, kept for the same
    /// reason as `payload_scratch` and worth far more -- this one carried the
    /// bulk of the 340 MB L19 was pushing through `realloc` on a 2 MiB board.
    /// W2: `(start, off, ml)`. The trailing `matched: bool` went away with
    /// W1 -- every entry is a match now, and the flag cost 4 bytes of padding
    /// in a 16-byte tuple that the parse writes once per sequence.
    pub(crate) opt_ops: Vec<(u32, u32, u32)>,
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
    pub(crate) opt_price: Vec<u32>,
    pub(crate) opt_prev: Vec<u32>,

    pub(crate) opt_om: Vec<u64>,
    /// GATE 6 @ L3: share of DFast positions where C's `_search_next_long`
    /// probe at `ip+1` actually BEAT the short-hash candidate, measured on the
    /// PREVIOUS block. Same self-calibrating shape as `rep_yield`: the probe
    /// cannot lose locally (it is taken only when strictly longer), so its
    /// losses are downstream parse-cascade effects, and the corpora it hurts are
    /// the ones where it fires often and buys little.
    pub(crate) next_long_yield: f32,
    /// GATE 14 @ L3 dispatch: EWMA of the share of raised-band next-long hits
    /// that take a LARGER offset than the match they replace.
    pub(crate) nl_off_worse: f32,
    /// Blocks in which the raised band was actually measured.
    pub(crate) nl_band_meas: u32,
    /// Re-probe countdown: the band cannot be measured while the cut is low, so
    /// the gate must periodically raise it again or it latches shut forever.
    pub(crate) nl_band_probe: u32,
    /// GATE 13 @ L1 -- share of the PREVIOUS block's literal runs short enough
    /// for the fixed-width copy to catch. Seeded optimistically so block 0
    /// always takes the fast path.
    pub(crate) lit_short_share: f32,
    /// GATE 13 WIDTH: share of the previous block's literal runs in (16, 32] --
    /// the runs a 32-byte copy catches that a 16-byte one does not.
    pub(crate) lit_mid_share: f32,
    /// GATE 2 second variable: mean rep match length divided by mean match
    /// length on the previous block. Below 1 the repcode search is trading a
    /// LONGER hash match for a shorter rep match; above 1 its matches are the
    /// long ones and taking them is free.
    pub(crate) rep_len_ratio: f32,
    /// Countdown to the next forced rep re-probe (the ratio can only be measured
    /// on a block where the search actually ran).
    pub(crate) rep_probe: u32,
    /// Consecutive blocks emitted RAW. Incompressible content otherwise pays the
    /// full match search before anything discovers it is incompressible.
    pub(crate) raw_run: u32,
    /// Countdown to the next forced re-probe of the raw short circuit.
    pub(crate) raw_probe: u32,
    /// GATE 10 @ L19: bytes the opt DP's repcode candidate covers per probe,
    /// EWMA'd. It runs at EVERY position and is worth keeping on almost nothing:
    /// versions-16m 434 B/probe and text-32m 26,932 need it, everything else is
    /// at most 35.6 and is SMALLER without it.
    pub(crate) opt_rep_rate: f32,
    /// Countdown to the next forced re-probe, so an off block can be re-measured.
    pub(crate) opt_rep_probe: u32,
    /// Highest bytes-per-rep-probe seen this frame, and how many real
    /// measurements it is built from. The PEAK is what characterises the
    /// content; a single dry block sends the instantaneous rate to 0.
    pub(crate) opt_rep_peak: f32,
    pub(crate) opt_rep_meas: u32,
    /// Blocks in which the candidate has actually RUN. The gate may not shut
    /// until it has real evidence: block 0 of a frame has no history, so its
    /// rate is unrepresentative -- and because an OFF block records no hits, a
    /// gate that shuts on block 0 suppresses its own measurement and can never
    /// reopen. Same cold-start defect as Gate 6's `pair_gain` (4.17).
    pub(crate) opt_rep_seen: u32,
    /// GATE 19: measured literal price in bits, fed to the next block's opt DP.
    /// 0 = not yet measured this frame.
    ///
    /// PER-FRAME, like every other feedback signal here. It first shipped as a
    /// process-global static, which made compression depend on CALL HISTORY:
    /// the same input at L19 gave a different result on the first call than on
    /// later ones (8/12 corpora), because the frame inherited whatever the
    /// PREVIOUS compression had left behind.
    pub(crate) opt_lit_price: u32,
    /// GATE 9 @ L3: mean MATCH LENGTH on the previous DFast block, EWMA'd.
    /// Skipping odd positions shifts a LONG match by a byte (free) but loses a
    /// SHORT match outright, and loses nothing where there are no matches.
    pub(crate) dfast_mean_ml: f32,
    /// GATE 8 @ L3: share of DFast's speculated loads that the next iteration
    /// actually consumed. Low = the pipeline is prefetching for positions the
    /// match logic then jumps past.
    pub(crate) dfast_spec_yield: f32,
    /// Countdown to the next forced DFast-pipeline re-probe.
    pub(crate) dfast_probe: u32,
    /// GATE 6 second variable: MATCH BYTES PER PROBE on the previous block --
    /// the pair search's exchange rate, benefit over cost in the units the cost
    /// is actually paid in. The pair search costs real probe time (+28.9% mean at L1), so it
    /// must be shown to be EARNING. Low gain = the extra probes find nothing:
    /// x-ray 0.0010 pays 19.8% time for 0.02% size; sao 0.0358 pays 61.8% for
    /// 1.82%. Winners sit an order of magnitude higher (nci 0.1875, mozilla
    /// 0.3105).
    pub(crate) pair_gain: f32,
    /// Countdown to the next forced pair RE-PROBE. Without it the gain term is a
    /// one-way latch: pair off => 0 bytes attributed => gain 0 => off forever,
    /// so content that changes phase mid-stream could never recover. On a
    /// rejected block the gain is RETAINED (not zeroed) and re-measured every
    /// `PAIR_PROBE_PERIOD` blocks.
    pub(crate) pair_probe: u32,
    /// GATE 6 ROUTE for this block: 0 = off, 1 = pipelined step-1, 2 = pair.
    /// The pair search and the step-1 loop probe the SAME positions, but step-1
    /// has a pipelined, HLOG-specialised body while the pair path forfeits
    /// pipelining entirely (`if PIPE && !pair`). On 13 of 16 corpora they tie on
    /// size and step-1 is ~7 points cheaper; on `nci` pair is twice as good
    /// (-12.97% vs -6.44%). Same work, opposite verdicts -- so it is routed.
    pub(crate) pair_route: u8,
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
    pub(crate) tags: alloc::vec::Vec<u8>,
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
    pub(crate) pack_tags: bool,
    /// ffanat hash-width: latched by the fast_lazy SWITCH, not by rep_yield.
    /// `find_lazy` reads this table with 4-byte `hash_mls` keys, so a wide
    /// frame that routes blocks to lazy would hand it a key-blind table (the
    /// residual +10.7% on versions after every probe-side protection). The
    /// switch is ground truth for rep-dominated frames: on its first fire the
    /// table is cleared once and the frame's key latches legacy, coherent for
    /// lazy and for every later Fast block.
    pub(crate) fast_hash_legacy: bool,
    /// Share of the PREVIOUS block's candidates the tag would have rejected --
    /// i.e. loads of `src[m]` it saves. Winners sit at 51-100%, the two losers
    /// at 34.3% (mr) and 12.5% (reymont), so the filter is worth its compare
    /// only above roughly half.
    pub(crate) tag_yield: f32,
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
    pub(crate) rep_run: u32,
}

// The BOARD list is probe-scoped too, not just the scratch: the one caller is
// GATE 18's step probe, which is gated on `params.strategy == Strategy::Fast`,
// and the fast family reads exactly TWO boards -- `hash` and `tags` (audited:
// `find_fast_impl_inner` takes those two and `fast_probe`/`fast_probe_wide`
// touch nothing else). `hash_long`, `chain`, `ltags`, `ctags` and `rows` are
// dfast/lazy/bt/row state the probe can never reach, so cloning them was more
// dead bytes -- `chain` alone is up to 64 MB at high chain logs. A future
// caller that probes a non-fast strategy must widen this list; the debug
// assert at the clone site is the tripwire.
impl Clone for MatchTables {
    fn clone(&self) -> Self {
        Self {
            hash: self.hash.clone(),
            wcls: (0, 0),
            null_link: 0,
            hash_long: Vec::new(),
            ltags: Vec::new(),
            rep_yield: self.rep_yield,
            hash_log: self.hash_log,
            chain: Vec::new(),
            rows: crate::rowfind::RowTable::default(),
            frame_start: self.frame_start,
            last_nseq: self.last_nseq,
            step_pick: self.step_pick,
            step_used: self.step_used,
            route_force: self.route_force,
            step_probed: self.step_probed,
            step_sum1: self.step_sum1,
            step_sum2: self.step_sum2,
            step_reprobe: self.step_reprobe,
            last_search_per_byte: self.last_search_per_byte,
            walk_first_share: self.walk_first_share,
            walk_probe: self.walk_probe,
            walk_share_meas: self.walk_share_meas,
            wide_ok_blocks: self.wide_ok_blocks,
            chain_pack: self.chain_pack,
            ctags: Vec::new(),
            coded_scratch: Vec::new(),
            bits_scratch: Vec::new(),
            chain_wide: self.chain_wide,
            blocks_done: self.blocks_done,
            seq_scratch: Vec::new(),
            lit_scratch: Vec::new(),
            opt_ops: Vec::new(),
            opt_price: Vec::new(),
            opt_prev: Vec::new(),
            opt_om: Vec::new(),
            next_long_yield: self.next_long_yield,
            nl_off_worse: self.nl_off_worse,
            nl_band_meas: self.nl_band_meas,
            nl_band_probe: self.nl_band_probe,
            lit_short_share: self.lit_short_share,
            lit_mid_share: self.lit_mid_share,
            rep_len_ratio: self.rep_len_ratio,
            rep_probe: self.rep_probe,
            raw_run: self.raw_run,
            raw_probe: self.raw_probe,
            opt_rep_rate: self.opt_rep_rate,
            opt_rep_probe: self.opt_rep_probe,
            opt_rep_peak: self.opt_rep_peak,
            opt_rep_meas: self.opt_rep_meas,
            opt_rep_seen: self.opt_rep_seen,
            opt_lit_price: self.opt_lit_price,
            dfast_mean_ml: self.dfast_mean_ml,
            dfast_spec_yield: self.dfast_spec_yield,
            dfast_probe: self.dfast_probe,
            pair_gain: self.pair_gain,
            pair_probe: self.pair_probe,
            pair_route: self.pair_route,
            tags: self.tags.clone(),
            pack_tags: self.pack_tags,
            fast_hash_legacy: self.fast_hash_legacy,
            tag_yield: self.tag_yield,
            rep_run: self.rep_run,
        }
    }
}

impl MatchTables {
    pub(crate) fn new(params: CompressionParameters) -> Self {
        // Size unknown (streaming, dict harvest, tests): the row finder's
        // AUTO band is a source-length band, so an unknown length must
        // resolve to the chain. See `row_auto_ok`.
        Self::new_sized(params, None)
    }

    pub(crate) fn new_sized(params: CompressionParameters, src_len: Option<u64>) -> Self {
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
            wcls: (0, 0),
            null_link: 0,
            hash_long: if use_long { vec![0; hsz] } else { Vec::new() },
            ltags: Vec::new(),
            chain: if use_chain { vec![0; csz] } else { Vec::new() },
            // Sized to the CHAIN it replaces, and only when the arm is on --
            // an empty `head` is what every hot-path site tests, so the
            // default build allocates nothing and branches once per insert.
            rows: {
                let mut r = crate::rowfind::RowTable::default();
                if use_chain && row_auto_ok(params, src_len) {
                    r.reset(params.chain_log.min(24));
                }
                r
            },
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
            tags: if use_tags {
                alloc::vec![0u8; hsz]
            } else {
                alloc::vec::Vec::new()
            },
            pack_tags: false,
            fast_hash_legacy: false,
            tag_yield: 1.0,
        }
    }

    pub(crate) fn reset(&mut self) {
        crate::copies::add(
            crate::copies::C_TABLE_CLEAR,
            self.tags.len()
                + self.hash.len() * 4
                + self.hash_long.len() * 4
                + self.ltags.len()
                + self.chain.len() * 4
                + self.ctags.len(),
        );
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
    pub(crate) fn store_fast(&mut self, h: usize, pos: usize, tag: u8, packed: bool) {
        debug_assert_eq!(packed, self.pack_tags);
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
        if packed {
            *unsafe { self.hash.get_unchecked_mut(h) } =
                (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        // Same provable bound as `fast_slot_store`'s.
        if !self.tags.is_empty() {
            debug_assert_eq!(self.tags.len(), self.hash.len());
            *unsafe { self.tags.get_unchecked_mut(h) } = tag;
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
    ///
    /// Its only call site is inside `#[cfg(feature = "profile")]` (the tag
    /// audit), so it is dead in a shipping build and gated to match.
    #[cfg(feature = "profile")]
    #[inline(always)]
    pub(crate) fn raw_fast(&self, h: usize) -> u32 {
        let e = self.hash[h];
        if self.pack_tags {
            e & 0x00FF_FFFF
        } else {
            e
        }
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
    pub(crate) fn put_h(&mut self, h: usize, pos: usize) {
        debug_assert!(h < self.hash.len());
        // W9: same unreachable saturation as the tagged stores -- `pos` is an
        // index into `src`, whose length is bounded well under `u32::MAX` on
        // every path that reaches a hash table.
        debug_assert!(pos < u32::MAX as usize);
        *unsafe { self.hash.get_unchecked_mut(h) } = (pos as u32) + 1;
    }

    /// T1: `put_h` that also writes the tag, so the short table obeys the same
    /// rule `store_fast` does -- the tag is written UNCONDITIONALLY whenever the
    /// array exists. Gating the store on the same flag as the compare is what
    /// lets tags go stale (190ad8b, and again in `prime_tables`).
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn put_h_tag(&mut self, h: usize, pos: usize, tag: u8, packed: bool, live: bool) {
        debug_assert_eq!(packed, self.pack_tags);
        debug_assert_eq!(live, !self.tags.is_empty());
        if packed {
            // `pos + 1` is guaranteed < 2^24 by `enable_packed_tags`, so the
            // mask cannot truncate a live position, and the low bits are never
            // 0 -- an all-zero word still means "empty".
            debug_assert!(h < self.hash.len());
            // W6: the saturating form's cmov is unreachable -- `pack_tags`
            // requires `len < 0x00FF_FFFF`, so `pos + 1` fits the field.
            debug_assert!(pos + 1 < 0x00FF_FFFF);
            *unsafe { self.hash.get_unchecked_mut(h) } =
                (((pos as u32) + 1) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        // W7 (fast ladder) + W8 (dfast): `tags` is allocated at EXACTLY
        // `hash.len()` at all four of its sites, so the bounds test was dead;
        // and its EMPTINESS is a per-BLOCK fact the caller now hoists, so the
        // length load and test leave the per-position path too.
        if live {
            debug_assert!(!self.tags.is_empty() && self.tags.len() == self.hash.len());
            *unsafe { self.tags.get_unchecked_mut(h) } = tag;
        }
        debug_assert!(h < self.hash.len());
        debug_assert!(pos < u32::MAX as usize);
        *unsafe { self.hash.get_unchecked_mut(h) } = (pos as u32) + 1;
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
    pub(crate) fn enable_packed_tags(&mut self, on: bool, len: usize) {
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
    pub(crate) fn get_h_tag(&self, h: usize, tag: u8, on: bool, packed: bool) -> Option<usize> {
        debug_assert_eq!(packed, self.pack_tags);
        debug_assert!(h < self.hash.len());
        let v = *unsafe { self.hash.get_unchecked(h) };
        if v == 0 {
            return None;
        }
        if packed {
            if (v >> 24) as u8 != tag {
                return None;
            }
            return Some(((v & 0x00FF_FFFF) as usize) - 1);
        }
        // W8: same provable bound as the store's -- per short-table PROBE.
        if on && !self.tags.is_empty() {
            debug_assert_eq!(self.tags.len(), self.hash.len());
            #[allow(unsafe_code)]
            let t = *unsafe { self.tags.get_unchecked(h) };
            if t != tag {
                return None;
            }
        }
        Some((v as usize) - 1)
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn get_h(&self, h: usize) -> Option<usize> {
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
    pub(crate) fn lz_head_raw(&self, h: usize) -> u32 {
        debug_assert!(h < self.hash.len());
        *unsafe { self.hash.get_unchecked(h) }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn lz_head_put(&mut self, h: usize, pos: usize, tag: u8, cp: bool) {
        debug_assert!(h < self.hash.len());
        // Same unreachable saturation as the fast/tag stores: `pos` indexes
        // `src`, and the packed route is bounded harder still by
        // `pack_tags`' `len < 0x00FF_FFFF`. Two cmovs per chain insert, i.e.
        // per POSITION across L5-L12, guarding an input no frame produces.
        debug_assert!(pos < u32::MAX as usize);
        let v = if cp {
            // BRICK 48 (P6): no field mask -- the bound above is the whole
            // proof, and the `and` was one instruction per chain insert.
            debug_assert!(pos + 1 < 0x00FF_FFFF);
            ((pos as u32) + 1) | (u32::from(tag) << 24)
        } else {
            (pos as u32) + 1
        };
        *unsafe { self.hash.get_unchecked_mut(h) } = v;
    }

    #[inline(always)]
    pub(crate) fn lz_head_pos(raw: u32, cp: bool) -> Option<usize> {
        let p = if cp { raw & 0x00FF_FFFF } else { raw };
        if p == 0 {
            None
        } else {
            Some((p as usize) - 1)
        }
    }

    #[inline(always)]
    pub(crate) fn lz_head_tag(raw: u32) -> u8 {
        (raw >> 24) as u8
    }

    /// BRICK 74 (K14): the tag every EMPTY head's packed link will carry --
    /// position 0's tag under the producer the block inserts with. Set at
    /// the start of every chain-ladder block, before priming, and before
    /// the wide-chain re-insert, so a link written by this block carries the
    /// tag the block's walks compare against. With it the packed walk needs
    /// no `m != 0` exemption: a phantom position-0 candidate is tag-tested
    /// like any other, and a sound tag rejects only what the first-word
    /// compare would reject. Links without a tag byte stay 0.
    #[inline(always)]
    pub(crate) fn set_null_tag(&mut self, tag0: u8) {
        self.null_link = if self.chain_pack {
            u32::from(tag0) << 24
        } else {
            0
        };
    }

    /// The link stored for a new position is the OLD head, re-encoded from
    /// `(pos+1) | tag<<24` to `pos | tag<<24` (empty stays 0).
    ///
    /// BRICK 47 (P5): one decode for all three representations, the fill's
    /// brick 36 applied to the per-POSITION insert. A packed head's low 24
    /// bits are `pos + 1 >= 1` unless the whole word is 0 (every writer
    /// stores `pos + 1` under `pack_tags`' `len < 0x00FF_FFFF`; the reset
    /// writes 0), so `raw - 1` cannot borrow out of the field and the
    /// seven-instruction split + guard + cmov was decode overhead on every
    /// chain-kernel call.
    #[inline(always)]
    pub(crate) fn lz_link_from_head(raw: u32, null_link: u32) -> u32 {
        debug_assert!(raw == 0 || raw & 0x00FF_FFFF != 0);
        // BRICK 74: an empty head links to position 0 WITH its tag (packed);
        // `null_link` is 0 otherwise, and this is the saturating decode.
        if raw == 0 {
            null_link
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
    pub(crate) fn chain_masked(&self, i: usize) -> u32 {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked(i) }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn chain_masked_set(&mut self, i: usize, v: u32) {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked_mut(i) } = v;
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn ctags_masked(&self, i: usize) -> u8 {
        debug_assert!(i < self.ctags.len());
        *unsafe { self.ctags.get_unchecked(i) }
    }

    /// Lazy-ladder insert, all representations: writes the chain link (and
    /// its tag, packed or array), the head (and its tag), and returns the
    /// OLD head's (pos, tag) for the walk. `ca` = array route.
    #[inline(always)]
    pub(crate) fn lz_insert(
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
        self.chain_masked_set(
            ip & chain_mask,
            Self::lz_link_from_head(raw, self.null_link),
        );
        if ca {
            debug_assert!((ip & chain_mask) < self.ctags.len() && h < self.tags.len());
            #[allow(unsafe_code)]
            unsafe {
                *self.ctags.get_unchecked_mut(ip & chain_mask) = old_tag;
                *self.tags.get_unchecked_mut(h) = gtag;
            }
        }
        self.lz_head_put(h, ip, gtag, cp);
        self.row_insert(h, ip, gtag);
        (Self::lz_head_pos(raw, cp), old_tag)
    }

    /// W10: `lz_insert` for callers that DISCARD the result -- the back-fills.
    /// Identical writes; it just does not decode the old head into the
    /// `Option<usize>` nobody reads.
    #[inline(always)]
    #[allow(unsafe_code)]
    /// `ROWS` is a CONST because both callers already know the answer per
    /// block. The row mirror below ends in `row_insert`, which re-tests
    /// `!rows.head.is_empty()` -- a Vec length load and a branch on EVERY
    /// insert. The lazy fill calls this only from the `else` arm of
    /// `if use_rows`, so rows are provably EMPTY there and the test can never
    /// fire; `lazyfill.rs` measures 41,742,765 fill inserts at L9, which is
    /// the frequency it was running at. Greedy's `g_fill` has no such guard
    /// and passes `true`.
    pub(crate) fn lz_insert_only<const ROWS: bool>(
        &mut self,
        h: usize,
        ip: usize,
        gtag: u8,
        cp: bool,
        ca: bool,
        chain_mask: usize,
    ) {
        let raw = self.lz_head_raw(h);
        let old_tag = if cp {
            Self::lz_head_tag(raw)
        } else if ca {
            debug_assert!(h < self.tags.len());
            *unsafe { self.tags.get_unchecked(h) }
        } else {
            0
        };
        self.chain_masked_set(
            ip & chain_mask,
            Self::lz_link_from_head(raw, self.null_link),
        );
        if ca {
            debug_assert!((ip & chain_mask) < self.ctags.len() && h < self.tags.len());
            unsafe {
                *self.ctags.get_unchecked_mut(ip & chain_mask) = old_tag;
                *self.tags.get_unchecked_mut(h) = gtag;
            }
        }
        self.lz_head_put(h, ip, gtag, cp);
        if ROWS {
            self.row_insert(h, ip, gtag);
        }
    }

    /// Mirror a chain insert into the ROW table, when the row arm allocated it.
    ///
    /// Placed here rather than in the finder so EVERY fill path feeds the rows
    /// -- including the back-fills, which is exactly where a hand-wired finder
    /// would have silently diverged from the chain it replaces.
    #[inline(always)]
    pub(crate) fn row_insert(&mut self, h: usize, ip: usize, gtag: u8) {
        if !self.rows.head.is_empty() {
            let r = self.rows.row_of(h);
            self.rows.insert(r, ip as u32, gtag);
        }
    }

    /// W28: the BACK-FILL's row insert. `row_insert` re-tests
    /// `!rows.head.is_empty()` on every call, but the fill only reaches this
    /// when `use_rows` is already true -- and `use_rows` is exactly
    /// `row_find_enabled() && !rows.head.is_empty()`, decided once per block.
    /// `lazyfill.rs` measures 41,742,765 fill inserts at L9, so that was a
    /// `Vec` length load and a branch on each one.
    ///
    /// SUPERSEDED, kept as the reference shape -- the convention this crate
    /// already uses for `encode_4_streams` and `segment_histograms`. The fill
    /// loop now calls `rows.insert_h` directly, which is strictly better: the
    /// row mask is hoisted, `row_of(h)` is not re-derived, and the emptiness
    /// test is gone. W28 was superseded, NOT reverted -- worth stating,
    /// because a dead optimisation helper looks identical to a reverted one.
    #[allow(dead_code)]
    #[inline(always)]
    pub(crate) fn row_insert_live(&mut self, h: usize, ip: usize, gtag: u8) {
        debug_assert!(!self.rows.head.is_empty());
        let r = self.rows.row_of(h);
        self.rows.insert(r, ip as u32, gtag);
    }

    /// The ROW finder's insert -- and it writes exactly ONE table.
    ///
    /// W8 started here by passing `r` in (`row_insert` re-derived `row_of(h)`
    /// the finder already held) and dropping the `(Option<usize>, u8)` return
    /// that caller discards. Section 14.8 then took the rest: `row_find_best`
    /// reads `rows` and NOTHING else, so while the row arm is on, the chain
    /// link, the chain tags, the hash head and the head tags are all written
    /// and never read. Proven by removing them and re-boarding: 32 cells --
    /// 12 corpora at L9, 10 each at L7 and L12 -- all byte-identical.
    ///
    /// THE RESIDUAL RISK, STATED: that is an empirical result about the paths
    /// these boards exercise, not a proof by construction. A future caller
    /// that reads the chain while rows are active would see a stale table.
    /// The row arm is bitstream-changing and defaults OFF, and `rowboard` is
    /// its gate -- any such caller shows up there as a changed cell.
    ///
    /// ATTRIBUTE HIJACK, repaired. `row_insert_live` had been inserted BETWEEN
    /// this doc block and this `fn` line, so the block documented `row_insert_live`
    /// (which has its own doc), the `#[inline(always)]` below re-parented onto
    /// it as a duplicate, and THIS function -- the row finder's per-position
    /// insert -- silently lost its inline hint. The `unused attribute` warning
    /// on the duplicate was the only signal, and it names the innocent line.
    #[inline(always)]
    pub(crate) fn lz_insert_rowknown(
        &mut self,
        r: usize,
        at: usize,
        head: u32,
        ip: usize,
        gtag: u8,
    ) {
        debug_assert!(!self.rows.head.is_empty());
        self.rows.insert_at(r, at, head, ip as u32, gtag);
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
    pub(crate) fn chain_at(&self, i: usize) -> u32 {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked(i) }
    }

    /// T2: binary-tree slot write. Same invariant as `chain_at`.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn chain_set(&mut self, i: usize, v: u32) {
        debug_assert!(i < self.chain.len());
        *unsafe { self.chain.get_unchecked_mut(i) } = v;
    }

    #[allow(unsafe_code)]
    pub(crate) fn put_hl(&mut self, h: usize, pos: usize) {
        debug_assert!(h < self.hash_long.len());
        // Last of the position-encoding saturations (see `put_h`): `pos`
        // indexes `src`, so `pos + 1` cannot wrap and the cmov is dead.
        debug_assert!(pos < u32::MAX as usize);
        *unsafe { self.hash_long.get_unchecked_mut(h) } = (pos as u32) + 1;
    }

    /// 1a: long-table stores carry the SHORT tag (`hash4_tag`'s byte) in the
    /// high 8 bits on packed frames -- the same representation and < 16 MiB
    /// bound as `put_h_tag`, and the tag costs NOTHING new: it is a function
    /// of the first 4 bytes and is already computed at every store site for
    /// the short table. Every long-candidate acceptance verifies at least 4
    /// leading bytes (`match_ok` with `max(4, ..)`), so a mismatch provably
    /// cannot hide a match. Representation follows `pack_tags`
    /// unconditionally (the 190ad8b rule); the arm gates only the COMPARE.
    /// `packed` is the caller's HOISTED `pack_tags` (per frame). Reading the
    /// field per call cost a load and a branch on every tag operation --
    /// eleven per position in the dfast twin -- for a value that cannot
    /// change inside a block. Same shape `find_fast_impl` uses for the short
    /// table (`let pack = tables.pack_tags`).
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn put_hl_tag(&mut self, h: usize, pos: usize, tag: u8, packed: bool, live: bool) {
        debug_assert!(h < self.hash_long.len());
        debug_assert_eq!(packed, self.pack_tags);
        debug_assert_eq!(live, !self.ltags.is_empty());
        if packed {
            // W5: `pack_tags` is set only when `len < 0x00FF_FFFF`, so
            // `pos + 1` fits the 24-bit field with room to spare and the
            // saturating form's cmov is unreachable. The mask stays: it is
            // what splits the field from the tag.
            debug_assert!(pos + 1 < 0x00FF_FFFF);
            *unsafe { self.hash_long.get_unchecked_mut(h) } =
                (((pos as u32) + 1) & 0x00FF_FFFF) | (u32::from(tag) << 24);
            return;
        }
        // Array route (>= 16 MiB / streaming): written UNCONDITIONALLY
        // whenever the array exists -- gating the store on the compare's flag
        // is what lets tags go stale (190ad8b).
        // W3 + W8: dead bound, and the emptiness is now the caller's hoisted
        // per-block fact.
        if live {
            debug_assert!(!self.ltags.is_empty() && self.ltags.len() == self.hash_long.len());
            *unsafe { self.ltags.get_unchecked_mut(h) } = tag;
        }
        // W4: `saturating_add` is a cmov the encoder can never take. A
        // position is an index into `src`, and a frame that reached this
        // route has `len < u32::MAX`, so `pos + 1` cannot wrap -- the
        // saturation was a per-store instruction guarding an impossible
        // input. (The packed route is bounded harder still, by pack_tags'
        // own `len < 0x00FF_FFFF`.)
        debug_assert!(pos < u32::MAX as usize);
        *unsafe { self.hash_long.get_unchecked_mut(h) } = (pos as u32) + 1;
    }

    /// 1a: tag-filtered long-table load. `on` gates the compare only; the
    /// unmask under `pack_tags` is unconditional, because the slot holds the
    /// packed form whenever the frame does.
    /// `packed` is the caller's HOISTED `pack_tags` (per frame). Reading the
    /// field per call cost a load and a branch on every tag operation --
    /// eleven per position in the dfast twin -- for a value that cannot
    /// change inside a block. Same shape `find_fast_impl` uses for the short
    /// table (`let pack = tables.pack_tags`).
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn get_hl_tag(&self, h: usize, tag: u8, on: bool, packed: bool) -> Option<usize> {
        debug_assert!(h < self.hash_long.len());
        debug_assert_eq!(packed, self.pack_tags);
        let v = *unsafe { self.hash_long.get_unchecked(h) };
        if v == 0 {
            return None;
        }
        if packed {
            if on && (v >> 24) as u8 != tag {
                return None;
            }
            return Some(((v & 0x00FF_FFFF) as usize) - 1);
        }
        // W2: `ltags` is allocated at EXACTLY `hash_long.len()` at both of
        // its sites, and `h` already indexed `hash_long` above -- so the
        // bounds test and its branch were provably dead on every long probe
        // of the array route. The emptiness check stays: `lt_on` allows the
        // array to be absent when `pack_tags` carries the tag instead.
        if on && !self.ltags.is_empty() {
            debug_assert_eq!(self.ltags.len(), self.hash_long.len());
            #[allow(unsafe_code)]
            let t = *unsafe { self.ltags.get_unchecked(h) };
            if t != tag {
                return None;
            }
        }
        Some((v as usize) - 1)
    }

    /// Diagnostic twin: the raw long-slot position with the mask honored
    /// (COUNT paths only -- the false-reject re-probe).
    #[cfg(feature = "profile")]
    #[inline(always)]
    pub(crate) fn raw_hl(&self, h: usize) -> u32 {
        let e = self.hash_long[h];
        if self.pack_tags {
            e & 0x00FF_FFFF
        } else {
            e
        }
    }
}
