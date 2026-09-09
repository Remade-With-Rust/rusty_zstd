//! Thread-local `Vec` recycling for per-block scratch buffers.
//!
//! ## Why this exists
//!
//! The allocation census (`docs/plans/allocation-census.md`) measured the
//! encoder at **596-747 allocations per MiB against the decoder's 1.4-2.0** --
//! a 400-500x ratio -- and differencing input sizes put it at **~78 allocations
//! per 128 KiB block, essentially unchanged from L1 to L19**. Level-independence
//! is the tell: the finder ladder changes completely across that range and the
//! count does not move, so the cost is in the shared per-block entropy path, not
//! in match finding.
//!
//! The decoder does not have this problem because it already recycles (W25/W26).
//! This is the encoder's equivalent, in the smallest form that needs no
//! signature changes anywhere.
//!
//! ## The shape
//!
//! A [`Lease`] takes a `Vec` out of a thread-local slot, derefs to it, and puts
//! it back on drop -- including on the `?` early-returns that make hand-written
//! take/restore pairs leak the buffer. The buffer keeps its capacity, so the
//! second and every later block reuses the same allocation.
//!
//! `lease()` hands back an EMPTY vec (length 0, capacity retained). Callers fill
//! it exactly as they filled the fresh `Vec` they used to build, so the contents
//! -- and therefore the bitstream -- are identical. **Nothing here may change an
//! output byte**; `examples/bytegate.rs` is the gate.
//!
//! ## Why thread-local rather than a scratch struct on `MatchTables`
//!
//! A `BlockScratch` field threaded through every entropy function is the tidier
//! design and it is a large signature refactor touching `fse.rs`, `huffman.rs`
//! and `encode.rs` at once. This gets the same allocations back for a two-line
//! change per site, and each site is independently revertible. If the plumbing
//! is ever unified, these leases collapse into it without changing behaviour.
//!
//! MT safety is free: each worker thread gets its own slots, so there is no
//! sharing and no lock. A thread that encodes one block keeps its buffers for
//! the next one.

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// A `Vec` borrowed from a thread-local slot, returned on drop.
///
/// Deref/DerefMut to `Vec<T>`, so a site changes from
/// `let mut v = vec![0u16; n];` to `let mut v = scratch::lease(&SLOT); v.resize(n, 0);`
/// and nothing else moves.
#[cfg(all(feature = "std", feature = "alloc"))]
pub(crate) struct Lease<T: 'static> {
    v: Vec<T>,
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<T>>>,
}

#[cfg(all(feature = "std", feature = "alloc"))]
impl<T: 'static> core::ops::Deref for Lease<T> {
    type Target = Vec<T>;
    #[inline(always)]
    fn deref(&self) -> &Vec<T> {
        &self.v
    }
}

#[cfg(all(feature = "std", feature = "alloc"))]
impl<T: 'static> core::ops::DerefMut for Lease<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.v
    }
}

#[cfg(all(feature = "std", feature = "alloc"))]
impl<T: 'static> Drop for Lease<T> {
    #[inline(always)]
    fn drop(&mut self) {
        // Hand the allocation back, keeping whichever buffer has more capacity.
        // A slot can be non-empty if this call RECURSED (an inner lease of the
        // same slot took the empty vec, filled it, and returned first) -- so
        // never blindly overwrite, or the larger allocation is the one dropped.
        let mine = core::mem::take(&mut self.v);
        let _ = self.slot.try_with(|c| {
            if let Ok(mut cur) = c.try_borrow_mut() {
                // Normal case: `lease` emptied the slot, so take it back. The
                // capacity test only matters under RECURSION, where an inner
                // lease of the same slot already returned its buffer -- then
                // keep whichever is larger rather than dropping the bigger one.
                //
                // Comparing `capacity()` alone was wrong for a POOL
                // (`Vec<Vec<u8>>`): the outer capacity is the slot count, equal
                // on both sides, so the pool was discarded every time and the
                // four inner buffers with it.
                if cur.is_empty() || mine.capacity() > cur.capacity() {
                    *cur = mine;
                }
            }
        });
    }
}

/// Take the thread-local buffer for `slot`, emptied and ready to fill.
///
/// Returns a `Vec` with length 0 and whatever capacity the previous user left.
/// On the first call of a thread, or if the slot is currently leased by an outer
/// frame, this is a fresh empty `Vec` -- correct either way, just not recycled.
#[cfg(all(feature = "std", feature = "alloc"))]
#[inline(always)]
pub(crate) fn lease<T: 'static>(
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<T>>>,
) -> Lease<T> {
    let v = slot
        .try_with(|c| {
            c.try_borrow_mut()
                .map(|mut b| core::mem::take(&mut *b))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    let mut v = v;
    v.clear();
    Lease { v, slot }
}

/// Like [`lease`] but does NOT clear -- for a POOL whose elements are themselves
/// the thing being recycled (`Vec<Vec<u8>>`).
///
/// `lease` clears, which is right for a scratch buffer and destroys a pool:
/// `clear()` drops every inner `Vec` and with it every allocation the pool
/// exists to keep. Callers of this must treat the contents as arbitrary
/// leftovers and overwrite what they use.
#[cfg(all(feature = "std", feature = "alloc"))]
#[inline(always)]
pub(crate) fn lease_pool<T: 'static>(
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<T>>>,
) -> Lease<T> {
    let v = slot
        .try_with(|c| {
            c.try_borrow_mut()
                .map(|mut b| core::mem::take(&mut *b))
                .unwrap_or_default()
        })
        .unwrap_or_default();
    Lease { v, slot }
}

/// Declare a thread-local scratch slot.
///
/// ```ignore
/// scratch_slot!(TABLE_SYMBOL: u16);
/// let mut ts = scratch::lease(&TABLE_SYMBOL);
/// ts.resize(table_size, 0);
/// ```
#[cfg(all(feature = "std", feature = "alloc"))]
macro_rules! scratch_slot {
    ($name:ident : $ty:ty) => {
        thread_local! {
            static $name: core::cell::RefCell<alloc::vec::Vec<$ty>> =
                const { core::cell::RefCell::new(alloc::vec::Vec::new()) };
        }
    };
}

#[cfg(all(feature = "std", feature = "alloc"))]
pub(crate) use scratch_slot;

// ---------------------------------------------------------------------------
// no_std / alloc-only fallback: no thread-locals, so no recycling. Same API, so
// call sites are identical and a `no_std` build simply allocates as it did.
// ---------------------------------------------------------------------------

/// Non-recycling stand-in for `Lease` when there is no `std`.
#[cfg(all(not(feature = "std"), feature = "alloc"))]
pub(crate) struct Lease<T> {
    v: Vec<T>,
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
impl<T> core::ops::Deref for Lease<T> {
    type Target = Vec<T>;
    #[inline(always)]
    fn deref(&self) -> &Vec<T> {
        &self.v
    }
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
impl<T> core::ops::DerefMut for Lease<T> {
    #[inline(always)]
    fn deref_mut(&mut self) -> &mut Vec<T> {
        &mut self.v
    }
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
#[inline(always)]
pub(crate) fn lease<T>(_slot: &()) -> Lease<T> {
    Lease { v: Vec::new() }
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
#[inline(always)]
pub(crate) fn lease_pool<T>(_slot: &()) -> Lease<T> {
    Lease { v: Vec::new() }
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
macro_rules! scratch_slot {
    ($name:ident : $ty:ty) => {
        #[allow(dead_code)]
        static $name: () = ();
    };
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
pub(crate) use scratch_slot;

// ---------------------------------------------------------------------------
// Bounded free list, for buffers that ESCAPE their constructor.
//
// `lease` covers scratch that dies where it was born. A buffer that is returned
// to a caller cannot use it -- but if the caller eventually drops the buffer
// (copies it into the output, compares it and discards it), the loop can still
// be closed by hand: `pool_take` at the constructor, `pool_give` wherever the
// value actually dies.
//
// **The return path is the whole thing.** ALLOC-13 pooled the literal-section
// candidates and measured EXACTLY ZERO improvement, because the winning section
// crossed a module boundary and was dropped there; nothing came back and every
// caller drew from an empty pool. Always ask where the value dies, not where it
// is made.
// ---------------------------------------------------------------------------

/// Take a buffer from a bounded thread-local free list, emptied.
#[cfg(all(feature = "std", feature = "alloc"))]
#[inline]
pub(crate) fn pool_take<T: 'static>(
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<Vec<T>>>>,
) -> Vec<T> {
    let got = slot
        .try_with(|c| c.try_borrow_mut().ok().and_then(|mut p| p.pop()))
        .ok()
        .flatten();
    // A pool that MISSES allocates, so its hit rate is the whole question:
    // a `pool_take` that returns `None` is indistinguishable at the call site
    // from never having pooled at all.
    #[cfg(feature = "profile")]
    {
        use core::sync::atomic::Ordering::Relaxed;
        if got.is_some() {
            POOL_HIT.fetch_add(1, Relaxed);
        } else {
            POOL_MISS.fetch_add(1, Relaxed);
        }
    }
    let mut v = got.unwrap_or_default();
    v.clear();
    v
}

/// Record a take on a pool that keeps its OWN free list (`fse::ct_pool`),
/// so one census covers every pool in the crate rather than just this one.
#[cfg(feature = "profile")]
#[inline]
pub(crate) fn note_pool(hit: bool) {
    use core::sync::atomic::Ordering::Relaxed;
    if hit {
        POOL_HIT.fetch_add(1, Relaxed);
    } else {
        POOL_MISS.fetch_add(1, Relaxed);
    }
}

/// Pool hit/miss census. A miss is an allocation.
#[cfg(feature = "profile")]
pub static POOL_HIT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Pool misses -- each one is a fresh allocation.
#[cfg(feature = "profile")]
pub static POOL_MISS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Buffers handed back with real capacity.
#[cfg(feature = "profile")]
pub static POOL_GIVE: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Buffers handed back with ZERO capacity -- never pooled, so the matching
/// take must allocate. A take/give imbalance shows up here first.
#[cfg(feature = "profile")]
pub static POOL_GIVE_EMPTY: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Buffers DROPPED because the free list was full.
#[cfg(feature = "profile")]
pub static POOL_DROP: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the pool census: `(hits, misses, drops, gives, give_empty)`.
#[cfg(feature = "profile")]
pub fn take_pool_census() -> (u64, u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        POOL_HIT.swap(0, Relaxed),
        POOL_MISS.swap(0, Relaxed),
        POOL_DROP.swap(0, Relaxed),
        POOL_GIVE.swap(0, Relaxed),
        POOL_GIVE_EMPTY.swap(0, Relaxed),
    )
}

/// How many buffers one slot's free list holds.
///
/// This was a bare `6` and it was STARVING the pool: a block hands back three
/// ncount headers plus their losing candidates, and anything past the cap is
/// dropped -- freed, so the next `pool_take` that wants it allocates. Measured
/// at 6: **78.2% hit rate, 1,437 misses and 708 drops** over four corpora.
///
/// REFUTED TWICE, recorded. Raising it to 32 changed hits and misses NOT AT
/// ALL -- first at a 78.2% hit rate (5,167 / 1,437 both ways), and again after
/// two `SC_NORM` leaks were fixed and the rate rose to 89.1% (5,885 / 719 at
/// caps 6, 12 and 32 alike). Only drops move. The misses are first-use per
/// slot and genuine concurrent liveness, not capacity.
pub(crate) const POOL_CAP: usize = 6;

/// Return a buffer to a bounded thread-local free list.
#[cfg(all(feature = "std", feature = "alloc"))]
#[inline]
pub(crate) fn pool_give<T: 'static>(
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<Vec<T>>>>,
    v: Vec<T>,
) {
    if v.capacity() == 0 {
        // A zero-capacity vec is not a buffer -- returning it would put an
        // empty shell in the pool that the next take has to grow anyway.
        #[cfg(feature = "profile")]
        POOL_GIVE_EMPTY.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return;
    }
    #[cfg(feature = "profile")]
    POOL_GIVE.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    let _ = slot.try_with(|c| {
        if let Ok(mut p) = c.try_borrow_mut() {
            if p.len() < POOL_CAP {
                p.push(v);
            } else {
                // Dropped: the free list is full, so this buffer is freed and
                // the next `pool_take` that wants it will allocate.
                #[cfg(feature = "profile")]
                POOL_DROP.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            }
        }
    });
}

/// Declare a bounded free-list slot (a pool of buffers, not one buffer).
#[cfg(all(feature = "std", feature = "alloc"))]
macro_rules! pool_slot {
    ($name:ident : $ty:ty) => {
        thread_local! {
            static $name: core::cell::RefCell<alloc::vec::Vec<alloc::vec::Vec<$ty>>> =
                const { core::cell::RefCell::new(alloc::vec::Vec::new()) };
        }
    };
}

#[cfg(all(feature = "std", feature = "alloc"))]
pub(crate) use pool_slot;

#[cfg(all(not(feature = "std"), feature = "alloc"))]
#[inline]
pub(crate) fn pool_take<T>(_slot: &()) -> Vec<T> {
    Vec::new()
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
#[inline]
pub(crate) fn pool_give<T>(_slot: &(), _v: Vec<T>) {}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
macro_rules! pool_slot {
    ($name:ident : $ty:ty) => {
        #[allow(dead_code)]
        static $name: () = ();
    };
}

#[cfg(all(not(feature = "std"), feature = "alloc"))]
pub(crate) use pool_slot;
