//! Thread CPU clock for the stage profiler -- an INSTRUMENT fix, not a codec
//! change and not a corpus change.
//!
//! ## Why not `Instant`
//!
//! `Instant` is a WALL clock. Every moment the thread spends DESCHEDULED gets
//! charged to whichever scope happened to be open, so on a loaded box the stage
//! timings absorb other processes' scheduling. codec-measurement 2 measured
//! this directly on identical work: elapsed-wall spread **0.78-1.50** against
//! CPU-time spread **0.950-1.089** -- roughly 5x tighter, and it needs no quiet
//! machine.
//!
//! It cost a real result here. `d9probe`/`d10probe` reported worst-pair null
//! arms of 27-33%, and E1's genuine **-23.6%, z = +9.95** was written down as
//! "unmeasurable, needs a quiet box". Part of that width was this clock
//! counting time the process did not spend running.
//!
//! ## What each platform gives
//!
//! * **Windows** -- `QueryThreadCycleTime`: CPU CYCLES the calling thread
//!   actually executed. Descheduled time is excluded outright.
//! * **Linux** -- `clock_gettime(CLOCK_THREAD_CPUTIME_ID)`: nanoseconds of
//!   thread CPU time, same property.
//! * **Everything else** -- `Instant`, i.e. the old wall behaviour, so no
//!   platform loses the profiler. It keeps the old caveat.
//!
//! Windows counts cycles and every consumer of `prof` reports nanoseconds, so
//! [`to_ns`] converts. `CYC_PER_NS` is calibrated ONCE against `Instant` over
//! short busy spins, keeping the HIGHEST cycles-per-wall-ns ratio across
//! trials: descheduling inflates wall while leaving cycles alone, so the
//! largest ratio is the least-interrupted sample. Absolute nanoseconds
//! therefore stay comparable with every figure already recorded in
//! `docs/plans/`.
//!
//! ## The tap's own cost
//!
//! codec-measurement 6 says price the tap before placing it.
//! `QueryThreadCycleTime` costs ~50-100 ns against `Instant::now()`'s ~20-30 ns.
//! Scopes in `prof` are per BLOCK, never per sequence (see
//! `Stage::DecSeqHeader`), so a 128 KiB block pays the difference twice against
//! thousands of sequences of real work. That tax is far below the wall clock's
//! own noise, which is the thing being removed.

#[cfg(all(windows, feature = "std"))]
mod imp {
    // This module is FFI by definition: the whole point is to reach a
    // clock the standard library does not expose.
    #![allow(unsafe_code)]
    use core::sync::atomic::{AtomicU64, Ordering};

    #[link(name = "kernel32")]
    extern "system" {
        fn GetCurrentThread() -> *mut core::ffi::c_void;
        fn QueryThreadCycleTime(thread: *mut core::ffi::c_void, cycles: *mut u64) -> i32;
    }

    /// Cycles per nanosecond, as `f64` bits. 0 means not yet calibrated.
    static CYC_PER_NS: AtomicU64 = AtomicU64::new(0);

    #[inline(always)]
    fn raw_cycles() -> u64 {
        let mut c = 0u64;
        // SAFETY: `GetCurrentThread` returns a pseudo-handle that is always
        // valid for the calling thread and must not be closed. `&mut c` is a
        // valid writable `u64`. A non-zero return means `c` was written.
        unsafe {
            if QueryThreadCycleTime(GetCurrentThread(), &mut c) != 0 {
                c
            } else {
                0
            }
        }
    }

    /// Cycles per wall-nanosecond; the least-descheduled trial wins.
    fn calibrate() -> f64 {
        use std::time::Instant;
        let mut best = 0f64;
        for _ in 0..5 {
            let w0 = Instant::now();
            let c0 = raw_cycles();
            // ~2 ms of spinning: short enough that the whole calibration is
            // ~10 ms once per process, long enough to swamp both clocks'
            // resolution.
            while w0.elapsed().as_micros() < 2000 {
                core::hint::spin_loop();
            }
            let c1 = raw_cycles();
            let w = w0.elapsed().as_nanos() as f64;
            let c = c1.saturating_sub(c0) as f64;
            if w > 0.0 && c > 0.0 {
                // Descheduling inflates `w` and leaves `c` alone, so the
                // LARGEST ratio ran most nearly uninterrupted.
                best = best.max(c / w);
            }
        }
        // A machine where the probe fails still has to produce something
        // monotone; 1.0 makes ticks read as nanoseconds.
        if best > 0.0 {
            best
        } else {
            1.0
        }
    }

    #[inline]
    fn cyc_per_ns() -> f64 {
        let bits = CYC_PER_NS.load(Ordering::Relaxed);
        if bits != 0 {
            return f64::from_bits(bits);
        }
        let v = calibrate();
        CYC_PER_NS.store(v.to_bits(), Ordering::Relaxed);
        v
    }

    /// Force calibration now, so it can never land inside a measured scope.
    pub fn warm() {
        let _ = cyc_per_ns();
    }

    #[inline(always)]
    pub fn now() -> u64 {
        raw_cycles()
    }

    #[inline(always)]
    pub fn to_ns(ticks: u64) -> u64 {
        (ticks as f64 / cyc_per_ns()) as u64
    }
}

#[cfg(all(target_os = "linux", feature = "std"))]
mod imp {
    // This module is FFI by definition: the whole point is to reach a
    // clock the standard library does not expose.
    #![allow(unsafe_code)]
    #[repr(C)]
    struct Timespec {
        tv_sec: i64,
        tv_nsec: i64,
    }
    extern "C" {
        fn clock_gettime(clk_id: i32, tp: *mut Timespec) -> i32;
    }

    /// `CLOCK_THREAD_CPUTIME_ID` on Linux. Deliberately NOT applied to macOS,
    /// where the constant differs -- that platform keeps the wall fallback
    /// rather than silently reading the wrong clock.
    const CLOCK_THREAD_CPUTIME_ID: i32 = 3;

    pub fn warm() {}

    #[inline(always)]
    pub fn now() -> u64 {
        let mut ts = Timespec {
            tv_sec: 0,
            tv_nsec: 0,
        };
        // SAFETY: `clock_gettime` writes only through the pointer given, which
        // is a valid stack `Timespec`.
        unsafe {
            if clock_gettime(CLOCK_THREAD_CPUTIME_ID, &mut ts) == 0 {
                (ts.tv_sec as u64)
                    .wrapping_mul(1_000_000_000)
                    .wrapping_add(ts.tv_nsec as u64)
            } else {
                0
            }
        }
    }

    /// Already nanoseconds.
    #[inline(always)]
    pub fn to_ns(ticks: u64) -> u64 {
        ticks
    }
}

/// Wall fallback, so no platform loses the profiler. This is the OLD behaviour
/// and carries the old caveat: it counts descheduled time.
#[cfg(not(any(
    all(windows, feature = "std"),
    all(target_os = "linux", feature = "std")
)))]
mod imp {
    use std::sync::OnceLock;
    use std::time::Instant;
    static T0: OnceLock<Instant> = OnceLock::new();

    pub fn warm() {
        let _ = T0.get_or_init(Instant::now);
    }

    #[inline(always)]
    pub fn now() -> u64 {
        T0.get_or_init(Instant::now).elapsed().as_nanos() as u64
    }

    #[inline(always)]
    pub fn to_ns(ticks: u64) -> u64 {
        ticks
    }
}

pub(crate) use imp::{now, to_ns, warm};
