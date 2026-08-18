//! Pure-Rust Zstandard (RFC 8878) codec.
//!
//! M6: multi-thread (`-T#` / job size / overlap) and CLI completeness, on top of
//! M5 LDM/seekable, M4 dictionaries, and M3's full level/strategy range.

#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]

#[cfg(feature = "alloc")]
extern crate alloc;

mod bit;
mod block;
mod compressed;
mod decode;
#[cfg(feature = "alloc")]
mod dict;
#[cfg(feature = "alloc")]
mod encode;
mod error;
mod frame;
mod fse;
mod huffman;
#[cfg(all(feature = "alloc", feature = "std"))]
mod in_bench;
#[cfg(feature = "alloc")]
mod ldm;
#[cfg(feature = "std")]
mod mt;
#[cfg(feature = "alloc")]
mod params;
#[cfg(feature = "alloc")]
mod prof;
mod reader;
#[cfg(feature = "alloc")]
mod seekable;
/// Tiny documented `unsafe` island: runtime-dispatched AVX2/NEON kernels.
/// Callers stay safe. Scalar twins remain the oracle and the fallback.
#[allow(unsafe_code)]
mod simd;
#[cfg(feature = "alloc")]
mod stream;
#[cfg(all(feature = "alloc", feature = "std"))]
mod train;
mod xxh64;

pub use compressed::{set_litcopy_arm, set_lut_arm, set_matchcopy_arm, set_seqcheck_arm};
pub use decode::{content_size, decompress_with, find_frame_compressed_size, DecompressOptions};
pub use error::Error;
pub use frame::{
    get_frame_header, FrameHeader, FrameKind, BLOCKSIZE_MAX, DEFAULT_WINDOW_MAX, MAGIC,
    MAGIC_SKIPPABLE_MAX, MAGIC_SKIPPABLE_MIN,
};

#[cfg(feature = "alloc")]
pub use decode::{
    decompress, decompress_using_dict, decompress_using_dict_with, decompress_using_prefix,
    decompress_into, decompress_into_with, decompress_using_prefix_with, frame_block_census,
    inspect_frames, BlockCensus, ListedFrame,
};
#[cfg(feature = "alloc")]
pub use dict::{
    public_dict_id, Dictionary, DICT_ID_PUBLIC_MAX, DICT_ID_PUBLIC_MIN, MAGIC_DICTIONARY,
};
#[cfg(feature = "alloc")]
pub use encode::{
    compress, compress_using_dict, compress_using_dict_with, compress_using_prefix, compress_with,
    compress_with_advanced, compress_with_history, compress_with_params, AdvancedOptions,
    CompressOptions,
};
#[cfg(feature = "alloc")]
pub use encode::{
    reset_env_arms, set_bt_spec_arm, set_next_long_arm, set_pair_on_arm, take_pair_stats, set_dfast_spec_arm, set_fast_spec_arm, take_bt_calls, take_dfast_calls, take_finder_calls,
    set_huff_fast_arm, set_lazy_fill_arm, set_litpush_arm, set_litpush_hoist_arm, set_payload_arm, set_pipe_arm,
    set_fast_lazy_arm, set_incomp_skip_arm, set_rep1_arm, set_rep1_mode,
    set_search_log_delta,
    set_step0_arm,
};
#[cfg(all(feature = "alloc", feature = "std"))]
pub use in_bench::{
    bench_roundtrip, bench_roundtrip_clocked, mbps, mbps_best, time_loops, InProcessBench,
    LoopTiming,
};
#[cfg(feature = "alloc")]
pub use ldm::{LdmParams, DEFAULT_LONG_WINDOW_LOG};
#[cfg(feature = "std")]
pub use mt::{
    compress_mt, default_nb_workers, default_overlap_log, overlap_size, resolve_job_size,
    JOB_SIZE_MIN, NB_WORKERS_MAX,
};
#[cfg(feature = "alloc")]
pub use params::{compression_params, set_strategy_arm, CompressionParameters, Strategy};
#[cfg(feature = "alloc")]
pub use prof::{
    dump as prof_dump, encode_counts as prof_encode_counts, note_hash_fill as prof_note_hash_fill,
    reset as prof_reset, scope as prof_scope, take_block_taps as prof_take_block_taps,
    BlockTap as ProfBlockTap, EncodeCounts as ProfEncodeCounts, Stage as ProfStage,
};
#[cfg(feature = "alloc")]
pub use seekable::{
    compress_seekable, compress_seekable_adv, decompress_frame_at, parse_seek_table, SeekEntry,
    SeekTable, DEFAULT_FRAME_SIZE, SEEKABLE_MAGIC, SEEKABLE_SKIPPABLE_MAGIC,
};
#[cfg(feature = "alloc")]
pub use stream::{
    compress_stream_in_size, compress_stream_out_size, decompress_stream_in_size,
    decompress_stream_out_size, Compressor, Decompressor, Flush, StreamStatus,
};
#[cfg(all(feature = "alloc", feature = "std"))]
pub use train::{train, TrainAlgo, TrainOptions, DEFAULT_MAX_DICT};

/// Library version (semver of this crate, not the zstd format).
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Minimum negative compression level (matches libzstd `ZSTD_minCLevel`).
pub const MIN_CLEVEL: i32 = -7;

/// Maximum compression level (matches libzstd `--ultra` 22).
pub const MAX_CLEVEL: i32 = 22;

/// Default compression level (matches libzstd `ZSTD_CLEVEL_DEFAULT`).
pub const DEFAULT_CLEVEL: i32 = 3;

/// Worst-case compressed size for a single-pass frame (libzstd `ZSTD_compressBound`).
///
/// Formula from the zstd source: `src + (src >> 8) + 64` when src < 256 KB extra
/// padding path is covered by the `+ 64`. Exact C formula is
/// `src + (src >> 8) + (src < 128 KiB ? extra : 0)` -- we use a conservative
/// bound that is never smaller than C's.
pub fn compress_bound(src_len: usize) -> usize {
    src_len
        .saturating_add(src_len >> 8)
        .saturating_add(64)
        .saturating_add(if src_len < 128 * 1024 { 32 } else { 0 })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn compress_rejects_bad_level() {
        assert_eq!(compress(b"hi", 99).unwrap_err(), Error::InvalidLevel);
        assert_eq!(compress(b"hi", -8).unwrap_err(), Error::InvalidLevel);
    }

    #[test]
    fn compress_roundtrip_default() {
        let zst = compress(b"hello", DEFAULT_CLEVEL).unwrap();
        assert_eq!(decompress(&zst).unwrap(), b"hello");
    }

    #[test]
    fn decompress_truncated_magic_is_eof() {
        assert_eq!(
            decompress(&[0x28, 0xB5, 0x2F, 0xFD]).unwrap_err(),
            Error::UnexpectedEof
        );
    }

    #[test]
    fn compress_bound_grows_with_input() {
        assert!(compress_bound(0) >= 64);
        assert!(compress_bound(1_000_000) > 1_000_000);
    }
}
