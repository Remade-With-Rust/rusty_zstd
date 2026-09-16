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
fn params_with_history(
    level: i32,
    src_len: usize,
    hist_len: usize,
) -> Result<CompressionParameters, Error> {
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
pub(crate) struct Seq {
    litlen: u32,
    matchlen: u32,
    offset: u32,
}

// Clone is MANUAL, and it is the GATE 18 probe's clone: the one caller
// (`let mut probe = tables.clone()` in `find_sequences`) needs the SEARCH
// state -- boards, rows, and every dispatch signal -- from an identical
// starting point, and then discards the copy. The nine scratch buffers'
// CONTENTS are dead at every read site (each is take-and-CLEARED or
// reset/ensure'd before use), so `derive(Clone)`'s deep copy of them was up
// to ~300 KB of memcpy per probed block to reproduce bytes nobody can read.
// They clone as empty Vecs; the probe's finder allocates its own on first
// use, exactly as a fresh encoder would. Byte-identical by construction.
/// The match tables: the hash/chain/tag arrays and every accessor over them. Lives in `encode/tables.rs`.
mod tables;
pub(crate) use tables::*;

/// ALLOC-7: a retained seq table that may be the process-constant Predefined
/// one, held BY REFERENCE.
///
/// `EntropyState` stored `Option<FseCTable>`, so selecting Predefined mode
/// cloned the cached RFC-constant table (two or three heap allocations) purely
/// to store a copy of something already `&'static`. After ALLOC-5 removed the
/// Repeat-path clone this was the single largest remaining allocation site in
/// the encoder.
#[derive(Clone)]
pub(crate) enum RetainedTable {
    Static(&'static FseCTable),
    Own(FseCTable),
}

impl core::ops::Deref for RetainedTable {
    type Target = FseCTable;
    #[inline(always)]
    fn deref(&self) -> &FseCTable {
        match self {
            RetainedTable::Static(t) => t,
            RetainedTable::Own(t) => t,
        }
    }
}

/// Huffman / FSE tables carried across compressed blocks (Repeat / Treeless).
///
/// ALLOC-8: the tables are behind `Arc` so that CLONING this state is four
/// refcount bumps rather than eight-to-twelve heap allocations.
///
/// `encode_block` takes a speculative snapshot of this state before trying the
/// compressed encoding, so it can roll back if Raw or RLE wins. Measured: the
/// rollback fires **0 times out of 707-876 saves, at every level from L1 to
/// L19** -- every snapshot was a full deep copy of three FSE tables and a
/// Huffman table, and every one was discarded.
///
/// `Arc` is sound here because nothing mutates a table in place: every use is a
/// read (`as_deref`) or a whole-value assignment, so there is never a writer to
/// share with. `Arc` rather than `Rc` keeps `Compressor` `Send`.
#[derive(Clone, Default)]
pub(crate) struct EntropyState {
    huff: Option<alloc::sync::Arc<HuffCTable>>,
    ll: Option<alloc::sync::Arc<RetainedTable>>,
    of: Option<alloc::sync::Arc<RetainedTable>>,
    ml: Option<alloc::sync::Arc<RetainedTable>>,
}

/// One outlined copy of the dictionary-seed table wrap. See
/// `EntropyState::seed_from_dict`.
#[inline(never)]
fn retain_ctable(t: &fse::FseCTable) -> alloc::sync::Arc<RetainedTable> {
    alloc::sync::Arc::new(RetainedTable::Own(t.clone()))
}

impl EntropyState {
    pub(crate) fn seed_from_dict(&mut self, e: &crate::dict::DictEntropy) {
        // C9: the encode-side mirror of C8. `Arc::new(RetainedTable::Own(
        // t.clone()))` -- an allocation, an enum construction and a table
        // clone -- was expanded at all three sites. One outlined helper leaves
        // that body once. Runs once per dictionary.
        self.huff = Some(alloc::sync::Arc::new(e.huff_c.clone()));
        self.ll = Some(retain_ctable(&e.ll_c));
        self.of = Some(retain_ctable(&e.of_c));
        self.ml = Some(retain_ctable(&e.ml_c));
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
        // The one-shot path KNOWS the source length, which is what the row
        // finder's AUTO band is measured against.
        MatchTables::new_sized(params, Some(src.len() as u64))
    };
    // T1: DFast's short-table rejection tag, packed into the slot it already
    // loads. Decided against the real buffer length, so the 24-bit bound is
    // proven per frame rather than assumed.
    tables.enable_packed_tags(
        (params.strategy == Strategy::DFast && dfast_tag_enabled())
            || (params.strategy == Strategy::Fast && tag_alloc_enabled() && fast_pack_enabled()),
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
    if let Some(kb) = crate::env_knob_parse::<usize>("RZSTD_BLOCK_KB") {
        if kb > 0 {
            block_max = block_max.min(kb * 1024);
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
                let keep = crate::env_knob_is1("RZSTD_PRIME_BT");
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
pub static PRIME_ITERS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// E4 ceiling probe: `[calls, positions hashed]` in the post-match fill helpers.
/// E4 proposes batching these into a vector tile; a tile needs positions.
/// N9 probe: rebuilds of the RFC-constant default FSE ctable in `select_seq_table`.
pub static N9_BASIC: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// ALLOC-8 probe: `[speculative EntropyState saves, saves actually ROLLED BACK]`.
/// The save clones 3 FSE tables + a Huffman table per block; if the rollback
/// almost never fires, the clone is almost pure waste.
pub static ENT_SAVE: [crate::census64::AtomicU64; 2] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
/// Read and clear the ALLOC-8 probe.
pub fn take_ent_save() -> [u64; 2] {
    use core::sync::atomic::Ordering;
    [
        ENT_SAVE[0].swap(0, Ordering::Relaxed),
        ENT_SAVE[1].swap(0, Ordering::Relaxed),
    ]
}
/// Read and clear the N9 probe.
pub fn take_n9_basic() -> u64 {
    N9_BASIC.swap(0, core::sync::atomic::Ordering::Relaxed)
}

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
        let n: usize = crate::env_knob_parse("RZSTD_PRIME_STRIDE")
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
    PREFIX_BOUND_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
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
    PRIME_BT_EXTENT_ARM
        .load(core::sync::atomic::Ordering::Relaxed)
        .max(1) as usize
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
                let on = crate::env_knob_is1("RZSTD_PRIME_BT_TREE");
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
static PRIME_BT_DEPTH_ARM: core::sync::atomic::AtomicU32 =
    core::sync::atomic::AtomicU32::new(u32::MAX);

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
        let d: u32 =
            crate::env_knob_parse("RZSTD_PRIME_BT_DEPTH").unwrap_or(PRIME_BT_DEPTH_DEFAULT);
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

pub static G5_RPREV: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static G5_RPREV_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static G5_DRIFTSUM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static G5_DRIFT_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// GATE 5 inputs, as block means: `(mean r_prev, mean drift)`.
pub fn take_g5_inputs() -> (f64, f64) {
    use core::sync::atomic::Ordering::Relaxed;
    let a = G5_RPREV_N.swap(0, Relaxed).max(1) as f64;
    let b = G5_DRIFT_N.swap(0, Relaxed).max(1) as f64;
    (
        // `/ 10000.0 / a` is two divisions; fold the constant into the
        // denominator for one.
        G5_RPREV.swap(0, Relaxed) as f64 / (10000.0 * a),
        G5_DRIFTSUM.swap(0, Relaxed) as f64 / (10000.0 * b),
    )
}

pub static G5_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static G5_HIT_RATIO: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static G5_HIT_DRIFT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// GATE 5 coverage: `(calls, raw-escape fires, drift fires)`.
pub fn take_g5() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        G5_CALLS.swap(0, Relaxed),
        G5_HIT_RATIO.swap(0, Relaxed),
        G5_HIT_DRIFT.swap(0, Relaxed),
    )
}

/// 4.77: the Fast ladder is OFF above this input length. Crossover measured
/// between 2 and 4 MiB (total -0.0274% -> +0.0692%); 2 MiB keeps every cell that
/// still earns and drops every cell that regressed.
const G5_FAST_MAX_LEN: usize = 2 << 20;

static G5_FAST_LEN_A: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

#[inline(always)]
fn g5_fast_max_len() -> usize {
    let v = G5_FAST_LEN_A.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        G5_FAST_MAX_LEN
    } else {
        v
    }
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
    if v == u32::MAX {
        G5_TINY_MAX
    } else {
        f32::from_bits(v)
    }
}

#[inline(always)]
fn g5_band() -> usize {
    let v = G5_BAND_A.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        G5_BAND
    } else {
        v
    }
}

/// Bench arms for the 4.76 band. `set_g5_band_arm(usize::MAX)` disables the band
/// (it can then never bind), restoring the pre-4.76 behaviour exactly.
pub fn set_g5_tiny_arm(v: f32) {
    G5_TINY_A.store(
        if v.is_nan() { u32::MAX } else { v.to_bits() },
        core::sync::atomic::Ordering::Relaxed,
    );
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
    if b == u32::MAX {
        G5_REP_MIN_OPT
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_ratio_min_opt() -> f32 {
    let b = G5_RATIO_O.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_RATIO_MIN_OPT
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_drift_min_opt() -> f32 {
    let b = G5_DRIFT_O.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_DRIFT_MIN_OPT
    } else {
        f32::from_bits(b)
    }
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
    if b == u32::MAX {
        G5_REP_MIN_FAST
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_ratio_min_fast() -> f32 {
    let b = G5_RATIO_F.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_RATIO_MIN_FAST
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_drift_min_fast() -> f32 {
    let b = G5_DRIFT_F.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_DRIFT_MIN_FAST
    } else {
        f32::from_bits(b)
    }
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
    if b == u32::MAX {
        G5_REP_MIN
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_ratio_min() -> f32 {
    let b = G5_RATIO.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_RATIO_MIN
    } else {
        f32::from_bits(b)
    }
}
#[inline]
fn g5_drift_min() -> f32 {
    let b = G5_DRIFT.load(core::sync::atomic::Ordering::Relaxed);
    if b == u32::MAX {
        G5_DRIFT_MIN
    } else {
        f32::from_bits(b)
    }
}

/// OUTLINED. A 223-line dictionary-priming walk that was `#[inline(always)]`
/// at five shipping call sites -- `encode_oneshot`, two in `stream.rs`'s
/// compressor setup, and the reset path. Every one of them runs ONCE PER
/// DICTIONARY or once per stream reset, never per block, so the call is free
/// and the five stamps were not.
#[inline(never)]
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
    // Hoisted per call: see the tag accessors' `packed` doc.
    let packed = tables.pack_tags;
    let stag_live = !tables.tags.is_empty();
    let ltag_live = !tables.ltags.is_empty();
    // BRICK 99 (K18): the contract's bound (brick 35) -- `wide_hash` below is
    // then provably false and the tree kernel's 8-byte-hash arm is gone.
    let mls = params.min_match.clamp(3, 7) as usize;
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
    // Counted ONCE per call from the loop bounds, never per position: this
    // walk runs tens of millions of times per slide and a per-iteration tap
    // would be the instrument dominating what it measures.
    crate::copies::add(
        crate::copies::C_PRIME_INSERT,
        ilimit.saturating_sub(from) / stride.max(1),
    );
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
            CompressionParameters {
                search_log: d,
                ..params
            }
        };
        let prime_attempts = bt_depth_apply(search_attempts(pparams), pparams, tables.opt_rep_rate);
        let btf = bt_resolve_ins(tables.hash_log, pparams.chain_log.min(24));
        // BRICK 34: the block's tree geometry, once.
        let (bt_mask, bt_ok) = bt_geom(pparams.chain_log.min(24), tables.chain.len());
        let prime_ctx = BtCtx {
            src,
            block_start: payload_off,
            block_end: payload_off,
            window,
            mls,
            attempts: prime_attempts,
            chain_log: pparams.chain_log.min(24),
            bt_lowest: payload_off.saturating_sub(window).max(tables.frame_start),
            chain_len: tables.chain.len(),
            wide_hash: mls >= 8,
            bt_mask,
            bt_shift32: 32u32.saturating_sub(tables.hash_log.min(32)),
            bt_shift64: 64u32.saturating_sub(tables.hash_log.min(32)),
            bt_ok,
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
            btf(&prime_ctx, p, tables);
            iters += 1;
            p += stride;
        }
        #[cfg(feature = "profile")]
        PRIME_ITERS.fetch_add(iters, core::sync::atomic::Ordering::Relaxed);
        #[cfg(not(feature = "profile"))]
        let _ = iters;
        return;
    }
    // REFUTED 2026-09-09, recorded so it is not retried: OUTLINING the
    // chain-strategy arm of this loop into its own `#[inline(never)]` frame
    // with the invariants (`smask`, `cp`, `ca`, `chain_wide`, the plain-head
    // test) hoisted to locals and the two checked indexings routed through
    // the mask-proven accessors. It reads like bricks 10/12 (the fill loops),
    // and it loses here: inside this large frame LLVM had UNSWITCHED the two
    // shipping arms into their own loops (dfast 42 instrs / 12 reloads per
    // position, lazy 50 / 15); the outlined function is one loop with eight
    // invariant tests per position that LLVM no longer unswitches (the
    // whole function IS the loop, so the size budget refuses eight
    // conditions), and the same arms measured 46 / 9 and 57 / 13 --
    // instructions UP, reloads down, 964 -> 671 + 318 static. Two counters
    // disagreeing in sign is not a win. And the path is not on the plain
    // `compress()` route at all (`payload_off == 0` returns above); it runs
    // with a dictionary/prefix and on streaming slides only. Left as-is.
    // BRICK 74: the empty-head link for this producer (see `set_null_tag`).
    if !is_fast {
        tables.set_null_tag(chain_null_tag(src, mls));
    }
    let mut p = from;
    while p <= ilimit && p + 8 <= src.len() {
        if is_fast {
            // ffanat hash-width: MUST mirror `fast_hash_tag` exactly, or every
            // primed slot mismatches the finder's keys -- the -59.3% priming
            // poison this function has already been bitten by once.
            let fhp = fast_hash_spec(mls, hash_log);
            let (h, tag) = fast_hash_tag::<true>(src, p, fhp.wide, fhp.mask, fhp.shift);
            tables.store_fast(h, p, tag, packed);
        } else {
            // Chain-tag frames prime in the finder's own format (packed or
            // array) -- the -59.3% priming-poison rule, third application.
            if tables.chain_pack || !tables.ctags.is_empty() {
                let cp = tables.chain_pack;
                let ca = !tables.ctags.is_empty();
                let smask = if mls >= 8 {
                    u64::MAX
                } else {
                    (1u64 << (8 * mls)) - 1
                };
                let (hh, gt) = if tables.chain_wide {
                    hash_wide_link_tag_b(src, p, 64u32.saturating_sub(hash_log.min(32)), smask, mls)
                } else {
                    hash4_link_tag_b(src, p, 32u32.saturating_sub(hash_log.min(32)), mls)
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
                // REFUTED, recorded (do not retry): routing this through
                // `chain_masked_set` -- whose contract this call satisfies, and
                // which every hot finder uses -- retires the bounds check but
                // measures **guards -1, instructions +1**. Two deterministic
                // counters disagreeing in sign is not a win, and `prime_tables`
                // runs once per block, so nothing here justifies trading a
                // plainly-safe index for an `unsafe` accessor. Left as-is.
                tables.chain[p & chain_mask] = tables
                    .get_h(h)
                    .map(|x| x as u32)
                    .unwrap_or(tables.null_link);
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
                let smask = if sk == 8 {
                    u64::MAX
                } else {
                    (1u64 << (8 * sk)) - 1
                };
                let tv = (load_u64le(src, p) & smask).wrapping_mul(FAST_HASH_PRIME64);
                let g = (tv >> 56) as u8; // BRICK 52: see `hash4_tag_from`
                tables.put_h_tag(h, p, g, packed, stag_live);
                // 1a: prime the LONG tag too, or the filter rejects every
                // primed long slot -- the exact -59.3% priming-poison class
                // the short table was bitten by.
                if do_long {
                    let hl = hash8(src, p, hash_log);
                    tables.put_hl_tag(hl, p, g, packed, ltag_live);
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

/// Block encoding: sequences and literals into a compressed block. Lives in `encode/block.rs`.
mod block;
pub use block::*;

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
    // TWIN RETIRED. This carried a wholesale BMI2 twin justified by "the
    // per-block section packing carried 34 variable shifts of its own,
    // outside every finer-grained twin". That premise has expired: the
    // packing moved into `write_sequences` and `write_literals`, which grew
    // their OWN twins, and every finder now runs its own `has_bmi2()`
    // dispatch. Measured on the emitted asm before removing it -- the twin
    // contained 2 `shrx` against 642 instructions of duplicated
    // body. It was buying single-digit shift encodings for four figures of
    // I-cache.
    find_sequences_inner(
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        ldm,
        ldm_p,
        reps,
    )
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
    // TWIN RETIRED. This carried a wholesale BMI2 twin justified by "the
    // per-block section packing carried 34 variable shifts of its own,
    // outside every finer-grained twin". That premise has expired: the
    // packing moved into `write_sequences` and `write_literals`, which grew
    // their OWN twins, and every finder now runs its own `has_bmi2()`
    // dispatch. Measured on the emitted asm before removing it -- the twin
    // contained 3 `shrx` against 254 instructions of duplicated
    // body. It was buying single-digit shift encodings for four figures of
    // I-cache.
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
        Strategy::BtLazy2 => {
            find_bt_lazy(src, block_start, block_end, window, params, tables, 2, reps)
        }
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
                let on = crate::env_knob_not0("RZSTD_FASTLAZY", true);
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
        let v: f32 = crate::env_knob_parse("RZSTD_FASTLAZY_T").unwrap_or(0.7);
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
        let v: f32 = crate::env_knob_parse("RZSTD_REPLEN").unwrap_or(1.0);
        REPLEN_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1.0
}

static REPLEN_CACHE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
        let v: f32 = crate::env_knob_parse("RZSTD_REP_DECAY").unwrap_or(0.0);
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
static REPMIN_OVR: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
            c = crate::env_knob_parse::<f32>("RZSTD_REPMIN")
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
        crate::env_knob_parse("RZSTD_REPMIN").unwrap_or(REP_YIELD_MIN_DEFAULT)
    }
    #[cfg(not(feature = "std"))]
    REP_YIELD_MIN_DEFAULT
}

/// The Fast ladder (L1-L2, and the Gate 1 dispatch): one hash table, one probe. Lives in `encode/fast.rs`.
mod fast;
pub(crate) use fast::*;

/// The tuning ARMS and census COUNTERS: bench hooks, env knobs, instruments. Lives in `encode/knobs.rs`.
mod knobs;
pub use knobs::*;

/// The Fast ladder's after-match end-fill on the LOCAL table -- same semantics
/// and same instruments as `fill_hash_after_match`, writing through the shared
/// `fast_slot_store` rule.
// W2: this carried BOTH `#[inline]` and `#[inline(always)]`.
#[inline(always)]
fn fill_fast_after_match(
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
    // W10: hoisted `!tags.is_empty()` -- see `fast_slot_swap`.
    tags_live: bool,
    // W1: this was `dfast_fill_ends()` INSIDE the helper -- an arm atomic load
    // and its match, per MATCH, on the Fast ladder. `find_dfast_impl` has
    // hoisted the same read per block since the brick-79 sweep; the Fast
    // ladder's own fill never did.
    ends: (bool, bool),
) {
    let (do_a, do_b) = ends;
    let mut n = 0u64;
    // W4/W5: `match_end` is `match_ip + n` with `n >= mls >= 4`, so the
    // `>= 2` test is dead at every call -- and `match_ip` is an index into
    // `src`, so the `saturating_add` guarding it is a cmov the encoder can
    // never take. Both ran per MATCH, in all three fill helpers.
    debug_assert!(match_ip < usize::MAX - 2);
    let a = match_ip + 2;
    if do_a && a <= ilimit {
        let (h, g) = fast_hash_tag::<true>(src, a, f_wide, f_mask, f_shift);
        fast_slot_store(hash, tags, pack, tags_live, h, a, g);
        n += 1;
    }
    debug_assert!(match_end >= 2);
    if do_b {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let (h, g) = fast_hash_tag::<true>(src, b, f_wide, f_mask, f_shift);
            fast_slot_store(hash, tags, pack, tags_live, h, b, g);
            n += 1;
        }
    }
    #[cfg(feature = "profile")]
    DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
    crate::prof::note_hash_fill(n);
}

/// W3: `mls` was a DEAD parameter -- the body's first statement was
/// `let _ = mls;`. It was set up at every one of the three call sites, on
/// every match, in all 280 copies.
#[inline(never)]
fn emit_fast_seq_plain(
    ctx: &FastEmitCtx,
    hash: &mut [u32],
    tags: &mut [u8],
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
) -> usize {
    emit_fast_seq_body(ctx, hash, tags, seqs, lits, anchor, found_ip, m, ml)
}

/// The ISA twin. See `FastEmitCtx` -- without this the BMI2 `find_fast_impl`
/// twins would call a baseline emitter.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
#[inline(never)]
unsafe fn emit_fast_seq_bmi2(
    ctx: &FastEmitCtx,
    hash: &mut [u32],
    tags: &mut [u8],
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
) -> usize {
    emit_fast_seq_body(ctx, hash, tags, seqs, lits, anchor, found_ip, m, ml)
}

/// `BMI2` is threaded from the wrapper that already made the CPUID decision
/// for the whole block, so this selection is a compile-time fold, not a
/// per-match branch.
#[inline(always)]
/// BRICK 9: `packed` was a DEAD parameter all the way down -- the body's copy
/// was `_packed`, the layout choice travels in `ctx.pack` -- and it was the
/// FIRST argument, so it held a register while a live scalar went to the
/// stack at every one of the three emit sites, on every sequence at L1/L2.
/// Same defect W3 removed for `mls`; this one survived because it was passed
/// through two dispatch layers before reaching the body that ignored it.
fn emit_fast_seq<const BMI2: bool>(
    ctx: &FastEmitCtx,
    hash: &mut [u32],
    tags: &mut [u8],
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
) -> usize {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if BMI2 {
        crate::kreach::hit(crate::kreach::K_EMIT_FAST_SEQ);
        // SAFETY: `BMI2` is only ever `true` inside `find_fast_impl_bmi2`,
        // which the plain wrapper reached under a `has_bmi2()` CPUID guard.
        #[allow(unsafe_code)]
        return unsafe { emit_fast_seq_bmi2(ctx, hash, tags, seqs, lits, anchor, found_ip, m, ml) };
    }
    crate::kreach::miss(crate::kreach::K_EMIT_FAST_SEQ);
    emit_fast_seq_plain(ctx, hash, tags, seqs, lits, anchor, found_ip, m, ml)
}

#[inline(always)]
fn emit_fast_seq_body(
    ctx: &FastEmitCtx,
    hash: &mut [u32],
    tags: &mut [u8],
    seqs: &mut Vec<Seq>,
    lits: &mut Vec<u8>,
    anchor: usize,
    found_ip: usize,
    m: usize,
    ml: usize,
) -> usize {
    let &FastEmitCtx {
        src,
        pack,
        f_wide,
        f_mask,
        f_shift,
        ilimit,
        frame_start,
        w,
        tags_live,
        ends,
    } = ctx;
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
    fill_fast_after_match(
        hash, tags, pack, f_wide, f_mask, f_shift, src, found_ip, end, ilimit, tags_live, ends,
    );
    end
}

/// FUSED DFast fill -- both tables, ONE walk. Replaces the
/// `fill_hash_after_match` + `fill_hash_long_after_match` pair at DFast's
/// commit point.
///
/// The two tables index the SAME byte at position `b` (`match_end - 2`) always,
/// and the same byte at position `a` whenever the anchors coincide -- which is
/// every match except the ones where the next-long probe won. `hash4_tag_mls`
/// yields `(hash, tag)`: the short store wants both, the long store wants only
/// the tag, and it was recomputing the pair to get it. Computing it once and
/// handing the tag to both stores is the whole idea.
///
/// `short_ip` and `long_ip` are passed separately rather than assumed equal:
/// `dfast_fill_anchor_c` defaults OFF, so the long table anchors on the
/// PRE-probe `ip` while the short anchors on the committed position. They
/// differ by at most 1 and only when the next-long probe won.
#[inline]
#[allow(clippy::too_many_arguments)]
fn fill_dfast_after_match(
    tables: &mut MatchTables,
    src: &[u8],
    short_ip: usize,
    long_ip: usize,
    match_end: usize,
    ends: (bool, bool),
    smask: u64,
    hash_shift: u32,
    lshift: u32,
    ilimit: usize,
    // BLOCK-HOISTED. These were read through `&mut MatchTables` inside each
    // helper -- six field reads per match across the pair. `pack_tags` is a
    // block constant and neither tag array is resized or cleared inside the
    // match loop (audited), so their emptiness is fixed for the block.
    packed: bool,
    stag_live: bool,
    ltag_live: bool,
) {
    // See the pair's note: `wanted` asks whether a tag is worth COMPUTING
    // (packed frames carry it in the slot), `live` asks whether the array
    // exists. They differ exactly on the packed case.
    let ltag_wanted = packed || ltag_live;
    let (do_a, do_b) = ends;
    // Pattern A: `n`'s only readers are the `profile` atomic below and
    // `note_hash_fill`, which is an empty stub in a shipping build. Folding it
    // to a unit there removes four increments per match.
    #[cfg(feature = "profile")]
    let mut n = 0u64;
    macro_rules! bump {
        () => {
            #[cfg(feature = "profile")]
            {
                n += 1;
            }
        };
    }

    debug_assert!(short_ip < usize::MAX - 2 && long_ip < usize::MAX - 2);
    let sa = short_ip + 2;
    let la = long_ip + 2;

    // Position `a` keeps the pair's independent shape: sharing the tag across
    // the two anchors needs an `la == sa` test, and MEASURED +31 instructions
    // -- the branch costs more than the hash it saves on a path LLVM had as
    // straight-line code. Recorded so it is not retried.
    if do_a {
        if sa <= ilimit {
            let (h, g) = hash4_tag_mls(src, sa, hash_shift, smask);
            tables.put_h_tag(h, sa, g, packed, stag_live);
            bump!();
        }
        if la <= ilimit {
            let g = if ltag_wanted {
                hash4_tag_mls(src, la, hash_shift, smask).1
            } else {
                0
            };
            tables.put_hl_tag(hash8_shift(src, la, lshift), la, g, packed, ltag_live);
            bump!();
        }
    }

    debug_assert!(match_end >= 2);
    if do_b {
        // Computed ONCE where the pair computed it twice, along with its bound.
        let b = match_end - 2;
        if b <= ilimit {
            let want_s = b != sa;
            let want_l = b != la;
            if want_s || want_l {
                // ONE `hash4_tag_mls` serves both stores: `h` for the short
                // slot, `g` for whichever tags are live.
                // THE win: ONE `hash4_tag_mls` where the pair ran two. Both
                // tables index `match_end - 2` identically, always -- the short
                // store takes `h` and `g`, the long store takes `g` and its own
                // 8-byte hash. Unconditional because at least one store is
                // happening (`want_s || want_l`), so the pair is never wasted.
                // ONE `load_u64le` feeds BOTH mixes. `hash4_tag_mls` and
                // `hash8_shift` each began with the same load at the same
                // position; splitting the load out (see `hash4_tag_from`)
                // removes the duplicate.
                let v = load_u64le(src, b);
                let (h, g) = hash4_tag_from(v, hash_shift, smask);
                if want_s {
                    tables.put_h_tag(h, b, g, packed, stag_live);
                    bump!();
                }
                if want_l {
                    tables.put_hl_tag(hash8_from(v, lshift), b, g, packed, ltag_live);
                    bump!();
                }
            }
        }
    }

    #[cfg(feature = "profile")]
    {
        DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
        crate::prof::note_hash_fill(n);
    }
}

/// C `zstd_fast.c` after a match: insert hash(start+2) and hash(end-2) only.
/// Filling every byte of a long match was ~src_len hash writes on repeating text.
///
/// STILL USED by the L1 Fast ladder and DFast's interior back-fill stride; the
/// DFast COMMIT point now goes through `fill_dfast_after_match`.
/// SUPERSEDED by `fill_dfast_after_match`, which fuses this with its
/// sibling and shares the position-`b` hash the two recomputed. Kept as
/// the reference shape -- the convention this crate uses for
/// `encode_4_streams` and `segment_histograms`. LLVM drops it.
#[allow(dead_code)]
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
    // Hoisted per call: see the tag accessors' `packed` doc.
    let packed = tables.pack_tags;
    let stag_live = !tables.tags.is_empty();
    // W18: `ltag_live` was computed here and never read -- an `is_empty()`
    // read through the `&mut MatchTables`, per MATCH, on the DFast ladder.
    let (do_a, do_b) = ends;
    let mut n = 0u64;
    // W4/W5: `match_end` is `match_ip + n` with `n >= mls >= 4`, so the
    // `>= 2` test is dead at every call -- and `match_ip` is an index into
    // `src`, so the `saturating_add` guarding it is a cmov the encoder can
    // never take. Both ran per MATCH, in all three fill helpers.
    debug_assert!(match_ip < usize::MAX - 2);
    let a = match_ip + 2;
    if do_a && a <= ilimit {
        let (h, g) = hash4_tag_mls(src, a, hash_shift, smask);
        // T1: this helper runs after EVERY match on the DFast path too, so it
        // must write the short table in whatever representation the frame is
        // using. Writing it unpacked while the reader is packed decodes the tag
        // bits as part of the position -- which is exactly what it did, and it
        // moved output on 12 of 18 corpora.
        // W10: this re-read `pack_tags` from the struct one line after
        // `packed` hoisted it, to choose between two helpers whose bodies are
        // exactly the two arms `put_h_tag` already branches on -- the packed
        // word, or the tag-array write plus the plain slot. One call does
        // both, with the flag already in a register.
        tables.put_h_tag(h, a, g, packed, stag_live);
        n += 1;
    }
    debug_assert!(match_end >= 2);
    if do_b {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let (h, g) = hash4_tag_mls(src, b, hash_shift, smask);
            // W10: see the `a` store above.
            tables.put_h_tag(h, b, g, packed, stag_live);
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

/// SUPERSEDED by `fill_dfast_after_match`, which fuses this with its
/// sibling and shares the position-`b` hash the two recomputed. Kept as
/// the reference shape -- the convention this crate uses for
/// `encode_4_streams` and `segment_histograms`. LLVM drops it.
#[allow(dead_code)]
#[inline]
fn fill_hash_long_after_match(
    tables: &mut MatchTables,
    src: &[u8],
    match_ip: usize,
    match_end: usize,
    // W41: the LONG hash shift, RESOLVED. This took `hash_log` and let
    // `hash8` re-derive `64 - hash_log.min(32)` at each of its two call
    // sites -- twice per match on the DFast ladder, for a block constant.
    // Same defect as W40 one function away, and the fourth family of it
    // found today.
    lshift: u32,
    ends: (bool, bool),
    smask: u64,
    // 1a: the short-tag shift, for the packed long store. Passed in like
    // `fill_hash_after_match`'s -- never re-derived from the struct field
    // (see 4a30eb4: that fold was build-to-build unstable).
    hash_shift: u32,
    ilimit: usize,
) {
    // Hoisted per call: see the tag accessors' `packed` doc.
    // W18: `stag_live` was computed here and never read -- an `is_empty()`
    // read through the `&mut MatchTables`, per MATCH, on the DFast ladder.
    let packed = tables.pack_tags;
    let ltag_live = !tables.ltags.is_empty();
    let (do_a, do_b) = ends;
    let mut n = 0u64;
    // W4/W5: `match_end` is `match_ip + n` with `n >= mls >= 4`, so the
    // `>= 2` test is dead at every call -- and `match_ip` is an index into
    // `src`, so the `saturating_add` guarding it is a cmov the encoder can
    // never take. Both ran per MATCH, in all three fill helpers.
    debug_assert!(match_ip < usize::MAX - 2);
    let a = match_ip + 2;
    // W10: this re-read the struct field one line after `packed` hoisted it.
    //
    // NAMES SEPARATED (debug-assert catch): `ltag_wanted` asks whether a tag
    // is worth COMPUTING -- true on packed frames, where the tag rides in the
    // slot and no array exists. The accessors' `live` asks whether the tag
    // ARRAY is non-empty. Passing the first as the second made them disagree
    // on exactly the packed case; harmless in release (the packed arm returns
    // before touching the array) but wrong, and the assert said so.
    let ltag_wanted = packed || ltag_live;
    if do_a && a <= ilimit {
        let g = if ltag_wanted {
            hash4_tag_mls(src, a, hash_shift, smask).1
        } else {
            0
        };
        tables.put_hl_tag(hash8_shift(src, a, lshift), a, g, packed, ltag_live);
        n += 1;
    }
    debug_assert!(match_end >= 2);
    if do_b {
        let b = match_end - 2;
        if b <= ilimit && b != a {
            let g = if ltag_wanted {
                hash4_tag_mls(src, b, hash_shift, smask).1
            } else {
                0
            };
            tables.put_hl_tag(hash8_shift(src, b, lshift), b, g, packed, ltag_live);
            n += 1;
        }
    }
    #[cfg(feature = "profile")]
    DF_ENDFILL.fetch_add(n, core::sync::atomic::Ordering::Relaxed);
    // INSTRUMENT DEFECT, repaired. This reported to `DF_ENDFILL` but NOT to
    // `note_hash_fill`, so `EncodeCounts::hash_fills` -- the crate-wide "table
    // positions written" counter -- omitted DFast's ENTIRE long-table fill.
    // DFast writes TWO tables per match; only the short one was counted, so
    // every per-position figure derived from `hash_fills` understated L3.
    // Section 7 of m7-anatomy.md was computed on the undercount and its L3
    // `fills/B` and `pos/B` are corrected there.
    crate::prof::note_hash_fill(n);
}

/// The DFast ladder (L3-L4): two hash tables, the short one tag-packed. Lives in `encode/dfast.rs`.
mod dfast;
pub use dfast::*;

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
    // TWIN RETIRED on its ISA density. Measured on the emitted asm: 1234
    // instructions of duplicated body converting 10 BMI2 ops -- 123 instructions
    // per op. The campaign's own precedent decides this: W4/W5/W6 retired the
    // HLOG and (hash_log, chain_log) specialisations on exactly the argument
    // that `shr %cl` and `shrx` are both one uop on every CPU that HAS BMI2,
    // and those were per-position paths too. Consistency, not a new judgement.
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
    // W12: ONE CALL SITE. `find_greedy_impl` is `#[inline(always)]`, so the
    // two arms inlined two ~1,100-instruction bodies; `MLS`'s entire reach is
    // `let mls = if MLS == 0 { params.min_match.max(3) } else { MLS };`, after
    // which `mls` is an ordinary runtime value in length comparisons. Same
    // shape as `find_fast`'s HLOG (W4/W5) and `find_dfast`'s (W11), and the
    // same correction applies: killing the const is only half of it -- the
    // second call site has to go too, or six-or-two inline expansions remain.
    // BRICK 79 (G0): the block's kernel shape as a const of the instance --
    // the lazy finder's brick 58. `walk_cont` is evaluated here exactly as
    // the body evaluates it (before the body's `walk_probe` update; asserted
    // there); `cp`/`ca` are per-frame facts.
    let kind = lazy_kind(
        false,
        tables.chain_pack,
        !tables.ctags.is_empty(),
        greedy_walk_cont(tables, search_attempts(params)),
    );
    match kind {
        1 => find_greedy_impl::<0, 1>(src, block_start, block_end, window, params, tables, reps),
        2 => find_greedy_impl::<0, 2>(src, block_start, block_end, window, params, tables, reps),
        3 => find_greedy_impl::<0, 3>(src, block_start, block_end, window, params, tables, reps),
        4 => find_greedy_impl::<0, 4>(src, block_start, block_end, window, params, tables, reps),
        _ => find_greedy_impl::<0, 7>(src, block_start, block_end, window, params, tables, reps),
    }
}

/// The greedy finder's WALK-CONTINUE dispatch, the one expression the
/// selector and the body both evaluate (BRICK 79). Unlike the lazy
/// finder's it has no `strategy` term.
#[inline(always)]
fn greedy_walk_cont(tables: &MatchTables, attempts: usize) -> bool {
    walk_cont_enabled()
        && tables.rep_yield <= walk_rep_max()
        && (tables.walk_first_share <= walk_first_max(attempts) || tables.walk_probe == 0)
}

/// Scratch acquisition + the too-short-block exit shared by `find_greedy_impl`,
/// `find_lazy_impl` and `find_bt_lazy` (and through them their bmi2 twins).
/// Runs once per block; `#[inline(never)]`.
#[inline(never)]
#[allow(clippy::type_complexity)]
fn chain_finder_prologue(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    tables: &mut MatchTables,
    mls: usize,
) -> Result<(Vec<Seq>, Vec<u8>), (Vec<Seq>, Vec<u8>)> {
    let keep = finder_scratch_enabled();
    let mut seqs = if keep {
        let mut v = core::mem::take(&mut tables.seq_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let mut lits = if keep {
        let mut v = core::mem::take(&mut tables.lit_scratch);
        v.clear();
        v
    } else {
        Vec::new()
    };
    let ilimit = block_end.saturating_sub(8);
    if block_start >= ilimit {
        crate::copies::add(crate::copies::C_LIT_PUSH, block_end - block_start);
        lits.extend_from_slice(&src[block_start..block_end]);
        return Err((seqs, lits));
    }
    // The RESERVE, once, here. `find_greedy` and `find_bt_lazy` each carried
    // their own copy of it INLINE after this call -- two `Vec::with_capacity`
    // arms, `__rust_alloc`, `handle_error` and the capacity compares, laid
    // out inside the hottest function on each of those levels -- and
    // `find_lazy` (L6-L12, the levels that carry the most traffic) had NONE:
    // its block-0 literals grew from `Vec::new()` by doubling, a chain of
    // reallocs and memcpys per frame that the other two never paid.
    //
    // `block_len + LIT_PUSH_WIDTH_MAX` is the bound `push_literals`' fixed-
    // width copy relies on: at most `block_len` literals plus one over-copy.
    let block_len = block_end - block_start;
    if lits.capacity() < block_len + LIT_PUSH_WIDTH_MAX {
        lits = Vec::with_capacity(block_len + LIT_PUSH_WIDTH_MAX);
    }
    let seq_guess = (tables.last_nseq + tables.last_nseq / 4 + 64).min(block_len / mls + 16);
    if seqs.capacity() < seq_guess {
        seqs = Vec::with_capacity(seq_guess);
    }
    Ok((seqs, lits))
}

/// C's "jump faster over incompressible sections" step, which this crate's
/// chain ladder never had.
///
/// `ZSTD_compressBlock_lazy_generic` advances a FAILED position by
/// `((ip - anchor) >> kSearchStrength) + 1`, so the step grows with the literal
/// run: on content that cannot match, C accelerates away while a plain
/// `ip += 1` walks every byte. `find_greedy_impl`, `find_lazy_impl` and
/// `find_bt_lazy` all did the plain thing -- the same "capability present in
/// one finder, absent in its neighbour" shape as the repcode and
/// back-extension defects. `find_fast`/`find_dfast` have had an accel shift
/// for levels.
///
/// MEASURED (1 MiB of `incomp-32m`, stage profiler, ONE process so box load
/// cancels between the arms): L1 943 us with MatchFind at 11.3%, against L9
/// 26,918 us with MatchFind at 86.0% -- 28.5x on data where the walk census
/// reads ZERO chain loads. All of it spent proving there is no match, one
/// byte at a time.
///
/// SWEPT, and the shipped value is 12, not C's 8. The shift trades size for
/// skipped positions and the two do not move together:
///
/// ```text
///   shift   L5 size   L7 size   L9 size   incomp speedup
///     8      +2,383    +2,597    +2,990     3.2x .. 4.8x
///    10        -254      -257      +155     3.1x .. 4.2x
///    12        -235      -216       -15     2.8x .. 3.7x
/// ```
///
/// 12 is the only value that is SMALLER on every level and both caps tested
/// (1 MiB and 4 MiB), so it is a strict win rather than a trade. 14 of 18
/// corpora are BYTE-IDENTICAL under it -- the step only grows on a long
/// literal run, so content that matches never sees it. `x-ray` is untouched
/// at 12 and regresses +1,581 at 10, which is what decided against 10.
///
/// Position arithmetic behind the speedup: with step `(x >> 12) + 1` a 1 MiB
/// literal run is crossed in ~4,100 positions instead of 1,048,576, and the
/// wall-clock ratio is smaller than that because once search stops dominating
/// the block/literal overhead does.
///
/// 0 = off (the historical `ip += 1`); otherwise the shift. C's is 8.
static LAZY_ACCEL_ARM: core::sync::atomic::AtomicUsize =
    core::sync::atomic::AtomicUsize::new(usize::MAX);

/// Bench hook: 0 disables, otherwise the shift.
pub fn set_lazy_accel_arm(v: usize) {
    LAZY_ACCEL_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn lazy_accel() -> usize {
    let v = LAZY_ACCEL_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v != usize::MAX {
        return v;
    }
    let n: usize = crate::env_knob_parse("RZSTD_LAZY_ACCEL").unwrap_or(12);
    LAZY_ACCEL_ARM.store(n, core::sync::atomic::Ordering::Relaxed);
    n
}

/// The accelerated no-match advance. BRICK 41 (P3): no `sh == 0` arm on the
/// per-position path -- `lazy_step_shift` maps the knob's 0 to 63 once per
/// block, and `(ip - anchor) >> 63` is 0 for every span a block can hold,
/// so the historical step of 1 falls out of the same expression.
#[inline(always)]
fn lazy_step(ip: usize, anchor: usize, sh: usize) -> usize {
    debug_assert!((1..64).contains(&sh) && anchor <= ip);
    ((ip - anchor) >> sh) + 1
}

/// BRICK 95 (P23): `lazy_step` with the `+ 1` folded into the anchor.
/// `anchor_adj` is `anchor - 2^sh` (wrapping), so `(ip - anchor_adj) >> sh`
/// is `(ip - anchor + 2^sh) >> sh == ((ip - anchor) >> sh) + 1` for every
/// span a block can hold (`ip - anchor < 2^63 - 2^sh`). Three instructions
/// per no-match position instead of five, and nothing to increment.
#[inline(always)]
fn lazy_step_adj(ip: usize, anchor_adj: usize, sh: usize) -> usize {
    debug_assert!((1..64).contains(&sh));
    ip.wrapping_sub(anchor_adj) >> sh
}

/// The anchor's folded form for `lazy_step_adj` (BRICK 95).
#[inline(always)]
fn anchor_adj_of(anchor: usize, sh: usize) -> usize {
    anchor.wrapping_sub(1usize << sh)
}

/// The `lazy_accel` knob as a shift `lazy_step` can apply unconditionally
/// (BRICK 41): 0 ("historical step") becomes 63, i.e. always 1; anything
/// above 63 saturates there (the old `>>` would have wrapped its amount).
#[inline(always)]
fn lazy_step_shift(sh: usize) -> usize {
    if sh == 0 {
        63
    } else {
        sh.min(63)
    }
}

/// The fill loops' block constants (BRICK 19). `lz_fill_range` took twelve
/// parameters, so on Win64 eight rode the stack: the census read 5-6 stack
/// argument stores at every call, and a call happens once per MATCH. Built
/// once per block, passed by reference, read once in the callee.
#[derive(Clone, Copy)]
struct FillCtx {
    stride: usize,
    shift32: u32,
    shift64: u32,
    smask: u64,
    /// BRICK 100: the tag byte's offset is `mls - 1`.
    mls: usize,
    chain_mask: usize,
    wide_h: bool,
    wchain: bool,
    cp: bool,
    ca: bool,
}
/// The chain-ladder fill loop, outlined (BRICK 10). `ROWS` mirrors the
/// `lz_insert_only` arm each caller used: greedy inserts rows too, lazy's
/// chain arm does not (its row arm is `row_fill_range`). `mode` is
/// `(wide_h, wchain)`, both block constants, resolved ONCE here into one
/// loop per arm exactly as the inline form did.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
#[allow(unsafe_code)]
fn lz_fill_range<const ROWS: bool, const CP: bool, const CA: bool, const SPEC: bool>(
    tables: &mut MatchTables,
    src: &[u8],
    mut p: usize,
    stop: usize,
    fc: &FillCtx,
) {
    let FillCtx {
        stride,
        shift32,
        shift64,
        smask,
        mls,
        chain_mask,
        wide_h,
        wchain,
        cp,
        ca,
    } = *fc;
    // BRICK 26: with `SPEC` the representation is the body's own and every
    // per-byte test on it folds; the rows body (`SPEC == false`) keeps the
    // runtime flags so the rare shape does not cost three more bodies.
    let (cp, ca) = if SPEC {
        debug_assert_eq!((cp, ca), (CP, CA));
        (CP, CA)
    } else {
        (cp, ca)
    };
    let src_len = src.len();
    // BRICK 12: the table bases, taken ONCE. `lz_insert_only` reached every
    // table through `&mut MatchTables`, and after each store LLVM re-read the
    // headers it could not prove the store had left alone -- the census read
    // 1-2 stack reloads per matched byte in the chain arms and none in the
    // row arm, which already worked from hoisted bases. Same stores, same
    // values, same order; only where the bases come from changes.
    let hp = tables.hash.as_mut_ptr();
    let chp = tables.chain.as_mut_ptr();
    let tp = tables.tags.as_mut_ptr();
    let ctp = tables.ctags.as_mut_ptr();
    // BRICK 74: the empty-head link (see `set_null_tag`); 0 unless packed.
    let null_link = tables.null_link;
    macro_rules! body {
        ($hash:expr) => {{
            let hash = $hash;
            macro_rules! ins {
                ($q:expr) => {{
                    let p = $q;
                    #[cfg(feature = "profile")]
                    if !ROWS {
                        LF_INSERTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    let (hh, gt) = hash(p);
                    if ROWS {
                        // Greedy inserts rows too, and the row insert goes through
                        // `&mut MatchTables`; measured with the raw form below, that arm
                        // got WORSE (22 -> 29 instructions, 1 -> 6 reloads per byte) as
                        // the header re-reads came back. It keeps the method call.
                        tables.lz_insert_only::<ROWS>(hh, p, gt, cp, ca, chain_mask);
                    } else {
                        debug_assert!(
                            hh < tables.hash.len() && (p & chain_mask) < tables.chain.len()
                        );
                        debug_assert!(
                            !ca || (hh < tables.tags.len()
                                && (p & chain_mask) < tables.ctags.len())
                        );
                        // SAFETY: `hh` is a hash output bounded by the table's own
                        // length, `p & chain_mask` by the chain's; `tags`/`ctags` are
                        // only touched when `ca` says they are sized to match. The
                        // bases were taken above and nothing here resizes a table.
                        unsafe {
                            let raw = *hp.add(hh);
                            let old_tag = if cp {
                                (raw >> 24) as u8
                            } else if ca {
                                *tp.add(hh)
                            } else {
                                0
                            };
                            // BRICK 36 (F1): ONE decode for all three representations.
                            // Packed heads are `(pos + 1) | tag << 24` with
                            // `pos + 1 < 0x00FF_FFFF` (the `pack_tags` guard, asserted
                            // at every writer), and the reset writes 0 -- so the low
                            // 24 bits are zero only when the word is, and for a live
                            // head `raw - 1` cannot borrow out of the field:
                            // `(q - 1) | tag << 24 == raw - 1`. The seven-instruction
                            // field split + guard + cmov was pure decode overhead on
                            // every inserted byte.
                            debug_assert!(!cp || raw == 0 || raw & 0x00FF_FFFF != 0);
                            // BRICK 74: an empty head links to position 0 WITH its tag --
                            // a select only where the link carries one (`cp`); otherwise
                            // the null link is 0 and the decode is the saturating form.
                            let link = if cp {
                                if raw == 0 {
                                    null_link
                                } else {
                                    raw - 1
                                }
                            } else {
                                debug_assert_eq!(null_link, 0);
                                raw.saturating_sub(1)
                            };
                            *chp.add(p & chain_mask) = link;
                            if ca {
                                *ctp.add(p & chain_mask) = old_tag;
                                *tp.add(hh) = gt;
                            }
                            // BRICK 39 (F2): no field mask. `pack_tags` bounds `p + 1`
                            // below 0x00FF_FFFF, so the mask was one dead `and` per
                            // inserted byte; the assertion is the bound it enforced.
                            debug_assert!(!cp || p + 1 < 0x00FF_FFFF);
                            *hp.add(hh) = if cp {
                                ((p as u32) + 1) | (u32::from(gt) << 24)
                            } else {
                                (p as u32) + 1
                            };
                        }
                    }
                }};
            }
            if !ROWS && stride == 1 && p < stop {
                // BRICK 43 (F3): two positions per trip for the default stride,
                // chain arms only -- the rows arm ships at stride 2 and its
                // method-call body measured +3 per position with the pair loop
                // present. Same inserts in the same order; the loop overhead
                // is paid once per pair and the pair's loads overlap.
                // BRICK 70 (F5): the bound as a countdown of the remaining
                // positions, so the loop test is on the counter and `stop`
                // is not a live value the loop has to reload.
                let mut left = stop - p;
                // BRICK 80 (F7): four per trip while there are four.
                while left >= 4 {
                    ins!(p);
                    ins!(p + 1);
                    ins!(p + 2);
                    ins!(p + 3);
                    p += 4;
                    left -= 4;
                }
                while left >= 2 {
                    ins!(p);
                    ins!(p + 1);
                    p += 2;
                    left -= 2;
                }
                if left != 0 {
                    ins!(p);
                }
            } else {
                while p < stop {
                    ins!(p);
                    p += stride;
                }
            }
        }};
    }
    // BRICK 88 (F8): no 8-byte-hash arm -- `wide_h` is `mls >= 8`, outside the
    // contract (brick 35); the test and three loop bodies were dead weight on
    // every call.
    debug_assert!(!wide_h);
    let _ = src_len;
    // BRICK 100 (F9): the byte tag -- see `link_tag`.
    if wchain {
        body!(|q: usize| hash_wide_link_tag_b(src, q, shift64, smask, mls));
    } else {
        body!(|q: usize| hash4_link_tag_b(src, q, shift32, mls));
    }
}

/// The row-table fill loop, outlined (BRICK 10) -- `find_lazy_impl`'s row arm.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn row_fill_range(
    rows: &mut crate::rowfind::RowTable,
    src: &[u8],
    mut p: usize,
    stop: usize,
    fc: &FillCtx,
) {
    let FillCtx {
        stride,
        shift32,
        shift64,
        smask,
        mls,
        wide_h,
        wchain,
        ..
    } = *fc;
    let src_len = src.len();
    let rmask = rows.mask();
    macro_rules! body {
        ($hash:expr) => {{
            while p < stop {
                #[cfg(feature = "profile")]
                LF_INSERTS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                let (hh, gt) = $hash;
                rows.insert_h(hh, rmask, p as u32, gt);
                p += stride;
            }
        }};
    }
    // BRICK 100 (F9): the byte tag -- see `link_tag`.
    if wide_h {
        body!(if p + 8 <= src_len {
            (hash8_shift(src, p, shift64), 0u8)
        } else if wchain {
            hash_wide_link_tag_b(src, p, shift64, smask, mls)
        } else {
            hash4_link_tag_b(src, p, shift32, mls)
        });
    } else if wchain {
        body!(hash_wide_link_tag_b(src, p, shift64, smask, mls));
    } else {
        body!(hash4_link_tag_b(src, p, shift32, mls));
    }
}

#[inline(always)]
fn find_greedy_impl<const MLS: usize, const KIND: u8>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BRICK 35 (K1): the CONTRACT's bound, both ends. `min_match` is
    // documented 3..=7 and `compression_params` / `apply_zstd_kv` both clamp
    // it there; only a hand-built `CompressionParameters` could carry more,
    // and zstd itself rejects such a value (`ZSTD_MINMATCH_MAX` is 7). With
    // the bound stated HERE, `mls_xor` needs no `mls > 8` arm -- which was a
    // compare and a branch on EVERY examined candidate in the chain walk,
    // and a callee-saved register holding `mls` for the walk's whole life.
    let mls = if MLS == 0 {
        params.min_match.clamp(3, 7) as usize
    } else {
        MLS
    };
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
    // W4: `finder_scratch_enabled()` is an arm read, and it was read TWICE
    // -- once per output buffer -- for one per-block answer.
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

    // W5: GATE 6 for Greedy. Both output buffers came from the frame but with
    // NO RESERVE, so they grew by repeated `realloc` with LIVE contents --
    // every growth a real memcpy. `find_fast` has had this since brick 38.
    // (reserve moved into `chain_finder_prologue`)
    // W6: GATE 13 for Greedy. Literals went out through `push_lits_range` -- a
    // runtime-length `extend_from_slice` -- while find_fast has used the
    // fixed-width `copy_nonoverlapping` since brick 38. W5 is its
    // precondition: the fast path declines unless spare capacity proves the
    // wide store in bounds. Byte-identical -- only `n` bytes are published.
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
    let lp_copy = lit_width_for(tables); // BRICK 71: repcode-1 search in find_greedy -- L5-L6 had none
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
    // W5: hoisted for the back-extension loop -- see its use.
    let fstart_c = tables.frame_start;
    let lowest_rep = block_start.saturating_sub(window).max(fstart_c);
    // WALK-CONTINUE dispatch: see `walk_rep_max`.
    // BRICK 79 (G0): the instance's const where the shape is known; the
    // expression is still evaluated (and asserted equal) in debug builds.
    let walk_cont_rt = greedy_walk_cont(tables, attempts);
    let walk_cont = match KIND {
        1 | 3 => true,
        2 | 4 => false,
        _ => walk_cont_rt,
    };
    debug_assert_eq!(walk_cont, walk_cont_rt);
    tables.walk_probe = if tables.walk_probe == 0 {
        WALK_PROBE_PERIOD
    } else {
        tables.walk_probe - 1
    };
    let mut wcls = (0u32, 0u32);
    maybe_latch_wide_chain(tables, src, block_start, window, mls);
    // BRICK 79 (G0): the representation as the instance's const (runtime on
    // the fallback instance). Packed and tag-array are exclusive by
    // construction, asserted here.
    let cp = match KIND {
        1 | 2 => true,
        3 | 4 => false,
        _ => tables.chain_pack,
    };
    let ca = match KIND {
        1 | 2 => false,
        3 | 4 => true,
        _ => !tables.ctags.is_empty(),
    };
    debug_assert_eq!(cp, tables.chain_pack);
    debug_assert_eq!(ca, !tables.ctags.is_empty());
    let wchain = tables.chain_wide;
    let smask = if mls >= 8 {
        u64::MAX
    } else {
        (1u64 << (8 * mls)) - 1
    };
    // W1: `cp || ca` -- whether ANY link-tag filter is active -- was re-OR'd on
    // every step of the chain chase.
    let tag_filter = cp || ca;
    // BRICK 78 (G4, brick 65 for L5): the position from which the walk's
    // lower bound is `ip - window` rather than `lowest_rep`.
    let lowest_w = lowest_rep + window;
    // W2: `mls >= 8` is the hash-width question, and it was re-asked per
    // POSITION (the head hash) and per FILLED POSITION, for one per-block
    // answer. `src.len()` beside it is a slice field re-read the same way.
    let wide_h = mls >= 8;
    // W36/W44: the two hash shifts, once per block. Both the MAIN loop's
    // per-position hash and the post-match fill re-derived `min` +
    // `saturating_sub` from `hash_log` -- the fill twice per matched byte,
    // the loop once per searched position.
    let g_shift32 = 32u32.saturating_sub(hash_log.min(32));
    let g_shift64 = 64u32.saturating_sub(hash_log.min(32));
    let src_len = src.len();
    // BRICK 74: the empty-head link for this block's producer.
    tables.set_null_tag(chain_null_tag(src, mls));
    // The searches/byte signal feeds the wide latch's second route; greedy
    // never maintained it, so at L5 the field held its 1.0 INIT and the
    // route always passed (smallmsg +1.62% leak).
    let mut searches = 0u64;
    let mut ip = block_start;
    // Hoisted per BLOCK: an atomic load per position would cost more
    // than the positions it skips.
    let accel_sh = lazy_step_shift(lazy_accel());
    // BRICK 23: the fill loop's shape is a per-block fact. With the row
    // table off (every input outside the row band) the `<true>` fill's
    // per-byte row test and its live row state buy nothing; the `<false>`
    // loop the lazy finder uses inserts the identical head/link/tag words
    // at 25-32 instructions per byte instead of 38-42.
    let rows_live = !tables.rows.head.is_empty();
    // BRICK 19: the fill's block constants, once, by reference.
    let fill_ctx = FillCtx {
        stride: 1,
        shift32: g_shift32,
        shift64: g_shift64,
        smask,
        mls,
        chain_mask,
        wide_h,
        wchain,
        cp,
        ca,
    };
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
        searches += 1;
        // W44: resolved shifts, same as the fill below.
        // BRICK 82 (G6, brick 68 for L5): no 8-byte-hash arm (`mls >= 8` is
        // outside the contract, brick 35).
        debug_assert!(!wide_h);
        // BRICK 100 (F9): the byte tag -- the LOAD form here; the lazy kernels
        // take the shared-word form (`hash4_link_tag_w`), and this finder
        // measured 4 fewer per no-match position with the load.
        let (h, gtag) = if wchain {
            hash_wide_link_tag_b(src, ip, g_shift64, smask, mls)
        } else {
            hash4_link_tag_b(src, ip, g_shift32, mls)
        };
        let (prev, head_tag) = tables.lz_insert(h, ip, gtag, cp, ca, chain_mask);

        let mut best_m = 0usize;
        // BRICK 75 (G1, brick 59 for L5): ONE length -- born `mls - 1`, so
        // `ml > best_ml` is the accept test before and after the first
        // accept; `pre_eq` at that index is sound on the first candidate.
        let mut best_ml = mls - 1;
        // W3: `best_ml` is 0 or a value that already cleared `mls`, so the
        // accept pair folds to one compare against a running bar.
        if let Some(mut m) = prev {
            let mut mtag = head_tag;
            // See `chain_find_best`: the three per-step validity tests fold
            // to one monotone bound; `m >= ip` is entry-only.
            let low = if ip >= lowest_w {
                ip - window
            } else {
                lowest_rep
            };
            debug_assert_eq!(low, lowest_rep.max(ip.saturating_sub(window)));
            // W48: `src_len` has been hoisted at block scope since W2; this
            // walk-entry guard kept re-reading the slice field anyway.
            // BRICK 82 (brick 49 for L5): `ip <= ilimit = block_end - 8` and
            // `mls <= 7` make `ip + mls <= src_len` the loop's own invariant.
            debug_assert!(ip + 8 <= src_len && mls <= 8);
            if m < ip {
                // BRICK 77 (G3, brick 63 for L5): two different non-zero values.
                let mut missed_before = 0u8;
                // BRICK 76 (G2, brick 62 for L5): an explicit countdown.
                let mut left = attempts;
                while left != 0 {
                    left -= 1;
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
                    // BRICK 87 (G9, brick 74 for L5): no `m != 0` exemption on the
                    // packed instances -- their null links carry position 0's tag.
                    if tag_filter && (cp || m != 0) && mtag != gtag {
                        #[cfg(feature = "profile")]
                        if COUNT {
                            use core::sync::atomic::Ordering::Relaxed;
                            LINK_SKIPS.fetch_add(1, Relaxed);
                            if mls_eq(src, m, ip, mls, smask) {
                                LINK_FALSE.fetch_add(1, Relaxed);
                            }
                        }
                        missed_before = 1;
                        if !walk_cont {
                            break;
                        }
                        // W9: `m & chain_mask` is the slot index for BOTH the
                        // link and its tag; it was masked twice per rejected
                        // link, on the path the tag filter exists to make cheap.
                        let slot = m & chain_mask;
                        let link = tables.chain_masked(slot);
                        let next = if cp {
                            (link & 0x00FF_FFFF) as usize
                        } else {
                            link as usize
                        };
                        if next >= m {
                            break;
                        }
                        mtag = if cp {
                            (link >> 24) as u8
                        } else {
                            tables.ctags_masked(slot)
                        };
                        m = next;
                        continue;
                    }
                    if COUNT {
                        probes += 1;
                        #[cfg(feature = "profile")]
                        WALK_EXAM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                    if let Some(x) = mls_xor(src, m, ip, mls, smask) {
                        // C's `match[ml] == ip[ml]` prefilter
                        // (`ZSTD_HcFindBestMatch`): a candidate that DIFFERS at
                        // the current best length cannot exceed it, so the full
                        // `count_match` is provably wasted. The same candidate
                        // still wins, so this is byte-identical.
                        if pre_eq(src, m, ip, best_ml) {
                            // Count past mls_eq's verified prefix (see
                            // `chain_find_best`).
                            let ml = fused_ml(x, src, m, ip, block_end); // BRICK 11: see `mls_xor`
                            if ml > best_ml {
                                if missed_before != 0 {
                                    if best_ml < mls {
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
                        missed_before = 2;
                        #[cfg(feature = "profile")]
                        if COUNT {
                            WALK_BYTEMISS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                        }
                        if !walk_cont {
                            break;
                        }
                    }
                    let link = tables.chain_masked(m & chain_mask);
                    let next = if cp {
                        (link & 0x00FF_FFFF) as usize
                    } else {
                        link as usize
                    };
                    if next >= m {
                        break;
                    }
                    mtag = if cp {
                        (link >> 24) as u8
                    } else if ca {
                        tables.ctags_masked(m & chain_mask)
                    } else {
                        0
                    };
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
            rep1 = ip - best_m;
            let end = ip + best_ml;
            // Positions `s..=ip` were ALREADY inserted as the loop walked to
            // `ip`; re-inserting them would self-loop the chain (see B2).
            // W7: `end` and `ilimit` are both fixed for this fill, so the
            // two bounds it tested on every inserted position fold to one.
            let stop = end.min(ilimit + 1);
            // BRICK 10: outlined, see `lz_fill_range` and the note in `find_lazy_impl`.
            if rows_live {
                lz_fill_range::<true, false, false, false>(tables, src, ip + 1, stop, &fill_ctx);
            } else if cp {
                lz_fill_range::<false, true, false, true>(tables, src, ip + 1, stop, &fill_ctx);
            } else if ca {
                lz_fill_range::<false, false, true, true>(tables, src, ip + 1, stop, &fill_ctx);
            } else {
                lz_fill_range::<false, false, false, true>(tables, src, ip + 1, stop, &fill_ctx);
            }
            ip = end;
            anchor = ip;
        } else {
            // C: `ip += ((ip-anchor) >> kSearchStrength) + 1`.
            ip += lazy_step(ip, anchor, accel_sh);
        }
    }
    greedy_finder_epilogue(
        tables,
        src,
        &seqs,
        &mut lits,
        anchor,
        block_start,
        block_end,
        rep_hits,
        walk_cont,
        wcls,
        attempts,
        searches,
        probes,
        hits,
    );
    (seqs, lits)
}

/// The per-block tail of `find_greedy_impl`, shared with its bmi2 twin --
/// the `fast_finder_epilogue` treatment at x2 scale. Runs once per block.
#[inline(never)]
#[allow(clippy::too_many_arguments)]
fn greedy_finder_epilogue(
    tables: &mut MatchTables,
    src: &[u8],
    seqs: &[Seq],
    lits: &mut Vec<u8>,
    anchor: usize,
    block_start: usize,
    block_end: usize,
    rep_hits: u64,
    walk_cont: bool,
    wcls: (u32, u32),
    attempts: usize,
    searches: u64,
    probes: u64,
    hits: u64,
) {
    const COUNT: bool = cfg!(feature = "profile");
    tables.rep_yield = if seqs.is_empty() {
        1.0
    } else {
        (rep_hits as f32 / seqs.len() as f32).max(tables.rep_yield * 0.5)
    };
    update_walk_first_share(tables, walk_cont, wcls, attempts);
    let span = (block_end - block_start).max(1) as f32;
    tables.last_search_per_byte = searches as f32 / span;
    push_lits_range(lits, src, anchor, block_end);
    note_finder_work(COUNT, probes, hits, seqs, lits);
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
/// W1: the walk's per-call PROLOGUE, hoisted. `chain_find_best` is called per
/// position AND per look-ahead step across L5-L12, and every one of those
/// calls re-derived the same six per-BLOCK values from `&mut MatchTables`:
/// `hash_log`, `chain.len() - 1`, `chain_pack`, `!ctags.is_empty()`,
/// `chain_wide` and the `mls` byte mask. Struct reads through a `&mut` that
/// LLVM must re-prove after every table write the walk performs.
///
/// Same shape `BtCtx` uses for the tree walk.
pub(crate) struct ChainCtx<'a> {
    src: &'a [u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    hash_log: u32,
    chain_mask: usize,
    smask: u64,
    cp: bool,
    ca: bool,
    wchain: bool,
    /// W2: `mls >= 8`, the hash-width question, answered once per block.
    wide_hash: bool,
    /// W24: `wchain` and `wide_hash` as ONE byte -- bit 0 is the wide-chain
    /// key, bit 1 the 8-byte hash. `row_find_best` is `inline(never)` behind a
    /// fn pointer, so nothing hoists across positions and every `ChainCtx`
    /// field it names is a real per-position load; two bools were two.
    hash_mode: u8,
    /// W20: `32 - hash_log` and `64 - hash_log`, the two hash shifts. Both are
    /// block constants that `hash4_link_tag` and `hash8` re-derived from
    /// `hash_log` on EVERY position -- a `min` and a `saturating_sub` each.
    hash_shift32: u32,
    hash_shift64: u32,
    /// W21: `lowest.max(1)`. The row walk clamps the window floor to 1 so the
    /// empty-slot sentinel folds into it (14.7 W6); the `.max(1)` is a block
    /// constant that was being re-applied per position.
    lowest1: usize,
    /// W4: `block_start.saturating_sub(window).max(frame_start)` -- a
    /// saturating sub, a max and a struct load, rebuilt on every call for a
    /// value the caller already computes as `lowest_rep`.
    lowest: usize,
    /// BRICK 65 (P17): `lowest + window`, the position from which the walk's
    /// lower bound is `ip - window` rather than `lowest` -- one compare per
    /// walk instead of a saturating subtract and a max.
    lowest_w: usize,
    /// BRICK 71 (P19): whether the row table exists (`!rows.head.is_empty()`),
    /// a per-block fact the walk's insert re-read through the tables pointer
    /// on every call to decide the row mirror.
    rows_live: bool,
    /// W5: `cp || ca` -- whether ANY link-tag filter is active. Both terms are
    /// per block, but the walk re-OR'd them on every LINK STEP.
    tag_filter: bool,
    /// BRICK 14: a per-BLOCK bool that was the kernel's THIRD argument, so
    /// `tables` -- the fifth -- went to the stack at every call (one store at
    /// each of the two call sites per position, one load in the callee) and
    /// the callee spilled the bool on entry. Here it is one field of a
    /// context the callee already dereferences.
    walk_cont: bool,
}

type ChainFn = for<'a> fn(&ChainCtx<'a>, usize, &mut MatchTables) -> (usize, usize);

/// E1: the ROW walk -- one dependent load per ROW instead of per CANDIDATE.
///
/// Conforms to `ChainFn`, so it is a drop-in for `chain_find_best` at the
/// dispatch. It does NOT walk the chain at all: the row's 16 tags come back in
/// one load, `row_tag_mask` compares them in one instruction, and the set bits
/// are the candidates -- already ordered newest-first, i.e. nearest-offset
/// first, which is what makes taking the first acceptable match sound.
///
/// `walk_cont` and `cls` are the CHAIN walk's amputation-recovery machinery:
/// the chain broke on the first tag mismatch and `walk_cont` let it carry on.
/// A row has no links to break, so there is nothing to continue past and
/// nothing to classify -- every candidate in the row is visited regardless.
///
/// Bitstream-CHANGING; see `set_row_arm`.
#[inline(never)]
fn row_find_best<const MLS: usize>(
    ctx: &ChainCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let ChainCtx {
        src,
        block_end,
        window,
        mls,
        attempts,
        smask,
        hash_mode,
        hash_shift32,
        hash_shift64,
        lowest1,
        ..
    } = *ctx;
    // W23: `hash_log` is gone from this destructure -- W20/W22 handed the two
    // shifts derived from it straight in, so the finder no longer loads it.
    // W14: `cp`, `ca` and `chain_mask` are gone from this destructure -- they
    // existed only to drive the chain writes section 14.8 proved dead, so the
    // row finder no longer loads them from `ChainCtx` at all.
    let mls = if MLS == 0 { mls } else { MLS };
    // W9: `src.len()` is a slice field re-read; both uses below want the same
    // per-call value.
    let src_len = src.len();
    // W20/W22/W24: one mode byte, and all three shifts arrive resolved.
    // Arm order is exactly the original `wide_hash && fits`, then `wchain`,
    // then the 4-byte key -- including the case where a wide-hash block near
    // the buffer end falls through to the wide-chain arm.
    let (h, gtag) = if hash_mode & 2 != 0 && ip + 8 <= src_len {
        (hash8_shift(src, ip, hash_shift64), 0u8)
    } else if hash_mode & 1 != 0 {
        hash_wide_link_tag_b(src, ip, hash_shift64, smask, mls)
    } else {
        hash4_link_tag_w(src, ip, hash_shift32, mls)
    };
    // W11: probe returns the WALK STATE, not a collected array. The row is
    // still read strictly before `ip` is inserted -- the insert now sits below
    // the walk instead of the walk being pre-materialised above it, which is
    // the same snapshot with none of the copying.
    let r = tables.rows.row_of(h);
    let (mut w, rhead, row, rat) = tables.rows.probe_view(r, gtag);
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        ROW_LOADS.fetch_add(1, Relaxed);
        ROW_BUCKET[4].fetch_add(1, Relaxed);
        if gtag == 0 {
            ROW_BUCKET[3].fetch_add(1, Relaxed);
        }
    }

    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    let mut best_m = 0usize;
    let mut best_ml = 0usize;
    // W7's acceptance bar, unchanged: `best_ml` is 0 or already >= mls, so one
    // compare against a running bar replaces two.
    let mut bar = mls;
    if ip + mls > src_len {
        tables.lz_insert_rowknown(r, rat, rhead, ip, gtag);
        return (0, 0);
    }
    // W6: `.max(1)` folds the empty-slot sentinel INTO the window floor.
    // Position 0 is the sentinel and can never be a real candidate, so
    // `m == 0` and `m < low` become the same rejection.
    let low = lowest1.max(ip.saturating_sub(window));
    // W10: an acceptable candidate needs `low <= m < ip`. When that range is
    // empty the whole walk cannot produce one, and the old code discovered
    // this by rejecting every candidate in turn. It also makes W7's fused
    // compare sound, by guaranteeing `low <= ip`.
    if ip <= low {
        tables.lz_insert_rowknown(r, rat, rhead, ip, gtag);
        return (0, 0);
    }
    let span = ip - low;
    // W15: the attempt budget is applied to the MASK, once, instead of counted
    // down per candidate. Section 14.9's census measured 4.83 candidates per
    // probe against `attempts = 1 << search_log` (32 at L9, 64+ at L12), so the
    // countdown fired on essentially no walk yet cost a decrement, a compare
    // and a branch on all 86.9M of them. Trimming instead: the walk visits set
    // bits from the high end, so "the newest `attempts` candidates" is "clear
    // the lowest set bits until popcount == attempts" -- and `w &= w - 1`
    // clears exactly the lowest. The fixup loop runs only when a row actually
    // over-delivers, which the census says is rare; the walk itself is then
    // unconditional.
    let mut extra = (w.count_ones() as usize).saturating_sub(attempts);
    while extra != 0 {
        w &= w - 1;
        extra -= 1;
    }
    while w != 0 {
        // W11's walk, inline: highest set bit is the newest slot.
        let b = (crate::rowfind::ROW as u32 - 1) - w.leading_zeros();
        w &= !(1u16 << b);
        let s = ((b + rhead) & (crate::rowfind::ROW as u32 - 1)) as usize;
        let m = row[s] as usize;
        // W7: THREE rejects, ONE compare. `m - low` is borrow-free exactly
        // when `m >= low`, and below `span` exactly when `m < ip`; the
        // sentinel rides along via W6. Same accepted set, one branch.
        if m.wrapping_sub(low) >= span {
            continue;
        }
        if COUNT {
            probes += 1;
            #[cfg(feature = "profile")]
            ROW_EXAM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        }
        #[cfg(feature = "profile")]
        {
            use core::sync::atomic::Ordering::Relaxed;
            ROW_BUCKET[0].fetch_add(1, Relaxed);
            if m + 8 <= src_len {
                let hm = if hash_mode & 2 != 0 && m + 8 <= src_len {
                    hash8_shift(src, m, hash_shift64)
                } else if hash_mode & 1 != 0 {
                    hash_wide_link_tag_b(src, m, hash_shift64, smask, mls).0
                } else {
                    hash4_link_tag_b(src, m, hash_shift32, mls).0
                };
                if hm == h {
                    ROW_BUCKET[1].fetch_add(1, Relaxed);
                }
            }
            if mls_eq(src, m, ip, mls, smask) {
                ROW_BUCKET[2].fetch_add(1, Relaxed);
            }
        }
        if let Some(x) = mls_xor(src, m, ip, mls, smask) {
            // C's `match[ml] == ip[ml]` prefilter, same as the chain walk.
            if best_ml == 0 || pre_eq(src, m, ip, best_ml) {
                let ml = fused_ml(x, src, m, ip, block_end); // BRICK 11: see `mls_xor`
                if ml >= bar {
                    best_ml = ml;
                    best_m = m;
                    bar = ml + 1;
                    if ip + best_ml >= block_end {
                        break;
                    }
                }
            }
        }
    }
    // W11's other half: the insert the walk was moved above. `row` (a borrow of
    // `tables.rows`) is dead by here, so the mutable borrow is free to start --
    // and every early return above does its own insert, so this path is reached
    // exactly when the walk ran.
    tables.lz_insert_rowknown(r, rat, rhead, ip, gtag);
    if COUNT {
        crate::prof::note_probes(probes);
    }
    (best_m, best_ml)
}

/// BRICK 20: one instantiation per TAG REPRESENTATION -- `CP` (packed
/// 24-bit link + 8-bit tag), `CA` (tag array), or neither. Both are
/// per-block constants that the walk re-tested on every candidate (two
/// `test`s and a `cmov` on `cp`, a stack-reloaded `tag_filter` compare),
/// holding a register and two stack slots across the hottest loop in the
/// L6-L12 encoder. The block selects the instantiation once, through the
/// fn pointer `find_lazy_impl` already dispatches on. `CP` and `CA` are
/// never both true (`ctags` is only allocated when `chain_pack` is off,
/// and the kernel consults `ca` only when `cp` is false), so three of the
/// four shapes exist.
#[inline(never)]
fn chain_find_best<const MLS: usize, const CP: bool, const CA: bool, const WC: bool>(
    ctx: &ChainCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    chain_find_best_inner::<MLS, CP, CA, WC>(ctx, ip, tables)
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
#[allow(unsafe_code)]
fn chain_find_best_inner<const MLS: usize, const CP: bool, const CA: bool, const WC: bool>(
    ctx: &ChainCtx,
    ip: usize,
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
    // W1/W2: all six of these were rebuilt on EVERY call -- see `ChainCtx`.
    let ChainCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts,
        hash_log,
        chain_mask,
        smask,
        cp,
        ca,
        wchain,
        wide_hash,
        lowest,
        lowest_w,
        rows_live,
        tag_filter,
        // W45: both shifts, resolved by `ChainCtx` since 14.9's W20.
        hash_shift32,
        hash_shift64,
        walk_cont,
        ..
    } = *ctx;
    debug_assert_eq!(tag_filter, cp || ca);
    // BRICK 20: the representation is the instantiation's; the ctx copies
    // are checked against it and then shadowed so every test below folds.
    debug_assert_eq!(cp, CP);
    debug_assert_eq!(ca, CA);
    let cp = CP;
    let ca = CA;
    let tag_filter = CP || CA;
    // BRICK 24: and the walk-continue decision, the last per-block flag the
    // mismatch path reloaded and tested per candidate.
    debug_assert_eq!(walk_cont, WC);
    let walk_cont = WC;
    let mls = if MLS == 0 { mls } else { MLS };
    debug_assert_eq!(hash_log, tables.hash_log);
    debug_assert_eq!(chain_mask, tables.chain.len() - 1);
    debug_assert_eq!(cp, tables.chain_pack);
    debug_assert_eq!(ca, !tables.ctags.is_empty());
    debug_assert_eq!(wchain, tables.chain_wide);
    debug_assert_eq!(wide_hash, mls >= 8);
    debug_assert_eq!(
        smask,
        if mls >= 8 {
            u64::MAX
        } else {
            (1u64 << (8 * mls)) - 1
        }
    );
    // W45: `ChainCtx` has carried both shifts resolved since 14.9's W20 --
    // the ROW finder consumed them and the CHAIN walk, one function away, kept
    // re-deriving them per searched position.
    // W49: and `src.len()` twice more, for the same reason the row finder
    // hoisted it (W9).
    let src_len = src.len();
    // BRICK 68 (P11, brick 54 retried inlined): no 8-byte-hash arm. `wide_hash` is `mls >= 8`, and the
    // finders bound `mls` to 3..=7 (brick 35), so the arm's flag test, add
    // and compare ran on every call for a case that cannot arrive. The row
    // finder still reads `wide_hash` through `hash_mode`; this kernel does
    // not.
    debug_assert!(!wide_hash, "chain kernel: mls >= 8 is outside the contract");
    // BRICK 100b: the hash4 arm's tag from the word `mls_xor` hoists (see
    // `link_tag_from`); the wide arm keeps the byte load -- sharing the
    // word there moved the tag-array shapes' dominant paths +1.
    let (h, gtag) = if wchain {
        hash_wide_link_tag_b(src, ip, hash_shift64, smask, mls)
    } else {
        hash4_link_tag_w(src, ip, hash_shift32, mls)
    };
    // BRICK 69 (P18): the insert on raw bases taken ONCE per call -- the
    // fill's brick 12 for the walk. `lz_insert` re-derived each base from
    // the tables pointer (three dependent loads per call) and the loop
    // derived the chain base again. Same writes as `lz_insert`: the old
    // head becomes the link (`raw - 1`, brick 36's identity), the tag array
    // is mirrored when live, the head takes `ip + 1 | tag << 24` (no mask,
    // brick 39's bound), and the rows are mirrored through the method as
    // before.
    let hp = tables.hash.as_mut_ptr();
    let chp = tables.chain.as_mut_ptr();
    let tp = tables.tags.as_mut_ptr();
    let ctp = tables.ctags.as_mut_ptr();
    debug_assert!(h < tables.hash.len() && chain_mask < tables.chain.len());
    debug_assert!(!ca || (h < tables.tags.len() && chain_mask < tables.ctags.len()));
    debug_assert!(!cp || ip + 1 < 0x00FF_FFFF);
    // SAFETY: `h` is the hash's own output, bounded by the table length; the
    // chain and tag-array indices are masked by `chain_mask`; `tags`/`ctags`
    // are touched only when `ca` says they are sized with the tables. The
    // bases were taken above and nothing here resizes a table.
    let (prev, head_tag) = unsafe {
        let raw = *hp.add(h);
        let old_tag = if cp {
            (raw >> 24) as u8
        } else if ca {
            *tp.add(h)
        } else {
            0
        };
        debug_assert!(!cp || raw == 0 || raw & 0x00FF_FFFF != 0);
        // BRICK 74: an empty head links to position 0 WITH its tag (packed).
        *chp.add(ip & chain_mask) = if cp {
            if raw == 0 {
                tables.null_link
            } else {
                raw - 1
            }
        } else {
            raw.saturating_sub(1)
        };
        if ca {
            *ctp.add(ip & chain_mask) = old_tag;
            *tp.add(h) = gtag;
        }
        *hp.add(h) = if cp {
            ((ip as u32) + 1) | (u32::from(gtag) << 24)
        } else {
            (ip as u32) + 1
        };
        (MatchTables::lz_head_pos(raw, cp), old_tag)
    };
    // BRICK 71 (P19): the row mirror behind the block's own bool.
    debug_assert_eq!(rows_live, !tables.rows.head.is_empty());
    if rows_live {
        let r = tables.rows.row_of(h);
        tables.rows.insert(r, ip as u32, gtag);
    }
    // P0/gg-matchfind: candidate examinations are the WORK COUNTER, the primary
    // evidence under the Great Gate 2026-08-06 law. Compiled out entirely when
    // the profile feature is off.
    const COUNT: bool = cfg!(feature = "profile");
    let mut probes = 0u64;
    // BRICK 64 (P14): the walk_cont classification is written straight to
    // `tables.wcls` on the rare accept (brick 50's locals, right for the
    // standalone kernel, cost nine instructions per walk once the walk was
    // inlined: duplicated zero stores at entry and a two-load test at exit).
    let mut best_m = 0usize;
    // BRICK 59 (K8): ONE length. `best_ml` is born `mls - 1`, so `ml >
    // best_ml` is the accept test before and after the first accept (W7's
    // bar was `best_ml + 1`, seeded with `mls`), `pre_eq` at that index is
    // sound on the first candidate (the first-word compare just verified
    // byte `mls - 1`), and "no match yet" is `best_ml < mls`.
    let mut best_ml = mls - 1;
    let Some(mut m) = prev else {
        #[cfg(feature = "profile")]
        WALK_EXIT[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return (0, 0);
    };
    let mut mtag = head_tag;
    // W4: from the context -- see `ChainCtx::lowest`.
    debug_assert_eq!(
        lowest,
        block_start.saturating_sub(window).max(tables.frame_start)
    );
    // The walk's THREE per-step validity tests fold to ONE: `m >= ip` can
    // only fire on ENTRY (afterwards m strictly decreases below ip), and the
    // window and lowest checks are both lower bounds on m, merged into a
    // per-walk constant. `ip - m > window  <=>  m < ip - window` for m < ip.
    // BRICK 65 (P17): one compare. `lowest_w = lowest + window`, so below it
    // the bound is `lowest`; at or above it `ip - window >= lowest`.
    debug_assert_eq!(lowest_w, lowest + window);
    let low = if ip >= lowest_w { ip - window } else { lowest };
    debug_assert_eq!(low, lowest.max(ip.saturating_sub(window)));
    // BRICK 63 (K12): a small integer, not a bool -- the two miss arms
    // write DIFFERENT values so LLVM cannot hoist one constant store above
    // the tag test and pay it (plus the restore) on the paths that never
    // miss. Readers test `!= 0`.
    let mut missed_before = 0u8;
    // BRICK 49 (P9): `ip + mls <= src_len` is the caller's invariant, not a
    // per-call question -- `find_lazy_impl` holds `ip <= ilimit = block_end - 8`
    // at both call sites and `mls <= 7` (brick 35). It was an add, a compare
    // and a branch per call, with both operands reloaded from the stack.
    debug_assert!(ip + 8 <= src_len && mls <= 8);
    if m < ip {
        // 5 = ran the full depth; each `break` below overwrites it.
        // BRICK 61 (K10): the chain and tag-array bases, once. Through
        // `&mut MatchTables` the inlined walk re-derived the chain base from
        // the tables pointer on every candidate (two dependent loads).
        // Nothing in the loop resizes a table; `m & chain_mask` is in bounds
        // by the mask (brick 50's argument, restated at each read).
        // (BRICK 69: `chp` / `ctp` are the entry's bases.)
        #[cfg(feature = "profile")]
        let mut exit_why = 5usize;
        // BRICK 62 (K11): an explicit countdown that nothing else reads.
        // `for _ in 0..attempts` is an up-counter against `attempts`, which
        // in the inlined finder is a frame slot reloaded on every candidate;
        // the countdown is `dec`/`je` with nothing to load.
        let mut left = attempts;
        while left != 0 {
            left -= 1;
            // Monotone: m only decreases, so one bound test per step.
            if m < low {
                #[cfg(feature = "profile")]
                {
                    exit_why = 2;
                }
                break;
            }
            // The next-link load, issued at the TOP rather than the bottom: it
            // depends only on `m`, which is already in hand, and its result is
            // not consumed until the end of the body, so hoisting lets it
            // overlap the tag compare and `mls_eq`'s own random `src[m]`
            // access instead of serialising behind them.
            //
            // MEASURED NO-OP, recorded so it is not "discovered" again: the
            // emitted code is byte-for-byte what the bottom-placed version
            // produced (`find_lazy` 1568, `find_greedy` 1494, unchanged).
            // `chain_masked` is a pure read with no aliasing barrier, so LLVM's
            // scheduler was already free to hoist it and had. The source form
            // is kept because it states the intent; it buys nothing.
            //
            // Byte-identical: `chain_masked` is a pure read and nothing in the
            // body writes `chain` (the insert happens before the loop).
            // SAFETY: `m & chain_mask <= chain_mask < chain.len()` (asserted
            // above); the base was taken above and nothing here resizes.
            let link = unsafe { *chp.add(m & chain_mask) };
            // Link-tag reject: skip `mls_eq`'s src[m] load on a tag byte the
            // link load already delivered. Sound: mls_eq true => 4 bytes
            // equal => tags equal.
            // See the greedy walk: position 0 is sentinel-ambiguous, never
            // tag-filtered.
            // BRICK 74 (K14): no `m != 0` exemption for the PACKED shape -- its
            // null links carry position 0's own tag (see `set_null_tag`), so the
            // phantom candidate is tag-tested like any other. The tag-array
            // shape keeps the exemption (its links carry no tag byte).
            if tag_filter && (cp || m != 0) && mtag != gtag {
                #[cfg(feature = "profile")]
                if COUNT {
                    use core::sync::atomic::Ordering::Relaxed;
                    LINK_SKIPS.fetch_add(1, Relaxed);
                    if mls_eq(src, m, ip, mls, smask) {
                        LINK_FALSE.fetch_add(1, Relaxed);
                    }
                }
                missed_before = 1;
                if !walk_cont {
                    break;
                }
                // DUPLICATE ADVANCE REMOVED: this branch carried its own copy
                // of the chain-link step and `continue`d past the shared one at
                // the loop bottom. Same code -- `tag_filter == cp || ca`, so
                // inside this branch `!cp` implies `ca`, and its unconditional
                // `ctags_masked` IS the bottom's `else if ca`. ~95% of steps at
                // L9 end in an advance, so the copy was on the hot edge.
            } else {
                if COUNT {
                    probes += 1;
                    #[cfg(feature = "profile")]
                    WALK_EXAM.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    // BRICK 46: the phantom position-0 candidate (see
                    // `take_walk_phantom`).
                    #[cfg(feature = "profile")]
                    if m == 0 {
                        WALK_M0.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    }
                }
                if let Some(x) = mls_xor(src, m, ip, mls, smask) {
                    // C's `match[ml] == ip[ml]` prefilter -- see `find_greedy`.
                    if pre_eq(src, m, ip, best_ml) {
                        // Count from the byte AFTER what mls_eq just verified --
                        // restarting at 0 re-compared the first word of every
                        // candidate (the fast_probe_wide rule, applied here).
                        // BRICK 11: see `mls_xor`; BRICK 56: the long
                        // continuation is outlined behind `ctx`.
                        let ml = if x != 0 {
                            #[cfg(feature = "profile")]
                            FUSED_SHORT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            (x.trailing_zeros() as usize) >> 3
                        } else {
                            walk_count8(ctx, m, ip)
                        };
                        // offset_ok and the frame_start floor are GUARANTEED by
                        // the walk bound (m >= low >= lowest >= frame_start,
                        // m >= ip - window, m < ip); re-checking per accept was
                        // pure redundancy.
                        //
                        // W7: `ml >= mls && ml > best_ml` is two compares and two
                        // branches per accepted candidate, but `best_ml` is only
                        // ever assigned a value that already cleared `mls` -- so
                        // it is 0 or >= mls, and the pair is one compare against
                        // a running bar. (The same fold REGRESSED in the Bt walk,
                        // where the extra live value spilled `best_m`; this loop
                        // carries fewer, so it is re-measured here, not assumed.)
                        if ml > best_ml {
                            if missed_before != 0 {
                                if best_ml < mls {
                                    tables.wcls.0 += 1;
                                } else {
                                    tables.wcls.1 += 1;
                                }
                                #[cfg(feature = "profile")]
                                if COUNT {
                                    use core::sync::atomic::Ordering::Relaxed;
                                    if best_ml < mls {
                                        WALK_CONT_FIRST.fetch_add(1, Relaxed);
                                    } else {
                                        WALK_CONT_UPGRADE.fetch_add(1, Relaxed);
                                    }
                                }
                            }
                            #[cfg(feature = "profile")]
                            if m == 0 {
                                WALK_M0_ACCEPT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            }
                            best_ml = ml;
                            best_m = m;
                            if ip + best_ml >= block_end {
                                #[cfg(feature = "profile")]
                                {
                                    exit_why = 4;
                                }
                                break;
                            }
                        }
                    }
                } else {
                    missed_before = 2;
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
            }
            let next = if cp {
                (link & 0x00FF_FFFF) as usize
            } else {
                link as usize
            };
            if next >= m {
                #[cfg(feature = "profile")]
                {
                    exit_why = 3;
                }
                break;
            }
            mtag = if cp {
                (link >> 24) as u8
            } else if ca {
                // SAFETY: as the chain read; `ctags` is sized with `chain` when
                // `ca` (asserted above).
                unsafe { *ctp.add(m & chain_mask) }
            } else {
                0
            };
            m = next;
        }
        #[cfg(feature = "profile")]
        WALK_EXIT[exit_why].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    } else {
        #[cfg(feature = "profile")]
        WALK_EXIT[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    if COUNT {
        crate::prof::note_probes(probes);
    }
    // BRICK 59: `best_ml < mls` is "nothing accepted" -- an accept needs
    // `ml > mls - 1`. Same `(0, 0)` as before.
    if best_ml < mls {
        return (0, 0);
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
    // TWIN RETIRED on its ISA density. Measured on the emitted asm: 1002
    // instructions of duplicated body converting 9 BMI2 ops -- 111 instructions
    // per op. The campaign's own precedent decides this: W4/W5/W6 retired the
    // HLOG and (hash_log, chain_log) specialisations on exactly the argument
    // that `shr %cl` and `shrx` are both one uop on every CPU that HAS BMI2,
    // and those were per-position paths too. Consistency, not a new judgement.
    find_lazy_sel(
        src,
        block_start,
        block_end,
        window,
        params,
        tables,
        depth,
        reps,
    )
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
    // W13: one call site, as W12. Note `MLS` here ALSO chose the chain/row
    // kernel monomorphisation (`chain_find_best::<MLS>` as a fn pointer), so
    // this collapse takes those generic too -- `mls` becomes a runtime compare
    // in the walk instead of an immediate. Measured, not assumed.
    //
    // BRICK 58 (P13b): the block's kernel SHAPE is a const of the finder so
    // the walk is inlined at its two call sites (see `lazy_search`). The
    // shape is derived here exactly as the body derives it (asserted there);
    // the row finder and the tags-off shapes keep the pointer (`KIND == 7`).
    let kind = lazy_kind(
        row_find_enabled() && !tables.rows.head.is_empty(),
        tables.chain_pack,
        !tables.ctags.is_empty(),
        lazy_walk_cont(params, tables, search_attempts(params)),
    );
    macro_rules! go {
        ($k:expr) => {
            find_lazy_impl::<0, $k>(
                src,
                block_start,
                block_end,
                window,
                params,
                tables,
                depth,
                reps,
            )
        };
    }
    match kind {
        1 => go!(1),
        2 => go!(2),
        3 => go!(3),
        4 => go!(4),
        _ => go!(7),
    }
}

/// BRICK 58: the lazy finder's kernel shape as a const. 1..=4 are the
/// shipping chain shapes (packed / tag-array, each with and without
/// walk_cont) and are inlined; 7 is "through the pointer" (rows, tags off).
#[inline(always)]
fn lazy_kind(use_rows: bool, cp: bool, ca: bool, walk_cont: bool) -> u8 {
    match (use_rows, cp, ca, walk_cont) {
        (false, true, _, true) => 1,
        (false, true, _, false) => 2,
        (false, false, true, true) => 3,
        (false, false, true, false) => 4,
        _ => 7,
    }
}

/// The WALK-CONTINUE dispatch (see `walk_rep_max`), as the one expression
/// both `find_lazy_sel` and the finder body evaluate -- it reads block
/// state the body updates AFTER deciding, so the two must agree on order.
#[inline(always)]
fn lazy_walk_cont(params: CompressionParameters, tables: &MatchTables, attempts: usize) -> bool {
    walk_cont_enabled()
        // GATE 3's rule for the L1-routed case: `find_lazy` reachable with
        // `strategy == Fast` is the Gate 1 dispatch, and the C-parity walk
        // must not change the Fast ladder's bytes.
        && params.strategy != Strategy::Fast
        && tables.rep_yield <= walk_rep_max()
        && (tables.walk_first_share <= walk_first_max(attempts) || tables.walk_probe == 0)
}

/// BRICK 58: the search, resolved by the finder's `KIND`. For the four
/// shipping chain shapes this is `chain_find_best_inner` -- `inline(always)`,
/// so the walk lands here, in the finder's own frame; otherwise the block's
/// pointer, as before.
#[inline(always)]
fn lazy_search<const MLS: usize, const KIND: u8>(
    cfb: ChainFn,
    ctx: &ChainCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    match KIND {
        1 => chain_find_best_inner::<MLS, true, false, true>(ctx, ip, tables),
        2 => chain_find_best_inner::<MLS, true, false, false>(ctx, ip, tables),
        3 => chain_find_best_inner::<MLS, false, true, true>(ctx, ip, tables),
        4 => chain_find_best_inner::<MLS, false, true, false>(ctx, ip, tables),
        _ => cfb(ctx, ip, tables),
    }
}

#[allow(clippy::too_many_arguments)]
#[inline(always)]
fn find_lazy_impl<const MLS: usize, const KIND: u8>(
    src: &[u8],
    block_start: usize,
    block_end: usize,
    window: usize,
    params: CompressionParameters,
    tables: &mut MatchTables,
    depth: usize,
    reps: [u32; 3],
) -> (Vec<Seq>, Vec<u8>) {
    // BRICK 35 (K1): the CONTRACT's bound, both ends. `min_match` is
    // documented 3..=7 and `compression_params` / `apply_zstd_kv` both clamp
    // it there; only a hand-built `CompressionParameters` could carry more,
    // and zstd itself rejects such a value (`ZSTD_MINMATCH_MAX` is 7). With
    // the bound stated HERE, `mls_xor` needs no `mls > 8` arm -- which was a
    // compare and a branch on EVERY examined candidate in the chain walk,
    // and a callee-saved register holding `mls` for the walk's whole life.
    let mls = if MLS == 0 {
        params.min_match.clamp(3, 7) as usize
    } else {
        MLS
    };
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
    // E1: the row finder is selected ONCE per block, never per position -- an
    // atomic in the walk is the mistake brick 64b already paid for.
    let use_rows = row_find_enabled() && !tables.rows.head.is_empty();
    // W25: the back-fill's hash shifts, hoisted. The fill re-derived
    // `32 - hash_log` / `64 - hash_log` on EVERY inserted position -- and the
    // fill strides one byte, so that is once per matched byte.
    let f_shift32 = 32u32.saturating_sub(hash_log.min(32));
    let f_shift64 = 64u32.saturating_sub(hash_log.min(32));
    // D4: the BMI2 chain twin is retired -- 457 instructions of duplicated
    // walk converting THREE BMI2 ops, 152 per op, the worst ratio in the
    // crate. Same reasoning as W4/W5/W6.
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
    // W5: hoisted for the back-extension loop -- see its use.
    let fstart_c = tables.frame_start;
    let lowest_rep = block_start.saturating_sub(window).max(fstart_c);
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
    // SECTION 14.12: the back-fill stride is ROW-SCOPED.
    //
    // `fillsweep.rs` priced the fill's marginal value at L9 and it is steeply
    // diminishing -- but the two finders pay different prices for the same
    // thinning, so one default cannot serve both:
    //
    //   stride 2   fill inserts 0.52x   ROW +0.27%   CHAIN +0.34%
    //   stride 4   fill inserts 0.28x   ROW +0.97%   CHAIN +1.16%
    //
    // The row finder ships at stride 2 -- the knee. See `row_fill_stride`.
    //
    // The chain is the SHIPPING path; moving its stride moves `bytegate` GOLD
    // for every user and needs its own board across all 18 corpora and every
    // level. The row finder is opt-in and already bitstream-changing, so it
    // can take the thinning on its own gate (`rowboard`) without dragging the
    // default with it. Two decisions, two boards, attributable separately.
    let fill_stride = if use_rows {
        row_fill_stride()
    } else {
        lazy_fill_stride()
    };
    // WALK-CONTINUE dispatch: see `walk_rep_max`. BRICK 58: for the inlined
    // shapes it is the finder's const; the expression is still evaluated
    // (and asserted equal) in debug builds, dead in release.
    let walk_cont_rt = lazy_walk_cont(params, tables, attempts);
    let walk_cont = match KIND {
        1 | 3 => true,
        2 | 4 => false,
        _ => walk_cont_rt,
    };
    debug_assert_eq!(walk_cont, walk_cont_rt);
    tables.walk_probe = if tables.walk_probe == 0 {
        WALK_PROBE_PERIOD
    } else {
        tables.walk_probe - 1
    };
    tables.wcls = (0, 0);
    // ORDER IS LOAD-BEARING: `maybe_latch_wide_chain` can flip
    // `tables.chain_wide` for the REST of the frame, so every value below it
    // must be read AFTER it. Building the context any earlier captured the
    // pre-latch key and moved output -- lazyid caught it at L9/L12.
    maybe_latch_wide_chain(tables, src, block_start, window, mls);
    let cp = tables.chain_pack;
    let ca = !tables.ctags.is_empty();
    debug_assert!(
        KIND == 7 || lazy_kind(use_rows, cp, ca, walk_cont) == KIND,
        "find_lazy_impl: KIND disagrees with the block's shape"
    );
    // The kernel is selected ONCE per block, as a FUNCTION POINTER, and that
    // is the right shape -- REFUTED 2026-09-09, both alternatives, on the
    // emitted-asm count so it is not retried:
    //   * a direct branch `if use_rows { row(..) } else { chain(..) }` at
    //     each call site took the indirect calls 2 -> 0 but DUPLICATED the
    //     five-argument marshalling in both arms: per-position loop +18
    //     instructions, look-ahead loop +70, whole function 1472 -> 1470;
    //   * inlining `chain_find_best_inner` into the loop instead landed the
    //     331-instruction walk TWICE (one per call site): 1472 -> 1982, and
    //     the per-position loop went 458 instrs / 10 spills -> 464 / 20.
    // One indirect call with one marshalling sequence is the cheapest form
    // LLVM produces here. The two cfg arms this used to carry were identical
    // (the BMI2 twin they selected between is retired, D4); one is kept.
    //
    // BRICK 20: the chain kernel comes in three tag-representation shapes
    // (see `chain_find_best`); the block picks its own here, once.
    let cfb: ChainFn = if use_rows {
        row_find_best::<MLS>
    } else {
        // BRICK 24: `walk_cont` is the third axis (six kernels).
        match (cp, ca, walk_cont) {
            (true, _, true) => chain_find_best::<MLS, true, false, true>,
            (true, _, false) => chain_find_best::<MLS, true, false, false>,
            (false, true, true) => chain_find_best::<MLS, false, true, true>,
            (false, true, false) => chain_find_best::<MLS, false, true, false>,
            (false, false, true) => chain_find_best::<MLS, false, false, true>,
            (false, false, false) => chain_find_best::<MLS, false, false, false>,
        }
    };
    let wchain = tables.chain_wide;
    let smask = if mls >= 8 {
        u64::MAX
    } else {
        (1u64 << (8 * mls)) - 1
    };
    // W6: the same two facts the fill loop re-derived per inserted position.
    let wide_h = mls >= 8;
    // BRICK 74: the empty-head link for this block's producer.
    tables.set_null_tag(chain_null_tag(src, mls));
    // W1/W2: the walk's prologue, hoisted -- see `ChainCtx`.
    let chain_ctx = ChainCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts,
        hash_log: tables.hash_log,
        chain_mask: tables.chain.len() - 1,
        smask,
        cp,
        ca,
        wchain,
        wide_hash: mls >= 8,
        hash_mode: u8::from(wchain) | (u8::from(mls >= 8) << 1),
        hash_shift32: 32u32.saturating_sub(tables.hash_log.min(32)),
        hash_shift64: 64u32.saturating_sub(tables.hash_log.min(32)),
        lowest1: lowest_rep.max(1),
        lowest: lowest_rep,
        lowest_w: lowest_rep + window,
        rows_live: !tables.rows.head.is_empty(),
        tag_filter: cp || ca,
        walk_cont,
    };
    // GATE 13, which this finder never received. Every other finder resolves
    // the literal-copy width once per block and emits through `push_literals`;
    // `find_lazy_impl` alone still called `push_lits_range`, so its
    // per-sequence literal appends went out through `extend_from_slice` -- a
    // `memcpy` CALL, measured at 1,632,910 of them at L9 for a mean run of
    // 3.51 bytes. Same expression as `find_greedy_impl` and `find_bt_lazy`.
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
    let gain_cmp = lazy_gain_enabled();
    // Hoisted per BLOCK: an atomic load per position would cost more
    // than the positions it skips.
    let accel_sh = lazy_step_shift(lazy_accel());
    // BRICK 95 (P23): the anchor in its folded form for the no-match step.
    let mut anchor_adj = anchor_adj_of(anchor, accel_sh);
    // BRICK 19: the fill's block constants, once, by reference.
    let fill_ctx = FillCtx {
        stride: fill_stride,
        shift32: f_shift32,
        shift64: f_shift64,
        smask,
        mls,
        chain_mask,
        wide_h,
        wchain,
        cp,
        ca,
    };
    // BRICK 38 (P2): the rep probe's four admission tests, as one bound on
    // `ip` -- see `rep_bar_for`. Refreshed where `rep1` changes.
    let mut rep_bar = rep_bar_for(use_rep, rep1, lowest_rep);
    while ip <= ilimit {
        if ip >= rep_bar {
            debug_assert!(use_rep && rep1 != 0 && ip + 1 >= rep1 + lowest_rep);
            debug_assert_eq!(
                rep1_len(src, ip + 1, ip + 1 - rep1, block_end),
                try_rep1(src, ip, rep1, lowest_rep, block_end, ilimit)
            );
            // BRICK 86 (P21): the probe's width from `ip < ilimit` -- the same
            // question as `ip + 9 <= block_end`, on operands the loop holds.
            if let Some(ml) = rep1_len_w(src, ip + 1, ip + 1 - rep1, block_end, ip < ilimit) {
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
                anchor_adj = anchor_adj_of(anchor, accel_sh);
                continue;
            }
        }
        searches += 1;
        let (mut best_m, mut best_ml) = lazy_search::<MLS, KIND>(cfb, &chain_ctx, ip, tables);
        // BRICK 37 (P1): ONE test per position. `cfb` returns `(0, 0)` or a
        // length that already cleared `mls` (W9), the look-ahead below only
        // ever raises `best_ml`, and the emit used to re-test `best_ml != 0`
        // after the look-ahead's `best_ml >= mls` -- the same predicate, so
        // the second `test`/`je` and the `mls` reload were pure overhead on
        // every position, and `best_ip`/`look_hi` were spilled across the
        // join on the no-match path, where nothing reads them.
        if best_ml != 0 {
            debug_assert!(best_ml >= mls);
            let mut best_ip = ip;
            let mut look_hi = ip; // PROBE: highest position the look-ahead inserted
                                  // W3: the in-hand match's gain, carried with it. MOVED INSIDE this
                                  // guard -- its only two readers are in this block, and computing it
                                  // above cost a multiply plus a `leading_zeros` on every position
                                  // where the walk found NOTHING, which is the common case at L9
                                  // (chain hit rate 3-10%).
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
                let (m, ml) = lazy_search::<MLS, KIND>(cfb, &chain_ctx, ip2, tables);
                // W3: `lazy_gain(best_ml, best_ip - best_m)` describes the
                // match ALREADY IN HAND, so it changes only when that match
                // does -- but it was recomputed on every look-ahead step (a
                // multiply, a `leading_zeros` and two subs). Carried beside
                // the best it describes, and refreshed only on improvement.
                // W9: `cfb` returns either `(0, 0)` or a length that already
                // cleared its own `>= mls` bar, so `ml >= mls` IS `ml != 0` --
                // a test against zero instead of against a value that has to
                // stay live.
                //
                // W8: and when the test passes, the gain it computed for THIS
                // candidate is exactly the new best's gain. It was thrown away
                // and rebuilt one line later (a multiply and a
                // `leading_zeros`, on every improvement).
                // `lazy_gain` is a multiply plus a `leading_zeros`, and it ran
                // on EVERY look-ahead step -- including the ones where `cfb`
                // found nothing. At L9 the chain's hit rate is 3-10%, so
                // `ml == 0` is the COMMON outcome, and the `take` test below
                // discards the gain in exactly that case. Guarded so the
                // arithmetic runs only when there is a candidate to describe.
                if gain_cmp {
                    if ml != 0 {
                        let cand_gain = lazy_gain(ml, ip2 - m);
                        // C parity: the +4 favors the match already in hand.
                        if cand_gain > best_gain + 4 {
                            best_ml = ml;
                            best_m = m;
                            best_ip = ip2;
                            best_gain = cand_gain;
                        }
                    }
                } else if ml > best_ml {
                    best_ml = ml;
                    best_m = m;
                    best_ip = ip2;
                }
            }
            // W10: same identity as W9 -- `best_ml` is 0 or already past `mls`,
            // and the look-ahead never lowers it. BRICK 37 (P1) folded the emit
            // into the guard above on exactly that identity.
            debug_assert!(best_ml >= mls);
            // DEFECT B3 FIX: back-extend the match -- see `find_greedy`.
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
            // The repcode must track the offset ACTUALLY EMITTED. Lazy
            // commits at `best_ip` (the look-ahead winner), not `ip`.
            rep1 = best_ip - best_m;
            // BRICK 85 (P20): `rep1` is a match offset here, never 0, so the
            // helper's `rep1 != 0` arm is dead on this path.
            debug_assert!(rep1 != 0);
            debug_assert_eq!(
                if use_rep {
                    rep1 + lowest_rep - 1
                } else {
                    usize::MAX
                },
                rep_bar_for(use_rep, rep1, lowest_rep)
            );
            rep_bar = if use_rep {
                rep1 + lowest_rep - 1
            } else {
                usize::MAX
            };
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
                let p = (best_ip + 1).max(look_hi + 1);
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
                // W29: ONE bound, not two. `end` and `ilimit` are both fixed
                // for this fill, so `p < end && p <= ilimit` folds -- exactly
                // the fold `find_greedy_impl`'s fill already had (its W7) and
                // this one never received.
                let stop = end.min(ilimit + 1);
                // BRICK 10: OUTLINED. This is the per-MATCHED-BYTE loop -- it runs once per
                // byte of every match at L6-L12 -- and it lived inline here in six copies
                // (three hash arms x two inserters), each reloading the chain mask, the
                // chain base and its bounds from spill slots on every byte: the census read
                // 22 instructions and 4 stack reloads per inserted position. In its own
                // frame those invariants are register-resident for the loop's whole life;
                // the caller pays one call per match, amortised over the match's length.
                if use_rows {
                    row_fill_range(&mut tables.rows, src, p, stop, &fill_ctx);
                } else {
                    if cp {
                        lz_fill_range::<false, true, false, true>(tables, src, p, stop, &fill_ctx);
                    } else if ca {
                        lz_fill_range::<false, false, true, true>(tables, src, p, stop, &fill_ctx);
                    } else {
                        lz_fill_range::<false, false, false, true>(tables, src, p, stop, &fill_ctx);
                    }
                }
            }
            ip = end;
            anchor = ip;
            anchor_adj = anchor_adj_of(anchor, accel_sh);
        } else {
            // C: `ip += ((ip-anchor) >> kSearchStrength) + 1`.
            debug_assert_eq!(
                lazy_step_adj(ip, anchor_adj, accel_sh),
                lazy_step(ip, anchor, accel_sh)
            );
            ip += lazy_step_adj(ip, anchor_adj, accel_sh);
        }
    }
    // Same shipping tail as `find_greedy_impl`, so it is the SAME helper --
    // one stamp instead of four across the two finders and their bmi2 twins.
    // Lazy reports probes=0 (its probe count arrives via `note_probes`) and
    // hits=nseq, exactly as its inline tail did. The profile-only WALK_SIG
    // stores move BELOW the call: they read the post-update EWMAs, which the
    // helper has written by the time it returns, and they publish to
    // independent statics, so order against `note_finder_work` is immaterial.
    let wcls = tables.wcls;
    greedy_finder_epilogue(
        tables,
        src,
        &seqs,
        &mut lits,
        anchor,
        block_start,
        block_end,
        rep_hits,
        walk_cont,
        wcls,
        attempts,
        searches,
        0,
        seqs.len() as u64,
    );
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
        let ob: u64 = seqs
            .iter()
            .map(|q| 64 - u64::from(q.offset.max(1)).leading_zeros() as u64)
            .sum();
        WALK_SIG_MB.store(mb, Relaxed);
        WALK_SIG_NS.store(seqs.len() as u64, Relaxed);
        WALK_SIG_OB.store(ob, Relaxed);
    }
    // `searches` is SEARCH POSITIONS, not candidate examinations -- reporting it
    // as `probes` was a work-count parity break against `find_fast`. The real
    // probe count comes from `chain_find_best` via `note_probes`, so pass 0.
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
static BT_DEEP_ARM: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

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
static BT_DEPTH_REP_C: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);
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
        let v = crate::env_knob_parse("RZSTD_BT_DEPTH_TARGET")
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
        let v: f32 = crate::env_knob_parse("RZSTD_BT_DEPTH_REP").unwrap_or(50.0);
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
        let v = crate::env_knob_parse("RZSTD_BT_DEPTH_SLOG").unwrap_or(7);
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
        let v = crate::env_knob_parse("RZSTD_BT_DEPTH").unwrap_or(1);
        BT_DEPTH_STEPS_C.store(v, Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1
}

pub static BT_WALKS2: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static BT_ITERS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static BT_FULL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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

// The `(hash_log, chain_log)` specialisation of the binary-tree walk is
// RETIRED. `bt_resolve` had returned the runtime body on every path since
// the spec copies were culled for I-cache density (6,003 + 5,280
// instructions of monomorphs), but the pairs list, the dead `_spec` /
// `_impl` bodies, the `RZSTD_BT_SPEC` knob and a coverage test that asserted
// against the list all outlived the dispatch they described -- the test
// passed while selecting nothing. Brick 8 removed them together.

/// The dispatch, RESOLVED ONCE PER BLOCK: `(hash_log, chain_log)` is
/// loop-invariant in every caller, yet `bt_find_best` re-ran a jump-table
/// dispatch (plus re-reading both fields) on every call -- per position,
/// per look-ahead, per fill insert and per DP edge. Callers hoist a fn
/// pointer instead; one predictable indirect call replaces the dance.
/// Same arms, same runtime fallback, same bt_spec parity gate.
/// The per-block-constant arguments of every bt call, packed: the fn
/// pointer previously re-marshaled NINE scalars per position, per
/// look-ahead step, per fill insert and per DP edge.
/// The per-block tree geometry: the node mask, and whether the tree fits the
/// chain table at all (BRICK 34). `bt_log` floors at 1, so the largest index
/// the walk can form is `(bt_mask << 1) | 1` -- the worst-case guard the
/// kernel used to re-derive on every call.
#[inline]
fn bt_geom(chain_log: u32, chain_len: usize) -> (usize, bool) {
    let bt_log = chain_log.min(24).saturating_sub(1).max(1);
    let bt_mask = (1usize << bt_log) - 1;
    (bt_mask, ((bt_mask << 1) | 1) < chain_len)
}

pub(crate) struct BtCtx<'a> {
    src: &'a [u8],
    /// BRICK 34: the tree geometry and both hash shifts, derived ONCE per
    /// block instead of on every call -- and this kernel is called per
    /// position, per look-ahead step and per fill insert, so its prologue
    /// is the hottest per-call unit in the Bt ladder (123 instructions
    /// against a 133-instruction walk loop). See `bt_geom`.
    bt_mask: usize,
    bt_shift32: u32,
    bt_shift64: u32,
    bt_ok: bool,
    block_start: usize,
    block_end: usize,
    window: usize,
    mls: usize,
    attempts: usize,
    chain_log: u32,
    /// W1/W2/W3: three values the walk's PROLOGUE recomputed on every call --
    /// and this function is called per position, per look-ahead step AND per
    /// fill insert (61.9% of all tree work at L13-L15), so "per call" is the
    /// hottest unit in the Bt ladder.
    ///
    /// `bt_lowest` is `block_start.saturating_sub(window).max(frame_start)`: a
    /// saturating sub, a max, and a struct load through `&mut MatchTables`
    /// that LLVM must re-prove after every `chain` write. `chain_len` served
    /// the entry guard, another struct load. Both are fixed for the block.
    bt_lowest: usize,
    chain_len: usize,
    /// W6: `hash_mls`'s `mls >= 8` question, answered once per BLOCK. The walk
    /// loaded `mls` from the context and compared it on EVERY call. Kept as a
    /// flag rather than deleted: the advanced API can set `min_match` to 8,
    /// which other guards in this file already respect, even though every
    /// shipping Bt row uses 3..=5.
    wide_hash: bool,
}

type BtFn = for<'a> fn(&BtCtx<'a>, usize, &mut MatchTables) -> (usize, usize);

/// The INSERT dispatch's own type. Insert callers discard the result -- both
/// fills and the priming pass -- but the `BtFn` signature forced every insert
/// trampoline to MATERIALISE one: the emitted wrapper built a 40-byte frame,
/// made a real call, then zeroed `rax`/`rdx` to return `(0, 0)`, on 61.9% of
/// all tree work at L13-L15. Returning `()` lets the same wrapper compile to
/// a bare tail `jmp`, which is what the SEARCH side already gets.
type BtInsFn = for<'a> fn(&BtCtx<'a>, usize, &mut MatchTables);

fn bt_rt_search(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    bt_find_best_runtime(true, ctx, ip, t)
}

fn bt_rt_insert(ctx: &BtCtx, ip: usize, t: &mut MatchTables) -> (usize, usize) {
    bt_find_best_runtime(false, ctx, ip, t)
}

/// `bt_resolve` for the insert side -- same table, `BtInsFn` shape.
fn bt_resolve_ins(_hash_log: u32, _chain_log: u32) -> BtInsFn {
    // Runtime insert only -- see `bt_resolve`.
    bt_rt_ins_plain
}
fn bt_resolve<const SEARCH: bool>(_hash_log: u32, _chain_log: u32) -> BtFn {
    // Runtime body only; the specialisation and its BMI2 twins are retired
    // (see the note where `BT_SPEC_PAIRS` used to live). The two parameters
    // are kept so the call sites read as the dispatch they once were.
    if SEARCH {
        bt_rt_search
    } else {
        bt_rt_insert
    }
}

fn bt_rt_ins_plain(ctx: &BtCtx, ip: usize, t: &mut MatchTables) {
    bt_find_best_runtime(false, ctx, ip, t);
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

#[inline(always)]
fn bt_find_best_runtime_inner(
    search: bool,
    ctx: &BtCtx,
    ip: usize,
    tables: &mut MatchTables,
) -> (usize, usize) {
    let BtCtx {
        src,
        block_start,
        block_end,
        window,
        mls,
        attempts,
        chain_log,
        bt_lowest,
        chain_len,
        wide_hash,
        bt_mask,
        bt_shift32,
        bt_shift64,
        bt_ok,
    } = *ctx;
    debug_assert_eq!(wide_hash, mls >= 8);
    debug_assert_eq!(chain_len, tables.chain.len());
    debug_assert_eq!(
        bt_lowest,
        block_start.saturating_sub(window).max(tables.frame_start)
    );
    // Diagnostic ONLY -- gated. Unguarded this was one atomic read-modify-write
    // per `bt_find_best` CALL, i.e. per POSITION across the whole L13-L22
    // ladder (~15.7M per level per corpus set). Same defect class as the two
    // per-probe atomics removed from `fast_probe`, which were worth +6.97%.
    // `take_bt_calls` therefore needs `--features rusty_zstd/profile`.
    if cfg!(feature = "profile") {
        BT_RUNTIME_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    // BRICK 34: the geometry and both shifts are BLOCK constants, built once
    // in `BtCtx` (see `bt_geom`). They were derived on every call, and the
    // call count here is per position + per look-ahead + per fill insert.
    // The assertions restate the derivations the context now owns; the
    // worst-case guard (`bt_log` floors at 1, so `chain.len() >= 4`) is
    // `bt_ok`, and it still closes the `chain_log = 1` edge the advanced API
    // can reach.
    debug_assert_eq!(
        (bt_mask, bt_ok),
        bt_geom(chain_log, chain_len),
        "BtCtx geometry disagrees with its own chain_log/chain_len"
    );
    debug_assert_eq!(bt_shift32, 32u32.saturating_sub(tables.hash_log.min(32)));
    debug_assert_eq!(bt_shift64, 64u32.saturating_sub(tables.hash_log.min(32)));
    let _ = chain_log;
    if !bt_ok {
        return (0, 0);
    }
    // W47: `src.len()` is a slice field re-read on the same line.
    let bt_src_len = src.len();
    // BRICK 99 (K18): no 8-byte-hash arm -- `mls >= 8` is outside the contract
    // (the three `BtCtx` builders clamp it), so the flag test, the add and the
    // compare against `src_len` ran on every call for a case that cannot arrive.
    debug_assert!(!wide_hash);
    let _ = (bt_shift64, bt_src_len);
    let h = hash4_shift(load_u32le(src, ip), bt_shift32);
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
    // W3: hoisted into `BtCtx` -- see its definition.
    // Hoisted: the per-node window test `ip - m > window` is `m < ip - window`
    // (m < ip is tested first), one cmp against a per-call constant instead
    // of sub+cmp per node.
    let win_low = ip.saturating_sub(window);
    // W4: the single hot-path lower bound (see the walk's break).
    let low = if win_low > bt_lowest {
        win_low
    } else {
        bt_lowest
    };
    // W1: the count head's only non-`m` precondition, hoisted out of the walk.
    // See the head itself for why the other two tests are implied.
    debug_assert!(block_end <= src.len());
    let head_ok = ip + 8 <= block_end;
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
        // W4 RETRIED: the walk tested TWO lower bounds per node, and both were
        // SPILLED -- two stack reloads and two compares on the hottest path in
        // the Bt ladder. They collapse to one compare against their max, with
        // the disambiguation moved into the break (taken once per walk).
        //
        // This was tried once before and REVERTED: it destabilised the
        // register allocator and the node path came back at 60 instructions.
        // The blocker was live-set pressure, and the prologue hoist above has
        // since removed `chain_len` and `frame_start` from it -- so the trade
        // is re-measured, not re-assumed.
        if m >= ip || m < low {
            if m >= ip || m < win_low {
                tables.chain_set(smaller, 0);
                tables.chain_set(larger, 0);
            }
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
        // BRICK 27: the children are read AFTER the write below, one per node
        // (see the `go_smaller` arms). The eager pair that lived here spilled
        // both words to the stack and reloaded them on the same path.
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
        // W1 GUARD COLLAPSE: the three-test head was two loop-INVARIANT
        // tests plus one redundant one. `block_end <= src.len()` (it is a
        // position in `src`) makes `ip + 8 <= src.len()` follow from
        // `ip + 8 <= block_end`, and `m < ip` -- proven by the break above --
        // makes `m + 8 <= src.len()` follow too. What is left does not depend
        // on `m`, so it leaves the loop entirely: `head_ok`, computed once
        // per walk.
        //
        // W2 DIRECTION BY BSWAP: the descent needs the ORDER of the two byte
        // strings, and `mb < ib` at the first differing byte IS lexicographic
        // order -- which big-endian u64 comparison gives directly. Two
        // `bswap`+`cmp` replace `and`+two `shrx`+`cmp`, and, more importantly,
        // the branch no longer waits on `bsf`: direction and length are now
        // INDEPENDENT chains instead of one serial dependency.
        //
        // W3 INSERT-ONLY LENGTH ELISION falls out of W2: with direction no
        // longer derived from `ml`, the SEARCH = false copies (both fills and
        // the priming pass -- 61.9% of all tree work at L13-L15) have no
        // reader for the head path's `ml` at all, so the whole
        // `bsf`/`shr` chain dead-codes away in those monomorphisations.
        //
        // Byte-identical on every path: same `ml` where `ml` is read, and the
        // same direction bit.
        let (ml, go_smaller) = if head_ok {
            let a = load_u64le(src, m);
            let b = load_u64le(src, ip);
            if a != b {
                (
                    ((a ^ b).trailing_zeros() as usize) >> 3,
                    a.swap_bytes() < b.swap_bytes(),
                )
            } else {
                let ml = 8 + count_match_fast(src, m + 8, ip + 8, block_end);
                let mb = src.get(m + ml).copied().unwrap_or(0);
                let ib = src.get(ip + ml).copied().unwrap_or(0);
                (ml, mb < ib)
            }
        } else {
            let ml = count_match(src, m, ip, block_end);
            let mb = src.get(m + ml).copied().unwrap_or(0);
            let ib = src.get(ip + ml).copied().unwrap_or(0);
            (ml, mb < ib)
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
        if go_smaller {
            tables.chain_set(smaller, m as u32);
            // BYTE-IDENTICAL (BRICK 27): read after the write. If the store
            // above targeted this slot the read returns `m` -- exactly what the
            // forwarding compare used to select -- and the untouched word
            // otherwise; nothing else writes the tree between the two.
            let v = tables.chain_at(bt_idx + 1);
            smaller = bt_idx + 1;
            match_idx = if v == 0 { None } else { Some(v as usize) };
        } else {
            tables.chain_set(larger, m as u32);
            let v = tables.chain_at(bt_idx);
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

/// The tree ladder (`find_bt_lazy`, `find_opt`) lives in `encode/tree.rs`.
mod tree;
pub use tree::*;

/// The BYTE half of `match_ok`, exactly (u32 head + tail slice to `mls`).
/// Split out for the chain walk: validity is MONOTONE along a chain (positions
/// strictly decrease), so a validity failure correctly ends the walk -- but a
/// byte mismatch is just a hash collision, and C's `ZSTD_HcFindBestMatch`
/// steps past it to the next link. Our walk broke on it, amputating the
/// remaining chain at the first collision.
// Since BRICK 11 the walks use `mls_xor`; the boolean form serves only the
// profile-build census sites.
#[cfg_attr(not(feature = "profile"), allow(dead_code))]
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
        debug_assert!(
            smask
                == if mls == 8 {
                    u64::MAX
                } else {
                    (1u64 << (8 * mls)) - 1
                }
        );
        return (load_u64le(src, m) ^ load_u64le(src, ip)) & smask == 0;
    }
    mls_eq_wide(src, m, ip, mls)
}

/// The `mls > 8` arm of `mls_eq`, OUTLINED AND COLD.
///
/// No shipping row has `min_match` above 7 (the tables pin 3..=7; only the
/// advanced API can ask for 8+), so this arm never runs in production -- yet
/// it was inlined into every walk that calls `mls_eq`: the greedy walk, the
/// chain walk and the row walk, six call sites. Each copy carried a slice
/// construction, TWO bounds-check guards (the `src[m + 4..m + mls]` slicing,
/// encode.rs:14004 by the panic-Location census) and a `call memcmp`, sitting
/// in the hot walk's cache lines. That is where every one of `find_greedy`'s
/// guard branches lived, and the only `memcmp` in the matchfind symbols.
///
/// Deterministic verdict: guards on the walk paths 6 -> 0, `memcmp` sites
/// 6 -> 1, byte-identical by construction (same predicate, same bytes).
#[cfg_attr(not(feature = "profile"), allow(dead_code))]
#[cold]
#[inline(never)]
fn mls_eq_wide(src: &[u8], m: usize, ip: usize, mls: usize) -> bool {
    if load_u32le(src, m) != load_u32le(src, ip) {
        return false;
    }
    src[m + 4..m + mls] == src[ip + 4..ip + mls]
}

/// `mls_eq` that RETURNS THE XOR it computed (BRICK 11) -- the fused head
/// that `fast_probe_wide` has had since W2 and the chain walks never got.
///
/// The walks tested `(load8(m) ^ load8(ip)) & smask == 0`, threw the xor
/// away, and then `count_match_fast(m + mls, ip + mls)` LOADED BOTH WORDS
/// AGAIN and xor'd them again to find the first differing byte. But when the
/// first xor is non-zero, its lowest set byte IS the match length: bytes
/// below `mls` are equal by the mask, so the difference sits at index
/// `>= mls`, and every walk holds `ip <= ilimit = block_end - 8`, so that
/// index is inside the block without a clamp. Only a zero xor -- all eight
/// bytes equal -- needs the counter, and it can start at 8. Identical to
/// `mls + count_match_fast(src, m + mls, ip + mls, block_end)` on every arm.
///
/// BRICK 35 (K1) removed the wide (`mls > 8`) arm: the chain-ladder finders
/// bound `mls` to the 3..=7 contract at their derivation, so the arm select
/// was a compare and a branch per examined candidate for a case that cannot
/// arrive.
#[inline(always)]
fn mls_xor(src: &[u8], m: usize, ip: usize, mls: usize, smask: u64) -> Option<u64> {
    // BRICK 35 (K1): no wide arm. Every caller derives `mls` through the
    // finders' `clamp(3, 7)`, so `mls <= 8` is not a per-candidate question;
    // the `mls_eq_wide` route stays available to the profile census's
    // `mls_eq` only.
    debug_assert!(mls <= 8, "mls_xor: min_match above the 3..=7 contract");
    debug_assert!(m < ip && ip + 8 <= src.len());
    let x = load_u64le(src, m) ^ load_u64le(src, ip);
    if x & smask == 0 {
        Some(x)
    } else {
        None
    }
}

/// BRICK 56 (K7): `fused_ml`'s long continuation (`x == 0`, the first eight
/// bytes matched), OUTLINED behind the walk context so the chain walk's
/// loop carries nothing for it -- `ip + 8` and `ip + 16` were hoisted call
/// arguments, spilled at every kernel entry, in a loop already one register
/// short. Called on `FUSED_LONG` only (~0.04 per byte at L9).
#[inline(never)]
fn walk_count8(ctx: &ChainCtx, m: usize, ip: usize) -> usize {
    #[cfg(feature = "profile")]
    FUSED_LONG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    8 + count_match_fast(ctx.src, m + 8, ip + 8, ctx.block_end)
}

/// The length that goes with `mls_xor`'s `Some(x)`.
#[inline(always)]
fn fused_ml(x: u64, src: &[u8], m: usize, ip: usize, block_end: usize) -> usize {
    if x != 0 {
        #[cfg(feature = "profile")]
        FUSED_SHORT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        (x.trailing_zeros() as usize) >> 3
    } else {
        #[cfg(feature = "profile")]
        FUSED_LONG.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        8 + count_match_fast(src, m + 8, ip + 8, block_end)
    }
}

#[cfg(feature = "profile")]
static FUSED_SHORT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
static FUSED_LONG: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// (short, long) fused-head resolutions since the last call -- BRICK 11's verdict.
#[cfg(feature = "profile")]
pub fn take_fused() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (FUSED_SHORT.swap(0, Relaxed), FUSED_LONG.swap(0, Relaxed))
}

/// WALK-CONTINUE arm: C-parity chain walk (step past byte mismatches).
/// Byte-CHANGING (finds matches the amputated walk missed), so it ships on
/// the adjudication board in `chainwalk`, not on byte-identity.
static WALK_CONT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the walk-continue arm.
pub fn set_walk_cont_arm(on: bool) {
    WALK_CONT_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
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
    LAZY_GAIN_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
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
static WALK_FIRST_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the first-find share above which the C-parity walk latches off.
pub fn set_walk_first_max_arm(v: f32) {
    WALK_FIRST_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

/// Latch for the env escape hatch below: 0 = not yet looked up, 1 = looked up.
/// `walk_first_max` is read once per BLOCK, so the parse must not repeat.
static WALK_FIRST_ENV: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

fn walk_first_max(attempts: usize) -> f32 {
    use core::sync::atomic::Ordering::Relaxed;
    let c = WALK_FIRST_ARM.load(Relaxed);
    if c != u32::MAX {
        return f32::from_bits(c);
    }
    // ESCAPE HATCH. The ladder below is a deliberate size-for-speed trade, and
    // a caller who wants the old ratio back must be able to get it without
    // rebuilding: `RZSTD_WALK_FIRST_MAX=0.70` restores the pre-trade bitstream
    // at L7/L9. Setting it pins ONE bar for every level, exactly as
    // `set_walk_first_max_arm` does -- the `attempts` scaling is what the
    // unset path provides.
    if WALK_FIRST_ENV.swap(1, Relaxed) == 0 {
        if let Some(v) = crate::env_knob_parse::<f32>("RZSTD_WALK_FIRST_MAX") {
            WALK_FIRST_ARM.store(v.to_bits(), Relaxed);
            return v;
        }
    }
    // The first-find share of BOTH classes falls as the walk deepens, so the
    // bar scales with `attempts` (swept: L5 wants 0.80 -- greedy is
    // first-heavy by construction -- L7/L9 want 0.70, L12 wants 0.55; a
    // static bar leaks jsonlog at one level or over-shuts dickens at
    // another).
    // Actual ladder attempts (clevels.h search_log): L5=8, L7/L9=16, L12=64.
    //
    // TIGHTENED BY 0.15 FROM THAT LADDER. This is a deliberate SIZE-FOR-SPEED
    // trade, the only one in this encoder, and it is the largest speed lever
    // the campaign found:
    //
    //     level   size      speed     probes
    //     L5     +0.351%    +5.2%      -2.8%
    //     L7     +1.213%   +11.1%     -11.6%
    //     L9     +1.297%   +13.5%     -13.0%
    //     L12    +1.388%   +38.1%     -15.1%
    //
    // L12 gains far more time than it drops probes because the walks this cuts
    // are the DEEP, cache-cold ones, not average ones -- the bar removes the
    // most expensive probes first. That is also why the size cost stays near
    // 1% while the time falls by a third.
    //
    // MEASUREMENT. The speed column is an INTERLEAVED A/B (`firstbar.rs`) --
    // arms alternating inside one process with a per-arm best. Run as separate
    // passes on a loaded host the arms drift apart and this same delta swung
    // from -3.9% to +24.8% on identical config. Do not re-adjudicate this with
    // two `cargo run`s and a stopwatch.
    //
    // The cost is NOT uniform across corpora: the worst single corpus is
    // reymont at +6.94% (L12) and xml at +5.19% (L7). A caller who picks a
    // level for ratio alone, on text-like input, is the one who pays. That is
    // what `RZSTD_WALK_FIRST_MAX` / `set_walk_first_max_arm` are for -- setting
    // the bar back to 0.80/0.70/0.55 restores the old bitstream exactly.
    if attempts <= 8 {
        0.65
    } else if attempts <= 16 {
        0.55
    } else {
        0.40
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
    REP_REPROBE_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn rep_reprobe_enabled() -> bool {
    // DEFAULT OFF -- REFUTED on its board (repro, 18 corpora x L5-L12):
    // totals -0.04% / +0.01% / +0.00% / +0.03%, worst xml +1.57% at L12.
    // The latch DFast paid for costs nothing here: the chain walk finds the
    // same matches the reopened rep search would, and reopening on
    // rep-hostile blocks trades offset economy for nothing. Arm kept for
    // study.
    matches!(
        REP_REPROBE_ARM.load(core::sync::atomic::Ordering::Relaxed),
        2
    )
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
/// E1 arm: the ROW match finder replaces the hash-chain walk on the lazy
/// ladder. **Bitstream-CHANGING** -- a row holds the last 16 positions for its
/// bucket where the chain held all of them, so the candidate set differs and
/// the encoder finds different matches. Ships on `examples/rowboard.rs`
/// (round-trip + compressed size per corpus) or not at all. Defaults OFF.
///
/// Only the LAZY ladder is wired: `find_greedy_impl` carries its own hand-copied
/// walk rather than going through `ChainFn`. Lazy is where the loads are anyway
/// -- 139M at L7, 221M at L9, 674M at L12, against greedy's 43M at L5.
/// Restore the row arm to AUTO (the shipped default): size-gated by
/// `row_auto_ok`. `set_row_arm` FORCES and cannot express this, which is what
/// made a single-process arm board impossible to baseline correctly.
pub fn set_row_arm_auto() {
    ROW_ARM.store(0, core::sync::atomic::Ordering::Relaxed);
}

pub fn set_row_arm(on: bool) {
    ROW_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// 0 = AUTO (size-gated, see `row_auto_ok`), 1 = forced off, 2 = forced on.
static ROW_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);
#[inline(always)]
pub(crate) fn row_find_enabled() -> bool {
    // AUTO and forced-on both permit; the ALLOCATION decides for AUTO, and
    // `use_rows` is this AND `!rows.head.is_empty()`, so an unallocated
    // table keeps the chain regardless.
    ROW_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// The measured AUTO band for the row match finder.
///
/// A row holds the last 16 positions for its bucket where the chain held
/// all of them, trading DEPTH for RECENCY -- and recent means SMALL
/// OFFSETS, which cost fewer bits. While the window is not yet full the row
/// gives up almost no depth and banks the offset saving; once the chain is
/// deep it finds matches the row cannot. So the verdict is monotone in
/// SOURCE LENGTH, and `rowcross.rs` measures the crossover (14 corpora,
/// every cell round-tripped):
///
/// ```text
///   L9   256K 1.0059 | 512K 0.9882 | 1M 0.9882 | 2M 0.9894 | 4M 0.9944 | 6M 1.0018
///   L7   256K 1.0078 | 512K 0.9866 | 1M 0.9880 | 2M 0.9895 | 4M 0.9936 | 6M 1.0002
///   L12  ---         | 512K 0.9959 | 1M 0.9968 | 2M 0.9992 | 4M 1.0070 | 6M 1.0163
/// ```
///
/// 512 KiB..2 MiB is the band that wins at EVERY level measured, so that is
/// the band taken; 4 MiB would still pay at L7/L9 but costs at L12, and the
/// strategy enum cannot separate L9 from L12. Below 512 KiB the row loses
/// (the chain is shallow there too, so recency buys nothing).
///
/// In the band this is a DOUBLE win: 1.1-1.3% smaller AND 2.5-8.5x fewer
/// dependent loads (`ROW_LOADS` vs `WALK_EXAM`).
///
/// NOT content-dispatched: literal share, mean match length and
/// sequences/KiB were each tested against the per-corpus win/loss split and
/// all three OVERLAP, so no content threshold separates them (`rowsig.rs`).
const ROW_AUTO_MIN: u64 = 512 << 10;
const ROW_AUTO_MAX: u64 = 2 << 20;

#[inline]
fn row_auto_ok(params: CompressionParameters, src_len: Option<u64>) -> bool {
    match ROW_ARM.load(core::sync::atomic::Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            matches!(params.strategy, Strategy::Lazy | Strategy::Lazy2)
                && matches!(src_len, Some(n) if (ROW_AUTO_MIN..=ROW_AUTO_MAX).contains(&n))
        }
    }
}
/// WIDE-CHAIN LATCH census: `[events, positions_rescanned]`. The latch does a
/// full O(window) chain rebuild when it fires; this is what that costs.
#[cfg(feature = "profile")]
pub static WIDE_LATCH: [crate::census64::AtomicU64; 2] =
    [const { crate::census64::AtomicU64::new(0) }; 2];

/// Read and clear `(latch_events, positions_rescanned)`.
#[cfg(feature = "profile")]
pub fn take_wide_latch() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        WIDE_LATCH[0].swap(0, Relaxed),
        WIDE_LATCH[1].swap(0, Relaxed),
    )
}

/// CHAIN-WALK EXIT CENSUS. Which of the seven ways the walk can end actually
/// fires, indexed:
///   0 empty bucket (`prev` is None -- the walk never starts)
///   1 entry guard (`m >= ip` or no room for `mls`)
///   2 window bound (`m < low`)
///   3 LINK GUARD (`next >= m`) -- the chain link did not go backwards
///   4 match reached `block_end`
///   5 attempts exhausted (the walk ran its full depth)
///   6 `walk_cont` off, stopped on a tag or byte miss
///
/// Built to test one claim: that shrinking `hash_log`/`chain_log` collapses the
/// probe count (1.845 -> 0.553 per byte) by making index 3 fire sooner. A
/// smaller `chain_mask` aliases many positions onto one slot, so the link a
/// walk reads may belong to some other position entirely.
#[cfg(feature = "profile")]
pub static WALK_EXIT: [crate::census64::AtomicU64; 8] =
    [const { crate::census64::AtomicU64::new(0) }; 8];

/// Read and clear the walk-exit census.
#[cfg(feature = "profile")]
pub fn take_walk_exit() -> [u64; 8] {
    use core::sync::atomic::Ordering::Relaxed;
    let mut o = [0u64; 8];
    for (i, sl) in o.iter_mut().enumerate() {
        *sl = WALK_EXIT[i].swap(0, Relaxed);
    }
    o
}

/// E1 work counter: candidates examined by the ROW walk, the direct
/// counterpart of `WALK_EXAM`. The row finder's whole claim is that it reaches
/// the same candidates with FEWER DEPENDENT LOADS, so both numbers are needed:
/// `ROW_EXAM` is candidates, `ROW_LOADS` is rows touched (one load each).
#[cfg(feature = "profile")]
pub static ROW_EXAM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
/// SECTION 14.9 census: the 16-to-1 bucket-sharing cost.
/// `[examined, same_bucket, mls_eq_pass, gtag0_probes, probes]`.
/// A row folds 16 hash buckets, so a tag match does NOT imply a bucket match.
/// This prices how much of the walk is cross-bucket collision -- work that
/// ends in a random `src` load inside `mls_eq` and (for mls >= 4) cannot
/// possibly succeed.
#[cfg(feature = "profile")]
pub static ROW_BUCKET: [crate::census64::AtomicU64; 5] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
/// Read and clear the bucket-sharing census.
#[cfg(feature = "profile")]
pub fn take_row_bucket() -> [u64; 5] {
    use core::sync::atomic::Ordering::Relaxed;
    let mut o = [0u64; 5];
    for (i, sl) in o.iter_mut().enumerate() {
        *sl = ROW_BUCKET[i].swap(0, Relaxed);
    }
    o
}

/// DFast back-extension arm. 0 = undecided, 1 = off, 2 = on.
///
/// SHIPS ON. It changes the bitstream, so it was boarded on size first, and the
/// board is one-sided: **-1.276% at L3 and -1.146% at L4** over 12 corpora at
/// 8 MiB, with ZERO corpora regressing at either level and the probe count flat
/// (-0.0% / -0.2%). It is not a ratio trade -- the backward walk recovers bytes
/// that were already matched and simply not credited, so there is no size to
/// give back.
///
/// L3 is the DEFAULT level, so this is the one flip on this ladder that every
/// caller of `compress(src, 3)` gets without asking.
static DFAST_BEXT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: turn DFast's backward match extension on or off in-process.
pub fn set_dfast_bext_arm(on: bool) {
    DFAST_BEXT_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

/// Read once per BLOCK and hoisted -- never per match.
pub(crate) fn dfast_bext_enabled() -> bool {
    use core::sync::atomic::Ordering::Relaxed;
    match DFAST_BEXT_ARM.load(Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = crate::env_knob_not0("RZSTD_DFAST_BEXT", true);
            DFAST_BEXT_ARM.store(if on { 2 } else { 1 }, Relaxed);
            on
        }
    }
}

/// DFAST BACK-EXTENSION PROBE. `emit_fast_seq_body` back-extends every Fast
/// match; DFast -- the DEFAULT level's finder -- does not, and C's
/// `ZSTD_compressBlock_doubleFast` does. These count what that costs: bytes a
/// backward walk WOULD recover at the commit point, and how many matches could
/// move at all. Measurement only; the walk is not applied.
#[cfg(feature = "profile")]
pub static DFAST_BEXT_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static DFAST_BEXT_MATCHES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static DFAST_BEXT_SEQS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(bytes_recoverable, matches_that_could_move, matches_seen)`.
#[cfg(feature = "profile")]
pub fn take_dfast_bext() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DFAST_BEXT_BYTES.swap(0, Relaxed),
        DFAST_BEXT_MATCHES.swap(0, Relaxed),
        DFAST_BEXT_SEQS.swap(0, Relaxed),
    )
}

#[cfg(feature = "profile")]
pub static ROW_LOADS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Read and clear `(candidates_examined, rows_loaded)`.
#[cfg(feature = "profile")]
pub fn take_row_census() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (ROW_EXAM.swap(0, Relaxed), ROW_LOADS.swap(0, Relaxed))
}

pub fn set_chain_tag_arm(on: bool) {
    CHAIN_TAG_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn chain_tag_enabled() -> bool {
    !matches!(CHAIN_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// The DFast position hash pair from ONE u64 load: short index (bit-exact
/// hash4), mls-width short tag, and long index (bit-exact hash8) all derive
/// from the same 8 bytes -- `hash4_tag_mls` and `hash8` each loaded them
/// separately, an optimizer-mood CSE (the 4a30eb4 rule: own the fold).
#[inline(always)]
fn dfast_hash_pair(
    src: &[u8],
    pos: usize,
    dtag_shift: u32,
    smask: u64,
    lshift: u32,
) -> (usize, u8, usize) {
    // W40: `lshift` arrives RESOLVED. This took `hlog` and re-derived
    // `64 - hlog.min(32)` -- a `min` and a `saturating_sub` -- on every
    // position of the SHIPPING DEFAULT level's finder, and twice per position
    // whenever the speculation arm is on, for a value fixed for the block.
    // The short shift (`dtag_shift`) had been passed in resolved since the
    // dfast cuts; the long one never was. Fourth site of this exact defect
    // today (14.9 W20 finder, 14.11 W25 lazy fill, 14.13 W36 greedy fill).
    debug_assert!(lshift < 64);
    let v = load_u64le(src, pos);
    let hv4 = (v as u32).wrapping_mul(HASH4_PRIME);
    let tv = (v & smask).wrapping_mul(FAST_HASH_PRIME64);
    let h8 = (v.wrapping_mul(0xCF1B_BCDC_B7A5_6463) >> lshift) as usize;
    ((hv4 >> dtag_shift) as usize, (tv >> 56) as u8, h8) // BRICK 52: see `hash4_tag_from`
}

/// BRICK 100 (F9): the chain ladder's link tag is the LAST BYTE of the
/// `mls`-byte gram, `src[pos + mls - 1]`.
///
/// Sound: an accept needs the first `mls` bytes equal, so a tag mismatch
/// can only reject what the compare would reject (`taggate`'s argument,
/// unchanged). What it filters: chain buckets are keyed by the 4-byte gram,
/// so the mates a walk meets mostly SHARE those 4 bytes and die at byte
/// `mls` (the census that chose the mls-width product tag over the 4-byte
/// one: 2.4M of L12's 169M bytemiss steps caught by 4 bytes, 1.4%) -- and
/// THIS tag is that byte, so it rejects every one of them where the
/// product's top byte rejected 255 in 256. The producer is one byte load
/// and one shift into the head word instead of a mask, a 64-bit multiply
/// and a shift-and-mask; the walk's goal tag loses the same three per call.
///
/// Contract: every caller holds `pos + 8 <= src.len()` (the fill's
/// `ilimit`, the walk's brick-49 invariant, the primer's and the wide
/// re-insert's own guards) and `mls <= 7` (brick 35's clamp; the wide
/// latch returns on `mls >= 8`).
#[inline(always)]
#[allow(unsafe_code)]
fn link_tag(src: &[u8], pos: usize, mls: usize) -> u8 {
    debug_assert!((3..=8).contains(&mls) && pos + mls <= src.len());
    // SAFETY: the contract above -- `pos + mls - 1 < pos + 8 <= src.len()`.
    unsafe { *src.get_unchecked(pos + mls - 1) }
}

/// hash4 index + the lazy ladder's link tag (BRICK 100: `link_tag`). Index
/// bit-identical to `hash4`; `hash_shift` arrives resolved.
#[inline(always)]
fn hash4_link_tag_b(src: &[u8], pos: usize, hash_shift: u32, mls: usize) -> (usize, u8) {
    (
        hash4_shift(load_u32le(src, pos), hash_shift),
        link_tag(src, pos, mls),
    )
}

/// `link_tag` from the little-endian word at `pos`: byte `mls - 1` of `v`.
/// The WALK takes its goal tag this way (BRICK 100b): `mls_xor` hoists
/// `load_u64le(src, ip)` for the first-word compare, and deriving the tag
/// from that same word keeps it ONE frame-resident value -- with a
/// separate byte load LLVM rematerialised the goal word on every candidate
/// (a load and a copy: dominant path 28 -> 29 / 25 -> 27). For the
/// const-MLS kernels the shift is a constant.
#[inline(always)]
fn link_tag_from(v: u64, mls: usize) -> u8 {
    debug_assert!((3..=8).contains(&mls));
    (v >> (8 * (mls - 1))) as u8
}

/// `hash4_link_tag_b` for the walk: index and tag from one u64 load, the
/// word `mls_xor` compares with (see `link_tag_from`).
#[inline(always)]
fn hash4_link_tag_w(src: &[u8], pos: usize, hash_shift: u32, mls: usize) -> (usize, u8) {
    let v = load_u64le(src, pos);
    debug_assert_eq!(link_tag_from(v, mls), link_tag(src, pos, mls));
    (hash4_shift(v as u32, hash_shift), link_tag_from(v, mls))
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
    WCHAIN_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn wide_chain_enabled() -> bool {
    // DEFAULT ON: adjudicated on the chainwide board with the hold-3 latch
    // and the attempts-scaled bar -- L5 -0.07% / L7 -0.46% / L9 -0.52% /
    // L12 -0.32% totals with ZERO losing corpora at any level.
    !matches!(WCHAIN_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

static WIDE_FIRST_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// Bench hook: the first-find-share bar for the wide-chain latch.
pub fn set_wide_first_max_arm(v: f32) {
    WIDE_FIRST_ARM.store(v.to_bits(), core::sync::atomic::Ordering::Relaxed);
}

static WIDE_SPB_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
    if attempts <= 16 {
        0.65
    } else {
        0.60
    }
}

/// The sao class is CAPTURED by the latch's second route (searches/byte
/// >= 0.50: sao 0.66 against every first-heavy loser <= 0.36 -- content
/// > where nearly every position searches has almost no literal+rep economy
/// > for the wide key to disturb). Still unlatched, deliberately: ooffice
/// > (-0.43), osdb (-0.77), mr (-0.69) -- first-heavy winners whose spb sits
/// > AMONG the losers'; no maintained signal separates them, recorded as the
/// > residual.
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
    // WIDE-LATCH CENSUS. This is an O(window) REBUILD of the chain, run once
    // per frame when the latch fires -- at L9 the window is 2^22, so it can
    // rescan up to 4M positions doing a FULL `lz_insert` each. Nothing counted
    // it, so its cost has never appeared beside the per-position work it is
    // meant to improve.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        WIDE_LATCH[0].fetch_add(1, Relaxed);
        WIDE_LATCH[1].fetch_add(to.saturating_sub(from) as u64, Relaxed);
    }
    let mut p = from;
    // BRICK 74: the links re-inserted below carry the WIDE producer's null tag.
    tables.set_null_tag(chain_null_tag(src, mls));
    let wshift = 64u32.saturating_sub(hash_log.min(32));
    while p <= to && p + 8 <= src.len() {
        let (h, g) = hash_wide_link_tag_b(src, p, wshift, smask, mls);
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

/// Wide bucket key from one u64 load and one multiply, with the ladder's
/// link tag (BRICK 100: `link_tag`); `shift` arrives resolved (W22).
#[inline(always)]
fn hash_wide_link_tag_b(src: &[u8], pos: usize, shift: u32, smask: u64, mls: usize) -> (usize, u8) {
    let hv = (load_u64le(src, pos) & smask).wrapping_mul(FAST_HASH_PRIME64);
    ((hv >> shift) as usize, link_tag(src, pos, mls))
}

/// BRICK 74 (K14): position 0's tag under the block's producer -- the tag an
/// EMPTY head's packed link carries (`MatchTables::set_null_tag`). The same
/// function the fill and the walk tag with (BRICK 100: `link_tag`), at
/// position 0; 0 when there is no first word (no walk reaches position 0
/// then either).
#[inline(always)]
fn chain_null_tag(src: &[u8], mls: usize) -> u8 {
    if src.len() < 8 {
        0
    } else {
        link_tag(src, 0, mls)
    }
}

/// Chain-walk census: src loads the link tag skipped, and (COUNT) the
/// FALSE-skip re-probe -- a skipped candidate whose bytes would have matched
/// must never exist.
#[cfg(feature = "profile")]
pub static LINK_SKIPS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LINK_FALSE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
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
    // NO wildcopy here, deliberately. A 16-byte fixed-width copy WAS added to
    // this function and measured 1,625,446 avoided `memcpy` calls at L9 -- and
    // then turned out to be a TWIN of `push_literals`' tier 1. The real defect
    // was that `find_lazy_impl` never routed its per-sequence emits through
    // `push_literals` at all; fixing that gives L9 the full 16/32/64 tiering
    // instead of a second copy of tier 1. This helper is now what its name
    // says: the per-block tail flush, a few hundred calls per corpus.
    crate::copies::add(crate::copies::C_LIT_PUSH, to - from);
    lits.extend_from_slice(unsafe { src.get_unchecked(from..to) });
}

/// Attribute only when the walk RAN and produced enough samples -- a block
/// that measured nothing must not move the EWMA (the Gate 14 rule).
fn update_walk_first_share(
    tables: &mut MatchTables,
    walked: bool,
    cls: (u32, u32),
    attempts: usize,
) {
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
pub static WALK_SIG_MB: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_NS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_SIG_OB: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
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
pub static BEXT_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_GE8: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static BEXT_MATCHES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
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
pub static WALK_EXAM: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_BYTEMISS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Walk-continue accept classes: (first-find past a collision -- legacy would
/// have emitted a literal; upgrade past a collision -- legacy had a match).
#[cfg(feature = "profile")]
pub static WALK_CONT_FIRST: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_CONT_UPGRADE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_walk_classes() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        WALK_CONT_FIRST.swap(0, Relaxed),
        WALK_CONT_UPGRADE.swap(0, Relaxed),
    )
}
#[cfg(feature = "profile")]
pub fn take_walk_census() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (WALK_EXAM.swap(0, Relaxed), WALK_BYTEMISS.swap(0, Relaxed))
}

/// BRICK 46: candidates examined AT POSITION 0 by the chain walk, and how
/// many of them were accepted. A link of 0 is both "no link" and position 0,
/// so every chain that ends inside the first window is followed to m = 0 and
/// examined there -- a phantom candidate that was never in the chain. The
/// accept count is what a representation with an unambiguous null would
/// change.
#[cfg(feature = "profile")]
pub static WALK_M0: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static WALK_M0_ACCEPT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub fn take_walk_phantom() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (WALK_M0.swap(0, Relaxed), WALK_M0_ACCEPT.swap(0, Relaxed))
}

/// REFUTED 2026-09-09, recorded so it is not retried: the FUSED HEAD that paid
/// in the chain walks (brick 11) loses HERE. `match_xor` returned this xor and
/// the caller took the length from it; the deterministic counter said dfast's
/// candidates resolve inside the first word only 37% of the time (L3) -- the
/// long-hash candidates match eight bytes BY CONSTRUCTION -- so the modelled
/// net was +43,877 instructions at L3 and +103,109 at L4 over the 16 MiB
/// corpus, and the static count +141. The same idea is right where the first
/// word usually decides and wrong where it usually does not.
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
    // The tail slice-eq compiled to a LIBC MEMCMP CALL per candidate (for
    // mls = 5, comparing ONE byte) -- the mls_eq lesson, applied to the
    // shared validity helper. Self-proving: the u64 path runs only when its
    // own 8-byte reads are in bounds (m < ip from the order check above).
    //
    // BRICK 33: the length tests `ip + mls > len || m + mls > len` used to
    // run BEFORE this arm, per candidate. Inside it they are implied --
    // `mls <= 8`, `ip + 8 <= len`, `m < ip` -- so they only guard the cold
    // tail, and that is where they live now.
    if mls <= 8 && ip + 8 <= src.len() {
        debug_assert!(m + 8 <= src.len() && ip + mls <= src.len() && m + mls <= src.len());
        let mask = if mls == 8 {
            u64::MAX
        } else {
            (1u64 << (8 * mls)) - 1
        };
        return (load_u64le(src, m) ^ load_u64le(src, ip)) & mask == 0;
    }
    if ip + mls > src.len() || m + mls > src.len() {
        return false;
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

/// The sub-8 boundary tail of [`count_match`]: under one call in 2000
/// (`eqwidth`: 99.956% of calls have `max >= 64`).
///
/// `#[cold]` + `#[inline(never)]` so its seven-step byte ladder is sunk out of
/// `count_match`'s straight line instead of padding it. One masked compare
/// answers this whenever the frame has 8-byte room past `ip`
/// (`m + 8 <= ip + 8 <= len` via `m <= ip`); the byte loop survives only at
/// the true frame edge.
#[cold]
#[inline(never)]
/// The sub-8 tail, on raw pointers. The old form tried one masked 8-byte
/// compare when the FRAME had room past `ip`; that needed `src.len()`, which
/// would have been a fifth argument on the stack at every hot call site.
/// This arm runs on ~1 call in 2000 and never more than seven compares, so
/// the byte ladder bounded by `max` is the right trade.
///
/// # Safety
/// Same contract as `count_match_raw`: `m <= ip`, `ip + max <= src.len()`.
#[allow(unsafe_code)]
unsafe fn count_match_sub8_raw(base: *const u8, m: usize, ip: usize, max: usize) -> usize {
    debug_assert!(m <= ip && max < 8);
    let mut n = 0usize;
    // SAFETY: `m + n <= ip + n < ip + max <= src.len()` for every `n < max`.
    while n < max && unsafe { *base.add(m + n) == *base.add(ip + n) } {
        n += 1;
    }
    #[cfg(feature = "profile")]
    crate::simd::note_eqlen(n);
    n
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
    // `m <= ip` (not `<`) is what the min-elimination needs: with `limit <=
    // src.len()`, `len - m >= len - ip >= limit - ip`, so `max` is `limit -
    // ip` either way. The oracle test exercises `m == ip` directly.
    debug_assert!(limit <= src.len() && m <= ip);
    if ip + 8 <= limit {
        let a = load_u64le(src, m);
        let b = load_u64le(src, ip);
        if a != b {
            return ((a ^ b).trailing_zeros() as usize) >> 3;
        }
        // SECOND-WORD PEEK MEASURED AND REJECTED (2026-08-21): inlining a
        // second register pair here covers matches of 8..15 without a call
        // (32.6% of counts land in 8..31), but it costs +156 instrs in EVERY
        // one of the 140 fast copies -- +12,931 across the family, in the
        // hottest function in the encoder -- and splits count_match's call
        // sites 12 -> 17. The executed-path saving is real; the I-cache cost
        // is real and this campaign's instruments cannot adjudicate it, so
        // the head stays one word deep.
        8 + count_match(src, m + 8, ip + 8, limit)
    } else {
        count_match(src, m, ip, limit)
    }
}

pub(crate) fn count_match(src: &[u8], m: usize, ip: usize, limit: usize) -> usize {
    // Same invariants as `count_match_fast`. They make every min redundant:
    // `len - m > len - ip >= limit - ip`, so max IS `limit - ip`, and the
    // three range guards collapse to `ip >= limit`. The slice constructions
    // keep memory safety: a violated invariant panics, it cannot read wild.
    // `m <= ip` (not `<`) is what the min-elimination needs: with `limit <=
    // src.len()`, `len - m >= len - ip >= limit - ip`, so `max` is `limit -
    // ip` either way. The oracle test exercises `m == ip` directly.
    debug_assert!(limit <= src.len() && m <= ip);
    // SAFETY: the two debug-asserted invariants are the whole contract of
    // `count_match_raw`. Every caller in this crate passes `limit = block_end`
    // (<= src.len() by construction) and a candidate `m` at or below `ip`;
    // `ldm` clamps `limit` to `src.len()` at its call site.
    #[allow(unsafe_code)]
    unsafe {
        count_match_raw(src.as_ptr(), m, ip, limit)
    }
}

/// The match-length kernel entry the finders actually pay for.
///
/// The safe form above takes `src: &[u8]` -- a FAT pointer -- so with `m`,
/// `ip` and `limit` it was FIVE machine arguments, and the Win64 ABI put the
/// fifth on the stack: one store at every call site (9-16 per finder) and
/// one load here. It then built two slices, `&src[m..m + max]` and
/// `&src[ip..limit]`, whose bounds LLVM cannot prove from debug-asserts in
/// release, and turned them straight back into pointers for the kernel.
///
/// This is NOT the experiment `count_eq_len_ge8`'s doc refutes. That one
/// KEPT the bounds proof (moved into `simd`), still built subslices on the
/// sub-8 branch, and added an `assert!` -- a third panic path -- to make a
/// raw call sound from safe code; 209 -> 244. This one has no proof to
/// relocate, no slice on any path, no assert, and four register arguments.
/// The instruction count is the verdict either way (see the changelog).
///
/// # Safety
/// `m <= ip` and `limit <= src.len()` for the `src` that `base` points into.
/// Then `max = limit - ip`, and `base[m..m + max]` and `base[ip..limit]` are
/// both in bounds.
#[inline(never)]
#[allow(unsafe_code)]
unsafe fn count_match_raw(base: *const u8, m: usize, ip: usize, limit: usize) -> usize {
    debug_assert!(m <= ip);
    if ip >= limit {
        return 0;
    }
    let max = limit - ip;
    // Sub-8 boundary tails: OUTLINED AND COLD. `max` is the room left in the
    // BLOCK, so it drops under 8 only at the very last bytes of one -- the
    // `eqwidth` counter reads `max >= 64` on 99.956% of calls.
    if max < 8 {
        // SAFETY: same contract, `max < 8` proven just above.
        return unsafe { count_match_sub8_raw(base, m, ip, max) };
    }
    // SAFETY: `m <= ip < limit <= src.len()` per the contract; both pointers
    // address readable bytes and `max` bytes follow each.
    let n = unsafe { crate::simd::count_eq_len_ge8_raw(base.add(m), base.add(ip), max) };
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
    hash4_shift(v, 32u32.saturating_sub(hash_log.min(32)))
}

/// W46: `hash4` with the shift already resolved -- the 4-byte sibling of
/// `hash8_shift`. Completes the set, so no hash entry in this crate needs a
/// `hash_log` when its caller already knows the shift.
#[inline(always)]
fn hash4_shift(v: u32, shift: u32) -> usize {
    debug_assert!(shift < 32);
    (v.wrapping_mul(HASH4_PRIME) >> shift) as usize
}

#[inline(always)]
fn hash8(src: &[u8], ip: usize, hash_log: u32) -> usize {
    hash8_shift(src, ip, 64u32.saturating_sub(hash_log.min(32)))
}

/// W20: `hash8` with the shift already resolved. The shift is a BLOCK
/// constant; deriving it inside the hash meant a `min` and a `saturating_sub`
/// on every position for a value that cannot change until the next block.
#[inline(always)]
fn hash8_shift(src: &[u8], ip: usize, shift: u32) -> usize {
    hash8_from(load_u64le(src, ip), shift)
}

/// The mixing half of `hash8_shift`. See `hash4_tag_from`.
#[inline(always)]
fn hash8_from(v: u64, shift: u32) -> usize {
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

/// Which `find_dfast` body actually executed. Probe counts and output bytes are
/// IDENTICAL between the two, by design -- they examine the same candidates in
/// the same order -- so neither can show which one ran. These can.
pub static DFAST_SPEC_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static DFAST_RUNTIME_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear both call counters.
pub fn take_dfast_calls() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        DFAST_SPEC_CALLS.swap(0, Ordering::Relaxed),
        DFAST_RUNTIME_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// Calls into `find_fast`'s Gate-4 dispatcher, for reachability proofs.
pub static FAST_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Calls into `find_opt` (L16+).
pub static OPT_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the finder reachability counters: `(find_fast, find_opt)`.
pub fn take_finder_calls() -> (u64, u64) {
    use core::sync::atomic::Ordering;
    (
        FAST_CALLS.swap(0, Ordering::Relaxed),
        OPT_CALLS.swap(0, Ordering::Relaxed),
    )
}

/// Which `bt_find_best` body ran: `(specialised, runtime_fallback)`.
pub static BT_SPEC_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static BT_RUNTIME_CALLS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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
/// Bench hook for in-process ABBA.
/// No-op since brick 8: the binary-tree specialisation this selected is
/// retired (`bt_resolve` returns the runtime body unconditionally). Kept so
/// the bench arms that name it still build; they measure nothing.
pub fn set_bt_spec_arm(_on: bool) {}

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
            let on = crate::env_knob_not0("RZSTD_NEXT_LONG", true);
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
        let v: f32 = crate::env_knob_parse("RZSTD_NEXT_LONG_T").unwrap_or(0.10);
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
            let on = crate::env_knob_not0("RZSTD_PAIR", true);
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
        let v: f32 = crate::env_knob_parse("RZSTD_PAIR_LO").unwrap_or(0.71);
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
    PAIR_LO_ARM.store(
        if v.is_nan() { u32::MAX } else { v.to_bits() },
        Ordering::Relaxed,
    );
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
        let v: f32 = crate::env_knob_parse("RZSTD_PAIR_T").unwrap_or(0.7);
        PAIR_T_CACHE.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.7
}

static PAIR_T_CACHE: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

/// PROMETHEUS PREREQ: how often is each fitted constant actually READ?
/// Each of these accessors calls `std::env::var` with no cache -- a
/// GetEnvironmentVariableW plus a String allocation for a process constant.
#[cfg(feature = "profile")]
pub static ENVHIT: [crate::census64::AtomicU64; 14] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];

/// Read and clear the fitted-constant read counts.
#[cfg(feature = "profile")]
pub fn take_envhits() -> [u64; 14] {
    let mut o = [0u64; 14];
    for i in 0..14 {
        o[i] = ENVHIT[i].swap(0, core::sync::atomic::Ordering::Relaxed);
    }
    o
}

/// Diagnostic counters for Gate 6 candidate variables: how often the pair probe
/// fires, how often it HITS, and how many bytes those hits cover. Activity vs
/// outcome -- the campaign's law says the signal must predict the outcome.
pub static PAIR_PROBES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_HITS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// Split the pair probe by the MAIN probe's slot state -- `m0 == 0` means that
/// hash bucket has never been written, which is free information already in a
/// register at the probe site.
pub static PAIR_M0_EMPTY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_M0_LIVE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_HIT_EMPTY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_HIT_LIVE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_BYTES_EMPTY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static PAIR_BYTES_LIVE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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

pub static MAIN_BYTES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear: `(probes, hits, pair_match_bytes, all_match_bytes)`.
static ROUTE_HIST: [crate::census64::AtomicU64; 3] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
static ROUTE_GAIN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static ROUTE_REP: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static ROUTE_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

static SIG_GAIN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_REP: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_N: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_TAG: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_REPLEN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_NSEQ: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
static SIG_OPTREP: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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
            let on = crate::env_knob_not0("RZSTD_TAG", true);
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
static TAG_MIN_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
        let v: f32 = crate::env_knob_parse("RZSTD_TAG_T").unwrap_or(0.0);
        TAG_MIN_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    0.0
}

/// Consume the counters and return the reject share.
///
/// Candidates a tag could reject without loading `src[m]`, and those it cannot:
/// the share rejected by the 4-byte compare is Gate 7's dispatch input.
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
pub static BT_PROBE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static BT_SHORT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
pub static BT_NOGAIN: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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
        let v: f32 = crate::env_knob_parse("RZSTD_PAIR_HI").unwrap_or(1.0);
        PAIR_HI_ARM.store(v.to_bits(), Ordering::Relaxed);
        v
    }
    #[cfg(not(feature = "std"))]
    1.0
}

static PAIR_HI_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
        let v: f32 = crate::env_knob_parse("RZSTD_PAIR_G").unwrap_or(0.20);
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
    TAG_ALLOC_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

static PAIR_GAIN_ARM: core::sync::atomic::AtomicU32 = core::sync::atomic::AtomicU32::new(u32::MAX);

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
    matches!(
        OPT_OPS_ARM.load(core::sync::atomic::Ordering::Relaxed),
        0 | 2
    )
}

fn opt_ops_blanket() -> bool {
    OPT_OPS_ARM.load(core::sync::atomic::Ordering::Relaxed) == 3
}

/// GATE 6 @ L1 arm: keep the finder's sequence/literal buffers on the frame
/// instead of building them fresh per block. Default ON.
static FINDER_SCRATCH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for GATE 6 @ L1.
pub fn set_finder_scratch_arm(on: bool) {
    FINDER_SCRATCH_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn finder_scratch_enabled() -> bool {
    !matches!(
        FINDER_SCRATCH_ARM.load(core::sync::atomic::Ordering::Relaxed),
        1
    )
}

/// T1 arm: give DFast the packed rejection tag that the Fast ladder already
/// uses. DEFAULT ON -- byte-identical on 18/18 at L3 and 72/72 across the board,
/// and it strictly removes work: 2,938,472 candidate loads avoided per board
/// pass (29.8% of non-empty short slots) for no added load, store, or byte of
/// memory, because the tag rides in the word the finder already touches.
static DFAST_TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for T1.
pub fn set_dfast_tag_arm(on: bool) {
    DFAST_TAG_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn dfast_tag_enabled() -> bool {
    !matches!(DFAST_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// 1a arm: the LONG-table rejection tag (packed frames only). DEFAULT ON.
static LONG_TAG_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for 1a.
pub fn set_long_tag_arm(on: bool) {
    LONG_TAG_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn long_tag_enabled() -> bool {
    !matches!(LONG_TAG_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// 1a ledger: (nonempty long probes, rejections, FALSE rejections). Three
/// counters with one meaning each -- see the tag audit's instrument trap.
#[cfg(feature = "profile")]
pub static LTAG_NONEMPTY: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_REJECT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_FALSE: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear the LONG-table tag audit: `[nonempty, rejected, FALSE]`.
/// A FALSE rejection is one the byte compare would have ACCEPTED -- i.e. a
/// match C's untagged `doubleFast` finds and the tag filter loses.
#[cfg(feature = "profile")]
pub fn take_ltag_audit() -> [u64; 3] {
    use core::sync::atomic::Ordering::Relaxed;
    [
        LTAG_NONEMPTY.swap(0, Relaxed),
        LTAG_REJECT.swap(0, Relaxed),
        LTAG_FALSE.swap(0, Relaxed),
    ]
}

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
pub static LTAG_SURV_FAIL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_SURV_WFAIL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static LTAG_SURV_ACC: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
/// SHORT-table consume-site residual, mirror of the long table's.
#[cfg(feature = "profile")]
pub static STAG_SURV_FAIL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STAG_SURV_WFAIL: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static STAG_SURV_ACC: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
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
pub static TAGARR_READS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static PACKED_TAG_READS: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

/// Read and clear `(tag-array reads, packed reads)`.
#[cfg(feature = "profile")]
pub fn take_tag_reads() -> (u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        TAGARR_READS.swap(0, Relaxed),
        PACKED_TAG_READS.swap(0, Relaxed),
    )
}

/// ffanat 5a arm: pack the Fast ladder's rejection tag into the hash slot
/// (dropping the separate `tags` array). DEFAULT ON; the guard in
/// `enable_packed_tags` still refuses frames >= 16 MiB.
static FAST_PACK_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the packed Fast tag.
pub fn set_fast_pack_arm(on: bool) {
    FAST_PACK_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
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
pub static FF_LAZY_FIRES: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_LATCH: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_CAND4: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static FF_ACCEPT: crate::census64::AtomicU64 = crate::census64::AtomicU64::new(0);

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
    FAST_HASH_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

fn fast_hash_wide_enabled() -> bool {
    !matches!(FAST_HASH_ARM.load(core::sync::atomic::Ordering::Relaxed), 1)
}

/// The unit tests live in `encode/tests.rs`. They are `#[cfg(test)]`, so a
/// release build never compiles them, and moving them out cost the emitted
/// code nothing (asm board identical in all thirty-two columns).
#[cfg(test)]
mod tests;
