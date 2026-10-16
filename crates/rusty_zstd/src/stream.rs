//! Streaming compress / decompress (`ZSTD_compressStream2` / `ZSTD_decompressStream` jobs).

use crate::block::{parse_block_header, BlockType};
use crate::compressed::{decode_compressed_block, BlockState};
use crate::encode::{
    checksum_u32, encode_block_from_scratch, write_frame_header, CompressOptions, EntropyState,
    MatchTables,
};
use crate::error::Error;
use crate::frame::{is_skippable_magic, parse_kind, FrameHeader, FrameKind, BLOCKSIZE_MAX, MAGIC};
use crate::params::{compression_params, CompressionParameters};
use crate::reader::Reader;
use crate::xxh64::Xxh64;
use crate::DecompressOptions;

/// Streaming-decoder compaction census: `[compactions, bytes memmoved]`.
/// The memmove per `decoded.drain(..drop)` is `len - drop` -- the retained
/// window -- and section 18's dig found it running once per `stream()` CALL.
#[cfg(feature = "profile")]
pub static DEC_COMPACT: [crate::census64::AtomicU64; 2] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
/// Read and clear the decoder-compaction census.
#[cfg(feature = "profile")]
pub fn take_dec_compact() -> [u64; 2] {
    use core::sync::atomic::Ordering;
    [
        DEC_COMPACT[0].swap(0, Ordering::Relaxed),
        DEC_COMPACT[1].swap(0, Ordering::Relaxed),
    ]
}

/// Encoder window-slide census: `[slides, hist bytes memmoved, table resets]`.
/// Each slide memmoves the retained window, zeroes six match tables and
/// re-primes the whole window -- section 19's decoder finding, mirrored.
#[cfg(feature = "profile")]
pub static ENC_SLIDE: [crate::census64::AtomicU64; 2] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
/// History multiplier at which the encoder slides its window.
///
/// `k` means: hold up to `k` windows of history, and on overflow drop back to
/// one. A slide then costs the same as ever but fires every `(k - 1) * window`
/// bytes, so the slide's whole overhead -- memmove, six table clears and the
/// re-prime -- scales as `1 / (k - 1)`. Default 3 (Section 20b; 2 was
/// Section 20's).
fn enc_slide_mul() -> usize {
    match crate::env_knob("RZSTD_ENC_SLIDE_MUL")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        Some(k) if (2..=8).contains(&k) => k,
        _ => 3,
    }
}

/// Ceiling on the EXTRA history held beyond two windows.
///
/// The slide's cost is per-slide and its frequency is `1 / ((k - 1) * window)`,
/// so the win from raising `k` is large exactly where the window is SMALL and
/// slides are constant, and negligible where the window is huge -- at L22 a
/// 128 MiB window does not slide until 128 MiB of input, which most streams
/// never reach. The memory cost runs the other way: it is `(k - 2) * window`,
/// so a flat multiplier would spend +128 MiB at L22 to remove slides that
/// mostly do not happen. Capping the extra in ABSOLUTE bytes keeps the whole
/// win at every level that slides often and bounds the worst case at +8 MiB.
const SLIDE_EXTRA_MAX: usize = 8 << 20;

/// Ablation: the decoded-buffer reserve cap. Shipping value 8 MiB; the probe
/// lifts it so `decoded` can be reserved to the full content size, making the
/// streaming buffer structurally identical to the one-shot output buffer.
#[cfg(feature = "std")]
fn dec_reserve_cap() -> u64 {
    static C: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *C.get_or_init(|| {
        crate::env_knob("RZSTD_DEC_RESERVE_CAP")
            .ok()
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or(8 << 20)
    })
}
#[cfg(not(feature = "std"))]
fn dec_reserve_cap() -> u64 {
    8 << 20
}

/// Bytes past a block's end that `Compressor::stream` holds in history before
/// coding the block. The finders read at most 8 bytes past `block_end` (the
/// widest hash load; the tree routes on 1), so 32 reproduces what they saw
/// when a call's whole input was appended before any block was coded.
const ABSORB_LOOKAHEAD: usize = 32;

/// History length at which the window slides, for this `window`.
fn slide_threshold(mul: usize, window: usize) -> usize {
    let extra = mul.saturating_sub(2).saturating_mul(window);
    2 * window + extra.min(SLIDE_EXTRA_MAX)
}

/// Read and clear the encoder window-slide census.
#[cfg(feature = "profile")]
pub fn take_enc_slide() -> [u64; 2] {
    use core::sync::atomic::Ordering;
    [
        ENC_SLIDE[0].swap(0, Ordering::Relaxed),
        ENC_SLIDE[1].swap(0, Ordering::Relaxed),
    ]
}

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// `ZSTD_EndDirective`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flush {
    /// Consume input; emit complete blocks only.
    Continue,
    /// Emit a complete (non-last) block for pending input.
    Flush,
    /// Last block + checksum; end the frame.
    End,
}

/// Bytes moved by one [`Compressor::stream`] / [`Decompressor::stream`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStatus {
    /// Input bytes this call consumed.
    pub input_consumed: usize,
    /// Output bytes this call produced.
    pub output_produced: usize,
    /// Frame fully finished (`Flush::End` completed, or a decompress frame ended and is drained).
    pub done: bool,
}

/// Recommended input chunk (`ZSTD_CStreamInSize`).
pub fn compress_stream_in_size() -> usize {
    BLOCKSIZE_MAX as usize
}

/// Recommended output chunk (`ZSTD_CStreamOutSize`).
pub fn compress_stream_out_size() -> usize {
    crate::compress_bound(BLOCKSIZE_MAX as usize)
}

/// Recommended decompress input chunk (`ZSTD_DStreamInSize`).
pub fn decompress_stream_in_size() -> usize {
    BLOCKSIZE_MAX as usize + 3
}

/// Recommended decompress output chunk (`ZSTD_DStreamOutSize`).
pub fn decompress_stream_out_size() -> usize {
    BLOCKSIZE_MAX as usize
}

/// Reusable compressor (one frame at a time).
///
/// After a frame ends -- `Flush::End` has reported `done` -- the next call
/// that carries input starts a new frame with the same level, checksum flag
/// and dictionary, as libzstd's `ZSTD_compressStream2` does. A prefix
/// ([`Self::set_prefix`]) and a pledged size apply to one frame only.
/// [`Self::reset`] abandons a frame in progress.
pub struct Compressor {
    /// History multiplier at which the window slides; see `set_slide_mul`.
    slide_mul: usize,
    params: CompressionParameters,
    checksum: bool,
    pledged: Option<u64>,
    /// The first error a frame hit. It sticks: every later call returns it
    /// until [`Self::reset`], so a retried `End` cannot report a frame done
    /// that is not (issue #24). libzstd's errors are sticky the same way.
    err: Option<Error>,
    /// Kept to load into every frame, as libzstd keeps a referenced CDict.
    dict: Option<crate::dict::Dictionary>,
    started: bool,
    ended: bool,
    xxh: Xxh64,
    reps: [u32; 3],
    tables: MatchTables,
    entropy: EntropyState,
    hist: Vec<u8>,
    /// End of the ENCODED prefix of `hist`; everything after it is pending
    /// input that no block has coded yet.
    ///
    /// There used to be a separate `in_acc` staging buffer, so every input
    /// byte was copied TWICE before encoding began -- once from the caller
    /// into `in_acc`, once from `in_acc` into `hist` -- measured at exactly
    /// 1.000 + 1.000 bytes per input byte. `hist` was always the buffer the
    /// finders read, so staging bought nothing that a cursor does not.
    hist_done: usize,
    out_acc: Vec<u8>,
    /// DECSEQ-II CUT 7 -- read cursor into `out_acc`, same shape as the
    /// decoder's `in_off` (CUT 5): the per-call `out_acc.drain(..n)` was an
    /// O(remaining) memmove of everything the caller had not read yet.
    out_off: usize,
    produced_in: u64,
    last_block_written: bool,
    dict_id: Option<u32>,
    write_dict_id: bool,
}

impl Compressor {
    /// New compressor at `level`.
    pub fn new(level: i32) -> Result<Self, Error> {
        Self::with_options(
            CompressOptions {
                level,
                checksum: true,
            },
            None,
        )
    }

    /// New compressor with options and an optional size hint.
    ///
    /// The hint only picks the parameters -- window, hash and chain sizes --
    /// as libzstd's `ZSTD_c_srcSizeHint` does: it is not a promise and is not
    /// written into the frame, and the stream may be longer or shorter. To
    /// promise the exact size and have it written as Frame_Content_Size, call
    /// [`Self::set_pledged_src_size`]. (Before 0.4.1 the hint was also the
    /// pledge, so a wrong hint failed the frame.)
    pub fn with_options(opts: CompressOptions, src_hint: Option<u64>) -> Result<Self, Error> {
        let params = compression_params(opts.level, src_hint)?;
        let mut tables = MatchTables::new(params);
        // The streaming compressor has no pledged total length to prove the
        // packed-tag bound, so it keeps the array form of the Fast tag filter;
        // `MatchTables::new` no longer allocates it (see ffanat), so do it here.
        tables.alloc_fast_tags(params);
        Ok(Self {
            // The env knob (`bench-arms` builds only), else the shipping 3.
            slide_mul: enc_slide_mul(),
            tables,
            params,
            checksum: opts.checksum,
            pledged: None,
            err: None,
            dict: None,
            started: false,
            ended: false,
            xxh: Xxh64::new(),
            reps: [1, 4, 8],
            entropy: EntropyState::default(),
            hist: Vec::new(),
            hist_done: 0,
            out_acc: Vec::new(),
            out_off: 0,
            produced_in: 0,
            last_block_written: false,
            dict_id: None,
            write_dict_id: true,
        })
    }

    /// Pledge the frame's exact uncompressed size before its first
    /// [`Self::stream`] call; it is written as Frame_Content_Size. Feeding
    /// more than the pledge, or ending the frame short of it, fails with
    /// [`Error::ContentSizeMismatch`] before a last block is written, and
    /// the error sticks until [`Self::reset`]. Applies to one frame.
    pub fn set_pledged_src_size(&mut self, size: u64) {
        self.next_frame_if_finished();
        if !self.started {
            self.pledged = Some(size);
        }
    }

    /// Load a dictionary before a frame's first [`Self::stream`] call. It is
    /// used for every later frame too, until replaced or a prefix is set.
    pub fn set_dictionary(&mut self, dict: &crate::dict::Dictionary) -> Result<(), Error> {
        self.next_frame_if_finished();
        if self.started {
            return Err(Error::Corruption);
        }
        if self.dict.is_some() || !self.hist.is_empty() {
            self.clear_history();
        }
        self.dict = Some(dict.clone());
        self.load_dictionary(dict);
        self.write_dict_id = self.dict_id.is_some();
        Ok(())
    }

    /// Prime cleared history with `dict` (its content, tables, entropy, reps).
    fn load_dictionary(&mut self, dict: &crate::dict::Dictionary) {
        self.dict_id = if dict.id() != 0 {
            Some(dict.id())
        } else {
            None
        };
        self.hist.extend_from_slice(dict.content());
        self.hist_done = self.hist.len();
        let window = 1usize << self.params.window_log.min(31);
        crate::encode::prime_tables(
            &mut self.tables,
            &self.hist,
            self.hist.len(),
            window,
            self.params,
        );
        if let Some(e) = dict.entropy() {
            self.entropy.seed_from_dict(e);
            self.reps = e.reps;
        }
    }

    /// Use `prefix` as match history (`--patch-from`) for the next frame
    /// only, as libzstd's `ZSTD_CCtx_refPrefix`; it replaces any dictionary.
    /// No Dictionary_ID is written.
    pub fn set_prefix(&mut self, prefix: &[u8]) -> Result<(), Error> {
        self.next_frame_if_finished();
        if self.started {
            return Err(Error::Corruption);
        }
        if self.dict.is_some() || !self.hist.is_empty() {
            self.clear_history();
        }
        self.dict = None;
        self.write_dict_id = false;
        self.hist.extend_from_slice(prefix);
        self.hist_done = self.hist.len();
        let window = 1usize << self.params.window_log.min(31);
        crate::encode::prime_tables(
            &mut self.tables,
            &self.hist,
            self.hist.len(),
            window,
            self.params,
        );
        Ok(())
    }

    /// Omit Dictionary_ID from the frame header (`--no-dictID`).
    /// Override the history multiplier at which the window slides.
    ///
    /// `k` windows of history are held and a slide drops back to one, so the
    /// slide's whole cost -- memmove, six table clears, and the re-prime that
    /// dominates it -- scales as `1 / (k - 1)`. It is a MEMORY-for-work trade
    /// and it also perturbs compressed size, because the re-prime inserts every
    /// position of the retained window and is therefore acting as a table
    /// densification pass. Present so both arms can be priced in one process.
    pub fn set_slide_mul(&mut self, k: usize) {
        self.slide_mul = k.clamp(2, 8);
    }

    pub fn set_write_dict_id(&mut self, write: bool) {
        self.next_frame_if_finished();
        if !self.started {
            self.write_dict_id = write;
        }
    }

    /// Abandon the frame in progress, if any, and any output not yet taken,
    /// and clear a sticky error and the pledged size. The level, checksum
    /// flag and dictionary are kept; a prefix is dropped (it applies to one
    /// frame). The next frame compresses exactly as it would on a fresh
    /// `Compressor` with the same settings. libzstd's
    /// `ZSTD_CCtx_reset(ZSTD_reset_session_only)`.
    pub fn reset(&mut self) {
        self.clear_history();
        self.pledged = None;
        self.err = None;
        self.started = false;
        self.ended = false;
        self.xxh = Xxh64::new();
        self.out_acc.clear();
        self.out_off = 0;
        self.produced_in = 0;
        self.last_block_written = false;
        if let Some(d) = self.dict.take() {
            self.load_dictionary(&d);
            self.dict = Some(d);
        }
    }

    /// Fresh match tables, entropy and reps, and no history: the state of a
    /// new `Compressor`, so nothing a previous frame, dictionary or prefix
    /// left behind can reach the next frame's bytes.
    fn clear_history(&mut self) {
        let mut tables = MatchTables::new(self.params);
        tables.alloc_fast_tags(self.params);
        self.tables = tables;
        self.reps = [1, 4, 8];
        self.entropy = EntropyState::default();
        self.hist.clear();
        self.hist_done = 0;
        self.dict_id = None;
    }

    /// A finished frame whose output is all taken: the next frame may begin.
    fn next_frame_if_finished(&mut self) {
        if self.ended && self.out_pending() == 0 && self.err.is_none() {
            self.reset();
        }
    }

    /// Record `e` as the frame's sticky error and return it.
    fn fail(&mut self, e: Error) -> Error {
        self.err = Some(e);
        e
    }

    /// Streaming compress. Copies produced bytes into `output`.
    ///
    /// Once a frame is done, a call with input starts the next frame (see
    /// the type's docs); a call with no input just reports `done` again.
    pub fn stream(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: Flush,
    ) -> Result<StreamStatus, Error> {
        if let Some(e) = self.err {
            return Err(e);
        }
        let mut produced = 0;
        if self.ended {
            produced = self.take_output(output);
            if self.out_pending() > 0 || input.is_empty() {
                return Ok(StreamStatus {
                    input_consumed: 0,
                    output_produced: produced,
                    done: self.out_pending() == 0,
                });
            }
            // Issue #30: this used to report `done` with nothing consumed,
            // forever -- a feed loop spun, and a loop that did not check
            // `input_consumed` silently dropped the data.
            self.reset();
        }
        // Issue #24: check the pledge as input arrives and before a last
        // block, never after it -- the frame must not end with a wrong size.
        if let Some(p) = self.pledged {
            let taken = self.produced_in + self.pending() as u64;
            let after = taken.saturating_add(input.len() as u64);
            if after > p || (flush == Flush::End && after != p) {
                return Err(self.fail(Error::ContentSizeMismatch));
            }
        }
        if !self.started {
            write_frame_header(
                &mut self.out_acc,
                0,
                self.params.window_log,
                self.checksum,
                self.pledged,
                if self.write_dict_id {
                    self.dict_id
                } else {
                    None
                },
                // External history is the dictionary or prefix already in
                // `hist`; the input is not appended yet. This read
                // `!hist.is_empty()` after the append, so a first call with
                // data never got a Single_Segment frame (issue #24).
                !self.hist.is_empty(),
            );
            self.started = true;
        }

        let block_max = (1usize << self.params.window_log.min(31)).min(BLOCKSIZE_MAX as usize);
        // Absorb the input one block at a time, coding each block as it
        // fills (issue #31). Appending the whole call first made `hist` hold
        // all of it, so every window slide memmoved the uncoded remainder too:
        // one 128 MiB call ran 1.7-3.1x slower than the same bytes in 128 KiB
        // chunks. The blocks, their boundaries and the slides are the ones
        // the whole-call append produced, so the output is unchanged -- with
        // one subtlety: the finders may read a few bytes past `block_end`
        // (the binary tree routes on `src[block_end]`), so a full block is
        // coded only once `ABSORB_LOOKAHEAD` bytes past it are in `hist`, or
        // the call's input has run out, exactly as when all of it was there.
        let mut rest = input;
        while !rest.is_empty() {
            if self.pending() >= block_max + ABSORB_LOOKAHEAD {
                if let Err(e) = self.emit_block(block_max, false) {
                    return Err(self.fail(e));
                }
                continue;
            }
            let n = (block_max + ABSORB_LOOKAHEAD - self.pending()).min(rest.len());
            crate::copies::add(crate::copies::C_ENC_IN_ACC, n);
            self.hist.extend_from_slice(&rest[..n]);
            rest = &rest[n..];
        }

        let emit_all = matches!(flush, Flush::Flush | Flush::End);
        while self.pending() >= block_max || (emit_all && self.pending() > 0) {
            let take = self.pending().min(block_max);
            let last = flush == Flush::End && take == self.pending();
            if let Err(e) = self.emit_block(take, last) {
                return Err(self.fail(e));
            }
            if !last && flush == Flush::Flush && self.pending() == 0 {
                break;
            }
            if last {
                break;
            }
        }
        // `compact_in` is gone with `in_acc`: there is no staging buffer to
        // reclaim any more. The history buffer's own reclaim is the window
        // slide in `emit_block`, which now measures the ENCODED prefix.

        if flush == Flush::End && !self.ended {
            if self.pending() == 0 {
                if let Err(e) = self.emit_empty_last_if_needed() {
                    return Err(self.fail(e));
                }
            }
            if self.checksum {
                self.out_acc
                    .extend_from_slice(&checksum_u32(&self.xxh).to_le_bytes());
            }
            self.ended = true;
        }

        let n = self.take_output(&mut output[produced..]);
        Ok(StreamStatus {
            input_consumed: input.len(),
            output_produced: produced + n,
            done: self.ended && self.out_pending() == 0,
        })
    }

    /// Unread output bytes.
    #[inline(always)]
    fn out_pending(&self) -> usize {
        self.out_acc.len() - self.out_off
    }

    /// CUT 7: hand the caller its bytes and advance the cursor; reclaim the
    /// dead prefix only when everything is read (a `clear`), or when it is at
    /// least 64 KiB AND at least as long as the unread remainder it would
    /// move. The second condition is issue #31: with a large backlog and a
    /// small output buffer, reclaiming at every 64 KiB moved the whole backlog
    /// per call -- 5.25 GB of memmove for one 128 MiB L3 call. Now each byte
    /// moves at most about once.
    fn take_output(&mut self, output: &mut [u8]) -> usize {
        let n = self.out_pending().min(output.len());
        crate::copies::add(crate::copies::C_ENC_OUT, n);
        output[..n].copy_from_slice(&self.out_acc[self.out_off..self.out_off + n]);
        self.out_off += n;
        if self.out_off == self.out_acc.len() {
            self.out_acc.clear();
            self.out_off = 0;
        } else if self.out_off >= 64 * 1024 && self.out_off >= self.out_pending() {
            crate::copies::add(
                crate::copies::C_ENC_OUT_COMPACT,
                self.out_acc.len() - self.out_off,
            );
            self.out_acc.drain(..self.out_off);
            self.out_off = 0;
        }
        n
    }

    fn pending(&self) -> usize {
        self.hist.len().saturating_sub(self.hist_done)
    }

    fn emit_empty_last_if_needed(&mut self) -> Result<(), Error> {
        if self.ended {
            return Ok(());
        }
        if self.last_block_written {
            return Ok(());
        }
        self.emit_block(0, true)
    }

    fn emit_block(&mut self, take: usize, last: bool) -> Result<(), Error> {
        // No copy here any more: the bytes are already in `hist`, appended
        // by `stream`. This advances a cursor instead, and the checksum is
        // taken over exactly the block rather than "to the end of hist",
        // which now has pending bytes past it.
        let block_start = self.hist_done;
        let block_end = block_start + take;
        if take > 0 {
            self.xxh.update(&self.hist[block_start..block_end]);
            self.hist_done = block_end;
        }
        let window = 1usize << self.params.window_log.min(31);
        self.tables.far_reps_ok = self.produced_in + take as u64 <= window as u64;
        encode_block_from_scratch(
            &mut self.out_acc,
            &self.hist,
            block_start,
            block_end,
            self.params,
            &mut self.tables,
            &mut self.reps,
            &mut self.entropy,
            last,
        )?;
        // SECTION 20 -- the ENCODER's mirror of section 19. Sliding at
        // `hist > window` fired on EVERY block at steady state, and each slide
        // memmoves the retained window, zeroes six match tables and re-primes
        // the WHOLE window. Measured (webster 32 MB, L3, 256 KiB chunks):
        // **240 slides, 503,316,480 hist bytes memmoved (15.0x the source),
        // 503,314,800 prime inserts** -- half a billion table inserts as pure
        // slide overhead. Waiting for a full EXTRA window amortises all three
        // costs to ~1x the stream, for one window of extra memory -- the same
        // trade section 19 brick B made, and safe for the same reason one-shot
        // encode is: the finders clamp offsets to `window` internally however
        // much history the buffer holds (`bytegate` proves that daily at 32 MB
        // inputs over a 2 MB window).
        //
        // NOTE: this changes WHICH table state each streamed block is found
        // under, so streamed bytes differ from the previous build's (both are
        // valid frames; the round-trip, the external decoder and the size are
        // the gate). One-shot output is untouched -- `emit_block` is
        // streaming-only.
        // SECTION 20b -- the same trade, one notch further, made MEASURABLE.
        // Section 20 moved the trigger from `hist > window` (a slide on every
        // block) to `hist >= 2 * window`, which cut webster from 240 slides and
        // 503 MB memmoved to 15 slides and 31 MB. The cost per slide is
        // unchanged -- memmove the window, zero six tables, re-prime the whole
        // window -- so the ONLY remaining lever on it is how often it fires,
        // and that is linear in this multiplier: at `k * window` a slide
        // happens every `(k - 1) * window` bytes.
        //
        // It is a MEMORY trade, not a free win: `k` windows of history are held
        // instead of two. So it is a knob with a shipping default rather than a
        // constant, and `streamcopies.rs` prices both arms.
        // Measured on `hist_done`, not `hist.len()`: the tail past the cursor
        // is pending input that has not been coded, and dropping any of it
        // would silently lose caller bytes.
        if self.hist_done >= slide_threshold(self.slide_mul, window) {
            let drop = self.hist_done - window;
            #[cfg(feature = "profile")]
            {
                ENC_SLIDE[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                ENC_SLIDE[1].fetch_add(
                    (self.hist.len() - drop) as u64,
                    core::sync::atomic::Ordering::Relaxed,
                );
            }
            crate::copies::add(crate::copies::C_HIST_SLIDE, self.hist.len() - drop);
            self.hist.drain(..drop);
            self.hist_done -= drop;
            self.tables.reset();
            crate::encode::prime_tables(
                &mut self.tables,
                &self.hist,
                self.hist_done,
                window,
                self.params,
            );
        }
        self.produced_in += take as u64;
        if last {
            self.last_block_written = true;
        }
        Ok(())
    }
}

/// Reusable decompressor. Multi-frame. Window-bounded history.
pub struct Decompressor {
    opts: DecompressOptions,
    input: Vec<u8>,
    /// DECSEQ-II CUT 5 -- read cursor into `input`. Every parsed unit used to
    /// `input.drain(..k)` -- an O(remaining) memmove per BLOCK (the
    /// codec-memory-copies opening pattern, N3). The cursor advances for free
    /// and `compact_input` amortises the reclaim.
    in_off: usize,
    decoded: Vec<u8>,
    decoded_off: usize,
    header: Option<FrameHeader>,
    block_state: BlockState,
    xxh: Xxh64,
    frame_out: u64,
    in_checksum: bool,
    saw_zstd: bool,
    dict: Option<crate::dict::Dictionary>,
    prefix: Vec<u8>,
    frame_start: usize,
    frame_skipped: usize,
}

impl Decompressor {
    /// Default window cap (128 MiB).
    pub fn new() -> Self {
        Self::with_options(DecompressOptions::default())
    }

    /// Explicit window cap.
    pub fn with_options(opts: DecompressOptions) -> Self {
        Self {
            opts,
            input: Vec::new(),
            in_off: 0,
            decoded: Vec::new(),
            decoded_off: 0,
            header: None,
            block_state: BlockState::new(),
            xxh: Xxh64::new(),
            frame_out: 0,
            in_checksum: false,
            saw_zstd: false,
            dict: None,
            prefix: Vec::new(),
            frame_start: 0,
            frame_skipped: 0,
        }
    }

    /// Load a dictionary before decompressing.
    pub fn set_dictionary(&mut self, dict: crate::dict::Dictionary) {
        self.prefix = dict.content().to_vec();
        self.dict = Some(dict);
    }

    /// Use `prefix` as match history (`--patch-from`).
    pub fn set_prefix(&mut self, prefix: &[u8]) {
        self.dict = None;
        self.prefix = prefix.to_vec();
    }

    /// Streaming decompress. `end` is true when the caller has no more input.
    pub fn stream(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        end: bool,
    ) -> Result<StreamStatus, Error> {
        crate::copies::add(crate::copies::C_DEC_IN_ACC, input.len());
        {
            let _s = crate::prof::scope(crate::prof::Stage::StreamInAcc);
            self.input.extend_from_slice(input);
        }
        // SECTION 19 BRICK A -- decode no further than the caller can drink.
        // The old exit only fired when `output` was EMPTY, so a caller feeding
        // a whole frame in one call had ALL of it decoded into `decoded`
        // before the first byte came back: 121 MB of allocator traffic to
        // stream a 32 MB frame, and a buffer the size of the content. Stopping
        // once the pending bytes can fill `output` bounds `decoded` at
        // ~output + window + one block, and later calls resume from `input`
        // exactly where this one stopped.
        {
            let _s = crate::prof::scope(crate::prof::Stage::StreamProgress);
            loop {
                let have = self.decoded.len() - self.decoded_off;
                if have > 0 && have >= output.len() {
                    break;
                }
                if !self.progress()? {
                    break;
                }
            }
        }
        {
            let _s = crate::prof::scope(crate::prof::Stage::StreamCompact);
            self.compact_input();
        }
        let avail = self.decoded.len() - self.decoded_off;
        let n = avail.min(output.len());
        crate::copies::add(crate::copies::C_DEC_OUT, n);
        {
            let _s = crate::prof::scope(crate::prof::Stage::StreamOutCopy);
            output[..n].copy_from_slice(&self.decoded[self.decoded_off..self.decoded_off + n]);
        }
        self.decoded_off += n;
        {
            let _s = crate::prof::scope(crate::prof::Stage::StreamCompact);
            self.compact();
        }
        if end && self.in_avail() != 0 && self.header.is_none() && n == 0 && avail == 0 {
            return Err(Error::UnexpectedEof);
        }
        let done = end
            && self.in_avail() == 0
            && self.header.is_none()
            && self.decoded_off == self.decoded.len();
        if done && !self.saw_zstd {
            return Err(Error::UnexpectedEof);
        }
        Ok(StreamStatus {
            input_consumed: input.len(),
            output_produced: n,
            done,
        })
    }

    /// Unconsumed input.
    #[inline(always)]
    fn in_avail(&self) -> usize {
        self.input.len() - self.in_off
    }

    /// The unconsumed input, as a slice.
    #[inline(always)]
    fn in_bytes(&self) -> &[u8] {
        &self.input[self.in_off..]
    }

    /// STREAMING-VS-ONE-SHOT GAP, measured and still OPEN. Streaming decode
    /// runs at 0.703-0.776x of `decompress_into` on identical bytes (ABBA
    /// interleaved, null arm 0.95-1.08x, z = -1.67 to -3.00). Eight causes
    /// have been eliminated by measurement, so none of these is worth
    /// re-testing without new information:
    ///
    ///  1. copy traffic -- cut 74% (9.87 -> 2.53 B/output), ratio unchanged
    ///  2. structural re-entry -- stage call counts are IDENTICAL (256/256)
    ///  3. a single hot stage -- scoped stages grow only 1.09x
    ///  4. the decoded-window compaction -- ablated, no change
    ///  5. per-call overhead -- 32x fewer `stream` calls, no change
    ///  6. buffer reallocation -- the header-time reserve already exists
    ///  7. buffer SHAPE -- made structurally one-shot-like, gap persists
    ///  8. the content checksum -- disabled, no change
    ///
    /// A ninth, the benchmark itself, WAS real and is why the figure above is
    /// lower than first reported: a harness that fed a chunk and read once
    /// under-drained and inflated the gap to ~1.8x. The next instrument is a
    /// sampling profiler on the two binaries; the stage profiler cannot
    /// resolve it (its rdtsc tax is 27-29% of wall and compresses the
    /// measured 1.4x into 1.04x).
    ///
    /// CUT 5's amortiser: reclaim consumed input only when it is all consumed
    /// (a `clear`, no memmove) or the dead prefix has grown past 64 KiB -- so
    /// the per-unit O(remaining) drains become O(1) cursor bumps and the total
    /// moved is genuinely bounded by the bytes fed.
    ///
    /// `2 * in_off >= len` is what makes that claim true, and it was missing.
    /// The trigger fired on an ABSOLUTE dead prefix (64 KiB) while the drain
    /// memmoves the LIVE remainder -- and Brick A deliberately stops decoding
    /// the moment the caller's buffer can be filled, so that remainder grows
    /// against a fixed trigger. Measured before the fix, 32 MiB streamed with
    /// a 64 KiB feed: **244 MB memmoved on webster, 7.63 bytes per OUTPUT
    /// byte** -- the O(n^2) front-drain this module's own header warns about,
    /// one level below where CUT 5 removed it.
    ///
    /// Requiring the reclaim to cover the move also bounds the buffer: the
    /// live tail is never more than half, so `input` holds at most ~2x what
    /// the caller has fed ahead of consumption.
    fn compact_input(&mut self) {
        if self.in_off == 0 {
            return;
        }
        if self.in_off == self.input.len() {
            self.input.clear();
            self.in_off = 0;
        // `3 * dead >= 2 * total` means the dead prefix is at least TWICE the
        // live tail, so each compaction reclaims twice what it moves rather
        // than merely matching it -- the same frequency lever as the decoded
        // window above, and it halves this traffic again for one more buffer's
        // worth of held input. The bound stays real: the live tail is never
        // more than a third, so `input` holds at most ~1.5x what the caller has
        // fed ahead.
        } else if self.in_off >= 64 * 1024 && 3 * self.in_off >= 2 * self.input.len() {
            // Counted AT the drain, so it records bytes actually memmoved. An
            // earlier version of this tap sat in `stream()` and recorded
            // `input.len() - in_off` once per call -- the standing unconsumed
            // input, a LEVEL summed as though it were a FLOW. It read 15.27
            // "bytes per output byte" and meant nothing.
            crate::copies::add(
                crate::copies::C_DEC_IN_COMPACT,
                self.input.len() - self.in_off,
            );
            self.input.drain(..self.in_off);
            self.in_off = 0;
        }
    }

    fn progress(&mut self) -> Result<bool, Error> {
        if self.in_checksum {
            if self.in_avail() < 4 {
                return Ok(false);
            }
            let b = self.in_bytes();
            let got = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
            self.in_off += 4;
            if checksum_u32(&self.xxh) != got {
                return Err(Error::ChecksumMismatch);
            }
            if let Some(n) = self.header.and_then(|h| h.content_size) {
                if self.frame_out != n {
                    return Err(Error::ContentSizeMismatch);
                }
            }
            self.finish_frame();
            return Ok(true);
        }
        if self.header.is_none() {
            return self.try_header();
        }
        self.try_block()
    }

    fn try_header(&mut self) -> Result<bool, Error> {
        if self.in_avail() < 4 {
            return Ok(false);
        }
        let b = self.in_bytes();
        let magic = u32::from_le_bytes([b[0], b[1], b[2], b[3]]);
        if is_skippable_magic(magic) {
            if self.in_avail() < 8 {
                return Ok(false);
            }
            let b = self.in_bytes();
            let n = u32::from_le_bytes([b[4], b[5], b[6], b[7]]) as usize;
            if self.in_avail() < 8 + n {
                return Ok(false);
            }
            self.in_off += 8 + n;
            return Ok(true);
        }
        if magic != MAGIC {
            return Err(if self.saw_zstd {
                Error::TrailingBytes
            } else {
                Error::BadMagic
            });
        }
        let mut r = Reader::new(self.in_bytes());
        match parse_kind(&mut r) {
            Err(Error::UnexpectedEof) => Ok(false),
            Err(e) => Err(e),
            Ok(FrameKind::Skippable { user_data_size, .. }) => {
                let n = user_data_size as usize;
                if r.remaining() < n {
                    return Ok(false);
                }
                self.in_off += r.pos() + n;
                Ok(true)
            }
            Ok(FrameKind::Zstd(h)) => {
                if h.window_size > self.opts.window_max {
                    return Err(Error::WindowTooLarge);
                }
                if let Some(id) = h.dict_id {
                    match self.dict.as_ref() {
                        None => return Err(Error::DictionaryNeeded { id }),
                        Some(d) if d.id() != id => {
                            return Err(Error::DictionaryMismatch {
                                frame: id,
                                loaded: d.id(),
                            });
                        }
                        Some(_) => {}
                    }
                }
                let pos = r.pos();
                self.in_off += pos;
                // SECTION 19 BRICK C -- size `decoded` once, from the header.
                // Bricks A and B pin its steady state near
                // `2*window + output + block`; growing there by doubling cost
                // ~7.5 MB of allocator traffic per 32 MB frame. The reserve is
                // clamped by the declared content size when the header carries
                // one and by a flat 8 MiB either way, so a hostile header
                // cannot make this allocate past the cap unverified.
                {
                    let keep = 2 * h.window_size + 2 * u64::from(h.block_size_max());
                    let want = h
                        .content_size
                        .map(|cs| cs.saturating_add(u64::from(h.block_size_max())).min(keep))
                        .unwrap_or(keep)
                        .min(dec_reserve_cap()) as usize;
                    self.decoded.reserve(want);
                }
                self.header = Some(h);
                self.block_state = BlockState::from_dict(self.dict.as_ref());
                self.xxh = Xxh64::new();
                self.frame_out = 0;
                self.frame_start = self.decoded.len();
                self.frame_skipped = 0;
                self.saw_zstd = true;
                Ok(true)
            }
        }
    }

    fn try_block(&mut self) -> Result<bool, Error> {
        let header = self.header.ok_or(Error::Corruption)?;
        if self.in_avail() < 3 {
            return Ok(false);
        }
        let mut r = Reader::new(self.in_bytes());
        let bh = match parse_block_header(&mut r) {
            Err(Error::UnexpectedEof) => return Ok(false),
            Err(e) => return Err(e),
            Ok(h) => h,
        };
        let payload_n = bh.payload_len() as usize;
        if r.remaining() < payload_n {
            return Ok(false);
        }
        let hdr_len = r.pos();
        // DECSEQ-II CUT 4: this was `self.input[a..b].to_vec()` -- a heap
        // allocation plus a full copy of EVERY block payload, per block,
        // purely to end `input`'s borrow before the drain. The cursor (CUT 5)
        // removes the drain, so the payload is now borrowed straight from
        // `input`: `input`, `decoded`, `block_state` and `prefix` are disjoint
        // fields, which is all the block decoder needs.
        let a = self.in_off + hdr_len;
        let b = a + payload_n;
        self.in_off = b;
        let block_max = header.block_size_max();
        let start = self.decoded.len();
        match bh.ty {
            BlockType::Raw => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                self.decoded.extend_from_slice(&self.input[a..b]);
            }
            BlockType::Rle => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                let b0 = *self
                    .input
                    .get(a)
                    .filter(|_| b > a)
                    .ok_or(Error::Corruption)?;
                let n = bh.size as usize;
                self.decoded.resize(self.decoded.len() + n, b0);
            }
            BlockType::Compressed => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                decode_compressed_block(
                    &self.input[a..b],
                    &mut self.decoded,
                    header.window_size,
                    block_max,
                    &mut self.block_state,
                    &self.prefix,
                    self.frame_start,
                    self.frame_skipped,
                )?;
            }
        }
        let produced = &self.decoded[start..];
        self.xxh.update(produced);
        self.frame_out += produced.len() as u64;
        if bh.last {
            if header.checksum {
                self.in_checksum = true;
            } else {
                if let Some(n) = header.content_size {
                    if self.frame_out != n {
                        return Err(Error::ContentSizeMismatch);
                    }
                }
                self.finish_frame();
            }
        }
        Ok(true)
    }

    fn finish_frame(&mut self) {
        self.header = None;
        self.in_checksum = false;
        self.block_state = BlockState::new();
    }

    fn compact(&mut self) {
        let window = self
            .header
            .map(|h| h.window_size as usize)
            .unwrap_or(BLOCKSIZE_MAX as usize);
        let keep_from = self.decoded.len().saturating_sub(window);
        let drop = self.decoded_off.min(keep_from);
        // SECTION 19 BRICK B -- amortise the reclaim. This drained on EVERY
        // `stream()` call, and each drain memmoves the retained WINDOW
        // (`len - drop`): measured 306 MB moved for 32 MB decoded (9.1x) on a
        // 64 KiB-chunk feed, and 4.28 GB (133x) on a one-shot feed. Waiting
        // until a full window of dead prefix has accumulated makes each
        // compaction reclaim at least as much as it moves, so total traffic is
        // bounded by ~1x the decoded bytes, for at most one extra window of
        // memory held.
        // SECTION 19c -- the same frequency lever the ENCODER slide took, and
        // strictly cheaper here. Brick B made each compaction reclaim at least
        // as much as it moves, bounding traffic at ~1x the decoded bytes.
        // Waiting for TWO windows of dead prefix halves how often it fires and
        // so halves that bound, for one more window of memory.
        //
        // Unlike the encoder's slide this has NO ratio cost: the encoder's
        // slide re-primes the match tables, so sliding less often changes which
        // matches are found and costs 0.06-0.14% size. Decode output is fixed
        // by the bitstream -- the only thing that changes is when the buffer is
        // reclaimed. The extra is capped in ABSOLUTE bytes for the same reason
        // as the encoder's: the win scales with compaction FREQUENCY (high when
        // the window is small) and the memory cost with window SIZE.
        let extra = window.min(SLIDE_EXTRA_MAX);
        if drop >= (window + extra).max(64 * 1024) {
            #[cfg(feature = "profile")]
            {
                DEC_COMPACT[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                DEC_COMPACT[1].fetch_add(
                    (self.decoded.len() - drop) as u64,
                    core::sync::atomic::Ordering::Relaxed,
                );
            }
            crate::copies::add(crate::copies::C_DEC_COMPACT, self.decoded.len() - drop);
            self.decoded.drain(..drop);
            self.decoded_off -= drop;
            if drop <= self.frame_start {
                self.frame_start -= drop;
            } else {
                self.frame_skipped += drop - self.frame_start;
                self.frame_start = 0;
            }
        }
    }
}

impl Default for Decompressor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    /// The slide threshold must hold the extra history where slides are
    /// FREQUENT and refuse to at high levels, where the window is huge and
    /// slides are rare. A flat multiplier costs +128 MiB at L22 to remove
    /// slides a 128 MiB window mostly never performs.
    #[test]
    fn slide_threshold_caps_the_extra_history() {
        // Small windows: the full extra window is held.
        for wlog in 19..=23u32 {
            let w = 1usize << wlog;
            let t = super::slide_threshold(3, w);
            let extra = t - 2 * w;
            assert!(
                extra <= super::SLIDE_EXTRA_MAX,
                "wlog {wlog}: extra {extra} exceeds the cap"
            );
            assert_eq!(extra, w.min(super::SLIDE_EXTRA_MAX));
        }
        // L22-class window: capped, NOT 3x.
        let w = 1usize << 27;
        let t = super::slide_threshold(3, w);
        assert_eq!(t, 2 * w + super::SLIDE_EXTRA_MAX);
        assert!(
            t < 3 * w,
            "a flat 3x would cost a whole extra 128 MiB window"
        );
        // k=2 reproduces the pre-change threshold exactly, at every size.
        for wlog in 10..=27u32 {
            let w = 1usize << wlog;
            assert_eq!(super::slide_threshold(2, w), 2 * w);
        }
    }

    use super::*;
    use crate::compress;

    #[test]
    fn stream_compress_chunked_roundtrip() {
        let src = b"The quick brown fox jumps over the lazy dog. ".repeat(200);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(src.len() as u64);
        let mut zst = Vec::new();
        let mut tmp = [0u8; 64];
        let mut off = 0usize;
        while off < src.len() {
            let n = (src.len() - off).min(7);
            let st = c
                .stream(&src[off..off + n], &mut tmp, Flush::Continue)
                .unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            off += st.input_consumed;
        }
        loop {
            let st = c.stream(&[], &mut tmp, Flush::End).unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            if st.done {
                break;
            }
        }
        let mut d = Decompressor::new();
        let mut out = vec![0u8; src.len() + 16];
        let st = d.stream(&zst, &mut out, true).unwrap();
        assert!(st.done);
        assert_eq!(&out[..st.output_produced], src.as_slice());
    }

    #[test]
    fn silesia_mr_prefix_stream_roundtrip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia/mr");
        if !path.is_file() {
            return;
        }
        let mut src = std::fs::read(&path).expect("read mr");
        src.truncate(277_521);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(src.len() as u64);
        let mut zst = Vec::new();
        let mut tmp = vec![0u8; 64 * 1024];
        let mut off = 0usize;
        while off < src.len() {
            let n = (src.len() - off).min(4096);
            let st = c
                .stream(&src[off..off + n], &mut tmp, Flush::Continue)
                .unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            off += n;
        }
        loop {
            let st = c.stream(&[], &mut tmp, Flush::End).unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            if st.done {
                break;
            }
        }
        let got = crate::decompress(&zst).expect("decompress stream frame");
        if got.as_slice() != src.as_slice() {
            let pos = got
                .iter()
                .zip(src.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(src.len()));
            panic!("stream mismatch at {pos}/{}", src.len());
        }
    }

    /// One frame of `data` through `c`: fed with `Continue`, then `End`.
    fn frame(c: &mut Compressor, data: &[u8]) -> Result<Vec<u8>, Error> {
        let mut out = vec![0u8; 4096];
        let mut z = Vec::new();
        let mut input = data;
        while !input.is_empty() {
            let st = c.stream(input, &mut out, Flush::Continue)?;
            z.extend_from_slice(&out[..st.output_produced]);
            input = &input[st.input_consumed..];
        }
        loop {
            let st = c.stream(&[], &mut out, Flush::End)?;
            z.extend_from_slice(&out[..st.output_produced]);
            if st.done {
                return Ok(z);
            }
        }
    }

    fn sample(n: usize) -> Vec<u8> {
        (0..n)
            .map(|i| b"stream issue sample text "[i % 25] ^ ((i / 997) as u8 & 3))
            .collect()
    }

    fn header(z: &[u8]) -> FrameHeader {
        match crate::get_frame_header(z).unwrap() {
            FrameKind::Zstd(h) => h,
            other => panic!("{other:?}"),
        }
    }

    // #24: the size hint picks parameters only; it is neither written nor
    // enforced.
    #[test]
    fn size_hint_is_not_a_pledge() {
        let src = sample(10_000);
        let opts = CompressOptions {
            level: 3,
            checksum: true,
        };
        for hint in [100u64, 10_000, 1 << 20] {
            let mut c = Compressor::with_options(opts, Some(hint)).unwrap();
            let z = frame(&mut c, &src).unwrap();
            assert_eq!(header(&z).content_size, None, "hint {hint}");
            assert_eq!(crate::decompress(&z).unwrap(), src, "hint {hint}");
        }
    }

    // #24: a pledge exceeded fails before a last block is written, and the
    // error sticks: a retried End used to return Ok(done) on a frame neither
    // decoder accepts.
    #[test]
    fn exceeded_pledge_fails_and_sticks() {
        let src = sample(300_000);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(200_000);
        let mut out = vec![0u8; 1 << 20];
        assert_eq!(
            c.stream(&src, &mut out, Flush::Continue).unwrap_err(),
            Error::ContentSizeMismatch
        );
        for flush in [Flush::End, Flush::End, Flush::Continue] {
            assert_eq!(
                c.stream(&[], &mut out, flush).unwrap_err(),
                Error::ContentSizeMismatch
            );
        }
        c.reset();
        let z = frame(&mut c, &src).unwrap();
        assert_eq!(crate::decompress(&z).unwrap(), src);
    }

    #[test]
    fn short_pledge_fails_at_end_and_sticks() {
        let src = sample(5_000);
        let mut c = Compressor::new(3).unwrap();
        c.set_pledged_src_size(6_000);
        let mut out = vec![0u8; 1 << 16];
        let st = c.stream(&src, &mut out, Flush::Continue).unwrap();
        assert_eq!(st.input_consumed, src.len());
        for _ in 0..3 {
            assert_eq!(
                c.stream(&[], &mut out, Flush::End).unwrap_err(),
                Error::ContentSizeMismatch
            );
        }
    }

    // #24: a pledged frame without a dictionary or prefix is Single_Segment,
    // as one-shot frames are, even when the first call carries the data.
    #[test]
    fn pledged_frame_is_single_segment() {
        let src = sample(1_000);
        let mut c = Compressor::new(3).unwrap();
        c.set_pledged_src_size(src.len() as u64);
        let mut out = vec![0u8; 4096];
        let st = c.stream(&src, &mut out, Flush::End).unwrap();
        assert!(st.done);
        let z = &out[..st.output_produced];
        let h = header(z);
        assert!(h.single_segment);
        assert_eq!(h.content_size, Some(1_000));
        assert_eq!(crate::decompress(z).unwrap(), src);
    }

    // #30: after End, the next call with input starts a new frame, which is
    // byte-identical to what a fresh Compressor makes. A call with no input
    // just reports done again.
    #[test]
    fn reused_compressor_starts_a_new_frame() {
        let a = sample(70_000);
        let b = sample(150_000);
        let mut c = Compressor::new(3).unwrap();
        let za = frame(&mut c, &a).unwrap();
        let mut out = vec![0u8; 64];
        let st = c.stream(&[], &mut out, Flush::End).unwrap();
        assert_eq!(
            (st.input_consumed, st.output_produced, st.done),
            (0, 0, true)
        );
        let zb = frame(&mut c, &b).unwrap();
        assert_eq!(zb, frame(&mut Compressor::new(3).unwrap(), &b).unwrap());
        let mut both = za.clone();
        both.extend_from_slice(&zb);
        let mut ab = a.clone();
        ab.extend_from_slice(&b);
        assert_eq!(crate::decompress(&both).unwrap(), ab);
    }

    // #30: the dictionary carries over to the next frame; a prefix does not.
    #[test]
    fn dictionary_persists_prefix_does_not() {
        let dict = crate::dict::Dictionary::raw(sample(8_000));
        let src = sample(20_000);
        let mut c = Compressor::new(3).unwrap();
        c.set_dictionary(&dict).unwrap();
        let z1 = frame(&mut c, &src).unwrap();
        let z2 = frame(&mut c, &src).unwrap();
        assert_eq!(z1, z2);
        assert_eq!(crate::decompress_using_dict(&z2, &dict).unwrap(), src);

        let prefix = sample(8_000);
        let mut c = Compressor::new(3).unwrap();
        c.set_prefix(&prefix).unwrap();
        let z1 = frame(&mut c, &src).unwrap();
        let z2 = frame(&mut c, &src).unwrap();
        assert_eq!(crate::decompress_using_prefix(&z1, &prefix).unwrap(), src);
        assert_eq!(z2, frame(&mut Compressor::new(3).unwrap(), &src).unwrap());
    }

    // #30: reset abandons a frame in progress and its untaken output.
    #[test]
    fn reset_abandons_the_frame() {
        let src = sample(300_000);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(1);
        let mut out = vec![0u8; 16];
        let _ = c.stream(&src[..1], &mut out, Flush::Continue).unwrap();
        c.reset();
        let z = frame(&mut c, &src).unwrap();
        assert_eq!(z, frame(&mut Compressor::new(1).unwrap(), &src).unwrap());
    }

    #[test]
    fn oneshot_and_stream_decompress() {
        let src = b"hello hello hello hello";
        let zst = compress(src, 3).unwrap();
        let mut d = Decompressor::new();
        let mut out = vec![0u8; 64];
        let mut got = Vec::new();
        let mut off = 0usize;
        while off < zst.len() {
            let n = (zst.len() - off).min(3);
            let end = off + n == zst.len();
            let st = d.stream(&zst[off..off + n], &mut out, end).unwrap();
            got.extend_from_slice(&out[..st.output_produced]);
            off += n;
            if st.done {
                break;
            }
        }
        assert_eq!(got, src);
    }
}
