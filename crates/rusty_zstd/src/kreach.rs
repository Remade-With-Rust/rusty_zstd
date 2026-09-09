//! KERNEL REACH CENSUS -- does the shipping path actually call the kernel?
//!
//! Every other gate in this crate is blind to the one defect this module
//! exists to find. A kernel that is written, tested, benchmarked and NOT
//! CALLED passes byte-identity (the two paths agree by design), passes the
//! round-trip, passes the conformance suite, and reads FLAT under an
//! arm-toggle A/B -- which looks exactly like "this kernel does not help"
//! and gets recorded as a refutation that nothing ever revisits.
//!
//! The only instrument that separates "the kernel does not help" from "the
//! arm is not wired to anything" is a COUNT of how much work goes down each
//! path. It is deterministic: same number on any machine, at any load, with
//! no pinning, no ABBA, no noise floor and no z-score. One run is the answer.
//!
//! ## The tap must not become the thing it measures
//!
//! `count_eq_len` runs ~247M times at L19 and `emit_fast_seq` once per
//! sequence. An `AtomicU64::fetch_add` lowers to `lock xaddq` on x86-64 at
//! every ordering -- a bus-locked full-barrier RMW -- so a per-call atomic
//! there would be the instrument dominating the measurement (this is exactly
//! what already inflated every pre-existing `EQ_OPS` share in this crate).
//! So the per-call counters are thread-local `Cell` bumps (load/add/store),
//! folded into the process totals when the thread ends or on an explicit
//! flush. Per-block sites could afford atomics but use the same path anyway,
//! because one shape is easier to trust than two.
//!
//! ## The label is part of the instrument
//!
//! A bucket printed as "scalar" that actually counts calls INTO a kernel
//! manufactures a finding that does not exist. Each slot below names one
//! dispatch site, and `hit` is bumped on the side that reaches the kernel,
//! `miss` on the side that does not -- both AT the dispatch, never inferred
//! from an eligibility test upstream of it. In particular `simd::eq_call`'s
//! existing `wide_eligible` counter is NOT kernel reach: it counts calls
//! where `max >= 64`, which is the vector arm's ELIGIBILITY, and most of
//! those are resolved by the 32-byte word ladder before any kernel runs.

/// One slot per shipping dispatch site.
///
/// ENCODE side.
pub const K_COUNT_EQ_WIDE: usize = 0;
/// `find_fast_impl` -- BMI2 twin vs baseline, once per block.
pub const K_FIND_FAST: usize = 1;
/// `emit_fast_seq` -- BMI2 twin vs baseline, once per emitted sequence.
pub const K_EMIT_FAST_SEQ: usize = 2;
/// `fse::compress_using_ctable` -- BMI2 twin vs baseline.
pub const K_FSE_CTABLE: usize = 3;
/// `fse::weights_into` -- BMI2 twin vs baseline.
pub const K_FSE_WEIGHTS: usize = 4;
/// `huffman::encode_stream_unrolled` -- BMI2 twin vs baseline.
pub const K_HUF_ENC: usize = 5;
/// DECODE side. `decode_sequences` -- the duplicated-loop twin vs baseline.
pub const K_DEC_SEQ: usize = 6;
/// `huffman::decode_4x` -- BMI2 twin vs baseline.
pub const K_HUF_DEC4X: usize = 7;
/// `huffman::decode_4x_x1` -- BMI2 twin vs baseline.
pub const K_HUF_DEC4X1: usize = 8;
/// BOTH sides. `xxh64` stripe loop -- AVX2/NEON kernel vs scalar stripes.
pub const K_XXH_STRIPE: usize = 9;

/// Number of census slots.
pub const N_SLOTS: usize = 10;

/// Human names, index-aligned with the `K_*` constants above. `(name, side)`.
pub const SLOT_NAMES: [(&str, &str); N_SLOTS] = [
    ("count_eq_len wide", "enc"),
    ("find_fast_impl", "enc"),
    ("emit_fast_seq", "enc"),
    ("fse compress_ctable", "enc"),
    ("fse weights_into", "enc"),
    ("huffman encode_stream", "enc"),
    ("decode_sequences", "dec"),
    ("huffman decode_4x", "dec"),
    ("huffman decode_4x_x1", "dec"),
    ("xxh64 stripes", "both"),
];

#[cfg(not(feature = "profile"))]
mod imp {
    /// Shipping build: every tap folds to nothing.
    #[inline(always)]
    pub fn hit(_slot: usize) {}
    /// Shipping build: every tap folds to nothing.
    #[inline(always)]
    pub fn miss(_slot: usize) {}
    /// Shipping build: every tap folds to nothing.
    #[inline(always)]
    pub fn flush_this_thread() {}
}

#[cfg(feature = "profile")]
mod imp {
    use super::N_SLOTS;
    use core::cell::Cell;
    use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub(super) static G_HIT: [AtomicU64; N_SLOTS] = [const { AtomicU64::new(0) }; N_SLOTS];
    pub(super) static G_MISS: [AtomicU64; N_SLOTS] = [const { AtomicU64::new(0) }; N_SLOTS];

    struct Tls {
        hit: [Cell<u64>; N_SLOTS],
        miss: [Cell<u64>; N_SLOTS],
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
                hit: [const { Cell::new(0) }; N_SLOTS],
                miss: [const { Cell::new(0) }; N_SLOTS],
            }
        }
        fn flush(&self) {
            for (c, g) in self.hit.iter().zip(G_HIT.iter()) {
                fold(c, g);
            }
            for (c, g) in self.miss.iter().zip(G_MISS.iter()) {
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

    /// Count one call that REACHED the kernel at `slot`.
    #[inline(always)]
    pub fn hit(slot: usize) {
        let _ = TLS.try_with(|t| t.hit[slot].set(t.hit[slot].get() + 1));
    }

    /// Count one call that did NOT reach the kernel at `slot`.
    #[inline(always)]
    pub fn miss(slot: usize) {
        let _ = TLS.try_with(|t| t.miss[slot].set(t.miss[slot].get() + 1));
    }

    /// Fold this thread's cells into the process totals.
    pub fn flush_this_thread() {
        let _ = TLS.try_with(|t| t.flush());
    }
}

pub use imp::{flush_this_thread, hit, miss};

/// Read and clear the whole census: `[(hit, miss); N_SLOTS]`.
///
/// Flushes the calling thread first. Worker threads fold on their own `Drop`,
/// so a multi-threaded run must be joined before this is read.
#[cfg(feature = "profile")]
pub fn take() -> [(u64, u64); N_SLOTS] {
    use core::sync::atomic::Ordering::Relaxed;
    imp::flush_this_thread();
    let mut out = [(0u64, 0u64); N_SLOTS];
    for (i, o) in out.iter_mut().enumerate() {
        *o = (
            imp::G_HIT[i].swap(0, Relaxed),
            imp::G_MISS[i].swap(0, Relaxed),
        );
    }
    out
}
