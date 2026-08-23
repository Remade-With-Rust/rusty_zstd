//! Feature-gated stage profiler (`codec-analyzer` spine).
//!
//! Off: `scope` is a ZST no-op the optimizer elides (release byte-identical).
//! On (`--features profile`): `Instant` RAII buckets + call counts + encode work
//! counters. Work counters are batched at block/search exit (not per-probe
//! atomics) so the stage dump stays trustworthy.

use core::fmt;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
pub enum Stage {
    EncodeTotal = 0,
    EncodeTables = 1,
    EncodeBlocks = 2,
    EncodeMatchFind = 3,
    EncodeEntropy = 4,
    EncodeHuff = 5,
    EncodeTableSelect = 6,
    EncodeFseSeq = 7,
    EncodeChecksum = 8,
    DecodeTotal = 9,
    DecodeBlocks = 10,
    DecodeLiterals = 11,
    DecodeSeq = 12,
    DecodeChecksum = 13,
    /// seqs -> CodedSeq transcode + the ll/of/ml histogram walk.
    EncodeSeqCode = 14,
    /// DecSeq anatomy. These four partition `DecodeSeq` and are scoped ONCE PER
    /// BLOCK, never per sequence -- an `Instant` pair costs more than the
    /// per-sequence body it would measure, so the loop's interior is resolved
    /// by counts and ablation, not by a clock. See `dsanat.rs`.
    /// nseq + mode-byte parse.
    DecSeqHeader = 15,
    /// The three `seq_table` FSE builds + `BitRev::new` + three `init_state`.
    DecSeqTables = 16,
    /// The per-sequence loop: entry, read_bits, copy_literals, resolve_offset,
    /// copy_match, advance.
    DecSeqLoop = 17,
    /// The trailing literal run after the last sequence.
    DecSeqTail = 18,
}

pub const N_STAGES: usize = 19;

const NAMES: [&str; N_STAGES] = [
    "EncodeTotal",
    "EncodeTables",
    "EncodeBlocks",
    "EncodeMatchFind",
    "EncodeEntropy",
    "EncodeHuff",
    "EncodeTableSelect",
    "EncodeFseSeq",
    "EncodeChecksum",
    "DecodeTotal",
    "DecodeBlocks",
    "DecodeLiterals",
    "DecodeSeq",
    "DecodeChecksum",
    "EncodeSeqCode",
    "DecSeqHeader",
    "DecSeqTables",
    "DecSeqLoop",
    "DecSeqTail",
];

/// Per-block Z1 harvest row (profile builds only).
#[derive(Clone, Copy, Debug, Default)]
pub struct BlockTap {
    /// Uncompressed block bytes.
    pub block_len: u32,
    /// Sequences in this block.
    pub nseq: u32,
    /// Bytes covered by matches.
    pub match_bytes: u32,
    /// Literal bytes.
    pub lit_bytes: u32,
    /// C `ZSTD_minGain` for this block.
    pub min_gain: u32,
    /// Sample peak: `max_freq * 1000 / n_sampled` (0..=1000).
    pub lit_peak: u32,
    /// 1 if the early-raw skip fired.
    pub early_raw: u8,
    /// P0/gg-matchfind: bytes this block actually emitted (payload for
    /// Compressed, `block_len` for Raw). This is the QUALITY half of the
    /// per-block gain -- without it a harvest can only score speed.
    pub csize: u32,
    /// Cumulative candidate examinations at block exit. The harvest differences
    /// consecutive rows to get this block's `work`.
    pub probes: u64,
    /// Cumulative probe hits at block exit.
    pub hits: u64,
    /// Tier-A signal: the repcode yield carried INTO this block, x1000.
    pub rep_yield_x1000: u32,
    /// P1/gg-matchfind candidate signal: collision probability over log2-offset
    /// buckets, x1000. HIGH = this block's matches cluster at a few offset
    /// scales; LOW = they are spread. Cheap (32-entry histogram over `nseq`).
    pub off_collision_x1000: u32,
    /// P1 candidate signal: how many of the 32 log2-offset buckets were used.
    pub off_buckets: u8,
    /// Cumulative `EncodeMatchFind` nanoseconds at block exit. Differenced by
    /// the harvest to give this block's `cpu_ms` -- the CONFIRMATORY half of
    /// the Great Gate speed pair (the counter rules when they disagree).
    pub mf_ns: u64,
}

/// Deterministic encode work counts (`codec-six-whys-unknowns`: count before time).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct EncodeCounts {
    pub hash_probes: u64,
    pub hash_fills: u64,
    pub probe_hits: u64,
    pub seqs: u64,
    pub match_bytes: u64,
    pub lit_bytes: u64,
    pub scratch_allocs: u64,
    pub rle_blocks: u64,
    pub raw_blocks: u64,
    pub comp_blocks: u64,
    pub table_hash_bytes: u64,
    pub table_hash_long_bytes: u64,
    pub table_chain_bytes: u64,
    pub checksum_bytes: u64,
    /// Bit accountant (codec-analyzer instrument 6): emitted bytes by section.
    /// Answers whether our size gap vs C is literals coding or sequence coding.
    pub emit_lit_bytes: u64,
    pub emit_seq_bytes: u64,
    /// Sequence-table mode selections, indexed by mode
    /// (0=Predefined, 1=RLE, 2=Compressed, 3=Repeat), summed over LL/OF/ML.
    pub seq_modes: [u64; 4],
    /// Backward match-extension: iterations of the byte-at-a-time loop in
    /// `emit_fast_seq`, and how many matches extended at all.
    pub back_ext_bytes: u64,
    pub back_ext_matches: u64,
    pub early_raw_blocks: u64,
}

#[cfg(feature = "profile")]
mod on {
    use super::{BlockTap, EncodeCounts, Stage, NAMES, N_STAGES};
    use core::cell::RefCell;
    use core::sync::atomic::{AtomicU64, Ordering};
    use std::time::Instant;

    thread_local! {
        static BLOCK_TAPS: RefCell<alloc::vec::Vec<BlockTap>> = const { RefCell::new(alloc::vec::Vec::new()) };
    }

    static NS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
    static CALLS: [AtomicU64; N_STAGES] = [const { AtomicU64::new(0) }; N_STAGES];
    static HASH_FILLS: AtomicU64 = AtomicU64::new(0);
    static HASH_PROBES: AtomicU64 = AtomicU64::new(0);
    static PROBE_HITS: AtomicU64 = AtomicU64::new(0);
    static SEQS: AtomicU64 = AtomicU64::new(0);
    static MATCH_BYTES: AtomicU64 = AtomicU64::new(0);
    static LIT_BYTES: AtomicU64 = AtomicU64::new(0);
    static SCRATCH_ALLOCS: AtomicU64 = AtomicU64::new(0);
    static RLE_BLOCKS: AtomicU64 = AtomicU64::new(0);
    static RAW_BLOCKS: AtomicU64 = AtomicU64::new(0);
    static COMP_BLOCKS: AtomicU64 = AtomicU64::new(0);
    static TABLE_HASH_BYTES: AtomicU64 = AtomicU64::new(0);
    static TABLE_HASH_LONG_BYTES: AtomicU64 = AtomicU64::new(0);
    static TABLE_CHAIN_BYTES: AtomicU64 = AtomicU64::new(0);
    static CHECKSUM_BYTES: AtomicU64 = AtomicU64::new(0);
    static EMIT_LIT_BYTES: AtomicU64 = AtomicU64::new(0);
    static EMIT_SEQ_BYTES: AtomicU64 = AtomicU64::new(0);
    /// Literal-section attempts: 0=blocks, 1=prev-table ENCODED, 2=prev won,
    /// 3=new-table ENCODED, 4=new won, 5=raw won.
    static HUFF_PATH: [AtomicU64; 20] = [const { AtomicU64::new(0) }; 20];

    static LIT_TRY: [AtomicU64; 7] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    static SEQ_MODES: [AtomicU64; 4] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];
    static BACK_EXT_BYTES: AtomicU64 = AtomicU64::new(0);
    static BACK_EXT_MATCHES: AtomicU64 = AtomicU64::new(0);
    static EARLY_RAW: AtomicU64 = AtomicU64::new(0);

    pub struct Guard {
        stage: Stage,
        start: Instant,
    }

    impl Drop for Guard {
        fn drop(&mut self) {
            let i = self.stage as usize;
            let ns = self.start.elapsed().as_nanos() as u64;
            NS[i].fetch_add(ns, Ordering::Relaxed);
            CALLS[i].fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn scope(stage: Stage) -> Guard {
        Guard {
            stage,
            start: Instant::now(),
        }
    }

    pub fn reset() {
        for i in 0..N_STAGES {
            NS[i].store(0, Ordering::Relaxed);
            CALLS[i].store(0, Ordering::Relaxed);
        }
        HASH_FILLS.store(0, Ordering::Relaxed);
        HASH_PROBES.store(0, Ordering::Relaxed);
        PROBE_HITS.store(0, Ordering::Relaxed);
        SEQS.store(0, Ordering::Relaxed);
        MATCH_BYTES.store(0, Ordering::Relaxed);
        LIT_BYTES.store(0, Ordering::Relaxed);
        SCRATCH_ALLOCS.store(0, Ordering::Relaxed);
        RLE_BLOCKS.store(0, Ordering::Relaxed);
        RAW_BLOCKS.store(0, Ordering::Relaxed);
        COMP_BLOCKS.store(0, Ordering::Relaxed);
        TABLE_HASH_BYTES.store(0, Ordering::Relaxed);
        TABLE_HASH_LONG_BYTES.store(0, Ordering::Relaxed);
        TABLE_CHAIN_BYTES.store(0, Ordering::Relaxed);
        CHECKSUM_BYTES.store(0, Ordering::Relaxed);
        EMIT_LIT_BYTES.store(0, Ordering::Relaxed);
        EMIT_SEQ_BYTES.store(0, Ordering::Relaxed);
        for c in &LIT_TRY {
            c.store(0, Ordering::Relaxed);
        }
        for c in &SEQ_MODES {
            c.store(0, Ordering::Relaxed);
        }
        BACK_EXT_BYTES.store(0, Ordering::Relaxed);
        BACK_EXT_MATCHES.store(0, Ordering::Relaxed);
        EARLY_RAW.store(0, Ordering::Relaxed);
        BLOCK_TAPS.with(|t| t.borrow_mut().clear());
    }

    pub fn note_hash_fill(n: u64) {
        HASH_FILLS.fetch_add(n, Ordering::Relaxed);
    }

    /// Cumulative nanoseconds attributed to one stage.
    pub fn stage_ns(stage: Stage) -> u64 {
        NS[stage as usize].load(Ordering::Relaxed)
    }

    /// How many times one stage's scope was entered. For the DecSeq sub-phases
    /// this is the BLOCK count, which is what turns a stage total into a
    /// per-block cost.
    pub fn stage_calls(stage: Stage) -> u64 {
        CALLS[stage as usize].load(Ordering::Relaxed)
    }

    /// Candidate examinations only, for finders whose probe loop lives in a
    /// shared helper (`chain_find_best`, `bt_find_best`). Those helpers report
    /// their own work; their callers then pass `probes = 0` to `note_search`
    /// so the count is not doubled.
    pub fn note_probes(n: u64) {
        HASH_PROBES.fetch_add(n, Ordering::Relaxed);
    }

    pub fn note_search(probes: u64, hits: u64, seqs: u64, match_bytes: u64, lit_bytes: u64) {
        HASH_PROBES.fetch_add(probes, Ordering::Relaxed);
        PROBE_HITS.fetch_add(hits, Ordering::Relaxed);
        SEQS.fetch_add(seqs, Ordering::Relaxed);
        MATCH_BYTES.fetch_add(match_bytes, Ordering::Relaxed);
        LIT_BYTES.fetch_add(lit_bytes, Ordering::Relaxed);
    }

    pub fn note_scratch(n: u64) {
        SCRATCH_ALLOCS.fetch_add(n, Ordering::Relaxed);
    }

    /// Bytes the literals section of one block emitted.
    pub fn note_emit_lit(n: u64) {
        EMIT_LIT_BYTES.fetch_add(n, Ordering::Relaxed);
    }

    /// Bytes the sequences section of one block emitted.
    pub fn note_emit_seq(n: u64) {
        EMIT_SEQ_BYTES.fetch_add(n, Ordering::Relaxed);
    }

    /// Backward-extension work for one emitted match.
    pub fn note_back_ext(bytes: u64) {
        BACK_EXT_BYTES.fetch_add(bytes, Ordering::Relaxed);
        if bytes > 0 {
            BACK_EXT_MATCHES.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// One sequence-table mode selection (0=Predef, 1=RLE, 2=Compressed, 3=Repeat).
    /// Huffman emit-path census: 0=fill, 1=k16..7=k6, 8=k5. Index 9..=15 hold
    /// the max_nbits histogram bucket (max_nbits-4, clamped).
    pub fn note_huff_path(kind: u8) {
        if let Some(c) = HUFF_PATH.get(kind as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    /// PROMETHEUS: the literals gate's MARGIN distribution.
    ///
    /// The harvest showed the gate never false-accepts (raw won 0/794), so there
    /// is no wasted encode work to reclaim. The open question is the other side:
    /// HOW MUCH size does an accepted block actually win? A block whose Huffman
    /// section is 1% smaller than raw bought 1% of bytes with a full Huffman
    /// DECODE instead of a memcpy -- the decode-cost term m7-anatomy S4.4 says
    /// the gate is missing. Buckets on `(raw - best) * 1000 / raw`:
    /// 0=<=0.5%, 1=0.5-1%, 2=1-2%, 3=2-5%, 4=5-10%, 5=10-20%, 6=20-40%, 7=>40%.
    /// Low half = block counts, high half = RAW BYTES in that bucket (so a rare
    /// but huge block cannot hide).
    static LIT_MARGIN: [AtomicU64; 16] = [const { AtomicU64::new(0) }; 16];

    /// One accepted literal section: how much smaller than raw did it come out?
    pub fn note_lit_margin(raw_len: usize, best_len: usize) {
        let saved = raw_len.saturating_sub(best_len);
        let permille = if raw_len == 0 {
            0
        } else {
            (saved as u64).saturating_mul(1000) / raw_len as u64
        };
        let b = match permille {
            0..=5 => 0usize,
            6..=10 => 1,
            11..=20 => 2,
            21..=50 => 3,
            51..=100 => 4,
            101..=200 => 5,
            201..=400 => 6,
            _ => 7,
        };
        LIT_MARGIN[b].fetch_add(1, Ordering::Relaxed);
        LIT_MARGIN[8 + b].fetch_add(raw_len as u64, Ordering::Relaxed);
    }

    /// Read and clear the margin histogram.
    pub fn take_lit_margin() -> [u64; 16] {
        let mut a = [0u64; 16];
        for i in 0..16 {
            a[i] = LIT_MARGIN[i].swap(0, Ordering::Relaxed);
        }
        a
    }

    pub fn note_lit_try(kind: u8) {
        if let Some(c) = LIT_TRY.get(kind as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note_seq_mode(mode: u8) {
        if let Some(c) = SEQ_MODES.get(mode as usize) {
            c.fetch_add(1, Ordering::Relaxed);
        }
    }

    pub fn note_rle_block() {
        RLE_BLOCKS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_raw_block() {
        RAW_BLOCKS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_comp_block() {
        COMP_BLOCKS.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_tables(hash: u64, hash_long: u64, chain: u64) {
        TABLE_HASH_BYTES.fetch_add(hash, Ordering::Relaxed);
        TABLE_HASH_LONG_BYTES.fetch_add(hash_long, Ordering::Relaxed);
        TABLE_CHAIN_BYTES.fetch_add(chain, Ordering::Relaxed);
    }

    pub fn note_checksum_bytes(n: u64) {
        CHECKSUM_BYTES.fetch_add(n, Ordering::Relaxed);
    }

    pub fn note_early_raw() {
        EARLY_RAW.fetch_add(1, Ordering::Relaxed);
    }

    pub fn note_block_tap(tap: BlockTap) {
        BLOCK_TAPS.with(|t| t.borrow_mut().push(tap));
    }

    pub fn take_block_taps() -> alloc::vec::Vec<BlockTap> {
        BLOCK_TAPS.with(|t| core::mem::take(&mut *t.borrow_mut()))
    }

    pub fn encode_counts() -> EncodeCounts {
        EncodeCounts {
            hash_probes: HASH_PROBES.load(Ordering::Relaxed),
            hash_fills: HASH_FILLS.load(Ordering::Relaxed),
            probe_hits: PROBE_HITS.load(Ordering::Relaxed),
            seqs: SEQS.load(Ordering::Relaxed),
            match_bytes: MATCH_BYTES.load(Ordering::Relaxed),
            lit_bytes: LIT_BYTES.load(Ordering::Relaxed),
            scratch_allocs: SCRATCH_ALLOCS.load(Ordering::Relaxed),
            rle_blocks: RLE_BLOCKS.load(Ordering::Relaxed),
            raw_blocks: RAW_BLOCKS.load(Ordering::Relaxed),
            comp_blocks: COMP_BLOCKS.load(Ordering::Relaxed),
            table_hash_bytes: TABLE_HASH_BYTES.load(Ordering::Relaxed),
            table_hash_long_bytes: TABLE_HASH_LONG_BYTES.load(Ordering::Relaxed),
            table_chain_bytes: TABLE_CHAIN_BYTES.load(Ordering::Relaxed),
            checksum_bytes: CHECKSUM_BYTES.load(Ordering::Relaxed),
            emit_lit_bytes: EMIT_LIT_BYTES.load(Ordering::Relaxed),
            emit_seq_bytes: EMIT_SEQ_BYTES.load(Ordering::Relaxed),
            back_ext_bytes: BACK_EXT_BYTES.load(Ordering::Relaxed),
            back_ext_matches: BACK_EXT_MATCHES.load(Ordering::Relaxed),
            seq_modes: [
                SEQ_MODES[0].load(Ordering::Relaxed),
                SEQ_MODES[1].load(Ordering::Relaxed),
                SEQ_MODES[2].load(Ordering::Relaxed),
                SEQ_MODES[3].load(Ordering::Relaxed),
            ],
            early_raw_blocks: EARLY_RAW.load(Ordering::Relaxed),
        }
    }

    pub fn dump() -> alloc::string::String {
        use alloc::format;
        let total = NS[Stage::EncodeTotal as usize].load(Ordering::Relaxed)
            + NS[Stage::DecodeTotal as usize].load(Ordering::Relaxed);
        let mut s = alloc::string::String::from("stage                 ms      %     calls\n");
        for i in 0..N_STAGES {
            let ns = NS[i].load(Ordering::Relaxed);
            let calls = CALLS[i].load(Ordering::Relaxed);
            let ms = ns as f64 / 1_000_000.0;
            let pct = if total == 0 {
                0.0
            } else {
                100.0 * ns as f64 / total as f64
            };
            s.push_str(&format!("{:<18} {ms:8.2} {pct:6.1} {calls:9}\n", NAMES[i]));
        }
        let c = encode_counts();
        s.push_str(&format!(
            "counts probes={} hits={} fills={} seqs={} match_b={} lit_b={} scratch={}\n",
            c.hash_probes,
            c.probe_hits,
            c.hash_fills,
            c.seqs,
            c.match_bytes,
            c.lit_bytes,
            c.scratch_allocs
        ));
        s.push_str(&format!(
            "blocks rle={} raw={} comp={}  tables hash={} long={} chain={}  xxh_b={}\n",
            c.rle_blocks,
            c.raw_blocks,
            c.comp_blocks,
            c.table_hash_bytes,
            c.table_hash_long_bytes,
            c.table_chain_bytes,
            c.checksum_bytes
        ));
        s.push_str(&format!(
            "lit_try blocks={} prev_ENCODED={} prev_won={} new_ENCODED={} new_won={} raw_won={} SKIPPED={}
",
            LIT_TRY[0].load(Ordering::Relaxed),
            LIT_TRY[1].load(Ordering::Relaxed),
            LIT_TRY[2].load(Ordering::Relaxed),
            LIT_TRY[3].load(Ordering::Relaxed),
            LIT_TRY[4].load(Ordering::Relaxed),
            LIT_TRY[5].load(Ordering::Relaxed),
            LIT_TRY[6].load(Ordering::Relaxed),
        ));
        let hp: alloc::vec::Vec<alloc::string::String> = HUFF_PATH
            .iter()
            .enumerate()
            .filter(|(_, c)| c.load(Ordering::Relaxed) > 0)
            .map(|(i, c)| alloc::format!("{}:{}", i, c.load(Ordering::Relaxed)))
            .collect();
        s.push_str(&alloc::format!(
            "huff_path {}
",
            hp.join(" ")
        ));
        s.push_str(&format!("early_raw={}\n", c.early_raw_blocks));
        s
    }
}

#[cfg(not(feature = "profile"))]
mod off {
    use super::EncodeCounts;

    pub struct Guard;

    #[inline(always)]
    pub fn scope(_stage: super::Stage) -> Guard {
        Guard
    }

    pub fn reset() {}

    #[inline(always)]
    pub fn note_hash_fill(_n: u64) {}

    #[inline(always)]
    pub fn stage_ns(_s: super::Stage) -> u64 {
        0
    }

    #[inline(always)]
    pub fn stage_calls(_s: super::Stage) -> u64 {
        0
    }

    #[inline(always)]
    pub fn note_probes(_n: u64) {}

    #[inline(always)]
    pub fn note_search(_p: u64, _h: u64, _s: u64, _m: u64, _l: u64) {}

    #[inline(always)]
    pub fn note_scratch(_n: u64) {}

    #[inline(always)]
    pub fn note_emit_lit(_n: u64) {}

    #[inline(always)]
    pub fn note_emit_seq(_n: u64) {}

    #[inline(always)]
    pub fn note_huff_path(_k: u8) {}

    #[inline(always)]
    pub fn note_lit_try(_k: u8) {}

    #[inline(always)]
    pub fn note_lit_margin(_r: usize, _b: usize) {}

    pub fn take_lit_margin() -> [u64; 16] {
        [0u64; 16]
    }

    #[inline(always)]
    pub fn note_seq_mode(_m: u8) {}

    #[inline(always)]
    pub fn note_back_ext(_b: u64) {}

    #[inline(always)]
    pub fn note_rle_block() {}

    #[inline(always)]
    pub fn note_raw_block() {}

    #[inline(always)]
    pub fn note_comp_block() {}

    #[inline(always)]
    pub fn note_tables(_h: u64, _l: u64, _c: u64) {}

    #[inline(always)]
    pub fn note_checksum_bytes(_n: u64) {}

    #[inline(always)]
    pub fn note_early_raw() {}

    #[inline(always)]
    pub fn note_block_tap(_t: super::BlockTap) {}

    pub fn take_block_taps() -> alloc::vec::Vec<super::BlockTap> {
        alloc::vec::Vec::new()
    }

    pub fn encode_counts() -> EncodeCounts {
        EncodeCounts::default()
    }

    pub fn dump() -> alloc::string::String {
        alloc::string::String::from("profile feature off\n")
    }
}

#[cfg(feature = "profile")]
pub use on::{
    dump, encode_counts, note_back_ext, note_block_tap, note_checksum_bytes, note_comp_block,
    note_early_raw, note_emit_lit, note_emit_seq, note_hash_fill, note_huff_path, note_lit_margin,
    note_lit_try, note_probes, note_raw_block, note_rle_block, note_scratch, note_search,
    note_seq_mode, note_tables, reset, scope, stage_calls, stage_ns, take_block_taps,
    take_lit_margin,
};

#[cfg(not(feature = "profile"))]
pub use off::{
    dump, encode_counts, note_back_ext, note_block_tap, note_checksum_bytes, note_comp_block,
    note_early_raw, note_emit_lit, note_emit_seq, note_hash_fill, note_huff_path, note_lit_margin,
    note_lit_try, note_probes, note_raw_block, note_rle_block, note_scratch, note_search,
    note_seq_mode, note_tables, reset, scope, stage_calls, stage_ns, take_block_taps,
    take_lit_margin,
};

impl fmt::Display for Stage {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(NAMES[*self as usize])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_is_safe_to_call() {
        reset();
        {
            let _g = scope(Stage::EncodeTotal);
        }
        let d = dump();
        assert!(!d.is_empty());
    }
}
