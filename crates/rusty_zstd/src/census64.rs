//! The census counter's 64-bit atomic, and what it becomes on a part that has
//! no 64-bit atomics.
//!
//! WHY THIS MODULE EXISTS. Every measurement instrument in this crate counts
//! into a `static AtomicU64`: reload calls, decode band histograms, kernel
//! reach, walk exits, copy bytes. That is deliberate and the codec skills
//! depend on it -- a count is the only instrument that separates "this kernel
//! does not help" from "this arm is wired to nothing".
//!
//! It also cost the crate two targets. `core::sync::atomic::AtomicU64` does
//! not exist on a Cortex-M4F (`thumbv7em-none-eabihf`) or on RV32
//! (`riscv32imac-unknown-none-elf`), so `--no-default-features --features
//! alloc` failed with 204 errors on both -- every one of them a counter, not a
//! line of codec. A `no-std` label on the registry is not a claim; a target
//! that compiles is.
//!
//! WHAT IT DOES. On every target that HAS 64-bit atomics this is exactly
//! `core::sync::atomic::AtomicU64`, re-exported. Same type, same ABI, no
//! wrapper, so the public statics keep their published type and the emitted
//! code is byte-for-byte what it was.
//!
//! On a target that does not, it is a zero-sized stub whose operations do
//! nothing and whose loads read 0. The statics stop occupying BSS, the
//! `fetch_add`s vanish, and the codec keeps its targets.
//!
//! WHY A STUB AND NOT `portable-atomic`. That crate is the other honest
//! answer, and it would keep the census READABLE on those parts. But on a
//! core with no 64-bit atomic instruction it needs a critical-section
//! implementation, and a library that turns that feature on forces the choice
//! onto every downstream firmware. A codec should not conscript the
//! application's interrupt policy so that a diagnostic counter can increment.
//! Anyone who does want the count on such a part can supply
//! `portable_atomic::AtomicU64` here; the seam is one type wide.
//!
//! HONESTY. A counter that silently reads zero is exactly the "stale
//! instrument" this crate's own rules warn about, so the fact is published
//! rather than buried: [`CENSUS_LIVE`] is `false` on such a target, and any
//! gate or report that asserts on a census must consult it before believing a
//! zero. **Zero here means "not measurable on this target", never "measured
//! zero".**
//!
//! The reverse mistake would be worse and is gated below: if this stub were
//! ever selected on a HOSTED target, every counter in the crate would read 0
//! and every census-based verdict would silently become fiction. The unit test
//! fails the build if that happens.

/// Whether the census counters in this build can actually count.
///
/// `true` wherever `AtomicU64` exists (every hosted target this crate ships
/// on). `false` on a part without 64-bit atomics, where every counter is a
/// stub and every reader returns 0. Check this before believing a zero.
pub const CENSUS_LIVE: bool = cfg!(target_has_atomic = "64");

#[cfg(target_has_atomic = "64")]
pub use core::sync::atomic::AtomicU64;

#[cfg(not(target_has_atomic = "64"))]
pub use stub::AtomicU64;

#[cfg(not(target_has_atomic = "64"))]
mod stub {
    use core::sync::atomic::Ordering;

    /// A do-nothing stand-in for `core::sync::atomic::AtomicU64` on a target
    /// that has no 64-bit atomics. Zero-sized: a `static` of this type costs
    /// no BSS and every operation folds away.
    ///
    /// It is `Sync` for the same reason `()` is: there is no state to race
    /// on. Loads return 0, and [`super::CENSUS_LIVE`] is `false` so a reader
    /// can tell that apart from a real zero.
    #[derive(Debug, Default)]
    pub struct AtomicU64(());

    impl AtomicU64 {
        /// The value is discarded: this build cannot hold one.
        #[inline(always)]
        pub const fn new(_v: u64) -> Self {
            Self(())
        }

        /// Always 0. See [`super::CENSUS_LIVE`].
        #[inline(always)]
        pub fn load(&self, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn store(&self, _v: u64, _order: Ordering) {}

        /// Always 0 (the previous value this build never held).
        #[inline(always)]
        pub fn swap(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_add(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_sub(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_max(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_min(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_or(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        #[inline(always)]
        pub fn fetch_and(&self, _v: u64, _order: Ordering) -> u64 {
            0
        }

        /// Always reports the stored value as 0, so a compare against
        /// anything else fails -- the same shape a real CAS has when it loses.
        #[inline(always)]
        pub fn compare_exchange(
            &self,
            current: u64,
            _new: u64,
            _success: Ordering,
            _failure: Ordering,
        ) -> Result<u64, u64> {
            if current == 0 {
                Ok(0)
            } else {
                Err(0)
            }
        }

        #[inline(always)]
        pub fn compare_exchange_weak(
            &self,
            current: u64,
            new: u64,
            success: Ordering,
            failure: Ordering,
        ) -> Result<u64, u64> {
            self.compare_exchange(current, new, success, failure)
        }
    }
}

/// The selected type must be a REAL 64-bit atomic wherever one exists. If a cfg
/// edit ever let the stub through here, this fails the build rather than the
/// census. (The stub is zero-sized; the real one is eight bytes.)
#[cfg(target_has_atomic = "64")]
const _: () = [(); 1][(core::mem::size_of::<AtomicU64>() != 8) as usize];

#[cfg(test)]
mod tests {
    /// The census must actually COUNT anywhere tests run. Asserted on behaviour,
    /// not on the `CENSUS_LIVE` constant: if a cfg edit ever selected the stub on
    /// a hosted target, every counter in the crate would read 0, `kreach_gate`
    /// would see a perfectly-routed 0/0 everywhere, and the whole instrument
    /// would report success while measuring nothing.
    #[test]
    fn census_counts_wherever_tests_run() {
        assert!(super::CENSUS_LIVE == cfg!(target_has_atomic = "64"));
        let c = super::AtomicU64::new(0);
        c.fetch_add(7, core::sync::atomic::Ordering::Relaxed);
        c.fetch_add(35, core::sync::atomic::Ordering::Relaxed);
        assert_eq!(
            c.swap(0, core::sync::atomic::Ordering::Relaxed),
            42,
            "the AtomicU64 stub was selected on a target that HAS 64-bit atomics: \
             every census counter in this crate now reads 0 and every count-based \
             verdict from it is fiction"
        );
    }
}
