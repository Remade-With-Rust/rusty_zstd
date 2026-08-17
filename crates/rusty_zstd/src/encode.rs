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
    let params = compression_params(opts.level, Some(src.len() as u64))?;
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
    let params = compression_params(level, Some(src.len() as u64))?;
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

pub(crate) struct MatchTables {
    hash: Vec<u32>,
    /// Fast-strategy slots carry an 8-bit tag in the TOP byte and a 24-bit
    /// position residue in the low bits, instead of a bare `pos+1`.
    ///
    /// Brick 41 put the tag in a SEPARATE 64 KiB array, which traded one
    /// random access (`src[m]`) for another (`tag[h]`) -- which is exactly why
    /// it only won 3-7%. Packing removes the added access outright: the slot
    /// we must load and store anyway now carries the tag for free.
    ///
    /// Positions are reconstructed modulo 2^24 -- unambiguous because any
    /// usable candidate lies within `window` of `ip`, and this is only enabled
    /// when `window < 2^24`. Not byte-identical: a slot whose residue AND tag
    /// are both 0 reads as empty (~1 position in 2^32), so it gates on RATIO.
    packed: bool,
    hash_long: Vec<u32>,
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
    /// Search positions per byte from the PREVIOUS block -- the dispatch signal
    /// for the lazy back-fill (defect B1). Measured, not assumed: see the
    /// truth table in `m7-benchmark-repair.md`. High = the search is working
    /// hard to find matches, so a richer chain pays; low = matches come easily
    /// (dense repetitive content) and extra chain density is pure walk cost.
    last_search_per_byte: f32,
    /// Blocks whose finder has actually RUN and written back its signals.
    ///
    /// GATE 1 @ L1 needs this because `rep_yield` starts OPTIMISTIC at 1.0 so
    /// the first block of every frame probes for repcodes. A dispatch reading
    /// `rep_yield` directly would therefore fire on block 0 of EVERY file,
    /// changing output everywhere. Gating on `blocks_done > 0` makes the
    /// dispatch fire only on measured evidence.
    blocks_done: u32,
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
        let packed = params.strategy == Strategy::Fast
            && tag_enabled()
            && (1u64 << params.window_log.min(31)) < (1u64 << 24);
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
            packed,
            rep_yield: 1.0,
            hash_log,
            hash: vec![0; hsz],
            hash_long: if use_long { vec![0; hsz] } else { Vec::new() },
            chain: if use_chain { vec![0; csz] } else { Vec::new() },
            frame_start: 0,
            last_nseq: 0,
            // Start optimistic: the first block back-fills, then measures.
            last_search_per_byte: 1.0,
            blocks_done: 0,
            rep_run: 0,
        }
    }

    pub(crate) fn reset(&mut self) {
        self.hash.fill(0);
        self.hash_long.fill(0);
        self.chain.fill(0);
    }

    /// Store `pos + 1` so slot 0 stays "empty" (C window index never uses 0).
    /// Store a Fast-strategy slot (packed with its tag, or plain).
    #[inline(always)]
    #[allow(unsafe_code)]
    fn store_fast<const PACKED: bool>(&mut self, h: usize, pos: usize, tag: u8) {
        // BRICK 50 -- SAFETY: `h` always arrives from `hash4_tag`, which returns
        // `(hv >> hash_shift) as usize & hash_mask`, and `hash_mask` is
        // `self.hash.len() - 1` where the length is `1 << hash_log` (a non-zero
        // power of two, allocated in `new`). A value masked by `len - 1` is
        // therefore always `< len`. LLVM cannot see this because `hash_mask`
        // spills to the stack, so it emitted a bounds check AND a reload of
        // `hash.len()` on EVERY probe -- 2 of the 6 stack accesses left in the
        // hot loop. The debug build still checks it.
        debug_assert!(h < self.hash.len());
        *unsafe { self.hash.get_unchecked_mut(h) } = if PACKED {
            (((pos as u32).wrapping_add(1)) & 0x00FF_FFFF) | (u32::from(tag) << 24)
        } else {
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

    /// Load a Fast-strategy candidate as `pos+1` (0 = none / tag mismatch).
    ///
    /// In packed mode a tag mismatch returns 0 WITHOUT reading `src[m]`, and
    /// the 24-bit residue is lifted back to an absolute position: the unique
    /// value congruent to it that does not exceed `ip+1`.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn load_fast<const PACKED: bool>(&self, h: usize, ip: usize, tag: u8) -> u32 {
        // SAFETY: identical invariant to `store_fast` -- `h` is masked by
        // `hash.len() - 1` with a power-of-two length. See brick 50.
        debug_assert!(h < self.hash.len());
        let e = *unsafe { self.hash.get_unchecked(h) };
        if e == 0 {
            return 0;
        }
        if !PACKED {
            return e;
        }
        if (e >> 24) as u8 != tag {
            return 0;
        }
        let ip1 = (ip as u32).wrapping_add(1);
        let cand = (ip1 & 0xFF00_0000) | (e & 0x00FF_FFFF);
        if cand > ip1 {
            // Wrapped into the previous 2^24 window.
            cand.checked_sub(1 << 24).unwrap_or(0)
        } else {
            cand
        }
    }

    #[inline(always)]
    fn put_h(&mut self, h: usize, pos: usize) {
        self.hash[h] = (pos as u32).saturating_add(1);
    }

    #[inline(always)]
    fn get_h(&self, h: usize) -> Option<usize> {
        let v = self.hash[h];
        if v == 0 {
            None
        } else {
            Some((v as usize) - 1)
        }
    }

    #[inline(always)]
    fn put_hl(&mut self, h: usize, pos: usize) {
        self.hash_long[h] = (pos as u32).saturating_add(1);
    }

    #[inline(always)]
    fn get_hl(&self, h: usize) -> Option<usize> {
        let v = self.hash_long[h];
        if v == 0 {
            None
        } else {
            Some((v as usize) - 1)
        }
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
        owned.reserve(hist_prefix.len() + src.len());
        owned.extend_from_slice(hist_prefix);
        owned.extend_from_slice(src);
        (owned.as_slice(), hist_prefix.len())
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
        while off < workspace.len() {
            let mut end = (off + block_max).min(workspace.len());
            if adv.rsyncable && end > off + 64 {
                if let Some(cut) = crate::ldm::rsync_cut(&workspace[off..end], rbits) {
                    if cut > 32 && off + cut < workspace.len() {
                        end = off + cut;
                    }
                }
            }
            let last = end == workspace.len();
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
    let hash_log = params.hash_log;
    let chain_mask = tables.chain.len().saturating_sub(1);
    let mut p = from;
    while p <= ilimit && p + 8 <= src.len() {
        let h = hash_mls(src, p, mls, hash_log);
        if !tables.chain.is_empty() {
            tables.chain[p & chain_mask] = tables.get_h(h).map(|x| x as u32).unwrap_or(0);
        }
        if tables.packed {
            // Fast strategy primes here on dict/prefix frames; the slot must
            // be written in the SAME format `load_fast` will read.
            let hs = 32u32.saturating_sub(tables.hash_log);
            let (th, tg) = hash4_tag::<true>(src, p, hs);
            tables.store_fast::<true>(th, p, tg);
        } else {
            tables.put_h(h, p);
        }
        if p + 8 <= src.len() && !tables.hash_long.is_empty() {
            let hl = hash8(src, p, hash_log);
            tables.put_hl(hl, p);
        }
        p += 1;
    }
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

    let (seqs, literals) = {
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
        return Ok(());
    }
    let match_b: usize = seqs.iter().map(|s| s.matchlen as usize).sum();
    let lit_b = literals.len();
    let mg = min_gain(block.len(), params.strategy);
    let peak = huffman::lit_sample_peak(if seqs.is_empty() { block } else { &literals });
    if early_raw_skip(match_b, block.len(), params) {
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
        return Ok(());
    }
    let saved_reps = *reps;
    let saved_ent = entropy.clone();
    crate::prof::note_scratch(1);
    // Brick 44: reserve the block payload. Same shape as brick 38 (which won
    // 6/6, z=+2.45 by removing per-block Vec growth): this grew from zero by
    // doubling on EVERY block, re-copying ~2x the compressed block size each
    // time. `block.len()` is a hard upper bound -- a payload that reaches it
    // is rejected for Raw by `raw_limit` immediately below.
    let mut payload = if payload_reserve_enabled() {
        Vec::with_capacity(block.len())
    } else {
        Vec::new()
    };
    {
        let _e = crate::prof::scope(crate::prof::Stage::EncodeEntropy);
        if seqs.is_empty() {
            write_literals(&mut payload, block, entropy)?;
            crate::prof::note_emit_lit(payload.len() as u64);
            payload.push(0);
        } else {
            write_literals(&mut payload, &literals, entropy)?;
            let lit_end = payload.len();
            crate::prof::note_emit_lit(lit_end as u64);
            write_sequences(&mut payload, &seqs, reps, entropy, params.strategy)?;
            crate::prof::note_emit_seq((payload.len() - lit_end) as u64);
        }
    }
    let raw_limit = if incomp_skip_on(params) {
        block.len().saturating_sub(mg)
    } else {
        block.len()
    };
    if payload.len() >= raw_limit {
        *reps = saved_reps;
        *entropy = saved_ent;
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
    write_block_header(out, last, BlockType::Compressed, payload.len() as u32);
    out.extend_from_slice(&payload);
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


/// Gate 6 arm: force the pair search on even at `step0 == 2`.
static PAIR_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the Gate 6 truth table.
pub fn set_pair_arm(on: bool) {
    PAIR_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn pair_forced() -> bool {
    PAIR_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
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

fn write_literals(dst: &mut Vec<u8>, lits: &[u8], entropy: &mut EntropyState) -> Result<(), Error> {
    let _h = crate::prof::scope(crate::prof::Stage::EncodeHuff);
    let (sec, upd) = huffman::encode_literals_section(lits, entropy.huff.as_ref())?;
    match upd {
        HuffUpdate::New(ct) => entropy.huff = Some(ct),
        HuffUpdate::Unchanged => {}
    }
    dst.extend_from_slice(&sec);
    Ok(())
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
) -> Result<(), Error> {
    write_nseq(dst, seqs.len() as u32);
    if seqs.is_empty() {
        return Ok(());
    }

    let (coded, ll_count, of_count, ml_count) = {
        let _sc = crate::prof::scope(crate::prof::Stage::EncodeSeqCode);
        let mut coded: Vec<CodedSeq> = Vec::with_capacity(seqs.len());
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
            let (llc, llx, llb) = ll_code(s.litlen);
            let (mlc, mlx, mlb) = ml_code(s.matchlen);
            let (ofc, ofx) = of_code(ov);
            if ofc > 31 {
                return Err(Error::Corruption);
            }
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

        let mut ll_count = [0u32; 36];
        let mut of_count = [0u32; 32];
        let mut ml_count = [0u32; 53];
        for c in &coded {
            ll_count[c.llc as usize] += 1;
            of_count[c.ofc as usize] += 1;
            ml_count[c.mlc as usize] += 1;
        }

        (coded, ll_count, of_count, ml_count)
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
        let of_needs_comp = coded
            .iter()
            .any(|c| c.ofc as usize >= fse::DEFAULT_OF_NORM.len());
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

    let mut bits = BitCStream::with_capacity(coded.len() * 4 + 16);
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
    dst.extend_from_slice(&bits.close());
    entropy.ll = Some(ll_t);
    entropy.of = Some(of_t);
    entropy.ml = Some(ml_t);
    Ok(())
}

/// libzstd `ZSTD_buildCTable`: last sequence is `FSE_initCState2` only.
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
    #[cfg(feature = "std")]
    {
        std::env::var("RZSTD_FASTLAZY_T")
            .ok()
            .and_then(|v| v.trim().parse().ok())
            .unwrap_or(0.7)
    }
    #[cfg(not(feature = "std"))]
    0.7
}

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
fn rep_yield_min_for(strategy: Strategy) -> f32 {
    #[cfg(feature = "std")]
    if let Some(v) = std::env::var("RZSTD_REPMIN")
        .ok()
        .and_then(|v| v.trim().parse::<f32>().ok())
    {
        return v;
    }
    match strategy {
        Strategy::DFast => 0.0,
        _ => REP_YIELD_MIN_DEFAULT,
    }
}

#[allow(dead_code)]
fn rep_yield_min() -> f32 {
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
    macro_rules! go {
        ($p:expr, $r:expr, $h:expr, $s:expr, $pi:expr) => {
            find_fast_impl::<$p, $r, $h, $s, $pi>(
                src,
                block_start,
                block_end,
                window,
                params,
                tables,
                reps,
            )
        };
    }
    let pipe_on = pipe_enabled();
    let s0 = if params.target_length == 0 {
        step0_default()
    } else {
        params.target_length as usize + 1
    };
    // BRICK 67: repcode-1 is DISPATCHED on its own yield, not globally on/off.
    //
    // It is a genuine sign-flip: a LOSS on Silesia (brick 40: 0/6, z=-2.45,
    // sao -23.0%) and a 10x RATIO WIN on constant-stride content
    // (versions-16m L1: 820,848 -> 81,206 bytes). A global default cannot serve
    // both, so each block inherits the previous block's measured repcode yield.
    // `rep_yield` starts at 1.0, so the first block of every frame always probes.
    let rep_on = rep_search_on(tables.rep_yield, params.strategy);
    match (tables.packed, rep_on, pipe_on, s0) {
        // The shipping configuration: no tag, no rep1, pipelined, default step.
        // Specialized on hash_log so the shift is an immediate too.
        (false, false, true, 2) => match tables.hash_log {
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
        (false, false, true, 1) => match tables.hash_log {
            12 => go!(false, false, 12, 1, true),
            13 => go!(false, false, 13, 1, true),
            14 => go!(false, false, 14, 1, true),
            15 => go!(false, false, 15, 1, true),
            16 => go!(false, false, 16, 1, true),
            _ => go!(false, false, 0, 1, true),
        },
        (false, false, true, _) => go!(false, false, 0, 0, true),
        (false, false, false, 2) => go!(false, false, 0, 2, false),
        (false, false, false, 1) => go!(false, false, 0, 1, false),
        (false, false, false, _) => go!(false, false, 0, 0, false),
        (true, true, true, _) => go!(true, true, 0, 0, true),
        (true, true, false, _) => go!(true, true, 0, 0, false),
        (true, false, true, _) => go!(true, false, 0, 0, true),
        (true, false, false, _) => go!(true, false, 0, 0, false),
        (false, true, true, _) => go!(false, true, 0, 0, true),
        (false, true, false, _) => go!(false, true, 0, 0, false),
    }
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
>(
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
    let seq_guess = (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16);
    let mut seqs = if reserve {
        Vec::with_capacity(seq_guess)
    } else {
        Vec::new()
    };
    let mut lits = if reserve {
        Vec::with_capacity(block_len + LIT_PUSH_WIDTH)
    } else {
        Vec::new()
    };
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
    // BRICK 51: `probes`/`hits` feed ONLY `note_search`, which is a no-op
    // without the `profile` feature (their other consumers, `last_hit_rate` and
    // `tag_latch`, were write-only dead state left by the brick-41 revert).
    // Register pressure had spilled `probes` to the stack, so the shipping build
    // was paying a read-modify-write to MEMORY on every probe to feed nothing.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut hits = 0u64;
    let mut rep_hits = 0u64;
    // Probe density. The bit accountant showed our size gap vs C is entirely
    // LITERALS, because C finds more matches -- and we probe only ~0.259
    // positions/byte against C's ~1.0. `step0 = 1` matches C's density.
    // Brick 39 made each probe substantially cheaper, so this trade is worth
    // re-testing. `RZSTD_STEP0` overrides (default 2 = pre-existing).
    // BRICK 55: `step0` is live in the hot advance (`ip + step0 + ..`). The
    // pipelined loop only runs when `!pair`, i.e. `step0 <= 2`, so the default
    // (2) is worth specializing -- it folds into the address arithmetic and
    // frees the register it was holding. `STEP == 0` keeps the runtime path.
    let step0 = if STEP != 0 {
        STEP
    } else if params.target_length == 0 {
        step0_default()
    } else {
        params.target_length as usize + 1
    };
    // Pair-search ip+1 only when step skips it (`--fast=4`, step 5). At step 2
    // that doubles incomp probes for no ratio. Do not grow step without the pair
    // (that blew --fast=4 ratio 0.845 -> 1.272).
    //
    // Gate 6 (gg-matchfind): forceable so the pair search can be given its own
    // truth table INDEPENDENTLY of step0, which is the only way to tell the two
    // apart -- they are the same physical decision reached by two switches.
    let pair = step0 > 2 || pair_forced();
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
    if PIPE && !pair && ip <= ilimit {
        let (mut h0, mut g0) = hash4_tag::<PACKED>(src, ip, hash_shift);
        let mut m0 = tables.load_fast::<PACKED>(h0, ip, g0);
        loop {
            if COUNT {
                if COUNT {
                    probes += 1;
                }
            }
            tables.store_fast::<PACKED>(h0, ip, g0);
            if REP {
                if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end) {
                    rep_hits += 1;
                    if COUNT {
                        if COUNT {
                            hits += 1;
                        }
                    }
                    let mstart = ip + 1;
                    crate::prof::note_huff_path(11);
                    lits.extend_from_slice(&src[anchor..mstart]);
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
                    h0 = (load_u32le(src, ip).wrapping_mul(HASH4_PRIME) >> hash_shift) as usize;
                    m0 = tables.hash[h0];
                    continue;
                }
            }
            // Next position, and its table load issued NOW -- this is the whole
            // point of the brick.
            let nip = ip + step0 + ((ip - anchor) >> 8);
            let (h1, g1, m1) = if nip <= ilimit {
                let (h, g) = hash4_tag::<PACKED>(src, nip, hash_shift);
                // The store above may have just overwritten this slot.
                let v = if h == h0 {
                    if g == g0 {
                        (ip as u32).wrapping_add(1)
                    } else {
                        0
                    }
                } else {
                    tables.load_fast::<PACKED>(h, nip, g)
                };
                (h, g, v)
            } else {
                (0usize, 0u8, 0u32)
            };
            if let Some((m, ml)) = fast_probe(src, m0, ip, window, lowest, mls, block_end) {
                if COUNT {
                    if COUNT {
                        hits += 1;
                    }
                }
                ip = emit_fast_seq::<PACKED>(
                    src,
                    tables,
                    &mut seqs,
                    &mut lits,
                    anchor,
                    ip,
                    m,
                    ml,
                    mls,
                    ilimit,
                    frame_start,
                    reserve,
                );
                anchor = ip;
                if ip > ilimit {
                    break;
                }
                let (nh, ng) = hash4_tag::<PACKED>(src, ip, hash_shift);
                h0 = nh;
                g0 = ng;
                m0 = tables.load_fast::<PACKED>(h0, ip, g0);
                continue;
            }
            if nip > ilimit {
                break;
            }
            ip = nip;
            h0 = h1;
            g0 = g1;
            m0 = m1;
        }
        crate::prof::note_huff_path(12);
        lits.extend_from_slice(&src[anchor..block_end]);
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
        return (seqs, lits);
    }
    while ip <= ilimit {
        if COUNT {
            if COUNT {
                probes += 1;
            }
        }
        let (h0, g0) = hash4_tag::<PACKED>(src, ip, hash_shift);
        let m0 = tables.load_fast::<PACKED>(h0, ip, g0);
        tables.store_fast::<PACKED>(h0, ip, g0);
        if REP {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest, block_end) {
                rep_hits += 1;
                if COUNT {
                    if COUNT {
                        hits += 1;
                    }
                }
                let mstart = ip + 1;
                crate::prof::note_huff_path(11);
                lits.extend_from_slice(&src[anchor..mstart]);
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
        if let Some((m, ml)) = fast_probe(src, m0, ip, window, lowest, mls, block_end) {
            if COUNT {
                hits += 1;
            }
            ip = emit_fast_seq::<PACKED>(
                src,
                tables,
                &mut seqs,
                &mut lits,
                anchor,
                ip,
                m,
                ml,
                mls,
                ilimit,
                frame_start,
                reserve,
            );
            anchor = ip;
            if let Some(sq) = seqs.last() {
                rep1 = sq.offset as usize;
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
                let (h1, g1) = hash4_tag::<PACKED>(src, ip1, hash_shift);
                let m1 = tables.load_fast::<PACKED>(h1, ip1, g1);
                tables.store_fast::<PACKED>(h1, ip1, g1);
                if let Some((m, ml)) = fast_probe(src, m1, ip1, window, lowest, mls, block_end) {
                    if COUNT {
                        if COUNT {
                            hits += 1;
                        }
                    }
                    ip = emit_fast_seq::<PACKED>(
                        src,
                        tables,
                        &mut seqs,
                        &mut lits,
                        anchor,
                        ip1,
                        m,
                        ml,
                        mls,
                        ilimit,
                        frame_start,
                        reserve,
                    );
                    anchor = ip;
                    continue;
                }
            }
        }
        ip += step0 + ((ip - anchor) >> 8);
    }
    crate::prof::note_huff_path(12);
    lits.extend_from_slice(&src[anchor..block_end]);
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
    (seqs, lits)
}

/// C zstd_fast: 4-byte probe then ZSTD_count from +4. `ilimit` keeps ip+4 in-bounds.
/// `match_slot` is the hash-table value (`pos+1`, or 0 = empty).
#[inline(always)]
fn fast_probe(
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
        return None;
    }
    let ml = 4 + count_match(src, m + 4, ip + 4, block_end);
    if ml >= mls {
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
fn lazy_fill_threshold() -> f32 {
    use std::sync::OnceLock;
    static T: OnceLock<f32> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("RZSTD_LAZY_FILL_T")
            .ok()
            .and_then(|v| v.parse().ok())
            .unwrap_or(0.0)
    })
}

/// Back-fill stride (1 = every covered position). `RZSTD_LAZY_FILL_S` sweeps.
fn lazy_fill_stride() -> usize {
    use std::sync::OnceLock;
    static T: OnceLock<usize> = OnceLock::new();
    *T.get_or_init(|| {
        std::env::var("RZSTD_LAZY_FILL_S")
            .ok()
            .and_then(|v| v.parse().ok())
            .filter(|&v: &usize| v >= 1)
            .unwrap_or(1)
    })
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
/// Brick 40 arm: repcode-1 search in `find_fast` (shelved, ratio-changing).
static REP1_ENABLED_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for in-process ABBA; shipping default from `RZSTD_REP1`.
pub fn set_rep1_arm(on: bool) {
    REP1_ENABLED_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// GATE 2 arm: the repcode-1 search, as a THREE-state choice so both constants
/// are reachable. `rep1_enabled()` alone can only force ON, which cannot answer
/// "does any corpus lose under a constant" -- the OFF constant was untestable.
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

/// The Gate 2 decision for this block: forced constant, or the measured yield.
#[inline]
fn rep_search_on(rep_yield: f32, strategy: Strategy) -> bool {
    match REP1_MODE_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => rep1_enabled() || rep_yield >= rep_yield_min_for(strategy),
    }
}

fn rep1_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match REP1_ENABLED_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            // Was `v == "0"`, which INVERTED the override: `RZSTD_REP1=1`
            // turned rep1 OFF and `=0` turned it ON. The default (absent =>
            // false) was correct and the ABBA harness drives `set_rep1_arm`
            // directly, so no shipped verdict was affected -- but an env-driven
            // A/B would have measured off-vs-off and read as "no effect".
            let on = std::env::var("RZSTD_REP1")
                .map(|v| v == "1")
                .unwrap_or(false);
            REP1_ENABLED_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
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
    Some(4 + count_match(src, back + 4, at + 4, block_end))
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

/// Brick 41 arm state: packed tag slots. Settable at RUNTIME so a harness can
/// interleave both arms inside ONE process.
///
/// Why that matters: with each arm as its own process run, the two arms sit
/// MINUTES apart, and this box drifts ~2x on that timescale -- measured, two
/// A/B pairs of the same brick came out -36.7% and +2.6%. ABBA only cancels
/// drift when the arms are SECONDS apart. `set_tag_arm` lets the bench flip
/// the arm between two adjacent measurements of the same file.
static TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Override the tag arm for the rest of the process (0 = re-read the env).
/// Bench/testing hook; the shipping default comes from `RZSTD_TAG`.
pub fn set_tag_arm(on: bool) {
    TAG_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn tag_enabled() -> bool {
    use core::sync::atomic::Ordering;
    match TAG_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            // DEFAULT OFF: the in-process ABBA A/B measured the tag SLOWER on
            // 6/6 files (-2.1% to -7.7%, z=-2.45) with same-arm spreads of
            // 0.3-2.1%. The earlier "+3.6-7.1% win" came from a cross-PROCESS
            // A/B whose arms sat minutes apart on a drifting box.
            let on = std::env::var("RZSTD_TAG")
                .map(|v| v == "1")
                .unwrap_or(false);
            TAG_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// hash4 index AND its 8-bit tag, from one multiply.
///
/// The tag is a pure function of the 4 bytes at `pos`, and `fast_probe`
/// requires those 4 bytes to be EQUAL -- so a tag mismatch implies the bytes
/// differ, i.e. the tag can only reject candidates the probe would reject
/// anyway. That is what makes the whole scheme byte-identical by construction.
#[inline(always)]
fn hash4_tag<const PACKED: bool>(src: &[u8], pos: usize, hash_shift: u32) -> (usize, u8) {
    let hv = load_u32le(src, pos).wrapping_mul(HASH4_PRIME);
    // Fold high bits down: the index consumes the top `hash_log` bits, so the
    // raw low byte would be the weakest part of a multiplicative hash.
    (
        // BRICK 52: no mask. `hash_shift == 32 - hash_log` with the CLAMPED
        // hash_log, so this yields at most `hash_log` bits, i.e. a value
        // strictly below `1 << hash_log == hash.len()`. The `& hash_mask` it
        // replaces was a live loop value costing a register and a stack reload
        // on every probe.
        (hv >> hash_shift) as usize,
        // BRICK 46: the tag is DEAD unless the packed arm is on, and this sits on
        // the hash's dependency chain in the hottest loop in the encoder. A
        // const generic folds it away entirely instead of computing-then-
        // discarding it on every probe.
        if PACKED { (hv ^ (hv >> 15)) as u8 } else { 0 },
    )
}

/// Brick 39 arm state: 2-way pipelined probe. Runtime-settable so the
/// in-process ABBA harness can flip it between adjacent measurements.
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
/// Same disease as brick 49 (`use_rep`) and brick 64 (`seqcheck_hoisted`):
/// a fixed-for-the-block flag re-read in the hottest loop.
fn push_literals(lits: &mut Vec<u8>, src: &[u8], from: usize, to: usize, arm: bool) {
    let n = to - from;
    if n <= LIT_PUSH_WIDTH
        && from + LIT_PUSH_WIDTH <= src.len()
        && lits.capacity() - lits.len() >= LIT_PUSH_WIDTH
        && if litpush_hoist_enabled() { arm } else { lit_push_enabled() }
    {
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
                LIT_PUSH_WIDTH,
            );
            lits.set_len(len + n);
        }
        return;
    }
    lits.extend_from_slice(&src[from..to]);
}

fn emit_fast_seq<const PACKED: bool>(
    src: &[u8],
    tables: &mut MatchTables,
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
    mls: usize,
    ilimit: usize,
    frame_start: usize,
    arm: bool,
) -> usize {
    let mut ip = found_ip;
    let mut mm = m;
    let mut n = ml;
    let back_from = ip;
    while ip > anchor && mm > frame_start && src[ip - 1] == src[mm - 1] {
        ip -= 1;
        mm -= 1;
        n += 1;
    }
    crate::prof::note_back_ext((back_from - ip) as u64);
    push_literals(lits, src, anchor, ip, arm);
    seqs.push(Seq {
        litlen: (ip - anchor) as u32,
        matchlen: n as u32,
        offset: (ip - mm) as u32,
    });
    let end = ip + n;
    fill_hash_after_match::<PACKED>(tables, src, found_ip, end, mls, ilimit);
    end
}

/// C `zstd_fast.c` after a match: insert hash(start+2) and hash(end-2) only.
/// Filling every byte of a long match was ~src_len hash writes on repeating text.
#[inline]
fn fill_hash_after_match<const PACKED: bool>(
    tables: &mut MatchTables,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    _mls: usize,
    ilimit: usize,
) {
    // Shift from the table's OWN clamped hash_log -- never from `params`.
    let hash_shift = 32u32.saturating_sub(tables.hash_log);
    let mut n = 0u64;
    let a = match_ip.saturating_add(2);
    if a <= ilimit {
        let (h, g) = hash4_tag::<PACKED>(src, a, hash_shift);
        tables.store_fast::<PACKED>(h, a, g);
        n += 1;
    }
    if match_end >= 2 {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let (h, g) = hash4_tag::<PACKED>(src, b, hash_shift);
            tables.store_fast::<PACKED>(h, b, g);
            n += 1;
        }
    }
    crate::prof::note_hash_fill(n);
}

#[inline]
fn fill_hash_long_after_match(
    tables: &mut MatchTables,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    hash_log: u32,
    ilimit: usize,
) {
    let a = match_ip.saturating_add(2);
    if a <= ilimit {
        tables.put_hl(hash8(src, a, hash_log), a);
    }
    if match_end >= 2 {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            tables.put_hl(hash8(src, b, hash_log), b);
        }
    }
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
        return find_dfast_runtime(src, block_start, block_end, window, params, tables, reps);
    }
    match tables.hash_log {
        12 => go!(12),
        13 => go!(13),
        14 => go!(14),
        15 => go!(15),
        16 => go!(16),
        17 => go!(17),
        18 => go!(18),
        19 => go!(19),
        20 => go!(20),
        _ => find_dfast_runtime(src, block_start, block_end, window, params, tables, reps),
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
    let mls = params.min_match.max(3) as usize;
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
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
                lits.extend_from_slice(&src[anchor..mstart]);
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
        let h4 = hash_mls(src, ip, 4, HLOG);
        let h8 = hash8(src, ip, HLOG);
        let m4 = tables.get_h(h4);
        let m8 = tables.get_hl(h8);
        tables.put_h(h4, ip);
        tables.put_hl(h8, ip);

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(m8) = m8 {
            if COUNT {
                probes += 1;
            }
            if match_ok(
                src,
                m8,
                ip,
                window,
                block_start,
                8.min(mls).max(4),
                tables.frame_start,
            ) {
                let ml = count_match(src, m8, ip, block_end);
                if ml >= mls {
                    best_m = m8;
                    best_ml = ml;
                }
            }
        }
        if best_ml < 8 {
            if let Some(m4) = m4 {
                if COUNT {
                    probes += 1;
                }
                if match_ok(src, m4, ip, window, block_start, mls, tables.frame_start) {
                    let ml = count_match(src, m4, ip, block_end);
                    if ml >= mls && ml > best_ml {
                        best_m = m4;
                        best_ml = ml;
                    }
                }
            }
        }
        if best_ml >= mls {
            lits.extend_from_slice(&src[anchor..ip]);
            seqs.push(Seq {
                litlen: (ip - anchor) as u32,
                matchlen: best_ml as u32,
                offset: (ip - best_m) as u32,
            });
            rep1 = ip - best_m;
            if COUNT {
                hits += 1;
            }
            let end = ip + best_ml;
            // DFast never sets `packed` (it is gated on Strategy::Fast).
            fill_hash_after_match::<false>(tables, src, ip, end, mls, ilimit);
            fill_hash_long_after_match(tables, src, ip, end, HLOG, ilimit);
            ip = end;
            anchor = ip;
        } else {
            ip += 1 + ((ip - anchor) >> 8);
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    lits.extend_from_slice(&src[anchor..block_end]);
    note_finder_work(COUNT, probes, hits, &seqs, &lits);
    (seqs, lits)
}

fn find_dfast_runtime(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let mls = params.min_match.max(3) as usize;
    let hash_log = tables.hash_log;
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
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
                lits.extend_from_slice(&src[anchor..mstart]);
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
        let h4 = hash_mls(src, ip, 4, hash_log);
        let h8 = hash8(src, ip, hash_log);
        let m4 = tables.get_h(h4);
        let m8 = tables.get_hl(h8);
        tables.put_h(h4, ip);
        tables.put_hl(h8, ip);

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(m8) = m8 {
            if COUNT {
                probes += 1;
            }
            if match_ok(
                src,
                m8,
                ip,
                window,
                block_start,
                8.min(mls).max(4),
                tables.frame_start,
            ) {
                let ml = count_match(src, m8, ip, block_end);
                if ml >= mls {
                    best_m = m8;
                    best_ml = ml;
                }
            }
        }
        if best_ml < 8 {
            if let Some(m4) = m4 {
                if COUNT {
                    probes += 1;
                }
                if match_ok(src, m4, ip, window, block_start, mls, tables.frame_start) {
                    let ml = count_match(src, m4, ip, block_end);
                    if ml >= mls && ml > best_ml {
                        best_m = m4;
                        best_ml = ml;
                    }
                }
            }
        }
        if best_ml >= mls {
            lits.extend_from_slice(&src[anchor..ip]);
            seqs.push(Seq {
                litlen: (ip - anchor) as u32,
                matchlen: best_ml as u32,
                offset: (ip - best_m) as u32,
            });
            rep1 = ip - best_m;
            if COUNT {
                hits += 1;
            }
            let end = ip + best_ml;
            // DFast never sets `packed` (it is gated on Strategy::Fast).
            fill_hash_after_match::<false>(tables, src, ip, end, mls, ilimit);
            fill_hash_long_after_match(tables, src, ip, end, hash_log, ilimit);
            ip = end;
            anchor = ip;
        } else {
            ip += 1 + ((ip - anchor) >> 8);
        }
    }
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    lits.extend_from_slice(&src[anchor..block_end]);
    note_finder_work(COUNT, probes, hits, &seqs, &lits);
    (seqs, lits)
}

/// P0/gg-matchfind: one reporting point for every finder, so `work` (candidate
/// examinations) is defined the SAME way across arms. Finders whose probe loop
/// lives in `chain_find_best` / `bt_find_best` pass `probes = 0` -- those
/// helpers already reported their own via `note_probes`.
#[inline]
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
    let mls = params.min_match.max(3) as usize;
    let hash_log = params.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let attempts = search_attempts(params);
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut hits = 0u64;
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 71: repcode-1 search in find_greedy -- L5-L6 had none
    // C checks `offset_1` at every position in `_greedy`/`_lazy` exactly as in
    // `_fast`/`_doubleFast`. Same dispatch on measured yield as bricks 67/70.
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
                lits.extend_from_slice(&src[anchor..mstart]);
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
        let h = hash_mls(src, ip, mls, hash_log);
        let prev = tables.get_h(h);
        tables.chain[ip & chain_mask] = prev.map(|p| p as u32).unwrap_or(0);
        tables.put_h(h, ip);

        let mut best_m = 0usize;
        let mut best_ml = 0usize;
        if let Some(mut m) = prev {
            for _ in 0..attempts {
                if !match_ok(src, m, ip, window, block_start, mls, tables.frame_start) {
                    break;
                }
                if COUNT {
                    probes += 1;
                }
                // C's `match[ml] == ip[ml]` prefilter (`ZSTD_HcFindBestMatch`):
                // a candidate that DIFFERS at the current best length cannot
                // exceed it, so the full `count_match` is provably wasted. The
                // same candidate still wins, so this is byte-identical.
                if best_ml == 0 || src[m + best_ml] == src[ip + best_ml] {
                    let ml = count_match(src, m, ip, block_end);
                    if ml >= mls && ml > best_ml {
                        best_ml = ml;
                        best_m = m;
                        // Reaches the block end -- nothing can be longer.
                        if ip + best_ml >= block_end {
                            break;
                        }
                    }
                }
                let next = tables.chain[m & chain_mask] as usize;
                if next >= m {
                    break;
                }
                m = next;
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
            while s > anchor && mm > tables.frame_start && src[s - 1] == src[mm - 1] {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            lits.extend_from_slice(&src[anchor..s]);
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
                let hh = hash_mls(src, p, mls, hash_log);
                tables.chain[p & chain_mask] = tables.get_h(hh).map(|x| x as u32).unwrap_or(0);
                tables.put_h(hh, p);
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
    lits.extend_from_slice(&src[anchor..block_end]);
    note_finder_work(COUNT, probes, hits, &seqs, &lits);
    (seqs, lits)
}

#[allow(clippy::too_many_arguments)]
fn chain_find_best(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let hash_log = params.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let attempts = search_attempts(params);
    let h = hash_mls(src, ip, mls, hash_log);
    let prev = tables.get_h(h);
    tables.chain[ip & chain_mask] = prev.map(|p| p as u32).unwrap_or(0);
    tables.put_h(h, ip);
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
    for _ in 0..attempts {
        if !match_ok(src, m, ip, window, block_start, mls, tables.frame_start) {
            break;
        }
        if COUNT {
            probes += 1;
        }
        // C's `match[ml] == ip[ml]` prefilter -- see `find_greedy`.
        if best_ml == 0 || src[m + best_ml] == src[ip + best_ml] {
            let ml = count_match(src, m, ip, block_end);
            if ml >= mls && ml > best_ml && offset_ok(ip - m, window) && m >= tables.frame_start {
                best_ml = ml;
                best_m = m;
                if ip + best_ml >= block_end {
                    break;
                }
            }
        }
        let next = tables.chain[m & chain_mask] as usize;
        if next >= m {
            break;
        }
        m = next;
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
    let mls = params.min_match.max(3) as usize;
    let hash_log = params.hash_log;
    let chain_mask = tables.chain.len() - 1;
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 71: repcode-1 search in find_lazy -- L7-L12 had none
    // C checks `offset_1` at every position in `_greedy`/`_lazy` exactly as in
    // `_fast`/`_doubleFast`. Same dispatch on measured yield as bricks 67/70.
    let use_rep = rep_search_on(tables.rep_yield, params.strategy);
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
    while ip <= ilimit {
        if use_rep {
            if let Some(ml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                rep_hits += 1;
                let mstart = ip + 1;
                lits.extend_from_slice(&src[anchor..mstart]);
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
            chain_find_best(src, ip, block_start, block_end, window, mls, params, tables);
        let mut best_ip = ip;
        let mut look_hi = ip; // PROBE: highest position the look-ahead inserted
        if best_ml >= mls {
            for d in 1..=depth {
                let ip2 = ip + d;
                if ip2 > ilimit {
                    break;
                }
                look_hi = ip2;
                let (m, ml) = chain_find_best(
                    src,
                    ip2,
                    block_start,
                    block_end,
                    window,
                    mls,
                    params,
                    tables,
                );
                if ml > best_ml {
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
            while s > anchor && mm > tables.frame_start && src[s - 1] == src[mm - 1] {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            lits.extend_from_slice(&src[anchor..s]);
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
                let stride = lazy_fill_stride();
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
                while p < end && p <= ilimit {
                    let hh = hash_mls(src, p, mls, hash_log);
                    tables.chain[p & chain_mask] = tables.get_h(hh).map(|x| x as u32).unwrap_or(0);
                    tables.put_h(hh, p);
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
    lits.extend_from_slice(&src[anchor..block_end]);
    let span = (block_end - block_start).max(1) as f32;
    tables.last_search_per_byte = searches as f32 / span;
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
#[inline]
fn search_attempts(params: CompressionParameters) -> usize {
    let v = SEARCH_LOG_ARM.load(core::sync::atomic::Ordering::Relaxed);
    let base = params.search_log.min(12) as i32;
    let d = if v == 0 { 0 } else { v as i32 - 8 };
    1usize << base.saturating_add(d).clamp(0, 12)
}

#[allow(clippy::too_many_arguments)]
fn bt_find_best(
    src: &[u8],
    ip: usize,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let hash_log = params.hash_log;
    let bt_log = params.chain_log.min(24).saturating_sub(1).max(1);
    let bt_mask = (1usize << bt_log) - 1;
    if tables.chain.len() < 2 {
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
    let attempts = search_attempts(params);
    // P0/gg-matchfind: work counter -- see `chain_find_best`.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut best_ml = 0usize;
    let mut best_m = 0usize;
    for _ in 0..attempts {
        let Some(m) = match_idx else {
            tables.chain[smaller] = 0;
            tables.chain[larger] = 0;
            break;
        };
        if m >= ip || ip - m > window {
            tables.chain[smaller] = 0;
            tables.chain[larger] = 0;
            break;
        }
        if m < block_start.saturating_sub(window).max(tables.frame_start) {
            break;
        }
        let bt_idx = (m & bt_mask) << 1;
        if bt_idx + 1 >= tables.chain.len() {
            break;
        }
        if COUNT {
            probes += 1;
        }
        let ml = count_match(src, m, ip, block_end);
        if ml >= mls && ml > best_ml && offset_ok(ip - m, window) && m >= tables.frame_start {
            best_ml = ml;
            best_m = m;
        }
        let mb = src.get(m + ml).copied().unwrap_or(0);
        let ib = src.get(ip + ml).copied().unwrap_or(0);
        if mb < ib {
            tables.chain[smaller] = m as u32;
            smaller = bt_idx + 1;
            let v = tables.chain[bt_idx + 1];
            match_idx = if v == 0 { None } else { Some(v as usize) };
        } else {
            tables.chain[larger] = m as u32;
            larger = bt_idx;
            let v = tables.chain[bt_idx];
            match_idx = if v == 0 { None } else { Some(v as usize) };
        }
        if smaller >= tables.chain.len() || larger >= tables.chain.len() {
            break;
        }
    }
    if COUNT {
        crate::prof::note_probes(probes);
    }
    (best_m, best_ml)
}

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
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
    let mut anchor = block_start;
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        lits.extend_from_slice(&src[block_start..block_end]);
        return (seqs, lits);
    }
    // BRICK 73: repcode-1 in BtLazy2 (L13-L14) -- the last finder without it.
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
                lits.extend_from_slice(&src[anchor..mstart]);
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
        let (mut best_m, mut best_ml) =
            bt_find_best(src, ip, block_start, block_end, window, mls, params, tables);
        let mut best_ip = ip;
        let mut look_hi = ip;
        if best_ml >= mls {
            for d in 1..=depth {
                let ip2 = ip + d;
                if ip2 > ilimit {
                    break;
                }
                look_hi = ip2;
                let (m, ml) = bt_find_best(
                    src,
                    ip2,
                    block_start,
                    block_end,
                    window,
                    mls,
                    params,
                    tables,
                );
                if ml > best_ml {
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
            while s > anchor && mm > tables.frame_start && src[s - 1] == src[mm - 1] {
                s -= 1;
                mm -= 1;
                n += 1;
            }
            lits.extend_from_slice(&src[anchor..s]);
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
            if lazy_fill_enabled() {
                // B2: the look-ahead already inserted up to `look_hi`.
                let mut p = (best_ip + 1).max(look_hi + 1);
                while p < end && p <= ilimit {
                    let _ =
                        bt_find_best(src, p, block_start, block_end, window, mls, params, tables);
                    p += 1;
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
    lits.extend_from_slice(&src[anchor..block_end]);
    // Probes reported by `bt_find_best`.
    note_finder_work(cfg!(feature = "profile"), 0, seqs.len() as u64, &seqs, &lits);
    (seqs, lits)
}

fn find_opt(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    let n = block_end - block_start;
    let mls = params.min_match.max(3) as usize;
    if n < 8 {
        return (Vec::new(), src[block_start..block_end].to_vec());
    }
    let inf = u32::MAX / 4;
    let mut price = vec![inf; n + 1];
    let mut prev = vec![0usize; n + 1];
    let mut is_match = vec![false; n + 1];
    let mut match_off = vec![0u32; n + 1];
    let mut match_ml = vec![0u32; n + 1];
    price[0] = 0;
    // BRICK 75: offer the REPCODE as a DP candidate (find_opt was the last
    // finder without repcode search).
    //
    // Correctness is the emit path's job: we record a candidate at byte
    // DISTANCE `rep1`, and `offset_value_for` turns that into a repcode code
    // using the real rep state at emit time. What the DP must get right is
    // WHERE the match starts and what it costs.
    let rep1 = reps[0] as usize;
    let lowest_rep = block_start.saturating_sub(window).max(tables.frame_start);
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
    const OPT_SKIP_FLOOR: usize = 1024;
    let sufficient_len = if params.target_length == 0 {
        usize::MAX
    } else {
        (params.target_length as usize).max(OPT_SKIP_FLOOR)
    };
    let mut i = 0usize;
    while i < n {
        if price[i] >= inf {
            i += 1;
            continue;
        }
        let np = price[i].saturating_add(6);
        if np < price[i + 1] {
            price[i + 1] = np;
            prev[i + 1] = i;
            is_match[i + 1] = false;
        }
        let ip = block_start + i;
        if ip + 8 > block_end {
            i += 1;
            continue;
        }
        // `try_rep1` matches at ip+1: a rep0 code requires litlen >= 1. So the DP
        // edge must ORIGINATE AT i+1 (after that literal), not at i. Basing it on
        // `price[i]` was the first attempt and it emitted every sequence with
        // litlen off by one -- an invalid stream that 36 conformance cases caught.
        // `price[i + 1]` is final here: the literal edge above already set it.
        if i + 1 <= n && price[i + 1] < inf {
            if let Some(rml) = try_rep1(src, ip, rep1, lowest_rep, block_end) {
                let j = i + 1 + rml;
                if j <= n {
                    let np = price[i + 1].saturating_add(rep_cost);
                    if np < price[j] {
                        price[j] = np;
                        prev[j] = i + 1;
                        is_match[j] = true;
                        match_off[j] = rep1 as u32;
                        match_ml[j] = rml as u32;
                    }
                }
            }
        }
        let (bm, bml) = bt_find_best(src, ip, block_start, block_end, window, mls, params, tables);
        if bml < mls {
            i += 1;
            continue;
        }
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
        let mut len = mls;
        loop {
            let j = i + len;
            if j > n {
                break;
            }
            let np = price[i].saturating_add(seq_cost);
            if np < price[j] {
                price[j] = np;
                prev[j] = i;
                is_match[j] = true;
                match_off[j] = (ip - bm) as u32;
                match_ml[j] = len as u32;
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
            i += bml;
            continue;
        }
        i += 1;
    }
    if price[n] >= inf {
        return find_bt_lazy(src, block_start, block_end, window, params, tables, 2, reps);
    }
    let mut ops: Vec<(usize, usize, bool, u32, u32)> = Vec::new();
    let mut i = n;
    while i > 0 {
        let p = prev[i];
        ops.push((p, i, is_match[i], match_off[i], match_ml[i]));
        i = p;
    }
    ops.reverse();
    let mut seqs = Vec::new();
    let mut lits = Vec::new();
    let mut anchor = 0usize;
    for &(start, end, matched, off, ml) in &ops {
        if matched {
            lits.extend_from_slice(&src[block_start + anchor..block_start + start]);
            seqs.push(Seq {
                litlen: (start - anchor) as u32,
                matchlen: ml,
                offset: off,
            });
            let _ = end;
            anchor = start + ml as usize;
        }
    }
    lits.extend_from_slice(&src[block_start + anchor..block_end]);
    // Probes reported by `bt_find_best`, which the DP calls per position.
    note_finder_work(cfg!(feature = "profile"), 0, seqs.len() as u64, &seqs, &lits);
    (seqs, lits)
}

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
    if mls >= 4 {
        if load_u32le(src, m) != load_u32le(src, ip) {
            return false;
        }
        return mls == 4 || src[m + 4..m + mls] == src[ip + 4..ip + mls];
    }
    src[m..m + mls] == src[ip..ip + mls]
}

fn count_match(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    if ip >= limit || m >= src.len() || ip >= src.len() {
        return 0;
    }
    let max = (limit - ip).min(src.len() - m).min(src.len() - ip);
    if max == 0 {
        return 0;
    }
    // Slice once so the inner loops see a proven length (LLVM drops per-word bounds).
    let a = &src[m..m + max];
    let b = &src[ip..ip + max];
    crate::simd::count_eq_len(a, b)
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

#[inline(always)]
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

fn offset_ok(offset: usize, window: usize) -> bool {
    offset > 0 && offset <= window
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
                let (llc, _, _) = ll_code(s.litlen);
                let (mlc, _, _) = ml_code(s.matchlen);
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
                for spare in [0usize, 1, 15, 16, 1024] {
                    let mut fast = Vec::with_capacity(4 + spare);
                    fast.extend_from_slice(b"HEAD");
                    let mut want = fast.clone();
                    want.extend_from_slice(&src[from..from + n]);
                    super::push_literals(&mut fast, &src, from, from + n, true);
                    assert_eq!(fast, want, "from={from} n={n} spare={spare}");
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
                        let (llc, _, _) = ll_code(s.litlen);
                        let (mlc, _, _) = ml_code(s.matchlen);
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
    TAG_ARM.store(0, Ordering::Relaxed);
    PIPE_ARM.store(0, Ordering::Relaxed);
    REP1_ENABLED_ARM.store(0, Ordering::Relaxed);
    LAZY_FILL_ENABLED_ARM.store(0, Ordering::Relaxed);
    FAST_LAZY_ARM.store(0, Ordering::Relaxed);
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
