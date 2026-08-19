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
/// primitive is correct.
#[allow(dead_code)]
///
/// A pure HINT: it cannot fault, cannot change any value, and an out-of-range
/// `at` simply does nothing. So any code path using it is byte-identical by
/// construction -- no oracle needed, only a benchmark.
///
/// The match copy in `decode_sequences` reads from a random earlier offset,
/// which is the decoder's one unpredictable load. C ships a whole separate
/// path for this (`ZSTD_decompressSequencesLong` + `ZSTD_DECODESEQUENCE_PREFETCH`).
#[inline(always)]
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
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: as above; `prefetch` is a hint instruction on aarch64 too.
        unsafe {
            core::arch::aarch64::_prefetch(
                slice.as_ptr().add(at) as *const i8,
                core::arch::aarch64::_PREFETCH_READ,
                core::arch::aarch64::_PREFETCH_LOCALITY3,
            );
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        let _ = at;
    }
}

/// Unaligned little-endian `u32`. Caller: `i + 4 <= src.len()`.
#[inline(always)]
pub(crate) fn load_u32_le(src: &[u8], i: usize) -> u32 {
    debug_assert!(i + 4 <= src.len());
    // SAFETY: caller proves `i + 4 <= src.len()`. Unaligned [u8; 4] then LE
    // integer — not a native-endian `u32` load (wrong on BE).
    let arr = unsafe { src.as_ptr().add(i).cast::<[u8; 4]>().read_unaligned() };
    u32::from_le_bytes(arr)
}

/// Unaligned little-endian `u64`. Caller: `i + 8 <= src.len()`.
#[inline(always)]
pub(crate) fn load_u64_le(src: &[u8], i: usize) -> u64 {
    debug_assert!(i + 8 <= src.len());
    // SAFETY: caller proves `i + 8 <= src.len()`.
    let arr = unsafe { src.as_ptr().add(i).cast::<[u8; 8]>().read_unaligned() };
    u64::from_le_bytes(arr)
}

/// Common prefix length of `a` and `b` (min of the two lengths).
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
#[cfg(feature = "profile")]
pub static EQ_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static EQ_WIDE_ELIGIBLE: core::sync::atomic::AtomicU64 =
    core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static EQ_LEN_HIST: [core::sync::atomic::AtomicU64; 5] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Read and clear `(calls, wide_eligible, [<8, 8-31, 32-63, 64-255, 256+])`.
#[cfg(feature = "profile")]
pub fn take_eqlen_stats() -> (u64, u64, [u64; 5]) {
    use core::sync::atomic::Ordering::Relaxed;
    let mut h = [0u64; 5];
    for (i, v) in EQ_LEN_HIST.iter().enumerate() {
        h[i] = v.swap(0, Relaxed);
    }
    (EQ_CALLS.swap(0, Relaxed), EQ_WIDE_ELIGIBLE.swap(0, Relaxed), h)
}

pub(crate) fn count_eq_len(a: &[u8], b: &[u8]) -> usize {
    let max = a.len().min(b.len());
    if max == 0 {
        return 0;
    }
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        EQ_CALLS.fetch_add(1, Relaxed);
        if max >= 64 {
            EQ_WIDE_ELIGIBLE.fetch_add(1, Relaxed);
        }
    }
    let arm = eqlen_arm();
    if arm == 1 {
        return count_eq_len_words(a, b, max);
    }
    if arm == 2 && max >= 8 {
        // Peek one word before committing to a 64-byte read.
        let av = load_u64(a, 0);
        let bv = load_u64(b, 0);
        if av != bv {
            return ((av ^ bv).trailing_zeros() as usize) / 8;
        }
    }
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        if arm != 1 && has_avx2() {
            // SAFETY: `a[..max]` and `b[..max]` are in-bounds.
            return unsafe { count_eq_len_avx2(a.as_ptr(), b.as_ptr(), max) };
        }
    }
    #[cfg(all(target_arch = "x86_64", not(feature = "std"), target_feature = "avx2"))]
    {
        return unsafe { count_eq_len_avx2(a.as_ptr(), b.as_ptr(), max) };
    }
    #[cfg(target_arch = "aarch64")]
    {
        // SAFETY: `a[..max]` and `b[..max]` are in-bounds. NEON is baseline aarch64.
        return unsafe { count_eq_len_neon(a.as_ptr(), b.as_ptr(), max) };
    }
    #[allow(unreachable_code)]
    count_eq_len_words(a, b, max)
}

/// Bucket a returned prefix length. Called by `count_match` so the histogram
/// reflects the lengths the ENCODER actually sees.
#[cfg(feature = "profile")]
#[inline]
pub(crate) fn note_eqlen(n: usize) {
    let b = match n {
        0..=7 => 0,
        8..=31 => 1,
        32..=63 => 2,
        64..=255 => 3,
        _ => 4,
    };
    EQ_LEN_HIST[b].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}

#[inline(always)]
fn has_avx2() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        use core::sync::atomic::{AtomicU8, Ordering};
        static C: AtomicU8 = AtomicU8::new(0);
        let v = C.load(Ordering::Relaxed);
        if v != 0 {
            return v == 1;
        }
        let yes = is_x86_feature_detected!("avx2");
        C.store(if yes { 1 } else { 2 }, Ordering::Relaxed);
        yes
    }
    #[cfg(not(all(target_arch = "x86_64", feature = "std")))]
    {
        cfg!(all(target_arch = "x86_64", target_feature = "avx2"))
    }
}

#[cfg(test)]
#[inline(always)]
pub(crate) fn has_bmi2() -> bool {
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    {
        use core::sync::atomic::{AtomicU8, Ordering};
        static C: AtomicU8 = AtomicU8::new(0);
        let v = C.load(Ordering::Relaxed);
        if v != 0 {
            return v == 1;
        }
        let yes = is_x86_feature_detected!("bmi2");
        C.store(if yes { 1 } else { 2 }, Ordering::Relaxed);
        yes
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
    debug_assert!(n >= 1 && n <= 56);
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
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) fn look_n_bits_shift(container: u64, consumed: u32, n: u32) -> u32 {
    debug_assert!(n >= 1 && n <= 56);
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

/// Scalar twin / fallback: u64 words then a byte tail. Oracle for SIMD tests.
pub(crate) fn count_eq_len_words(a: &[u8], b: &[u8], max: usize) -> usize {
    let max = max.min(a.len()).min(b.len());
    let mut n = 0usize;
    while n + 32 <= max {
        let a0 = load_u64(a, n);
        let b0 = load_u64(b, n);
        if a0 != b0 {
            return n + ((a0 ^ b0).trailing_zeros() as usize / 8);
        }
        let a1 = load_u64(a, n + 8);
        let b1 = load_u64(b, n + 8);
        if a1 != b1 {
            return n + 8 + ((a1 ^ b1).trailing_zeros() as usize / 8);
        }
        let a2 = load_u64(a, n + 16);
        let b2 = load_u64(b, n + 16);
        if a2 != b2 {
            return n + 16 + ((a2 ^ b2).trailing_zeros() as usize / 8);
        }
        let a3 = load_u64(a, n + 24);
        let b3 = load_u64(b, n + 24);
        if a3 != b3 {
            return n + 24 + ((a3 ^ b3).trailing_zeros() as usize / 8);
        }
        n += 32;
    }
    while n + 8 <= max {
        let av = load_u64(a, n);
        let bv = load_u64(b, n);
        if av != bv {
            return n + ((av ^ bv).trailing_zeros() as usize / 8);
        }
        n += 8;
    }
    while n < max && a[n] == b[n] {
        n += 1;
    }
    n
}

#[inline(always)]
fn load_u64(s: &[u8], i: usize) -> u64 {
    load_u64_le(s, i)
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
unsafe fn count_eq_len_avx2(a: *const u8, b: *const u8, max: usize) -> usize {
    use core::arch::x86_64::{
        __m256i, _mm256_cmpeq_epi8, _mm256_loadu_si256, _mm256_movemask_epi8,
    };
    let mut n = 0usize;
    // SAFETY: caller promised `a[0..max]` and `b[0..max]` are valid.
    unsafe {
        while n + 64 <= max {
            let a0 = _mm256_loadu_si256(a.add(n) as *const __m256i);
            let b0 = _mm256_loadu_si256(b.add(n) as *const __m256i);
            let m0 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(a0, b0)) as u32;
            if m0 != 0xFFFF_FFFF {
                return n + m0.trailing_ones() as usize;
            }
            let a1 = _mm256_loadu_si256(a.add(n + 32) as *const __m256i);
            let b1 = _mm256_loadu_si256(b.add(n + 32) as *const __m256i);
            let m1 = _mm256_movemask_epi8(_mm256_cmpeq_epi8(a1, b1)) as u32;
            if m1 != 0xFFFF_FFFF {
                return n + 32 + m1.trailing_ones() as usize;
            }
            n += 64;
        }
        while n + 32 <= max {
            let av = _mm256_loadu_si256(a.add(n) as *const __m256i);
            let bv = _mm256_loadu_si256(b.add(n) as *const __m256i);
            let mask = _mm256_movemask_epi8(_mm256_cmpeq_epi8(av, bv)) as u32;
            if mask != 0xFFFF_FFFF {
                return n + mask.trailing_ones() as usize;
            }
            n += 32;
        }
        while n + 8 <= max {
            let av = core::ptr::read_unaligned(a.add(n) as *const u64);
            let bv = core::ptr::read_unaligned(b.add(n) as *const u64);
            if av != bv {
                return n + ((av ^ bv).trailing_zeros() as usize / 8);
            }
            n += 8;
        }
        while n < max && *a.add(n) == *b.add(n) {
            n += 1;
        }
    }
    n
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
unsafe fn count_eq_len_neon(a: *const u8, b: *const u8, max: usize) -> usize {
    use core::arch::aarch64::{
        uint8x16_t, vceqq_u8, vgetq_lane_u64, vld1q_u8, vreinterpretq_u64_u8,
    };
    let mut n = 0usize;
    // SAFETY: caller promised `a[0..max]` and `b[0..max]` are valid.
    unsafe {
        while n + 16 <= max {
            let eq: uint8x16_t = vceqq_u8(vld1q_u8(a.add(n)), vld1q_u8(b.add(n)));
            let lo = vgetq_lane_u64(vreinterpretq_u64_u8(eq), 0);
            let hi = vgetq_lane_u64(vreinterpretq_u64_u8(eq), 1);
            if lo != u64::MAX {
                return n + (lo.trailing_ones() as usize / 8);
            }
            if hi != u64::MAX {
                return n + 8 + (hi.trailing_ones() as usize / 8);
            }
            n += 16;
        }
        while n + 8 <= max {
            let av = core::ptr::read_unaligned(a.add(n) as *const u64);
            let bv = core::ptr::read_unaligned(b.add(n) as *const u64);
            if av != bv {
                return n + ((av ^ bv).trailing_zeros() as usize / 8);
            }
            n += 8;
        }
        while n < max && *a.add(n) == *b.add(n) {
            n += 1;
        }
    }
    n
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
                    if is_x86_feature_detected!("avx2") {
                        let got = unsafe { count_eq_len_avx2(a.as_ptr(), b.as_ptr(), max) };
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
