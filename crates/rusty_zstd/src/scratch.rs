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
    let mut v = slot
        .try_with(|c| c.try_borrow_mut().ok().and_then(|mut p| p.pop()))
        .ok()
        .flatten()
        .unwrap_or_default();
    v.clear();
    v
}

/// Return a buffer to a bounded thread-local free list (cap 6).
#[cfg(all(feature = "std", feature = "alloc"))]
#[inline]
pub(crate) fn pool_give<T: 'static>(
    slot: &'static std::thread::LocalKey<core::cell::RefCell<Vec<Vec<T>>>>,
    v: Vec<T>,
) {
    if v.capacity() == 0 {
        return;
    }
    let _ = slot.try_with(|c| {
        if let Ok(mut p) = c.try_borrow_mut() {
            if p.len() < 6 {
                p.push(v);
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
