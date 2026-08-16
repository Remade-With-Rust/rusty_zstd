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
}

pub const N_STAGES: usize = 15;

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
        s.push_str(&alloc::format!("huff_path {}
", hp.join(" ")));
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
    note_early_raw, note_emit_lit, note_emit_seq, note_hash_fill, note_huff_path, note_lit_try, note_raw_block,
    note_rle_block, note_scratch, note_search, note_seq_mode, note_tables, reset, scope,
    take_block_taps,
};

#[cfg(not(feature = "profile"))]
pub use off::{
    dump, encode_counts, note_back_ext, note_block_tap, note_checksum_bytes, note_comp_block,
    note_early_raw, note_emit_lit, note_emit_seq, note_hash_fill, note_huff_path, note_lit_try, note_raw_block,
    note_rle_block, note_scratch, note_search, note_seq_mode, note_tables, reset, scope,
    take_block_taps,
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
