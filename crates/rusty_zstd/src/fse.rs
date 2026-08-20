//! FSE table description, build, and symbol decode (RFC 8878 section 4.1).

use crate::bit::{BitFwd, BitRev};
use crate::error::Error;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

#[derive(Clone, Copy, Debug)]
#[repr(C)]
pub(crate) struct FseEntry {
    /// C `FSE_decode_t.newState`.
    pub baseline: u16,
    pub symbol: u8,
    pub num_bits: u8,
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

    pub(crate) fn from_norm(norm: &[i16], accuracy_log: u8) -> Result<Self, Error> {
        if !(5..=9).contains(&accuracy_log) {
            return Err(Error::Corruption);
        }
        let table_size = 1usize << accuracy_log;
        let mut decode = vec![
            FseEntry {
                baseline: 0,
                symbol: 0,
                num_bits: 0,
            };
            table_size
        ];
        let mut symbol_next = vec![0u16; norm.len().max(1)];
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

        for item in &mut decode {
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

        Ok(Self {
            decode,
            accuracy_log,
        })
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
    #[inline(always)]
    #[allow(unsafe_code)]
    pub(crate) fn entry(&self, state: u16) -> FseEntry {
        let dt = self.decode.as_slice();
        debug_assert!(!dt.is_empty() && dt.len().is_power_of_two());
        let i = (state as usize) & dt.len().wrapping_sub(1);
        debug_assert!(i < dt.len());
        *unsafe { dt.get_unchecked(i) }
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
pub(crate) fn read_ncount(
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(FseTable, usize), Error> {
    let (norm, accuracy, consumed) = parse_ncount(src, max_symbol, max_log)?;
    let table = FseTable::from_norm(&norm, accuracy)?;
    Ok((table, consumed))
}

/// NCount header plus matching CTable (dictionary entropy / trainer).
#[cfg(feature = "alloc")]
pub(crate) fn read_ncount_ctable(
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(FseTable, FseCTable, usize), Error> {
    let (norm, accuracy, consumed) = parse_ncount(src, max_symbol, max_log)?;
    let dt = FseTable::from_norm(&norm, accuracy)?;
    let ct = FseCTable::from_norm(&norm, accuracy)?;
    Ok((dt, ct, consumed))
}

fn parse_ncount(
    src: &[u8],
    max_symbol: usize,
    max_log: u8,
) -> Result<(Vec<i16>, u8, usize), Error> {
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
    let mut norm = vec![0i16; max_symbol + 1];
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
    let used = charnum.max(1);
    norm.truncate(used);
    let consumed = bits.bytes_consumed().min(src.len());
    Ok((norm, accuracy, consumed))
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

pub(crate) fn default_ll() -> Result<FseTable, Error> {
    FseTable::from_norm(&DEFAULT_LL_NORM, 6)
}

pub(crate) fn default_ml() -> Result<FseTable, Error> {
    FseTable::from_norm(&DEFAULT_ML_NORM, 6)
}

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

/// FSE encode table (`FSE_buildCTable_wksp`).
#[cfg(feature = "alloc")]
#[derive(Clone, Debug)]
pub(crate) struct FseCTable {
    table_log: u8,
    state_table: Vec<u16>,
    delta: Vec<FseCDelta>,
}

#[cfg(feature = "alloc")]
impl FseCTable {
    pub(crate) fn from_norm(norm: &[i16], table_log: u8) -> Result<Self, Error> {
        if !(5..=9).contains(&table_log) {
            return Err(Error::Corruption);
        }
        let table_size = 1usize << table_log;
        let max_sv = norm.len().saturating_sub(1);
        let mut table_symbol = vec![0u16; table_size];
        let mut cumul = vec![0u16; max_sv + 2];
        let mut high_threshold = table_size - 1;

        cumul[0] = 0;
        for u in 1..=max_sv + 1 {
            if norm[u - 1] == -1 {
                cumul[u] = cumul[u - 1] + 1;
                table_symbol[high_threshold] = (u - 1) as u16;
                high_threshold = high_threshold.saturating_sub(1);
            } else {
                cumul[u] = cumul[u - 1].wrapping_add(norm[u - 1].max(0) as u16);
            }
        }
        cumul[max_sv + 1] = (table_size + 1) as u16;

        let mask = table_size - 1;
        let step = (table_size >> 1) + (table_size >> 3) + 3;
        let mut position = 0usize;
        for (symbol, &freq) in norm.iter().enumerate() {
            for _ in 0..freq.max(0) {
                table_symbol[position] = symbol as u16;
                position = (position + step) & mask;
                while position > high_threshold {
                    position = (position + step) & mask;
                }
            }
        }

        let mut state_table = vec![0u16; table_size];
        for (u, &s) in table_symbol.iter().enumerate() {
            let idx = cumul[s as usize] as usize;
            if idx >= state_table.len() {
                return Err(Error::Corruption);
            }
            state_table[idx] = (table_size + u) as u16;
            cumul[s as usize] = cumul[s as usize].wrapping_add(1);
        }

        let mut delta = vec![FseCDelta { nb: 0, find: 0 }; max_sv + 1];
        let mut total: u32 = 0;
        for s in 0..=max_sv {
            match norm[s] {
                0 => {
                    delta[s].nb = ((u32::from(table_log) + 1) << 16) - (1 << table_log);
                }
                -1 | 1 => {
                    delta[s].nb = (u32::from(table_log) << 16) - (1 << table_log);
                    delta[s].find = total as i32 - 1;
                    total += 1;
                }
                freq => {
                    let freq = freq as u32;
                    let hb = 31 - (freq - 1).leading_zeros();
                    let max_bits_out = u32::from(table_log) - hb;
                    let min_state_plus = freq << max_bits_out;
                    delta[s].nb = (max_bits_out << 16).wrapping_sub(min_state_plus);
                    delta[s].find = total as i32 - freq as i32;
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
pub(crate) fn normalize_count(
    count: &[u32],
    table_log: u8,
    total: u32,
    use_low_prob: bool,
) -> Result<Vec<i16>, Error> {
    if total == 0 || !(5..=9).contains(&table_log) {
        return Err(Error::Corruption);
    }
    let max_sv = count.len() - 1;
    if count.contains(&total) {
        return Err(Error::Corruption);
    }
    let low_prob: i16 = if use_low_prob { -1 } else { 1 };
    let scale = 62u32 - u32::from(table_log);
    let step = (1u64 << 62) / u64::from(total.max(1));
    let v_step = 1u64 << (scale.saturating_sub(20));
    let rtb: [u64; 8] = [0, 473195, 504333, 520860, 550000, 700000, 750000, 830000];
    let mut norm = vec![0i16; max_sv + 1];
    let mut still = 1i32 << table_log;
    let low_threshold = total >> table_log;
    let mut largest = 0usize;
    let mut largest_p: i16 = 0;
    for s in 0..=max_sv {
        let c = count[s];
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
            if rest > v_step * rtb[proba as usize] {
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
    if still.abs() >= i32::from(norm[largest].unsigned_abs()) / 2 && still < 0 {
        // fallback: scale by table size
        let mut n2 = vec![0i16; max_sv + 1];
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
        n2[largest] = (i32::from(n2[largest]) + leftover) as i16;
        if n2[largest] < 1 {
            n2[largest] = 1;
        }
        return Ok(n2);
    }
    norm[largest] = (i32::from(norm[largest]) + still) as i16;
    if norm[largest] < 1 {
        norm[largest] = 1;
    }
    Ok(norm)
}

/// FSE_writeNCount.
#[cfg(feature = "alloc")]
pub(crate) fn write_ncount(norm: &[i16], table_log: u8) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
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
pub(crate) fn decompress_weights(src: &[u8], max_out: usize) -> Result<(Vec<u8>, usize), Error> {
    let (table, n) = read_ncount(src, 255, 6)?;
    if n >= src.len() {
        return Err(Error::Corruption);
    }
    let rest = &src[n..];
    let mut br = BitRev::new(rest)?;
    let mut s1 = table.init_state(&mut br);
    let mut s2 = table.init_state(&mut br);
    let mut out = Vec::new();
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
