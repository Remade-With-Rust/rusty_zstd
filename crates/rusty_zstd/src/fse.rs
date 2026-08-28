//! FSE table description, build, and symbol decode (RFC 8878 section 4.1).

use crate::bit::{BitFwd, BitRev};
use crate::error::Error;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// W4: `align(4)` so the whole entry is one naturally-aligned u32. Without it
/// the struct is align-2 and `entry_u32` must use `read_unaligned`, which LLVM
/// will not always fold into an addressing mode.
#[derive(Clone, Copy, Debug)]
#[repr(C, align(4))]
pub(crate) struct FseEntry {
    /// C `FSE_decode_t.newState`.
    pub baseline: u16,
    pub symbol: u8,
    pub num_bits: u8,
}

/// A borrowed, loop-invariant handle on a built FSE decode table: the base
/// pointer and the index mask, both already resolved. Copy, so it lives in
/// registers across the sequence loop.
#[derive(Clone, Copy)]
pub(crate) struct FseView<'a> {
    ptr: *const FseEntry,
    mask: usize,
    _m: core::marker::PhantomData<&'a [FseEntry]>,
}

/// D6 probe: `[dtable builds, builds with NO low-probability symbols]`.
/// D6's zstd-parity fast path (write 8 symbols at a time as a broadcast u64)
/// applies only when `high_threshold == table_size - 1`. This counts how often
/// that is true, before anything is built.
#[cfg(feature = "profile")]
pub static D6_SPREAD: [core::sync::atomic::AtomicU64; 3] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];
/// Read and clear the D6 probe.
#[cfg(feature = "profile")]
pub fn take_d6_spread() -> [u64; 3] {
    use core::sync::atomic::Ordering;
    [
        D6_SPREAD[0].swap(0, Ordering::Relaxed),
        D6_SPREAD[1].swap(0, Ordering::Relaxed),
        D6_SPREAD[2].swap(0, Ordering::Relaxed),
    ]
}

impl FseView<'_> {
    /// Same invariant as `FseTable::entry`: the table is a non-empty power of
    /// two (both constructors guarantee it), and `mask == len - 1`, so the
    /// masked index is always in range.
    #[inline(always)]
    #[allow(unsafe_code)]
    #[allow(dead_code)] // the `FseView` twin of `FseTable::entry`; kept as its oracle.
    pub(crate) fn entry(self, state: u16) -> FseEntry {
        // SAFETY: `mask == len - 1` for a non-empty power-of-two table, so
        // `state & mask < len`; `ptr` borrows that same live allocation.
        unsafe { *self.ptr.add((state as usize) & self.mask) }
    }

    /// WIN 2 -- one 4-byte load instead of three field loads.
    ///
    /// `FseEntry` is `#[repr(C)]` {baseline: u16, symbol: u8, num_bits: u8} = 4
    /// bytes, and the emitted asm reads it as THREE loads (movzwl +0, movzbl +2,
    /// movzbl +3) -- 3 per table, 9 per sequence. Reading the whole entry as one
    /// `u32` and extracting in registers makes it one load.
    /// DECSEQ-II CUT 3 -- no mask. Three AND instructions per sequence (LL, ML,
    /// OF) re-proved a CONSTRUCTION invariant:
    ///
    /// * `init_state` reads `accuracy_log` bits, so `state < 2^log == len`.
    /// * `advance_w` produces `baseline + add` where `from_norm_buf` sets
    ///   `baseline = (ns << nb) - len` with `nb = log - highbit(ns)` and
    ///   `ns >= 1`, `highbit(ns) <= log` ENFORCED on every entry (it rejects
    ///   `next_state == 0` and `hb > log` outright, hostile tables included).
    ///   With `ns in [2^hb, 2^(hb+1))`: `ns << nb <= 2*len - 2^nb`, so
    ///   `baseline + (2^nb - 1) <= len - 1`. Every reachable state is in range.
    /// * `rle` builds `accuracy_log == 0`, len 1: the state is 0 forever.
    ///
    /// The mask stays in the struct (and in `entry`, the oracle) and the
    /// invariant is asserted here in debug builds.
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn entry_u32(self, state: u16) -> u32 {
        debug_assert!((state as usize) <= self.mask, "FSE state out of table");
        // SAFETY: `state <= mask == len - 1` by the construction invariant
        // proven above, so the pointer is inside the live table allocation.
        unsafe {
            let p = self.ptr.add(state as usize);
            // SAFETY: `FseEntry` is repr(C, align(4)) and 4 bytes, so `p` is a
            // validly-aligned u32 address holding 4 initialised bytes.
            *p.cast::<u32>()
        }
    }
}

/// Field extraction for the packed form (`FseView::entry_u32`). Layout is
/// `repr(C)` little-endian: baseline in bits 0..16, symbol 16..24, nb 24..32.
#[inline(always)]
pub(crate) const fn fse_baseline(w: u32) -> u16 {
    w as u16
}
#[inline(always)]
pub(crate) const fn fse_symbol(w: u32) -> u8 {
    (w >> 16) as u8
}
#[inline(always)]
pub(crate) const fn fse_nbits(w: u32) -> u8 {
    (w >> 24) as u8
}

#[derive(Clone, Debug)]
pub(crate) struct FseTable {
    pub decode: Vec<FseEntry>,
    pub accuracy_log: u8,
}

impl FseTable {
    pub(crate) fn rle(symbol: u16) -> Self {
        Self {
            decode: vec![FseEntry {
                baseline: 0,
                symbol: symbol as u8,
                num_bits: 0,
            }],
            accuracy_log: 0,
        }
    }

    /// W26 -- build into a RECYCLED buffer.
    ///
    /// W25 removed the Repeat-mode clone; Compressed mode -- **51.3% of table
    /// selections at L3** -- still allocated a fresh `Vec<FseEntry>` per table
    /// per block. Since `seq_table` now takes the previous table BY VALUE, its
    /// allocation is right there: `resize` reuses the buffer whenever it is
    /// already large enough, which it is for every block after the first at a
    /// given accuracy log.
    // REFUTED, recorded: `#[inline(never)]` here measured NEUTRAL (+5
    // instructions). Unlike `select_seq_table`, this has few inline sites, so
    // there was no duplication to remove -- LLVM had already made the call.
    #[inline(always)]
    pub(crate) fn from_norm_into(
        recycle: Option<Self>,
        norm: &[i16],
        accuracy_log: u8,
    ) -> Result<Self, Error> {
        match recycle {
            Some(mut t) => {
                Self::from_norm_buf(&mut t.decode, norm, accuracy_log)?;
                t.accuracy_log = accuracy_log;
                Ok(t)
            }
            None => Self::from_norm(norm, accuracy_log),
        }
    }

    #[inline(always)]
    pub(crate) fn from_norm(norm: &[i16], accuracy_log: u8) -> Result<Self, Error> {
        let mut decode = Vec::new();
        Self::from_norm_buf(&mut decode, norm, accuracy_log)?;
        Ok(Self {
            decode,
            accuracy_log,
        })
    }

    /// W26: the table build, writing into a caller-owned buffer. `resize` keeps
    /// the existing allocation whenever it is already large enough -- which it
    /// is for every block after the first at a given accuracy log.
    fn from_norm_buf(
        decode: &mut Vec<FseEntry>,
        norm: &[i16],
        accuracy_log: u8,
    ) -> Result<(), Error> {
        if !(5..=9).contains(&accuracy_log) {
            return Err(Error::Corruption);
        }
        let table_size = 1usize << accuracy_log;
        decode.clear();
        decode.resize(
            table_size,
            FseEntry {
                baseline: 0,
                symbol: 0,
                num_bits: 0,
            },
        );
        // W27 -- `symbol_next` on the STACK, not the heap.
        //
        // It was `vec![0u16; norm.len().max(1)]` -- a heap allocation per table
        // build, and the 64..127-byte size class was the single largest bucket
        // in the decode allocation census (3,530 of 14,871). `norm.len()` is
        // bounded: the spread loop below rejects `s > 255`, so 256 entries
        // always suffice and the buffer fits comfortably in a frame.
        //
        // `n_sym` preserves the EFFECTIVE length exactly, so the
        // `s >= symbol_next.len()` reject in the final pass keeps rejecting the
        // same symbols it did before -- a fixed 256-long array would silently
        // accept symbols outside the norm table.
        let n_sym = norm.len().max(1);
        if n_sym > 256 {
            return Err(Error::Corruption);
        }
        let mut symbol_next_buf = [0u16; 256];
        let symbol_next = &mut symbol_next_buf[..n_sym];
        let mut high_threshold = table_size - 1;

        for (s, &p) in norm.iter().enumerate() {
            if s > 255 {
                return Err(Error::Corruption);
            }
            let sym = s as u8;
            if p == -1 {
                if high_threshold == 0 && s > 0 {
                    return Err(Error::Corruption);
                }
                // T4: `high_threshold` starts at `table_size - 1` and only
                // decreases, and the `== 0` tests below/above stop it wrapping,
                // so it indexes `decode` (len `table_size`) in range.
                debug_assert!(high_threshold < decode.len());
                #[allow(unsafe_code)]
                unsafe {
                    decode.get_unchecked_mut(high_threshold).symbol = sym;
                }
                if high_threshold == 0 {
                    return Err(Error::Corruption);
                }
                high_threshold -= 1;
                // T4: `s` indexes `norm`, and `symbol_next` is
                // `norm.len().max(1)` long -- the `.max(1)` is what hides the
                // relation from LLVM, since an empty `norm` never enters this
                // loop at all.
                debug_assert!(s < symbol_next.len());
                #[allow(unsafe_code)]
                unsafe {
                    *symbol_next.get_unchecked_mut(s) = 1;
                }
            } else if p > 0 {
                debug_assert!(s < symbol_next.len());
                #[allow(unsafe_code)]
                unsafe {
                    *symbol_next.get_unchecked_mut(s) = p as u16;
                }
            }
        }

        let mask = table_size - 1;
        let step = (table_size >> 1) + (table_size >> 3) + 3;
        // D6 probe: does zstd's spread fast path condition ever hold here?
        #[cfg(feature = "profile")]
        {
            D6_SPREAD[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            if high_threshold == table_size - 1 {
                D6_SPREAD[1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                // entries the fast path would actually cover
                D6_SPREAD[2].fetch_add(table_size as u64, core::sync::atomic::Ordering::Relaxed);
            }
        }
        let mut position = 0usize;
        for (s, &p) in norm.iter().enumerate() {
            let sym = s as u8;
            for _ in 0..p.max(0) {
                // `position` is always `& mask` with `mask == table_size - 1`
                // and `table_size` a power of two, so it is in range for
                // `decode`.
                debug_assert!(position < decode.len());
                #[allow(unsafe_code)]
                unsafe {
                    decode.get_unchecked_mut(position).symbol = sym;
                }
                position = (position + step) & mask;
                while position > high_threshold {
                    position = (position + step) & mask;
                }
            }
        }

        for item in decode.iter_mut() {
            let s = item.symbol as usize;
            if s >= symbol_next.len() {
                return Err(Error::Corruption);
            }
            let next_state = symbol_next[s];
            symbol_next[s] = symbol_next[s].wrapping_add(1);
            if next_state == 0 {
                return Err(Error::Corruption);
            }
            let hb = 31 - (next_state as u32).leading_zeros();
            if hb > u32::from(accuracy_log) {
                return Err(Error::Corruption);
            }
            let nb = u32::from(accuracy_log) - hb;
            item.num_bits = nb as u8;
            item.baseline = ((u32::from(next_state) << nb) - table_size as u32) as u16;
        }

        Ok(())
    }

    pub(crate) fn init_state(&self, br: &mut BitRev<'_>) -> u16 {
        br.read_bits(u32::from(self.accuracy_log)) as u16
    }

    pub(crate) fn peek_symbol(&self, state: u16) -> Result<u16, Error> {
        Ok(u16::from(self.entry(state).symbol))
    }

    /// Power-of-two DTable (or RLE len=1): mask, no Option.
    ///
    /// T4 -- SAFETY. The index is already masked by `len - 1`, so it is in range
    /// for any non-empty power-of-two table, and both constructors give exactly
    /// that: `rle` builds len 1, and `from_norm` builds `1 << accuracy_log`
    /// after rejecting any log outside `5..=9`. `FseTable` is crate-private, is
    /// not re-exported from `lib.rs`, and nothing else writes `decode`, so no
    /// other shape can reach here.
    ///
    /// This runs THREE TIMES PER SEQUENCE on the decode path (LL, ML and OF
    /// tables), and LLVM cannot see the mask invariant because the length is a
    /// runtime value -- so it emitted a compare and a branch on every one.
    /// WIN 1 -- the loop-invariant half of `entry`, hoisted.
    ///
    /// `entry` re-derives BOTH halves of the fat pointer on every call: a load
    /// of `decode.ptr`, a load of `decode.len`, and a `dec` to make the mask.
    /// That is 3x per sequence, 160,527 sequences/MiB at L3 -- and because the
    /// sequence loop also holds `out: &mut Vec<u8>`, LLVM cannot always prove
    /// the table buffer is not aliased by it, so the reload is not hoistable
    /// by the optimiser either. Taking the view ONCE per block turns the
    /// per-call work into a register-held pointer plus an `and`.
    ///
    /// Same shape as the `litcopy_arm` / `seqcheck` hoists already in the
    /// sequence loop, for the same reason.
    #[inline(always)]
    pub(crate) fn view(&self) -> FseView<'_> {
        let dt = self.decode.as_slice();
        debug_assert!(!dt.is_empty() && dt.len().is_power_of_two());
        FseView {
            ptr: dt.as_ptr(),
            mask: dt.len().wrapping_sub(1),
            _m: core::marker::PhantomData,
        }
    }

    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn entry(&self, state: u16) -> FseEntry {
        let dt = self.decode.as_slice();
        debug_assert!(!dt.is_empty() && dt.len().is_power_of_two());
        let i = (state as usize) & dt.len().wrapping_sub(1);
        debug_assert!(i < dt.len());
        *unsafe { dt.get_unchecked(i) }
    }

    /// WIN 2: `advance` over the packed word, so the caller never materialises
    /// an `FseEntry` and both fields come from a register it already holds.
    #[inline(always)]
    pub(crate) fn advance_w(w: u32, br: &mut BitRev<'_>) -> u16 {
        let add = br.read_bits(u32::from(fse_nbits(w)));
        fse_baseline(w).wrapping_add(add as u16)
    }

    #[inline(always)]
    pub(crate) fn advance(e: FseEntry, br: &mut BitRev<'_>) -> u16 {
        let nb = u32::from(e.num_bits);
        let add = br.read_bits(nb);
        e.baseline.wrapping_add(add as u16)
    }

    pub(crate) fn update(&self, state: u16, br: &mut BitRev<'_>) -> Result<u16, Error> {
        Ok(Self::advance(self.entry(state), br))
    }
}

/// Read an FSE NCount header. Returns (table, bytes_consumed).
///
/// Superseded on every shipping path by the recycled-table W31 route; kept as
/// the allocating reference shape that route is gated against.
#[inline(always)]
#[allow(dead_code)]
pub(crate) fn read_ncount(
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(FseTable, usize), Error> {
    let mut nbuf = [0i16; 256];
    let (nlen, accuracy, consumed) = parse_ncount_into(&mut nbuf, src, max_symbol, max_log)?;
    let norm = &nbuf[..nlen];
    let table = FseTable::from_norm(norm, accuracy)?;
    Ok((table, consumed))
}

/// W26: `read_ncount` that rebuilds into a recycled table allocation.
#[inline(never)]
pub(crate) fn read_ncount_into(
    recycle: Option<FseTable>,
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(FseTable, usize), Error> {
    let mut nbuf = [0i16; 256];
    let (nlen, accuracy, consumed) = parse_ncount_into(&mut nbuf, src, max_symbol, max_log)?;
    let norm = &nbuf[..nlen];
    let table = FseTable::from_norm_into(recycle, norm, accuracy)?;
    Ok((table, consumed))
}

/// NCount header plus matching CTable (dictionary entropy / trainer).
#[cfg(feature = "alloc")]
pub(crate) fn read_ncount_ctable(
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(FseTable, FseCTable, usize), Error> {
    let mut nbuf = [0i16; 256];
    let (nlen, accuracy, consumed) = parse_ncount_into(&mut nbuf, src, max_symbol, max_log)?;
    let norm = &nbuf[..nlen];
    let dt = FseTable::from_norm(norm, accuracy)?;
    let ct = FseCTable::from_norm(norm, accuracy)?;
    Ok((dt, ct, consumed))
}

// PREMISE EXPIRED -- this carried `#[inline(always)]` "so callers compiled
// with BMI2 (the decode twin) get this bit-reading loop in their own ISA
// context -- the shim-trap rule". There is no such caller any more: the chain
// is `decode_seq_header` -> `seq_table` -> `read_ncount_into` -> here, and
// `decode_seq_header` is now `#[inline(never)]` and ISA-neutral, so this was
// inlining three times into a BASELINE function. The other caller is
// `dict.rs`'s dictionary load, also baseline. See the outlining note below.
/// W28 -- `norm` filled into a caller-owned STACK buffer.
///
/// `parse_ncount` allocated `vec![0i16; max_symbol + 1]` per table build and
/// returned it by value. `max_symbol` is at most 52 (ML) on the sequence path
/// and 255 for Huffman weights, so a 256-entry array covers every case and the
/// heap allocation disappears. Returns the filled LENGTH; the caller slices.
///
/// Outlined: LLVM inlined this into ALL THREE `read_ncount*` entries above (no
/// symbol of its own in the emitted asm), and it is the bulk of each -- an RFC
/// 8878 normalized-count parser stamped three times for a parse that runs at
/// most three times per block (ll/of/ml) and once per dictionary table. A call
/// is free at that rate.
#[inline(never)]
fn parse_ncount_into(
    norm: &mut [i16; 256],
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(usize, u8, usize), Error> {
    if src.is_empty() {
        return Err(Error::Corruption);
    }
    let mut tmp = [0u8; 256];
    let n = src.len().min(tmp.len());
    tmp[..n].copy_from_slice(&src[..n]);
    let padded = n.max(8);
    let mut bits = BitFwd::new(&tmp[..padded]);

    let accuracy = bits.get(4)? as u8 + 5;
    if accuracy < 5 || accuracy > max_log {
        return Err(Error::Corruption);
    }
    let mut remaining = (1i32 << accuracy) + 1;
    let mut threshold = 1i32 << accuracy;
    let mut nb_bits = accuracy as u32 + 1;
    if max_symbol + 1 > 256 {
        return Err(Error::Corruption);
    }
    let norm = &mut norm[..max_symbol + 1];
    norm.fill(0);
    let mut charnum = 0usize;
    let mut previous0 = false;

    loop {
        if previous0 {
            let mut extra = 0usize;
            loop {
                let r = bits.get(2)? as usize;
                extra += r;
                if r < 3 {
                    break;
                }
            }
            charnum += extra;
            if charnum > max_symbol {
                return Err(Error::Corruption);
            }
            previous0 = false;
            continue;
        }
        if remaining <= 1 {
            break;
        }
        let max = (2 * threshold - 1) - remaining;
        let peek_n = nb_bits;
        let v = bits.peek(peek_n)?;
        let low_mask = (threshold as u32).wrapping_sub(1);
        let count = if (v & low_mask) < max as u32 {
            bits.get(nb_bits - 1)?;
            (v & low_mask) as i32
        } else {
            bits.get(nb_bits)?;
            let mut c = (v & ((threshold as u32) * 2 - 1)) as i32;
            if c >= threshold {
                c -= max;
            }
            c
        };

        let count = count - 1;
        if count >= 0 {
            remaining -= count;
        } else {
            remaining += count;
        }
        if charnum > max_symbol {
            return Err(Error::Corruption);
        }
        norm[charnum] = count as i16;
        charnum += 1;
        previous0 = count == 0;
        if remaining < threshold {
            if remaining <= 1 {
                break;
            }
            nb_bits = 32 - (remaining as u32).leading_zeros();
            threshold = 1 << (nb_bits - 1);
        }
        if charnum > max_symbol {
            break;
        }
    }
    if remaining != 1 {
        return Err(Error::Corruption);
    }
    // W28: return the USED length; the caller slices the stack buffer, which is
    // exactly what `norm.truncate(used)` did to the heap Vec.
    let used = charnum.max(1);
    let consumed = bits.bytes_consumed().min(src.len());
    Ok((used, accuracy, consumed))
}

pub(crate) const DEFAULT_LL_NORM: [i16; 36] = [
    4, 3, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 2, 2, 2, 2, 2, 2, 2, 2, 2, 3, 2, 1, 1, 1, 1, 1,
    -1, -1, -1, -1,
];
pub(crate) const DEFAULT_ML_NORM: [i16; 53] = [
    1, 4, 3, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
    1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1, -1, -1,
];
pub(crate) const DEFAULT_OF_NORM: [i16; 29] = [
    1, 1, 1, 1, 1, 1, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, -1, -1, -1, -1, -1,
];

#[inline(always)]
pub(crate) fn default_ll() -> Result<FseTable, Error> {
    FseTable::from_norm(&DEFAULT_LL_NORM, 6)
}

#[inline(always)]
pub(crate) fn default_ml() -> Result<FseTable, Error> {
    FseTable::from_norm(&DEFAULT_ML_NORM, 6)
}

#[inline(always)]
pub(crate) fn default_of() -> Result<FseTable, Error> {
    FseTable::from_norm(&DEFAULT_OF_NORM, 5)
}

/// One CTable symbol slot (`deltaNbBits` + `deltaFindState`).
#[cfg(feature = "alloc")]
#[derive(Clone, Copy, Debug)]
struct FseCDelta {
    nb: u32,
    find: i32,
}

#[cfg(feature = "alloc")]
crate::scratch::scratch_slot!(SC_TABLE_SYMBOL: u16);
#[cfg(feature = "alloc")]
crate::scratch::scratch_slot!(SC_CUMUL: u16);
#[cfg(feature = "alloc")]
crate::scratch::pool_slot!(SC_NORM: i16);
#[cfg(feature = "alloc")]
crate::scratch::pool_slot!(SC_NCOUNT: u8);

/// FSE encode table (`FSE_buildCTable_wksp`).
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub(crate) struct FseCTable {
    table_log: u8,
    state_table: Vec<u16>,
    delta: Vec<FseCDelta>,
}

/// ALLOC-9: a bounded free list for `FseCTable`'s two owned buffers.
///
/// `state_table` and `delta` are owned BY the table, so they cannot be leased
/// like pure scratch -- they outlive `from_norm`. But their lifetime is still
/// short and bursty: `select_seq_table` builds a candidate table per seq table
/// per block and DROPS it whenever it loses, and even a winner is replaced next
/// block. Closing the loop at `Drop` recycles the buffers without changing a
/// single signature.
///
/// Bounded at 8 entries each, so a thread retains at most ~24 KiB rather than an
/// unbounded pool.
///
/// Equivalence: take + `clear` + `resize(n, default)` produces exactly what
/// `vec![default; n]` produced, and both buffers are fully overwritten by the
/// build that follows. `bytegate` is the gate.
#[cfg(all(feature = "std", feature = "alloc"))]
mod ct_pool {
    use super::FseCDelta;
    use alloc::vec::Vec;
    use core::cell::RefCell;
    const CAP: usize = 8;
    thread_local! {
        static STATE: RefCell<Vec<Vec<u16>>> = const { RefCell::new(Vec::new()) };
        static DELTA: RefCell<Vec<Vec<FseCDelta>>> = const { RefCell::new(Vec::new()) };
    }
    pub(super) fn take_state(n: usize) -> Vec<u16> {
        let mut v = STATE
            .try_with(|c| c.try_borrow_mut().ok().and_then(|mut p| p.pop()))
            .ok()
            .flatten()
            .unwrap_or_default();
        v.clear();
        v.resize(n, 0u16);
        v
    }
    pub(super) fn take_delta(n: usize) -> Vec<FseCDelta> {
        let mut v = DELTA
            .try_with(|c| c.try_borrow_mut().ok().and_then(|mut p| p.pop()))
            .ok()
            .flatten()
            .unwrap_or_default();
        v.clear();
        v.resize(n, FseCDelta { nb: 0, find: 0 });
        v
    }
    pub(super) fn give_state(v: Vec<u16>) {
        if v.capacity() == 0 {
            return;
        }
        let _ = STATE.try_with(|c| {
            if let Ok(mut p) = c.try_borrow_mut() {
                if p.len() < CAP {
                    p.push(v)
                }
            }
        });
    }
    pub(super) fn give_delta(v: Vec<FseCDelta>) {
        if v.capacity() == 0 {
            return;
        }
        let _ = DELTA.try_with(|c| {
            if let Ok(mut p) = c.try_borrow_mut() {
                if p.len() < CAP {
                    p.push(v)
                }
            }
        });
    }
}

#[cfg(all(feature = "std", feature = "alloc"))]
impl Drop for FseCTable {
    fn drop(&mut self) {
        ct_pool::give_state(core::mem::take(&mut self.state_table));
        ct_pool::give_delta(core::mem::take(&mut self.delta));
    }
}

#[cfg(feature = "alloc")]
impl FseCTable {
    /// OUTLINED. A 140-line table build that was `#[inline(always)]` at EIGHT
    /// call sites -- `select_seq_table`, `read_ncount_ctable`, the three
    /// `default_*_ctable` builders, the dictionary load and the cached probe.
    /// It runs at most a few times per BLOCK (and most of those sites run once
    /// per frame or once ever), so the call is free at that rate.
    ///
    /// Note the sibling directly above: `FseTable::from_norm_buf`, the DECODE
    /// side of the same build, is already `#[inline(never)]` for exactly this
    /// reason. This is the encode side of that parity.
    #[inline(never)]
    pub(crate) fn from_norm(norm: &[i16], table_log: u8) -> Result<Self, Error> {
        // T4: `max_sv` is `norm.len().saturating_sub(1)`, which SILENTLY yields
        // 0 for an empty `norm` and then indexes `norm[0]`. Rejecting it here
        // turns that into a clean error and makes every index below provable.
        if norm.is_empty() {
            return Err(Error::Corruption);
        }
        if !(5..=9).contains(&table_log) {
            return Err(Error::Corruption);
        }
        let table_size = 1usize << table_log;
        let max_sv = norm.len().saturating_sub(1);
        // ALLOC-1: both are pure scratch -- dropped when this function
        // returns -- so they recycle per thread. Two allocations per candidate
        // table build, x3 seq tables, per block. See `scratch.rs` for why a
        // lease rather than a hand-written take/restore: the `?` exits below
        // would leak the buffer.
        let mut table_symbol = crate::scratch::lease(&SC_TABLE_SYMBOL);
        table_symbol.resize(table_size, 0u16);
        let mut cumul = crate::scratch::lease(&SC_CUMUL);
        cumul.resize(max_sv + 2, 0u16);
        let mut high_threshold = table_size - 1;

        cumul[0] = 0;
        for u in 1..=max_sv + 1 {
            // SAFETY: `u - 1` spans `0..=max_sv`, i.e. exactly `norm`'s range
            // (non-empty, checked at entry); `cumul` is `max_sv + 2` long so
            // both `u` and `u - 1` are in range; and `high_threshold` starts at
            // `table_size - 1` and only saturating-decreases, so it indexes
            // `table_symbol` (len `table_size`) in range.
            debug_assert!(
                u - 1 < norm.len() && u < cumul.len() && high_threshold < table_symbol.len()
            );
            #[allow(unsafe_code)]
            let nv = *unsafe { norm.get_unchecked(u - 1) };
            #[allow(unsafe_code)]
            let prev = *unsafe { cumul.get_unchecked(u - 1) };
            if nv == -1 {
                #[allow(unsafe_code)]
                unsafe {
                    *cumul.get_unchecked_mut(u) = prev + 1;
                    *table_symbol.get_unchecked_mut(high_threshold) = (u - 1) as u16;
                }
                high_threshold = high_threshold.saturating_sub(1);
            } else {
                #[allow(unsafe_code)]
                unsafe {
                    *cumul.get_unchecked_mut(u) = prev.wrapping_add(nv.max(0) as u16);
                }
            }
        }
        // `cumul` was built `max_sv + 2` long, so this is its last slot.
        debug_assert!(max_sv + 1 < cumul.len());
        #[allow(unsafe_code)]
        unsafe {
            *cumul.get_unchecked_mut(max_sv + 1) = (table_size + 1) as u16;
        }

        let mask = table_size - 1;
        let step = (table_size >> 1) + (table_size >> 3) + 3;
        let mut position = 0usize;
        for (symbol, &freq) in norm.iter().enumerate() {
            for _ in 0..freq.max(0) {
                // `position` is always `& mask` with `mask == table_size - 1`
                // and `table_size` a power of two, so it indexes
                // `table_symbol` (len `table_size`) in range.
                debug_assert!(position < table_symbol.len());
                #[allow(unsafe_code)]
                unsafe {
                    *table_symbol.get_unchecked_mut(position) = symbol as u16;
                }
                position = (position + step) & mask;
                while position > high_threshold {
                    position = (position + step) & mask;
                }
            }
        }

        #[cfg(all(feature = "std", feature = "alloc"))]
        let mut state_table = ct_pool::take_state(table_size);
        #[cfg(not(all(feature = "std", feature = "alloc")))]
        let mut state_table = vec![0u16; table_size];
        for (u, &s) in table_symbol.iter().enumerate() {
            // SAFETY: every value in `table_symbol` is <= max_sv -- they are
            // written as `(u - 1)` with `u - 1 <= max_sv`, as a `norm` index in
            // the spread loop above, or left at the 0 default -- and `cumul` is
            // built `max_sv + 2` long. LLVM cannot derive that, because `s` is a
            // value READ OUT OF a Vec rather than a loop induction variable.
            debug_assert!((s as usize) < cumul.len());
            #[allow(unsafe_code)]
            let cs = unsafe { cumul.get_unchecked_mut(s as usize) };
            let idx = *cs as usize;
            if idx >= state_table.len() {
                return Err(Error::Corruption);
            }
            state_table[idx] = (table_size + u) as u16;
            *cs = cs.wrapping_add(1);
        }

        #[cfg(all(feature = "std", feature = "alloc"))]
        let mut delta = ct_pool::take_delta(max_sv + 1);
        #[cfg(not(all(feature = "std", feature = "alloc")))]
        let mut delta = vec![FseCDelta { nb: 0, find: 0 }; max_sv + 1];
        let mut total: u32 = 0;
        for s in 0..=max_sv {
            // SAFETY: `s <= max_sv` is `norm`'s range (non-empty, checked at
            // entry) and `delta` is built `max_sv + 1` long just above.
            debug_assert!(s < norm.len() && s < delta.len());
            #[allow(unsafe_code)]
            let nv = *unsafe { norm.get_unchecked(s) };
            // One bound reference for the whole arm, instead of re-proving `s`
            // on each of the six writes below.
            #[allow(unsafe_code)]
            let d = unsafe { delta.get_unchecked_mut(s) };
            match nv {
                0 => {
                    d.nb = ((u32::from(table_log) + 1) << 16) - (1 << table_log);
                }
                -1 | 1 => {
                    d.nb = (u32::from(table_log) << 16) - (1 << table_log);
                    d.find = total as i32 - 1;
                    total += 1;
                }
                freq => {
                    let freq = freq as u32;
                    let hb = 31 - (freq - 1).leading_zeros();
                    let max_bits_out = u32::from(table_log) - hb;
                    let min_state_plus = freq << max_bits_out;
                    d.nb = (max_bits_out << 16).wrapping_sub(min_state_plus);
                    d.find = total as i32 - freq as i32;
                    total += freq;
                }
            }
        }

        Ok(Self {
            table_log,
            state_table,
            delta,
        })
    }

    pub(crate) fn rle(symbol: u16) -> Self {
        let n = 64usize;
        let mut delta = vec![
            FseCDelta {
                nb: u32::MAX,
                find: 0
            };
            n
        ];
        if (symbol as usize) < n {
            delta[symbol as usize] = FseCDelta { nb: 0, find: 0 };
        }
        Self {
            table_log: 0,
            state_table: vec![0, 0],
            delta,
        }
    }

    #[inline(always)]
    fn delta_at(&self, symbol: usize) -> FseCDelta {
        self.delta
            .get(symbol)
            .copied()
            .unwrap_or(FseCDelta { nb: 0, find: 0 })
    }

    pub(crate) fn init_state2(&self, symbol: usize) -> u32 {
        let d = self.delta_at(symbol);
        let nb_bits_out = (d.nb.wrapping_add(1 << 15)) >> 16;
        let value = (nb_bits_out << 16).wrapping_sub(d.nb);
        let idx = (value >> nb_bits_out) as i32 + d.find;
        self.state_table
            .get(idx as usize)
            .copied()
            .unwrap_or(0)
            .into()
    }

    #[inline(always)]
    pub(crate) fn encode(&self, state: &mut u32, bits: &mut crate::bit::BitCStream, symbol: usize) {
        let d = self.delta_at(symbol);
        let nb_bits_out = state.wrapping_add(d.nb) >> 16;
        bits.add_bits(u64::from(*state), nb_bits_out);
        let idx = (*state >> nb_bits_out) as i32 + d.find;
        *state = self
            .state_table
            .get(idx as usize)
            .copied()
            .unwrap_or(0)
            .into();
    }

    #[inline(always)]
    pub(crate) fn flush(&self, state: u32, bits: &mut crate::bit::BitCStream) {
        bits.add_bits(u64::from(state), u32::from(self.table_log));
        bits.flush();
    }
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(crate) fn default_ll_ctable() -> Result<FseCTable, Error> {
    FseCTable::from_norm(&DEFAULT_LL_NORM, 6)
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(crate) fn default_ml_ctable() -> Result<FseCTable, Error> {
    FseCTable::from_norm(&DEFAULT_ML_NORM, 6)
}

#[cfg(feature = "alloc")]
#[allow(dead_code)]
pub(crate) fn default_of_ctable() -> Result<FseCTable, Error> {
    FseCTable::from_norm(&DEFAULT_OF_NORM, 5)
}

/// The three RFC-constant default ctables, built ONCE per process.
///
/// N9 (inline-execution): `select_seq_table` rebuilt one of these on EVERY call
/// -- three heap allocations, a cumul pass, the serial spread loop, a scatter
/// into `state_table` and the delta build -- for a value fixed by RFC 8878 and
/// byte-identical for the life of the process. Measured before the fix:
/// **22-30 rebuilds per MiB encoded**, ~7,500 heap allocations per 88 MiB.
///
/// This is V1's defect class exactly, and the three helpers directly above are
/// the evidence: `default_ll_ctable()` and its siblings already existed here,
/// correct and tested -- marked `#[allow(dead_code)]` and called only from
/// tests, while the shipping path rebuilt the same tables by hand.
///
/// Returns `None` for any norm that is not one of the three, so the caller
/// keeps its existing build path and this can never change a result.
///
/// Dispatch is by SHAPE, then VERIFIED BY CONTENT.
///
/// The obvious implementation -- pointer identity against the three constants --
/// is a silent no-op here, and the N9 counter caught it: `DEFAULT_*_NORM` are
/// `const`, not `static`, so each use site gets its own inlined copy at its own
/// address and `ptr::eq` never matches. The rebuild count did not move.
///
/// So match on `(len, log)`, which is unique across the three today, and then
/// confirm the CONTENT before handing back a cached table. The compare is 58-106
/// bytes against three heap allocations plus a spread, a scatter and a delta
/// build -- and it is what makes a future fourth table with the same shape
/// impossible to bind to the wrong slot. A wrong ctable is a wrong bitstream,
/// not a crash, so this is checked at runtime and not merely asserted.
#[cfg(all(feature = "std", feature = "alloc"))]
pub(crate) fn default_ctable_cached(norm: &[i16], log: u8) -> Option<&'static FseCTable> {
    use std::sync::OnceLock;
    static LL: OnceLock<FseCTable> = OnceLock::new();
    static ML: OnceLock<FseCTable> = OnceLock::new();
    static OF: OnceLock<FseCTable> = OnceLock::new();
    let (slot, expect): (&'static OnceLock<FseCTable>, &[i16]) = match (norm.len(), log) {
        (36, 6) => (&LL, &DEFAULT_LL_NORM),
        (53, 6) => (&ML, &DEFAULT_ML_NORM),
        (29, 5) => (&OF, &DEFAULT_OF_NORM),
        _ => return None,
    };
    if norm != expect {
        return None;
    }
    if let Some(t) = slot.get() {
        return Some(t);
    }
    // Build outside the lock. A race just builds twice and discards one, which
    // is harmless: the result is a pure function of two constants.
    let built = FseCTable::from_norm(norm, log).ok()?;
    Some(slot.get_or_init(|| built))
}

/// FSE_optimalTableLog (minus=2).
#[cfg(feature = "alloc")]
pub(crate) fn optimal_table_log(max_log: u8, src_size: usize, max_symbol: usize) -> u8 {
    if src_size <= 1 {
        return 5;
    }
    let max_bits_src = 31 - (src_size as u32 - 1).leading_zeros();
    let max_bits_src = max_bits_src.saturating_sub(2);
    let min_bits_src = 31 - (src_size as u32).leading_zeros() + 1;
    let min_bits_sym = 31 - (max_symbol as u32).leading_zeros() + 2;
    let min_bits = min_bits_src.min(min_bits_sym).max(5);
    let mut log = u32::from(max_log).min(max_bits_src).max(min_bits);
    log = log.clamp(5, 9);
    log as u8
}

/// FSE_normalizeCount (primary path + simple fallback).
#[cfg(feature = "alloc")]
#[inline(always)]
pub(crate) fn normalize_count(
    count: &[u32],
    table_log: u8,
    total: u32,
    use_low_prob: bool,
) -> Result<Vec<i16>, Error> {
    if total == 0 || !(5..=9).contains(&table_log) || count.is_empty() {
        return Err(Error::Corruption);
    }
    // T4: the `count.is_empty()` arm above is what makes `max_sv` -- and every
    // index derived from it -- provable. `max_sv = count.len() - 1` already
    // relied on non-empty; stating it turns a latent underflow into a clean
    // error AND lets the loop below index without a check.
    debug_assert!(!count.is_empty());
    let max_sv = count.len() - 1;
    if count.contains(&total) {
        return Err(Error::Corruption);
    }
    let low_prob: i16 = if use_low_prob { -1 } else { 1 };
    let scale = 62u32 - u32::from(table_log);
    let step = (1u64 << 62) / u64::from(total.max(1));
    let v_step = 1u64 << (scale.saturating_sub(20));
    let rtb: [u64; 8] = [0, 473195, 504333, 520860, 550000, 700000, 750000, 830000];
    // ALLOC-14: `norm` escapes to `ncount_and_ctable`, which drops it after
    // building the header and the ctable -- so it comes from a bounded pool and
    // is given back at that death site, not here.
    let mut norm = crate::scratch::pool_take(&SC_NORM);
    norm.resize(max_sv + 1, 0i16);
    let mut still = 1i32 << table_log;
    let low_threshold = total >> table_log;
    let mut largest = 0usize;
    let mut largest_p: i16 = 0;
    for s in 0..=max_sv {
        // SAFETY: `max_sv == count.len() - 1` with `count` non-empty (checked at
        // entry), and `norm` is built `max_sv + 1` long just above.
        debug_assert!(s < count.len() && s < norm.len());
        #[allow(unsafe_code)]
        let c = *unsafe { count.get_unchecked(s) };
        if c == 0 {
            continue;
        }
        if c <= low_threshold {
            norm[s] = low_prob;
            still -= 1;
            continue;
        }
        let mut proba = ((u64::from(c) * step) >> scale) as i16;
        if proba < 8 {
            let rest = (u64::from(c) * step) - ((proba as u64) << scale);
            // `proba < 8` is guaranteed by the branch above and `proba` comes
            // from an unsigned shift, so the clamp is a no-op -- it exists only
            // to let LLVM drop the bounds check on an 8-entry table.
            debug_assert!((0..8).contains(&proba));
            if rest > v_step * rtb[(proba as usize).min(7)] {
                proba += 1;
            }
        }
        if proba > largest_p {
            largest_p = proba;
            largest = s;
        }
        norm[s] = proba;
        still -= i32::from(proba);
    }
    // SAFETY for the `largest` accesses here and below: `largest` starts at 0
    // and is only ever assigned `s` from the loop above, so `largest <= max_sv`,
    // and both `norm` and `n2` are built `max_sv + 1` long. `count` is non-empty
    // by the check at entry, so `max_sv + 1 >= 1` and index 0 is valid too.
    #[allow(unsafe_code)]
    let norm_largest = || *unsafe { norm.get_unchecked(largest) };
    debug_assert!(largest < norm.len());
    if still.abs() >= i32::from(norm_largest().unsigned_abs()) / 2 && still < 0 {
        // fallback: scale by table size
        let mut n2 = crate::scratch::pool_take(&SC_NORM);
        n2.resize(max_sv + 1, 0i16);
        let mut dist = 0i32;
        for s in 0..=max_sv {
            if count[s] == 0 {
                continue;
            }
            let w = ((u64::from(count[s]) << table_log) / u64::from(total)).max(1) as i16;
            n2[s] = w;
            dist += i32::from(w);
        }
        let leftover = (1i32 << table_log) - dist;
        debug_assert!(largest < n2.len());
        #[allow(unsafe_code)]
        unsafe {
            let v = n2.get_unchecked_mut(largest);
            *v = (i32::from(*v) + leftover) as i16;
            if *v < 1 {
                *v = 1;
            }
        }
        return Ok(n2);
    }
    #[allow(unsafe_code)]
    let nl = unsafe { norm.get_unchecked_mut(largest) };
    *nl = (i32::from(*nl) + still) as i16;
    if *nl < 1 {
        *nl = 1;
    }
    Ok(norm)
}

/// FSE_writeNCount.
#[cfg(feature = "alloc")]
#[inline(always)]
/// ALLOC-14: give an ncount header buffer back after it has been copied out.
#[cfg(feature = "alloc")]
pub(crate) fn give_ncount_buf(v: alloc::vec::Vec<u8>) {
    crate::scratch::pool_give(&SC_NCOUNT, v);
}

pub(crate) fn write_ncount(norm: &[i16], table_log: u8) -> Result<Vec<u8>, Error> {
    // ALLOC-4: `Vec::new()` grown by `push` reallocated on every doubling --
    // 1, 2, 4, 8 ... which is why the attribution sampler kept landing in
    // `flush`, the only function that pushes. The stream is at most two bytes
    // per symbol plus a short header, so one sized allocation replaces
    // log2(n) of them. Capacity cannot affect content: byte-identical.
    // ALLOC-14: the ncount header is copied into `dst` by the sequence-section
    // writer and dropped there; pooled, with the give-back at that copy.
    let mut out = crate::scratch::pool_take(&SC_NCOUNT);
    out.reserve(2 * norm.len() + 8);
    let mut bit_stream: u32 = 0;
    let mut bit_count: i32 = 0;
    bit_stream |= u32::from(table_log.saturating_sub(5)) << bit_count;
    bit_count += 4;
    let table_size = 1i32 << table_log;
    let mut remaining = table_size + 1;
    let mut threshold = table_size;
    let mut nb_bits = table_log as i32 + 1;
    let mut symbol = 0usize;
    let alphabet = norm.len();
    let mut previous0 = false;

    fn flush(out: &mut Vec<u8>, bit_stream: &mut u32, bit_count: &mut i32) {
        if *bit_count > 16 {
            out.push(*bit_stream as u8);
            out.push((*bit_stream >> 8) as u8);
            *bit_stream >>= 16;
            *bit_count -= 16;
        }
    }

    while symbol < alphabet && remaining > 1 {
        if previous0 {
            let start = symbol;
            while symbol < alphabet && norm[symbol] == 0 {
                symbol += 1;
            }
            if symbol == alphabet {
                break;
            }
            let mut start_i = start;
            while symbol >= start_i + 24 {
                start_i += 24;
                bit_stream |= 0xFFFF << bit_count;
                out.push(bit_stream as u8);
                out.push((bit_stream >> 8) as u8);
                bit_stream >>= 16;
            }
            while symbol >= start_i + 3 {
                start_i += 3;
                bit_stream |= 3 << bit_count;
                bit_count += 2;
            }
            bit_stream |= ((symbol - start_i) as u32) << bit_count;
            bit_count += 2;
            flush(&mut out, &mut bit_stream, &mut bit_count);
        }
        if symbol >= alphabet {
            break;
        }
        let mut count = i32::from(norm[symbol]);
        symbol += 1;
        let max = (2 * threshold - 1) - remaining;
        remaining -= if count < 0 { -count } else { count };
        count += 1;
        if count >= threshold {
            count += max;
        }
        bit_stream |= (count as u32) << bit_count;
        bit_count += nb_bits;
        if count < max {
            bit_count -= 1;
        }
        previous0 = count == 1;
        if remaining < 1 {
            return Err(Error::Corruption);
        }
        while remaining < threshold {
            nb_bits -= 1;
            threshold >>= 1;
        }
        flush(&mut out, &mut bit_stream, &mut bit_count);
    }
    if remaining != 1 {
        return Err(Error::Corruption);
    }
    out.push(bit_stream as u8);
    out.push((bit_stream >> 8) as u8);
    let extra = ((bit_count + 7) / 8) as usize;
    out.truncate(out.len().saturating_sub(2) + extra.max(1));
    Ok(out)
}

/// Two-state FSE compress of a byte slice (Huffman weights / generic).
/// Matches libzstd `FSE_compress_usingCTable` on a 64-bit `BIT_CStream`.
#[cfg(feature = "alloc")]
pub(crate) fn compress_using_ctable(src: &[u8], table: &FseCTable) -> Result<Vec<u8>, Error> {
    // Its own BMI2 twin (the 621a140 pattern): this is the Huffman tree
    // header's FSE bitstream, a variable-shift loop outside every other twin.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe { compress_using_ctable_bmi2(src, table) };
    }
    compress_using_ctable_inner(src, table)
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn compress_using_ctable_bmi2(src: &[u8], table: &FseCTable) -> Result<Vec<u8>, Error> {
    compress_using_ctable_inner(src, table)
}

#[inline(always)]
fn compress_using_ctable_inner(src: &[u8], table: &FseCTable) -> Result<Vec<u8>, Error> {
    if src.len() <= 2 {
        return Err(Error::Corruption);
    }
    // T4: `i` starts at `src.len()` and EVERY access below decrements first, so
    // `i < src.len()` holds at every read. LLVM cannot follow that through the
    // unrolled 2/4-way tail, so it bounds-checked a per-SYMBOL access.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn at(src: &[u8], i: usize) -> usize {
        debug_assert!(i < src.len());
        (*unsafe { src.get_unchecked(i) }) as usize
    }
    let mut bits = crate::bit::BitCStream::new();
    let mut i = src.len();
    let mut state1: u32;
    let mut state2: u32;
    if src.len() & 1 != 0 {
        i -= 1;
        state1 = table.init_state2(at(src, i));
        i -= 1;
        state2 = table.init_state2(at(src, i));
        i -= 1;
        table.encode(&mut state1, &mut bits, at(src, i));
        bits.flush();
    } else {
        i -= 1;
        state2 = table.init_state2(at(src, i));
        i -= 1;
        state1 = table.init_state2(at(src, i));
    }
    if ((src.len() - 2) & 2) != 0 {
        i -= 1;
        table.encode(&mut state2, &mut bits, at(src, i));
        i -= 1;
        table.encode(&mut state1, &mut bits, at(src, i));
        bits.flush();
    }
    while i >= 4 {
        i -= 1;
        table.encode(&mut state2, &mut bits, at(src, i));
        i -= 1;
        table.encode(&mut state1, &mut bits, at(src, i));
        i -= 1;
        table.encode(&mut state2, &mut bits, at(src, i));
        i -= 1;
        table.encode(&mut state1, &mut bits, at(src, i));
        bits.flush();
    }
    table.flush(state2, &mut bits);
    table.flush(state1, &mut bits);
    Ok(bits.close())
}

/// Build an NCount header + CTable from symbol counts (`max_log` 5..=9).
#[cfg(feature = "alloc")]
#[inline(always)]
pub(crate) fn ncount_and_ctable(
    count: &[u32],
    max_log: u8,
    use_low_prob: bool,
) -> Result<(Vec<u8>, FseCTable), Error> {
    let total: u32 = count.iter().sum();
    let max_sv = count
        .iter()
        .rposition(|&c| c > 0)
        .ok_or(Error::Corruption)?;
    if count[max_sv] == total {
        return Err(Error::Corruption);
    }
    let table_log = optimal_table_log(max_log, total as usize, max_sv);
    let norm = normalize_count(&count[..=max_sv], table_log, total, use_low_prob)?;
    let header = write_ncount(&norm, table_log)?;
    let ct = FseCTable::from_norm(&norm, table_log)?;
    crate::scratch::pool_give(&SC_NORM, norm);
    Ok((header, ct))
}

#[cfg(feature = "alloc")]
impl FseCTable {
    /// `true` if `symbol` has a usable CTable slot (C `FSE_getMaxNbBits` <= tableLog).
    ///
    /// Zero-probability slots store `deltaNbBits = ((tableLog+1)<<16) - tableSize`.
    /// A raw `>> 16` yields `tableLog` (the subtract borrows), so C rounds with
    /// `+ 0xFFFF` before the shift (`FSE_getMaxNbBits`).
    pub(crate) fn can_encode_symbol(&self, symbol: usize) -> bool {
        if self.table_log == 0 {
            return self.delta.get(symbol).map(|d| d.nb == 0).unwrap_or(false);
        }
        match self.delta.get(symbol) {
            Some(d) => ((d.nb + 0xFFFF) >> 16) <= u32::from(self.table_log),
            None => false,
        }
    }

    pub(crate) fn bit_cost(&self, counts: &[u32]) -> u64 {
        if self.table_log == 0 {
            for (s, &n) in counts.iter().enumerate() {
                if n == 0 {
                    continue;
                }
                if !self.can_encode_symbol(s) {
                    return u64::MAX / 4;
                }
            }
            return 0;
        }
        let mut c = 0u64;
        for (s, &n) in counts.iter().enumerate() {
            if n == 0 {
                continue;
            }
            if !self.can_encode_symbol(s) {
                // libzstd ZSTD_fseBitCost: Repeat is illegal when Prob[s]==0.
                return u64::MAX / 4;
            }
            let dnb = self.delta[s].nb;
            let nb = ((dnb + 0xFFFF) >> 16).max(1);
            c += u64::from(n) * u64::from(nb);
        }
        c
    }
}

/// FSE-decompress Huffman weights (two interleaved states).
///
/// TWIN RETIRED, and the note it carried was wrong about its own frequency.
/// It read "per new-table block on the literal decode path" -- but the decode
/// path stopped calling this at W39. Its two remaining callers are
/// `huffman::read_ctable` (dictionary load, once per dictionary) and a test.
/// A twin cannot pay for itself at once-per-dictionary, and this one was not
/// even trying: `decompress_weights_inner` is `#[inline(never)]`, so the twin
/// compiled to a single `jmp` to a baseline symbol and every shift in the body
/// stayed `%cl`. The real per-block path is `decompress_weights_into` below,
/// which had no twin at all -- it has one now.
pub(crate) fn decompress_weights(src: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), Error> {
    decompress_weights_inner(src, max_out)
}

// W31 -- a recycled FSE table for the Huffman WEIGHT decoder.
//
// `read_ncount(src, 255, 6)` built a fresh table per weight decode; the
// allocation census attributed ~1,150 allocations to it. This path has no
// `BlockState` to hang a buffer on, so it uses a thread-local scratch -- the
// same pattern `huffman.rs` already uses. Handed back only on success; an
// error fails the frame anyway.
#[cfg(feature = "std")]
std::thread_local! {
    static WEIGHT_TBL: core::cell::RefCell<Option<FseTable>> =
        const { core::cell::RefCell::new(None) };
}

/// W39 -- fill a caller-owned buffer instead of returning a `Vec`.
///
/// The only decode caller (`huffman::read_table`) copies the result into its
/// own stack buffer and drops the Vec, so the allocation is pure overhead --
/// ~800 per board, one per FSE-coded weight table. Writing straight into the
/// caller's buffer removes both the allocation and the copy.
pub(crate) fn decompress_weights_into(
    dst: &mut [u8],
    src: &[u8],
    max_out: usize,
) -> Result<(usize, usize), Error> {
    let cap = max_out.min(dst.len());
    // The twin the retired one above should have been. This IS the per-block
    // literal decode path (`huffman::read_table`), and the body is a two-state
    // FSE loop -- one `table.update` plus one `br.reload()` per weight symbol,
    // both variable-shift bit reads. That is what BMI2 `shrx`/`bzhi` exist for.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body via `weights_into_body`.
        #[allow(unsafe_code)]
        return unsafe { weights_into_bmi2(dst, src, cap) };
    }
    weights_into_inner(dst, src, cap)
}

/// The BMI2-compiled arm. Calls the `#[inline(always)]` BODY, never the
/// `#[inline(never)]` baseline arm -- a twin that calls its sibling is a `jmp`
/// thunk and does nothing at all. See the retired twin above for the shape
/// this is deliberately not.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn weights_into_bmi2(
    dst: &mut [u8],
    src: &[u8],
    max_out: usize,
) -> Result<(usize, usize), Error> {
    weights_into_body(dst, src, max_out)
}

/// The BASELINE arm. Outlined so the body is generated once for non-BMI2
/// hosts rather than inlined into every caller.
#[inline(never)]
fn weights_into_inner(dst: &mut [u8], src: &[u8], max_out: usize) -> Result<(usize, usize), Error> {
    weights_into_body(dst, src, max_out)
}

/// The weight decode, writing symbols straight into `dst`. Mirrors
/// `decompress_weights_inner` exactly; `n_out` replaces `out.len()`.
///
/// `#[inline(always)]` is LOAD-BEARING: it is what lets each ISA arm above
/// re-generate this body under its own feature set.
#[inline(always)]
fn weights_into_body(dst: &mut [u8], src: &[u8], max_out: usize) -> Result<(usize, usize), Error> {
    #[cfg(feature = "std")]
    let recycled = WEIGHT_TBL.with(|c| c.borrow_mut().take());
    #[cfg(not(feature = "std"))]
    let recycled: Option<FseTable> = None;
    let (table, n) = read_ncount_into(recycled, src, 255, 6)?;
    if n >= src.len() {
        return Err(Error::Corruption);
    }
    let rest = &src[n..];
    let mut br = BitRev::new(rest)?;
    let mut s1 = table.init_state(&mut br);
    let mut s2 = table.init_state(&mut br);
    let mut n_out = 0usize;
    loop {
        if n_out >= max_out {
            return Err(Error::Corruption);
        }
        dst[n_out] = table.entry(s1).symbol;
        n_out += 1;
        s1 = table.update(s1, &mut br)?;
        let _ = br.reload();
        if br.overflowed() {
            if n_out < max_out {
                dst[n_out] = table.entry(s2).symbol;
                n_out += 1;
            }
            break;
        }
        if n_out >= max_out {
            return Err(Error::Corruption);
        }
        dst[n_out] = table.entry(s2).symbol;
        n_out += 1;
        s2 = table.update(s2, &mut br)?;
        let _ = br.reload();
        if br.overflowed() {
            if n_out < max_out {
                dst[n_out] = table.entry(s1).symbol;
                n_out += 1;
            }
            break;
        }
    }
    #[cfg(feature = "std")]
    WEIGHT_TBL.with(|c| *c.borrow_mut() = Some(table));
    Ok((n_out, n + rest.len()))
}

// `#[inline(never)]`, not `always`: this decodes the Huffman weight header -- ONCE per block,
// so a call is free at that frequency -- while inlining reproduced its
// whole body at every site, and the hosts here are twinned
// (baseline / bmi2 / avx2). Same finding as `select_seq_table`, which
// shrank `write_sequences` from 12,413 to 2,216 instructions.
#[inline(never)]
fn decompress_weights_inner(src: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), Error> {
    #[cfg(feature = "std")]
    let recycled = WEIGHT_TBL.with(|c| c.borrow_mut().take());
    #[cfg(not(feature = "std"))]
    let recycled: Option<FseTable> = None;
    let (table, n) = read_ncount_into(recycled, src, 255, 6)?;
    if n >= src.len() {
        return Err(Error::Corruption);
    }
    let rest = &src[n..];
    let mut br = BitRev::new(rest)?;
    let mut s1 = table.init_state(&mut br);
    let mut s2 = table.init_state(&mut br);
    // W29 -- pre-size the weight output.
    //
    // This was `Vec::new()` grown by `push`, so a weight table of N symbols paid
    // ~log2(N) reallocations plus their copies. The loop's own guard proves the
    // bound: it errors the moment `out.len() >= max_out`, so `max_out` is an
    // exact ceiling and one allocation replaces the whole growth chain.
    let mut out = Vec::with_capacity(max_out);
    loop {
        if out.len() >= max_out {
            return Err(Error::Corruption);
        }
        out.push(table.peek_symbol(s1)? as u8);
        s1 = table.update(s1, &mut br)?;
        let _ = br.reload();
        if br.overflowed() {
            if out.len() < max_out {
                out.push(table.peek_symbol(s2)? as u8);
            }
            break;
        }
        if out.len() >= max_out {
            return Err(Error::Corruption);
        }
        out.push(table.peek_symbol(s2)? as u8);
        s2 = table.update(s2, &mut br)?;
        let _ = br.reload();
        if br.overflowed() {
            if out.len() < max_out {
                out.push(table.peek_symbol(s1)? as u8);
            }
            break;
        }
    }
    // W31: hand the table back for the next weight decode.
    #[cfg(feature = "std")]
    WEIGHT_TBL.with(|c| *c.borrow_mut() = Some(table));
    Ok((out, n + rest.len()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_ll_matches_rfc_appendix() {
        let t = default_ll().unwrap();
        assert_eq!(t.accuracy_log, 6);
        assert_eq!(t.decode.len(), 64);
        // RFC 8878 Appendix A.1 (skip the duplicated header row).
        assert_eq!(t.decode[0].symbol, 0);
        assert_eq!(t.decode[0].num_bits, 4);
        assert_eq!(t.decode[0].baseline, 0);
        assert_eq!(t.decode[1].symbol, 0);
        assert_eq!(t.decode[1].num_bits, 4);
        assert_eq!(t.decode[1].baseline, 16);
        assert_eq!(t.decode[63].symbol, 32);
        assert_eq!(t.decode[63].num_bits, 6);
        assert_eq!(
            core::mem::size_of::<FseEntry>(),
            4,
            "C FSE_decode_t is 4 bytes"
        );
    }

    #[test]
    fn ncount_roundtrip_default_ll() {
        let bytes = write_ncount(&DEFAULT_LL_NORM, 6).unwrap();
        let (t, n) = read_ncount(&bytes, 35, 9).unwrap();
        assert!(
            n > 0 && n <= bytes.len(),
            "consumed={n} len={}",
            bytes.len()
        );
        assert_eq!(t.accuracy_log, 6);
        assert_eq!(t.decode.len(), 64);
        assert_eq!(t.decode[0].symbol, 0);
    }

    #[test]
    fn fse_custom_table_symbol_roundtrip() {
        let mut count = [0u32; 36];
        count[0] = 10;
        count[1] = 5;
        count[4] = 3;
        let (hdr, ct) = ncount_and_ctable(&count, 9, true).expect("ncount");
        let (dt, n) = read_ncount(&hdr, 35, 9).expect("read");
        assert_eq!(n, hdr.len());
        let syms = [0u8, 1, 4, 0, 1, 0, 4, 0, 1, 0];
        let mut bits = crate::bit::BitCStream::new();
        let last = *syms.last().unwrap();
        let mut st = ct.init_state2(last as usize);
        bits.flush();
        for &s in syms.iter().rev().skip(1) {
            ct.encode(&mut st, &mut bits, s as usize);
            bits.flush();
        }
        ct.flush(st, &mut bits);
        let stream = bits.close();
        let mut br = crate::bit::BitRev::new(&stream).expect("bitrev");
        let mut ds = dt.init_state(&mut br);
        let mut out = Vec::new();
        for i in 0..syms.len() {
            out.push(dt.peek_symbol(ds).unwrap() as u8);
            if i + 1 != syms.len() {
                ds = dt.update(ds, &mut br).unwrap();
            }
        }
        assert_eq!(out, syms);
    }

    #[test]
    fn ncount_roundtrip_default_of() {
        let bytes = write_ncount(&DEFAULT_OF_NORM, 5).unwrap();
        let (t, _) = read_ncount(&bytes, 31, 8).unwrap();
        assert_eq!(t.accuracy_log, 5);
        assert_eq!(t.decode[0].symbol, 0);
    }

    fn assert_init_state2_matches_dtable(ct: &FseCTable, dt: &FseTable, max_sym: usize) {
        for s in 0..=max_sym {
            if !ct.can_encode_symbol(s) {
                continue;
            }
            let st = ct.init_state2(s);
            let mut bits = crate::bit::BitCStream::new();
            ct.flush(st, &mut bits);
            let stream = bits.close();
            let mut br = crate::bit::BitRev::new(&stream).unwrap();
            let ds = dt.init_state(&mut br);
            let got = dt.peek_symbol(ds).unwrap() as usize;
            assert_eq!(
                got, s,
                "init_state2({s}) peeks as {got} table_log={}",
                ct.table_log
            );
        }
    }

    #[test]
    fn init_state2_matches_dtable_defaults() {
        let ll_c = default_ll_ctable().unwrap();
        let ll_d = default_ll().unwrap();
        assert_init_state2_matches_dtable(&ll_c, &ll_d, 35);
        let ml_c = default_ml_ctable().unwrap();
        let ml_d = default_ml().unwrap();
        assert_init_state2_matches_dtable(&ml_c, &ml_d, 52);
        let of_c = default_of_ctable().unwrap();
        let of_d = default_of().unwrap();
        assert_init_state2_matches_dtable(&of_c, &of_d, 28);
    }

    #[test]
    fn init_state2_matches_dtable_compressed() {
        let mut count = [0u32; 36];
        count[0] = 20;
        count[1] = 8;
        count[2] = 5;
        count[4] = 2;
        count[16] = 1;
        let (hdr, ct) = ncount_and_ctable(&count, 9, false).unwrap();
        let (dt, _) = read_ncount(&hdr, 35, 9).unwrap();
        assert_init_state2_matches_dtable(&ct, &dt, 16);
        assert!(!ct.can_encode_symbol(3));
        assert!(ct.can_encode_symbol(4));
        let mut buf = count;
        buf[4] -= 1;
        let (hdr2, ct2) = ncount_and_ctable(&buf, 9, false).unwrap();
        let (dt2, _) = read_ncount(&hdr2, 35, 9).unwrap();
        assert_init_state2_matches_dtable(&ct2, &dt2, 16);
    }

    #[test]
    fn bit_cost_rejects_zero_prob_and_missing_symbol() {
        let mut count = [0u32; 8];
        count[0] = 10;
        count[1] = 5;
        count[2] = 3;
        let (_, ct) = ncount_and_ctable(&count, 9, false).expect("ncount");
        let mut ok = [0u32; 8];
        ok[0] = 4;
        ok[1] = 4;
        assert!(ct.bit_cost(&ok) < u64::MAX / 4);
        let mut missing = [0u32; 8];
        missing[0] = 3;
        missing[7] = 1;
        assert_eq!(ct.bit_cost(&missing), u64::MAX / 4);
        let mut zero_prob = [0u32; 8];
        zero_prob[4] = 1;
        assert_eq!(ct.bit_cost(&zero_prob), u64::MAX / 4);
        assert!(ct.can_encode_symbol(0));
        assert!(!ct.can_encode_symbol(4));
        assert!(!ct.can_encode_symbol(7));
    }

    #[test]
    fn last_symbol_init_only_roundtrips_when_in_table() {
        let mut count = [0u32; 36];
        count[0] = 10;
        count[1] = 5;
        count[4] = 3;
        let mut buf = count;
        if buf[4] > 1 {
            buf[4] -= 1;
        }
        let (hdr, ct) = ncount_and_ctable(&buf, 9, false).expect("ncount");
        assert!(ct.can_encode_symbol(4));
        let (dt, n) = read_ncount(&hdr, 35, 9).expect("read");
        assert_eq!(n, hdr.len());
        let syms = [0u8, 1, 0, 1, 0, 0, 1, 0, 4];
        let mut bits = crate::bit::BitCStream::new();
        let last = *syms.last().unwrap();
        let mut st = ct.init_state2(last as usize);
        bits.flush();
        for &s in syms.iter().rev().skip(1) {
            ct.encode(&mut st, &mut bits, s as usize);
            bits.flush();
        }
        ct.flush(st, &mut bits);
        let stream = bits.close();
        let mut br = crate::bit::BitRev::new(&stream).expect("bitrev");
        let mut ds = dt.init_state(&mut br);
        let mut out = Vec::new();
        for i in 0..syms.len() {
            out.push(dt.peek_symbol(ds).unwrap() as u8);
            if i + 1 != syms.len() {
                ds = dt.update(ds, &mut br).unwrap();
            }
        }
        assert_eq!(out, syms);
    }
}
