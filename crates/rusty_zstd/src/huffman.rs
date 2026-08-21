//! Huffman tree description and stream decode (RFC 8878 section 4.2).

use crate::bit::BitRev;
use crate::error::Error;
use crate::fse;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

const MAX_BITS: u8 = 11;
/// C `HUF_DECODER_FAST_TABLELOG`. X1/X2 DTables are stretched to this so the
/// fast 4X2 loop can index with `bits >> 53`.
const FAST_TABLELOG: u8 = 11;

#[derive(Clone, Debug)]
pub(crate) struct HuffmanTable {
    /// `1 << max_bits` entries: low 8 = symbol, high 8 = nbits. X1 oracle.
    table: Vec<u16>,
    /// C `HUF_DEltX2`: `seq16 | nbits<<16 | length<<24`. length is 1 or 2.
    table_x2: Vec<u32>,
    max_bits: u8,
}

impl HuffmanTable {
    pub(crate) fn decode_stream(&self, src: &[u8], dst: &mut [u8]) -> Result<(), Error> {
        if dst.is_empty() {
            return Ok(());
        }
        let mut br = BitRev::new(src)?;
        if self.use_x2(dst.len(), src.len()) {
            self.decode_into_x2(&mut br, dst)
        } else {
            self.decode_into_x1(&mut br, dst)
        }
    }

    /// Per-symbol reload + look + read. Oracle for the unroll / skip_bits path.
    #[cfg(test)]
    pub(crate) fn decode_stream_scalar(&self, src: &[u8], dst: &mut [u8]) -> Result<(), Error> {
        if dst.is_empty() {
            return Ok(());
        }
        let mut br = BitRev::new(src)?;
        let max = u32::from(self.max_bits);
        let dt = self.table.as_slice();
        // Required by `decode_one`/`write_x2`: `saturating_sub` would turn an
        // empty table into `mask == 0` and then index it. Checked once per call.
        if dt.is_empty() {
            return Err(Error::Corruption);
        }
        let mask = dt.len() - 1;
        for slot in dst.iter_mut() {
            let _ = br.reload();
            let e = dt[br.look_bits(max) as usize & mask];
            let nbits = (e >> 8) as u8;
            if nbits == 0 {
                return Err(Error::Corruption);
            }
            br.read_bits(u32::from(nbits));
            *slot = e as u8;
        }
        Ok(())
    }

    /// SAFETY for the lookup below (and the same argument serves `write_x2`):
    /// `mask` is `dt.len() - 1` and every DTable is built `1 << table_log`
    /// entries -- a non-empty power of two -- so `idx & mask < dt.len()`. The
    /// callers check `dt.is_empty()` once per call, because `saturating_sub`
    /// would otherwise turn an empty table into `mask == 0` and index it.
    ///
    /// This runs once per LITERAL, which is why it is worth proving: it was 7 of
    /// the 63 bounds checks on the Huffman decode path.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn decode_one(br: &mut BitRev<'_>, dt: &[u16], mask: usize, max: u32) -> Result<u8, Error> {
        debug_assert!(!dt.is_empty() && dt.len().is_power_of_two() && mask == dt.len() - 1);
        let e = *unsafe { dt.get_unchecked(br.look_bits_fast(max) as usize & mask) };
        let nbits = (e >> 8) as u8;
        if nbits == 0 {
            return Err(Error::Corruption);
        }
        br.skip_bits(u32::from(nbits));
        Ok(e as u8)
    }

    fn use_x2(&self, dst_size: usize, src_size: usize) -> bool {
        self.table_x2.len() == self.table.len() && select_x2(dst_size, src_size)
    }

    fn decode_into_x1(&self, br: &mut BitRev<'_>, dst: &mut [u8]) -> Result<(), Error> {
        let max = u32::from(self.max_bits);
        let dt = self.table.as_slice();
        // Required by `decode_one`/`write_x2`: `saturating_sub` would turn an
        // empty table into `mask == 0` and then index it. Checked once per call.
        if dt.is_empty() {
            return Err(Error::Corruption);
        }
        let mask = dt.len() - 1;
        let n = dst.len();
        let mut i = 0usize;
        // The loop guard is `i + 5 <= n`, so `i + 4 <= n - 1`: all five writes
        // are in range by the condition that admitted the iteration.
        while i + 5 <= n {
            let _ = br.reload();
            debug_assert!(i + 4 < n);
            for k in 0..5 {
                let v = Self::decode_one(br, dt, mask, max)?;
                #[allow(unsafe_code)]
                unsafe {
                    *dst.get_unchecked_mut(i + k) = v;
                }
            }
            i += 5;
        }
        while i < n {
            let _ = br.reload();
            dst[i] = Self::decode_one(br, dt, mask, max)?;
            i += 1;
        }
        Ok(())
    }

    /// C `HUF_decodeStreamX2`: one peek can emit 1 or 2 symbols.
    fn decode_into_x2(&self, br: &mut BitRev<'_>, dst: &mut [u8]) -> Result<(), Error> {
        let max = u32::from(self.max_bits);
        let dt = self.table_x2.as_slice();
        // Required by `decode_one`/`write_x2`: `saturating_sub` would turn an
        // empty table into `mask == 0` and then index it. Checked once per call.
        if dt.is_empty() {
            return Err(Error::Corruption);
        }
        let mask = dt.len() - 1;
        let n = dst.len();
        let mut i = 0usize;
        while i + 10 <= n {
            let _ = br.reload();
            i += Self::write_x2(br, dt, mask, max, dst, i);
            i += Self::write_x2(br, dt, mask, max, dst, i);
            i += Self::write_x2(br, dt, mask, max, dst, i);
            i += Self::write_x2(br, dt, mask, max, dst, i);
            i += Self::write_x2(br, dt, mask, max, dst, i);
        }
        while i + 2 <= n {
            let _ = br.reload();
            i += Self::write_x2(br, dt, mask, max, dst, i);
        }
        while i < n {
            let _ = br.reload();
            dst[i] = Self::decode_one(
                br,
                self.table.as_slice(),
                self.table.len().saturating_sub(1),
                max,
            )?;
            i += 1;
        }
        Ok(())
    }

    /// Peek X2, skip `nbits`, write 2 bytes (C memcpy of `sequence`). Advance by `length`.
    /// Caller: `i + 2 <= dst.len()`. A length-1 extra byte is overwritten by the next write
    /// or by the X1 tail.
    #[inline(always)]
    fn write_x2(
        br: &mut BitRev<'_>,
        dt: &[u32],
        mask: usize,
        max: u32,
        dst: &mut [u8],
        i: usize,
    ) -> usize {
        // SAFETY: same masked-table argument as `decode_one`. For the two
        // output bytes, every caller advances `i` under a `i + 10 <= dst.len()`
        // loop guard and emits at most 5 symbols of 2 bytes per stream per pass,
        // so `i <= dst.len() - 2` at every write -- that 10-byte headroom is
        // exactly what the guard reserves.
        debug_assert!(!dt.is_empty() && mask == dt.len() - 1);
        debug_assert!(i + 1 < dst.len());
        #[allow(unsafe_code)]
        let e = *unsafe { dt.get_unchecked(br.look_bits_fast(max) as usize & mask) };
        debug_assert!(((e >> 16) & 0xff) != 0);
        br.skip_bits((e >> 16) & 0xff);
        #[allow(unsafe_code)]
        unsafe {
            *dst.get_unchecked_mut(i) = e as u8;
            *dst.get_unchecked_mut(i + 1) = (e >> 8) as u8;
        }
        (e >> 24) as usize
    }

    /// C `HUF_decompress4X2`: four readers, X2 DTable, independent output cursors.
    /// Sequential 4x `decode_stream` is the oracle (`decode_4x_matches_sequential`).
    pub(crate) fn decode_4x(
        &self,
        s0: &[u8],
        s1: &[u8],
        s2: &[u8],
        s3: &[u8],
        d0: &mut [u8],
        d1: &mut [u8],
        d2: &mut [u8],
        d3: &mut [u8],
    ) -> Result<(), Error> {
        // The seq-loop precedent (621a140): `avx2` does not imply BMI2, and
        // this loop is made of variable shifts. The twin compiles the SAME
        // body with shrx/shlx/bzhi available; byte-identity by construction.
        #[cfg(all(target_arch = "x86_64", feature = "std"))]
        if crate::simd::has_bmi2() {
            // SAFETY: guarded by runtime CPUID; the body is identical.
            #[allow(unsafe_code)]
            return unsafe { self.decode_4x_bmi2(s0, s1, s2, s3, d0, d1, d2, d3) };
        }
        self.decode_4x_inner(s0, s1, s2, s3, d0, d1, d2, d3)
    }

    /// The BMI2-compiled twin of `decode_4x_inner`.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    #[target_feature(enable = "bmi2,lzcnt")]
    #[allow(clippy::too_many_arguments)]
    #[allow(unsafe_code)]
    unsafe fn decode_4x_bmi2(
        &self,
        s0: &[u8],
        s1: &[u8],
        s2: &[u8],
        s3: &[u8],
        d0: &mut [u8],
        d1: &mut [u8],
        d2: &mut [u8],
        d3: &mut [u8],
    ) -> Result<(), Error> {
        self.decode_4x_inner(s0, s1, s2, s3, d0, d1, d2, d3)
    }

    #[inline(always)]
    #[allow(clippy::too_many_arguments)]
    fn decode_4x_inner(
        &self,
        s0: &[u8],
        s1: &[u8],
        s2: &[u8],
        s3: &[u8],
        d0: &mut [u8],
        d1: &mut [u8],
        d2: &mut [u8],
        d3: &mut [u8],
    ) -> Result<(), Error> {
        if s0.is_empty() || s1.is_empty() || s2.is_empty() || s3.is_empty() {
            return Err(Error::Corruption);
        }
        let dst_size = d0.len() + d1.len() + d2.len() + d3.len();
        let src_size = s0.len() + s1.len() + s2.len() + s3.len();
        if !self.use_x2(dst_size, src_size) {
            return self.decode_4x_x1(s0, s1, s2, s3, d0, d1, d2, d3);
        }
        match self.fast_4x2(s0, s1, s2, s3, d0, d1, d2, d3)? {
            Some(st) => {
                let mut b0 = BitRev::from_window(s0, st.ip0, st.c0)?;
                let mut b1 = BitRev::from_window(s1, st.ip1, st.c1)?;
                let mut b2 = BitRev::from_window(s2, st.ip2, st.c2)?;
                let mut b3 = BitRev::from_window(s3, st.ip3, st.c3)?;
                self.decode_into_x2(&mut b0, &mut d0[st.op0..])?;
                self.decode_into_x2(&mut b1, &mut d1[st.op1..])?;
                self.decode_into_x2(&mut b2, &mut d2[st.op2..])?;
                self.decode_into_x2(&mut b3, &mut d3[st.op3..])?;
                return Ok(());
            }
            None => {}
        }
        let mut b0 = BitRev::new(s0)?;
        let mut b1 = BitRev::new(s1)?;
        let mut b2 = BitRev::new(s2)?;
        let mut b3 = BitRev::new(s3)?;
        let max = u32::from(self.max_bits);
        let dt = self.table_x2.as_slice();
        // Required by `decode_one`/`write_x2`: `saturating_sub` would turn an
        // empty table into `mask == 0` and then index it. Checked once per call.
        if dt.is_empty() {
            return Err(Error::Corruption);
        }
        let mask = dt.len() - 1;
        let mut i0 = 0usize;
        let mut i1 = 0usize;
        let mut i2 = 0usize;
        let mut i3 = 0usize;
        while i0 + 10 <= d0.len()
            && i1 + 10 <= d1.len()
            && i2 + 10 <= d2.len()
            && i3 + 10 <= d3.len()
        {
            let _ = b0.reload();
            let _ = b1.reload();
            let _ = b2.reload();
            let _ = b3.reload();
            i0 += Self::write_x2(&mut b0, dt, mask, max, d0, i0);
            i1 += Self::write_x2(&mut b1, dt, mask, max, d1, i1);
            i2 += Self::write_x2(&mut b2, dt, mask, max, d2, i2);
            i3 += Self::write_x2(&mut b3, dt, mask, max, d3, i3);
            i0 += Self::write_x2(&mut b0, dt, mask, max, d0, i0);
            i1 += Self::write_x2(&mut b1, dt, mask, max, d1, i1);
            i2 += Self::write_x2(&mut b2, dt, mask, max, d2, i2);
            i3 += Self::write_x2(&mut b3, dt, mask, max, d3, i3);
            i0 += Self::write_x2(&mut b0, dt, mask, max, d0, i0);
            i1 += Self::write_x2(&mut b1, dt, mask, max, d1, i1);
            i2 += Self::write_x2(&mut b2, dt, mask, max, d2, i2);
            i3 += Self::write_x2(&mut b3, dt, mask, max, d3, i3);
            i0 += Self::write_x2(&mut b0, dt, mask, max, d0, i0);
            i1 += Self::write_x2(&mut b1, dt, mask, max, d1, i1);
            i2 += Self::write_x2(&mut b2, dt, mask, max, d2, i2);
            i3 += Self::write_x2(&mut b3, dt, mask, max, d3, i3);
            i0 += Self::write_x2(&mut b0, dt, mask, max, d0, i0);
            i1 += Self::write_x2(&mut b1, dt, mask, max, d1, i1);
            i2 += Self::write_x2(&mut b2, dt, mask, max, d2, i2);
            i3 += Self::write_x2(&mut b3, dt, mask, max, d3, i3);
        }
        self.decode_into_x2(&mut b0, &mut d0[i0..])?;
        self.decode_into_x2(&mut b1, &mut d1[i1..])?;
        self.decode_into_x2(&mut b2, &mut d2[i2..])?;
        self.decode_into_x2(&mut b3, &mut d3[i3..])?;
        Ok(())
    }

    /// C `HUF_decompress4X2_usingDTable_internal_fast_c_loop`.
    /// Left-justified container, peek `bits >> 53` (tableLog=11), reload via CTZ.
    /// `None` = use the BIT_DStream X2 loop (short streams / not 64-bit).
    fn fast_4x2(
        &self,
        s0: &[u8],
        s1: &[u8],
        s2: &[u8],
        s3: &[u8],
        d0: &mut [u8],
        d1: &mut [u8],
        d2: &mut [u8],
        d3: &mut [u8],
    ) -> Result<Option<Fast4x2>, Error> {
        if !cfg!(target_pointer_width = "64") {
            return Ok(None);
        }
        if self.max_bits != FAST_TABLELOG || self.table_x2.len() != 1 << FAST_TABLELOG {
            return Ok(None);
        }
        if s0.len() < 8 || s1.len() < 8 || s2.len() < 8 || s3.len() < 8 {
            return Ok(None);
        }
        let dt = self.table_x2.as_slice();
        let mut ip0 = s0.len() - 8;
        let mut ip1 = s1.len() - 8;
        let mut ip2 = s2.len() - 8;
        let mut ip3 = s3.len() - 8;
        let mut bits0 = init_fast_dstream(s0, ip0);
        let mut bits1 = init_fast_dstream(s1, ip1);
        let mut bits2 = init_fast_dstream(s2, ip2);
        let mut bits3 = init_fast_dstream(s3, ip3);
        let mut op0 = 0usize;
        let mut op1 = 0usize;
        let mut op2 = 0usize;
        let mut op3 = 0usize;
        loop {
            let mut iters = ip0 / 7;
            iters = iters.min(ip1 / 7).min(ip2 / 7).min(ip3 / 7);
            iters = iters
                .min(d0.len().saturating_sub(op0) / 10)
                .min(d1.len().saturating_sub(op1) / 10)
                .min(d2.len().saturating_sub(op2) / 10)
                .min(d3.len().saturating_sub(op3) / 10);
            if iters == 0 {
                break;
            }
            let olimit = op3 + iters * 5;
            while op3 < olimit {
                // 5 X2 symbols from streams 0..=2 (stream 3 during reload).
                x2_fast_sym(&mut bits0, &mut op0, d0, dt);
                x2_fast_sym(&mut bits1, &mut op1, d1, dt);
                x2_fast_sym(&mut bits2, &mut op2, d2, dt);
                x2_fast_sym(&mut bits0, &mut op0, d0, dt);
                x2_fast_sym(&mut bits1, &mut op1, d1, dt);
                x2_fast_sym(&mut bits2, &mut op2, d2, dt);
                x2_fast_sym(&mut bits0, &mut op0, d0, dt);
                x2_fast_sym(&mut bits1, &mut op1, d1, dt);
                x2_fast_sym(&mut bits2, &mut op2, d2, dt);
                x2_fast_sym(&mut bits0, &mut op0, d0, dt);
                x2_fast_sym(&mut bits1, &mut op1, d1, dt);
                x2_fast_sym(&mut bits2, &mut op2, d2, dt);
                x2_fast_sym(&mut bits0, &mut op0, d0, dt);
                x2_fast_sym(&mut bits1, &mut op1, d1, dt);
                x2_fast_sym(&mut bits2, &mut op2, d2, dt);
                x2_fast_sym(&mut bits3, &mut op3, d3, dt);
                x2_fast_sym(&mut bits3, &mut op3, d3, dt);
                reload_fast(&mut bits0, &mut ip0, s0);
                x2_fast_sym(&mut bits3, &mut op3, d3, dt);
                reload_fast(&mut bits1, &mut ip1, s1);
                x2_fast_sym(&mut bits3, &mut op3, d3, dt);
                reload_fast(&mut bits2, &mut ip2, s2);
                x2_fast_sym(&mut bits3, &mut op3, d3, dt);
                reload_fast(&mut bits3, &mut ip3, s3);
            }
        }
        Ok(Some(Fast4x2 {
            op0,
            op1,
            op2,
            op3,
            ip0,
            ip1,
            ip2,
            ip3,
            c0: bits0.trailing_zeros(),
            c1: bits1.trailing_zeros(),
            c2: bits2.trailing_zeros(),
            c3: bits3.trailing_zeros(),
        }))
    }

    fn decode_4x_x1(
        &self,
        s0: &[u8],
        s1: &[u8],
        s2: &[u8],
        s3: &[u8],
        d0: &mut [u8],
        d1: &mut [u8],
        d2: &mut [u8],
        d3: &mut [u8],
    ) -> Result<(), Error> {
        let mut b0 = BitRev::new(s0)?;
        let mut b1 = BitRev::new(s1)?;
        let mut b2 = BitRev::new(s2)?;
        let mut b3 = BitRev::new(s3)?;
        let max = u32::from(self.max_bits);
        let dt = self.table.as_slice();
        // Required by `decode_one`/`write_x2`: `saturating_sub` would turn an
        // empty table into `mask == 0` and then index it. Checked once per call.
        if dt.is_empty() {
            return Err(Error::Corruption);
        }
        let mask = dt.len() - 1;
        let n = d0.len().min(d1.len()).min(d2.len()).min(d3.len());
        let mut i = 0usize;
        while i + 4 <= n {
            let _ = b0.reload();
            let _ = b1.reload();
            let _ = b2.reload();
            let _ = b3.reload();
            // `n` is the MINIMUM of the four output lengths and the guard is
            // `i + 4 <= n`, so `i + 3` is in range for every stream. Stream
            // order within each k is preserved exactly (b0, b1, b2, b3), which
            // is what keeps the four bit readers in step.
            debug_assert!(i + 3 < n);
            for k in 0..4 {
                let v0 = Self::decode_one(&mut b0, dt, mask, max)?;
                let v1 = Self::decode_one(&mut b1, dt, mask, max)?;
                let v2 = Self::decode_one(&mut b2, dt, mask, max)?;
                let v3 = Self::decode_one(&mut b3, dt, mask, max)?;
                #[allow(unsafe_code)]
                unsafe {
                    *d0.get_unchecked_mut(i + k) = v0;
                    *d1.get_unchecked_mut(i + k) = v1;
                    *d2.get_unchecked_mut(i + k) = v2;
                    *d3.get_unchecked_mut(i + k) = v3;
                }
            }
            i += 4;
        }
        while i < n {
            let _ = b0.reload();
            let _ = b1.reload();
            let _ = b2.reload();
            let _ = b3.reload();
            d0[i] = Self::decode_one(&mut b0, dt, mask, max)?;
            d1[i] = Self::decode_one(&mut b1, dt, mask, max)?;
            d2[i] = Self::decode_one(&mut b2, dt, mask, max)?;
            d3[i] = Self::decode_one(&mut b3, dt, mask, max)?;
            i += 1;
        }
        self.decode_into_x1(&mut b0, &mut d0[n..])?;
        self.decode_into_x1(&mut b1, &mut d1[n..])?;
        self.decode_into_x1(&mut b2, &mut d2[n..])?;
        self.decode_into_x1(&mut b3, &mut d3[n..])?;
        Ok(())
    }
}

/// Parse Huffman_Tree_Description at the start of `src`. Returns (table, bytes used).
pub(crate) fn read_table(src: &[u8]) -> Result<(HuffmanTable, usize), Error> {
    if src.is_empty() {
        return Err(Error::Corruption);
    }
    let header = src[0];
    let (weights, used) = if header >= 128 {
        let nsym = header as usize - 127;
        let nbytes = nsym.div_ceil(2);
        if 1 + nbytes > src.len() {
            return Err(Error::Corruption);
        }
        let mut w = vec![0u8; nsym];
        for i in 0..nsym {
            // SAFETY: guarded directly above -- `nbytes == nsym.div_ceil(2)` and
            // `i < nsym` give `1 + i / 2 < 1 + nbytes <= src.len()`.
            debug_assert!(1 + i / 2 < src.len());
            #[allow(unsafe_code)]
            let b = *unsafe { src.get_unchecked(1 + i / 2) };
            w[i] = if i % 2 == 0 { b >> 4 } else { b & 0x0F };
        }
        (w, 1 + nbytes)
    } else {
        let csize = header as usize;
        if csize == 0 || 1 + csize > src.len() {
            return Err(Error::Corruption);
        }
        let (w, _) = fse::decompress_weights(&src[1..1 + csize], 255)?;
        (w, 1 + csize)
    };
    let table = table_from_weights(&weights)?;
    Ok((table, used))
}

fn table_from_weights(weights_wo_last: &[u8]) -> Result<HuffmanTable, Error> {
    if weights_wo_last.is_empty() {
        return Err(Error::Corruption);
    }
    let mut rank = [0u32; 13];
    let mut weight_total = 0u32;
    for &w in weights_wo_last {
        if w > MAX_BITS {
            return Err(Error::Corruption);
        }
        // SAFETY: `w > MAX_BITS` (11) was rejected just above; `rank` is [_; 13].
        debug_assert!((w as usize) < rank.len());
        #[allow(unsafe_code)]
        unsafe {
            *rank.get_unchecked_mut(w as usize) += 1;
        }
        if w > 0 {
            weight_total += 1 << (w - 1);
        }
    }
    if weight_total == 0 {
        return Err(Error::Corruption);
    }
    let table_log = (31 - weight_total.leading_zeros() + 1) as u8;
    if table_log > MAX_BITS {
        return Err(Error::Corruption);
    }
    let total = 1u32 << table_log;
    let rest = total - weight_total;
    if rest == 0 || (rest & (rest - 1)) != 0 {
        return Err(Error::Corruption);
    }
    let last_weight = (31 - rest.leading_zeros() + 1) as u8;
    if last_weight > MAX_BITS {
        return Err(Error::Corruption);
    }
    let mut weights = Vec::from(weights_wo_last);
    weights.push(last_weight);
    // SAFETY: `last_weight > MAX_BITS` was rejected above; `rank` is [_; 13].
    debug_assert!((last_weight as usize) < rank.len());
    #[allow(unsafe_code)]
    unsafe {
        *rank.get_unchecked_mut(last_weight as usize) += 1;
    }
    if rank[1] < 2 || rank[1] % 2 != 0 {
        return Err(Error::Corruption);
    }

    // C HUF_readDTableX1: fill consecutive slots by increasing weight.
    let table_size = 1usize << table_log;
    let mut table = vec![0u16; table_size];
    let mut symbols = vec![0u8; weights.len()];
    let mut rank_start = [0usize; 13];
    let mut acc = 0usize;
    // SAFETY for the weight-indexed arrays here and below: every weight was
    // rejected above unless `w <= MAX_BITS` (11); `table_log > MAX_BITS` is
    // rejected; so is `last_weight > MAX_BITS`. All three arrays are `[_; 13]`,
    // so an index of at most 11 is in range. LLVM cannot carry three separate
    // validations this far.
    debug_assert!(table_log as usize <= MAX_BITS as usize);
    for w in 0..=table_log as usize {
        #[allow(unsafe_code)]
        unsafe {
            *rank_start.get_unchecked_mut(w) = acc;
            acc += *rank.get_unchecked(w) as usize;
        }
    }
    let mut rs = rank_start;
    for (s, &w) in weights.iter().enumerate() {
        if w == 0 {
            continue;
        }
        debug_assert!((w as usize) < rs.len());
        #[allow(unsafe_code)]
        let slot = *unsafe { rs.get_unchecked(w as usize) };
        if slot >= symbols.len() {
            return Err(Error::Corruption);
        }
        symbols[slot] = s as u8;
        #[allow(unsafe_code)]
        unsafe {
            *rs.get_unchecked_mut(w as usize) += 1;
        }
    }

    let mut pos = 0usize;
    // Walk `symbols` with an ITERATOR rather than an index. The counting
    // argument that keeps `sym_i` in range -- `rank[0] + sum(rank[1..]) ==
    // weights.len() == symbols.len()` -- is true but spans the whole function,
    // so LLVM re-checks it on every symbol. An iterator states it structurally
    // and needs no unsafe; a malformed table now yields Corruption instead of a
    // panic, which is the better failure anyway.
    let mut syms = symbols.get(rank[0] as usize..).unwrap_or(&[]).iter();
    for w in 1..=table_log {
        debug_assert!((w as usize) < rank.len());
        #[allow(unsafe_code)]
        let count = *unsafe { rank.get_unchecked(w as usize) } as usize;
        let length = 1usize << (w - 1);
        let nb_bits = table_log + 1 - w;
        for _ in 0..count {
            let sym = *syms.next().ok_or(Error::Corruption)?;
            if pos + length > table.len() {
                return Err(Error::Corruption);
            }
            // SAFETY: `pos + length > table.len()` was rejected immediately
            // above, and `k < length`, so `pos + k < table.len()`.
            for k in 0..length {
                debug_assert!(pos + k < table.len());
                #[allow(unsafe_code)]
                unsafe {
                    *table.get_unchecked_mut(pos + k) =
                        u16::from(sym) | (u16::from(nb_bits) << 8);
                }
            }
            pos += length;
        }
    }
    if pos != table.len() {
        return Err(Error::Corruption);
    }
    // C HUF_readDTableX2: fill at targetLog=11 so one peek can pair more symbols
    // and the fast loop's `bits >> 53` is legal. nbits in each entry stay native;
    // encode codes (`idx >> (max-nb)`) are unchanged (see ctable_from_weights).
    let (table, table_log) = upsample_dtable(table, table_log);
    let table_x2 = x2_from_x1(&table, table_log);
    Ok(HuffmanTable {
        table,
        table_x2,
        max_bits: table_log,
    })
}

fn upsample_dtable(table: Vec<u16>, table_log: u8) -> (Vec<u16>, u8) {
    if table_log >= FAST_TABLELOG {
        return (table, table_log);
    }
    let scale = FAST_TABLELOG - table_log;
    let factor = 1usize << scale;
    let mut wide = vec![0u16; 1 << FAST_TABLELOG];
    // SAFETY: `i < table.len() == 1 << table_log` and `scale ==
    // FAST_TABLELOG - table_log`, so `base = i << scale < 1 << FAST_TABLELOG`;
    // `k < factor == 1 << scale` keeps `base + k` inside the same bound, and
    // `wide` is exactly `1 << FAST_TABLELOG` long.
    for (i, &e) in table.iter().enumerate() {
        let base = i << scale;
        for k in 0..factor {
            debug_assert!(base + k < wide.len());
            #[allow(unsafe_code)]
            unsafe {
                *wide.get_unchecked_mut(base + k) = e;
            }
        }
    }
    (wide, FAST_TABLELOG)
}

struct Fast4x2 {
    op0: usize,
    op1: usize,
    op2: usize,
    op3: usize,
    ip0: usize,
    ip1: usize,
    ip2: usize,
    ip3: usize,
    c0: u32,
    c1: u32,
    c2: u32,
    c3: u32,
}

/// C `HUF_initFastDStream`: left-justify, sentinel `1` in the LSB after the shift.
fn init_fast_dstream(src: &[u8], ip: usize) -> u64 {
    debug_assert!(ip + 8 <= src.len());
    let last = src[ip + 7];
    let skip = if last == 0 {
        0
    } else {
        8 - (31 - (last as u32).leading_zeros())
    };
    (crate::simd::load_u64_le(src, ip) | 1) << skip
}

#[inline(always)]
fn x2_fast_sym(bits: &mut u64, op: &mut usize, dst: &mut [u8], dt: &[u32]) {
    // SAFETY. `bits >> 53` is an 11-bit value, 0..=2047, and the caller refuses
    // the whole fast path unless `table_x2.len() == 1 << FAST_TABLELOG` (2048) --
    // `upsample_dtable` widens any narrower table to exactly that. For the
    // output, `iters` is floored at `(dst.len() - op) / 10` and each pass emits
    // 5 symbols of at most 2 bytes per stream, so `op <= dst.len() - 2` at every
    // write.
    //
    // This is the hottest of the lot: 48 of the 63 Huffman bounds checks were in
    // this one unrolled loop.
    debug_assert!(dt.len() == 1 << FAST_TABLELOG);
    debug_assert!(*op + 1 < dst.len());
    #[allow(unsafe_code)]
    let e = *unsafe { dt.get_unchecked((*bits >> 53) as usize) };
    #[allow(unsafe_code)]
    unsafe {
        *dst.get_unchecked_mut(*op) = e as u8;
        *dst.get_unchecked_mut(*op + 1) = (e >> 8) as u8;
    }
    *bits <<= (e >> 16) & 0x3F;
    *op += (e >> 24) as usize;
}

#[inline(always)]
fn reload_fast(bits: &mut u64, ip: &mut usize, src: &[u8]) {
    let ctz = bits.trailing_zeros();
    let nb_bytes = (ctz >> 3) as usize;
    *ip -= nb_bytes;
    debug_assert!(*ip + 8 <= src.len());
    *bits = crate::simd::load_u64_le(src, *ip) | 1;
    *bits <<= ctz & 7;
}

/// C `HUF_selectDecoder` decode half. We already built X1 and X2, so tableTime
/// is sunk; using it would pick X1 too often (C pays tableTime because it builds
/// only one). X2 still pays the 1/32 cache penalty. `dst < 256` stays X1 (D256=0).
fn select_x2(dst_size: usize, src_size: usize) -> bool {
    if dst_size < 256 {
        return false;
    }
    let q = if src_size >= dst_size {
        15
    } else {
        ((src_size * 16) / dst_size).min(15)
    };
    let d256 = (dst_size >> 8) as u32;
    let (_, d0, _, d1) = ALGO_TIME[q];
    let time0 = d0.saturating_mul(d256);
    let mut time1 = d1.saturating_mul(d256);
    time1 += time1 >> 5;
    time1 < time0
}

/// C `algoTime[Q][single, double]` as `(tableTime, decode256Time)` pairs.
const ALGO_TIME: [(u32, u32, u32, u32); 16] = [
    (0, 0, 1, 1),
    (0, 0, 1, 1),
    (150, 216, 381, 119),
    (170, 205, 514, 112),
    (177, 199, 539, 110),
    (197, 194, 644, 107),
    (221, 192, 735, 107),
    (256, 189, 881, 106),
    (359, 188, 1167, 109),
    (582, 187, 1570, 114),
    (688, 187, 1712, 122),
    (825, 186, 1965, 136),
    (976, 185, 2131, 150),
    (1180, 186, 2070, 175),
    (1377, 185, 1731, 202),
    (1412, 185, 1695, 202),
];

/// C `HUF_DEltX2` composed from the X1 DTable: one peek of `table_log` bits can
/// emit two symbols when `n1 + n2 <= table_log`. Pack: `seq16 | nbits<<16 | length<<24`.
fn x2_from_x1(table: &[u16], table_log: u8) -> Vec<u32> {
    let n = table.len();
    let log = u32::from(table_log);
    let mut min_nbits = log;
    for &e in table {
        let nb = u32::from(e >> 8);
        if nb > 0 && nb < min_nbits {
            min_nbits = nb;
        }
    }
    let mask = n.saturating_sub(1);
    let mut out = vec![0u32; n];
    for (val, slot) in out.iter_mut().enumerate() {
        // SAFETY: `out` is `vec![0u32; n]` with `n == table.len()`, so the
        // enumeration index is in range for `table` by construction.
        debug_assert!(val < table.len());
        #[allow(unsafe_code)]
        let e1 = *unsafe { table.get_unchecked(val) };
        let s1 = u32::from(e1 as u8);
        let n1 = u32::from(e1 >> 8);
        let leftover = log.saturating_sub(n1);
        if n1 == 0 || leftover < min_nbits {
            *slot = s1 | (n1 << 16) | (1 << 24);
            continue;
        }
        let second_index = (val & ((1usize << leftover) - 1)) << n1;
        let e2 = table[second_index & mask];
        let s2 = u32::from(e2 as u8);
        let n2 = u32::from(e2 >> 8);
        if n2 == 0 || n2 > leftover {
            *slot = s1 | (n1 << 16) | (1 << 24);
        } else {
            *slot = s1 | (s2 << 8) | ((n1 + n2) << 16) | (2 << 24);
        }
    }
    out
}

/// Huffman encode table (codes + nbits) plus the DTable used as an oracle.
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub(crate) struct HuffCTable {
    /// Per-symbol: low 16 = code, bits 16..24 = nbits (0 = missing).
    entry: [u32; 256],
    /// Decode twin kept as the test oracle (`ct.table.decode_stream`).
    #[allow(dead_code)]
    table: HuffmanTable,
    weights_wo_last: Vec<u8>,
    /// Longest code in this table (`tableLog`). Fixed unroll width is `floor((64-7)/max)`.
    max_nbits: u8,
    /// Freq-weighted mean nbits × 10. Fill-vs-5 dispatch; 110 if empty.
    mean_nbits_x10: u8,
}

#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
#[allow(clippy::large_enum_variant)]
pub(crate) enum HuffUpdate {
    Unchanged,
    New(HuffCTable),
}

#[cfg(feature = "alloc")]
impl HuffCTable {
    fn encode_stream(&self, src: &[u8]) -> Result<Vec<u8>, Error> {
        // PHASE C re-adjudication switch. `RZSTD_HUFF_FAST=0` routes the
        // Huffman literal emit through the scalar twin, disabling bricks 16
        // (packed LUT + 4-symbol unroll), 29 (`covers` once, no per-symbol
        // `Result`) and 32 (K-from-max + fill dispatch) as ONE batch.
        //
        // codec-measurement 15: batch bricks behind one switch and let the
        // BATCH carry the timing verdict, where the effect is resolvable.
        // Each brick keeps its own byte-identity gate
        // (`encode_stream_unrolled_matches_scalar`) regardless of the switch.
        if crate::encode::huff_fast_enabled() {
            self.encode_stream_unrolled(src)
        } else {
            self.encode_stream_scalar(src)
        }
    }

    /// True iff every byte in `src` has a code. Treeless reuse of `prev` must
    /// check this before the emit loop — missing symbols are a fallback, not a bug.
    fn covers(&self, src: &[u8]) -> bool {
        for &b in src {
            if self.entry[b as usize] >> 16 == 0 {
                return false;
            }
        }
        true
    }

    /// Per-byte `add_bits` oracle. Same symbols, same bits as the unrolled path.
    ///
    /// Compiled in release as well as test: it is both the byte-identity
    /// oracle AND the `RZSTD_HUFF_FAST=0` arm used to re-adjudicate bricks
    /// 16 / 29 / 32 on the repaired instrument.
    fn encode_stream_scalar(&self, src: &[u8]) -> Result<Vec<u8>, Error> {
        if src.is_empty() {
            return Err(Error::Corruption);
        }
        let mut bits = crate::bit::BitCStream::with_capacity(src.len() + 8);
        for &b in src.iter().rev() {
            let e = self.entry[b as usize];
            let nb = e >> 16;
            if nb == 0 {
                return Err(Error::Corruption);
            }
            bits.add_bits(u64::from(e & 0xFFFF), nb);
        }
        Ok(bits.close())
    }

    /// C `HUF_compress1X` body: flush, then K symbols without a container check.
    /// `K` from `max_nbits` so `K*max + 7 leftover < 64` (16/8/6/5 analog of 4×4/8×8/16×16).
    fn encode_stream_unrolled(&self, src: &[u8]) -> Result<Vec<u8>, Error> {
        if src.is_empty() {
            return Err(Error::Corruption);
        }
        let mut bits = crate::bit::BitCStream::with_capacity(src.len() + 8);
        self.encode_rev_into(&mut bits, src);
        Ok(bits.close())
    }

    fn encode_rev_into(&self, bits: &mut crate::bit::BitCStream, src: &[u8]) {
        crate::prof::note_huff_path(if self.use_fill() {
            0
        } else {
            match self.max_nbits {
                0..=3 => 1,
                4 => 2,
                5 => 3,
                6 => 4,
                7 => 5,
                8 => 6,
                9 => 7,
                _ => 8,
            }
        });
        crate::prof::note_huff_path(9 + u8::from(self.max_nbits).min(10));
        if self.use_fill() {
            self.emit_fill(bits, src);
            return;
        }
        match self.max_nbits {
            0..=3 => self.emit_k::<16>(bits, src),
            4 => self.emit_k::<14>(bits, src),
            5 => self.emit_k::<11>(bits, src),
            6 => self.emit_k::<9>(bits, src),
            7 => self.emit_k::<8>(bits, src),
            8 => self.emit_k::<7>(bits, src),
            9 => self.emit_k::<6>(bits, src),
            _ => self.emit_k5(bits, src),
        }
    }

    /// Fill when expected symbols/word beat the max-nbits K by >2 (pays the fit check).
    /// Hard cap mean ≤ 7.0 from the Silesia census: sao is 7.5 / one table (brick 31 sign-flip).
    #[inline(always)]
    fn use_fill(&self) -> bool {
        if self.mean_nbits_x10 > 70 {
            return false;
        }
        let mean_x10 = u32::from(self.mean_nbits_x10.max(1));
        let k = k_from_max(self.max_nbits);
        600 / mean_x10 > k + 2
    }

    /// SAFETY throughout: `i` starts at `src.len()` and every access is preceded
    /// by `i -= 1` under a `while i >= K` guard, so `i < src.len()` at each read.
    /// This is brick 69's argument -- `emit_fill` next door has used it since --
    /// and `emit_k5`/`emit_k` were simply never given it. Per LITERAL.
    #[allow(unsafe_code)]
    fn emit_k5(&self, bits: &mut crate::bit::BitCStream, src: &[u8]) {
        let mut i = src.len();
        while i >= 5 {
            bits.flush();
            for _ in 0..5 {
                i -= 1;
                debug_assert!(i < src.len());
                self.huff_sym(bits, unsafe { *src.get_unchecked(i) });
            }
        }
        self.emit_tail(bits, src, i);
    }

    /// SAFETY: identical to `emit_k5` and `emit_fill` -- `i` only decreases from
    /// `src.len()` and every read follows an `i -= 1` under `while i >= K`.
    #[inline(always)]
    #[allow(unsafe_code)]
    fn emit_k<const K: usize>(&self, bits: &mut crate::bit::BitCStream, src: &[u8]) {
        let mut i = src.len();
        while i >= K {
            bits.flush();
            let mut n = 0usize;
            while n < K {
                i -= 1;
                debug_assert!(i < src.len());
                self.huff_sym(bits, unsafe { *src.get_unchecked(i) });
                n += 1;
            }
        }
        self.emit_tail(bits, src, i);
    }

    /// Pack the max-nbits K with no container check, then fill extras.
    /// After `flush`, leftover is ≤7 so `K*max + 7 < 64` is guaranteed.
    #[allow(unsafe_code)]
    fn emit_fill(&self, bits: &mut crate::bit::BitCStream, src: &[u8]) {
        let k = k_from_max(self.max_nbits) as usize;
        let mut i = src.len();
        while i >= k {
            bits.flush();
            let mut n = 0usize;
            while n < k {
                i -= 1;
                // SAFETY: `i` starts at `src.len()` and only decreases; the
                // `while i >= k` guard means at least `k` symbols remain, so
                // after `i -= 1` we have `i < src.len()`. See brick 69.
                self.huff_sym(bits, unsafe { *src.get_unchecked(i) });
                n += 1;
            }
            while i > 0 {
                // SAFETY: guarded by `i > 0`, and `i <= src.len()` always.
                let e = self.entry[unsafe { *src.get_unchecked(i - 1) } as usize];
                let nb = e >> 16;
                debug_assert!(nb != 0, "CTable missing symbol {}", src[i - 1]);
                if !bits.huff_fits(nb) {
                    break;
                }
                i -= 1;
                bits.add_bits_huff(u64::from(e & 0xFFFF), nb);
            }
        }
        self.emit_tail(bits, src, i);
    }

    #[allow(unsafe_code)]
    fn emit_tail(&self, bits: &mut crate::bit::BitCStream, src: &[u8], mut i: usize) {
        while i > 0 {
            i -= 1;
            // SAFETY: guarded by `i > 0` before the decrement, so `i` is a valid
            // index; `i` only ever decreases from an initial `<= src.len()`.
            let b = unsafe { *src.get_unchecked(i) };
            let e = self.entry[b as usize];
            let nb = e >> 16;
            debug_assert!(nb != 0, "CTable missing symbol {b}");
            bits.add_bits(u64::from(e & 0xFFFF), nb);
        }
    }

    /// Caller (`covers` on treeless, `build_ctable` on new) guarantees nbits.
    #[inline(always)]
    fn huff_sym(&self, bits: &mut crate::bit::BitCStream, b: u8) {
        let e = self.entry[b as usize];
        let nb = e >> 16;
        debug_assert!(nb != 0, "CTable missing symbol {b}");
        bits.add_bits_huff(u64::from(e & 0xFFFF), nb);
    }
}

#[cfg(feature = "alloc")]
fn huffman_nbits(freq: &[u32; 256]) -> Result<[u8; 256], Error> {
    let present: Vec<u8> = (0..256u16)
        .filter(|&s| freq[s as usize] > 0)
        .map(|s| s as u8)
        .collect();
    if present.len() < 2 {
        return Err(Error::Corruption);
    }
    let mut nbits = [0u8; 256];
    // A slice pattern states `len == 2` structurally, so neither index needs
    // re-proving; the symbols are `u8` and `nbits` is `[u8; 256]`.
    if let [a, b] = present[..] {
        nbits[a as usize] = 1;
        nbits[b as usize] = 1;
        return Ok(nbits);
    }

    struct Node {
        count: u64,
        left: usize,
        right: usize,
        sym: i16,
    }
    let mut nodes: Vec<Node> = present
        .iter()
        .map(|&s| Node {
            count: u64::from(freq[s as usize]),
            left: usize::MAX,
            right: usize::MAX,
            sym: i16::from(s),
        })
        .collect();
    let mut active: Vec<usize> = (0..nodes.len()).collect();
    // `active` starts as `0..nodes.len()` and only ever gains `parent`, which
    // is the index of a node pushed in the same step -- so every element is a
    // valid arena index. That is true but spans the loop, so the checked
    // accessors are used instead of asserting it: they cost the same compare and
    // cannot abort. This runs once per BLOCK, not per literal.
    while active.len() > 1 {
        active.sort_by_key(|&i| nodes.get(i).map_or(0, |n| n.count));
        let a = active.remove(0);
        let b = active.remove(0);
        let parent = nodes.len();
        let (ca, cb) = match (nodes.get(a), nodes.get(b)) {
            (Some(x), Some(y)) => (x.count, y.count),
            _ => return Err(Error::Corruption),
        };
        nodes.push(Node {
            count: ca + cb,
            left: a,
            right: b,
            sym: -1,
        });
        active.push(parent);
    }
    /// LEFT CHECKED, deliberately. This walks a node ARENA by indices stored in
    /// the nodes themselves (`left`/`right`), so its invariant lives in the tree
    /// construction above rather than in any local guard. Proving it means
    /// auditing every push into `nodes`, and the function runs once per BLOCK on
    /// the table-build path -- not per literal. The two checks stay until the
    /// arena invariant is written down and tested, not before.
    fn walk(nodes: &[Node], i: usize, depth: u8, nbits: &mut [u8; 256]) {
        // This walks a node ARENA by indices stored in the nodes themselves, so
        // its invariant lives in the construction above rather than in any local
        // guard. Rather than assert an arena invariant I have not proven, take
        // the checked accessors: `get`/`get_mut` cost the same compare the panic
        // path did but cannot abort, so a malformed arena degrades to a
        // truncated walk instead of a crash. Safe, and no `unsafe`.
        let Some(node) = nodes.get(i) else { return };
        if node.sym >= 0 {
            if let Some(slot) = nbits.get_mut(node.sym as usize) {
                *slot = depth.max(1);
            }
            return;
        }
        walk(nodes, node.left, depth.saturating_add(1), nbits);
        walk(nodes, node.right, depth.saturating_add(1), nbits);
    }
    // The loop above exits only at `len <= 1`, and `present.len() > 2` got us
    // here, so exactly one root remains -- taken through `first()` regardless.
    let root = *active.first().ok_or(Error::Corruption)?;
    walk(&nodes, root, 0, &mut nbits);
    limit_nbits(&mut nbits, &present, MAX_BITS);
    Ok(nbits)
}

#[cfg(feature = "alloc")]
fn limit_nbits(nbits: &mut [u8; 256], present: &[u8], max_bits: u8) {
    let max = i32::from(max_bits);
    let mut kraft = 0i32;
    for &s in present {
        if nbits[s as usize] > max_bits || nbits[s as usize] == 0 {
            nbits[s as usize] = max_bits;
        }
        kraft += 1 << (max - i32::from(nbits[s as usize]));
    }
    let target = 1 << max;
    while kraft > target {
        let mut best: Option<usize> = None;
        let mut best_nb = 0u8;
        for &s in present {
            let nb = nbits[s as usize];
            if nb < max_bits && (best.is_none() || nb < best_nb) {
                best = Some(s as usize);
                best_nb = nb;
            }
        }
        let Some(s) = best else {
            break;
        };
        kraft -= 1 << (max - i32::from(nbits[s]) - 1);
        nbits[s] += 1;
    }
    while kraft < target {
        let mut best: Option<usize> = None;
        let mut best_nb = 0u8;
        for &s in present {
            let nb = nbits[s as usize];
            if nb > 1 && (best.is_none() || nb > best_nb) {
                best = Some(s as usize);
                best_nb = nb;
            }
        }
        let Some(s) = best else {
            break;
        };
        kraft += 1 << (max - i32::from(nbits[s]));
        nbits[s] -= 1;
    }
}

#[cfg(feature = "alloc")]
fn ctable_from_nbits(nbits: &[u8; 256], freq: Option<&[u32; 256]>) -> Result<HuffCTable, Error> {
    let max_symbol = nbits
        .iter()
        .rposition(|&nb| nb > 0)
        .ok_or(Error::Corruption)?;
    let huff_log = nbits.iter().copied().max().unwrap_or(0);
    if huff_log == 0 || huff_log > MAX_BITS {
        return Err(Error::Corruption);
    }
    if max_symbol == 0 {
        return Err(Error::Corruption);
    }
    let mut weights = vec![0u8; max_symbol];
    for (s, slot) in weights.iter_mut().enumerate() {
        // SAFETY: `max_symbol` is an `rposition` INDEX into a `[u8; 256]`, so it
        // is at most 255, and `s < weights.len() == max_symbol`.
        debug_assert!(s < nbits.len());
        #[allow(unsafe_code)]
        let nb = *unsafe { nbits.get_unchecked(s) };
        *slot = if nb == 0 { 0 } else { huff_log + 1 - nb };
    }
    let table = table_from_weights(&weights)?;
    let mut out_nbits = [0u8; 256];
    let mut code = [0u16; 256];
    let max = table.max_bits;
    for (idx, &e) in table.table.iter().enumerate() {
        let sym = e as u8;
        let nb = (e >> 8) as u8;
        if nb == 0 {
            continue;
        }
        // SAFETY: `sym` is a `u8` and `out_nbits`/`code` are `[_; 256]` --
        // in range BY TYPE, for every possible value.
        #[allow(unsafe_code)]
        unsafe {
            if *out_nbits.get_unchecked(sym as usize) == 0 {
                *out_nbits.get_unchecked_mut(sym as usize) = nb;
                let shift = u32::from(max.saturating_sub(nb));
                *code.get_unchecked_mut(sym as usize) = (idx >> shift) as u16;
            }
        }
    }
    Ok(finish_ctable(
        pack_huff_entries(&out_nbits, &code),
        table,
        weights,
        &out_nbits,
        freq,
    ))
}

#[cfg(feature = "alloc")]
pub(crate) fn ctable_from_weights(weights: &[u8]) -> Result<HuffCTable, Error> {
    let table = table_from_weights(weights)?;
    let mut out_nbits = [0u8; 256];
    let mut code = [0u16; 256];
    let max = table.max_bits;
    for (idx, &e) in table.table.iter().enumerate() {
        let sym = e as u8;
        let nb = (e >> 8) as u8;
        if nb == 0 {
            continue;
        }
        // SAFETY: `sym` is a `u8` and `out_nbits`/`code` are `[_; 256]` --
        // in range BY TYPE, for every possible value.
        #[allow(unsafe_code)]
        unsafe {
            if *out_nbits.get_unchecked(sym as usize) == 0 {
                *out_nbits.get_unchecked_mut(sym as usize) = nb;
                let shift = u32::from(max.saturating_sub(nb));
                *code.get_unchecked_mut(sym as usize) = (idx >> shift) as u16;
            }
        }
    }
    Ok(finish_ctable(
        pack_huff_entries(&out_nbits, &code),
        table,
        weights.to_vec(),
        &out_nbits,
        None,
    ))
}

/// Parse a Huffman_Tree_Description into an encode table.
#[cfg(feature = "alloc")]
pub(crate) fn read_ctable(src: &[u8]) -> Result<(HuffCTable, usize), Error> {
    let (table, used) = read_table(src)?;
    let _ = table;
    let header = src[0];
    let weights = if header >= 128 {
        let nsym = header as usize - 127;
        let nbytes = nsym.div_ceil(2);
        // This site had NO bound of its own: `_nbytes` was computed and thrown
        // away, and it was safe only because `read_table(src)` above validated
        // the same bound for the same header and would have returned `Err`.
        // That is an indirect argument across a call boundary; state it here.
        if 1 + nbytes > src.len() {
            return Err(Error::Corruption);
        }
        let mut w = vec![0u8; nsym];
        for i in 0..nsym {
            debug_assert!(1 + i / 2 < src.len());
            #[allow(unsafe_code)]
            let b = *unsafe { src.get_unchecked(1 + i / 2) };
            w[i] = if i % 2 == 0 { b >> 4 } else { b & 0x0F };
        }
        w
    } else {
        let csize = header as usize;
        let (w, _) = fse::decompress_weights(&src[1..1 + csize], 255)?;
        w
    };
    ctable_from_weights(&weights).map(|ct| (ct, used))
}

#[cfg(feature = "alloc")]
fn pack_huff_entries(nbits: &[u8; 256], code: &[u16; 256]) -> [u32; 256] {
    let mut entry = [0u32; 256];
    for i in 0..256 {
        entry[i] = u32::from(code[i]) | (u32::from(nbits[i]) << 16);
    }
    entry
}

#[cfg(feature = "alloc")]
/// Whole-input convenience wrapper. The SHIPPING path no longer uses this:
/// brick 74 derives the frequencies from the per-segment histograms it
/// already builds, so calling this would walk the literals a second time.
/// Retained as the oracle the histogram tests compare against.
#[cfg(test)]
pub(crate) fn build_ctable(src: &[u8]) -> Result<HuffCTable, Error> {
    let mut freq = [0u32; 256];
    for &b in src {
        freq[b as usize] += 1;
    }
    build_ctable_from_freq(&freq)
}

#[cfg(feature = "alloc")]
pub(crate) fn build_ctable_from_freq(freq: &[u32; 256]) -> Result<HuffCTable, Error> {
    let nbits = huffman_nbits(freq)?;
    ctable_from_nbits(&nbits, Some(freq))
}

#[cfg(feature = "alloc")]
fn huff_mean_nbits_x10(nbits: &[u8; 256], freq: Option<&[u32; 256]>) -> u8 {
    let mut acc = 0u64;
    let mut n = 0u64;
    if let Some(freq) = freq {
        for i in 0..256 {
            let nb = nbits[i];
            if nb != 0 {
                let f = u64::from(freq[i]);
                acc += f * u64::from(nb);
                n += f;
            }
        }
    } else {
        for &nb in nbits {
            if nb != 0 {
                acc += u64::from(nb);
                n += 1;
            }
        }
    }
    if n == 0 {
        return 110;
    }
    ((acc * 10 + n / 2) / n) as u8
}

/// Largest K with `K * max_nbits + 7 leftover < 64`. 16/8/5 are the 4×4/8×8/16×16 rungs.
#[cfg(feature = "alloc")]
#[inline(always)]
fn k_from_max(max_nbits: u8) -> u32 {
    match max_nbits {
        0..=3 => 16,
        4 => 14,
        5 => 11,
        6 => 9,
        7 => 8,
        8 => 7,
        9 => 6,
        _ => 5,
    }
}

#[cfg(all(feature = "alloc", test))]
mod nbits_census {
    use std::cell::{Cell, RefCell};

    thread_local! {
        static ON: Cell<bool> = const { Cell::new(false) };
        static ROWS: RefCell<Vec<(u8, u8)>> = const { RefCell::new(Vec::new()) };
    }

    pub(super) fn note(max_nbits: u8, mean_nbits_x10: u8) {
        if ON.with(Cell::get) {
            ROWS.with(|r| r.borrow_mut().push((max_nbits, mean_nbits_x10)));
        }
    }

    pub(super) fn start() {
        ON.with(|c| c.set(true));
        ROWS.with(|r| r.borrow_mut().clear());
    }

    pub(super) fn take() -> Vec<(u8, u8)> {
        ON.with(|c| c.set(false));
        ROWS.with(|r| std::mem::take(&mut *r.borrow_mut()))
    }
}

#[cfg(feature = "alloc")]
fn finish_ctable(
    entry: [u32; 256],
    table: HuffmanTable,
    weights_wo_last: Vec<u8>,
    nbits: &[u8; 256],
    freq: Option<&[u32; 256]>,
) -> HuffCTable {
    let max_nbits = nbits.iter().copied().max().unwrap_or(0);
    let mean_nbits_x10 = huff_mean_nbits_x10(nbits, freq);
    #[cfg(test)]
    nbits_census::note(max_nbits, mean_nbits_x10);
    HuffCTable {
        entry,
        table,
        weights_wo_last,
        max_nbits,
        mean_nbits_x10,
    }
}

#[cfg(feature = "alloc")]
fn write_tree_raw(weights: &[u8]) -> Result<Vec<u8>, Error> {
    if weights.is_empty() || weights.len() > 128 {
        return Err(Error::Corruption);
    }
    let nsym = weights.len();
    let mut out = Vec::with_capacity(1 + nsym.div_ceil(2));
    out.push(128 + (nsym as u8 - 1));
    let mut i = 0usize;
    while i < nsym {
        let hi = weights[i];
        let lo = if i + 1 < nsym { weights[i + 1] } else { 0 };
        if hi > 15 || lo > 15 {
            return Err(Error::Corruption);
        }
        out.push((hi << 4) | (lo & 0x0F));
        i += 2;
    }
    Ok(out)
}

#[cfg(feature = "alloc")]
fn write_tree_fse(weights: &[u8]) -> Result<Vec<u8>, Error> {
    if weights.len() <= 2 {
        return Err(Error::Corruption);
    }
    let mut count = [0u32; 13];
    for &w in weights {
        if w as usize >= count.len() {
            return Err(Error::Corruption);
        }
        count[w as usize] += 1;
    }
    let total = weights.len() as u32;
    if count.contains(&total) {
        return Err(Error::Corruption);
    }
    let max_sv = count
        .iter()
        .rposition(|&c| c > 0)
        .ok_or(Error::Corruption)?;
    let table_log = fse::optimal_table_log(6, weights.len(), max_sv).min(6);
    let norm = fse::normalize_count(&count[..=max_sv], table_log, total, false)?;
    let ncount = fse::write_ncount(&norm, table_log)?;
    let ct = fse::FseCTable::from_norm(&norm, table_log)?;
    let payload = fse::compress_using_ctable(weights, &ct)?;
    let csize = ncount.len() + payload.len();
    if csize == 0 || csize >= 128 {
        return Err(Error::Corruption);
    }
    let mut out = Vec::with_capacity(1 + csize);
    out.push(csize as u8);
    out.extend_from_slice(&ncount);
    out.extend_from_slice(&payload);
    Ok(out)
}

/// Pick the shorter of direct 4-bit weights (header >= 128) and FSE-compressed
/// weights (header < 128), matching libzstd `HUF_writeCTable`.
#[cfg(feature = "alloc")]
pub(crate) fn write_tree(ct: &HuffCTable) -> Result<Vec<u8>, Error> {
    let weights = &ct.weights_wo_last;
    let raw = write_tree_raw(weights).ok();
    let fse = match write_tree_fse(weights) {
        Ok(fse) if fse.len() > 2 && fse[0] < 128 && fse.len() == 1 + usize::from(fse[0]) => {
            match fse::decompress_weights(&fse[1..], 255) {
                Ok((got, _)) if got == *weights => Some(fse),
                _ => None,
            }
        }
        _ => None,
    };
    match (raw, fse) {
        (Some(r), Some(f)) if f.len() < r.len() => Ok(f),
        (Some(r), _) => Ok(r),
        (None, Some(f)) => Ok(f),
        _ => Err(Error::Corruption),
    }
}

#[cfg(feature = "alloc")]
fn write_lit_huff_header(
    lit_type: u8,
    n_streams: u32,
    regen: u32,
    csize: u32,
) -> Result<Vec<u8>, Error> {
    let mut h = Vec::new();
    if n_streams == 1 {
        if regen > 0x3FF || csize > 0x3FF {
            return Err(Error::Corruption);
        }
        h.push(lit_type | ((regen & 0xF) << 4) as u8);
        h.push((((regen >> 4) & 0x3F) as u8) | (((csize & 3) as u8) << 6));
        h.push((csize >> 2) as u8);
        return Ok(h);
    }
    if regen <= 0x3FF && csize <= 0x3FF {
        h.push(lit_type | (1 << 2) | ((regen & 0xF) << 4) as u8);
        h.push((((regen >> 4) & 0x3F) as u8) | (((csize & 3) as u8) << 6));
        h.push((csize >> 2) as u8);
    } else if regen <= 0x3FFF && csize <= 0x3FFF {
        h.push(lit_type | (2 << 2) | ((regen & 0xF) << 4) as u8);
        h.push((regen >> 4) as u8);
        h.push((((regen >> 12) & 3) as u8) | (((csize & 0x3F) as u8) << 2));
        h.push((csize >> 6) as u8);
    } else if regen <= 0x3FFFF && csize <= 0x3FFFF {
        // libzstd 5-byte header: 2+2+18+18, `cLitSize<<22` then `cLitSize>>10`.
        let lhc = u32::from(lit_type) | (3 << 2) | (regen << 4) | (csize << 22);
        h.extend_from_slice(&lhc.to_le_bytes());
        h.push((csize >> 10) as u8);
    } else {
        return Err(Error::Corruption);
    }
    Ok(h)
}

#[cfg(feature = "alloc")]
/// BRICK 61: exact encoded BODY size without encoding.
///
/// `close()` appends a 1-bit end sentinel, so a stream is
/// `ceil((sum nbits + 1) / 8)` bytes; a 4-stream body is `6 + sum_i` over
/// segments of `ceil(n/4)` (mirroring `encode_4_streams` exactly, including its
/// `> 65535` per-stream failure). `None` = this table cannot encode this data,
/// or the encode would fail -- the caller must then fall back to trying it.
///
/// Segment histograms make this EXACT rather than approximate:
/// `sum_i ceil(bits_i/8) != ceil(sum bits/8)` (up to 3 bytes apart), and 3 bytes
/// is enough to flip the winner and move the bitstream.
#[cfg(feature = "alloc")]
fn body_bytes_exact(ct: &HuffCTable, seg: &[[u32; 256]], n_streams: u32) -> Option<usize> {
    let mut total = if n_streams == 4 { 6 } else { 0 };
    for h in seg.iter() {
        let mut bits: u64 = 0;
        let mut any = false;
        for (sym, &f) in h.iter().enumerate() {
            if f == 0 {
                continue;
            }
            any = true;
            // SAFETY: `h` is a `[u32; 256]` (from `seg: &[[u32; 256]]`) and
            // `ct.entry` is `[u32; 256]`, so the enumeration index is in range
            // for both by type.
            debug_assert!(sym < ct.entry.len());
            #[allow(unsafe_code)]
            let nb = *unsafe { ct.entry.get_unchecked(sym) } >> 16;
            if nb == 0 {
                return None;
            }
            bits += u64::from(f) * u64::from(nb);
        }
        if !any {
            // `encode_4_streams` rejects an empty piece.
            return None;
        }
        let bytes = ((bits + 1 + 7) / 8) as usize;
        if n_streams == 4 && bytes > 65535 {
            return None;
        }
        total += bytes;
    }
    Some(total)
}

/// Per-segment symbol histograms matching `encode_4_streams`' split, in ONE
/// pass. For `n_streams == 1` this is a single whole-input histogram.
#[cfg(feature = "alloc")]
fn segment_histograms(lits: &[u8], n_streams: u32) -> Vec<[u32; 256]> {
    if n_streams != 4 {
        let mut h = [0u32; 256];
        hist_count(lits, &mut h);
        return alloc::vec![h];
    }
    let chunk = lits.len().div_ceil(4);
    let mut segs = alloc::vec![[0u32; 256]; 4];
    let mut off = 0usize;
    for (i, h) in segs.iter_mut().enumerate() {
        let end = if i == 3 {
            lits.len()
        } else {
            (off + chunk).min(lits.len())
        };
        hist_count(&lits[off..end], h);
        off = end;
    }
    segs
}

/// C's `HIST_count_parallel` shape: a single count table serializes on the
/// store-to-load forward of the SAME slot whenever bytes repeat -- on runs,
/// every increment waits ~5 cycles for the previous one. Four independent
/// sub-tables round-robin the increments so consecutive equal bytes hit
/// different slots; the final fold is 256 adds x 3. Counts are IDENTICAL by
/// commutativity, so this is byte-exact by construction.
#[cfg(feature = "alloc")]
fn hist_count(bytes: &[u8], h: &mut [u32; 256]) {
    let mut h1 = [0u32; 256];
    let mut h2 = [0u32; 256];
    let mut h3 = [0u32; 256];
    let mut it = bytes.chunks_exact(4);
    for c in &mut it {
        h[c[0] as usize] += 1;
        h1[c[1] as usize] += 1;
        h2[c[2] as usize] += 1;
        h3[c[3] as usize] += 1;
    }
    for &b in it.remainder() {
        h[b as usize] += 1;
    }
    for i in 0..256 {
        h[i] += h1[i] + h2[i] + h3[i];
    }
}

fn encode_4_streams(ct: &HuffCTable, src: &[u8]) -> Result<Vec<u8>, Error> {
    let chunk = src.len().div_ceil(4);
    let mut streams = Vec::with_capacity(4);
    let mut off = 0usize;
    for i in 0..4 {
        let end = if i == 3 {
            src.len()
        } else {
            (off + chunk).min(src.len())
        };
        let piece = &src[off..end];
        if piece.is_empty() {
            return Err(Error::Corruption);
        }
        let s = ct.encode_stream(piece)?;
        if s.len() > 65535 {
            return Err(Error::Corruption);
        }
        streams.push(s);
        off = end;
    }
    let body: usize = streams.iter().map(|s| s.len()).sum();
    let mut out = Vec::with_capacity(6 + body);
    // The loop above pushes exactly four streams or returns `Err`, so a slice
    // pattern states the length structurally instead of re-proving it three
    // times. These were the last three panic sites in the file.
    let [s0, s1, s2, _s3] = &streams[..] else {
        return Err(Error::Corruption);
    };
    out.extend_from_slice(&(s0.len() as u16).to_le_bytes());
    out.extend_from_slice(&(s1.len() as u16).to_le_bytes());
    out.extend_from_slice(&(s2.len() as u16).to_le_bytes());
    for s in &streams {
        out.extend_from_slice(s);
    }
    Ok(out)
}

#[cfg(feature = "alloc")]
fn pack_huff_section(
    lit_type: u8,
    n_streams: u32,
    regen: u32,
    tree: &[u8],
    body: &[u8],
) -> Result<Vec<u8>, Error> {
    let csize = (tree.len() + body.len()) as u32;
    let mut out = write_lit_huff_header(lit_type, n_streams, regen, csize)?;
    out.extend_from_slice(tree);
    out.extend_from_slice(body);
    Ok(out)
}

/// Cheap sample: skip Huffman when the alphabet looks uniform (incompressible).
/// C `ZSTD_compressLiterals` bails out the same way instead of encoding then discarding.
#[cfg(feature = "alloc")]
pub(crate) fn literals_worth_huffman(lits: &[u8]) -> bool {
    const SAMPLE: usize = 1024;
    if lits.len() < 64 {
        return true;
    }
    let mut freq = [0u32; 256];
    // ODD stride. `len/SAMPLE` is a power of two on 128 KiB blocks, and a
    // power-of-two stride ALIASES with the period of fixed-width binary
    // content: x-ray is 16-bit samples, so an even step only ever lands on
    // one byte-phase and histograms half the data. Measured: x-ray size
    // ratio 1.030 vs 1.159 purely from the stride landing differently.
    // `| 1` makes the walk cycle through every phase.
    let step = ((lits.len() / SAMPLE).max(1)) | 1;
    let mut n = 0u32;
    let mut i = 0usize;
    while i < lits.len() && n < SAMPLE as u32 {
        freq[lits[i] as usize] += 1;
        i += step;
        n += 1;
    }
    if n == 0 {
        return true;
    }
    // BRICK 86: this used to be `max * 8 >= n` -- "some byte is >= 12.5% of the
    // sample". That measures PEAK FREQUENCY, but what decides whether Huffman
    // pays is ENTROPY. Text over a moderate alphabet with no dominant symbol
    // fails the peak test and is emitted RAW even though Huffman would win
    // ~30% on it. Measured on jsonlog-16m: 4,069,169 literal bytes went out as
    // a 3,797,461-byte section -- a 0.93 ratio, i.e. essentially uncompressed --
    // while C's literals section was 2,052,405. That single gate was **87% of
    // our whole size gap** on that corpus.
    //
    // Use the collision entropy of the sample instead:
    //   H2 = -log2( sum p^2 ),  worth trying when H2 <= 7 bits/symbol
    //   => sum(f^2)/n^2 >= 2^-7  =>  sum(f^2) * 128 >= n^2
    // SAMPLE is 1024, not 256, for ESTIMATOR MARGIN. With 256 samples over a
    // 256-symbol alphabet, uniform random data gives sum(f^2) ~ 511 against
    // n^2 = 65536, and 511*128 = 65408 -- within 0.2% of the threshold, so
    // noise flips it ~half the time and incompressible blocks pay for a full
    // Huffman attempt that is always discarded (measured: incomp-32m compress
    // 6640 -> 2020 MB/s at SAMPLE=256, with byte-identical output). At 1024
    // the expected random sum(f^2) is ~5116 against n^2 = 1048576, a 1.6x
    // margin on the reject side.
    //
    // Integer-only, so this stays no_std-clean, and it is strictly MORE
    // permissive than the peak test (a dominant symbol makes sum(f^2) large
    // too). Uniform random bytes still fail: 256 samples over 256 values give
    // sum(f^2) ~ 256 against n^2 = 65536, and 256*128 < 65536.
    //
    // Being too permissive is the SAFE direction: the caller keeps `raw_len` as
    // the baseline and only emits Huffman if it actually comes out smaller, so
    // a false positive costs encode time, never bytes.
    // BRICK 88: TREE AMORTIZATION. Entropy alone decides whether Huffman codes
    // the BODY smaller; it says nothing about whether the section can pay for
    // the WEIGHT TABLE it must carry (~`distinct/2` bytes, 4 bits per symbol).
    // When the alphabet is large relative to the SECTION, no distribution can
    // pay that back.
    //
    // `versions-16m` L1 is the case that exposed it: 31,047 literal bytes over
    // 128 blocks -- ~304 bytes per block across ~200 distinct symbols, so a
    // ~100-byte tree sits against a 304-byte section. The H2 test accepted 102
    // of 128 blocks, every one of which then lost to raw (`raw_won=102`), and
    // the ctable build plus `write_tree` cost 2.24 ms to process 31 KB of
    // literals -- 13.8 MB/s, which is what halved L1 compress on that corpus.
    //
    // `distinct` comes from the SAMPLE, so for a section longer than the sample
    // it UNDERCOUNTS, and the test fires less often than the true alphabet
    // warrants -- the safe direction, and it is why this cannot regress the
    // large-literal corpora the entropy fix was built for.
    let distinct = freq.iter().filter(|&&f| f != 0).count() as u64;
    if distinct.saturating_mul(2) >= lits.len() as u64 {
        return false;
    }
    let sum_sq: u64 = freq.iter().map(|&f| u64::from(f) * u64::from(f)).sum();
    sum_sq.saturating_mul(128) >= u64::from(n) * u64::from(n)
}

/// Sample peak in 0..=1000 (`max_freq * 1000 / n_sampled`). 0 if too small to sample.
#[cfg(feature = "alloc")]
pub(crate) fn lit_sample_peak(lits: &[u8]) -> u32 {
    const SAMPLE: usize = 256;
    if lits.len() < 64 {
        return 0;
    }
    let mut freq = [0u32; 256];
    let step = (lits.len() / SAMPLE).max(1);
    let mut n = 0u32;
    let mut i = 0usize;
    while i < lits.len() && n < SAMPLE as u32 {
        freq[lits[i] as usize] += 1;
        i += step;
        n += 1;
    }
    if n == 0 {
        return 0;
    }
    let mut max = 0u32;
    for f in freq {
        if f > max {
            max = f;
        }
    }
    max.saturating_mul(1000) / n
}

/// Encode a literals section: raw, RLE, Huffman (1/4-stream), or treeless.
///
/// Returns the RFC 8878 literals header+payload and whether a new Huffman table
/// should be remembered for later treeless blocks.
#[cfg(feature = "alloc")]
pub(crate) fn encode_literals_section(
    lits: &[u8],
    prev: Option<&HuffCTable>,
) -> Result<(Vec<u8>, HuffUpdate), Error> {
    let n = lits.len() as u32;
    if n == 0 {
        return Ok((vec![0], HuffUpdate::Unchanged));
    }
    let all_same = n >= 2 && lits.iter().all(|&b| b == lits[0]);
    if all_same {
        return Ok((write_raw_or_rle(lits, true), HuffUpdate::Unchanged));
    }
    // BRICK 60: do NOT materialize the raw section just to hold a baseline
    // LENGTH. It is a full copy of every literal byte, and on Huffman-friendly
    // content (mr: 6.9 MB of literals, Huffman 61.3% of encode) it is thrown
    // away every time. Its size is exact arithmetic -- `hdr + n` -- so carry the
    // NUMBER and build the bytes only if raw actually wins.
    if n < 8 {
        return Ok((write_raw_or_rle(lits, false), HuffUpdate::Unchanged));
    }
    if n >= 64 && !literals_worth_huffman(lits) {
        return Ok((write_raw_or_rle(lits, false), HuffUpdate::Unchanged));
    }

    crate::prof::note_lit_try(0);
    let raw_len = raw_section_len(n);
    // `None` = raw is still the best candidate.
    let mut best: Option<Vec<u8>> = None;
    let mut best_len = raw_len;
    let mut update = HuffUpdate::Unchanged;
    // libzstd `ZSTD_compressLiterals`: 1-stream iff regen < 256, else 4-stream.
    let preferred: u32 = if n >= 256 { 4 } else { 1 };

    // BRICK 61: build the new table + tree FIRST. This is PURE COMPUTATION --
    // it emits nothing and mutates nothing -- so hoisting it above the previous
    // -table attempt cannot change which section wins. It buys the size we need
    // to prove the speculative encode futile.
    // BRICK 74: ONE pass over the literals, not two.
    //
    // Brick 61 added `segment_histograms` (a full O(n) walk) while
    // `build_ctable(lits)` was already doing its own full O(n) histogram --
    // so a block with a usable previous table walked 24.4 MB of mozilla's
    // literals TWICE. A brick that removes expensive work can still add
    // cheaper work nobody counted.
    //
    // The per-segment histograms SUM to the whole-input histogram, so build
    // them once and derive the overall frequencies from them. Byte-identical:
    // `build_ctable_from_freq` receives exactly the counts `build_ctable`
    // would have computed. Costs no extra work when there is no previous
    // table either -- a segment histogram is the same increments as a whole
    // one, just indexed by segment.
    let segs = segment_histograms(lits, preferred);
    let mut freq = [0u32; 256];
    for h in segs.iter() {
        for (s, &c) in h.iter().enumerate() {
            freq[s] += c;
        }
    }
    let new_tbl = build_ctable_from_freq(&freq)
        .ok()
        .and_then(|ct| write_tree(&ct).ok().map(|t| (ct, t)));

    if let Some(prev_ct) = prev {
        if prev_ct.covers(lits) {
            // Can the previous table possibly win? The new table is Huffman-
            // OPTIMAL for these frequencies, so `body_new <= body_prev` always;
            // prev can only win by saving the tree. If it loses by more than the
            // tree plus a header-slack margin, encoding it is provably wasted.
            //
            // `body_bytes_exact` returning `Some` also means the 4-stream encode
            // would SUCCEED (same coverage and 65535 checks), so the 1-stream
            // retry below would not have run either.
            let futile = match &new_tbl {
                Some((ct, tree)) => {
                    match (
                        body_bytes_exact(prev_ct, &segs, preferred),
                        body_bytes_exact(ct, &segs, preferred),
                    ) {
                        (Some(bp), Some(bn)) => bp >= bn + tree.len() + 8,
                        _ => false,
                    }
                }
                None => false,
            };
            if futile {
                crate::prof::note_lit_try(6);
            }
            if !futile {
                crate::prof::note_lit_try(1);
                if let Some(sec) = try_huff_section(3, preferred, n, &[], prev_ct, lits) {
                    if sec.len() < best_len {
                        crate::prof::note_lit_try(2);
                        best_len = sec.len();
                        best = Some(sec);
                        update = HuffUpdate::Unchanged;
                    }
                } else if preferred == 4 {
                    if let Some(sec) = try_huff_section(3, 1, n, &[], prev_ct, lits) {
                        if sec.len() < best_len {
                            best_len = sec.len();
                            best = Some(sec);
                            update = HuffUpdate::Unchanged;
                        }
                    }
                }
            }
        }
    }

    if let Some((ct, tree)) = new_tbl {
        {
            crate::prof::note_lit_try(3);
            if let Some(sec) = try_huff_section(2, preferred, n, &tree, &ct, lits) {
                if sec.len() < best_len {
                    crate::prof::note_lit_try(4);
                    best = Some(sec);
                    update = HuffUpdate::New(ct.clone());
                }
            } else if preferred == 4 {
                if let Some(sec) = try_huff_section(2, 1, n, &tree, &ct, lits) {
                    if sec.len() < best_len {
                        best = Some(sec);
                        update = HuffUpdate::New(ct);
                    }
                }
            }
        }
    }

    // Raw only gets built if nothing beat it.
    let best = match best {
        Some(sec) => sec,
        None => {
            crate::prof::note_lit_try(5);
            write_raw_or_rle(lits, false)
        }
    };
    Ok((best, update))
}

#[cfg(feature = "alloc")]
fn try_huff_section(
    lit_type: u8,
    n_streams: u32,
    regen: u32,
    tree: &[u8],
    ct: &HuffCTable,
    lits: &[u8],
) -> Option<Vec<u8>> {
    let body = if n_streams == 1 {
        ct.encode_stream(lits).ok()?
    } else {
        encode_4_streams(ct, lits).ok()?
    };
    pack_huff_section(lit_type, n_streams, regen, tree, &body).ok()
}

#[cfg(feature = "alloc")]
#[cfg(test)]
fn huff_section_roundtrips(sec: &[u8], lits: &[u8]) -> bool {
    if sec.is_empty() {
        return false;
    }
    let lit_type = sec[0] & 3;
    let size_fmt = (sec[0] >> 2) & 3;
    let n_streams = match (lit_type, size_fmt) {
        (2, 0) => 1,
        (2, 1..=3) => 4,
        _ => return false,
    };
    let hlen = match size_fmt {
        0 | 1 => 3,
        2 => 4,
        3 => 5,
        _ => return false,
    };
    if sec.len() <= hlen {
        return false;
    }
    let payload = &sec[hlen..];
    let hdr_csize = match size_fmt {
        0 | 1 => (((u32::from(sec[1]) >> 6) + (u32::from(sec[2]) << 2)) & 0x3FF) as usize,
        2 => ((u32::from(sec[2]) >> 2) + (u32::from(sec[3]) << 6)) as usize & 0x3FFF,
        3 => {
            ((u32::from(sec[2]) >> 6) + (u32::from(sec[3]) << 2) + (u32::from(sec[4]) << 10))
                as usize
                & 0x3FFFF
        }
        _ => return false,
    };
    if hdr_csize != payload.len() {
        return false;
    }
    let Ok((table, tree)) = read_table(payload) else {
        return false;
    };
    if tree > payload.len() {
        return false;
    }
    huff_body_roundtrips(&table, &payload[tree..], lits, n_streams)
}

#[cfg(test)]
fn huff_body_roundtrips(table: &HuffmanTable, body: &[u8], lits: &[u8], n_streams: u32) -> bool {
    let mut out = vec![0u8; lits.len()];
    if n_streams == 1 {
        if table.decode_stream(body, &mut out).is_err() {
            return false;
        }
        return out == lits;
    }
    if body.len() < 6 {
        return false;
    }
    let s1 = u16::from_le_bytes([body[0], body[1]]) as usize;
    let s2 = u16::from_le_bytes([body[2], body[3]]) as usize;
    let s3 = u16::from_le_bytes([body[4], body[5]]) as usize;
    let total = body.len() - 6;
    if s1 + s2 + s3 > total {
        return false;
    }
    let s4 = total - s1 - s2 - s3;
    let rest = &body[6..];
    let chunk = lits.len().div_ceil(4);
    let mut off = 0usize;
    let mut dst = 0usize;
    let sizes = [s1, s2, s3, s4];
    for (i, &sz) in sizes.iter().enumerate() {
        let end = if i == 3 {
            lits.len()
        } else {
            (dst + chunk).min(lits.len())
        };
        if off + sz > rest.len() || dst > end {
            return false;
        }
        if table
            .decode_stream(&rest[off..off + sz], &mut out[dst..end])
            .is_err()
        {
            return false;
        }
        off += sz;
        dst = end;
    }
    out == lits
}

#[cfg(feature = "alloc")]
/// Byte length `write_raw_or_rle(lits, false)` would produce, without building
/// it. Mirrors that function's header sizing exactly (brick 60).
#[cfg(feature = "alloc")]
fn raw_section_len(n: u32) -> usize {
    let hdr = if n < 32 {
        1
    } else if n < 4096 {
        2
    } else {
        3
    };
    hdr + n as usize
}

fn write_raw_or_rle(lits: &[u8], rle: bool) -> Vec<u8> {
    let n = lits.len() as u32;
    let ty: u8 = if rle { 1 } else { 0 };
    let mut dst = Vec::new();
    if n < 32 {
        dst.push((n << 3) as u8 | ty);
    } else if n < 4096 {
        dst.push((1 << 2) | ty | ((n & 0xF) << 4) as u8);
        dst.push((n >> 4) as u8);
    } else {
        dst.push((3 << 2) | ty | ((n & 0xF) << 4) as u8);
        dst.push((n >> 4) as u8);
        dst.push((n >> 12) as u8);
    }
    if rle {
        // `rle` promising a non-empty `lits` is a CALLER contract with no local
        // witness, so this must not become `unsafe`. `first()` keeps it safe and
        // still drops the panic path.
        debug_assert!(!lits.is_empty());
        if let Some(&b) = lits.first() {
            dst.push(b);
        }
    } else {
        dst.extend_from_slice(lits);
    }
    dst
}

#[cfg(all(test, feature = "alloc"))]
mod tests {

    /// RLE literals (section type 1) emitted directly. The mode-coverage test
    /// in `encode.rs` used to reach this through the match finder's residue,
    /// but repcode-1 search (brick 40) consumes those runs -- so the mode is
    /// gated HERE, on the emit path itself, which cannot be invalidated by a
    /// change in matcher quality.
    #[test]
    fn rle_literals_section_emits_type_1_and_round_trips() {
        for n in [2usize, 7, 63, 64, 300, 5000] {
            let lits = vec![b'q'; n];
            let (sec, upd) = encode_literals_section(&lits, None).expect("rle lits");
            assert!(matches!(upd, HuffUpdate::Unchanged), "n={n}");
            assert_eq!(sec[0] & 3, 1, "n={n}: literals section type must be RLE");
            let mut r = crate::reader::Reader::new(&sec);
            let mut st = crate::compressed::BlockState::new();
            let got = crate::compressed::decode_literals(&mut r, &mut st).expect("decode");
            assert_eq!(got, lits, "n={n}");
        }
        let mut mixed = vec![b'q'; 64];
        mixed[10] = b'r';
        let (sec, _) = encode_literals_section(&mixed, None).expect("mixed");
        assert_ne!(sec[0] & 3, 1, "mixed literals must not be RLE");
    }
    use super::*;

    #[test]
    fn huffman_length_sweep() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        for n in [
            8, 9, 16, 31, 32, 63, 64, 127, 128, 224, 255, 256, 267, 400, 512,
        ] {
            let mut src = Vec::new();
            while src.len() < n {
                src.extend_from_slice(fox);
            }
            src.truncate(n);
            let ct = build_ctable(&src).expect("build");
            let stream = ct.encode_stream(&src).expect("encode");
            let mut out = vec![0u8; src.len()];
            ct.table
                .decode_stream(&stream, &mut out)
                .unwrap_or_else(|e| panic!("orig n={n}: {e:?}"));
            assert_eq!(out, src, "orig-table n={n}");
            let mut scalar_d = vec![0u8; src.len()];
            ct.table
                .decode_stream_scalar(&stream, &mut scalar_d)
                .expect("decode scalar");
            assert_eq!(scalar_d, src, "decode unroll vs scalar n={n}");
            let scalar = ct.encode_stream_scalar(&src).expect("scalar");
            assert_eq!(stream, scalar, "unrolled vs per-byte add_bits n={n}");
            let (sec, upd) = encode_literals_section(&src, None).expect("section");
            if n >= 224 {
                assert_eq!(sec[0] & 3, 2, "Huffman Compressed literals n={n}");
                match upd {
                    HuffUpdate::New(_) => {}
                    HuffUpdate::Unchanged => panic!("expected a new Huffman table n={n}"),
                }
                assert!(
                    huff_section_roundtrips(&sec, &src),
                    "read_table section n={n}"
                );
            }
        }
    }

    #[test]
    fn incompressible_literals_stay_raw() {
        let mut src = vec![0u8; 4096];
        let mut x = 0xA5A5_5A5A_u64;
        for b in &mut src {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        assert!(!literals_worth_huffman(&src));
        let (sec, _) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 0, "incomp literals should be raw");
    }

    #[test]
    fn fox_literals_still_huffman() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 512 {
            src.extend_from_slice(fox);
        }
        let ct = build_ctable(&src).expect("build");
        let stream = ct.encode_stream(&src).expect("encode");
        let mut out = vec![0u8; src.len()];
        ct.table.decode_stream(&stream, &mut out).expect("decode");
        assert_eq!(out, src);
        let (sec, _) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 2, "fox text should still Huffman");
    }

    #[test]
    fn huffman_section_roundtrip_via_read_table() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 224 {
            src.extend_from_slice(fox);
        }
        src.truncate(224);
        let (sec, upd) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 2, "expected Compressed Huffman literals");
        match upd {
            HuffUpdate::New(_) => {}
            HuffUpdate::Unchanged => panic!("expected a new Huffman table"),
        }
        // Skip the 3-5 byte literals header and decode the Huffman payload.
        let lit_type = sec[0] & 3;
        let size_fmt = (sec[0] >> 2) & 3;
        let header_len = match (lit_type, size_fmt) {
            (2 | 3, 0 | 1) => 3,
            (2 | 3, 2) => 4,
            (2 | 3, 3) => 5,
            _ => panic!("unexpected header"),
        };
        let payload = &sec[header_len..];
        let (table, tree) = read_table(payload).expect("read_table");
        let mut out = vec![0u8; src.len()];
        table
            .decode_stream(&payload[tree..], &mut out)
            .expect("decode_stream");
        assert_eq!(out, src);

        // Same section inside a real compressed block (nseq=0) through the public decoder.
        let mut frame = Vec::new();
        crate::encode::write_frame_header(
            &mut frame,
            src.len() as u64,
            10,
            true,
            Some(src.len() as u64),
            None,
            false,
        );
        let mut block = sec.clone();
        block.push(0);
        let n = block.len() as u32;
        let hdr = 1u32 | (2 << 1) | (n << 3);
        frame.push(hdr as u8);
        frame.push((hdr >> 8) as u8);
        frame.push((hdr >> 16) as u8);
        frame.extend_from_slice(&block);
        frame.extend_from_slice(&crate::xxh64::content_checksum(&src).to_le_bytes());
        let got = crate::decompress(&frame).expect("frame decode");
        assert_eq!(got, src);
    }

    #[test]
    fn covers_rejects_unseen_symbol() {
        let src = b"aaaaabbbbbccccc";
        let ct = build_ctable(src).expect("build");
        assert!(ct.covers(src));
        assert!(!ct.covers(b"aaaaabbbbbcccccZ"));
    }

    #[test]
    fn encode_stream_unrolled_matches_scalar() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 4096 {
            src.extend_from_slice(fox);
        }
        let ct = build_ctable(&src).expect("build");
        for n in 1..=64 {
            let s = &src[..n];
            let a = ct.encode_stream(s).expect("fast");
            let b = ct.encode_stream_scalar(s).expect("scalar");
            assert_eq!(a, b, "n={n}");
        }
        for &n in &[65usize, 127, 128, 255, 256, 257, 511, 512, 1024, 4096] {
            let s = &src[..n.min(src.len())];
            let a = ct.encode_stream(s).expect("fast");
            let b = ct.encode_stream_scalar(s).expect("scalar");
            assert_eq!(a, b, "n={}", s.len());
        }
        let mut noise = vec![0u8; 1024];
        let mut x = 0xC0FF_EE00_u64;
        for b in &mut noise {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let ct2 = build_ctable(&noise).expect("noise table");
        let a = ct2.encode_stream(&noise).expect("fast");
        let b = ct2.encode_stream_scalar(&noise).expect("scalar");
        assert_eq!(a, b, "noise");

        // Peaked alphabet → short max_nbits (K16) or fill. Must stay byte-identical.
        let mut peaked = vec![b'a'; 4096];
        peaked.extend_from_slice(b"bc");
        let ct3 = build_ctable(&peaked).expect("peaked table");
        let a = ct3.encode_stream(&peaked).expect("fast");
        let b = ct3.encode_stream_scalar(&peaked).expect("scalar");
        assert_eq!(a, b, "peaked");
        assert!(
            ct3.max_nbits <= 3 || ct3.use_fill(),
            "peaked max={} mean_x10={} should take K16 or fill",
            ct3.max_nbits,
            ct3.mean_nbits_x10
        );
    }

    #[test]
    fn huff_pack_dispatch_separates_peaked_from_flat() {
        let mut peaked = vec![b'a'; 8192];
        peaked.extend_from_slice(b"bcdefgh");
        let ct = build_ctable(&peaked).expect("peaked");
        assert!(
            ct.use_fill() || ct.max_nbits <= 7,
            "peaked should fill or take a wide K max={} mean_x10={}",
            ct.max_nbits,
            ct.mean_nbits_x10
        );

        let mut flat = vec![0u8; 8192];
        for (i, b) in flat.iter_mut().enumerate() {
            *b = (i % 251) as u8;
        }
        let ct_f = build_ctable(&flat).expect("flat");
        // Sao-like: long mean → fixed K, not fill (the brick-31 sign-flip).
        assert!(
            !ct_f.use_fill(),
            "flat/long-code must not fill max={} mean_x10={}",
            ct_f.max_nbits,
            ct_f.mean_nbits_x10
        );
        assert_eq!(k_from_max(9), 6);
        assert_eq!(k_from_max(11), 5);
        assert_eq!(k_from_max(7), 8);
        assert_eq!(k_from_max(3), 16);
    }

    #[test]
    fn decode_stream_unrolled_matches_scalar() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 4096 {
            src.extend_from_slice(fox);
        }
        let ct = build_ctable(&src).expect("build");
        for n in 1..=64 {
            let s = &src[..n];
            let stream = ct.encode_stream_scalar(s).expect("enc");
            let mut a = vec![0u8; n];
            let mut b = vec![0u8; n];
            ct.table.decode_stream(&stream, &mut a).expect("fast");
            ct.table
                .decode_stream_scalar(&stream, &mut b)
                .expect("scalar");
            assert_eq!(a, b, "n={n}");
            assert_eq!(a, s, "roundtrip n={n}");
        }
        for &n in &[65usize, 127, 128, 255, 256, 257, 511, 512, 1024, 4096] {
            let s = &src[..n.min(src.len())];
            let stream = ct.encode_stream_scalar(s).expect("enc");
            let mut a = vec![0u8; s.len()];
            let mut b = vec![0u8; s.len()];
            ct.table.decode_stream(&stream, &mut a).expect("fast");
            ct.table
                .decode_stream_scalar(&stream, &mut b)
                .expect("scalar");
            assert_eq!(a, b, "n={}", s.len());
            assert_eq!(a, s);
        }
        let mut noise = vec![0u8; 1024];
        let mut x = 0xC0FF_EE00_u64;
        for b in &mut noise {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            *b = x as u8;
        }
        let ct2 = build_ctable(&noise).expect("noise table");
        let stream = ct2.encode_stream_scalar(&noise).expect("enc");
        let mut a = vec![0u8; noise.len()];
        let mut b = vec![0u8; noise.len()];
        ct2.table.decode_stream(&stream, &mut a).expect("fast");
        ct2.table
            .decode_stream_scalar(&stream, &mut b)
            .expect("scalar");
        assert_eq!(a, b, "noise");
        assert_eq!(a, noise);
    }

    #[test]
    fn encode_4_streams_matches_sequential_1x() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 1024 {
            src.extend_from_slice(fox);
        }
        src.truncate(1024);
        let ct = build_ctable(&src).expect("build");
        let four = encode_4_streams(&ct, &src).expect("4x");
        let chunk = src.len().div_ceil(4);
        let mut off = 0usize;
        let mut body = Vec::new();
        let mut hdr = Vec::new();
        for i in 0..4 {
            let end = if i == 3 {
                src.len()
            } else {
                (off + chunk).min(src.len())
            };
            let s = ct.encode_stream(&src[off..end]).expect("1x");
            if i < 3 {
                hdr.extend_from_slice(&(s.len() as u16).to_le_bytes());
            }
            body.extend_from_slice(&s);
            off = end;
        }
        assert_eq!(&four[..6], hdr.as_slice());
        assert_eq!(&four[6..], body.as_slice());
    }

    #[test]
    fn decode_4x_matches_sequential() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 1024 {
            src.extend_from_slice(fox);
        }
        src.truncate(1024);
        let ct = build_ctable(&src).expect("build");
        let packed = encode_4_streams(&ct, &src).expect("4x enc");
        let s1 = u16::from_le_bytes([packed[0], packed[1]]) as usize;
        let s2 = u16::from_le_bytes([packed[2], packed[3]]) as usize;
        let s3 = u16::from_le_bytes([packed[4], packed[5]]) as usize;
        let rest = &packed[6..];
        let s4 = rest.len() - s1 - s2 - s3;
        let chunk = src.len().div_ceil(4);
        let mut lock = vec![0u8; src.len()];
        let (d0, r) = lock.split_at_mut(chunk);
        let (d1, r) = r.split_at_mut(chunk);
        let (d2, d3) = r.split_at_mut(chunk);
        ct.table
            .decode_4x(
                &rest[..s1],
                &rest[s1..s1 + s2],
                &rest[s1 + s2..s1 + s2 + s3],
                &rest[s1 + s2 + s3..s1 + s2 + s3 + s4],
                d0,
                d1,
                d2,
                d3,
            )
            .expect("lockstep");
        let mut seq = vec![0u8; src.len()];
        let mut off = 0usize;
        let mut dst = 0usize;
        for (i, &sz) in [s1, s2, s3, s4].iter().enumerate() {
            let end = if i == 3 { seq.len() } else { dst + chunk };
            ct.table
                .decode_stream(&rest[off..off + sz], &mut seq[dst..end])
                .expect("seq");
            off += sz;
            dst = end;
        }
        assert_eq!(lock, seq);
        assert_eq!(lock, src);
    }

    #[test]
    fn select_x2_follows_c_breakpoints() {
        assert!(!select_x2(255, 32), "dst < 256 (1-stream): X1");
        assert!(select_x2(256, 32), "256B Q=2, table already built: X2");
        assert!(select_x2(128 * 1024, 16 * 1024), "128KiB at ~12% : X2");
        assert!(
            !select_x2(128 * 1024, 128 * 1024),
            "Q=15 incompressible: X1"
        );
    }

    #[test]
    fn huffman_four_stream_and_tree_encodings() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 512 {
            src.extend_from_slice(fox);
        }
        src.truncate(512);
        let (sec, upd) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 2, "Compressed Huffman");
        let size_fmt = (sec[0] >> 2) & 3;
        assert_ne!(size_fmt, 0, "4-stream size format, got {size_fmt}");
        match upd {
            HuffUpdate::New(_) => {}
            HuffUpdate::Unchanged => panic!("expected a new Huffman table"),
        }
        assert!(huff_section_roundtrips(&sec, &src), "4-stream read_table");

        let ct = build_ctable(&src).expect("build");
        let raw = write_tree_raw(&ct.weights_wo_last).expect("raw tree");
        assert!(raw[0] >= 128, "direct 4-bit weight header");
        let (t_raw, n_raw) = read_table(&raw).expect("read raw tree");
        assert_eq!(n_raw, raw.len());
        let stream = ct.encode_stream(&src).expect("encode");
        let mut out = vec![0u8; src.len()];
        t_raw
            .decode_stream(&stream, &mut out)
            .expect("raw-tree decode");
        assert_eq!(out, src);

        let fse = write_tree_fse(&ct.weights_wo_last).expect("FSE tree");
        assert!(fse[0] < 128, "FSE-compressed weight header");
        assert_eq!(fse.len(), 1 + usize::from(fse[0]));
        let (got_w, _) = fse::decompress_weights(&fse[1..], 255).expect("weights");
        assert_eq!(
            got_w,
            ct.weights_wo_last,
            "FSE weight roundtrip len got={} want={}",
            got_w.len(),
            ct.weights_wo_last.len()
        );
        let (t_fse, n_fse) = read_table(&fse).expect("read FSE tree");
        assert_eq!(n_fse, fse.len());
        out.fill(0);
        t_fse
            .decode_stream(&stream, &mut out)
            .expect("FSE-tree decode");
        assert_eq!(out, src);

        let chosen = write_tree(&ct).expect("write_tree");
        read_table(&chosen).expect("chosen tree");
    }

    #[test]
    fn huffman_one_stream_below_256() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 224 {
            src.extend_from_slice(fox);
        }
        src.truncate(224);
        let (sec, _) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 2);
        assert_eq!(sec[0] >> 2 & 3, 0, "1-stream size format 0");
    }

    #[test]
    fn huffman_five_byte_header_csize_matches() {
        let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
        let mut src = Vec::new();
        while src.len() < 20_000 {
            src.extend_from_slice(fox);
        }
        src.truncate(20_000);
        let (sec, _) = encode_literals_section(&src, None).expect("section");
        assert_eq!(sec[0] & 3, 2);
        assert_eq!(sec[0] >> 2 & 3, 3, "18-bit 4-stream header");
        assert!(huff_section_roundtrips(&sec, &src));
        let mut frame = Vec::new();
        crate::encode::write_frame_header(
            &mut frame,
            src.len() as u64,
            15,
            true,
            Some(src.len() as u64),
            None,
            false,
        );
        let mut block = sec.clone();
        block.push(0);
        let n = block.len() as u32;
        let hdr = 1u32 | (2 << 1) | (n << 3);
        frame.push(hdr as u8);
        frame.push((hdr >> 8) as u8);
        frame.push((hdr >> 16) as u8);
        frame.extend_from_slice(&block);
        frame.extend_from_slice(&crate::xxh64::content_checksum(&src).to_le_bytes());
        let got = crate::decompress(&frame).expect("frame decode");
        assert_eq!(got, src);
    }

    /// Count, not time: per-CTable max/mean nbits on real `-1` literals (mr vs sao).
    #[ignore = "needs corpora/data/silesia; run with --ignored --nocapture"]
    #[test]
    fn silesia_huff_nbits_census() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia");
        if !root.is_dir() {
            return;
        }
        for name in ["mr", "mozilla", "sao", "nci", "xml", "x-ray"] {
            let path = root.join(name);
            let src = match std::fs::read(&path) {
                Ok(s) => s,
                Err(_) => continue,
            };
            nbits_census::start();
            crate::encode::compress(&src, 1).expect("compress");
            let rows = nbits_census::take();
            if rows.is_empty() {
                println!("{name}: 0 Huffman tables");
                continue;
            }
            let n = rows.len() as u32;
            let mut max_hist = [0u32; 12];
            let mut mean_le50 = 0u32;
            let mut mean_le55 = 0u32;
            let mut mean_le60 = 0u32;
            let mut mean_le70 = 0u32;
            let mut max_le3 = 0u32;
            let mut max_le7 = 0u32;
            let mut sum_mean = 0u32;
            for &(max_nb, mean_x10) in &rows {
                if (max_nb as usize) < max_hist.len() {
                    max_hist[max_nb as usize] += 1;
                }
                if max_nb <= 3 {
                    max_le3 += 1;
                }
                if max_nb <= 7 {
                    max_le7 += 1;
                }
                if mean_x10 <= 50 {
                    mean_le50 += 1;
                }
                if mean_x10 <= 55 {
                    mean_le55 += 1;
                }
                if mean_x10 <= 60 {
                    mean_le60 += 1;
                }
                if mean_x10 <= 70 {
                    mean_le70 += 1;
                }
                sum_mean += u32::from(mean_x10);
            }
            println!(
                "{name}: tables={n} mean={:.1} max_hist={:?} max<=3={:.0}% max<=7={:.0}% mean<=5.0={:.0}% <=5.5={:.0}% <=6.0={:.0}% <=7.0={:.0}%",
                f64::from(sum_mean) / 10.0 / f64::from(n),
                max_hist,
                100.0 * f64::from(max_le3) / f64::from(n),
                100.0 * f64::from(max_le7) / f64::from(n),
                100.0 * f64::from(mean_le50) / f64::from(n),
                100.0 * f64::from(mean_le55) / f64::from(n),
                100.0 * f64::from(mean_le60) / f64::from(n),
                100.0 * f64::from(mean_le70) / f64::from(n),
            );
        }
    }
}
