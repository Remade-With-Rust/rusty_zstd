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

/// Positions per row. 16 = one SSE register of tags, one `pmovmskb`.
pub(crate) const ROW: usize = 16;

/// W2's walk census: `[probes, OLD slot-visits, NEW slot-visits]`. Both cost
/// models are evaluated from the SAME mask in one run, so the comparison needs
/// no A/B build and carries no clock.
#[cfg(feature = "profile")]
pub static ROW_WALK: [core::sync::atomic::AtomicU64; 3] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
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

/// Tag-scan ORACLE: which of `tags`' 16 lanes equal `want`, as a bitmask.
///
/// Stays in the tree as the correctness reference and the non-x86/ARM path.
/// Dead on x86_64/aarch64 by construction -- the vector kernel is used
/// there. It stays compiled so the unit tests can hold it against the
/// kernel, and it IS the implementation on every other arch.
#[cfg_attr(any(target_arch = "x86_64", target_arch = "aarch64"), allow(dead_code))]
#[inline]
pub(crate) fn row_tag_mask_oracle(tags: &[u8; ROW], want: u8) -> u16 {
    let mut m = 0u16;
    for (i, &t) in tags.iter().enumerate() {
        if t == want {
            m |= 1 << i;
        }
    }
    m
}

/// Tag-scan KERNEL: 16 lanes compared in one instruction.
#[inline(always)]
#[allow(unsafe_code)]
pub(crate) fn row_tag_mask(tags: &[u8; ROW], want: u8) -> u16 {
    #[cfg(target_arch = "x86_64")]
    {
        // SSE2 is baseline on x86_64, so this needs no runtime detect and no
        // `#[target_feature]` -- which matters, because a `target_feature`
        // helper becomes a CALL from a baseline caller and the call would cost
        // more than the compare saves (codec-vectorize-kernel, Law 1).
        //
        // SAFETY: `tags.len() == ROW == 16`, so a 16-byte unaligned load is in
        // bounds. `loadu` has no alignment requirement.
        unsafe {
            use core::arch::x86_64::*;
            let v = _mm_loadu_si128(tags.as_ptr() as *const __m128i);
            let w = _mm_set1_epi8(want as i8);
            _mm_movemask_epi8(_mm_cmpeq_epi8(v, w)) as u16
        }
    }
    #[cfg(target_arch = "aarch64")]
    {
        // NEON has no `pmovmskb`. The standard replacement: compare, AND with a
        // per-lane bit weight, then reduce each half. This is the shape every
        // NEON codec uses for movemask.
        //
        // SAFETY: `tags.len() == ROW == 16`, so `vld1q_u8` reads in bounds.
        unsafe {
            use core::arch::aarch64::*;
            let v = vld1q_u8(tags.as_ptr());
            let w = vdupq_n_u8(want);
            let eq = vceqq_u8(v, w);
            const BITS: [u8; 16] = [1, 2, 4, 8, 16, 32, 64, 128, 1, 2, 4, 8, 16, 32, 64, 128];
            let sel = vandq_u8(eq, vld1q_u8(BITS.as_ptr()));
            let lo = vaddv_u8(vget_low_u8(sel)) as u16;
            let hi = vaddv_u8(vget_high_u8(sel)) as u16;
            lo | (hi << 8)
        }
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    {
        row_tag_mask_oracle(tags, want)
    }
}

/// The row table: `ROW` positions per row with their tags contiguous.
///
/// Sized to exactly the chain it replaces: `1 << hash_log` total entries.
#[cfg(feature = "alloc")]
#[derive(Default, Clone)]
pub(crate) struct RowTable {
    /// `rows * ROW` positions.
    pub pos: alloc::vec::Vec<u32>,
    /// `rows * ROW` tags, so each row's 16 tags are one load.
    pub tags: alloc::vec::Vec<u8>,
    /// Next slot to write, per row. Wraps at `ROW`, so a row always holds the
    /// most recent `ROW` positions for its bucket.
    pub head: alloc::vec::Vec<u8>,
    /// W1: `rows - 1`, cached. `row_of` ran on every position and derived this
    /// from `head.len()` -- a Vec field load feeding the address computation of
    /// the very next load, i.e. on the dependency path.
    row_mask: usize,
}

#[cfg(feature = "alloc")]
impl RowTable {
    /// Allocate (or resize and clear) for a `1 << hash_log` entry budget.
    pub fn reset(&mut self, hash_log: u32) {
        let entries = 1usize << hash_log.clamp(6, 24);
        let rows = entries / ROW;
        self.pos.clear();
        self.pos.resize(rows * ROW, 0);
        self.tags.clear();
        self.tags.resize(rows * ROW, 0);
        self.head.clear();
        self.head.resize(rows, 0);
        // Same value the old `head.len().wrapping_sub(1)` produced, including
        // the empty-table case.
        self.row_mask = rows.wrapping_sub(1);
    }

    /// Row index for hash bucket `h`. 16 buckets share a row, which is what
    /// keeps this table the same size as the chain.
    #[inline(always)]
    pub fn row_of(&self, h: usize) -> usize {
        debug_assert_eq!(self.row_mask, self.head.len().wrapping_sub(1));
        (h >> 4) & self.row_mask
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
    pub fn insert_h(&mut self, h: usize, rmask: usize, ip: u32, tag: u8) {
        debug_assert_eq!(rmask, self.row_mask);
        let r = (h >> 4) & rmask;
        debug_assert!(r < self.head.len());
        // SAFETY: identical to `insert` -- `r <= row_mask == head.len() - 1`,
        // and `pos`/`tags` are `head.len() * ROW` long.
        unsafe {
            let s = *self.head.get_unchecked(r) as usize;
            debug_assert!(s < ROW);
            let at = r * ROW + s;
            *self.pos.get_unchecked_mut(at) = ip;
            *self.tags.get_unchecked_mut(at) = tag;
            *self.head.get_unchecked_mut(r) = ((s + 1) & (ROW - 1)) as u8;
        }
    }

    /// Insert `ip` under `tag` into row `r`, evicting the oldest entry.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn insert(&mut self, r: usize, ip: u32, tag: u8) {
        debug_assert!(r < self.head.len());
        // SAFETY: `r < head.len()` and `pos`/`tags` are `head.len() * ROW`
        // long, so `r * ROW + s` with `s < ROW` is in bounds for both.
        unsafe {
            let s = *self.head.get_unchecked(r) as usize;
            debug_assert!(s < ROW);
            let at = r * ROW + s;
            *self.pos.get_unchecked_mut(at) = ip;
            *self.tags.get_unchecked_mut(at) = tag;
            *self.head.get_unchecked_mut(r) = ((s + 1) & (ROW - 1)) as u8;
        }
    }

    /// The 16 tags of row `r`, as a statically-sized array (W5: a slice here
    /// carried a runtime length the kernel then re-asserted).
    #[inline(always)]
    #[allow(unsafe_code)]
    #[cfg(test)]
    pub fn tag_row(&self, r: usize) -> &[u8; ROW] {
        debug_assert!(r < self.head.len());
        // SAFETY: `tags` is `head.len() * ROW` long and `r < head.len()`, so
        // the ROW-byte window at `r * ROW` is in bounds and `[u8; ROW]` has
        // the same layout as those ROW bytes.
        unsafe { &*(self.tags.as_ptr().add(r * ROW) as *const [u8; ROW]) }
    }

    /// W16 -- the row's ENTIRE walk state in one call.
    ///
    /// This was two: `probe_mask` (tags -> mask, plus `head`) and `row_ref`
    /// (positions). Each derived `r * ROW` and each loaded its own `Vec` base,
    /// so a probe paid the row-offset multiply twice. One call now resolves the
    /// offset once and returns everything the walk and the following insert
    /// need -- including `at`, so `insert_at` re-derives nothing (W17).
    ///
    /// Returns `(rotated mask, head, row positions, row byte offset)`.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn probe_view(&self, r: usize, want: u8) -> (u16, u32, &[u32; ROW], usize) {
        debug_assert!(r < self.head.len());
        // SAFETY: `r < head.len()`; `tags` and `pos` are both
        // `head.len() * ROW` long, so both ROW-element windows at `at` are in
        // bounds and `[T; ROW]` matches the layout of those elements.
        let at = r * ROW;
        let (tags, row, head) = unsafe {
            (
                &*(self.tags.as_ptr().add(at) as *const [u8; ROW]),
                &*(self.pos.as_ptr().add(at) as *const [u32; ROW]),
                u32::from(*self.head.get_unchecked(r)),
            )
        };
        let mask = row_tag_mask(tags, want);
        // W2's RECEIPT, computed from the same mask both forms consume, so one
        // run prices both without an A/B build. NEW cost is one iteration per
        // candidate (popcount). OLD cost was one iteration per SLOT VISITED.
        #[cfg(feature = "profile")]
        {
            use core::sync::atomic::Ordering::Relaxed;
            let w0 = mask.rotate_right(head);
            let newi = u64::from(mask.count_ones());
            let oldi = if w0 == 0 {
                1u64
            } else {
                u64::from((ROW as u32 - w0.trailing_zeros() + 1).min(ROW as u32))
            };
            ROW_WALK[0].fetch_add(1, Relaxed);
            ROW_WALK[1].fetch_add(oldi, Relaxed);
            ROW_WALK[2].fetch_add(newi, Relaxed);
        }
        (mask.rotate_right(head), head, row, at)
    }

    /// W17/W18/W19 -- insert with everything the probe already resolved.
    ///
    /// `insert(r, ..)` re-derived `r * ROW` (W17) and re-loaded `head` from the
    /// `head` array (W18) that `probe_view` had just read one line earlier --
    /// a redundant load on the dependency path of two stores. The advanced
    /// head is computed from the value in hand (W19), so the only read left is
    /// the write-back itself.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn insert_at(&mut self, r: usize, at: usize, head: u32, ip: u32, tag: u8) {
        debug_assert!(r < self.head.len());
        debug_assert_eq!(at, r * ROW);
        debug_assert_eq!(head, u32::from(self.head[r]));
        debug_assert!((head as usize) < ROW);
        // SAFETY: `at + head < (r + 1) * ROW <= pos.len() == tags.len()`, and
        // `r < head.len()`.
        unsafe {
            let slot = at + head as usize;
            *self.pos.get_unchecked_mut(slot) = ip;
            *self.tags.get_unchecked_mut(slot) = tag;
            *self.head.get_unchecked_mut(r) = ((head + 1) & (ROW as u32 - 1)) as u8;
        }
    }

    /// Candidate positions of row `r`, MOST RECENT FIRST.
    ///
    /// Ordering is not cosmetic: the finder takes the first acceptable match,
    /// and the most recent position is the SMALLEST offset, which is cheapest
    /// to code. Walking the row in slot order instead of insertion order
    /// inflates offsets.
    #[cfg(test)]
    #[inline(always)]
    #[allow(unsafe_code)]
    pub fn candidates(&self, r: usize, mut mask: u16, out: &mut [u32; ROW]) -> usize {
        debug_assert!(r < self.head.len());
        // SAFETY: `r < head.len()`, and every index below is `r * ROW + s`
        // with `s < ROW`, inside `pos`'s `head.len() * ROW` elements.
        let head = unsafe { *self.head.get_unchecked(r) } as usize;
        let mut n = 0usize;
        // Walk slots newest-first: head-1, head-2, ... wrapping.
        for k in 1..=ROW {
            if mask == 0 {
                break;
            }
            let s = (head + ROW - k) & (ROW - 1);
            if mask & (1 << s) != 0 {
                mask &= !(1u16 << s);
                // SAFETY: see above.
                unsafe {
                    *out.get_unchecked_mut(n) = *self.pos.get_unchecked(r * ROW + s);
                }
                n += 1;
            }
        }
        n
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The vector kernel must agree with the oracle on every pattern that
    /// matters, including all-match and no-match -- the two the movemask lane
    /// order gets wrong when it is wrong.
    #[test]
    fn kernel_matches_oracle() {
        let mut tags = [0u8; ROW];
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
                    row_tag_mask(&tags, want),
                    row_tag_mask_oracle(&tags, want),
                    "seed {seed} want {want}"
                );
            }
        }
        let all = [7u8; ROW];
        assert_eq!(row_tag_mask(&all, 7), 0xFFFF);
        assert_eq!(row_tag_mask(&all, 8), 0);
        // Exactly one lane, walked across every position, to pin lane ORDER.
        for i in 0..ROW {
            let mut t = [0u8; ROW];
            t[i] = 0x99;
            assert_eq!(row_tag_mask(&t, 0x99), 1 << i, "lane {i}");
        }
    }

    /// Insertion must evict oldest-first and report newest-first, or the
    /// finder prefers far offsets over near ones.
    #[test]
    fn ring_order_is_newest_first() {
        let mut t = RowTable::default();
        t.reset(10);
        let r = 3usize;
        for i in 0..ROW as u32 {
            t.insert(r, 100 + i, 0x42);
        }
        let mask = row_tag_mask(t.tag_row(r), 0x42);
        assert_eq!(mask, 0xFFFF);
        let mut out = [0u32; ROW];
        let n = t.candidates(r, mask, &mut out);
        assert_eq!(n, ROW);
        for (k, v) in out.iter().enumerate() {
            assert_eq!(*v, 100 + (ROW - 1 - k) as u32, "slot {k}");
        }
        // Overfill by one: the oldest (100) is evicted, newest is 200.
        t.insert(r, 200, 0x42);
        let n = t.candidates(r, row_tag_mask(t.tag_row(r), 0x42), &mut out);
        assert_eq!(n, ROW);
        assert_eq!(out[0], 200);
        assert!(!out[..n].contains(&100), "oldest not evicted");
    }

    /// Only matching tags are reported, newest-first.
    #[test]
    fn mixed_tags_filter() {
        let mut t = RowTable::default();
        t.reset(8);
        let r = t.row_of(0x37);
        for i in 0..ROW as u32 {
            t.insert(r, 1000 + i, if i % 4 == 0 { 0xAB } else { 0x11 });
        }
        let mut out = [0u32; ROW];
        let n = t.candidates(r, row_tag_mask(t.tag_row(r), 0xAB), &mut out);
        assert_eq!(n, 4);
        assert_eq!(&out[..4], &[1012, 1008, 1004, 1000]);
    }
}
