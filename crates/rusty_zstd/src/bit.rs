//! Reverse bitstream (FSE / Huffman) matching libzstd `BIT_DStream_t`.
//!
//! Bits are read from the end of the buffer toward the start. The last byte
//! contains a 1-bit end mark in its highest set bit; bits above that mark are
//! padding and are not part of the stream.

use crate::error::Error;

pub(crate) struct BitRev<'a> {
    src: &'a [u8],
    /// Index of the 8-byte window currently in `bit_container` (C `ptr`).
    ptr: usize,
    bit_container: u64,
    bits_consumed: u32,
}

/// Executed `BitRev::reload` calls. See the note inside `reload`.
#[cfg(feature = "profile")]
pub static RELOAD_CALLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Executed refills -- reloads that reached the container load rather than
/// taking one of the four early-outs. See the note inside `reload`.
#[cfg(feature = "profile")]
pub static RELOAD_REFILLS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// Read and clear the refill counter.
#[cfg(feature = "profile")]
pub fn take_reload_refills() -> u64 {
    RELOAD_REFILLS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// Read and clear the reload counter.
#[cfg(feature = "profile")]
pub fn take_reload_calls() -> u64 {
    RELOAD_CALLS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

impl<'a> BitRev<'a> {
    /// DecSeq loop anatomy (profile only): snapshot/restore the reader so an op
    /// can be executed a SECOND time and undone, leaving output byte-identical.
    /// See `dsloop.rs` -- duplication is how a ~33 ns loop body is attributed
    /// without a clock that costs 74.8 ns.
    #[cfg(feature = "dupladder")]
    #[inline(always)]
    pub(crate) fn dup_save(&self) -> (usize, u64, u32) {
        (self.ptr, self.bit_container, self.bits_consumed)
    }

    #[cfg(feature = "dupladder")]
    #[inline(always)]
    pub(crate) fn dup_restore(&mut self, s: (usize, u64, u32)) {
        self.ptr = s.0;
        self.bit_container = s.1;
        self.bits_consumed = s.2;
    }

    // The bit-engine helpers are inline(always): outlined, they compile as
    // baseline code even when called from a BMI2 twin (the shim-trap rule),
    // and the twin call-graph trace caught exactly that.
    #[inline(always)]
    pub(crate) fn new(src: &'a [u8]) -> Result<Self, Error> {
        if src.is_empty() {
            return Err(Error::Corruption);
        }
        let last = src[src.len() - 1];
        if last == 0 {
            return Err(Error::Corruption);
        }
        let highbit = 31 - (last as u32).leading_zeros();
        let skip_in_last = 8 - highbit;
        if src.len() >= 8 {
            let ptr = src.len() - 8;
            // `ptr + 8 == src.len()` by construction, so the checked range and
            // its landing pad were re-proving the line above.
            let raw = crate::simd::load_u64_le(src, ptr);
            Ok(Self {
                src,
                ptr,
                bit_container: shl64(raw, skip_in_last),
                bits_consumed: skip_in_last,
            })
        } else {
            // REFUTED, recorded: outlining this as `#[cold] fn new_short`
            // measured WORSE (+660 instructions), even though it is the exact
            // shape that won -3,854 in `reload`. The difference is DUPLICATION,
            // not rarity: `reload` is called four times per unrolled group
            // INSIDE the decode loop, so its cold tail existed in many inline
            // copies; `new` runs once per stream, so outlining only adds a
            // call. Outlining pays in proportion to how often the HOST is
            // duplicated.
            let mut buf = [0u8; 8];
            buf[..src.len()].copy_from_slice(src);
            let consumed = skip_in_last + (8 - src.len() as u32) * 8;
            Ok(Self {
                src,
                ptr: 0,
                bit_container: shl64(u64::from_le_bytes(buf), consumed),
                bits_consumed: consumed,
            })
        }
    }

    #[inline(always)]
    #[cfg_attr(not(test), allow(dead_code))]
    pub(crate) fn look_bits(&self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        self.look_bits_fast(n)
    }

    /// Next `n` bits from a left-justified container (C fast-loop `bits >> (64-n)`).
    /// `new` / `reload` / `skip_bits` keep the consumed bits shifted out the top,
    /// so this is one shift instead of `(container << consumed) >> (64-n)` every peek.
    #[inline(always)]
    pub(crate) fn look_bits_fast(&self, n: u32) -> u32 {
        debug_assert!((1..=56).contains(&n));
        (self.bit_container >> (64 - n)) as u32
    }

    #[inline(always)]
    pub(crate) fn skip_bits(&mut self, n: u32) {
        self.bit_container = shl64(self.bit_container, n);
        self.bits_consumed = self.bits_consumed.saturating_add(n);
    }

    #[inline(always)]
    pub(crate) fn read_bits(&mut self, n: u32) -> u32 {
        if n == 0 {
            return 0;
        }
        let v = self.look_bits_fast(n);
        // REFUTED, recorded: `shl64`'s `n >= 64` guard IS dead here
        // (`look_bits_fast` above only defines `1 <= n <= 63`), but replacing
        // `skip_bits` with a direct shift measured WORSE -- +228 instructions
        // crate-wide. `read_bits` is inlined widely and LLVM's range analysis
        // already folds the guard per site; hand-removing it just stops the
        // shared `skip_bits` body from being reused. Third branch-removal in
        // this file to measure worse -- bit.rs's branchy helpers are already
        // optimal in context.
        self.skip_bits(n);
        v
    }

    pub(crate) fn overflowed(&self) -> bool {
        self.bits_consumed > 64 && self.ptr == 0
    }

    /// Resume after C `HUF_decompress4X2` fast loop: `ptr` is the loaded window,
    /// `bits_consumed` is `trailing_zeros` of the left-justified register.
    #[inline(always)]
    pub(crate) fn from_window(
        src: &'a [u8],
        ptr: usize,
        bits_consumed: u32,
    ) -> Result<Self, Error> {
        if src.len() < 8 || ptr + 8 > src.len() || bits_consumed > 64 {
            return Err(Error::Corruption);
        }
        Ok(Self {
            src,
            ptr,
            // The guard above already rejected `ptr + 8 > src.len()`.
            bit_container: shl64(crate::simd::load_u64_le(src, ptr), bits_consumed),
            bits_consumed,
        })
    }

    /// Unconsumed bits from the start of `src` through the current window.
    #[allow(dead_code)]
    pub(crate) fn remaining_bits(&self) -> u64 {
        let behind = self.ptr as u64 * 8;
        let in_win = u64::from(64u32.saturating_sub(self.bits_consumed.min(64)));
        behind + in_win
    }

    #[inline(always)]
    pub(crate) fn reload(&mut self) -> Result<(), Error> {
        // D9/D12 ADJUDICATION INSTRUMENT (profile only): count EXECUTED
        // reloads. The question "was deleting the X1 unrolls a mistake" is a
        // work-count question, not a clock question -- the unroll called this
        // once per N positions and the tail loop calls it once per position.
        #[cfg(feature = "profile")]
        RELOAD_CALLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if self.bits_consumed > 64 {
            return Err(Error::Corruption);
        }
        if self.src.len() < 8 {
            return Ok(());
        }
        let bytes = (self.bits_consumed / 8) as usize;
        if bytes == 0 {
            return Ok(());
        }
        if self.ptr >= bytes {
            #[cfg(feature = "profile")]
            RELOAD_REFILLS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            self.ptr -= bytes;
            self.bits_consumed &= 7;
            if self.ptr + 8 <= self.src.len() {
                // The `if` above IS the bound, so the slice form re-derived it:
                // `&self.src[ptr..ptr + 8]` is a checked range with its own
                // `slice_index_fail` pad, feeding a `read_u64_le` that then
                // indexes eight times. `simd::load_u64_le` is a safe function
                // whose `unsafe` is contained, and it takes the offset directly.
                self.bit_container = crate::simd::load_u64_le(self.src, self.ptr);
            } else {
                self.bit_container = self.tail_word();
            }
            // `bits_consumed &= 7` two lines up, so it is 0..=7 here and
            // `shl64`'s `n >= 64` guard is DEAD -- a compare and a select per
            // refill, and `reload` runs four times per unrolled group in the
            // 4-stream Huffman loop. The guard stays on every OTHER `shl64`
            // caller, where the count genuinely can reach 64.
            debug_assert!(self.bits_consumed < 8);
            self.bit_container <<= self.bits_consumed;
            Ok(())
        } else {
            self.rewind_to_start();
            Ok(())
        }
    }

    /// Fewer than 8 bytes left ahead of `ptr`: assemble the last word by hand.
    ///
    /// `#[cold]` + `#[inline(never)]`. `reload` is `#[inline(always)]` and is
    /// reproduced at every call site in the 4-stream Huffman loop and the
    /// sequence loop, so this end-of-stream fallback was being stamped out
    /// with it -- an 8-byte zeroed buffer, a `copy_from_slice` and a
    /// `from_le_bytes`, in every copy, for a case that happens once per
    /// STREAM.
    #[cold]
    #[inline(never)]
    /// D21: three checks become one. `self.src.len() - self.ptr` could
    /// underflow, `buf[..n]` could exceed 8, and `&self.src[self.ptr..]` could
    /// be out of range -- each its own test and panic pad, in a function the
    /// bit reader calls on every short reload.
    ///
    /// Taking the tail ONCE through `get` proves the offset, and clamping `n`
    /// to `min(8)` makes both remaining slices provably in range: `n <=
    /// buf.len()` and `n <= tail.len()`, so LLVM drops the tests entirely.
    ///
    /// The clamp never fires: the sole caller reaches here only when
    /// `ptr + 8 > src.len()`, i.e. `n < 8`. It exists to make the bound STATIC
    /// rather than to change behaviour -- the C1/C2 lesson, applied to a hot
    /// path instead of a header parser.
    fn tail_word(&self) -> u64 {
        let mut buf = [0u8; 8];
        let tail = self.src.get(self.ptr..).unwrap_or(&[]);
        let n = tail.len().min(8);
        buf[..n].copy_from_slice(&tail[..n]);
        u64::from_le_bytes(buf)
    }

    /// The reader has consumed past the start of `src`: clamp to 0 and refill.
    /// Also once per stream, also stamped into every inline copy of `reload`.
    #[cold]
    #[inline(never)]
    fn rewind_to_start(&mut self) {
        let nb = self.ptr;
        self.ptr = 0;
        self.bits_consumed -= (nb as u32) * 8;
        let mut buf = [0u8; 8];
        let n = self.src.len().min(8);
        buf[..n].copy_from_slice(&self.src[..n]);
        self.bit_container = shl64(u64::from_le_bytes(buf), self.bits_consumed);
    }
}

fn shl64(v: u64, n: u32) -> u64 {
    if n >= 64 {
        0
    } else {
        v << n
    }
}

fn ones(n: u32) -> u32 {
    if n >= 32 {
        u32::MAX
    } else {
        (1u32 << n).wrapping_sub(1)
    }
}

// `read_u64_le` DELETED: it assembled a u64 from EIGHT separately-indexed byte
// loads, and all three callers had already proven `ptr + 8 <= len` on the line
// above. They now use `simd::load_u64_le`, which is the same primitive the
// encoder and the match finders have used for bricks -- one unaligned load,
// with its `unsafe` contained in the simd island.

/// Forward bit writer matching libzstd `BIT_CStream_t` (little-endian container).
#[cfg(feature = "alloc")]
pub(crate) struct BitCStream {
    buf: alloc::vec::Vec<u8>,
    container: u64,
    bit_pos: u32,
}

#[cfg(feature = "alloc")]
impl BitCStream {
    pub(crate) fn new() -> Self {
        Self {
            buf: alloc::vec::Vec::new(),
            container: 0,
            bit_pos: 0,
        }
    }

    /// Frame-scratch constructor: reuse a caller-kept buffer (cleared here)
    /// so the per-block bitstream costs no allocation after warm-up.
    pub(crate) fn from_vec(mut buf: alloc::vec::Vec<u8>, want: usize) -> Self {
        buf.clear();
        if buf.capacity() < want {
            buf = alloc::vec::Vec::with_capacity(want);
        }
        Self {
            buf,
            container: 0,
            bit_pos: 0,
        }
    }

    pub(crate) fn add_bits(&mut self, value: u64, nb_bits: u32) {
        if nb_bits == 0 {
            return;
        }
        if self.bit_pos + nb_bits >= 64 {
            self.flush();
        }
        // REFUTED, recorded: replacing this with a branchless
        // `(1u64 << nb_bits) - 1` under a `debug_assert!(nb_bits < 64)`
        // measured WORSE -- +36 instructions crate-wide. `add_bits` is
        // inlined, and at the call sites where `nb_bits` is a compile-time
        // constant the BRANCHY form folds the whole select away; the
        // "branchless" one does not fold as well.
        let mask = if nb_bits >= 64 {
            u64::MAX
        } else {
            (1u64 << nb_bits) - 1
        };
        self.container |= (value & mask) << self.bit_pos;
        self.bit_pos += nb_bits;
    }

    /// Huffman fast path: `1 <= nb_bits <= 11` and `bit_pos + nb_bits < 64`.
    #[inline(always)]
    pub(crate) fn add_bits_huff(&mut self, code: u64, nb_bits: u32) {
        debug_assert!(nb_bits > 0 && nb_bits <= 11);
        debug_assert!(self.bit_pos + nb_bits < 64);
        self.container |= code << self.bit_pos;
        self.bit_pos += nb_bits;
    }

    /// Remaining container room for one Huffman code (fill dispatch).
    #[inline(always)]
    pub(crate) fn huff_fits(&self, nb_bits: u32) -> bool {
        self.bit_pos + nb_bits < 64
    }

    #[inline(always)]
    /// BRICK 68: FIXED-WIDTH flush.
    ///
    /// This wrote `buf.extend_from_slice(&bytes[..nbytes])` with `nbytes` a
    /// RUNTIME 0..8 -- i.e. a memcpy CALL per flush -- and `flush` runs once
    /// per K-group (~every 9 symbols). On mozilla's 24.4 MB of literals that is
    /// ~2.7M variable-length memcpys; `encode_stream` carried 9 memcpy call
    /// sites because of it.
    ///
    /// Store 8 bytes into spare capacity unconditionally, then commit only
    /// `nbytes` -- the trick bricks 36/37 proved on the decode copies. Output is
    /// byte-identical: bytes past `nbytes` are never published.
    #[allow(unsafe_code)]
    pub(crate) fn flush(&mut self) {
        let nbytes = (self.bit_pos / 8) as usize;
        if nbytes == 0 {
            return;
        }
        let bytes = self.container.to_le_bytes();
        self.buf.reserve(8);
        // SAFETY: `reserve(8)` guarantees 8 writable bytes at `len()`, and
        // `nbytes <= 8`, so `set_len` never exceeds the reserved capacity.
        unsafe {
            let dst = self.buf.as_mut_ptr().add(self.buf.len());
            core::ptr::copy_nonoverlapping(bytes.as_ptr(), dst, 8);
            self.buf.set_len(self.buf.len() + nbytes);
        }
        self.container >>= nbytes * 8;
        self.bit_pos &= 7;
    }

    /// End mark `1` plus zero-pad, matching `BIT_closeCStream`.
    #[inline(always)]
    pub(crate) fn close(mut self) -> alloc::vec::Vec<u8> {
        self.add_bits(1, 1);
        self.flush();
        if self.bit_pos > 0 {
            self.buf.push(self.container as u8);
        }
        self.buf
    }
}

/// Forward little-endian bit reader (FSE NCount / Huffman header weights).
pub(crate) struct BitFwd<'a> {
    src: &'a [u8],
    pos: usize,
    buf: u64,
    nbits: u32,
    bits_read: u32,
}

impl<'a> BitFwd<'a> {
    pub(crate) fn new(src: &'a [u8]) -> Self {
        Self {
            src,
            pos: 0,
            buf: 0,
            nbits: 0,
            bits_read: 0,
        }
    }

    fn refill(&mut self) {
        // FIXED-WIDTH REFILL. The loop below adds ONE bounds-checked byte per
        // iteration, up to eight per `peek`, and `peek` runs once per symbol of
        // every FSE ncount header. When eight bytes are readable, the same
        // bytes can be taken in one unaligned load.
        //
        // Byte-identical by algebra: the loop ORs `src[pos+i] << (nbits + 8i)`
        // for `i in 0..k`, and that IS the k-byte little-endian word shifted
        // left by `nbits`. `k` is exactly the number of iterations the loop
        // would run -- `nbits` climbs by 8 while it is `<= 56`.
        if self.nbits <= 56 && self.pos + 8 <= self.src.len() {
            let k = (56 - self.nbits) / 8 + 1;
            let word = crate::simd::load_u64_le(self.src, self.pos);
            // `k == 8` only when `nbits == 0`, and `1u64 << 64` would overflow.
            let masked = if k >= 8 {
                word
            } else {
                word & ((1u64 << (k * 8)) - 1)
            };
            self.buf |= masked << self.nbits;
            self.nbits += k * 8;
            self.pos += k as usize;
            debug_assert!(self.nbits <= 64);
            return;
        }
        // Tail: fewer than eight bytes left. One byte at a time, as before.
        while self.nbits <= 56 && self.pos < self.src.len() {
            self.buf |= u64::from(self.src[self.pos]) << self.nbits;
            self.nbits += 8;
            self.pos += 1;
        }
    }

    pub(crate) fn peek(&mut self, n: u32) -> Result<u32, Error> {
        if n == 0 {
            return Ok(0);
        }
        self.refill();
        if n > self.nbits {
            return Err(Error::Corruption);
        }
        Ok((self.buf as u32) & ones(n))
    }

    #[inline(always)]
    pub(crate) fn get(&mut self, n: u32) -> Result<u32, Error> {
        let v = self.peek(n)?;
        self.buf >>= n;
        self.nbits -= n;
        self.bits_read += n;
        Ok(v)
    }

    /// Bytes consumed, rounded up to a whole byte.
    pub(crate) fn bytes_consumed(&self) -> usize {
        (self.bits_read.div_ceil(8)) as usize
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flush_writes_container_words() {
        let mut bits = BitCStream::new();
        bits.add_bits(0x0123_4567_89AB_CDEF, 56);
        bits.flush();
        bits.add_bits(0x11, 8);
        let out = bits.close();
        assert!(out.len() >= 8);
        assert_eq!(&out[..7], &[0xEF, 0xCD, 0xAB, 0x89, 0x67, 0x45, 0x23]);
    }

    #[test]
    fn look_bits_fast_zero_pads_at_start() {
        // One payload bit 1, then end mark. Last (only) byte = 0b0000_0011.
        let src = [0x03u8];
        let br = BitRev::new(&src).unwrap();
        // Remaining payload is 1 bit (the low 1). A 5-bit Huffman-style peek
        // must place that bit in the high side and zero-pad the rest.
        let v = br.look_bits(5);
        assert_eq!(v, 1 << 4, "got {v:#b}");
    }

    #[test]
    fn left_justified_look_matches_c_shift() {
        let src: Vec<u8> = (0u8..=255).collect();
        let last = *src.last().unwrap();
        let highbit = 31 - (last as u32).leading_zeros();
        let skip = 8 - highbit;
        let raw = u64::from_le_bytes(src[src.len() - 8..].try_into().unwrap());
        let br = BitRev::new(&src).unwrap();
        for n in 1..=16u32 {
            let got = br.look_bits(n);
            let want = crate::simd::look_n_bits_shift(raw, skip, n);
            assert_eq!(got, want, "n={n}");
        }
    }
}
