//! XXH64 (seed 0), used for the zstd content checksum (low 32 bits).
//!
//! Spec: <https://github.com/Cyan4973/xxHash/blob/v0.8.2/doc/xxhash_spec.md>
//! Pure Rust, no_std, no alloc.
//!
//! Primes are from that spec. Do not "correct" P2 against other writeups --
//! `0xC2B2AE3D27D4EB4F` is required for XXH64("", 0) == `0xEF46DB3751D8E999`.

const P1: u64 = 0x9E37_79B1_85EB_CA87;
const P2: u64 = 0xC2B2_AE3D_27D4_EB4F;
const P3: u64 = 0x1656_67B1_9E37_79F9;
const P4: u64 = 0x85EB_CA77_C2B2_AE63;
const P5: u64 = 0x27D4_EB2F_1656_67C5;

#[inline(always)]
fn round(acc: u64, input: u64) -> u64 {
    acc.wrapping_add(input.wrapping_mul(P2))
        .rotate_left(31)
        .wrapping_mul(P1)
}

#[inline(always)]
fn merge(acc: u64, val: u64) -> u64 {
    (acc ^ round(0, val)).wrapping_mul(P1).wrapping_add(P4)
}

#[inline(always)]
fn read_u64_at(s: &[u8], i: usize) -> u64 {
    let mut buf = [0u8; 8];
    buf.copy_from_slice(&s[i..i + 8]);
    u64::from_le_bytes(buf)
}

#[inline(always)]
fn read_u32_at(s: &[u8], i: usize) -> u32 {
    let mut buf = [0u8; 4];
    buf.copy_from_slice(&s[i..i + 4]);
    u32::from_le_bytes(buf)
}

#[inline(always)]
fn stripe(v: &mut [u64; 4], src: &[u8], off: usize) {
    v[0] = round(v[0], read_u64_at(src, off));
    v[1] = round(v[1], read_u64_at(src, off + 8));
    v[2] = round(v[2], read_u64_at(src, off + 16));
    v[3] = round(v[3], read_u64_at(src, off + 24));
}

/// HYBRID: AVX2 for the multiply that is OFF the critical path, scalar for the
/// chain. This is the design the all-vector attempt should have been.
///
/// The all-ymm version (4 accumulators packed into one register) measured
/// **0.60x** -- 40% SLOWER. Counting multiply-port pressure was the wrong model:
/// packing v1..v4 into one register collapses FOUR independent dependency chains
/// into ONE, and the emulated 64x64 multiply's critical path is
/// `srl(1) + vpmuludq(5) + add(1) + sll(1) + add(1)` = **9 cycles** against a
/// scalar `imul`'s 3. Scalar was never latency-bound; it was extracting exactly
/// the 4-way ILP the algorithm offers.
///
/// So vectorise only the half that carries no dependency. Per round,
/// `input * P2` depends solely on loaded bytes -- its latency is free, only its
/// throughput costs. Precompute it 128 bytes at a time with `vpmuludq` (port 0),
/// leaving the scalar loop just `add`, `rotl` and `* P1` (port 1) on the four
/// live chains:
///
/// ```text
///   scalar today   8 imul/stripe on p1                  -> 8 cycles
///   hybrid         3 vpmuludq on p0 || 4 imul on p1     -> 4 cycles
/// ```
/// TILE size for the pre-multiply. The vector half must live in its own
/// `#[target_feature]` function and the accumulator half must NOT -- with AVX2
/// enabled over the whole loop, LLVM auto-vectorises the four scalar chains back
/// into one vector chain and the win collapses from 1.23x to 1.06x.
///
/// That leaves a real `call` per tile (a target_feature fn cannot inline into a
/// baseline caller, 4.56), so the tile is sized to amortise it. Swept:
/// 128B 1.05-1.32x, **256B 1.14-1.26x**, 512B 0.98-1.15x, 1KiB 0.94-1.14x,
/// 4KiB 0.80-1.12x. Bigger tiles amortise the call but the intermediate buffer
/// costs more L1 traffic than the call saves; 256 B is the knee and is the only
/// value that never reads below 1.14x.
const PRE_TILE: usize = 256;

/// `out[i] = src[i*8..][..8] as u64le * P2`, for one tile.
///
/// Every product here is independent of the accumulators, so its LATENCY is free
/// and only its throughput is charged -- on port 0, beside the scalar `imul`s on
/// port 1. `vpmuludq` already multiplies the LOW 32 bits of each 64-bit lane, so
/// `alo` needs no mask; only `ahi` costs a shift:
///
/// ```text
///   a*P2 = alo*P2lo + ((alo*P2hi + ahi*P2lo) << 32)      (mod 2^64)
/// ```
#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unsafe_code)]
unsafe fn premul_p2_avx2(
    src: &[u8; PRE_TILE],
    out: &mut [core::mem::MaybeUninit<u64>; PRE_TILE / 8],
) {
    use core::arch::x86_64::*;
    unsafe {
        let p2lo = _mm256_set1_epi64x((P2 & 0xFFFF_FFFF) as i64);
        let p2hi = _mm256_set1_epi64x((P2 >> 32) as i64);
        let n = PRE_TILE / 32;
        let sp = src.as_ptr();
        let op = out.as_mut_ptr().cast::<u64>();
        let mut i = 0usize;
        while i < n {
            let a = _mm256_loadu_si256(sp.add(i * 32) as *const __m256i);
            let t0 = _mm256_mul_epu32(a, p2lo);
            let t1 = _mm256_mul_epu32(a, p2hi);
            let ah = _mm256_srli_epi64(a, 32);
            let t2 = _mm256_mul_epu32(ah, p2lo);
            let cross = _mm256_slli_epi64(_mm256_add_epi64(t1, t2), 32);
            _mm256_storeu_si256(op.add(i * 4) as *mut __m256i, _mm256_add_epi64(t0, cross));
            i += 1;
        }
    }
}

/// `out[i] = src[i*8..][..8] as u64le * P2`, for one tile -- aarch64 twin.
///
/// D8b. A direct lane-for-lane translation of [`premul_p2_avx2`]: same
/// decomposition, 2 lanes per vector instead of 4.
///
/// ```text
///   a*P2 = alo*P2lo + ((alo*P2hi + ahi*P2lo) << 32)      (mod 2^64)
/// ```
///
/// The intrinsic correspondence is exact:
///
/// | x86 AVX2                | aarch64 NEON                     |
/// | ----------------------- | -------------------------------- |
/// | `_mm256_mul_epu32(a,b)` | `vmull_u32(alo(a), alo(b))`      |
/// | `_mm256_srli_epi64`     | `vshrn_n_u64` (narrows AND shifts) |
/// | `_mm256_add_epi64`      | `vaddq_u64`                      |
/// | `_mm256_slli_epi64`     | `vshlq_n_u64`                    |
///
/// `vpmuludq` reads the LOW 32 bits of each 64-bit lane implicitly; NEON's
/// `vmull_u32` instead takes `uint32x2_t`, so the low halves are extracted
/// explicitly with `vmovn_u64` and the high halves with `vshrn_n_u64` -- which
/// is why this needs one narrow the x86 form does not, and no mask either way.
///
/// # Safety
/// NEON is baseline on ARMv8-A, so this is always callable on aarch64; it is
/// `unsafe` only because `core::arch` intrinsics are.
///
/// **NOT VERIFIED ON HARDWARE.** No aarch64 machine or emulator was available
/// when this was written. What IS gated: the decomposition is proven
/// exhaustively against `wrapping_mul` by `premul_decomposition_matches_mul`
/// (which runs on any host), and the emitted aarch64 assembly was inspected for
/// the expected `umull`/`shrn`/`shl`/`add` sequence. What is NOT gated: that it
/// runs correctly on a real core, and any speed claim at all. `PRE_TILE` is 256
/// because that is the knee of an x86 sweep against an x86 call cost -- on
/// aarch64 the kernel needs no call at all (NEON is baseline, so it inlines),
/// so the tile size is very likely wrong here and MUST be re-swept. Run
/// `xxhdiff` and `xxhgold` on the target before trusting this path.
#[cfg(target_arch = "aarch64")]
#[allow(unsafe_code)]
unsafe fn premul_p2_neon(
    src: &[u8; PRE_TILE],
    out: &mut [core::mem::MaybeUninit<u64>; PRE_TILE / 8],
) {
    use core::arch::aarch64::*;
    unsafe {
        let p2lo = vdup_n_u32((P2 & 0xFFFF_FFFF) as u32);
        let p2hi = vdup_n_u32((P2 >> 32) as u32);
        let sp = src.as_ptr();
        let op = out.as_mut_ptr().cast::<u64>();
        // Two u64 lanes per q register; 16 bytes of source per step.
        let n = PRE_TILE / 16;
        let mut i = 0usize;
        while i < n {
            let a = vld1q_u64(sp.add(i * 16).cast::<u64>());
            let alo = vmovn_u64(a); // low 32 of each lane
            let ahi = vshrn_n_u64(a, 32); // high 32 of each lane
            let t0 = vmull_u32(alo, p2lo);
            let t1 = vmull_u32(alo, p2hi);
            let t2 = vmull_u32(ahi, p2lo);
            let cross = vshlq_n_u64(vaddq_u64(t1, t2), 32);
            vst1q_u64(op.add(i * 2), vaddq_u64(t0, cross));
            i += 1;
        }
    }
}

/// Scalar accumulator step over PRE-MULTIPLIED input: the `* P2` is already done.
#[inline(always)]
fn round_pre(acc: u64, pre: u64) -> u64 {
    acc.wrapping_add(pre).rotate_left(31).wrapping_mul(P1)
}

/// The ARCH-INDEPENDENT half of the hybrid: walk whole `PRE_TILE` tiles, and run
/// the four scalar accumulator chains over the pre-multiplied words.
///
/// `premul` is the ONLY arch-specific part. Factored out for D8b: adding the
/// NEON twin as a second `#[cfg]` branch would have hand-copied the tile walk,
/// the `MaybeUninit` discipline, the accumulator unroll and the census -- the
/// exact twin BRICK 10 deleted from the scalar path. One walk, two kernels.
///
/// **The kernel must stay a SEPARATE function compiled for its own ISA.** With
/// the vector feature enabled over this loop as well, LLVM folds the four
/// independent scalar chains back into one vector chain and the win collapses
/// (measured on x86: 1.23x -> 1.06x). That is also why the all-vector attempt
/// measured 0.60x: four independent dependency chains are the whole point, and
/// the emulated 64x64 multiply's critical path is 9 cycles against `imul`'s 3.
#[inline(always)]
fn stripes_pre<F>(input: &[u8], v: &mut [u64; 4], mut premul: F) -> usize
where
    F: FnMut(&[u8; PRE_TILE], &mut [core::mem::MaybeUninit<u64>; PRE_TILE / 8]),
{
    // WHOLE TILES ONLY (BRICK 2). Consuming a partial final tile made `take` a
    // runtime value, and that one fact cost, per tile: a `min` cmov, a
    // slice-range bounds check with a `slice_index_fail` panic block, and --
    // because `words` was unknown -- an 8-way unrolled consume loop with a
    // `cmp`+`jb` guard before EVERY group of four rounds. Sixteen guard
    // instructions per 256 bytes, to handle a case that occurs at most once per
    // call. The <256-byte remainder falls to the scalar stripe walk the caller
    // already has; inputs under one tile skip the kernel entirely.
    // REFUTED, recorded so it is not re-tried: rewriting this as
    // `input.len() & !(PRE_TILE - 1)` measured IDENTICAL (158 instructions
    // either way, same two `movabsq` -- those are the P1/P2 primes, not this
    // mask). LLVM already folds a power-of-two divide-multiply into an `and`.
    // The mask spelling is also strictly more fragile: it is only correct
    // while PRE_TILE is a power of two, which the divide-multiply never cared
    // about.
    let n = (input.len() / PRE_TILE) * PRE_TILE;
    if n == 0 {
        return 0;
    }
    let (mut v1, mut v2, mut v3, mut v4) = (v[0], v[1], v[2], v[3]);
    // UNINIT, not zeroed (BRICK 1). `[0u64; 32]` compiled to `xorps` + 16
    // `movaps` -- 17 instructions per call writing 256 bytes the kernel
    // overwrites before anything reads them. The callee is an opaque call, so
    // LLVM cannot prove the full overwrite and cannot drop the store itself;
    // only the type can.
    let mut pre = [const { core::mem::MaybeUninit::<u64>::uninit() }; PRE_TILE / 8];
    for tile in input[..n].chunks_exact(PRE_TILE) {
        let Ok(tile) = <&[u8; PRE_TILE]>::try_from(tile) else {
            unreachable!()
        };
        premul(tile, &mut pre);
        // SAFETY: both arguments are fixed-size arrays, so the kernel wrote
        // every one of the PRE_TILE/8 words and `pre` is fully initialised.
        #[allow(unsafe_code)]
        let pre = unsafe { &*pre.as_ptr().cast::<[u64; PRE_TILE / 8]>() };
        let mut k = 0usize;
        while k + 4 <= PRE_TILE / 8 {
            v1 = round_pre(v1, pre[k]);
            v2 = round_pre(v2, pre[k + 1]);
            v3 = round_pre(v3, pre[k + 2]);
            v4 = round_pre(v4, pre[k + 3]);
            k += 4;
        }
    }
    v[0] = v1;
    v[1] = v2;
    v[2] = v3;
    v[3] = v4;
    #[cfg(feature = "profile")]
    {
        census::HYBRID_BYTES.fetch_add(n as u64, core::sync::atomic::Ordering::Relaxed);
        census::HYBRID_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    n
}

#[inline]
fn stripes_hybrid(input: &[u8], v: &mut [u64; 4]) -> usize {
    #[cfg(target_arch = "x86_64")]
    {
        // REFUTED, recorded: branching on `simd::avx2_state()` and sending the
        // not-yet-probed case to a `#[cold]` re-entry helper -- the change that
        // retired three `pushq` from `count_match` in `simd.rs` -- costs MORE
        // here. It removes the `avx2_detect` call and one `pushq`, but the
        // helper has to call back into this function, so `stripes_hybrid` can
        // no longer be inlined into `stripes_all` and its 165 instructions
        // exist TWICE: xxh64 total went 722 -> 773. The technique needs a
        // dispatch whose arms are tail calls; a re-entry is not one.
        if crate::simd::has_avx2() && vec_arm_enabled() {
            return stripes_pre(input, v, |tile, out| {
                // SAFETY: runtime AVX2 check above.
                #[allow(unsafe_code)]
                unsafe {
                    premul_p2_avx2(tile, out)
                }
            });
        }
    }
    // D8b: NEON is BASELINE on aarch64 (mandatory in ARMv8-A), so unlike the
    // x86 twin this needs no runtime detection and no `#[target_feature]` --
    // and therefore pays no non-inlinable-call barrier. Until this existed,
    // every aarch64 build ran the scalar stripe loop even after D8a wired the
    // x86 kernel in.
    #[cfg(target_arch = "aarch64")]
    {
        if vec_arm_enabled() {
            return stripes_pre(input, v, |tile, out| {
                // SAFETY: NEON is guaranteed by the target.
                #[allow(unsafe_code)]
                unsafe {
                    premul_p2_neon(tile, out)
                }
            });
        }
    }
    let _ = (input, v);
    0
}

static XXH_AVX2_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench/test arm: `false` forces the scalar stripe loop. Both arms MUST agree
/// bit-for-bit -- this is a format checksum, not a heuristic.
pub fn set_xxh_avx2_arm(on: bool) {
    XXH_AVX2_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

/// D8b: this gates the NEON twin as well as the AVX2 one, so it is no longer
/// "avx2_enabled". The public setter keeps its name -- bench harnesses use it.
#[inline(always)]
fn vec_arm_enabled() -> bool {
    XXH_AVX2_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
}

/// The length-add, 8/4/1-byte tail and avalanche.
///
/// 4.81 BRICK 5: the doc comment above this function used to claim there was
/// "exactly ONE copy of the finishing arithmetic", shared by the scalar and
/// AVX2 paths. There were TWO -- `Xxh64::digest` carried a full hand-written
/// second copy of the tail loops AND the three-step avalanche, because it
/// needed to add `total` rather than `len`. Splitting that one parameter out
/// is all it took to make the claim true. The twin is the whole risk: this is
/// a FORMAT checksum, and a drifted copy is a corrupt frame nobody notices
/// until interop.
///
/// NOT `#[inline(always)]`. BRICK 5 made this ONE function; `inline(always)`
/// then put a full COPY of it back into each of the two callers -- the 8-byte
/// loop, the 4-byte step, the three-step byte walk and the avalanche, in both
/// `xxh64_seed` and `Xxh64::digest`. That is the twin's code cost without the
/// twin's risk, and it buys nothing: both callers run this exactly ONCE, at the
/// very end of a whole-buffer hash, so a call is free where the duplication is
/// not. Correctness is unaffected -- it is still one source function.
#[inline(never)]
fn finish_tail(mut tail: &[u8], mut acc: u64, total: u64) -> u64 {
    // THE BOUND THAT EVERY TRIP COUNT BELOW RESTS ON, asserted rather than
    // assumed. Both callers hand over a sub-stripe remainder: `finish` passes
    // `&input[(len/32)*32..len]` and `digest` passes `&buf[..buf_len & 31]`.
    // The byte loop already asserted its own `<= 3`; the invariant that makes
    // it three -- and makes the word loop three -- was never written down.
    debug_assert!(tail.len() <= 31, "xxh64 tail {} exceeded 31", tail.len());
    acc = acc.wrapping_add(total);

    // REFUTED, recorded so it is not re-tried: BRICK 6's constant-trip-count
    // fix does NOT transfer from the byte loop to this one. Rewritten as
    // `for _ in 0..3 { if tail.len() < 8 { break } .. }`, `digest` still
    // emitted TWO `cmpq $8` tests -- the duplication the rewrite was meant to
    // remove -- and now three unrolled copies of the body instead of two,
    // costing +17 static instructions across `digest` and `xxh64_seed`. The
    // byte loop's version won because `tail.get(i)` made each step
    // individually guarded; this one's body cannot be expressed that way.
    while tail.len() >= 8 {
        let k1 = round(0, read_u64_at(tail, 0));
        acc = (acc ^ k1).rotate_left(27).wrapping_mul(P1).wrapping_add(P4);
        tail = &tail[8..];
    }
    if tail.len() >= 4 {
        let k1 = u64::from(read_u32_at(tail, 0));
        acc = (acc ^ k1.wrapping_mul(P1))
            .rotate_left(23)
            .wrapping_mul(P2)
            .wrapping_add(P3);
        tail = &tail[4..];
    }
    // 4.81 BRICK 6: at most THREE bytes reach here -- the loop above drains
    // every group of 8 and the step above that takes the 4 -- but `tail` is
    // just a slice, so LLVM vectorised for the general case and emitted FIVE
    // copies of this step: a 4x-unrolled body plus a 1x remainder loop, for a
    // walk that can never reach a second iteration of the unrolled body. The
    // clamp is a no-op that says so. It is also load-bearing, so it is
    // asserted, not assumed: dropping a tail byte silently is a corrupt frame.
    // Clamping the LENGTH (`&tail[..tail.len().min(3)]`) was not enough -- LLVM
    // kept all five copies. It needs a constant TRIP COUNT, not a bounded
    // slice: `0..3` it will fully unroll into three guarded steps and emit no
    // vector body and no remainder loop at all.
    debug_assert!(tail.len() <= 3, "byte tail {} exceeded 3", tail.len());
    for i in 0..3 {
        let Some(&b) = tail.get(i) else { break };
        acc = (acc ^ u64::from(b).wrapping_mul(P5))
            .rotate_left(11)
            .wrapping_mul(P1);
    }

    acc ^= acc >> 33;
    acc = acc.wrapping_mul(P2);
    acc ^= acc >> 29;
    acc = acc.wrapping_mul(P3);
    acc ^= acc >> 32;
    acc
}

/// D8a census (inline-execution V1). DETERMINISTIC instrument: how many bytes
/// of a real workload actually reach the AVX2 kernel, versus the scalar stripe
/// walk. This is a COUNT, not a clock -- it needs no pinning, no noise floor
/// and no interleaving, and it answers the exact question V1 asked ("is the
/// kernel reachable from the shipping path?") in one run.
///
/// `profile`-gated so the shipping path carries nothing.
#[cfg(feature = "profile")]
pub mod census {
    use core::sync::atomic::{AtomicU64, Ordering};
    pub static HYBRID_BYTES: AtomicU64 = AtomicU64::new(0);
    pub static SCALAR_BYTES: AtomicU64 = AtomicU64::new(0);
    pub static HYBRID_CALLS: AtomicU64 = AtomicU64::new(0);
    /// Read and reset. Returns `(hybrid_bytes, scalar_bytes, hybrid_calls)`.
    pub fn take() -> (u64, u64, u64) {
        (
            HYBRID_BYTES.swap(0, Ordering::Relaxed),
            SCALAR_BYTES.swap(0, Ordering::Relaxed),
            HYBRID_CALLS.swap(0, Ordering::Relaxed),
        )
    }
}

/// The scalar bulk walk: 128-byte chunks, then whole 32-byte stripes.
/// Returns bytes consumed -- always `(input.len() / 32) * 32`.
///
/// REFUTED, recorded: making this and `stripes_hybrid` return their REMAINDER
/// SLICE (as `stripes_all` does) so every slice happens where its bound is
/// live measured WORSE -- 626 -> 648. It collapsed `stripes_all` into its
/// callers and re-emitted both walkers as separate functions, which costs more
/// than the one rematerialised mask it saved. The remainder shape pays at the
/// `stripes_all` boundary and nowhere below it.
///
/// 4.81 BRICK 10: this walk existed TWICE, hand-copied. `xxh64_seed` had a
/// 128-byte chunk loop plus a 32-byte remainder loop, and `Xxh64::update` had
/// its own 128-byte loop plus its own 32-byte loop doing the identical
/// arithmetic on the identical four lanes. Roughly 115 instructions of bulk
/// stripe code emitted twice, and -- far worse for a FORMAT checksum -- two
/// places to fix whenever one of them learns something. That is exactly the
/// twin BRICK 3 and BRICK 5 removed from the epilogue and the tail; this is
/// the last one, in the bulk path.
#[inline]
fn stripes_scalar(input: &[u8], v: &mut [u64; 4]) -> usize {
    let len = input.len();
    // Slice a proven 128-byte window so LLVM drops per-load bounds checks
    // (the unrolled index loop emitted 16 cmp+ja per stripe).
    let n128 = (len / 128) * 128;
    for chunk in input[..n128].chunks_exact(128) {
        stripe(v, chunk, 0);
        stripe(v, chunk, 32);
        stripe(v, chunk, 64);
        stripe(v, chunk, 96);
    }
    let n32 = (len / 32) * 32;
    for chunk in input[n128..n32].chunks_exact(32) {
        stripe(v, chunk, 0);
    }
    #[cfg(feature = "profile")]
    census::SCALAR_BYTES.fetch_add(n32 as u64, core::sync::atomic::Ordering::Relaxed);
    n32
}

/// Hybrid tiles first, then the scalar stripe walk -- the WHOLE bulk pass.
///
/// ONE COPY. `xxh64_seed` and `Xxh64::update` each spelled this pair out, and
/// that twin is not hypothetical: it is exactly what D8a had to repair. When
/// the hybrid kernel was wired in, only `xxh64_seed` got the call -- and
/// nothing on the shipping path calls `xxh64_seed`. The encoder, the decoder
/// and the whole streaming API all drive `Xxh64::update`, so x86-64 ran a fully
/// scalar checksum for months while a tested, A/B-armed vector kernel sat two
/// functions away. This file has already deleted three twins for exactly this
/// reason (BRICK 3, 5, 10); this is the fourth, and the one that actually cost
/// something.
///
/// Returns the UNCONSUMED REMAINDER -- always the last `input.len() % 32` bytes.
///
/// Returning the slice rather than a count is what keeps the bound provable.
/// A count makes every caller re-slice (`&data[took..]`, or `split_at`) against
/// a value it cannot reason about, and each of those is a checked range with a
/// landing pad. Both walkers derive their counts from `(len / K) * K` on the
/// slice they were handed, so INSIDE here LLVM can see the bound and the
/// slicing costs nothing; outside, it could not.
#[inline]
fn stripes_all<'a>(input: &'a [u8], v: &mut [u64; 4]) -> &'a [u8] {
    // The hybrid takes whole 256-byte tiles and reports its count; the scalar
    // walk resumes from there. Under one tile the hybrid declines and the
    // scalar walk does all of it.
    let done = stripes_hybrid(input, v);
    let rest = &input[done..];
    let done2 = stripes_scalar(rest, v);
    &rest[done2..]
}

/// The four-lane fold: rotate-add the accumulators, then merge each in turn.
///
/// 4.81 BRICK 3: this sequence existed in THREE hand-copied places -- the AVX2
/// branch of `xxh64_seed`, its scalar branch, and `Xxh64::digest`. Twenty-odd
/// instructions each, and every one a place for the arms to drift apart. A
/// format checksum cannot afford a twin that drifts, so there is now one.
///
/// Plain `#[inline]`, not `always`: same argument as `finish_tail`. The
/// `#[inline(never)]`, and it took that to get one copy. Plain `#[inline]`
/// measured IDENTICAL to `always` (722 either way) -- LLVM inlines a fold this
/// small whatever the hint says -- so the duplication BRICK 3 collapsed from
/// three copies to one was quietly being re-created in both callers.
///
/// Price it at the CRATE level, not on the xxh64 symbols: outlining pulls
/// `content_checksum` out of its three shipping call sites into one function
/// too, so the xxh64 subset reads +9 while the crate reads **281_376 ->
/// 281_364**. A subset that grows because code moved INTO it is not a
/// regression. The fold runs once per hash, so the call is free.
#[inline(never)]
fn combine(v1: u64, v2: u64, v3: u64, v4: u64) -> u64 {
    let acc = v1
        .rotate_left(1)
        .wrapping_add(v2.rotate_left(7))
        .wrapping_add(v3.rotate_left(12))
        .wrapping_add(v4.rotate_left(18));
    let acc = merge(acc, v1);
    let acc = merge(acc, v2);
    let acc = merge(acc, v3);
    merge(acc, v4)
}

/// XXH64 with seed 0 -- the only seed zstd uses for the content checksum.
pub fn xxh64(input: &[u8]) -> u64 {
    xxh64_seed(input, 0)
}

pub fn xxh64_seed(input: &[u8], seed: u64) -> u64 {
    let len = input.len();
    if len >= 32 {
        let mut v = [
            seed.wrapping_add(P1).wrapping_add(P2),
            seed.wrapping_add(P2),
            seed,
            seed.wrapping_sub(P1),
        ];
        // The bulk walk hands back the REMAINDER, which is exactly the tail the
        // finisher wants -- so it goes straight through. The old shape took a
        // byte COUNT, stored it in `off`, and rebuilt the same slice as
        // `&input[off..]` inside `finish`: a subtraction and a second checked
        // range to recover a value the walk already had. `finish` existed only
        // to perform that reconstruction and is gone with it.
        let rest = stripes_all(input, &mut v);
        let [v1, v2, v3, v4] = v;
        debug_assert_eq!(len - rest.len(), (len / 32) * 32);
        finish_tail(rest, combine(v1, v2, v3, v4), len as u64)
    } else {
        finish_tail(input, seed.wrapping_add(P5), len as u64)
    }
}

/// Incremental XXH64 (seed 0), matching [`xxh64`] on the concatenated input.
pub struct Xxh64 {
    total: u64,
    v1: u64,
    v2: u64,
    v3: u64,
    v4: u64,
    buf: [u8; 32],
    buf_len: usize,
}

impl Xxh64 {
    /// Seed 0 -- the only seed zstd uses for the content checksum.
    pub fn new() -> Self {
        Self {
            total: 0,
            v1: P1.wrapping_add(P2),
            v2: P2,
            v3: 0,
            v4: 0u64.wrapping_sub(P1),
            buf: [0; 32],
            buf_len: 0,
        }
    }

    /// Absorb more bytes.
    pub fn update(&mut self, mut data: &[u8]) {
        self.total = self.total.wrapping_add(data.len() as u64);
        // 4.81 BRICK 7: the BRICK 4 finding again, on the writing side. An
        // unbounded `buf_len` made `self.buf[buf_len..buf_len + take]` cost an
        // overflow check AND a range check, both branching to a shared
        // `slice_index_fail` block. Masking to the 0..=31 the field already
        // obeys retires both branches.
        debug_assert!(self.buf_len < 32, "buf_len {} escaped 0..32", self.buf_len);
        let buf_len = self.buf_len & 31;

        // ...and a stripe cannot complete, so nothing below can run: buffer
        // and leave. This guard is what makes BRICK 9 free -- without it the
        // hoisted load/store set is paid by every small streaming update that
        // does no stripe work at all.
        if buf_len + data.len() < 32 {
            // REFUTED, recorded: masking this length (`data.len() & 31`, the
            // BRICK 4 idiom applied to a length rather than an offset) does NOT
            // stop the copy going out to `memcpy` -- xxh64 total identical at
            // 722, still three `memcpy` calls in this function. Small-copy
            // inlining is a codegen decision the length bound does not reach.
            self.buf[buf_len..buf_len + data.len()].copy_from_slice(data);
            self.buf_len = buf_len + data.len();
            return;
        }

        // 4.81 BRICK 9: the four accumulators live in REGISTERS for the whole
        // call. LLVM already kept them in registers WITHIN each loop, so the
        // usual "hoist the accumulator" brick was already done -- what it could
        // not do was carry them ACROSS the loops, because `consume_stripe` took
        // `&mut self` and every boundary was a visible write to memory. The
        // 128-byte loop therefore ended `movq %r10, 8(%rdi)` x4 and the 32-byte
        // loop opened `movq 8(%rdi), %r8` x4 -- eight memory ops to hand four
        // values between two adjacent loops. One load set in, one store set
        // out, and `consume_stripe` disappears with them.
        let mut v = [self.v1, self.v2, self.v3, self.v4];

        if buf_len > 0 {
            // 4.81 BRICK 10: control flow above has already established
            // `buf_len + data.len() >= 32`, so this top-up ALWAYS completes the
            // stripe. That makes three things here dead: the `.min(data.len())`
            // (a cmov -- the take is exactly `32 - buf_len`), the `buf_len +=
            // take` accumulate-and-store, and the `== 32` test with its branch
            // that guarded the drain. Same shape as BRICK 2: a guard added
            // upstream turns downstream generality into dead code, and it only
            // shows up if you go back and look.
            let take = 32 - buf_len;
            // ONE bounds check, not two. `&data[..take]` and `&data[take..]`
            // are each a checked range with its own `slice_index_fail` landing
            // pad; `split_at` proves the same bound once and yields both
            // halves. The guard above already established `data.len() >= take`.
            let (head, rest) = data.split_at(take);
            self.buf[buf_len..].copy_from_slice(head);
            data = rest;
            // ...and no 32-byte COPY of the buffer. `let chunk = self.buf;`
            // moved all 32 bytes into a fresh local purely to end the borrow
            // before `self.buf_len = 0`; reordering the store does the same for
            // nothing. `stripe` only reads.
            stripe(&mut v, &self.buf, 0);
            self.buf_len = 0;
        }
        // D8a (inline-execution V1): the bulk goes through the AVX2 hybrid
        // FIRST, exactly as `xxh64_seed` does, then the shared scalar walk
        // finishes whatever whole stripes are left.
        //
        // Until this line existed, `premul_p2_avx2` -- one of the codec's only
        // two hand-written vector kernels, carrying a documented 1.14-1.26x --
        // was reachable ONLY from `xxh64_seed`, and nothing on the shipping
        // path calls that. The encoder (`encode_oneshot`), the decoder (frame
        // verify) and the whole streaming API all drive `Xxh64::update`, so
        // x86-64 has been running a fully scalar checksum while a tested,
        // A/B-armed vector kernel sat two functions away. `DecodeChecksum` is
        // 22-26% of decode on the incompressible corpora.
        //
        // Nothing about the buffering discipline moves: the hybrid consumes
        // only whole 256-byte tiles and reports the count, so `buf`/`buf_len`
        // and the stripe boundary are untouched.
        let rest = stripes_all(data, &mut v);
        if !rest.is_empty() {
            // BRICK 4's idiom on the WRITING side's last slice. Both walkers
            // consume whole 32-byte stripes, so `rest.len()` is `len % 32` and
            // cannot reach 32 -- but nothing in the type says so, and
            // `self.buf` is `[u8; 32]`, so the store carried its own range
            // check. The mask is a no-op the optimiser can read.
            debug_assert!(rest.len() < 32, "xxh64 remainder {} reached 32", rest.len());
            let n = rest.len() & 31;
            self.buf[..n].copy_from_slice(&rest[..n]);
            self.buf_len = n;
        }

        [self.v1, self.v2, self.v3, self.v4] = v;
    }

    /// Current digest of all bytes absorbed so far.
    pub fn digest(&self) -> u64 {
        // 4.81 BRICK 4: `buf_len` is 0..=31 by construction -- `update` drains
        // the buffer the instant it reaches 32 -- but nothing in the TYPE says
        // so, so LLVM opened every `digest()` with `cmpq $33, %rdx; jae ...`
        // into a `slice_index_fail` panic block, and had to assume the 8-byte
        // tail loop could run any number of times. The mask is a no-op the
        // optimiser can actually read, and it costs nothing: it folds into the
        // load's addressing.
        debug_assert!(self.buf_len < 32, "buf_len {} escaped 0..32", self.buf_len);
        let rest = &self.buf[..self.buf_len & 31];
        // ...and the third copy of the four-lane fold goes with it (BRICK 3).
        // 4.81 BRICK 8: `large` was a stored bool set to true at all three
        // stripe sites -- including INSIDE the 32-byte loop, where it compiled
        // to `movb $1, 80(%rdi)` once per stripe: a store to a SECOND cache
        // line, per 32 bytes, to re-assert something already true. It is also
        // pure redundancy. The buffer drains at exactly 32 and never holds
        // more, so at least one stripe has run iff at least 32 bytes have been
        // absorbed -- `large` was only ever a slower spelling of `total >= 32`.
        let acc = if self.total >= 32 {
            combine(self.v1, self.v2, self.v3, self.v4)
        } else {
            P5
        };
        finish_tail(rest, acc, self.total)
    }
}

impl Default for Xxh64 {
    fn default() -> Self {
        Self::new()
    }
}

/// Low 32 bits, little-endian -- the 4-byte zstd content checksum field.
pub fn content_checksum(data: &[u8]) -> u32 {
    xxh64(data) as u32
}

#[cfg(test)]
mod tests {
    use super::*;

    /// SPEC ORACLE at every length across every boundary.
    ///
    /// The shipping path is now four walks stitched together -- 256-byte hybrid
    /// tiles, 128-byte scalar chunks, 32-byte stripes, then a tail whose three
    /// loops all run on CONSTANT trip counts. Every one of those bounds is an
    /// assumption about how much input can still be left, and a wrong one drops
    /// bytes silently: this is a FORMAT checksum, so a dropped byte is a corrupt
    /// frame nobody notices until interop.
    ///
    /// `ref_xxh64` is an independent transcription of the spec -- one stripe
    /// loop, one 8-byte loop, one byte loop, no tiles, no bounded counts. It is
    /// deliberately the slow obvious thing.
    #[test]
    fn matches_spec_oracle_at_every_length() {
        const RP1: u64 = 0x9E37_79B1_85EB_CA87;
        const RP2: u64 = 0xC2B2_AE3D_27D4_EB4F;
        const RP3: u64 = 0x1656_67B1_9E37_79F9;
        const RP4: u64 = 0x85EB_CA77_C2B2_AE63;
        const RP5: u64 = 0x27D4_EB2F_1656_67C5;
        fn rnd(acc: u64, x: u64) -> u64 {
            acc.wrapping_add(x.wrapping_mul(RP2))
                .rotate_left(31)
                .wrapping_mul(RP1)
        }
        fn w(d: &[u8], i: usize) -> u64 {
            u64::from_le_bytes(d[i..i + 8].try_into().unwrap())
        }
        fn ref_xxh64(d: &[u8]) -> u64 {
            let n = d.len();
            let mut i = 0usize;
            let mut acc;
            if n >= 32 {
                let (mut v1, mut v2, mut v3, mut v4) =
                    (RP1.wrapping_add(RP2), RP2, 0u64, 0u64.wrapping_sub(RP1));
                while i + 32 <= n {
                    v1 = rnd(v1, w(d, i));
                    v2 = rnd(v2, w(d, i + 8));
                    v3 = rnd(v3, w(d, i + 16));
                    v4 = rnd(v4, w(d, i + 24));
                    i += 32;
                }
                acc = v1
                    .rotate_left(1)
                    .wrapping_add(v2.rotate_left(7))
                    .wrapping_add(v3.rotate_left(12))
                    .wrapping_add(v4.rotate_left(18));
                for v in [v1, v2, v3, v4] {
                    acc = (acc ^ rnd(0, v)).wrapping_mul(RP1).wrapping_add(RP4);
                }
            } else {
                acc = RP5;
            }
            acc = acc.wrapping_add(n as u64);
            while i + 8 <= n {
                acc = (acc ^ rnd(0, w(d, i)))
                    .rotate_left(27)
                    .wrapping_mul(RP1)
                    .wrapping_add(RP4);
                i += 8;
            }
            if i + 4 <= n {
                let k = u64::from(u32::from_le_bytes(d[i..i + 4].try_into().unwrap()));
                acc = (acc ^ k.wrapping_mul(RP1))
                    .rotate_left(23)
                    .wrapping_mul(RP2)
                    .wrapping_add(RP3);
                i += 4;
            }
            while i < n {
                acc = (acc ^ u64::from(d[i]).wrapping_mul(RP5))
                    .rotate_left(11)
                    .wrapping_mul(RP1);
                i += 1;
            }
            acc ^= acc >> 33;
            acc = acc.wrapping_mul(RP2);
            acc ^= acc >> 29;
            acc = acc.wrapping_mul(RP3);
            acc ^= acc >> 32;
            acc
        }

        let mut data = alloc::vec![0u8; 1100];
        for (i, b) in data.iter_mut().enumerate() {
            *b = (i.wrapping_mul(167).wrapping_add(29) % 251) as u8;
        }
        // 0..=600 walks the <32 tail, the 32-byte stripe edge, the 128-byte
        // chunk edge and the 256-byte tile edge; 1024/1088 clear two tiles.
        let mut checked = 0usize;
        for len in (0usize..=600).chain([768, 1023, 1024, 1025, 1088, 1099]) {
            let d = &data[..len];
            let want = ref_xxh64(d);
            assert_eq!(xxh64(d), want, "one-shot len {len}");
            // ...and the streaming path, at chunk sizes that land on and just
            // off every one of those boundaries.
            for c in [1usize, 7, 31, 32, 33, 127, 128, 129, 255, 256, 257] {
                let mut h = Xxh64::new();
                for part in d.chunks(c) {
                    h.update(part);
                }
                assert_eq!(h.digest(), want, "stream len {len} chunk {c}");
            }
            checked += 1;
        }
        assert_eq!(checked, 607, "lengths actually exercised");
    }

    /// D8b gate, runnable on ANY host including this x86 one.
    ///
    /// Both vector kernels compute `a * P2` the same way -- not with a 64x64
    /// multiply (no ISA here has one across lanes) but with the 32x32 partial
    /// products:
    ///
    /// ```text
    ///   a*P2 = alo*P2lo + ((alo*P2hi + ahi*P2lo) << 32)      (mod 2^64)
    /// ```
    ///
    /// That identity is the only thing a lane-for-lane port can get wrong, and
    /// it is arch-independent, so it can be proven here even though no aarch64
    /// machine was available. The high*high term is correctly absent: it is
    /// worth 2^64 and vanishes mod 2^64. The `<< 32` is allowed to drop the
    /// carry out of the cross term for the same reason.
    ///
    /// This does NOT prove `premul_p2_neon` runs correctly on a real core --
    /// only that the arithmetic it implements is right. See that function's
    /// docs for what remains ungated.
    #[test]
    fn premul_decomposition_matches_mul() {
        fn decomposed(a: u64) -> u64 {
            let (alo, ahi) = (a & 0xFFFF_FFFF, a >> 32);
            let (p2lo, p2hi) = (P2 & 0xFFFF_FFFF, P2 >> 32);
            let t0 = alo.wrapping_mul(p2lo);
            let t1 = alo.wrapping_mul(p2hi);
            let t2 = ahi.wrapping_mul(p2lo);
            t0.wrapping_add(t1.wrapping_add(t2) << 32)
        }
        // edges: zero, all-ones, lane boundaries, and the carry seams
        for &a in &[
            0u64,
            1,
            u64::MAX,
            1 << 31,
            1 << 32,
            (1 << 32) - 1,
            u32::MAX as u64,
            (u32::MAX as u64) << 32,
            0xFFFF_FFFF_0000_0000,
            0x0000_0000_FFFF_FFFF,
            0x8000_0000_8000_0000,
        ] {
            assert_eq!(decomposed(a), a.wrapping_mul(P2), "a = {a:#018x}");
        }
        // a wide deterministic sweep -- no RNG, so this is reproducible
        let mut x = 0x243F_6A88_85A3_08D3u64;
        for _ in 0..200_000 {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            assert_eq!(decomposed(x), x.wrapping_mul(P2), "a = {x:#018x}");
        }
        // and every single-bit value, which is where a shift error shows
        for b in 0..64 {
            let a = 1u64 << b;
            assert_eq!(decomposed(a), a.wrapping_mul(P2), "bit {b}");
        }
    }

    #[test]
    fn empty_seed0() {
        // Published XXH64("", 0).
        assert_eq!(xxh64(b""), 0xEF46_DB37_51D8_E999);
        assert_eq!(content_checksum(b""), 0x51D8_E999);
    }

    #[test]
    fn c_zstd_157_checksums() {
        // facebook/zstd v1.5.7 CLI frames (see decode tests).
        assert_eq!(content_checksum(b"a"), 0xA98C_6E5B);
        assert_eq!(content_checksum(b"hello"), 0x889F_6DA3);
    }

    #[test]
    fn thirty_two_zeros_stripe_path() {
        assert_eq!(xxh64(&[0u8; 32]), 0xF6E9_BE5D_7063_2CF5);
    }

    #[test]
    fn hasher_matches_oneshot() {
        let samples: &[&[u8]] = &[
            b"", b"a", b"hello", &[0u8; 31], &[0u8; 32], &[0u8; 33], &[0u8; 64],
        ];
        for &s in samples {
            let mut h = Xxh64::new();
            h.update(s);
            assert_eq!(h.digest(), xxh64(s), "len {}", s.len());
            let mut h2 = Xxh64::new();
            for chunk in s.chunks(3) {
                h2.update(chunk);
            }
            assert_eq!(h2.digest(), xxh64(s), "chunked len {}", s.len());
        }
        let mut long = vec![0u8; 100_003];
        for (i, b) in long.iter_mut().enumerate() {
            *b = (i.wrapping_mul(251) % 251) as u8;
        }
        let mut h = Xxh64::new();
        h.update(&long);
        assert_eq!(h.digest(), xxh64(&long), "long oneshot");
        let mut h2 = Xxh64::new();
        for chunk in long.chunks(17) {
            h2.update(chunk);
        }
        assert_eq!(h2.digest(), xxh64(&long), "long chunked");
    }
}

#[cfg(test)]
mod locality_probe {
    use super::*;
    use std::time::Instant;

    /// Is our xxh64 COMPUTE-bound or MEMORY-bound? If hashing a cache-resident
    /// buffer is much faster per byte than hashing a 32 MiB one, the checksum
    /// is limited by reading cold memory -- and fusing it into the decode loop
    /// (hashing each block while it is still hot) would be a real win.
    #[ignore]
    #[test]
    fn xxh64_throughput_by_working_set() {
        for (label, sz) in [
            ("32 KiB (L1/L2)", 32usize << 10),
            ("256 KiB (L2)", 256 << 10),
            ("4 MiB (L3)", 4 << 20),
            ("32 MiB (DRAM)", 32 << 20),
        ] {
            let buf = alloc::vec![0u8; sz];
            // Equalise total bytes hashed across sizes.
            let total: usize = 512 << 20;
            let reps = total / sz;
            let t = Instant::now();
            let mut acc = 0u64;
            for _ in 0..reps {
                acc ^= u64::from(content_checksum(&buf));
            }
            let s = t.elapsed().as_secs_f64();
            std::hint::black_box(acc);
            println!("  {label:16} {:7.1} GB/s", (total as f64) / s / 1e9);
        }
    }
}
