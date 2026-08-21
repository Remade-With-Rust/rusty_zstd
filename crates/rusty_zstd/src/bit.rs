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

impl<'a> BitRev<'a> {
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
            let raw = read_u64_le(&src[ptr..ptr + 8]);
            Ok(Self {
                src,
                ptr,
                bit_container: shl64(raw, skip_in_last),
                bits_consumed: skip_in_last,
            })
        } else {
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
        debug_assert!(n >= 1 && n <= 56);
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
            bit_container: shl64(read_u64_le(&src[ptr..ptr + 8]), bits_consumed),
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
            self.ptr -= bytes;
            self.bits_consumed &= 7;
            if self.ptr + 8 <= self.src.len() {
                self.bit_container = read_u64_le(&self.src[self.ptr..self.ptr + 8]);
            } else {
                let mut buf = [0u8; 8];
                let n = self.src.len() - self.ptr;
                buf[..n].copy_from_slice(&self.src[self.ptr..]);
                self.bit_container = u64::from_le_bytes(buf);
            }
            self.bit_container = shl64(self.bit_container, self.bits_consumed);
            Ok(())
        } else {
            let nb = self.ptr;
            self.ptr = 0;
            self.bits_consumed -= (nb as u32) * 8;
            let mut buf = [0u8; 8];
            let n = self.src.len().min(8);
            buf[..n].copy_from_slice(&self.src[..n]);
            self.bit_container = shl64(u64::from_le_bytes(buf), self.bits_consumed);
            Ok(())
        }
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

fn read_u64_le(s: &[u8]) -> u64 {
    u64::from_le_bytes([s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7]])
}

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

    pub(crate) fn with_capacity(n: usize) -> Self {
        Self {
            buf: alloc::vec::Vec::with_capacity(n),
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
        Self { buf, container: 0, bit_pos: 0 }
    }

    pub(crate) fn add_bits(&mut self, value: u64, nb_bits: u32) {
        if nb_bits == 0 {
            return;
        }
        if self.bit_pos + nb_bits >= 64 {
            self.flush();
        }
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
