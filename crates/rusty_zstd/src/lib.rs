//! Pure-Rust Zstandard (RFC 8878) codec.
//!
//! The crate README is the module documentation, so its examples are compiled
//! and run by `cargo test --doc` -- an install guide that cannot go stale.
#![doc = include_str!("../README.md")]
#![cfg_attr(not(feature = "std"), no_std)]
#![deny(unsafe_code)]
// ATTRIBUTE HIJACK GUARD. Deleting a function's `fn` line while leaving its
// attribute block, or inserting a new function between an existing block and
// its `fn`, silently re-parents those attributes onto the next item. This has
// happened twice here. Once it re-parented `#[target_feature(enable = "avx2")]`
// onto a twin dispatched on `has_bmi2()` alone -- a latent illegal-instruction
// bug on parts that ship BMI2 with AVX2 fused off, which no test on any host
// that runs the suite can observe. Once it stole `#[inline(always)]` from
// `encode::lz_insert_rowknown`. The duplicate-attribute case is the half the
// compiler can see, so it is denied rather than warned; `scripts/twinguard.py`
// covers the half it cannot (see that script's header).
#![deny(unused_attributes)]
// Under `no_std` the runtime ISA dispatch and every env knob compile out -- both
// need `std` -- which strands the caches, `*_on()` predicates and SIMD entry
// points they are the only callers of. They are live in every `std`
// configuration; see `env_knob` below.
#![cfg_attr(
    not(feature = "std"),
    allow(dead_code, unused_variables, unused_imports)
)]

#[cfg(feature = "alloc")]
extern crate alloc;

// Linking `rusty_alloc_default` is what INSTALLS the allocator: that crate holds
// the `#[global_allocator]`, and Cargo links an rlib only when something in the
// build actually references it. Turning the feature on without this line
// compiles the dependency and links nothing -- the process quietly keeps the
// platform allocator, and the feature reads as working while doing nothing.
#[cfg(feature = "rusty-alloc")]
extern crate rusty_alloc_default;

// The supported floor is `no_std + alloc` (docs/plans/rusty-zstd-mission.md
// §3.6: "Core decode + one-shot encode: no_std + alloc"). Core-only is NOT a
// configuration this crate has ever built in -- every entry point returns or
// fills a `Vec`. Say so in one line instead of emitting 64 confusing errors
// about `Vec` and missing modules.
#[cfg(not(feature = "alloc"))]
compile_error!(
    "rusty_zstd requires the `alloc` feature (implied by `std`). \
     The minimum supported configuration is `--no-default-features --features alloc`."
);

/// `std::env::var` where there is a `std`, `Err(())` where there is not.
///
/// Every env knob in this crate is a BENCH ARM with a shipping default, and
/// `no_std + alloc` is a supported target with no `std::env` -- so 20 of them
/// were hard build errors there. Routing them all through one shim keeps the
/// call sites unchanged, keeps the knobs working under `std`, and stops the
/// next knob anyone adds from breaking the no_std build again.
#[cfg(all(feature = "alloc", feature = "std"))]
#[inline]
pub(crate) fn env_knob(name: &str) -> Result<alloc::string::String, ()> {
    std::env::var(name).map_err(|_| ())
}

/// No-std twin: every knob reads as unset, so every call site takes its
/// shipping default.
#[cfg(all(feature = "alloc", not(feature = "std")))]
#[inline]
pub(crate) fn env_knob(_name: &str) -> Result<alloc::string::String, ()> {
    Err(())
}

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
#[cfg(feature = "profile")]
mod profclock;
mod reader;
mod rowfind;
#[cfg(feature = "alloc")]
mod scratch;
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
pub use xxh64::xxh64;

// ---------------------------------------------------------------------------
// Public API -- the codec surface. Everything below this block is the
// product; everything in the `#[doc(hidden)]` block at the bottom of this file
// is campaign instrumentation and is NOT part of the semver contract.
// ---------------------------------------------------------------------------

pub use decode::{content_size, decompress_with, find_frame_compressed_size, DecompressOptions};
pub use error::Error;
pub use frame::{
    get_frame_header, FrameHeader, FrameKind, BLOCKSIZE_MAX, DEFAULT_WINDOW_MAX, MAGIC,
    MAGIC_SKIPPABLE_MAX, MAGIC_SKIPPABLE_MIN,
};

#[cfg(feature = "alloc")]
pub use decode::{
    decompress, decompress_into, decompress_into_with, decompress_using_dict,
    decompress_using_dict_with, decompress_using_prefix, decompress_using_prefix_with,
    inspect_frames, ListedFrame,
};
#[cfg(feature = "alloc")]
pub use dict::{
    public_dict_id, Dictionary, DICT_ID_PUBLIC_MAX, DICT_ID_PUBLIC_MIN, MAGIC_DICTIONARY,
};
#[cfg(feature = "profile")]
pub use encode::take_row_bucket;
#[cfg(feature = "alloc")]
pub use encode::{
    compress, compress_using_dict, compress_using_dict_with, compress_using_prefix, compress_with,
    compress_with_advanced, compress_with_history, compress_with_params, AdvancedOptions,
    CompressOptions,
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
pub use params::{compression_params, CompressionParameters, Strategy};
#[cfg(feature = "profile")]
pub use rowfind::take_row_walk;
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
#[cfg(feature = "profile")]
pub use stream::{take_dec_compact, take_enc_slide};
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

// ---------------------------------------------------------------------------
// Campaign instrumentation -- `#[doc(hidden)]`, NOT public API.
//
// These are the A/B arms and counters the M7 speed campaign drives from
// `rusty_zstd-bench`. Every one has a shipping default and exists to be flipped
// inside a measurement harness. They are hidden from the rendered docs and
// carry NO semver promise: they may be renamed or removed in any release.
// Nothing here changes a default or an output byte.
// ---------------------------------------------------------------------------

#[doc(hidden)]
#[cfg(feature = "profile")]
#[doc(hidden)]
pub use bit::take_reload_calls;
#[cfg(feature = "profile")]
#[doc(hidden)]
pub use bit::take_reload_refills;
#[doc(hidden)]
#[cfg(feature = "dupladder")]
pub use compressed::{set_dup_arm, set_dup_k};
#[doc(hidden)]
pub use compressed::{
    set_litcopy_arm, set_lut_arm, set_matchcopy_arm, set_pipe1_arm, set_pipeline_arm,
    set_prefetch_arm, set_seqcheck_arm, set_seqloop_avx2_arm,
};
#[doc(hidden)]
#[cfg(feature = "profile")]
pub use compressed::{
    take_d3_iters, take_d4_paths, take_dec_bands, take_dec_copies, take_dec_lit64,
    take_dec_untiered, take_n21_predef,
};
#[doc(hidden)]
#[cfg(feature = "alloc")]
pub use decode::{frame_block_census, BlockCensus};
#[doc(hidden)]
pub use decode::{set_ck_fuse_arm, set_ck_stream_arm};
#[doc(hidden)]
pub use encode::{set_enc_avx2_arm, set_fast_hash_arm, set_fast_pack_arm};
#[doc(hidden)]
#[cfg(feature = "profile")]
pub use encode::{
    take_bext, take_envhits, take_ff_arms, take_ff_waste, take_link_tag, take_long_tag,
    take_long_tag_residual, take_raw_exits, take_raw_margin, take_row_census,
    take_short_tag_residual, take_step_forfeit, take_tag_reads, take_walk_census,
    take_walk_classes, take_walk_signals, FF_LATCH, FF_LAZY_FIRES,
};
#[doc(hidden)]
pub use encode::{take_ent_save, take_n9_basic};
#[doc(hidden)]
#[cfg(feature = "profile")]
pub use fse::take_d6_spread;

#[doc(hidden)]
pub use encode::set_dfast_bext_arm;
#[cfg(feature = "profile")]
pub use encode::take_dfast_bext;
#[cfg(feature = "profile")]
pub use encode::take_walk_exit;
#[cfg(feature = "profile")]
pub use huffman::{take_e11_walked, take_e12_scan, take_n13_stats, take_x2_stats};
#[cfg(feature = "profile")]
pub use huffman::{take_f4x2_arm, take_x4_arms, take_x4_x1_calls};
#[doc(hidden)]
#[cfg(feature = "alloc")]
pub use params::set_cparam_clamp_arm;
#[doc(hidden)]
#[cfg(feature = "std")]
pub use params::set_strategy_arm;
#[doc(hidden)]
#[cfg(all(feature = "alloc", feature = "profile"))]
pub use simd::{bench_eq_avx2, bench_eq_words, set_eqlen_arm, take_eq_ops, take_eqlen_stats};
#[doc(hidden)]
#[cfg(feature = "profile")]
pub use xxh64::census as xxh_census;
#[doc(hidden)]
pub use xxh64::{set_xxh_avx2_arm, Xxh64 as Xxh64Pub};

/// Bench-only re-export of the frame checksum hash (GATE 20 ceiling work).
#[doc(hidden)]
pub fn xxh64_pub(d: &[u8]) -> u64 {
    xxh64::xxh64(d)
}

#[doc(hidden)]
#[cfg(feature = "profile")]
pub use encode::take_ltag_audit;
#[doc(hidden)]
#[cfg(feature = "alloc")]
pub use encode::{
    reset_env_arms, set_accel_shift_arm, set_bt_spec_arm, set_chain_tag_arm,
    set_dfast_fill_anchor_arm, set_dfast_fill_n_arm, set_dfast_fill_stride_arm,
    set_dfast_good_ml2_arm, set_dfast_good_ml_arm, set_dfast_pipe_arm, set_dfast_spec_arm,
    set_dfast_spec_min_arm, set_dfast_step_arm, set_dfast_tag_arm, set_fast_lazy_arm,
    set_fast_spec_arm, set_finder_scratch_arm, set_g5_arms, set_g5_band_arm, set_g5_fast_arms,
    set_g5_fast_len_arm, set_g5_opt_arms, set_g5_tiny_arm, set_huff_fast_arm, set_incomp_skip_arm,
    set_lazy_fill_arm, set_lazy_fill_stride_arm, set_lazy_fill_threshold_arm, set_lazy_gain_arm,
    set_lit_short_arm, set_litpush_arm, set_litpush_hoist_arm, set_long_tag_arm, set_next_long_arm,
    set_nl_dispatch_arm, set_nl_off_worse_arm, set_opt_fill_max_arm, set_opt_fill_stride_arm,
    set_opt_hoist_arm, set_opt_lit_arm, set_opt_mlbits_arm, set_opt_ops_arm, set_opt_rep_arm,
    set_pair_gain_arm, set_pair_hi_arm, set_pair_lo_arm, set_pair_on_arm, set_payload_arm,
    set_pipe_arm, set_pipe_rep1_arm, set_prefix_bound_arm, set_prefix_window_arm, set_prime_bt_arm,
    set_prime_bt_depth_arm, set_prime_bt_extent_arm, set_prime_bt_tree_arm, set_prime_stride_arm,
    set_raw_probe_arm, set_raw_run_min_arm, set_raw_skip_arm, set_rep1_mode, set_rep_reprobe_arm,
    set_replen_pipe_arm, set_row_arm, set_row_fill_stride_arm, set_search_log_delta, set_step0_arm,
    set_step_forfeit_arm, set_step_probe_arm, set_step_seq_arm, set_tag_alloc_arm, set_tag_arm,
    set_walk_cont_arm, set_walk_first_max_arm, set_walk_rep_max_arm, set_wide_chain_arm,
    set_wide_first_max_arm, set_wide_spb_min_arm, take_bt_calls, take_bt_iters,
    take_bt_probe_stats, take_content_signals, take_dfast_calls, take_dfast_endfill,
    take_dfast_fill, take_dfast_match_stats, take_dfast_rep_blocks, take_dfast_spec, take_ff_pipe,
    take_finder_calls, take_g5, take_g5_inputs, take_lazy_fill, take_lp_guard, take_lp_stats,
    take_mm, take_next_long, take_nl_band, take_nl_off, take_opt_bt, take_opt_fill_ins,
    take_opt_rep, take_opt_skips, take_pair_split, take_pair_stats, take_prime_iters,
    take_rep_rate, take_route_hist, take_tag_rejects,
};
#[doc(hidden)]
pub use encode::{
    set_bt_deep_arm, set_bt_deep_min_arm, set_bt_depth_cached_arm, set_bt_depth_target_arm,
    set_dfast_litpush_arm, set_lit_push_tiers_arm, take_lit_hist, take_lit_push, take_lit_tiers,
    take_opt_signals, BT_SPEC_PAIRS,
};
#[doc(hidden)]
#[cfg(feature = "alloc")]
pub use prof::{
    dump as prof_dump, encode_counts as prof_encode_counts, note_hash_fill as prof_note_hash_fill,
    reset as prof_reset, scope as prof_scope, stage_calls as prof_stage_calls,
    stage_ns as prof_stage_ns, take_block_taps as prof_take_block_taps,
    take_lit_margin as prof_take_lit_margin, BlockTap as ProfBlockTap,
    EncodeCounts as ProfEncodeCounts, Stage as ProfStage,
};

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

/// D9 prefetch coverage counters. Behind `pfcensus`, NOT `profile`: they fire
/// per sequence, and a build carrying them cannot honestly time the brick
/// they describe (codec-measurement 6).
#[cfg(feature = "pfcensus")]
pub use compressed::take_pf_census;
