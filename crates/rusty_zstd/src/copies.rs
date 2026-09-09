//! COPY CENSUS -- how many times does the encoder move each input byte?
//!
//! The catalogue in `tools/copycat.py` reads the emitted asm and says WHERE the
//! `memcpy` calls are. It cannot say how much traffic each one carries, and a
//! call count is a bad proxy: one call on the literal path moves a whole block,
//! while six in a table-setup loop move a few hundred bytes between them.
//!
//! So this counts BYTES, at each site, and the useful figure it produces is
//! **copies per input byte**. That number has a floor and the floor is not
//! zero: an encoder must physically place literal bytes into its output, so one
//! traversal of the literal volume is the job. Anything above one traversal is
//! a byte moved a second time, and that is what is worth removing.
//!
//! Deterministic: byte totals are a property of the input and the code path, so
//! the same corpus gives the same numbers on any machine at any load. No
//! pinning, no interleaving, no noise floor.
//!
//! The counters are thread-local `Cell` bumps folded into process totals, for
//! the same reason as `kreach`: `lock xaddq` per copy would be the instrument
//! dominating what it measures. All of it compiles to nothing without
//! `profile`.

/// `src` -> the block's literal buffer (`push_literals`).
pub const C_LIT_PUSH: usize = 0;
/// literal buffer -> a materialised raw/RLE section.
///
/// Reads ZERO since the raw/RLE arms began writing through `dst`: the
/// allocating twin that produced this traffic has no caller left. Kept as the
/// standing proof that the materialise-then-copy path has not come back.
pub const C_LIT_RAW_SECTION: usize = 1;
/// the finished literals section -> `dst` (`write_literals_inner`).
pub const C_SECTION_TO_DST: usize = 2;
/// Huffman-coded literal bytes emitted into the section.
pub const C_HUFF_EMIT: usize = 3;
/// sequence bytes -> `dst`.
pub const C_SEQ_TO_DST: usize = 4;
/// a finished block -> the frame buffer.
pub const C_BLOCK_TO_FRAME: usize = 5;

/// a RAW block -> `dst` (incompressible input; src to output directly).
pub const C_RAW_BLOCK_TO_DST: usize = 6;

/// STREAMING: the retained window memmoved down by `hist.drain(..drop)`.
pub const C_HIST_SLIDE: usize = 7;
/// STREAMING: the six match tables zeroed by `MatchTables::reset`.
pub const C_TABLE_CLEAR: usize = 8;

/// STREAMING: positions re-inserted by `prime_tables` after a slide.
///
/// NOT a copy -- it is table WORK, counted here because it scales with slide
/// frequency exactly as the memmove and the table clear do, and it is the
/// part of the slide that actually decides whether the frequency is worth
/// tuning. Section 20 measured 503,314,800 of these before the trigger moved.
pub const C_PRIME_INSERT: usize = 9;

/// STREAMING DECODE: compressed bytes copied into the input accumulator.
pub const C_DEC_IN_ACC: usize = 10;
/// STREAMING DECODE: decoded bytes copied out into the caller's buffer.
///
/// Inherent to the streaming contract -- the caller owns the destination --
/// and therefore a cost one-shot `decompress_into` does not pay at all.
pub const C_DEC_OUT: usize = 11;
/// STREAMING DECODE: the decoded-window compaction memmove.
pub const C_DEC_COMPACT: usize = 12;

/// STREAMING DECODE: the INPUT accumulator compaction memmove.
///
/// Distinct from `C_DEC_IN_ACC`, which counts bytes copied IN. This counts
/// the memmove that reclaims the consumed prefix, and it fires per call once
/// the consumed prefix passes 64 KiB -- so with a 64 KiB feed it can fire on
/// every single call, moving whatever is still unconsumed each time.
pub const C_DEC_IN_COMPACT: usize = 13;

/// STREAMING ENCODE: the input-accumulator compaction memmove.
///
/// Sibling of `C_DEC_IN_COMPACT`, same absolute-trigger shape. Measured so
/// the decoder finding is not assumed to transfer.
pub const C_ENC_IN_COMPACT: usize = 14;

/// STREAMING ENCODE: reclaims taken by the free `clear` arm (count, not bytes).
///
/// The CONTROL for `C_ENC_IN_COMPACT`: a zero on the drain arm only means
/// something once this one is non-zero, otherwise the tap is simply unreached.
pub const C_ENC_IN_CLEAR: usize = 15;

/// STREAMING ENCODE: caller bytes -> `in_acc` staging buffer.
pub const C_ENC_IN_ACC: usize = 16;
/// STREAMING ENCODE: `in_acc` -> `hist`, the match window.
///
/// The SECOND copy of every input byte before encoding begins. `in_acc`
/// stages a partial block and `hist` is the window the finders read, so a
/// byte lands in both.
pub const C_ENC_TO_HIST: usize = 17;
/// STREAMING ENCODE: `out_acc` -> the caller's buffer.
pub const C_ENC_OUT: usize = 18;
/// STREAMING ENCODE: the `out_acc` compaction memmove.
pub const C_ENC_OUT_COMPACT: usize = 19;

/// MT: per-job compressed output concatenated into the frame buffer.
pub const C_MT_CONCAT: usize = 20;
/// MT: bytes the concat buffer moved because it GREW instead of reserving.
///
/// A `Vec::new()` grown to N by doubling copies ~N bytes in reallocs, on top
/// of the concat itself -- so an unreserved concatenation of job outputs pays
/// for the compressed stream roughly TWICE.
pub const C_MT_REGROW: usize = 21;

/// DECODE: bytes reserved by extrapolation when the header declared no size.
pub const C_DEC_RESERVE: usize = 22;

/// ENCODE: a finder scratch buffer DISCARDED and reallocated because the
/// pooled one was too small. Bytes = the new capacity.
///
/// `lit_scratch` is sized `block_len + LIT_PUSH_WIDTH_MAX`, i.e. past the
/// 128 KiB large-allocation threshold, so each of these is a fresh VirtualAlloc
/// and a page-table edit. Once per frame is fine; once per BLOCK is not.
pub const C_SCRATCH_REALLOC: usize = 23;

/// Number of census slots.
pub const N_COPY_SLOTS: usize = 24;

/// Human names, index-aligned with the `C_*` constants.
pub const COPY_NAMES: [&str; N_COPY_SLOTS] = [
    "src -> lits",
    "lits -> raw section",
    "section -> dst",
    "huffman emit",
    "sequences -> dst",
    "block -> frame",
    "raw block -> dst",
    "hist slide (memmove)",
    "table clear (memset)",
    "prime inserts (positions)",
    "dec: in -> in_acc",
    "dec: decoded -> caller",
    "dec: compaction memmove",
    "dec: in_acc compaction",
    "enc: in_acc compaction",
    "enc: in_acc clear (control)",
    "enc: caller -> in_acc",
    "enc: in_acc -> hist",
    "enc: out_acc -> caller",
    "enc: out_acc compaction",
    "mt: job -> frame",
    "mt: concat regrow",
    "dec: extrapolated reserve",
    "enc: scratch realloc",
];

#[cfg(not(feature = "profile"))]
mod imp {
    /// Shipping build: the tap folds to nothing.
    #[inline(always)]
    pub fn add(_slot: usize, _bytes: usize) {}
    /// Shipping build: the tap folds to nothing.
    #[inline(always)]
    pub fn flush_this_thread() {}
}

#[cfg(feature = "profile")]
mod imp {
    use super::N_COPY_SLOTS;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub(super) static G_BYTES: [AtomicU64; N_COPY_SLOTS] =
        [const { AtomicU64::new(0) }; N_COPY_SLOTS];
    pub(super) static G_CALLS: [AtomicU64; N_COPY_SLOTS] =
        [const { AtomicU64::new(0) }; N_COPY_SLOTS];

    struct Tls {
        bytes: [Cell<u64>; N_COPY_SLOTS],
        calls: [Cell<u64>; N_COPY_SLOTS],
    }

    fn fold(c: &Cell<u64>, g: &AtomicU64) {
        let v = c.replace(0);
        if v != 0 {
            g.fetch_add(v, Relaxed);
        }
    }

    impl Tls {
        const fn new() -> Self {
            Tls {
                bytes: [const { Cell::new(0) }; N_COPY_SLOTS],
                calls: [const { Cell::new(0) }; N_COPY_SLOTS],
            }
        }
        fn flush(&self) {
            for (c, g) in self.bytes.iter().zip(G_BYTES.iter()) {
                fold(c, g);
            }
            for (c, g) in self.calls.iter().zip(G_CALLS.iter()) {
                fold(c, g);
            }
        }
    }

    impl Drop for Tls {
        fn drop(&mut self) {
            self.flush();
        }
    }

    std::thread_local! {
        static TLS: Tls = const { Tls::new() };
    }

    /// Record `bytes` moved at `slot`.
    #[inline(always)]
    pub fn add(slot: usize, bytes: usize) {
        let _ = TLS.try_with(|t| {
            t.bytes[slot].set(t.bytes[slot].get() + bytes as u64);
            t.calls[slot].set(t.calls[slot].get() + 1);
        });
    }

    /// Fold this thread's cells into the process totals.
    pub fn flush_this_thread() {
        let _ = TLS.try_with(|t| t.flush());
    }
}

pub use imp::{add, flush_this_thread};

/// Read and clear the census: `[(bytes, calls); N_COPY_SLOTS]`.
#[cfg(feature = "profile")]
pub fn take() -> [(u64, u64); N_COPY_SLOTS] {
    use core::sync::atomic::Ordering::Relaxed;
    imp::flush_this_thread();
    let mut out = [(0u64, 0u64); N_COPY_SLOTS];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (
            imp::G_BYTES[i].swap(0, Relaxed),
            imp::G_CALLS[i].swap(0, Relaxed),
        );
    }
    out
}
