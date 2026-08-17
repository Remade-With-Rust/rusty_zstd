//! Compressed block: literals, sequences, match copy.

use crate::bit::BitRev;
use crate::error::Error;
use crate::fse::{self, FseTable};
use crate::huffman::{self, HuffmanTable};
use crate::reader::Reader;

#[cfg(feature = "alloc")]
use alloc::vec;
#[cfg(feature = "alloc")]
use alloc::vec::Vec;

pub(crate) struct BlockState {
    pub huff: Option<HuffmanTable>,
    pub ll: Option<FseTable>,
    pub of: Option<FseTable>,
    pub ml: Option<FseTable>,
    pub reps: [u32; 3],
}

impl BlockState {
    pub(crate) fn new() -> Self {
        Self {
            huff: None,
            ll: None,
            of: None,
            ml: None,
            reps: [1, 4, 8],
        }
    }

    pub(crate) fn from_dict(dict: Option<&crate::dict::Dictionary>) -> Self {
        let Some(d) = dict else {
            return Self::new();
        };
        let Some(e) = d.entropy() else {
            return Self::new();
        };
        Self {
            huff: Some(e.huff_d.clone()),
            ll: Some(e.ll_d.clone()),
            of: Some(e.of_d.clone()),
            ml: Some(e.ml_d.clone()),
            reps: e.reps,
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn decode_compressed_block(
    payload: &[u8],
    out: &mut Vec<u8>,
    window_size: u64,
    block_max: u32,
    state: &mut BlockState,
    dict: &[u8],
    frame_start: usize,
    frame_skipped: usize,
) -> Result<(), Error> {
    let mut r = Reader::new(payload);
    let before = r.remaining();
    let literals = {
        let _l = crate::prof::scope(crate::prof::Stage::DecodeLiterals);
        decode_literals(&mut r, state)?
    };
    // Bit accountant, decode side. Running this over C's OWN frame gives C's
    // literals/sequences split in the same units as the encoder's counters,
    // which is the only way to attribute our size gap to a section.
    crate::prof::note_emit_lit((before - r.remaining()) as u64);
    crate::prof::note_emit_seq(r.remaining() as u64);
    let seq_bytes = r.take(r.remaining())?;
    let _s = crate::prof::scope(crate::prof::Stage::DecodeSeq);
    decode_sequences(
        seq_bytes,
        &literals,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

pub(crate) fn decode_literals(
    r: &mut Reader<'_>,
    state: &mut BlockState,
) -> Result<Vec<u8>, Error> {
    let first = r.u8()?;
    let lit_type = first & 3;
    let size_fmt = (first >> 2) & 3;

    let (regen, csize, n_streams, header_rest) = match lit_type {
        0 | 1 => {
            // Raw / RLE
            let (regen, consumed_after_first) = match size_fmt {
                0 | 2 => (u32::from(first >> 3), 0usize),
                1 => {
                    let b1 = r.u8()?;
                    ((u32::from(first >> 4) + (u32::from(b1) << 4)), 1)
                }
                3 => {
                    let b1 = r.u8()?;
                    let b2 = r.u8()?;
                    (
                        u32::from(first >> 4) + (u32::from(b1) << 4) + (u32::from(b2) << 12),
                        2,
                    )
                }
                _ => return Err(Error::Corruption),
            };
            let _ = consumed_after_first;
            (regen, regen, 1u32, 0u32)
        }
        2 | 3 => {
            // Compressed / Treeless. Size_Format is 2 bits. Header 3-5 bytes.
            let (regen, csize, streams, extra) = match size_fmt {
                0 | 1 => {
                    let b1 = r.u8()?;
                    let b2 = r.u8()?;
                    let regen = (u32::from(first >> 4) + (u32::from(b1) << 4)) & 0x3FF;
                    let csize = ((u32::from(b1) >> 6) + (u32::from(b2) << 2)) & 0x3FF;
                    let streams = if size_fmt == 0 { 1 } else { 4 };
                    (regen, csize, streams, 0u8)
                }
                2 => {
                    let b1 = r.u8()?;
                    let b2 = r.u8()?;
                    let b3 = r.u8()?;
                    let regen =
                        u32::from(first >> 4) + (u32::from(b1) << 4) + ((u32::from(b2) & 3) << 12);
                    let csize = (u32::from(b2) >> 2) + (u32::from(b3) << 6);
                    (regen, csize & 0x3FFF, 4, 0u8)
                }
                3 => {
                    let b1 = r.u8()?;
                    let b2 = r.u8()?;
                    let b3 = r.u8()?;
                    let b4 = r.u8()?;
                    let regen = (u32::from(first) >> 4)
                        + (u32::from(b1) << 4)
                        + ((u32::from(b2) & 0x3F) << 12);
                    let csize = (u32::from(b2) >> 6) + (u32::from(b3) << 2) + (u32::from(b4) << 10);
                    (regen, csize & 0x3FFFF, 4, 0u8)
                }
                _ => return Err(Error::Corruption),
            };
            let _ = extra;
            (regen, csize, streams, 0)
        }
        _ => return Err(Error::Corruption),
    };
    let _ = header_rest;

    match lit_type {
        0 => {
            let src = r.take(regen as usize)?;
            Ok(src.to_vec())
        }
        1 => {
            let b = r.u8()?;
            Ok(vec![b; regen as usize])
        }
        2 => {
            let section = r.take(csize as usize)?;
            let (table, tree_size) = huffman::read_table(section)?;
            // BRICK 63: MOVE the freshly-read table into `state`, then borrow it
            // back to decode with. It was being CLONED into `state` and the
            // original used for the decode -- a full decode-table allocation and
            // copy per block that nothing ever read. `DecodeLiterals` is 69.6% of
            // mr's decode, and mr takes this arm on nearly every block.
            state.huff = Some(table);
            let table = state.huff.as_ref().ok_or(Error::Corruption)?;
            decode_huff_streams(table, &section[tree_size..], regen, n_streams)
        }
        3 => {
            let table = state.huff.as_ref().ok_or(Error::Corruption)?;
            let section = r.take(csize as usize)?;
            decode_huff_streams(table, section, regen, n_streams)
        }
        _ => Err(Error::Corruption),
    }
}

fn decode_huff_streams(
    table: &HuffmanTable,
    src: &[u8],
    regen: u32,
    n_streams: u32,
) -> Result<Vec<u8>, Error> {
    let mut out = vec![0u8; regen as usize];
    if n_streams == 1 {
        table.decode_stream(src, &mut out)?;
        return Ok(out);
    }
    if src.len() < 6 {
        return Err(Error::Corruption);
    }
    let s1 = u16::from_le_bytes([src[0], src[1]]) as usize;
    let s2 = u16::from_le_bytes([src[2], src[3]]) as usize;
    let s3 = u16::from_le_bytes([src[4], src[5]]) as usize;
    let total = src.len() - 6;
    if s1 + s2 + s3 > total {
        return Err(Error::Corruption);
    }
    let s4 = total - s1 - s2 - s3;
    let rest = &src[6..];
    if s1 == 0 || s2 == 0 || s3 == 0 || s4 == 0 {
        return Err(Error::Corruption);
    }
    let n = out.len();
    let chunk = regen.div_ceil(4) as usize;
    if chunk == 0 || n < 4 {
        return Err(Error::Corruption);
    }
    let (d0, rest_d) = out.split_at_mut(chunk.min(n));
    let n1 = rest_d.len();
    let (d1, rest_d) = rest_d.split_at_mut(chunk.min(n1));
    let n2 = rest_d.len();
    let (d2, d3) = rest_d.split_at_mut(chunk.min(n2));
    if d0.is_empty() || d1.is_empty() || d2.is_empty() || d3.is_empty() {
        return Err(Error::Corruption);
    }
    table.decode_4x(
        &rest[..s1],
        &rest[s1..s1 + s2],
        &rest[s1 + s2..s1 + s2 + s3],
        &rest[s1 + s2 + s3..],
        d0,
        d1,
        d2,
        d3,
    )?;
    Ok(out)
}

pub(crate) const LL_BASE: [u32; 36] = [
    0, 1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 18, 20, 22, 24, 28, 32, 40, 48, 64,
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536,
];
pub(crate) const LL_BITS: [u8; 36] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 2, 2, 3, 3, 4, 6, 7, 8, 9, 10, 11,
    12, 13, 14, 15, 16,
];
pub(crate) const ML_BASE: [u32; 53] = [
    3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26, 27,
    28, 29, 30, 31, 32, 33, 34, 35, 37, 39, 41, 43, 47, 51, 59, 67, 83, 99, 131, 259, 515, 1027,
    2051, 4099, 8195, 16387, 32771, 65539,
];
pub(crate) const ML_BITS: [u8; 53] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0,
    1, 1, 1, 1, 2, 2, 3, 3, 4, 4, 5, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16,
];

#[allow(clippy::too_many_arguments)]
/// BRICK 64: `SEQCHECK` is a const generic, not a runtime read.
///
/// The per-sequence guard called `seqcheck_hoisted()` -- an ATOMIC load plus a
/// match -- on EVERY sequence (1.8M times on webster). LLVM will not hoist an
/// atomic out of a loop, so the shipping build paid it per sequence to ask a
/// question whose answer is fixed for the whole process. As a const it vanishes
/// from the loop entirely.
pub(crate) fn decode_sequences(
    src: &[u8],
    literals: &[u8],
    out: &mut Vec<u8>,
    window_size: u64,
    block_max: u32,
    state: &mut BlockState,
    dict: &[u8],
    frame_start: usize,
    frame_skipped: usize,
) -> Result<(), Error> {
    if src.is_empty() {
        return Err(Error::Corruption);
    }
    let mut pos = 0usize;
    let byte0 = src[0];
    pos += 1;
    let nseq = if byte0 == 0 {
        out.extend_from_slice(literals);
        return Ok(());
    } else if byte0 < 128 {
        byte0 as u32
    } else if byte0 < 255 {
        if pos >= src.len() {
            return Err(Error::Corruption);
        }
        let b1 = src[pos];
        pos += 1;
        ((u32::from(byte0) - 128) << 8) + u32::from(b1)
    } else {
        if pos + 1 >= src.len() {
            return Err(Error::Corruption);
        }
        let b1 = src[pos];
        let b2 = src[pos + 1];
        pos += 2;
        u32::from(b1) + (u32::from(b2) << 8) + 0x7F00
    };
    if pos >= src.len() {
        return Err(Error::Corruption);
    }
    let modes = src[pos];
    pos += 1;
    if modes & 3 != 0 {
        return Err(Error::Corruption);
    }
    let ll_mode = modes >> 6;
    let of_mode = (modes >> 4) & 3;
    let ml_mode = (modes >> 2) & 3;

    let (ll, n) = seq_table(
        &src[pos..],
        ll_mode,
        35,
        9,
        state.ll.as_ref(),
        fse::default_ll,
    )?;
    pos += n;
    let (of, n) = seq_table(
        &src[pos..],
        of_mode,
        31,
        8,
        state.of.as_ref(),
        fse::default_of,
    )?;
    pos += n;
    let (ml, n) = seq_table(
        &src[pos..],
        ml_mode,
        52,
        9,
        state.ml.as_ref(),
        fse::default_ml,
    )?;
    pos += n;

    let bitstream = &src[pos..];
    let mut br = BitRev::new(bitstream)?;
    let mut ll_s = ll.init_state(&mut br);
    let mut of_s = of.init_state(&mut br);
    let mut ml_s = ml.init_state(&mut br);

    let mut lit_pos = 0usize;
    // Built ONCE per block; the loop then passes a single reference.
    let mctx = MatchCtx {
        dict,
        frame_start,
        frame_skipped,
        window_size,
        block_max,
    };
    // BRICK 64b: hoist the arm read OUT of the loop, WITHOUT making the function
    // generic.
    //
    // `seqcheck_hoisted()` is an ATOMIC load; LLVM will not hoist an atomic out
    // of a loop, so it was paid per sequence (1.8M times on webster). The const
    // -generic version (brick 64) removed it but split `decode_sequences` out of
    // `decode_compressed_block` as its own symbol, and that split cost MORE than
    // the atomic saved -- measured, on a quiet box (null arm 1.0000):
    // webster C/us decomp 1.80 -> 1.92, nci 2.37 -> 2.45, samba 2.01 -> 2.09,
    // xml 2.10 -> 2.16, i.e. 3-6% SLOWER on every sequence-heavy file.
    //
    // A plain local gets the same per-sequence saving with no structural change.
    let litcopy_arm = litcopy_on();
    let seqcheck = seqcheck_hoisted();
    for i in 0..nseq {
        let _ = br.reload();
        let ll_e = ll.entry(ll_s);
        let of_e = of.entry(of_s);
        let ml_e = ml.entry(ml_s);
        let ll_code = ll_e.symbol as usize;
        let of_code = of_e.symbol as usize;
        let ml_code = ml_e.symbol as usize;
        // No per-sequence range test: ALL FOUR table modes now bound their
        // symbols at build time (see `seq_table`), so `ll_code <= 35`,
        // `ml_code <= 52` and `of_code <= 31` hold by construction. This ran
        // ~1M times per file to re-prove a per-block invariant.
        debug_assert!(ll_code <= 35 && ml_code <= 52 && of_code <= 31);
        if !seqcheck && (ll_code > 35 || ml_code > 52 || of_code > 31) {
            return Err(Error::Corruption);
        }
        let offset_add = br.read_bits(of_code as u32);
        let ml_add = br.read_bits(u32::from(ML_BITS[ml_code]));
        let ll_add = br.read_bits(u32::from(LL_BITS[ll_code]));
        let litlen = LL_BASE[ll_code] + ll_add;
        let matchlen = ML_BASE[ml_code] + ml_add;
        let offset_value = if of_code == 0 {
            1
        } else {
            (1u32 << of_code) + offset_add
        };

        copy_literals(literals, &mut lit_pos, litlen, out, litcopy_arm)?;
        let offset = resolve_offset(offset_value, litlen, &mut state.reps)?;
        copy_match(out, &mctx, offset, matchlen)?;

        if i + 1 != nseq {
            let _ = br.reload();
            ll_s = FseTable::advance(ll_e, &mut br);
            ml_s = FseTable::advance(ml_e, &mut br);
            of_s = FseTable::advance(of_e, &mut br);
        }
    }
    out.extend_from_slice(&literals[lit_pos..]);
    state.ll = Some(ll);
    state.of = Some(of);
    state.ml = Some(ml);
    Ok(())
}

/// Decode sequence codes without executing (entropy-tail oracle).
#[cfg(test)]
pub(crate) fn debug_seq_codes(
    src: &[u8],
    state: &BlockState,
) -> Result<(u32, u8, Vec<(u32, u32, u32, u8, u8, u8)>), Error> {
    if src.is_empty() {
        return Err(Error::Corruption);
    }
    let mut pos = 0usize;
    let byte0 = src[0];
    pos += 1;
    let nseq = if byte0 == 0 {
        return Ok((0, 0, Vec::new()));
    } else if byte0 < 128 {
        byte0 as u32
    } else if byte0 < 255 {
        let b1 = *src.get(pos).ok_or(Error::Corruption)?;
        pos += 1;
        ((u32::from(byte0) - 128) << 8) + u32::from(b1)
    } else {
        let b1 = *src.get(pos).ok_or(Error::Corruption)?;
        let b2 = *src.get(pos + 1).ok_or(Error::Corruption)?;
        pos += 2;
        u32::from(b1) + (u32::from(b2) << 8) + 0x7F00
    };
    let modes = *src.get(pos).ok_or(Error::Corruption)?;
    pos += 1;
    let ll_mode = modes >> 6;
    let of_mode = (modes >> 4) & 3;
    let ml_mode = (modes >> 2) & 3;
    let (ll, n) = seq_table(
        &src[pos..],
        ll_mode,
        35,
        9,
        state.ll.as_ref(),
        fse::default_ll,
    )?;
    pos += n;
    let (of, n) = seq_table(
        &src[pos..],
        of_mode,
        31,
        8,
        state.of.as_ref(),
        fse::default_of,
    )?;
    pos += n;
    let (ml, n) = seq_table(
        &src[pos..],
        ml_mode,
        52,
        9,
        state.ml.as_ref(),
        fse::default_ml,
    )?;
    pos += n;
    let bitstream = &src[pos..];
    let mut br = BitRev::new(bitstream)?;
    let mut ll_s = ll.init_state(&mut br);
    let mut of_s = of.init_state(&mut br);
    let mut ml_s = ml.init_state(&mut br);
    let mut out = Vec::with_capacity(nseq as usize);
    for i in 0..nseq {
        let _ = br.reload();
        let ll_e = ll.entry(ll_s);
        let of_e = of.entry(of_s);
        let ml_e = ml.entry(ml_s);
        let llc = ll_e.symbol as u8;
        let ofc = of_e.symbol as u8;
        let mlc = ml_e.symbol as u8;
        if llc > 35 || mlc > 52 || ofc > 31 {
            return Err(Error::Corruption);
        }
        let offset_add = br.read_bits(u32::from(ofc));
        let ml_add = br.read_bits(u32::from(ML_BITS[mlc as usize]));
        let ll_add = br.read_bits(u32::from(LL_BITS[llc as usize]));
        let litlen = LL_BASE[llc as usize] + ll_add;
        let matchlen = ML_BASE[mlc as usize] + ml_add;
        let ov = if ofc == 0 {
            1
        } else {
            (1u32 << ofc) + offset_add
        };
        out.push((litlen, matchlen, ov, llc, mlc, ofc));
        if i + 1 != nseq {
            let _ = br.reload();
            ll_s = FseTable::advance(ll_e, &mut br);
            ml_s = FseTable::advance(ml_e, &mut br);
            of_s = FseTable::advance(of_e, &mut br);
        }
    }
    Ok((nseq, modes, out))
}

fn seq_table(
    src: &[u8],
    mode: u8,
    max_sym: usize,
    max_log: u8,
    prev: Option<&FseTable>,
    predefined: fn() -> Result<FseTable, Error>,
) -> Result<(FseTable, usize), Error> {
    match mode {
        0 => Ok((predefined()?, 0)),
        1 => {
            // RLE takes its symbol as a RAW STREAM BYTE. Every other mode
            // bounds the symbol by `max_sym` at build time (predefined by its
            // norm table's length, compressed by `read_ncount`'s `charnum >
            // max_symbol` reject, repeat by inheriting an already-validated
            // table) -- this one did not. Untrusted input could therefore
            // reach `1u32 << of_code` with of_code >= 32, which is UB, and
            // index `LL_BITS`/`ML_BITS` out of range. Bound it HERE, once per
            // block, instead of re-testing every sequence in the hot loop.
            let sym = *src.first().ok_or(Error::Corruption)?;
            if usize::from(sym) > max_sym {
                return Err(Error::Corruption);
            }
            Ok((FseTable::rle(u16::from(sym)), 1))
        }
        2 => fse::read_ncount(src, max_sym, max_log),
        3 => {
            let t = prev.cloned().ok_or(Error::Corruption)?;
            Ok((t, 0))
        }
        _ => Err(Error::Corruption),
    }
}

/// Brick 45 arm: per-sequence symbol-range test hoisted to table build.
static SEQCHECK_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook; `RZSTD_SEQCHECK_HOIST=0` restores the per-sequence test.
pub fn set_seqcheck_arm(on: bool) {
    SEQCHECK_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

#[inline(always)]
fn seqcheck_hoisted() -> bool {
    use core::sync::atomic::Ordering;
    match SEQCHECK_ARM.load(Ordering::Relaxed) {
        1 => false,
        2 => true,
        _ => {
            let on = std::env::var("RZSTD_SEQCHECK_HOIST")
                .map(|v| v != "0")
                .unwrap_or(true);
            SEQCHECK_ARM.store(if on { 2 } else { 1 }, Ordering::Relaxed);
            on
        }
    }
}

/// Copy one literal run into the output.
///
/// The measured mean literal run is short (~7.8 bytes on samba, ~7.6 on
/// webster), and `extend_from_slice` is a **runtime-length** memcpy that the
/// compiler cannot lower to a constant-width move -- so the call overhead,
/// not the bytes, is the cost at ~740k calls per block-set. C solves this with
/// a fixed-width `ZSTD_copy16` plus an over-allocated output buffer.
///
/// The fixed-width path is taken only when 16 source bytes are readable and 16
/// destination bytes are already reserved; otherwise the checked path runs.
/// Both produce identical output (`copy_literals_fast_matches_checked`).
#[allow(unsafe_code)]
#[inline]
/// BRICK 79: the literal-copy arm is a PARAMETER, not a per-sequence read.
///
/// `litcopy_on()` sat in the fast-path guard, so it was read once per SEQUENCE
/// (~15M across the corpus). It is fixed for the whole process. Fourth instance
/// of this shape: brick 49 (`use_rep`), 64 (`seqcheck_hoisted`), 77
/// (`lit_push_enabled`), now this.
/// BRICK 81: inlined into the sequence loop.
///
/// The decode loop's stack traffic is DIFFUSE -- ~12 slots touched 1-3x each,
/// with no dominant frame-constant. That signature is not missing
/// specialisation, it is a CALL BOUNDARY: everything live across a call must be
/// spilled because caller-saved registers are clobbered. `copy_literals`' fast
/// path is a 16/32-byte copy, so inlining it removes the boundary without
/// meaningful code growth.
#[inline(always)]
fn copy_literals(
    literals: &[u8],
    lit_pos: &mut usize,
    litlen: u32,
    out: &mut Vec<u8>,
    arm: bool,
) -> Result<(), Error> {
    let n = litlen as usize;
    let end = lit_pos.checked_add(n).ok_or(Error::Corruption)?;
    if end > literals.len() {
        return Err(Error::Corruption);
    }
    let len = out.len();
    if arm && n <= 16 && *lit_pos + 16 <= literals.len() && out.capacity() - len >= 16 {
        // SAFETY: `lit_pos + 16 <= literals.len()` gives 16 readable source
        // bytes. `capacity - len >= 16` gives 16 writable destination bytes
        // inside the allocation. `literals` and `out` are distinct buffers
        // (separate borrows), so the regions cannot overlap. Exactly `n <= 16`
        // bytes are published by `set_len`; the rest stay in spare capacity.
        unsafe {
            core::ptr::copy_nonoverlapping(
                literals.as_ptr().add(*lit_pos),
                out.as_mut_ptr().add(len),
                16,
            );
            out.set_len(len + n);
        }
        *lit_pos = end;
        return Ok(());
    }
    // BRICK 80: a 32-byte tier above the 16-byte one.
    //
    // MEASURED FIRST (19.6M literal copies across the corpus):
    //   <=16B  17,775,105  90.5%  -- already fast
    //   17-32B  1,128,428   5.7%  -- fell to `extend_from_slice`, i.e. a memcpy CALL
    //   >32B      728,346   3.7%  -- memcpy regardless
    // So this tier converts 1.13M memcpy calls, 61% of the remaining slow path.
    // Same invariant as the 16-byte tier, just wider -- and the same shape brick
    // 37 already proved on match copies.
    if arm && n <= 32 && *lit_pos + 32 <= literals.len() && out.capacity() - len >= 32 {
        // SAFETY: 32 readable source bytes and 32 writable destination bytes are
        // guaranteed by the two bounds above; `literals` and `out` are distinct
        // buffers. Exactly `n <= 32` bytes are published by `set_len`.
        unsafe {
            core::ptr::copy_nonoverlapping(
                literals.as_ptr().add(*lit_pos),
                out.as_mut_ptr().add(len),
                32,
            );
            out.set_len(len + n);
        }
        *lit_pos = end;
        return Ok(());
    }
    out.extend_from_slice(&literals[*lit_pos..end]);
    *lit_pos = end;
    Ok(())
}

/// Runtime arms for the pre-2026-08-15 bricks, so the in-process ABBA
/// harness can re-adjudicate them. Each defaults ON (shipping behaviour).
static LUT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);
static LITCOPY_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);
static MATCHCOPY_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);

/// Brick 35 arm: LL/ML code LUT vs the linear-scan oracle.
pub fn set_lut_arm(on: bool) {
    LUT_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// Brick 36 arm: fixed-width literal copy.
pub fn set_litcopy_arm(on: bool) {
    LITCOPY_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// Brick 37 arm: fixed-width match copy.
pub fn set_matchcopy_arm(on: bool) {
    MATCHCOPY_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

#[inline(always)]
fn lut_on() -> bool {
    LUT_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[inline(always)]
fn litcopy_on() -> bool {
    LITCOPY_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[inline(always)]
fn matchcopy_on() -> bool {
    MATCHCOPY_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}

/// Highest table code whose base is `<= val`. **The oracle.**
///
/// A linear scan from the top of the table. Literal and match lengths are
/// overwhelmingly small, so `val >= base[i]` fails nearly all the way down:
/// ~36 iterations for LL and ~53 for ML on a typical sequence. This stays as
/// the correctness reference and as the path for values above the LUT.
pub(crate) fn code_from_base(val: u32, base: &[u32], bits: &[u8]) -> (u8, u32, u8) {
    let mut i = base.len() - 1;
    loop {
        if val >= base[i] {
            return (i as u8, val - base[i], bits[i]);
        }
        if i == 0 {
            return (0, val, 0);
        }
        i -= 1;
    }
}

/// Direct value-to-code lookup covering the common range (C keeps `LL_Code[64]`
/// / `ML_Code[128]` for exactly this reason). Sized so that measured mean
/// lengths -- samba ll ~7.8, ml ~21.3 -- sit far inside.
const LL_LUT_LEN: usize = 64;
const ML_LUT_LEN: usize = 256;

/// Build the LUT by evaluating [`code_from_base`] at compile time, so the
/// fast path cannot drift from the oracle by construction.
const fn build_code_lut<const N: usize>(base: &[u32]) -> [u8; N] {
    let mut out = [0u8; N];
    let mut v = 0usize;
    while v < N {
        let mut i = base.len() - 1;
        loop {
            if v as u32 >= base[i] {
                out[v] = i as u8;
                break;
            }
            if i == 0 {
                out[v] = 0;
                break;
            }
            i -= 1;
        }
        v += 1;
    }
    out
}

static LL_CODE_LUT: [u8; LL_LUT_LEN] = build_code_lut::<LL_LUT_LEN>(&LL_BASE);
static ML_CODE_LUT: [u8; ML_LUT_LEN] = build_code_lut::<ML_LUT_LEN>(&ML_BASE);

pub(crate) fn ll_code(len: u32) -> (u8, u32, u8) {
    if lut_on() && (len as usize) < LL_LUT_LEN {
        let c = LL_CODE_LUT[len as usize] as usize;
        // `code_from_base` falls off the bottom as `(0, val, 0)` rather than
        // `val - base[0]`, so mirror that instead of subtracting blindly.
        let base = LL_BASE[c];
        return if len >= base {
            (c as u8, len - base, LL_BITS[c])
        } else {
            (0, len, 0)
        };
    }
    code_from_base(len, &LL_BASE, &LL_BITS)
}

pub(crate) fn ml_code(len: u32) -> (u8, u32, u8) {
    if lut_on() && (len as usize) < ML_LUT_LEN {
        let c = ML_CODE_LUT[len as usize] as usize;
        // ML_BASE[0] is 3, so any len < 3 lands here and must NOT subtract.
        let base = ML_BASE[c];
        return if len >= base {
            (c as u8, len - base, ML_BITS[c])
        } else {
            (0, len, 0)
        };
    }
    code_from_base(len, &ML_BASE, &ML_BITS)
}

pub(crate) fn of_code(offset_value: u32) -> (u8, u32) {
    if offset_value <= 1 {
        return (0, 0);
    }
    let code = 31 - offset_value.leading_zeros();
    let extra = offset_value - (1u32 << code);
    (code as u8, extra)
}

/// Repeat-offset code (`1..=3`) or `offset + 3`, matching [`resolve_offset`].
pub(crate) fn offset_value_for(offset: u32, litlen: u32, reps: &[u32; 3]) -> u32 {
    if litlen == 0 {
        if offset == reps[1] {
            1
        } else if offset == reps[2] {
            2
        } else if reps[0] > 1 && offset == reps[0] - 1 {
            3
        } else {
            offset.saturating_add(3)
        }
    } else if offset == reps[0] {
        1
    } else if offset == reps[1] {
        2
    } else if offset == reps[2] {
        3
    } else {
        offset.saturating_add(3)
    }
}

pub(crate) fn resolve_offset(
    offset_value: u32,
    litlen: u32,
    reps: &mut [u32; 3],
) -> Result<u32, Error> {
    let offset = if offset_value > 3 {
        offset_value - 3
    } else if litlen == 0 {
        match offset_value {
            1 => reps[1],
            2 => reps[2],
            3 => reps[0]
                .checked_sub(1)
                .filter(|&o| o > 0)
                .ok_or(Error::Corruption)?,
            _ => return Err(Error::Corruption),
        }
    } else {
        match offset_value {
            1 => reps[0],
            2 => reps[1],
            3 => reps[2],
            _ => return Err(Error::Corruption),
        }
    };

    let is_new = offset_value > 3 || (offset_value == 3 && litlen == 0);
    if is_new {
        reps[2] = reps[1];
        reps[1] = reps[0];
        reps[0] = offset;
    } else {
        let which = if litlen == 0 {
            offset_value + 1
        } else {
            offset_value
        };
        match which {
            1 => {}
            2 => reps.swap(0, 1),
            3 => reps.rotate_right(1),
            _ => {}
        }
    }
    Ok(offset)
}

/// BRICK 66: the five FRAME-CONSTANT arguments of `copy_match`, bundled.
///
/// `copy_match` took 8 arguments. The Windows x64 ABI passes only FOUR in
/// registers, so every sequence marshalled the rest onto the stack -- and since
/// they were already spilled, that was a stack-to-stack shuffle of values that
/// do not change for the whole block (visible as the `movq N(%rbp),%rax;
/// movq %rax,M(%rsp)` pairs ahead of the call). Bundling them into one
/// reference brings the call to 4 arguments: ctx, out, offset, matchlen --
/// entirely in registers.
struct MatchCtx<'a> {
    dict: &'a [u8],
    frame_start: usize,
    frame_skipped: usize,
    window_size: u64,
    block_max: u32,
}

#[allow(clippy::explicit_counter_loop)]
fn copy_match(
    out: &mut Vec<u8>,
    ctx: &MatchCtx<'_>,
    offset: u32,
    matchlen: u32,
) -> Result<(), Error> {
    let MatchCtx {
        dict,
        frame_start,
        frame_skipped,
        window_size,
        block_max,
    } = *ctx;
    let off = offset as usize;
    if off == 0 {
        return Err(Error::Corruption);
    }
    let retained = out.len().saturating_sub(frame_start);
    let produced = retained.saturating_add(frame_skipped);
    let virtual_len = dict.len() + produced;
    if off > virtual_len {
        return Err(Error::Corruption);
    }
    let src_pos0 = virtual_len - off;
    // libzstd keeps the full dictionary as a prefix (ZSTD_refDictContent).
    // Window_Size limits back-refs into decoded output; dict matches may exceed it.
    if src_pos0 >= dict.len() && (off as u64) > window_size {
        return Err(Error::Corruption);
    }
    let len = matchlen as usize;
    if len > block_max as usize {
        return Err(Error::Corruption);
    }
    if src_pos0 >= dict.len() {
        let frame_off = src_pos0 - dict.len();
        if frame_off < frame_skipped {
            return Err(Error::Corruption);
        }
        let i = frame_start + (frame_off - frame_skipped);
        return copy_from_decoded(out, i, len);
    }
    let mut src_pos = src_pos0;
    out.reserve(len);
    for _ in 0..len {
        let b = if src_pos < dict.len() {
            dict[src_pos]
        } else {
            let frame_off = src_pos - dict.len();
            if frame_off < frame_skipped {
                return Err(Error::Corruption);
            }
            let i = frame_start + (frame_off - frame_skipped);
            *out.get(i).ok_or(Error::Corruption)?
        };
        out.push(b);
        src_pos += 1;
    }
    Ok(())
}

/// Overlapping-safe copy from already-decoded `out[src..]`. C wildcopy equivalent.
#[allow(unsafe_code)]
fn copy_from_decoded(out: &mut Vec<u8>, src: usize, len: usize) -> Result<(), Error> {
    if src >= out.len() {
        return Err(Error::Corruption);
    }
    if len == 0 {
        return Ok(());
    }
    let offset = out.len() - src;
    if offset == 0 {
        return Err(Error::Corruption);
    }
    // Offset 1 is a byte splat (C wildcopy / ZSTD_overlapCopy8). Doubling
    // extend_from_within would memcpy 1, then 2, then 4, ...
    if offset == 1 {
        let b = out[src];
        out.resize(out.len() + len, b);
        return Ok(());
    }
    // FAST PATH: fixed-width 32-byte move for the common short match.
    // `extend_from_within` is a runtime-length copy and the measured mean match
    // run is only ~21 bytes, so the call overhead dominates (same shape as the
    // literal copy). C uses a fixed-width wildcopy here for the same reason.
    //
    // `offset >= 32` does double duty: it guarantees 32 readable source bytes
    // (`src + 32 <= out.len()`) AND that the source range ends at or before the
    // destination start, so the two 32-byte regions cannot overlap.
    if matchcopy_on() && len <= 32 && offset >= 32 && out.capacity() - out.len() >= 32 {
        let dst_at = out.len();
        // SAFETY: `offset >= 32` means `src + 32 <= out.len() == dst_at`, so
        // the source is fully initialised and disjoint from the destination.
        // `capacity - len >= 32` gives 32 writable bytes inside the
        // allocation. Only `len <= 32` bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 32);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    // BRICK 82: a 16-byte tier for short matches with SHORT offsets.
    //
    // The 32-byte path above needs `offset >= 32` so the 32-byte source read
    // cannot overlap the destination. The census found **1,153,839 copies
    // (5.8%)** with `len <= 32` and `2 <= offset < 32` falling past it to
    // `extend_from_within` -- a runtime-length memcpy CALL.
    //
    // Halving the width halves the requirement: `offset >= 16` guarantees the
    // 16-byte regions are disjoint, capturing the `len <= 16` part of that
    // slice. Same invariant as the 32-byte tier, same shape as brick 80 on
    // literals.
    if matchcopy_on() && len <= 16 && offset >= 16 && out.capacity() - out.len() >= 16 {
        let dst_at = out.len();
        // SAFETY: `offset >= 16` means `src + 16 <= out.len() == dst_at`, so the
        // source is initialised and disjoint from the destination.
        // `capacity - len >= 16` gives 16 writable bytes. Only `len <= 16`
        // bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 16);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    if offset >= len {
        out.extend_from_within(src..src + len);
        return Ok(());
    }
    out.reserve(len);
    let mut copied = 0usize;
    while copied < len {
        let avail = out.len() - src;
        if avail == 0 {
            return Err(Error::Corruption);
        }
        let take = (len - copied).min(avail);
        out.extend_from_within(src..src + take);
        copied += take;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn repeat_offset_rfc_table18() {
        let mut reps = [1u32, 4, 8];
        assert_eq!(resolve_offset(1114, 11, &mut reps).unwrap(), 1111);
        assert_eq!(reps, [1111, 1, 4]);
        assert_eq!(resolve_offset(1, 22, &mut reps).unwrap(), 1111);
        assert_eq!(reps, [1111, 1, 4]);
        assert_eq!(resolve_offset(2225, 22, &mut reps).unwrap(), 2222);
        assert_eq!(reps, [2222, 1111, 1]);
    }

    #[test]
    fn copy_from_decoded_matches_byte_push() {
        // Offsets and lengths chosen to straddle the 32-byte fixed-width gate
        // in BOTH directions, and capacities chosen so the same (off, len)
        // runs once through the fast path and once through the fallback.
        for off in [1usize, 2, 3, 4, 7, 8, 15, 16, 31, 32, 33, 40, 70, 128, 300] {
            for len in [1usize, 2, 3, 5, 7, 8, 9, 15, 16, 31, 32, 33, 40, 64, 1000] {
                for spare in [0usize, 31, 32, 4096] {
                    let prefix: Vec<u8> = (0..off.max(8)).map(|i| (i % 251) as u8).collect();
                    let mut slow = prefix.clone();
                    let mut fast = Vec::with_capacity(prefix.len() + spare);
                    fast.extend_from_slice(&prefix);
                    let src = slow.len() - off;
                    for _ in 0..len {
                        let b = slow[slow.len() - off];
                        slow.push(b);
                    }
                    copy_from_decoded(&mut fast, src, len).unwrap();
                    assert_eq!(slow, fast, "off={off} len={len} spare={spare}");
                }
            }
        }
    }

    /// The fixed-width match copy must never read or publish beyond the match:
    /// bytes past `len` in spare capacity must not become visible.
    #[test]
    fn copy_from_decoded_publishes_exactly_len() {
        for off in [32usize, 33, 64, 200] {
            for len in [1usize, 5, 17, 31, 32] {
                let prefix: Vec<u8> = (0..off + 64).map(|i| (i % 251) as u8).collect();
                let before = prefix.len();
                let mut v = Vec::with_capacity(before + 4096);
                v.extend_from_slice(&prefix);
                copy_from_decoded(&mut v, before - off, len).unwrap();
                assert_eq!(v.len(), before + len, "off={off} len={len}");
                assert_eq!(&v[..before], &prefix[..], "prefix damaged");
                for k in 0..len {
                    assert_eq!(v[before + k], prefix[before - off + k], "off={off} k={k}");
                }
            }
        }
    }

    /// An RLE sequence-table symbol comes straight off the wire. It MUST be
    /// rejected at build time when it exceeds the type's max symbol -- the hot
    /// loop no longer re-checks, and an out-of-range `of_code` would reach
    /// `1u32 << of_code` (UB at >= 32) and index `LL_BITS`/`ML_BITS` OOB.
    #[test]
    fn rle_seq_table_rejects_out_of_range_symbol() {
        // (max_sym, in-range symbol, out-of-range symbol)
        for &(max_sym, ok_sym, bad_sym) in &[(35usize, 35u8, 36u8), (31, 31, 32), (52, 52, 53)] {
            let good = seq_table(&[ok_sym], 1, max_sym, 9, None, fse::default_ll);
            assert!(good.is_ok(), "max_sym={max_sym} sym={ok_sym} should build");
            let bad = seq_table(&[bad_sym], 1, max_sym, 9, None, fse::default_ll);
            assert!(
                matches!(bad, Err(Error::Corruption)),
                "max_sym={max_sym} sym={bad_sym} must be rejected, got {:?}",
                bad.map(|_| ())
            );
        }
        // 255 is the worst case a single byte can carry.
        assert!(matches!(
            seq_table(&[255u8], 1, 31, 8, None, fse::default_of),
            Err(Error::Corruption)
        ));
    }

    /// The LUT fast path must equal the linear-scan oracle everywhere,
    /// including across the LUT boundary and at every table base/base-1.
    #[test]
    fn ll_ml_code_lut_matches_linear_scan() {
        let mut probes: Vec<u32> = (0..4096).collect();
        for &b in LL_BASE.iter().chain(ML_BASE.iter()) {
            probes.push(b.saturating_sub(1));
            probes.push(b);
            probes.push(b.saturating_add(1));
        }
        probes.push(LL_LUT_LEN as u32 - 1);
        probes.push(LL_LUT_LEN as u32);
        probes.push(ML_LUT_LEN as u32 - 1);
        probes.push(ML_LUT_LEN as u32);
        probes.extend([65535, 65536, 65537, 100_000, u32::MAX - 1]);
        for v in probes {
            assert_eq!(
                ll_code(v),
                code_from_base(v, &LL_BASE, &LL_BITS),
                "ll_code({v})"
            );
            assert_eq!(
                ml_code(v),
                code_from_base(v, &ML_BASE, &ML_BITS),
                "ml_code({v})"
            );
        }
    }

    /// The fixed-width literal copy must equal the checked path for every
    /// length and every capacity/tail combination, including the cases that
    /// must FALL BACK (short source tail, no spare capacity, n > 16).
    #[test]
    fn copy_literals_fast_matches_checked() {
        let lits: Vec<u8> = (0..300u32).map(|i| (i % 251) as u8).collect();
        for &pos in &[0usize, 1, 7, 63, 280, 284, 299] {
            for n in 0usize..=40 {
                if pos + n > lits.len() {
                    continue;
                }
                // Exercise both a roomy buffer (fast path) and an exact-fit
                // one (forces the checked path via the capacity guard).
                for spare in [0usize, 1, 15, 16, 64] {
                    let mut fast = Vec::with_capacity(8 + spare);
                    fast.extend_from_slice(b"PREFIX!!");
                    let mut want = fast.clone();
                    want.extend_from_slice(&lits[pos..pos + n]);

                    let mut p = pos;
                    copy_literals(&lits, &mut p, n as u32, &mut fast, true).unwrap();
                    assert_eq!(fast, want, "pos={pos} n={n} spare={spare}");
                    assert_eq!(p, pos + n, "lit_pos pos={pos} n={n}");
                }
            }
        }
    }

    #[test]
    fn copy_literals_rejects_overrun() {
        let lits = [1u8, 2, 3];
        let mut out = Vec::with_capacity(64);
        let mut p = 0usize;
        assert!(copy_literals(&lits, &mut p, 4, &mut out, true).is_err());
        let mut p2 = 2usize;
        assert!(copy_literals(&lits, &mut p2, u32::MAX, &mut out, true).is_err());
    }

    /// Exhaustive over the whole LUT range -- no sampling.
    #[test]
    fn code_lut_exhaustive_over_lut_domain() {
        for v in 0..(ML_LUT_LEN as u32 * 2) {
            assert_eq!(ll_code(v), code_from_base(v, &LL_BASE, &LL_BITS));
            assert_eq!(ml_code(v), code_from_base(v, &ML_BASE, &ML_BITS));
        }
    }
}
