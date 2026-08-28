//! Runtime-dispatched SIMD kernels. Safe wrappers; `unsafe` stays in this file.
//!
//! Why the compiler cannot auto-vec `count_eq_len`: the loop returns on the first
//! mismatch (early exit). CRT `memcmp` of a whole equal run is a call, then a
//! second scan to find the index. AVX2/NEON `pcmpeqb` + mask tzcnt is one pass.

/// Hint the CPU to start loading `slice[at]` into L1.
///
/// UNUSED. Bricks 42 (decoder match source) and 43 (encoder candidate) both
/// tried it and both measured WORSE -- see `m7-encoder-whys.md`. The reason is
/// the same in both: the target is already cache-resident, so there is no miss
/// to hide and the hint is pure instruction overhead. Kept because the
/// primitive is correct -- so it lives in the TEST build only, where "kept
/// because it is correct" costs nothing. It used to be `#[allow(dead_code)]`
/// in the shipping build, with that attribute sitting in the MIDDLE of this
/// doc comment. That is the same drift that put `#[cfg(feature = "std")]` on
/// `params::CPARAM_CLAMP_ARM` while both its users stayed ungated and broke
/// the whole `no_std + alloc` build; an attribute wedged between doc lines is
/// a hazard, not a style choice.
///
/// A pure HINT: it cannot fault, cannot change any value, and an out-of-range
/// `at` simply does nothing. So any code path using it is byte-identical by
/// construction -- no oracle needed, only a benchmark.
///
/// The match copy in `decode_sequences` reads from a random earlier offset,
/// which is the decoder's one unpredictable load. C ships a whole separate
/// path for this (`ZSTD_decompressSequencesLong` + `ZSTD_DECODESEQUENCE_PREFETCH`).
#[cfg(test)]
#[inline(always)]
#[allow(dead_code)] // ISA hint kept beside its kernel; no shipping caller today.
pub(crate) fn prefetch_read(slice: &[u8], at: usize) {
    if at >= slice.len() {
        return;
    }
    #[cfg(target_arch = "x86_64")]
    {
        // SAFETY: `at < slice.len()`, so the pointer is in-bounds of a live
        // allocation. `_mm_prefetch` only touches the cache hierarchy; it
        // never dereferences architecturally and has no observable effect.
        unsafe {
            core::arch::x86_64::_mm_prefetch(
                slice.as_ptr().add(at) as *const i8,
                core::arch::x86_64::_MM_HINT_T0,
            );
        }
    }
    // aarch64: NO-OP ON PURPOSE. `core::arch::aarch64::_prefetch` and its
    // `_PREFETCH_*` constants are still unstable (rust-lang #117217), so this
    // arm did not compile on stable -- `cargo check --target
    // aarch64-unknown-linux-gnu` failed on the whole crate because of a
    // function nothing calls. Since a prefetch is a pure hint and both bricks
    // that tried it measured WORSE, dropping to a no-op costs nothing and
    // gives the NEON twin below a target that actually builds. Restore it with
    // `asm!("prfm pldl1keep, [{}]")` if a use ever justifies it.
    #[cfg(not(target_arch = "x86_64"))]
    {
        let _ = at;
    }
}

/// Unaligned little-endian `u32`. Caller: `i + 4 <= src.len()`.
#[inline(always)]
pub(crate) fn load_u32_le(src: &[u8], i: usize) -> u32 {
    // `i + 4 <= len` OVERFLOWS for `i` near `usize::MAX` and panics in debug
    // before the real condition is ever evaluated. Subtract instead.
    debug_assert!(src.len() >= 4 && i <= src.len() - 4);
    // SAFETY: caller proves `i + 4 <= src.len()`. Unaligned [u8; 4] then LE
    // integer — not a native-endian `u32` load (wrong on BE).
    let arr = unsafe { src.as_ptr().add(i).cast::<[u8; 4]>().read_unaligned() };
    u32::from_le_bytes(arr)
}

/// Unaligned little-endian `u64`. Caller: `i + 8 <= src.len()`.
#[inline(always)]
pub(crate) fn load_u64_le(src: &[u8], i: usize) -> u64 {
    // As above: written as a subtraction so the assert cannot overflow.
    debug_assert!(src.len() >= 8 && i <= src.len() - 8);
    // SAFETY: caller proves `i + 8 <= src.len()`.
    let arr = unsafe { src.as_ptr().add(i).cast::<[u8; 8]>().read_unaligned() };
    u64::from_le_bytes(arr)
}

/// Drive the AVX2 kernel under its REAL contract: prove bytes [0, 32) equal
/// with the same word ladder `count_eq_len_ge8_raw` runs, then hand over the
/// ORIGINAL pointers and length.
///
/// The bench arm and the oracle tests both go through this, so they exercise
/// the shipping shape. Calling the kernel directly with `max >= 32` -- which is
/// what they used to do -- now silently skips the first 32 bytes, and nothing
/// in the type system says so.
///
/// # Safety
/// `a[0..max]` and `b[0..max]` readable, and `max >= 64`.
#[cfg(all(target_arch = "x86_64", any(test, feature = "profile")))]
#[inline]
unsafe fn avx2_with_ladder(a: *const u8, b: *const u8, max: usize) -> usize {
    debug_assert!(max >= 64);
    // SAFETY: `max >= 64` makes bytes [0, 32) readable; the kernel's contract
    // is then satisfied by the ladder above it.
    unsafe {
        if let Some(r) = ladder32(a, b) {
            return r;
        }
        count_eq_len_avx2(a, b, max)
    }
}

/// Common prefix length of `a` and `b` (min of the two lengths).
/// Bench entry points for the GATE 15 latency study (4.60). `count_eq_len` is
/// where the two implementations differ; whole-encode timing cannot resolve a
/// four-cycle dependency-chain difference, a tight loop can.
#[cfg(feature = "profile")]
pub fn bench_eq_avx2(a: &[u8], b: &[u8]) -> usize {
    let max = a.len().min(b.len());
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        // `max >= 64` is the dispatch's gate AND the kernel's contract; going
        // through `avx2_with_ladder` measures the shipping shape (ladder then
        // kernel) rather than a kernel entered on terms it no longer accepts.
        if max >= 64 && has_avx2() {
            // SAFETY: `a[..max]` and `b[..max]` are in bounds and `max >= 64`.
            return unsafe { avx2_with_ladder(a.as_ptr(), b.as_ptr(), max) };
        }
    }
    count_eq_len_words(a, b, max)
}

/// The word-loop twin, for the same study.
#[cfg(feature = "profile")]
pub fn bench_eq_words(a: &[u8], b: &[u8]) -> usize {
    count_eq_len_words(a, b, a.len().min(b.len()))
}

/// GATE 15 arm. 0 = shipped (AVX2 where available), 1 = force the word loop,
/// 2 = peek the first 8 bytes before going wide.
///
/// The question the CPU-capability dispatch does not answer: AVX2's first loop
/// reads 64 bytes per side before it can return, and at L3 the mean match is
/// ~9.6 bytes with literal runs of 3.75. Most `count_match` calls die inside the
/// first word, so the wide load is memory traffic for a result eight bytes of
/// it already decided.
#[cfg(feature = "profile")]
pub static EQLEN_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook: select the common-prefix implementation. Present ONLY under
/// `--features profile` -- `count_eq_len` runs 247M times at L19, so an atomic
/// load here would be 247M loads in the shipped build to serve a bench knob.
#[cfg(feature = "profile")]
pub fn set_eqlen_arm(v: u8) {
    EQLEN_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[cfg(feature = "profile")]
#[inline(always)]
fn eqlen_arm() -> u8 {
    EQLEN_ARM.load(core::sync::atomic::Ordering::Relaxed)
}

#[cfg(not(feature = "profile"))]
#[inline(always)]
fn eqlen_arm() -> u8 {
    0
}

/// GATE 15 study: how long ARE the prefixes this returns, and how often is the
/// wide path even eligible?
///
/// THE COUNTERS ARE THREAD-LOCAL CELLS, NOT ATOMICS. `AtomicU64::fetch_add`
/// lowers to `lock xaddq` on x86-64 whatever ordering you ask for -- a
/// bus-locked, full-barrier RMW -- and the word counter fires once per 8-byte
/// compare, on the order of 1e9 times at L19. That is the instrument
/// dominating the thing it measures, and it means every EQ_OPS-derived share
/// taken BEFORE this change is inflated. A `Cell<u64>` bump is load/add/store.
///
/// Totals stay per-process: a thread folds its cells into the global atomics
/// on `take_*` (its own cells) and on thread exit (the `Drop` impl). A worker
/// still running when `take_*` is called has not contributed yet -- the GATE
/// benches compress single-threaded, so the receipt is exact for them.
#[cfg(feature = "profile")]
mod counters {
    use core::cell::Cell;
    use core::sync::atomic::{AtomicU64, Ordering::Relaxed};

    pub(super) static G_CALLS: AtomicU64 = AtomicU64::new(0);
    pub(super) static G_WIDE: AtomicU64 = AtomicU64::new(0);
    /// Compare operations executed: wide (32B cmpeq), word (8B), byte.
    pub(super) static G_OPS: [AtomicU64; 3] =
        [AtomicU64::new(0), AtomicU64::new(0), AtomicU64::new(0)];
    pub(super) static G_HIST: [AtomicU64; 5] = [
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
        AtomicU64::new(0),
    ];

    pub(super) struct Tls {
        pub(super) calls: Cell<u64>,
        pub(super) wide: Cell<u64>,
        pub(super) ops: [Cell<u64>; 3],
        pub(super) hist: [Cell<u64>; 5],
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
                calls: Cell::new(0),
                wide: Cell::new(0),
                ops: [Cell::new(0), Cell::new(0), Cell::new(0)],
                hist: [
                    Cell::new(0),
                    Cell::new(0),
                    Cell::new(0),
                    Cell::new(0),
                    Cell::new(0),
                ],
            }
        }

        fn flush(&self) {
            fold(&self.calls, &G_CALLS);
            fold(&self.wide, &G_WIDE);
            for (c, g) in self.ops.iter().zip(G_OPS.iter()) {
                fold(c, g);
            }
            for (c, g) in self.hist.iter().zip(G_HIST.iter()) {
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
        pub(super) static TLS: Tls = const { Tls::new() };
    }

    /// Fold this thread's cells into the process totals.
    pub(super) fn flush_this_thread() {
        let _ = TLS.try_with(|t| t.flush());
    }
}

/// Add `n` to a compare counter: 0 = wide (32B cmpeq), 1 = word (8B), 2 = byte.
///
/// ONE thread-local access, taking a COUNT. The counters used to be bumped once
/// per compare -- four TLS lookups for the word ladder, two per 64-byte vector
/// iteration, four per 32-byte scalar block -- which is the instrument
/// competing with the loop it measures (codec-measurement §6). Every site now
/// accumulates in a register and folds in once at its exit.
#[cfg(feature = "profile")]
#[inline(always)]
fn eq_op_n(kind: usize, n: usize) {
    if n == 0 {
        return;
    }
    let _ = counters::TLS.try_with(|t| {
        let c = &t.ops[kind];
        c.set(c.get() + n as u64);
    });
}

/// Shipping twin: a real no-op, so the CALL SITES need no `#[cfg]`.
///
/// Every counter bump used to be a `#[cfg(feature = "profile")]` statement
/// wedged into a hot function body. This crate has now been bitten twice by
/// attributes drifting onto the wrong item (`prefetch_read`,
/// `params::CPARAM_CLAMP_ARM` -- the latter broke the whole `no_std + alloc`
/// build), so removing a dozen of them from the bodies that matter most is
/// worth more than the zero instructions it costs.
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn eq_op_n(_kind: usize, _n: usize) {}

/// One `count_eq_len_ge8_raw` entry; shipping twin is a no-op.
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn eq_call(_wide_eligible: bool) {}

/// One `count_eq_len_ge8` entry; `wide_eligible` is `max >= 64`.
#[cfg(feature = "profile")]
#[inline(always)]
fn eq_call(wide_eligible: bool) {
    let _ = counters::TLS.try_with(|t| {
        t.calls.set(t.calls.get() + 1);
        if wide_eligible {
            t.wide.set(t.wide.get() + 1);
        }
    });
}

/// Read and clear `(wide_ops, word_ops, byte_ops)`.
#[cfg(feature = "profile")]
pub fn take_eq_ops() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    counters::flush_this_thread();
    (
        counters::G_OPS[0].swap(0, Relaxed),
        counters::G_OPS[1].swap(0, Relaxed),
        counters::G_OPS[2].swap(0, Relaxed),
    )
}

/// Read and clear `(calls, wide_eligible, [<8, 8-31, 32-63, 64-255, 256+])`.
#[cfg(feature = "profile")]
pub fn take_eqlen_stats() -> (u64, u64, [u64; 5]) {
    use core::sync::atomic::Ordering::Relaxed;
    counters::flush_this_thread();
    let mut h = [0u64; 5];
    for (i, v) in counters::G_HIST.iter().enumerate() {
        h[i] = v.swap(0, Relaxed);
    }
    (
        counters::G_CALLS.swap(0, Relaxed),
        counters::G_WIDE.swap(0, Relaxed),
        h,
    )
}

/// Common prefix length of `a` and `b` (min of the two lengths).
///
/// TEST-ONLY, and rustc had been saying so (`function count_eq_len is never
/// used`). The shipping caller is `encode::count_match`, which answers the
/// sub-8 case itself -- as ONE masked compare when the frame has 8-byte room
/// past `ip` -- and enters `count_eq_len_ge8` with `max >= 8` already proven.
/// Keeping this out of the release build keeps it out of the I-cache too.
#[cfg(test)]
pub(crate) fn count_eq_len(a: &[u8], b: &[u8]) -> usize {
    let max = a.len().min(b.len());
    // No `max == 0` early-out: the sub-8 loop below already answers 0 for it,
    // so that compare-and-branch was dead on every call.
    if max < 8 {
        let mut n = 0usize;
        while n < max && a[n] == b[n] {
            n += 1;
        }
        return n;
    }
    count_eq_len_ge8(a, b, max)
}

/// One 8-byte compare of the ladder. `Some(index of the first differing byte)`
/// or `None` when the word matched.
///
/// # Safety
/// `a[n..n+8]` and `b[n..n+8]` must be readable.
#[inline(always)]
unsafe fn word_ne(a: *const u8, b: *const u8, n: usize) -> Option<usize> {
    // SAFETY: the caller guarantees both 8-byte reads are in bounds.
    let (av, bv) = unsafe {
        (
            core::ptr::read_unaligned(a.add(n) as *const u64),
            core::ptr::read_unaligned(b.add(n) as *const u64),
        )
    };
    if av != bv {
        Some(n + ((av ^ bv).trailing_zeros() as usize) / 8)
    } else {
        None
    }
}

/// The 32-byte word ladder: four 8-byte compares, `Some(first differing byte)`
/// or `None` if all 32 matched.
///
/// ONE COPY. `avx2_with_ladder` used to carry its own transcription of this,
/// which is precisely how the bench and the oracle tests came to drive the
/// kernel on a contract it no longer had -- a twin that compiled fine and
/// tested the wrong thing. The kernels' contract ("bytes [0, 32) proven
/// equal") is now expressible only by calling this.
///
/// It stays `#[inline(always)]`: the returned-length histogram says word 0
/// decides 50.5% of calls, so half of all calls must answer without a call.
///
/// # Safety
/// `a[0..32]` and `b[0..32]` must be readable.
#[inline(always)]
unsafe fn ladder32(a: *const u8, b: *const u8) -> Option<usize> {
    // SAFETY: the caller guarantees 32 readable bytes on both sides.
    unsafe {
        if let Some(r) = word_ne(a, b, 0) {
            eq_op_n(1, 1);
            return Some(r);
        }
        if let Some(r) = word_ne(a, b, 8) {
            eq_op_n(1, 2);
            return Some(r);
        }
        if let Some(r) = word_ne(a, b, 16) {
            eq_op_n(1, 3);
            return Some(r);
        }
        if let Some(r) = word_ne(a, b, 24) {
            eq_op_n(1, 4);
            return Some(r);
        }
        eq_op_n(1, 4);
        None
    }
}

/// Words from `n`, then the sub-8 remainder as ONE overlapped compare.
///
/// THE SINGLE COPY. This exact shape -- word ladder, then an overlapped tail
/// counted from `max - 8` -- had grown four separate implementations: the two
/// arms of `count_eq_len_ge8`, `count_eq_len_words_raw`, and the AVX2 kernel,
/// with the NEON twin running a per-byte loop instead. Four copies is how the
/// tail drifted apart in the first place (the AVX2 kernel had the overlapped
/// form for bricks while its twins did not), and duplicating the ladder is
/// what grew `count_match` from 189 to 209 static instructions last round.
///
/// The tail needs no shift: bytes `[max-8, n)` sit inside the region the
/// ladder just proved EQUAL, so their XOR bytes are already zero and counting
/// from `max - 8` is identical --
/// `tz(av^bv) == 8*(n - (max-8)) + tz(shifted)`.
///
/// # Safety
/// `a[0..max]` and `b[0..max]` readable, `max >= 8`, `n` a multiple of 8 with
/// `n <= max`, and bytes `[0, n)` already proven equal.
#[inline(always)]
unsafe fn finish_words(a: *const u8, b: *const u8, mut n: usize, max: usize) -> usize {
    debug_assert!(max >= 8 && n % 8 == 0 && n <= max);
    // SAFETY: every offset below is `< max`, which the caller made readable.
    unsafe {
        let start = n;
        let cap = max & !7;
        while n < cap {
            if let Some(r) = word_ne(a, b, n) {
                // One thread-local access, not one per word. The subtraction
                // is free in the shipping build -- `eq_op_n` is a no-op there,
                // so the whole expression folds away.
                eq_op_n(1, (n - start) / 8 + 1);
                return r;
            }
            n += 8;
        }
        eq_op_n(1, (n - start) / 8);
        if n == max {
            return max;
        }
        let av = core::ptr::read_unaligned(a.add(max - 8) as *const u64);
        let bv = core::ptr::read_unaligned(b.add(max - 8) as *const u64);
        let x = av ^ bv;
        if x != 0 {
            return (max - 8) + (x.trailing_zeros() as usize) / 8;
        }
        max
    }
}

/// Safe entry: the encoder's, and the tests'. Forwards to the raw twin.
///
/// REFUTED EXPERIMENT, recorded so it is not re-tried: an offset-pair entry
/// (`count_eq_len_at(src, m, ip, max)`) was built to spare `count_match` the
/// `&src[m..m + max]` / `&src[ip..limit]` bounds checks. It measured WORSE --
/// `count_match` went 209 -> 244 static instructions. Two reasons, both
/// visible only in the asm: the two `slice_index_fail` pads did NOT go away
/// (the sub-8 frame-edge branch still needs subslices), and the `assert!` that
/// made the raw call sound in safe code added a THIRD panic path with its own
/// `panic_fmt` setup. `unsafe` cannot move to the call site either -- the
/// crate root is `#![deny(unsafe_code)]` with `mod simd` the only island.
///
/// This is the "bounds-check tax is ~0" law landing again: LLVM had already
/// folded both range checks into six instructions shared by every path.
#[inline(always)]
pub(crate) fn count_eq_len_ge8(a: &[u8], b: &[u8], max: usize) -> usize {
    debug_assert!(max >= 8 && a.len() >= max && b.len() >= max);
    // SAFETY: `max <= a.len()` and `max <= b.len()` are the caller's contract,
    // asserted above in debug.
    unsafe { count_eq_len_ge8_raw(a.as_ptr(), b.as_ptr(), max) }
}

/// The known-length entry: `count_match` has already proven both regions are
/// exactly `max >= 8` long, so the re-min, the zero test, and the sub-8 branch
/// the public wrapper pays are skipped.
///
/// RAW POINTERS. The slice form forced the caller to build `&src[m..m + max]`
/// and `&src[ip..limit]`, and LLVM cannot see that those ranges are already
/// proven: `count_match` emitted an overflow check, two range compares and two
/// `slice_index_fail` landing pads on EVERY call, for bounds the finder
/// established before it ever computed `max`. The proof lives at the call
/// site; this signature stops re-deriving it here.
///
/// # Safety
/// `a[0..max]` and `b[0..max]` must be readable, and `max >= 8`.
#[inline(always)]
pub(crate) unsafe fn count_eq_len_ge8_raw(a: *const u8, b: *const u8, max: usize) -> usize {
    debug_assert!(max >= 8);
    eq_call(max >= 64);
    // Recorded refutations, so nobody re-chases them: (1) `eqlen_arm` is a
    // per-call atomic ONLY under `profile` -- the shipping build folds it to
    // the constant 0 (asm receipt: `has_avx2`'s cache byte is the sole static
    // load in `count_match`). (2) bsf-vs-tzcnt is already optimal: LLVM emits
    // `rep bsf`, the tzcnt encoding, at every `trailing_zeros/ones` here.
    let arm = eqlen_arm();
    if arm == 1 {
        // SAFETY: same contract, forwarded.
        return unsafe { count_eq_len_words_raw(a, b, 0, max) };
    }
    // WORDS TO 32 BEFORE ANY VECTOR. The L19 histogram (eqwork, 511M calls):
    // 50.5% die in word 0 and another 32.6% by byte 31 -- 83% of calls never
    // need a ymm register, its power-up, or the vzeroupper on exit. C's
    // ZSTD_count is word-at-a-time for exactly this distribution. Calls that
    // survive to 32 hand the wide arm the remainder, count-from-verified.
    //
    // SPLIT ON `max >= 32` so the common ladder is STRAIGHT-LINE. The old
    // shared loop computed `cap = if max < 32 { max & !7 } else { 32 }` and
    // LLVM lost the `max >= 8` range fact across that `and`+`cmov`: it emitted
    // a provably-not-taken `testq %r11,%r11 / je` on 100% of calls, plus
    // `cmpl $9` / `cmpl $17` / `cmpq $32` guards between the words. With the
    // branch taken up front, `cap` is the CONSTANT 32 on this side and all
    // four guards are gone.
    // ONE GATE, NOT TWO. The old shape tested `max >= 32` and then `max < 64`.
    // The `eqwidth` counter says 99.956% of calls satisfy BOTH -- `max` is the
    // room left in the BLOCK, not the match length -- so that second compare
    // was paid by essentially every call to serve one in 2300.
    if max >= 64 {
        // The 4-word ladder stays INLINE, deliberately. The returned-length
        // histogram says word 0 decides 50.5% of calls, so half of all calls
        // must be able to answer without a call at all. That is also why the
        // ladder is NOT folded into a `#[target_feature]` twin alongside the
        // vector code: such a function cannot be inlined into a caller that
        // lacks the feature, so that would put a real call in front of the
        // half of calls that never need one.
        //
        // SAFETY: `max >= 64` makes bytes [0, 32) of both regions readable.
        // SAFETY: `max >= 64` makes bytes [0, 32) of both regions readable.
        if let Some(r) = unsafe { ladder32(a, b) } {
            return r;
        }
        // The kernels RESUME AT 32 instead of being handed shifted pointers.
        // `a.add(32)`, `b.add(32)`, `max - 32` and the `+ 32` on the way back
        // were four address/length instructions per call, on ~100% of calls,
        // to express an offset that x86 and aarch64 both fold into the
        // addressing mode for free.
        //
        // SAFETY (every arm): bytes [0, 32) are proven equal above, `a[0..max]`
        // and `b[0..max]` are readable, and `max >= 64` is each kernel's
        // stated contract.
        unsafe {
            #[cfg(all(target_arch = "x86_64", feature = "std"))]
            {
                // EVERY ARM IS A TAIL CALL, including the not-yet-probed one.
                // `has_avx2()` returns a bool, so the probe sat in the MIDDLE
                // of this function as a value-returning call -- which forces
                // `a`, `b` and `max` to survive it, and that is what put
                // `pushq %rsi/%rdi/%rbx` in `count_match`'s prologue on every
                // one of ~71M calls at L19, to serve a branch taken once per
                // process. Branching on the raw cache state instead means
                // nothing is live across anything.
                match AVX2_CACHE.load(core::sync::atomic::Ordering::Relaxed) {
                    1 => return count_eq_len_avx2(a, b, max),
                    2 => return count_eq_len_words_raw(a, b, 32, max),
                    _ => return avx2_first_call(a, b, max),
                }
            }
            #[cfg(all(target_arch = "x86_64", not(feature = "std"), target_feature = "avx2"))]
            {
                // No `std` means no runtime probe: the ISA is proven at compile
                // time and there is nothing to dispatch on.
                return count_eq_len_avx2(a, b, max);
            }
            #[cfg(target_arch = "aarch64")]
            {
                // NEON is baseline aarch64.
                return count_eq_len_neon(a, b, max);
            }
            #[allow(unreachable_code)]
            {
                return count_eq_len_words_raw(a, b, 32, max);
            }
        }
    }
    // 8..=63 -- the other 0.044%. Behind a `#[cold]` call so LLVM sinks the
    // whole arm out of the hot function's straight-line layout.
    // SAFETY: `a[0..max]` and `b[0..max]` are readable and `max >= 8`.
    unsafe { count_eq_len_small(a, b, max) }
}

/// First call on this process: probe, then hand off. `#[cold]` so it never
/// shares a register budget with the hot dispatch above it.
///
/// # Safety
/// Same contract as `count_eq_len_avx2`: `a[0..max]`, `b[0..max]` readable,
/// `max >= 64`, and bytes [0, 32) already proven equal.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[cold]
#[inline(never)]
unsafe fn avx2_first_call(a: *const u8, b: *const u8, max: usize) -> usize {
    // SAFETY: contract forwarded unchanged to whichever arm wins.
    unsafe {
        if avx2_detect() {
            count_eq_len_avx2(a, b, max)
        } else {
            count_eq_len_words_raw(a, b, 32, max)
        }
    }
}

/// The rare arm: `8 <= max < 64`, one call in 2300 (`eqwidth`).
///
/// `#[cold]` is the whole point -- it tells LLVM to move this call and its
/// setup off the hot path's fall-through, which calling the (already
/// `#[inline(never)]`) word twin directly does not.
///
/// # Safety
/// `a[0..max]` and `b[0..max]` readable, and `max >= 8`.
#[cold]
#[inline(never)]
unsafe fn count_eq_len_small(a: *const u8, b: *const u8, max: usize) -> usize {
    // SAFETY: contract forwarded unchanged.
    unsafe { count_eq_len_words_raw(a, b, 0, max) }
}

/// Bucket a returned prefix length. Called by `count_match` so the histogram
/// reflects the lengths the ENCODER actually sees.
///
/// BRANCHLESS. The buckets split at 8/32/64/256, which are bit-length
/// boundaries, so `64 - n.leading_zeros()` indexes a 65-entry table directly
/// and the four-compare `match` ladder disappears. This fires once per
/// `count_match` -- ~500M times at L19 -- and an instrument that branches
/// four times per sample is measuring itself (codec-measurement §6).
#[cfg(feature = "profile")]
#[inline]
pub(crate) fn note_eqlen(n: usize) {
    // bits = 0..=64; bucket = [<8, 8-31, 32-63, 64-255, 256+].
    // bits<=3 -> 0 | 4..=5 -> 1 | 6 -> 2 | 7..=8 -> 3 | >=9 -> 4
    const BUCKET: [u8; 65] = {
        let mut t = [4u8; 65];
        let mut i = 0;
        while i <= 64 {
            t[i] = if i <= 3 {
                0
            } else if i <= 5 {
                1
            } else if i == 6 {
                2
            } else if i <= 8 {
                3
            } else {
                4
            };
            i += 1;
        }
        t
    };
    let bits = (usize::BITS - n.leading_zeros()) as usize;
    let b = BUCKET[bits] as usize;
    let _ = counters::TLS.try_with(|t| {
        let c = &t.hist[b];
        c.set(c.get() + 1);
    });
}

/// AVX2 capability cache: 0 = not yet probed, 1 = present, 2 = absent.
///
/// At module scope, not inside `has_avx2`, so `count_eq_len_ge8_raw` can branch
/// on the raw state and tail-call each arm. See `avx2_first_call`.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
static AVX2_CACHE: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Probe once, memoise, return. `#[cold]` + `#[inline(never)]`: inlined,
/// std_detect's cache probe and its `detect_and_initialize` call were compiled
/// into every dispatch site.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[cold]
#[inline(never)]
fn avx2_detect() -> bool {
    let yes = is_x86_feature_detected!("avx2");
    AVX2_CACHE.store(
        if yes { 1 } else { 2 },
        core::sync::atomic::Ordering::Relaxed,
    );
    yes
}

/// The raw AVX2 cache state: 0 = not yet probed, 1 = present, 2 = absent.
///
/// For callers that want to branch on the state directly instead of on a
/// `bool`. `has_avx2()` returning a value forces the once-per-process probe to
/// be a value-returning call in the middle of the caller, which makes every
/// live value survive it; reading the state lets the caller send the
/// not-yet-probed case somewhere `#[cold]` and keep nothing live.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[inline(always)]
#[allow(dead_code)] // kept with its rationale above; no shipping caller today.
pub(crate) fn avx2_state() -> u8 {
    AVX2_CACHE.load(core::sync::atomic::Ordering::Relaxed)
}

#[inline(always)]
pub(crate) fn has_avx2() -> bool {
    // The detect+store path is OUTLINED AND COLD: inlined, std_detect's cache
    // probe and its `detect_and_initialize` call were compiled into every
    // dispatch site -- count_match carried a 6-register prologue for a
    // once-per-process branch. The hot path is one load and one compare.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        // THE PRESENT CASE FIRST, and tested against its own value rather
        // than against zero. `if v != 0 { return v == 1 }` made LLVM emit
        // `cmpb $1 / je` for the hit AND a `movzbl / testl / je` to separate
        // "absent" from "undetected" -- six instructions inline at every
        // dispatch site. Comparing `1` then `2` lets the hit fall out of one
        // load, one compare and one branch, with the once-per-process
        // `avx2_detect` call reached only from the fall-through.
        match AVX2_CACHE.load(core::sync::atomic::Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => avx2_detect(),
        }
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    {
        cfg!(all(target_arch = "x86_64", target_feature = "avx2"))
    }
}

#[inline(always)]
pub(crate) fn has_bmi2() -> bool {
    // Same cold-outline shape as `has_avx2` -- see the comment there.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        use core::sync::atomic::{AtomicU8, Ordering};
        static C: AtomicU8 = AtomicU8::new(0);
        #[cold]
        #[inline(never)]
        fn detect(c: &AtomicU8) -> bool {
            // LZCNT IS TESTED HERE because the twins this gates ENABLE it:
            // eight `#[target_feature(enable = "bmi2,lzcnt")]` functions hang
            // off this one predicate. Testing only `bmi2` left the guard
            // narrower than the code it admits -- the same shape as the
            // `decode_4x_bmi2` AVX2 hijack, just arrived at by omission rather
            // than by a stray attribute.
            //
            // Every real part that has BMI2 also has LZCNT (Intel introduced
            // both with Haswell; AMD has had LZCNT since Barcelona, long
            // before BMI2), so this costs one extra CPUID bit test, once per
            // process, and rejects nothing that exists. It is tested anyway:
            // an invariant that holds because of a hardware-history argument
            // is one nobody can check, and `scripts/twinguard.py` needed a
            // hand-written exemption to stay quiet about it. Now it does not.
            let yes = is_x86_feature_detected!("bmi2") && is_x86_feature_detected!("lzcnt");
            c.store(if yes { 1 } else { 2 }, Ordering::Relaxed);
            yes
        }
        // THE PRESENT CASE FIRST, and tested against its own value rather
        // than against zero. `if v != 0 { return v == 1 }` made LLVM emit
        // `cmpb $1 / je` for the hit AND a `movzbl / testl / je` to separate
        // "absent" from "undetected" -- six instructions inline at every
        // dispatch site. Comparing `1` then `2` lets the hit fall out of one
        // load, one compare and one branch, with the once-per-process
        // `detect` call reached only from the fall-through.
        match C.load(Ordering::Relaxed) {
            1 => true,
            2 => false,
            _ => detect(&C),
        }
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    {
        false
    }
}

/// C `BIT_lookBitsFast`. Scalar twin is `look_n_bits_shift`.
/// BMI2 `_bextr_u64` is C's path: extract `n` bits at `64-consumed-n`.
/// Named reason auto-vec cannot: variable-width extract, ISA above SSE2 baseline.
#[cfg(test)]
#[inline(always)]
pub(crate) fn look_n_bits(container: u64, consumed: u32, n: u32) -> u32 {
    debug_assert!((1..=56).contains(&n));
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if has_bmi2() {
        // SAFETY: `has_bmi2` is runtime CPUID; start/len computed in the kernel.
        return unsafe { look_n_bits_bmi2(container, consumed, n) };
    }
    look_n_bits_shift(container, consumed, n)
}

/// C `BIT_lookBitsFast` without BMI2: left-shift consumed, right-shift so `n`
/// bits land in the low end. No extra mask — the shift already zeros the rest.
#[inline(always)]
/// C `BIT_lookBitsFast` on a *raw* container + consumed count.
/// BitRev now left-justifies so the hot peek is `container >> (64-n)`; this
/// stays as the formula oracle (`left_justified_look_matches_c_shift`).
// TEST-ONLY. Every caller -- `look_n_bits`, `look_n_bits_bmi2` and the
// `left_justified_look_matches_c_shift` parity test -- is already
// `#[cfg(test)]`, so `allow(dead_code)` was silencing the compiler about a
// function shipped into the binary for nobody. BitRev left-justifies now,
// making the hot peek `container >> (64 - n)`; this stays as the formula
// oracle, in the test build where the oracle is used.
#[cfg(test)]
pub(crate) fn look_n_bits_shift(container: u64, consumed: u32, n: u32) -> u32 {
    debug_assert!((1..=56).contains(&n));
    ((container << (consumed & 63)) >> (64 - n)) as u32
}

#[cfg(all(test, target_arch = "x86_64"))]
#[target_feature(enable = "bmi2")]
pub(crate) fn look_n_bits_bmi2(container: u64, consumed: u32, n: u32) -> u32 {
    if consumed.saturating_add(n) > 64 {
        return look_n_bits_shift(container, consumed, n);
    }
    // SAFETY: BMI2 guaranteed by `#[target_feature]`. `n <= 56` so start+len <= 64
    // when consumed+n <= 64.
    unsafe { core::arch::x86_64::_bextr_u64(container, 64 - consumed - n, n) as u32 }
}

/// Scalar twin / fallback: u64 words then an overlapped tail. Oracle for the
/// SIMD tests. Safe wrapper over `..._raw`.
///
/// NOT in the shipping build. `count_eq_len_ge8_raw` now forwards its
/// non-AVX2 fallback and its `arm == 1` bench arm straight to the raw twin,
/// so the only remaining callers of the slice form are the tests and
/// `bench_eq_words`. The clamp it performs is pure tax on anyone else.
#[cfg(any(test, feature = "profile"))]
#[inline]
pub(crate) fn count_eq_len_words(a: &[u8], b: &[u8], max: usize) -> usize {
    let max = max.min(a.len()).min(b.len());
    if max < 8 {
        // THE ONLY path that can be handed fewer than 8 bytes, so the crate's
        // last per-byte compare loop lives here rather than in the raw twin --
        // which gets a hard `max >= 8` contract in exchange.
        let mut n = 0usize;
        while n < max && a[n] == b[n] {
            n += 1;
        }
        eq_op_n(2, n);
        return n;
    }
    // SAFETY: `max` is clamped to both lengths above and is at least 8.
    unsafe { count_eq_len_words_raw(a.as_ptr(), b.as_ptr(), 0, max) }
}

/// OUTLINED: on AVX2 machines this body is dead code, and inlined it sat in
/// the middle of `count_match`'s per-candidate I-cache footprint.
///
/// RAW POINTERS, NOT SLICES. The old `(&[u8], &[u8], usize)` signature is five
/// Win64 arguments, so `max` travelled on the STACK -- `movq %rsi, 32(%rsp)`
/// at the call site, `movq 40(%rsp), %r10` on entry -- and the body then
/// re-clamped it against both lengths (`cmpq`/`cmovbq`, twice) for a bound
/// every caller had already proven. Five instructions before any work. Three
/// register arguments and no clamp, matching the AVX2 twin's shape.
///
/// # Safety
/// `a[0..max]` and `b[0..max]` must be valid to read.
#[inline(never)]
pub(crate) unsafe fn count_eq_len_words_raw(
    a: *const u8,
    b: *const u8,
    start: usize,
    max: usize,
) -> usize {
    debug_assert!(start % 8 == 0 && start <= max && max >= 8);
    let mut n = start;
    // SAFETY: the caller promised `a[0..max]` and `b[0..max]` are readable;
    // every offset below is `< max`.
    unsafe {
        // FOUR words per iteration, ONE counter bump for them.
        while n + 32 <= max {
            eq_op_n(1, 4);
            let a0 = core::ptr::read_unaligned(a.add(n) as *const u64);
            let b0 = core::ptr::read_unaligned(b.add(n) as *const u64);
            if a0 != b0 {
                return n + ((a0 ^ b0).trailing_zeros() as usize / 8);
            }
            let a1 = core::ptr::read_unaligned(a.add(n + 8) as *const u64);
            let b1 = core::ptr::read_unaligned(b.add(n + 8) as *const u64);
            if a1 != b1 {
                return n + 8 + ((a1 ^ b1).trailing_zeros() as usize / 8);
            }
            let a2 = core::ptr::read_unaligned(a.add(n + 16) as *const u64);
            let b2 = core::ptr::read_unaligned(b.add(n + 16) as *const u64);
            if a2 != b2 {
                return n + 16 + ((a2 ^ b2).trailing_zeros() as usize / 8);
            }
            let a3 = core::ptr::read_unaligned(a.add(n + 24) as *const u64);
            let b3 = core::ptr::read_unaligned(b.add(n + 24) as *const u64);
            if a3 != b3 {
                return n + 24 + ((a3 ^ b3).trailing_zeros() as usize / 8);
            }
            n += 32;
        }
        // `max >= 8` is now a CONTRACT, not a runtime test. The byte loop that
        // used to live here moved to `count_eq_len_words`, the only caller that
        // can be handed fewer than 8 bytes -- and the `other` counter proves
        // the encoder never ran it: it reads 0 at L3/L9/L12/L19. That deletes a
        // compare from every call on this path and the loop from the binary.
        //
        // Words then the overlapped tail, via the shared helper. `n` is a
        // multiple of 8 here and bytes [0, n) are proven equal.
        finish_words(a, b, n, max)
    }
}

/// # Safety
/// `a[0..max]` and `b[0..max]` must be valid to read, and `max >= 32`. The
/// dispatch in `count_eq_len_ge8` enters at `max >= 64` total and hands the
/// kernel `max - 32`, so the contract holds there; `bench_eq_avx2` guards it.
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn count_eq_len_avx2(a: *const u8, b: *const u8, max: usize) -> usize {
    use core::arch::x86_64::{
        __m256i, _mm256_and_si256, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
    };
    debug_assert!(max >= 64);
    // RESUME AT 32. The dispatch has already proven bytes [0, 32) equal
    // with its word ladder, so it hands over the ORIGINAL pointers and
    // length instead of `a.add(32)` / `b.add(32)` / `max - 32` -- and
    // takes the result unshifted. Those four instructions were paid on
    // ~100% of calls to express an offset the addressing mode encodes for
    // free (`32(%rcx,%r9)`).
    let mut n = 32usize;
    let mut wide = 0usize;
    // SAFETY: caller promised `a[0..max]` and `b[0..max]` are valid.
    unsafe {
        while n + 64 <= max {
            wide += 2;
            let e0 = _mm256_cmpeq_epi8(
                _mm256_loadu_si256(a.add(n) as *const __m256i),
                _mm256_loadu_si256(b.add(n) as *const __m256i),
            );
            let e1 = _mm256_cmpeq_epi8(
                _mm256_loadu_si256(a.add(n + 32) as *const __m256i),
                _mm256_loadu_si256(b.add(n + 32) as *const __m256i),
            );
            // ONE movemask and ONE branch for 64 bytes. `and(e0,e1)` is
            // all-ones iff BOTH halves are. LLVM lowers this pair to
            // `vpxor/vpxor/vpor/vptest` -- it never materialises a mask on the
            // all-equal path at all, which is better than what was written.
            if _mm256_movemask_epi8(_mm256_and_si256(e0, e1)) as u32 != 0xFFFF_FFFF {
                eq_op_n(0, wide);
                let m0 = _mm256_movemask_epi8(e0) as u32;
                if m0 != 0xFFFF_FFFF {
                    return n + m0.trailing_ones() as usize;
                }
                let m1 = _mm256_movemask_epi8(e1) as u32;
                return n + 32 + m1.trailing_ones() as usize;
            }
            n += 64;
        }
        // AT MOST ONE 32-byte block can remain -- the loop above exited with
        // `max - n < 64`. `if`, not `while`.
        if n + 32 <= max {
            wide += 1;
            let av = _mm256_loadu_si256(a.add(n) as *const __m256i);
            let bv = _mm256_loadu_si256(b.add(n) as *const __m256i);
            let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(av, bv)) as u32;
            if mask != 0xFFFF_FFFF {
                eq_op_n(0, wide);
                return n + mask.trailing_ones() as usize;
            }
            n += 32;
        }
        // THE TAIL IS A VECTOR COMPARE, NOT A WORD LOOP.
        //
        // This kernel used to finish through `finish_words`: up to three
        // 8-byte compares and then an overlapped u64 -- about twenty scalar
        // instructions sitting inside an AVX2 function, to cover fewer than 32
        // bytes. It is the same overlapped trick one level wider: `max >= 64`
        // makes bytes [max-32, max) readable, `max - n < 32` puts `max - 32`
        // BELOW `n`, and bytes [max-32, n) are already proven equal -- so their
        // mask bits are ones, `trailing_ones` walks straight past them, and
        // counting from `max - 32` lands on the first real difference.
        if n < max {
            wide += 1;
            let s = max - 32;
            let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(
                _mm256_loadu_si256(a.add(s) as *const __m256i),
                _mm256_loadu_si256(b.add(s) as *const __m256i),
            )) as u32;
            if mask != 0xFFFF_FFFF {
                eq_op_n(0, wide);
                return s + mask.trailing_ones() as usize;
            }
        }
        eq_op_n(0, wide);
    }
    max
}

/// aarch64 has no `pmovmskb`. `vshrn_n_u16::<4>` narrows each 16-bit lane -- a
/// PAIR of byte masks -- to 8 bits by taking bits 4..12, which leaves exactly
/// one NIBBLE per input byte: nibble `i` of the result is byte `i`'s mask. So
/// ONE 64-bit word describes all 16 bytes, and the first differing byte is
/// `trailing_ones() >> 2`.
///
/// The old form extracted TWO u64 lanes (`vgetq_lane_u64` x2) and branched
/// twice per 16 bytes, because a raw byte-mask vector needs 128 bits.
///
/// # Safety
/// NEON only; guaranteed by `#[target_feature]`.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[inline]
unsafe fn neon_nib_mask(eq: core::arch::aarch64::uint8x16_t) -> u64 {
    use core::arch::aarch64::{
        vget_lane_u64, vreinterpret_u64_u8, vreinterpretq_u16_u8, vshrn_n_u16,
    };
    // SAFETY: pure register shuffles on a value the caller supplied.
    unsafe {
        vget_lane_u64::<0>(vreinterpret_u64_u8(vshrn_n_u16::<4>(vreinterpretq_u16_u8(
            eq,
        ))))
    }
}

/// # Safety
/// `a[0..max]` and `b[0..max]` must be valid to read, and `max >= 32` -- the
/// same contract as the AVX2 twin, from the same `max >= 64` dispatch gate.
#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn count_eq_len_neon(a: *const u8, b: *const u8, max: usize) -> usize {
    use core::arch::aarch64::{vandq_u8, vceqq_u8, vld1q_u8};
    debug_assert!(max >= 64);
    // RESUME AT 32, exactly as the AVX2 twin does -- see its comment.
    let mut n = 32usize;
    // SAFETY: caller promised `a[0..max]` and `b[0..max]` are valid.
    unsafe {
        while n + 32 <= max {
            let e0 = vceqq_u8(vld1q_u8(a.add(n)), vld1q_u8(b.add(n)));
            let e1 = vceqq_u8(vld1q_u8(a.add(n + 16)), vld1q_u8(b.add(n + 16)));
            // One mask, one branch for 32 bytes -- mirroring the AVX2 twin's
            // `vpand` fusion. This twin had no unroll at all.
            if neon_nib_mask(vandq_u8(e0, e1)) != u64::MAX {
                let m0 = neon_nib_mask(e0);
                if m0 != u64::MAX {
                    return n + (m0.trailing_ones() as usize >> 2);
                }
                return n + 16 + (neon_nib_mask(e1).trailing_ones() as usize >> 2);
            }
            n += 32;
        }
        // At most one 16-byte block remains.
        if n + 16 <= max {
            let m = neon_nib_mask(vceqq_u8(vld1q_u8(a.add(n)), vld1q_u8(b.add(n))));
            if m != u64::MAX {
                return n + (m.trailing_ones() as usize >> 2);
            }
            n += 16;
        }
        // THE TAIL IS A VECTOR COMPARE, mirroring the AVX2 twin exactly.
        //
        // `max - n < 16` here, so `max - 16` sits at or below `n`, and bytes
        // [max-16, n) are already proven equal -- their nibbles are ones, so
        // `trailing_ones >> 2` walks past them and counting from `max - 16`
        // gives the first real difference. That replaces the word loop and the
        // overlapped u64 this twin used to reach through `finish_words`.
        if n < max {
            let s = max - 16;
            let m = neon_nib_mask(vceqq_u8(vld1q_u8(a.add(s)), vld1q_u8(b.add(s))));
            if m != u64::MAX {
                return s + (m.trailing_ones() as usize >> 2);
            }
        }
    }
    max
}

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec::Vec;

    fn bytes(a: &[u8], b: &[u8]) -> usize {
        let max = a.len().min(b.len());
        let mut n = 0usize;
        while n < max && a[n] == b[n] {
            n += 1;
        }
        n
    }

    #[test]
    fn load_u32_u64_le_matches_from_le_bytes() {
        let mut src = vec![0u8; 64];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i.wrapping_mul(37) + 11) as u8;
        }
        for i in 0..=src.len() - 4 {
            let want = u32::from_le_bytes(src[i..i + 4].try_into().unwrap());
            assert_eq!(load_u32_le(&src, i), want, "u32 i={i}");
        }
        for i in 0..=src.len() - 8 {
            let want = u64::from_le_bytes(src[i..i + 8].try_into().unwrap());
            assert_eq!(load_u64_le(&src, i), want, "u64 i={i}");
        }
    }

    #[test]
    fn count_eq_len_matches_byte_and_words() {
        let mut src = vec![0u8; 4096];
        for (i, b) in src.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let head: Vec<u8> = src[0..200].to_vec();
        src[200..400].copy_from_slice(&head);
        let mid: Vec<u8> = src[3..20].to_vec();
        src[800..817].copy_from_slice(&mid);
        for m in [0usize, 1, 3, 7, 8, 15, 200] {
            for ip in [200usize, 201, 400, 800, 801, 2000] {
                if m >= src.len() || ip >= src.len() {
                    continue;
                }
                for n in [0usize, 1, 7, 8, 9, 31, 32, 33, 64, 200, 512] {
                    let end_a = (m + n).min(src.len());
                    let end_b = (ip + n).min(src.len());
                    let a = &src[m..end_a];
                    let b = &src[ip..end_b];
                    let max = a.len().min(b.len());
                    let want = bytes(a, b);
                    assert_eq!(
                        count_eq_len_words(a, b, max),
                        want,
                        "words m={m} ip={ip} n={n}"
                    );
                    assert_eq!(count_eq_len(a, b), want, "dispatch m={m} ip={ip} n={n}");
                    #[cfg(all(target_arch = "x86_64", feature = "std"))]
                    if max >= 64 && is_x86_feature_detected!("avx2") {
                        // `max >= 64` is the kernel's contract; below it the
                        // dispatch never calls it. See `eq_oracle_exhaustive`
                        // for the tail coverage this loop is too coarse for.
                        let got = unsafe { avx2_with_ladder(a.as_ptr(), b.as_ptr(), max) };
                        assert_eq!(got, want, "avx2 m={m} ip={ip} n={n}");
                    }
                }
            }
        }
        let long_a = vec![0xA5u8; 10_000];
        let mut long_b = long_a.clone();
        long_b[9999] = 0x5A;
        assert_eq!(count_eq_len(&long_a, &long_b), 9999);
        assert_eq!(count_eq_len(&long_a, &long_a), 10_000);
    }

    /// `note_eqlen`'s branchless bucket must agree with the compare ladder it
    /// replaced, at every boundary and beyond. A histogram that silently
    /// mis-buckets is worse than no histogram -- it reads like data.
    #[test]
    fn eqlen_bucket_matches_compare_ladder() {
        fn ladder(n: usize) -> usize {
            match n {
                0..=7 => 0,
                8..=31 => 1,
                32..=63 => 2,
                64..=255 => 3,
                _ => 4,
            }
        }
        // The same LUT `note_eqlen` uses, evaluated the same way.
        fn lut(n: usize) -> usize {
            let bits = (usize::BITS - n.leading_zeros()) as usize;
            (if bits <= 3 {
                0
            } else if bits <= 5 {
                1
            } else if bits == 6 {
                2
            } else if bits <= 8 {
                3
            } else {
                4
            }) as usize
        }
        for n in 0usize..=2048 {
            assert_eq!(lut(n), ladder(n), "n={n}");
        }
        for n in [4096usize, 65_535, 65_536, 1 << 20, usize::MAX] {
            assert_eq!(lut(n), ladder(n), "n={n}");
        }
    }

    /// THE GATE for the ten-cut pass. Every implementation against the byte
    /// oracle for EVERY `(max, first-differing-byte)` pair up to 200.
    ///
    /// The coarse loop above hits almost none of what changed: it samples a
    /// handful of lengths, so `max % 8` -- which is exactly what the overlapped
    /// tails index off -- is 0 or 1 nearly everywhere. This walks every tail
    /// alignment, both dispatch edges (`max >= 32`, `max >= 64`), and, for
    /// `max >= 96`, the AVX2 64-byte `vpand` loop's mismatch-resolution path in
    /// both halves.
    #[test]
    fn eq_oracle_exhaustive() {
        // A pattern with no self-similarity, so a planted flip is the ONLY
        // difference and the expected answer is exactly its index.
        let mut a = vec![0u8; 260];
        for (i, v) in a.iter_mut().enumerate() {
            *v = (i.wrapping_mul(97).wrapping_add(13) % 251) as u8;
        }
        for max in 8usize..=200 {
            for p in 0..=max {
                let mut b = a.clone();
                if p < max {
                    b[p] ^= 0xFF;
                }
                let (sa, sb) = (&a[..max], &b[..max]);
                let want = p;
                assert_eq!(bytes(sa, sb), want, "oracle max={max} p={p}");
                assert_eq!(count_eq_len_ge8(sa, sb, max), want, "ge8 max={max} p={p}");
                assert_eq!(
                    count_eq_len_words(sa, sb, max),
                    want,
                    "words max={max} p={p}"
                );
                assert_eq!(count_eq_len(sa, sb), want, "wrapper max={max} p={p}");
                #[cfg(all(target_arch = "x86_64", feature = "std"))]
                if max >= 64 && is_x86_feature_detected!("avx2") {
                    // SAFETY: both slices are exactly `max` long, `max >= 64`.
                    let got = unsafe { avx2_with_ladder(sa.as_ptr(), sb.as_ptr(), max) };
                    assert_eq!(got, want, "avx2 max={max} p={p}");
                }
            }
        }
    }

    #[test]
    fn look_n_bits_bmi2_matches_shift() {
        let containers = [
            0u64,
            1,
            u64::MAX,
            0x0123_4567_89AB_CDEF,
            0x8000_0000_0000_0001,
            0x00FF_00FF_00FF_00FF,
        ];
        for c in containers {
            for consumed in 0u32..=56 {
                for n in 1u32..=24 {
                    if n > 56 {
                        continue;
                    }
                    let shift = look_n_bits_shift(c, consumed, n);
                    let got = look_n_bits(c, consumed, n);
                    assert_eq!(
                        got, shift,
                        "c={c:#x} consumed={consumed} n={n} got={got:#x} shift={shift:#x}"
                    );
                    #[cfg(all(target_arch = "x86_64", feature = "std"))]
                    if is_x86_feature_detected!("bmi2") {
                        let b = unsafe { look_n_bits_bmi2(c, consumed, n) };
                        assert_eq!(b, shift, "bmi2 c={c:#x} consumed={consumed} n={n}");
                    }
                }
            }
        }
    }
}
