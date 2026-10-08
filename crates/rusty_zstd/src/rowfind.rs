//! The ROW match finder -- our `ZSTD_row_match_finder`.
//!
//! ## Why
//!
//! `chain_find_best_inner` walks a hash CHAIN: `m = chain[m & mask]`, one
//! DEPENDENT load per candidate, each a potential cache miss that cannot issue
//! until the previous one retires. `examples/rowceiling.rs` counts them:
//!
//! | level | strategy | dependent chain loads | loads/KiB |
//! |---|---|---:|---:|
//! | L1 | fast | 0 | 0.0 |
//! | L3 | dfast | 0 | 0.0 |
//! | L5 | greedy | 43,009,953 | 335.7 |
//! | L7 | lazy | 139,427,236 | 1088.1 |
//! | L9 | lazy2 | 220,979,813 | 1724.6 |
//! | L12 | lazy2 | 674,337,493 | 5262.8 |
//!
//! **Note the zeros.** L1 and L3 walk no chain at all, so this finder cannot
//! help there -- it is an L5-L12 lever. `docs/plans/inline-execution.md` E1
//! originally justified it with an L3 profile; that was wrong and the plan now
//! says so.
//!
//! ## The shape
//!
//! A ROW holds `ROW` positions that share a hash bucket, with their tags
//! CONTIGUOUS. One load brings in all 16 tags; one `pcmpeqb` + `pmovmskb`
//! compares them all; the set bits are the candidates. **One dependent load per
//! ROW instead of per CANDIDATE.**
//!
//! Rows fold 16 hash buckets together (`row = h >> 4`), so the table is exactly
//! the size of the chain it replaces. The tag comes from `hash4_tag_mls`'s
//! SECOND multiply, which is independent of the index, so it still discriminates
//! inside a row.
//!
//! ## The gate
//!
//! This is **bitstream-CHANGING**: a row holds the last `ROW` positions for its
//! bucket, where the chain held all of them linked, so the candidate SET differs
//! and the encoder finds different matches. It cannot ship on byte-identity.
//! Its gate is `examples/rowboard.rs` -- round-trip on every cell, plus
//! compressed size per corpus, plus the deterministic load count above.
//!
//! The scalar oracle stays in the tree forever and `row_tag_mask_oracle` is
//! asserted equal to the vector kernel on every pattern in the unit tests.
//!
//! ## Row WIDTH is a per-frame parameter (2026-10-04)
//!
//! A row holds `1 << row_log` positions, `row_log` in 4..=6 -- 16, 32 or 64
//! slots, the three widths libzstd's row finder uses. The width is fixed when
//! the table is `reset` and the walk / fill kernels are monomorphised on it
//! (`const RL`), so each width keeps its shift and its mask as immediates and
//! the 16-slot code is the code it always was. Width buys DEPTH back: measured
//! on six full silesia files against the hash chain, 16 slots cost +0.5% size
//! at L7/L9 and +2.0% at L12, while 64 slots sized from the hash log with a
//! full fill are SMALLER than the chain at all three (-1.0% / -1.1% / -0.5%).

/// Narrowest row: 16 slots = one SSE register of tags, one `pmovmskb`.
pub(crate) const ROW_LOG_MIN: u32 = 4;
/// Widest row: 64 slots = four SSE registers, one 64-bit candidate mask.
pub(crate) const ROW_LOG_MAX: u32 = 6;
/// Slots in the widest row.
#[cfg(test)]
pub(crate) const ROW_MAX: usize = 1 << ROW_LOG_MAX;
/// A row's candidate mask: bit `i` is slot `i`. Wide enough for 64 slots; the
/// 16- and 32-slot kernels use the low bits.
pub(crate) type RowMask = u64;

/// W2's walk census: `[probes, OLD slot-visits, NEW slot-visits]`. Both cost
/// models are evaluated from the SAME mask in one run, so the comparison needs
/// no A/B build and carries no clock.
#[cfg(feature = "profile")]
pub static ROW_WALK: [crate::census64::AtomicU64; 3] = [
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
    crate::census64::AtomicU64::new(0),
];
/// Read and clear the row-walk census.
#[cfg(feature = "profile")]
pub fn take_row_walk() -> [u64; 3] {
    use core::sync::atomic::Ordering::Relaxed;
    [
        ROW_WALK[0].swap(0, Relaxed),
        ROW_WALK[1].swap(0, Relaxed),
        ROW_WALK[2].swap(0, Relaxed),
    ]
}

/// Rotate an `N`-bit candidate mask right by `head` (`N = 1 << RL`,
/// `head < N`), so that bit 0 is the newest slot and bit `N - 1` the oldest.
///
/// NEWEST ON BIT 0 (2026-10-08). The row used to fill UPWARD, newest on the
/// top bit, so the walk took candidates with `bsr` and cleared them with
/// `btr` -- and `btr` needs `bsr`'s result, so every iteration carried a
/// `bsr` latency (3 cycles) plus the clear on the loop's dependency chain.
/// libzstd fills its rows DOWNWARD (`ZSTD_row_nextIndex`) for exactly this:
/// newest on bit 0, `ctz` off the chain, `m &= m - 1` the only carried op.
/// Rows now fill downward too. Same ages, same newest-first order, same
/// candidates: the slot permutation is invisible to the output.
///
/// Callgrind cannot see this (same instruction count); the clock can.
/// Pinned, best of 4 alternating rounds, MB/s before -> after: L12 dickens
/// 14.6 -> 20.3, samba 32.4 -> 45.8, mozilla 24.5 -> 33.8, nci 55.8 -> 75.3,
/// xml 52.1 -> 70.2, x-ray 22.8 -> 24.5; L9 dickens 34.6 -> 42.3, samba 72.9
/// -> 85.3, mozilla 54.5 -> 62.6, nci 119.4 -> 134.9, xml 105.3 -> 116.9,
/// x-ray 39.4 -> 44.6. The win scales with candidates per probe (41 on
/// dickens at L12, 12 at L9), which is the length of the carried chain.
#[inline(always)]
pub(crate) fn row_rot<const RL: u32>(mask: RowMask, head: u32) -> RowMask {
    debug_assert!((ROW_LOG_MIN..=ROW_LOG_MAX).contains(&RL) && head < (1 << RL));
    if RL == ROW_LOG_MAX {
        mask.rotate_right(head)
    } else {
        let n = 1u32 << RL;
        ((mask >> head) | (mask << (n - head))) & ((1u64 << n) - 1)
    }
}

/// Tag-scan ORACLE: which of the row's lanes equal `want`, as a bitmask.
///
/// Stays in the tree as the correctness reference and the non-x86/ARM path.
/// Dead on x86_64/aarch64 by construction -- the vector kernel is used
/// there. It stays compiled so the unit tests can hold it against the
/// kernel, and it IS the implementation on every other arch.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), allow(dead_code))]
#[inline]
pub(crate) fn row_tag_mask_oracle(tags: &[u8], want: u8) -> RowMask {
    debug_assert!(tags.len() <= 1 << ROW_LOG_MAX);
    let mut m: RowMask = 0;
    for (i, &t) in tags.iter().enumerate() {
        if t == want {
            m |= 1 << i;
        }
    }
    m
}

/// Tag-scan KERNEL: 16 lanes per compare, `1 << (RL - 4)` compares per row.
///
/// The trip count is a constant of the monomorphisation (1, 2 or 4), so the
/// 16-slot kernel is still exactly one load, one `pcmpeqb`, one `pmovmskb`.
///
/// # Safety
/// `tags` must be valid for reads of `1 << RL` bytes.
#[inline(always)]
#[allow(unsafe_code)]
pub(crate) unsafe fn row_tag_mask_raw<const RL: u32>(tags: *const u8, want: u8) -> RowMask {
    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is baseline on x86_64, so this needs no runtime detect and no
        // `#[target_feature]` -- which matters, because a `target_feature`
        // helper becomes a CALL from a baseline caller and the call would cost
        // more than the compare saves (codec-vectorize-kernel, Law 1).
        //
        // SAFETY: the caller guarantees `1 << RL` readable bytes; chunk `k`
        // reads bytes `16k .. 16k + 16` with `16k + 16 <= 1 << RL`. `loadu`
        // has no alignment requirement.
        unsafe {
            use core::arch::x86_64::*;
            let w = _mm_set1_epi8(want as i8);
            let mut m: RowMask = 0;
            let mut k = 0usize;
            while k < (1usize << RL) / 16 {
                let v = _mm_loadu_si128(tags.add(k * 16) as *const __m128i);
                let bits = _mm_movemask_epi8(_mm_cmpeq_epi8(v, w)) as u32 as u64;
                m |= bits << (k * 16);
                k += 1;
            }
            m
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON has no `pmovmskb`. The standard replacement: compare, AND with a
        // per-lane bit weight, then reduce each half. This is the shape every
        // NEON codec uses for movemask.
        //
        // SAFETY: as above -- every 16-byte chunk read lies inside the
        // caller's `1 << RL` bytes.
        unsafe {
            use core::arch::aarch64::*;
            const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            let w = vdupq_n_u8(want);
            let weights = vld1q_u8(BITS.as_ptr());
            let mut m: RowMask = 0;
            let mut k = 0usize;
            while k < (1usize << RL) / 16 {
                let v = vld1q_u8(tags.add(k * 16));
                let sel = vandq_u8(vceqq_u8(v, w), weights);
                let lo = vaddv_u8(vget_low_u8(sel)) as u64;
                let hi = vaddv_u8(vget_high_u8(sel)) as u64;
                m |= (lo | (hi << 8)) << (k * 16);
                k += 1;
            }
            m
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        // SAFETY: the caller guarantees `1 << RL` readable bytes.
        row_tag_mask_oracle(
            unsafe { core::slice::from_raw_parts(tags, 1usize << RL) },
            want,
        )
    }
}

/// Safe wrapper over [`row_tag_mask_raw`] for a whole row slice.
#[cfg(test)]
#[allow(unsafe_code)]
pub(crate) fn row_tag_mask<const RL: u32>(tags: &[u8], want: u8) -> RowMask {
    assert_eq!(tags.len(), 1usize << RL);
    // SAFETY: the slice is exactly `1 << RL` bytes long.
    unsafe { row_tag_mask_raw::<RL>(tags.as_ptr(), want) }
}

/// 16 row positions on one cache line. The storage unit of `RowTable::pos`:
/// a `Vec` of these is 64-byte aligned, so a row starts on a line boundary.
#[cfg(feature = "alloc")]
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub(crate) struct PosLine([u32; 16]);

/// 64 row tags on one cache line. The storage unit of `RowTable::tags`.
#[cfg(feature = "alloc")]
#[repr(C, align(64))]
#[derive(Clone, Copy)]
pub(crate) struct TagLine([u8; 64]);

/// The row table: `1 << row_log` positions per row with their tags contiguous.
///
/// ## Line-aligned storage (2026-10-08)
///
/// `pos` and `tags` were plain `Vec<u32>` / `Vec<u8>`, whose bases the
/// allocator places 16 bytes past a line (glibc's `mmap` chunk header; the
/// Windows heap is no better), so EVERY row straddled one more cache line than
/// it spans: a 16-slot row's 64 position bytes touched 2 lines instead of 1,
/// a 64-slot row's 256 bytes 5 instead of 4. libzstd asserts its rows are
/// line-aligned (`ZSTD_row_prefetch`). Storage is now a `Vec` of
/// 64-byte-aligned lines; the element layout is unchanged, so the output is
/// too.
///
/// Measured: callgrind's cache simulation with a 2 MiB last level (an L2
/// stand-in), L9, last-level misses: dickens 17.4M -> 14.6M, x-ray 23.2M ->
/// 21.1M (libzstd 1.5.7: 12.9M / 17.8M). Wall clock on six silesia files at
/// L9 and L12, pinned: within this box's +-10% noise -- the adjacent-line
/// prefetcher was already hiding most of each straddle. Kept for the
/// deterministic miss count, not for a timed win.
///
/// RE-MEASURED after the hash cache, downward rows and hash tags (same day):
/// the aligned table against one deliberately skewed by 16 bytes (the old
/// layout), pinned, best of 4 alternating rounds -- aligned faster in 15 of
/// 18 cells, by 1-4% (L7 samba 115.2 vs 110.6 MB/s, L9 x-ray 54.2 vs 51.3,
/// L12 x-ray 27.7 vs 26.6); the three others within 2%. `veins/greedy`
/// measured aligned tables SLOWER at L7/L9 on its own code (1.049 / 1.084);
/// not reproduced on this one.
#[cfg(feature = "alloc")]
#[derive(Default, Clone)]
pub(crate) struct RowTable {
    /// `rows << row_log` positions, 16 to a line.
    pos: alloc::vec::Vec<PosLine>,
    /// `rows << row_log` tags, so each row's tags are contiguous, 64 to a line.
    tags: alloc::vec::Vec<TagLine>,
    /// Next slot to write, per row. Wraps at the row width, so a row always
    /// holds the most recent `1 << row_log` positions for its buckets.
    pub head: alloc::vec::Vec<u8>,
    /// W1: `rows - 1`, cached. `row_of` ran on every position and derived this
    /// from `head.len()` -- a Vec field load feeding the address computation of
    /// the very next load, i.e. on the dependency path.
    row_mask: usize,
    /// log2 of the slots per row (4, 5 or 6); 0 while the table is unallocated.
    row_log: u32,
}

#[cfg(feature = "alloc")]
impl RowTable {
    /// Allocate (or resize and clear) for a `1 << entries_log` entry budget
    /// split into rows of `1 << row_log` slots.
    pub fn reset(&mut self, entries_log: u32, row_log: u32) {
        let row_log = row_log.clamp(ROW_LOG_MIN, ROW_LOG_MAX);
        let entries = 1usize << entries_log.clamp(ROW_LOG_MAX, 24);
        let rows = entries >> row_log;
        // `entries >= 64`: a whole number of lines of either kind.
        self.pos.clear();
        self.pos.resize(entries / 16, PosLine([0; 16]));
        self.tags.clear();
        self.tags.resize(entries / 64, TagLine([0; 64]));
        self.head.clear();
        self.head.resize(rows, 0);
        self.row_mask = rows - 1;
        self.row_log = row_log;
    }

    /// Slots in the table (`rows << row_log`); 0 while unallocated.
    #[inline(always)]
    fn slots(&self) -> usize {
        self.pos.len() * 16
    }

    /// Base of the position array, as `u32` elements.
    #[inline(always)]
    fn pos_ptr(&self) -> *const u32 {
        self.pos.as_ptr() as *const u32
    }

    /// Base of the tag array, as bytes.
    #[inline(always)]
    fn tags_ptr(&self) -> *const u8 {
        self.tags.as_ptr() as *const u8
    }

    /// Store `ip` / `tag` in row `r` (first slot `row_at`, current head
    /// `head`), one slot BELOW the head, and make that slot the head: rows
    /// fill downward, so the head is always the newest slot (see `row_rot`).
    ///
    /// # Safety
    /// `r < head.len()`, `row_at == r << rl` and `head < 1 << rl`.
    #[inline(always)]
    #[allow(unsafe_code)]
    unsafe fn put(&mut self, r: usize, row_at: usize, head: usize, rl: u32, ip: u32, tag: u8) {
        debug_assert!(r < self.head.len() && row_at == r << rl && head < (1 << rl));
        let s = head.wrapping_sub(1) & ((1usize << rl) - 1);
        let at = row_at + s;
        debug_assert!(at < self.slots());
        // SAFETY: the caller's bounds; `PosLine` / `TagLine` are `repr(C)`
        // arrays, so the line vectors are `slots()` contiguous `u32` / `u8`.
        unsafe {
            *(self.pos.as_mut_ptr() as *mut u32).add(at) = ip;
            *(self.tags.as_mut_ptr() as *mut u8).add(at) = tag;
            *self.head.get_unchecked_mut(r) = s as u8;
        }
    }

    /// Touch the lines a probe of hash bucket `h`'s row will read: its tags,
    /// its positions and its head byte. A pure hint -- see
    /// `simd::prefetch_read_known`; nothing is read and nothing can change.
    #[inline(always)]
    pub fn prefetch_row<const RL: u32>(&self, h: usize) {
        debug_assert_eq!(RL, self.row_log);
        let r = (h >> RL) & self.row_mask;
        let at = r << RL;
        // Every address below is inside its table: `r <= row_mask`, so
        // `head[r]`, `tags[at .. at + (1 << RL)]` and the row's `4 << RL`
        // position bytes all exist. The `_known` hint drops the per-address
        // `< len` guard the checked hint re-tested -- seven compares and
        // branches per probe at 64 slots (2026-10-08 callgrind).
        debug_assert!(r < self.head.len() && at + (1usize << RL) <= self.slots());
        crate::simd::prefetch_read_known(&self.head, r);
        // Tags: `1 << RL <= 64` bytes at a multiple of `1 << RL` from a
        // line-aligned base -- inside ONE line.
        let tags: &[u8] = {
            #[allow(unsafe_code)]
            // SAFETY: the tag lines are `slots()` initialised bytes (see
            // `put`); the view is used for address arithmetic by a prefetch
            // only.
            unsafe {
                core::slice::from_raw_parts(self.tags_ptr(), self.slots())
            }
        };
        crate::simd::prefetch_read_known(tags, at);
        // Positions: 4 bytes per slot, i.e. 1 / 2 / 4 whole lines.
        let pos: &[u8] = {
            #[allow(unsafe_code)]
            // SAFETY: a `[u32]` is valid to view as `4 * len` initialised
            // bytes; the slice is used for address arithmetic by a prefetch
            // and never read through.
            unsafe {
                core::slice::from_raw_parts(self.pos_ptr() as *const u8, self.slots() * 4)
            }
        };
        // At most the FIRST TWO position lines, as libzstd names (`relRow`
        // and `relRow + 16`; "prefetching more of the hash table does not
        // appear to be beneficial"). A 64-slot row is four lines, and since
        // the hash cache names a row for EVERY inserted position, the other
        // two were bandwidth and fill buffers the row may never repay: on a
        // literal-heavy probe only a few slots are read. Measured 2026-10-08,
        // pinned, best of 4 alternating rounds, MB/s all four -> first two
        // lines: L12 x-ray 27.1 -> 30.4, nci 79.0 -> 85.5, samba 48.0 ->
        // 51.2, xml 73.1 -> 77.6, mozilla 37.8 -> 40.2, dickens 21.2 -> 22.4;
        // L11 (also 64 slots) 4 of 6 files up 3-10%, the others within noise.
        // ONE line was no better than two at L10 (32 slots) or L12. 16-slot
        // rows are one line either way.
        let mut off = 0usize;
        while off < (4usize << RL).min(128) {
            crate::simd::prefetch_read_known(pos, (at << 2) + off);
            off += 64;
        }
    }

    /// Forget every position, keeping the allocation and the geometry.
    pub fn clear(&mut self) {
        self.pos.fill(PosLine([0; 16]));
        self.tags.fill(TagLine([0; 64]));
        self.head.fill(0);
    }

    /// log2 of the slots per row. Only meaningful when the table is allocated.
    #[inline(always)]
    pub fn row_log(&self) -> u32 {
        self.row_log
    }

    /// Row index for hash bucket `h`. `1 << row_log` buckets share a row.
    #[inline(always)]
    pub fn row_of(&self, h: usize) -> usize {
        debug_assert_eq!(self.row_mask, self.head.len().wrapping_sub(1));
        (h >> self.row_log) & self.row_mask
    }

    /// `row_of` with the width as an immediate, for the monomorphised kernels.
    #[inline(always)]
    pub fn row_of_c<const RL: u32>(&self, h: usize) -> usize {
        debug_assert_eq!(RL, self.row_log);
        (h >> RL) & self.row_mask
    }

    /// W33: the row mask, for callers that insert in a LOOP.
    ///
    /// `row_of` reads it from the struct on every call, which is right for a
    /// one-shot probe and wrong for the back-fill, where the mask is fixed for
    /// the whole run. Hoisting it turns a struct load per insert into one per
    /// fill site.
    #[inline(always)]
    pub fn mask(&self) -> usize {
        self.row_mask
    }

    /// W33/W34: insert by HASH with a caller-hoisted mask -- `row_of` plus
    /// `insert` in one call, so the loop neither re-loads the mask nor pays a
    /// second call boundary per position.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn insert_h<const RL: u32>(&mut self, h: usize, rmask: usize, ip: u32, tag: u8) {
        debug_assert_eq!(rmask, self.row_mask);
        debug_assert_eq!(RL, self.row_log);
        let r = (h >> RL) & rmask;
        debug_assert!(r < self.head.len());
        // SAFETY: `r <= row_mask == head.len() - 1`, and the table holds
        // `head.len() << RL` slots, `s < 1 << RL`.
        unsafe {
            let head = *self.head.get_unchecked(r) as usize;
            self.put(r, r << RL, head, RL, ip, tag);
        }
    }

    /// Insert `ip` under `tag` into row `r`, evicting the oldest entry. The
    /// width is read from the table: this is the MIRROR path (priming, the
    /// greedy fill), not the walk.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn insert(&mut self, r: usize, ip: u32, tag: u8) {
        debug_assert!(r < self.head.len());
        let rl = self.row_log;
        // SAFETY: `r < head.len()` and the table holds `head.len() << rl`
        // slots, so `(r << rl) + s` with `s < 1 << rl` is in bounds.
        unsafe {
            let head = *self.head.get_unchecked(r) as usize;
            self.put(r, r << rl, head, rl, ip, tag);
        }
    }

    /// The tags of row `r`.
    #[cfg(test)]
    #[allow(unsafe_code)]
    pub fn tag_row(&self, r: usize) -> &[u8] {
        let n = 1usize << self.row_log;
        assert!((r + 1) * n <= self.slots());
        // SAFETY: bounds asserted; the tag lines are contiguous bytes.
        unsafe { core::slice::from_raw_parts(self.tags_ptr().add(r * n), n) }
    }

    /// W16 -- the row's ENTIRE walk state in one call.
    ///
    /// This was two: `probe_mask` (tags -> mask, plus `head`) and `row_ref`
    /// (positions). Each derived the row offset and each loaded its own `Vec`
    /// base, so a probe paid the row-offset multiply twice. One call now
    /// resolves the offset once and returns everything the walk and the
    /// following insert need -- including `at`, so `insert_at` re-derives
    /// nothing (W17).
    ///
    /// Returns `(rotated mask, head, row positions, row element offset)`.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn probe_view<const RL: u32>(&self, r: usize, want: u8) -> (RowMask, u32, &[u32], usize) {
        debug_assert!(r < self.head.len());
        debug_assert_eq!(RL, self.row_log);
        // SAFETY: `r < head.len()`; the table holds `head.len() << RL` tags
        // and positions, so both `1 << RL`-element windows at `at` are in
        // bounds.
        let at = r << RL;
        let (mask, row, head) = unsafe {
            (
                row_tag_mask_raw::<RL>(self.tags_ptr().add(at), want),
                core::slice::from_raw_parts(self.pos_ptr().add(at), 1usize << RL),
                u32::from(*self.head.get_unchecked(r)),
            )
        };
        // W2's RECEIPT, computed from the same mask both forms consume, so one
        // run prices both without an A/B build. NEW cost is one iteration per
        // candidate (popcount). OLD cost was one iteration per SLOT VISITED.
        #[cfg(feature = "profile")]
        {
            use core::sync::atomic::Ordering::Relaxed;
            let n = 1u32 << RL;
            let w0 = row_rot::<RL>(mask, head);
            let newi = u64::from(mask.count_ones());
            let oldi = if w0 == 0 {
                1u64
            } else {
                u64::from((64 - w0.leading_zeros() + 1).min(n))
            };
            ROW_WALK[0].fetch_add(1, Relaxed);
            ROW_WALK[1].fetch_add(oldi, Relaxed);
            ROW_WALK[2].fetch_add(newi, Relaxed);
        }
        (row_rot::<RL>(mask, head), head, row, at)
    }

    /// W17/W18/W19 -- insert with everything the probe already resolved.
    ///
    /// `insert(r, ..)` re-derived the row offset (W17) and re-loaded `head`
    /// from the `head` array (W18) that `probe_view` had just read one line
    /// earlier -- a redundant load on the dependency path of two stores. The
    /// advanced head is computed from the value in hand (W19), so the only
    /// read left is the write-back itself.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn insert_at<const RL: u32>(&mut self, r: usize, at: usize, head: u32, ip: u32, tag: u8) {
        debug_assert!(r < self.head.len());
        debug_assert_eq!(RL, self.row_log);
        debug_assert_eq!(at, r << RL);
        debug_assert_eq!(head, u32::from(self.head[r]));
        debug_assert!(head < (1 << RL));
        // SAFETY: `at == r << RL`, `head < 1 << RL` and `r < head.len()`.
        unsafe { self.put(r, at, head as usize, RL, ip, tag) }
    }

    /// Candidate positions of row `r`, MOST RECENT FIRST.
    ///
    /// Ordering is not cosmetic: the finder takes the first acceptable match,
    /// and the most recent position is the SMALLEST offset, which is cheapest
    /// to code. Walking the row in slot order instead of insertion order
    /// inflates offsets.
    #[cfg(test)]
    #[allow(unsafe_code)]
    pub fn candidates(&self, r: usize, mut mask: RowMask, out: &mut [u32; ROW_MAX]) -> usize {
        let n = 1usize << self.row_log;
        let head = self.head[r] as usize;
        assert!((r + 1) * n <= self.slots());
        // SAFETY: bounds asserted; the position lines are contiguous `u32`s.
        let row = unsafe { core::slice::from_raw_parts(self.pos_ptr().add(r * n), n) };
        let mut cnt = 0usize;
        // Walk slots newest-first: head, head+1, ... wrapping.
        for k in 0..n {
            if mask == 0 {
                break;
            }
            let s = (head + k) & (n - 1);
            if mask & (1 << s) != 0 {
                mask &= !(1u64 << s);
                out[cnt] = row[s];
                cnt += 1;
            }
        }
        cnt
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn kernel_vs_oracle<const RL: u32>() {
        let n = 1usize << RL;
        let mut buf = [0u8; ROW_MAX];
        let tags = &mut buf[..n];
        for seed in 0u32..512 {
            let mut x = seed.wrapping_mul(2654435761).wrapping_add(1);
            for t in tags.iter_mut() {
                x ^= x << 13;
                x ^= x >> 17;
                x ^= x << 5;
                // Narrow the alphabet on some seeds so collisions are dense.
                *t = if seed % 3 == 0 {
                    (x & 3) as u8
                } else {
                    x as u8
                };
            }
            for want in [0u8, 1, 2, 3, 0x5A, 0xFF] {
                assert_eq!(
                    row_tag_mask::<RL>(tags, want),
                    row_tag_mask_oracle(tags, want),
                    "width {n} seed {seed} want {want}"
                );
            }
        }
        let all = [7u8; ROW_MAX];
        let full = if n == 64 { u64::MAX } else { (1u64 << n) - 1 };
        assert_eq!(row_tag_mask::<RL>(&all[..n], 7), full);
        assert_eq!(row_tag_mask::<RL>(&all[..n], 8), 0);
        // Exactly one lane, walked across every position, to pin lane ORDER.
        for i in 0..n {
            let mut t = [0u8; ROW_MAX];
            t[i] = 0x99;
            assert_eq!(
                row_tag_mask::<RL>(&t[..n], 0x99),
                1 << i,
                "width {n} lane {i}"
            );
        }
    }

    /// The vector kernel must agree with the oracle on every pattern that
    /// matters, including all-match and no-match -- the two the movemask lane
    /// order gets wrong when it is wrong -- at every row width.
    #[test]
    fn kernel_matches_oracle() {
        kernel_vs_oracle::<4>();
        kernel_vs_oracle::<5>();
        kernel_vs_oracle::<6>();
    }

    fn rot_check<const RL: u32>() {
        let n = 1u32 << RL;
        for head in 0..n {
            // Rows fill downward: the newest slot is `head`, the oldest
            // `head - 1` (mod n).
            let oldest = (head + n - 1) & (n - 1);
            assert_eq!(
                row_rot::<RL>(1 << oldest, head),
                1 << (n - 1),
                "n {n} head {head}"
            );
            assert_eq!(row_rot::<RL>(1 << head, head), 1, "n {n} head {head}");
        }
    }

    /// The rotation puts the NEWEST slot on bit 0 for every head.
    #[test]
    fn rot_puts_newest_on_bit0() {
        rot_check::<4>();
        rot_check::<5>();
        rot_check::<6>();
    }

    fn ring_order<const RL: u32>() {
        let n = 1usize << RL;
        let mut t = RowTable::default();
        t.reset(10, RL);
        assert_eq!(t.row_log(), RL);
        let r = 3usize;
        for i in 0..n as u32 {
            t.insert(r, 100 + i, 0x42);
        }
        let mask = row_tag_mask::<RL>(t.tag_row(r), 0x42);
        assert_eq!(mask.count_ones() as usize, n);
        let mut out = [0u32; ROW_MAX];
        let cnt = t.candidates(r, mask, &mut out);
        assert_eq!(cnt, n);
        for (k, v) in out[..n].iter().enumerate() {
            assert_eq!(*v, 100 + (n - 1 - k) as u32, "slot {k}");
        }
        // Overfill by one: the oldest (100) is evicted, newest is 200.
        t.insert(r, 200, 0x42);
        let cnt = t.candidates(r, row_tag_mask::<RL>(t.tag_row(r), 0x42), &mut out);
        assert_eq!(cnt, n);
        assert_eq!(out[0], 200);
        assert!(!out[..cnt].contains(&100), "oldest not evicted");
        // The monomorphised probe agrees with the test walk: bit 0 = newest.
        let (w, head, row, at) = t.probe_view::<RL>(r, 0x42);
        assert_eq!(at, r << RL);
        let b = w.trailing_zeros();
        let s = ((b + head) & ((1u32 << RL) - 1)) as usize;
        assert_eq!(row[s], 200);
    }

    /// Insertion must evict oldest-first and report newest-first, or the
    /// finder prefers far offsets over near ones -- at every row width.
    #[test]
    fn ring_order_is_newest_first() {
        ring_order::<4>();
        ring_order::<5>();
        ring_order::<6>();
    }

    fn mixed_tags<const RL: u32>() {
        let n = 1u32 << RL;
        let mut t = RowTable::default();
        t.reset(8 + RL, RL);
        let r = t.row_of(0x37 << RL);
        assert_eq!(r, t.row_of_c::<RL>(0x37 << RL));
        for i in 0..n {
            t.insert(r, 1000 + i, if i % 2 == 0 { 0xAA } else { 0xBB });
        }
        let mut out = [0u32; ROW_MAX];
        let cnt = t.candidates(r, row_tag_mask::<RL>(t.tag_row(r), 0xAA), &mut out);
        assert_eq!(cnt as u32, n / 2);
        for (k, v) in out[..cnt].iter().enumerate() {
            assert_eq!(*v, 1000 + (n - 2 - 2 * k as u32), "slot {k}");
        }
    }

    /// Only matching tags are reported, newest-first.
    #[test]
    fn mixed_tags_filter() {
        mixed_tags::<4>();
        mixed_tags::<5>();
        mixed_tags::<6>();
    }
}
