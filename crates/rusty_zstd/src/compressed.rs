// The `drop(g_*)` calls below END A PROFILER SCOPE. With `--features profile`
// the guard is a real `Drop` that records the stage; without it the guard is a
// ZST and the drop is a no-op -- which is the configuration clippy sees.
#![allow(clippy::drop_non_drop)]

//! Compressed block: literals, sequences, match copy.

use crate::bit::BitRev;
use crate::error::Error;
use crate::fse::{self, FseTable};
use crate::huffman::{self, HuffmanTable};
use crate::reader::Reader;

/// AVX2 AUDIT: how often do the 32-byte decoder copies actually execute?
/// Only >=32-byte ops can benefit from AVX2 -- a 16-byte copy is already one
/// `movups`.
#[cfg(feature = "profile")]
pub static DEC_LIT32: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static DEC_MATCH32: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static DEC_LIT16: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Literal copies served by the 64-byte tier (BRICK 80's missing third rung).
#[cfg(feature = "profile")]
pub static DEC_LIT64: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
#[cfg(feature = "profile")]
pub static DEC_MATCH16: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);

/// T4 band census for `copy_match`: which route does each match actually take?
/// 0 = offset 1 splat, 1 = 32-byte tier, 2 = 16-byte tier,
/// 3 = extend_from_within (offset >= len, runtime-length memcpy CALL),
/// 4 = overlapping loop (offset < len).
#[cfg(feature = "profile")]
pub static DEC_BAND: [core::sync::atomic::AtomicU64; 8] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Bytes moved by each route, so a rare-but-long band cannot hide.
#[cfg(feature = "profile")]
pub static DEC_BAND_B: [core::sync::atomic::AtomicU64; 8] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];

/// Length histogram for the UN-TIERED bands (3 = `extend_from_within`,
/// 4 = overlapping chunked). Buckets: 0=<=16, 1=17-32, 2=33-64, 3=65-128,
/// 4=129-256, 5=257-512, 6=513-1024, 7=>1024. Calls in the low half, BYTES in
/// the high half -- a band's mean hides its distribution, and the tier width has
/// to be chosen from the distribution.
#[cfg(feature = "profile")]
pub static DEC_UNTIERED: [core::sync::atomic::AtomicU64; 16] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 16];

#[cfg(feature = "profile")]
pub fn take_dec_untiered() -> [u64; 16] {
    use core::sync::atomic::Ordering::Relaxed;
    let mut a = [0u64; 16];
    for i in 0..16 {
        a[i] = DEC_UNTIERED[i].swap(0, Relaxed);
    }
    a
}

#[cfg(feature = "profile")]
#[inline(always)]
fn note_untiered(len: usize) {
    use core::sync::atomic::Ordering::Relaxed;
    let b = match len {
        0..=16 => 0usize,
        17..=32 => 1,
        33..=64 => 2,
        65..=128 => 3,
        129..=256 => 4,
        257..=512 => 5,
        513..=1024 => 6,
        _ => 7,
    };
    DEC_UNTIERED[b].fetch_add(1, Relaxed);
    DEC_UNTIERED[8 + b].fetch_add(len as u64, Relaxed);
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn note_untiered(_len: usize) {}

#[cfg(feature = "profile")]
pub fn take_dec_bands() -> ([u64; 8], [u64; 8]) {
    use core::sync::atomic::Ordering::Relaxed;
    let mut a = [0u64; 8];
    let mut b = [0u64; 8];
    for i in 0..8 {
        a[i] = DEC_BAND[i].swap(0, Relaxed);
        b[i] = DEC_BAND_B[i].swap(0, Relaxed);
    }
    (a, b)
}

#[cfg(feature = "profile")]
#[inline(always)]
fn note_band(i: usize, len: usize) {
    use core::sync::atomic::Ordering::Relaxed;
    DEC_BAND[i].fetch_add(1, Relaxed);
    DEC_BAND_B[i].fetch_add(len as u64, Relaxed);
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn note_band(_i: usize, _len: usize) {}

/// Read and clear `(lit32, match32, lit16, match16)`.
#[cfg(feature = "profile")]
pub fn take_dec_copies() -> (u64, u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        DEC_LIT32.swap(0, Relaxed),
        DEC_MATCH32.swap(0, Relaxed),
        DEC_LIT16.swap(0, Relaxed),
        DEC_MATCH16.swap(0, Relaxed),
    )
}

/// Read and clear the 64-byte literal tier.
#[cfg(feature = "profile")]
pub fn take_dec_lit64() -> u64 {
    DEC_LIT64.swap(0, core::sync::atomic::Ordering::Relaxed)
}

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

pub(crate) struct BlockState {
    /// W30: the literal buffer, recycled across blocks.
    pub lit_buf: Vec<u8>,
    pub huff: Option<HuffmanTable>,
    pub ll: Option<FseTable>,
    pub of: Option<FseTable>,
    pub ml: Option<FseTable>,
    pub reps: [u32; 3],
}

impl BlockState {
    pub(crate) fn new() -> Self {
        Self {
            lit_buf: Vec::new(),
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
            lit_buf: Vec::new(),
            huff: Some(e.huff_d.clone()),
            ll: Some(e.ll_d.clone()),
            of: Some(e.of_d.clone()),
            ml: Some(e.ml_d.clone()),
            reps: e.reps,
        }
    }
}

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
    // Wholesale BMI2 twin -- the decode-side analog of encode_block: the
    // block driver carried 100 variable shifts of its own (section headers,
    // direct-weights table reads) outside every finer-grained twin.
    // SIMD-2: AVX2 arm FIRST, kept for ISA CONTINUITY across the decode path.
    //
    // The bmi2-only twin below is `enable = "bmi2,lzcnt"`, which does NOT imply
    // avx2, so the literal-section work inlined into it was emitted as **57
    // LEGACY SSE instructions** (27 `movdqa`, 9 `movaps`, 7 `movdqu`, plus a
    // `pshufd`/`punpcklbw` group). This arm converts all 57 to VEX encoding and
    // emits **71 ymm** ops -- deterministic, verified from the emitted asm, and
    // byte-identical by construction (same `#[inline(always)]` body).
    //
    // HONEST LEDGER: on THIS box it measured +0.3% DecLits / +0.5% decode over
    // the bmi2-only arm -- i.e. no speed win, inside noise (`simd2ab.rs`,
    // 14-corpus in-process ABBA x9). It is kept on the DETERMINISTIC ground:
    // the whole decode path is then uniformly VEX-encoded, with no legacy-SSE
    // island sitting beside the AVX2 sequence twin. `BLOCK_AVX2_ARM` keeps it
    // adjudicable on other microarchitectures without a rebuild.
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if block_avx2_on() && crate::simd::has_avx2() && crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            decode_compressed_block_avx2(
                payload,
                out,
                window_size,
                block_max,
                state,
                dict,
                frame_start,
                frame_skipped,
            )
        };
    }
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if crate::simd::has_bmi2() {
        // SAFETY: runtime CPUID guard; identical body.
        #[allow(unsafe_code)]
        return unsafe {
            decode_compressed_block_bmi2(
                payload,
                out,
                window_size,
                block_max,
                state,
                dict,
                frame_start,
                frame_skipped,
            )
        };
    }
    decode_compressed_block_inner(
        payload,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn decode_compressed_block_bmi2(
    payload: &[u8],
    out: &mut Vec<u8>,
    window_size: u64,
    block_max: u32,
    state: &mut BlockState,
    dict: &[u8],
    frame_start: usize,
    frame_skipped: usize,
) -> Result<(), Error> {
    decode_compressed_block_inner(
        payload,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

/// SIMD-2: the AVX2 + BMI2 block driver. Byte-identical by construction -- it
/// calls the same `#[inline(always)]` body as the other two arms.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
#[target_feature(enable = "avx2,bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn decode_compressed_block_avx2(
    payload: &[u8],
    out: &mut Vec<u8>,
    window_size: u64,
    block_max: u32,
    state: &mut BlockState,
    dict: &[u8],
    frame_start: usize,
    frame_skipped: usize,
) -> Result<(), Error> {
    decode_compressed_block_inner(
        payload,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

#[inline(always)]
fn decode_compressed_block_inner(
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
        let recycle = core::mem::take(&mut state.lit_buf);
        decode_literals(recycle, &mut r, state)?
    };
    // Bit accountant, decode side. Running this over C's OWN frame gives C's
    // literals/sequences split in the same units as the encoder's counters,
    // which is the only way to attribute our size gap to a section.
    crate::prof::note_emit_lit((before - r.remaining()) as u64);
    crate::prof::note_emit_seq(r.remaining() as u64);
    let seq_bytes = r.take(r.remaining())?;
    let _s = crate::prof::scope(crate::prof::Stage::DecodeSeq);
    let r = decode_sequences(
        seq_bytes,
        &literals,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    );
    // W30: hand the literal buffer back for the next block. Both borrows above
    // have ended, so this is a move.
    state.lit_buf = literals;
    r
}

// The literal chain is inline(always) into the twinned block driver -- outlined, it ran baseline (transitive trap trace).
#[inline(always)]
pub(crate) fn decode_literals(
    recycle: Vec<u8>,
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
        // W37 -- the RAW and RLE literal arms recycle too.
        //
        // W30 wired the recycled buffer only into the Huffman arm; these two
        // still did `to_vec()` / `vec![b; regen]`, a fresh allocation per block
        // on every raw or RLE literal section. Both fully overwrite the buffer,
        // so reusing it is free.
        0 => {
            let src = r.take(regen as usize)?;
            let mut out = recycle;
            out.clear();
            out.extend_from_slice(src);
            Ok(out)
        }
        1 => {
            let b = r.u8()?;
            let mut out = recycle;
            out.clear();
            out.resize(regen as usize, b);
            Ok(out)
        }
        2 => {
            let section = r.take(csize as usize)?;
            // W36: donate the previous table's X1/X2 buffers (up to 4 KiB and
            // 8 KiB) to the new build instead of dropping them and allocating
            // fresh. It is replaced on the next line anyway.
            let huff_recycle = state.huff.take();
            let (table, tree_size) = huffman::read_table(huff_recycle, section)?;
            // BRICK 63: MOVE the freshly-read table into `state`, then borrow it
            // back to decode with. It was being CLONED into `state` and the
            // original used for the decode -- a full decode-table allocation and
            // copy per block that nothing ever read. `DecodeLiterals` is 69.6% of
            // mr's decode, and mr takes this arm on nearly every block.
            state.huff = Some(table);
            let table = state.huff.as_ref().ok_or(Error::Corruption)?;
            decode_huff_streams(recycle, table, &section[tree_size..], regen, n_streams)
        }
        3 => {
            let table = state.huff.as_ref().ok_or(Error::Corruption)?;
            let section = r.take(csize as usize)?;
            decode_huff_streams(recycle, table, section, regen, n_streams)
        }
        _ => Err(Error::Corruption),
    }
}

#[inline(always)]
fn decode_huff_streams(
    recycle: Vec<u8>,
    table: &HuffmanTable,
    src: &[u8],
    regen: u32,
    n_streams: u32,
) -> Result<Vec<u8>, Error> {
    // W30 -- reuse the previous block's literal buffer.
    //
    // This was `vec![0u8; regen]`, a fresh allocation of up to 128 KiB PER
    // BLOCK -- the >=4096 size class was the largest source of the decode's
    // ~147 MB of allocation traffic. The buffer is fully overwritten before
    // use, so recycling is free: `resize` keeps the allocation once capacity
    // suffices, which it does after the first block.
    let mut out = recycle;
    out.clear();
    out.resize(regen as usize, 0);
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

/// WIN 3 -- LL/ML baseline and extra-bits packed into ONE u32 table each.
///
/// The sequence loop read FOUR separate arrays per sequence (`ML_BITS`,
/// `LL_BITS`, `LL_BASE`, `ML_BASE`) = four loads. The values fit together with
/// room to spare: baseline needs 17 bits (LL max 65,536; ML max 65,539) and the
/// extra-bit count needs 5 (max 16), so `base | (bits << 24)` is lossless in a
/// u32 and halves the loads to two. Same AoS-packing that collapsed the
/// `FseEntry` field split (WIN 2).
///
/// Built by `const fn` from the RFC tables above, so the two forms cannot drift
/// -- and `packed_tables_match_rfc` asserts equality element-by-element.
const fn pack_ll() -> [u32; 36] {
    let mut o = [0u32; 36];
    let mut i = 0;
    while i < 36 {
        o[i] = LL_BASE[i] | ((LL_BITS[i] as u32) << 24);
        i += 1;
    }
    o
}
const fn pack_ml() -> [u32; 53] {
    let mut o = [0u32; 53];
    let mut i = 0;
    while i < 53 {
        o[i] = ML_BASE[i] | ((ML_BITS[i] as u32) << 24);
        i += 1;
    }
    o
}
pub(crate) const LL_PACK: [u32; 36] = pack_ll();
pub(crate) const ML_PACK: [u32; 53] = pack_ml();

/// Baseline half of a packed LL/ML entry.
#[inline(always)]
const fn pk_base(w: u32) -> u32 {
    w & 0x00FF_FFFF
}
/// Extra-bits half of a packed LL/ML entry.
#[inline(always)]
const fn pk_bits(w: u32) -> u8 {
    (w >> 24) as u8
}

/// BRICK 64: `SEQCHECK` is a const generic, not a runtime read.
///
/// The per-sequence guard called `seqcheck_hoisted()` -- an ATOMIC load plus a
/// match -- on EVERY sequence (1.8M times on webster). LLVM will not hoist an
/// atomic out of a loop, so the shipping build paid it per sequence to ask a
/// question whose answer is fixed for the whole process. As a const it vanishes
/// from the loop entirely.
/// T4 -- AVX2 by DUPLICATING THE WHOLE LOOP, which is the only shape that can
/// work on a portable build.
///
/// A `#[target_feature(enable = "avx2")]` function cannot be inlined into a
/// caller that lacks the feature. Putting the attribute on the 32-byte copy
/// alone therefore turned a 4-instruction inline SSE move into
/// `call` + 2 `vmovups` + `vzeroupper` + `ret` -- measured, and a loss. The fix
/// is libzstd's: compile the ENTIRE sequence loop twice and dispatch once per
/// block, so every copy inside it inlines as AVX2.
///
/// `decode_sequences_inner` is `#[inline(always)]` so its body is compiled into
/// both wrappers -- once at baseline, once with AVX2 enabled.
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
    #[cfg(all(target_arch = "x86_64", feature = "std"))]
    if seqloop_avx2_on() && crate::simd::has_avx2() && crate::simd::has_bmi2() {
        // SAFETY: guarded by a runtime AVX2 check; the body is identical.
        #[allow(unsafe_code)]
        return unsafe {
            decode_sequences_avx2(
                src,
                literals,
                out,
                window_size,
                block_max,
                state,
                dict,
                frame_start,
                frame_skipped,
            )
        };
    }
    decode_sequences_inner(
        src,
        literals,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

/// The AVX2-compiled twin. Everything `decode_sequences_inner` does -- including
/// the 16- and 32-byte fixed-width copies -- is emitted with AVX2 available.
#[cfg(all(target_arch = "x86_64", feature = "std"))]
// BMI2 + LZCNT added to the twin: `enable = "avx2"` does NOT imply them,
// so the bitstream's variable shifts still compiled to shl/shr-through-CL
// chains here. With bmi2 LLVM emits single-uop, flag-free shrx/shlx/bzhi --
// exactly the instruction class the sequence decode loop lives on (DecSeq
// is the m7 decode leader in 16/18 corpora). Runtime guard extended to
// has_bmi2; the body is unchanged, so this is byte-identical by construction.
#[target_feature(enable = "avx2,bmi2,lzcnt")]
#[allow(unsafe_code)]
unsafe fn decode_sequences_avx2(
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
    decode_sequences_inner(
        src,
        literals,
        out,
        window_size,
        block_max,
        state,
        dict,
        frame_start,
        frame_skipped,
    )
}

/// T4 arm: compile-twice + dispatch for the sequence loop. **DEFAULT ON.**
///
/// Copy instructions per 12-corpus board pass, priced from the emitted asm
/// (SSE 16B = 2, SSE 32B = 4, AVX2 32B = 2):
///
///   16-first, SSE  (was shipping)   13,064,232
///   32-first, SSE  (original)       23,970,116
///   **16-first + this twin          12,054,710**  <- lowest of every arrangement
///
/// It is byte-identical on 12/12 corpora, the corruption sweep is unchanged to
/// the frame with the twin ACTIVE, and a null arm at the same protocol shows the
/// clock cannot separate it (real -5.29%..+6.62% against a null -4.63%..+5.09%).
///
/// It was briefly defaulted OFF on the grounds that the clock could not see it.
/// That was the standard applied backwards: the tier reorder shipped ON with
/// exactly the same evidence. Strictly-less-work plus byte-identity is the bar,
/// and this clears it -- 1,009,522 fewer instructions per board pass.
///
/// The duplicated body costs no I-cache: a given CPU only ever executes one of
/// the two twins.
static SEQLOOP_AVX2_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the AVX2 sequence-loop twin.
pub fn set_seqloop_avx2_arm(on: bool) {
    SEQLOOP_AVX2_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

#[inline(always)]
fn seqloop_avx2_on() -> bool {
    !matches!(
        SEQLOOP_AVX2_ARM.load(core::sync::atomic::Ordering::Relaxed),
        1
    )
}

#[inline(always)]
fn decode_sequences_inner(
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
    // DecSeq anatomy: four PER-BLOCK guards partition this function. Never per
    // sequence -- see `Stage::DecSeqHeader`.
    let g_hdr = crate::prof::scope(crate::prof::Stage::DecSeqHeader);
    let mut pos = 0usize;
    let byte0 = src[0];
    pos += 1;
    let nseq = if byte0 == 0 {
        // Literals-only block: this is tail work, not header work.
        drop(g_hdr);
        let _g = crate::prof::scope(crate::prof::Stage::DecSeqTail);
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

    drop(g_hdr);
    let g_tab = crate::prof::scope(crate::prof::Stage::DecSeqTables);
    let (ll, n) = seq_table(
        &src[pos..],
        ll_mode,
        35,
        9,
        state.ll.take(),
        fse::default_ll,
    )?;
    pos += n;
    let (of, n) = seq_table(
        &src[pos..],
        of_mode,
        31,
        8,
        state.of.take(),
        fse::default_of,
    )?;
    pos += n;
    let (ml, n) = seq_table(
        &src[pos..],
        ml_mode,
        52,
        9,
        state.ml.take(),
        fse::default_ml,
    )?;
    pos += n;

    let bitstream = &src[pos..];
    let mut br = BitRev::new(bitstream)?;
    let mut ll_s = ll.init_state(&mut br);
    let mut of_s = of.init_state(&mut br);
    let mut ml_s = ml.init_state(&mut br);
    drop(g_tab);

    let mut lit_pos = 0usize;
    // Built ONCE per block; the loop then passes a single reference.
    let mctx = MatchCtx {
        dict,
        frame_start,
        frame_skipped,
        window_size,
        block_max,
        wide: matchcopy_on(),
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
    // W7: is this the common shape? Decided ONCE per block.
    let nodict = dict.is_empty() && frame_start == 0 && frame_skipped == 0;
    let win_sz = window_size;
    let blk_max = block_max;
    let wide_arm = matchcopy_on();
    // WIN 1: resolve each table's (ptr, mask) ONCE per block instead of 3x per
    // sequence. See `FseTable::view`.
    let llv = ll.view();
    let ofv = of.view();
    let mlv = ml.view();
    // Hoisted for the same reason `litcopy_arm` is: an atomic load will not
    // leave a loop. Profile builds only.
    #[cfg(feature = "dupladder")]
    let dup = DUP_ARM.load(core::sync::atomic::Ordering::Relaxed);
    #[cfg(feature = "dupladder")]
    let dup_k = DUP_K.load(core::sync::atomic::Ordering::Relaxed);
    let g_loop = crate::prof::scope(crate::prof::Stage::DecSeqLoop);
    // W22 -- count DOWN, so the loop test is the decrement's own flags.
    //
    // `for i in 0..nseq` keeps BOTH `i` and `nseq` live and costs a compare per
    // iteration, plus a second compare for the `i + 1 != nseq` last-iteration
    // test. A countdown collapses both: `rem` decrements to zero, the loop test
    // reads the flags the decrement already set, and "is this the last one" is
    // the same `rem == 0`. One fewer live value in a loop the spill map shows
    // reloading 203 times.
    let mut rem = nseq;
    while rem != 0 {
        rem -= 1;
        let _ = br.reload();
        // WIN 2: one 4-byte load per table instead of three field loads.
        let ll_w = llv.entry_u32(ll_s);
        let of_w = ofv.entry_u32(of_s);
        let ml_w = mlv.entry_u32(ml_s);
        let ll_code = crate::fse::fse_symbol(ll_w) as usize;
        // W5: `of_code` is only ever consumed as a u32 (read_bits, the shift and
        // the range test), so widen once from u8 instead of u8 -> usize -> u32.
        let of_code = u32::from(crate::fse::fse_symbol(of_w));
        let ml_code = crate::fse::fse_symbol(ml_w) as usize;
        // No per-sequence range test: ALL FOUR table modes now bound their
        // symbols at build time (see `seq_table`), so `ll_code <= 35`,
        // `ml_code <= 52` and `of_code <= 31` hold by construction. This ran
        // ~1M times per file to re-prove a per-block invariant.
        debug_assert!(ll_code <= 35 && ml_code <= 52 && of_code <= 31);
        if !seqcheck && (ll_code > 35 || ml_code > 52 || of_code > 31) {
            return Err(Error::Corruption);
        }
        let offset_add = br.read_bits(of_code);
        // T4: the same build-time bound the `debug_assert` above states, used.
        // `seq_table` bounds the symbol in ALL FOUR modes -- predefined by its
        // norm length, compressed by `read_ncount`'s `charnum > max_symbol`
        // reject, repeat by inheriting a validated table, and RLE by an explicit
        // `sym > max_sym` test added for exactly this reason. So `ll_code <= 35`
        // and `ml_code <= 52` hold even for hostile input, and the tables are
        // `[_; 36]` and `[_; 53]` -- the bound and the length match exactly.
        //
        // This is per SEQUENCE, and LLVM cannot derive the bound from a value
        // that came out of an FSE table.
        // WIN 3: two packed loads instead of four.
        #[allow(unsafe_code)]
        let (ll_w2, ml_w2) = unsafe {
            debug_assert!(ll_code < LL_PACK.len() && ml_code < ML_PACK.len());
            (
                *LL_PACK.get_unchecked(ll_code),
                *ML_PACK.get_unchecked(ml_code),
            )
        };
        let (ml_bits, ll_bits) = (pk_bits(ml_w2), pk_bits(ll_w2));
        let (ll_base, ml_base) = (pk_base(ll_w2), pk_base(ml_w2));
        let ml_add = br.read_bits(u32::from(ml_bits));
        let ll_add = br.read_bits(u32::from(ll_bits));
        let litlen = ll_base + ll_add;
        let matchlen = ml_base + ml_add;
        let offset_value = if of_code == 0 {
            1
        } else {
            (1u32 << of_code) + offset_add
        };

        // ---- DecSeq loop anatomy: duplicate ONE op, then undo it ----
        #[cfg(feature = "dupladder")]
        {
            use core::hint::black_box;
            for _ in 0..dup_k {
                match dup {
                    1 => {
                        black_box(llv.entry_u32(ll_s));
                        black_box(ofv.entry_u32(of_s));
                        black_box(mlv.entry_u32(ml_s));
                    }
                    2 => {
                        let sv = br.dup_save();
                        black_box(br.read_bits(of_code as u32));
                        black_box(br.read_bits(u32::from(ml_bits)));
                        black_box(br.read_bits(u32::from(ll_bits)));
                        br.dup_restore(sv);
                    }
                    3 => {
                        let sv = br.dup_save();
                        black_box(FseTable::advance_w(ll_w, &mut br));
                        black_box(FseTable::advance_w(ml_w, &mut br));
                        black_box(FseTable::advance_w(of_w, &mut br));
                        br.dup_restore(sv);
                    }
                    4 => {
                        let sv = br.dup_save();
                        black_box(br.reload());
                        black_box(br.reload());
                        br.dup_restore(sv);
                    }
                    5 => {
                        let (lp, len) = (lit_pos, out.len());
                        let _ = copy_literals(literals, &mut lit_pos, litlen, out, litcopy_arm);
                        lit_pos = lp;
                        out.truncate(len);
                    }
                    _ => {}
                }
            }
        }
        copy_literals(literals, &mut lit_pos, litlen, out, litcopy_arm)?;
        #[cfg(feature = "dupladder")]
        if dup == 6 {
            for _ in 0..dup_k {
                let sv = state.reps;
                let _ =
                    core::hint::black_box(resolve_offset(offset_value, litlen, &mut state.reps));
                state.reps = sv;
            }
        }
        let offset = resolve_offset(offset_value, litlen, &mut state.reps)?;
        #[cfg(feature = "dupladder")]
        if dup == 7 {
            for _ in 0..dup_k {
                let len = out.len();
                let _ = copy_match(out, &mctx, offset, matchlen);
                out.truncate(len);
            }
        }
        if nodict {
            copy_match_nodict(out, offset, matchlen, win_sz, blk_max, wide_arm)?;
        } else {
            copy_match(out, &mctx, offset, matchlen)?;
        }

        if rem != 0 {
            let _ = br.reload();
            ll_s = FseTable::advance_w(ll_w, &mut br);
            ml_s = FseTable::advance_w(ml_w, &mut br);
            of_s = FseTable::advance_w(of_w, &mut br);
        }
    }
    drop(g_loop);
    {
        let _g = crate::prof::scope(crate::prof::Stage::DecSeqTail);
        out.extend_from_slice(&literals[lit_pos..]);
    }
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
        state.ll.clone(),
        fse::default_ll,
    )?;
    pos += n;
    let (of, n) = seq_table(
        &src[pos..],
        of_mode,
        31,
        8,
        state.of.clone(),
        fse::default_of,
    )?;
    pos += n;
    let (ml, n) = seq_table(
        &src[pos..],
        ml_mode,
        52,
        9,
        state.ml.clone(),
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
        let llc = ll_e.symbol;
        let ofc = of_e.symbol;
        let mlc = ml_e.symbol;
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

#[inline(always)]
// W18 -- the per-block table SETUP is cold relative to the sequence loop.
//
// `seq_table` -> `read_ncount` -> `from_norm` are all `inline(always)`, so the
// entire FSE table build was inlined into BOTH `decode_sequences` twins beside
// the hot loop -- 3,385 instructions and 436 spill slots' worth. It runs three
// times per BLOCK against a loop that runs 53,509 times per MiB. Outlining it
// gives the loop back its register budget and stops the build being duplicated
// into both ISA arms.
//
// Kept where its sibling outlinings were REVERTED: outlining anything reachable
// PER SEQUENCE (the cold copy tiers, the dict path) measured 2.5% SLOWER even
// though it cut more instructions. Per-block is the safe side of that line.
//
// FINDING (v0.1.0 release audit): the `#[inline(never)]` this comment argues
// for was written BELOW the `#[inline(always)]` above, so rustc took the first
// and DISCARDED it -- silently, until `unused_attributes` was promoted. W18 has
// therefore never been in effect, and whatever the brick measured, it did not
// measure this outlining. The dead attribute is removed rather than the live
// one so the shipped binary is the one that was measured. Re-run the W18 A/B
// before promoting `seq_table` to `inline(never)`.
fn seq_table(
    src: &[u8],
    mode: u8,
    max_sym: usize,
    max_log: u8,
    // W25 -- taken by VALUE so Repeat mode need not clone.
    //
    // Repeat is **48.3% of all table selections at L3** (1,399 of 2,895 over the
    // 14-corpus board), and it used `prev.cloned()`: a heap allocation plus a
    // memcpy of up to 512 x 4 bytes, to reproduce a table the caller already
    // owns and writes straight back into `state`. Moving it through is free.
    //
    // HONEST LEDGER: this is a WORK reduction, not an instruction or speed one.
    // Static instructions rose 42 (inlining shifted) and whole-decode time was
    // dead neutral (2.084 -> 2.084 ms/MiB, ABBA x3) -- at ~12.5 clones per MiB
    // the allocator traffic is too rare to move the clock. Kept because it is
    // provably less work and byte-identical, not because it measured faster.
    prev: Option<FseTable>,
    predefined: fn() -> Result<FseTable, Error>,
) -> Result<(FseTable, usize), Error> {
    match mode {
        0 => {
            // N21 probe: how often is an RFC-CONSTANT table rebuilt from scratch?
            #[cfg(feature = "profile")]
            N21_PREDEF.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            Ok((predefined()?, 0))
        }
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
        // W26: recycle the previous table's allocation for the new build.
        2 => fse::read_ncount_into(prev, src, max_sym, max_log),
        3 => {
            // W25: hand the caller's own table back -- no clone.
            let t = prev.ok_or(Error::Corruption)?;
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
            let on = crate::env_knob("RZSTD_SEQCHECK_HOIST")
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
            #[cfg(feature = "profile")]
            DEC_LIT16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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
    // EVERYTHING BELOW TIER 1 IS OUTLINED AND COLD -- the same shape the
    // ENCODER already uses (`push_literals_tiers`), and for the reason its
    // comment gives: "Inlining them here pushed `push_literals` past LLVM's
    // inlining threshold and it stopped being inlined AT ALL... That is the
    // linkage trap."
    //
    // `copy_literals` is fully inlined into all THREE
    // `decode_compressed_block` twins, and tiers 2/3 plus the fallback were
    // stamped into every copy. Measured share (`declit`, 8 corpora):
    // tier1 99.7%, tier2 0.12%, tier3 0.09%, fallback ~0.1% -- so <0.4% of
    // copies were paying for the other 99.6%'s code size.
    copy_literals_cold(literals, lit_pos, end, n, out, arm, len)
}

/// Tiers 2 and 3 plus the `extend_from_slice` fallback: under 0.4% of literal
/// copies. Outlined and cold so tier 1 keeps its inlining.
#[allow(unsafe_code)]
#[inline(never)]
#[cold]
fn copy_literals_cold(
    literals: &[u8],
    lit_pos: &mut usize,
    end: usize,
    n: usize,
    out: &mut alloc::vec::Vec<u8>,
    arm: bool,
    len: usize,
) -> Result<(), Error> {
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
            #[cfg(feature = "profile")]
            DEC_LIT32.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
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
    // BRICK 80's THIRD RUNG, which it never got. The encoder's `push_literals`
    // has tiered at 16/32/**64** since GATE 13, and this file's own MATCH copy
    // (`copy_from_decoded`) tiers at 16/32/**64** too -- the literal copy was
    // the one path stopping at two rungs. Same sibling-path-parity shape as
    // `find_lazy_impl` bypassing `push_literals` on the encode side.
    //
    // BRICK 80 measured the tail it left behind: of 19.6M literal copies,
    // >32B was 728,346 (3.7%), every one of them an `extend_from_slice` --
    // a `memcpy` CALL. This rung takes the 33..=64 slice of that.
    if arm && n <= 64 && *lit_pos + 64 <= literals.len() && out.capacity() - len >= 64 {
        // SAFETY: identical invariant to the two rungs above, at 64 bytes --
        // 64 readable source bytes, 64 writable destination bytes inside the
        // allocation, distinct buffers, and only `n <= 64` published.
        unsafe {
            #[cfg(feature = "profile")]
            DEC_LIT64.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            core::ptr::copy_nonoverlapping(
                literals.as_ptr().add(*lit_pos),
                out.as_mut_ptr().add(len),
                64,
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

/// DecSeq LOOP anatomy arm (profile builds only). Selects ONE per-sequence op to
/// execute a SECOND time and then undo, so the arm's delta over baseline prices
/// that op. Every arm stays byte-identical -- the duplicate work is reverted --
/// so `dsloop.rs` can assert round-trip on every arm it times.
///
/// 0 = baseline, 1 = FseTable::entry x3, 2 = read_bits x3, 3 = advance x3,
/// 4 = reload x2, 5 = copy_literals, 6 = resolve_offset, 7 = copy_match.
#[cfg(feature = "dupladder")]
pub static DUP_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// How many extra executions the selected arm performs per sequence. Cheap ops
/// (the entropy primitives) execute in spare superscalar slots and measure ~0 at
/// K=1; raising K lifts them above the noise floor so the per-execution cost is
/// a division rather than a guess.
#[cfg(feature = "dupladder")]
pub static DUP_K: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(1);

/// Select the duplication arm. `0` restores baseline.
#[cfg(feature = "dupladder")]
pub fn set_dup_arm(a: u8) {
    DUP_ARM.store(a, core::sync::atomic::Ordering::Relaxed);
}

/// Set the duplication multiplier.
#[cfg(feature = "dupladder")]
pub fn set_dup_k(k: u8) {
    DUP_K.store(k.max(1), core::sync::atomic::Ordering::Relaxed);
}

/// SIMD-2 arm: the AVX2 block driver. Kept so the in-process ABBA harness can
/// re-adjudicate it on other microarchitectures. 1 = off, 2 = on (default).
static BLOCK_AVX2_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);

/// Bench hook: `false` routes the block driver to the bmi2-only twin.
pub fn set_block_avx2_arm(on: bool) {
    BLOCK_AVX2_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

#[inline(always)]
fn block_avx2_on() -> bool {
    BLOCK_AVX2_ARM.load(core::sync::atomic::Ordering::Relaxed) != 1
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

/// T4/brick-79, third and last instance. `lut_on()` was read INSIDE `ll_code`
/// and `ml_code`, which the sequence-coder calls once each PER SEQUENCE -- two
/// atomic loads per sequence, ~15M across the corpus. `litcopy_on` and
/// `matchcopy_on` are both resolved once per block and passed down; this one
/// never was. Callers now hoist it the same way.
#[inline(always)]
pub(crate) fn lut_on() -> bool {
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

pub(crate) fn ll_code(len: u32, lut: bool) -> (u8, u32, u8) {
    if lut && (len as usize) < LL_LUT_LEN {
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

pub(crate) fn ml_code(len: u32, lut: bool) -> (u8, u32, u8) {
    if lut && (len as usize) < ML_LUT_LEN {
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
    /// T4/brick-79: `matchcopy_on()` was read THREE TIMES PER CALL inside
    /// `copy_match`, i.e. three atomic loads per SEQUENCE. The literal path
    /// already carries its arm as a parameter (`copy_literals(.., arm)`); the
    /// match path never got the same treatment. Resolved once per block here.
    wide: bool,
}

// T4 -- AVX2 WAS BUILT HERE AND REFUTED. Kept as a note so it is not retried.
//
// `#[target_feature(enable = "avx2")]` on a 32-byte copy compiles to exactly
// what you want:
//
//     vmovups (%rcx), %ymm0 ; vmovups %ymm0, (%rdx) ; vzeroupper ; retq
//
// but a `target_feature` function CANNOT be inlined into a caller that lacks
// the feature, and this crate targets baseline x86-64. So the emitted code was
// a CALL from `copy_from_decoded` -- call + 2 vmovups + vzeroupper + ret --
// replacing 4 inline SSE instructions. A net loss, deterministically, before
// any clock is involved.
//
// The only way AVX2 pays here is the way libzstd does it: duplicate the WHOLE
// sequence-decode loop under `#[target_feature]` and dispatch once per block,
// so the wide copy is inlined inside an AVX2-compiled loop. That is a real
// refactor, and its ceiling is bounded by how much traffic the 32-byte tier
// actually carries -- 8.0% of calls once the tiers are ordered 16-first.
//
// RESOLVED (SIMD-1). `decode_sequences_avx2` is that duplicated loop, and it has
// existed since the twin campaign -- but the copy stayed OUTLINED, so it kept
// being generated at baseline and the twin's 26 ymm moves all belonged to
// `copy_literals`. The missing half was `#[inline(always)]`, below.

// SIMD-1: `#[inline(always)]` so the match copy is compiled INSIDE whichever
// twin calls it. Without it LLVM left `copy_match` (141 instrs) and
// `copy_from_decoded` (230) outlined, and a function carrying no
// `#[target_feature]` is generated at the crate's BASELINE ISA no matter who
// calls it -- so ~42% of the DecSeq loop ran with 0 ymm and two nested calls per
// sequence, while `copy_literals`, which IS inlined, got 256-bit moves. A
// baseline callee's feature set is a SUBSET of the twin's, so inlining is legal
// and LLVM re-generates the body under `avx2,bmi2`.
/// W7 -- the no-dictionary, frame-start-0 specialisation of `copy_match`.
///
/// `MatchCtx` is loop-invariant but STACK-RESIDENT: the spill map shows its
/// fields read 36x and 21x per function with zero writes, because `copy_match`
/// is `inline(always)` and re-reads `&mctx` every sequence. In the common shape
/// -- no dictionary, `frame_start == 0`, `frame_skipped == 0` -- four of the
/// seven fields are constants, and the bounds collapse from seven field reads
/// plus three saturating ops to two compares against two scalars.
///
/// Byte-identical to `copy_match` under those preconditions: with `dict` empty
/// and both frame offsets 0, `virtual_len == out.len()`, `src_pos0 >= 0 ==
/// dict.len()` always holds, so the dict arm is unreachable and `i == src_pos0`.
#[inline(always)]
fn copy_match_nodict(
    out: &mut Vec<u8>,
    offset: u32,
    matchlen: u32,
    window_size: u64,
    block_max: u32,
    wide: bool,
) -> Result<(), Error> {
    let off = offset as usize;
    if off == 0 {
        return Err(Error::Corruption);
    }
    let produced = out.len();
    if off > produced {
        return Err(Error::Corruption);
    }
    if (off as u64) > window_size {
        return Err(Error::Corruption);
    }
    let len = matchlen as usize;
    if len > block_max as usize {
        return Err(Error::Corruption);
    }
    copy_from_decoded(out, produced - off, len, wide)
}

/// N21 probe: rebuilds of the RFC-constant Predefined FSE decode tables.
/// Each is a full `from_norm` -- heap alloc + serial spread + finalize -- for a
/// value fixed by RFC 8878 and identical for the life of the process.
#[cfg(feature = "profile")]
pub static N21_PREDEF: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Read and clear the N21 probe.
#[cfg(feature = "profile")]
pub fn take_n21_predef() -> u64 {
    N21_PREDEF.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// D3 probe: `extend_from_within` calls made by the overlapping (band 4) loop.
/// D3 assumes "a memcpy call per period"; this counts what actually happens.
#[cfg(feature = "profile")]
pub static D3_ITERS: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
/// Read and clear the D3 probe.
#[cfg(feature = "profile")]
pub fn take_d3_iters() -> u64 {
    D3_ITERS.swap(0, core::sync::atomic::Ordering::Relaxed)
}

/// D4 coverage census: `[frame_only, dict_only, dict_CROSSING]` calls.
/// A brick on the crossing path is unverified until index 2 is non-zero.
#[cfg(feature = "profile")]
pub static D4_PATHS: [core::sync::atomic::AtomicU64; 3] = [
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
    core::sync::atomic::AtomicU64::new(0),
];
/// Read and clear the D4 coverage census.
#[cfg(feature = "profile")]
pub fn take_d4_paths() -> [u64; 3] {
    use core::sync::atomic::Ordering;
    [
        D4_PATHS[0].swap(0, Ordering::Relaxed),
        D4_PATHS[1].swap(0, Ordering::Relaxed),
        D4_PATHS[2].swap(0, Ordering::Relaxed),
    ]
}

#[inline(always)]
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
        wide,
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
        #[cfg(feature = "profile")]
        D4_PATHS[0].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        return copy_from_decoded(out, i, len, wide);
    }
    // D4 (inline-execution): SPLIT AT THE BOUNDARY, don't re-test it per byte.
    //
    // This was a byte-at-a-time push that re-evaluated `src_pos < dict.len()`
    // on every iteration -- a branch whose answer changes exactly ONCE in the
    // whole loop, plus a bounds-checked `out.get(i)` per byte. It is not a SIMD
    // problem, it is a decomposition problem: the match is a dictionary run
    // followed by a frame run, so compute where it crosses and do two bulk
    // copies. Vector stores fall out for free because both halves become slice
    // operations (`extend_from_slice` lowers to `memcpy`; `copy_from_decoded`
    // is the existing tiered wildcopy the pure-frame path above already uses).
    //
    // Byte-identical by construction: `src_pos0 < dict.len()` on this path, so
    // the first `from_dict` bytes are exactly `dict[src_pos0..]`, and `src_pos`
    // reaches `dict.len()` precisely when the dictionary is exhausted -- making
    // the first frame byte's `frame_off` exactly 0. The old loop's per-byte
    // `frame_off < frame_skipped` test therefore fails on that first frame byte
    // iff `frame_skipped > 0`, which is the hoisted check below; and for
    // `frame_skipped == 0` the frame source starts at `frame_start`, which is
    // what the pure-frame early return above passes too.
    out.reserve(len);
    let from_dict = core::cmp::min(len, dict.len() - src_pos0);
    out.extend_from_slice(&dict[src_pos0..src_pos0 + from_dict]);
    let rest = len - from_dict;
    #[cfg(feature = "profile")]
    D4_PATHS[usize::from(rest > 0) + 1].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    if rest > 0 {
        if frame_skipped > 0 {
            return Err(Error::Corruption);
        }
        return copy_from_decoded(out, frame_start, rest, wide);
    }
    Ok(())
}

/// Overlapping-safe copy from already-decoded `out[src..]`. C wildcopy equivalent.
///
/// SIMD-1: `#[inline(always)]` for the reason on `copy_match` -- this is where the
/// 16/32/64-byte tiers live, and outlined they were emitted as SSE.
#[inline(always)]
#[allow(unsafe_code)]
fn copy_from_decoded(out: &mut Vec<u8>, src: usize, len: usize, wide: bool) -> Result<(), Error> {
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
        note_band(0, len);
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
    // T4 -- ORDER MATTERS, and it was backwards.
    //
    // The 32-byte tier was tested FIRST, so every short match with a large
    // offset landed in it: a band census over 12 corpora at L3 puts **86.6% of
    // ALL match copies** there with `len <= 16`, at a mean of 7.4 bytes. They
    // moved 32 bytes to publish 7.4.
    //
    // And the 32-byte move is not one instruction. This crate targets baseline
    // x86-64, so there is no AVX2 anywhere in the decode path -- the emitted asm
    // is SSE `movups`/`movdqu` on `%xmm` throughout, and a 32-byte copy is TWO
    // 16-byte load/store pairs (visible as the `movups %xmm6, 16(%rax)` /
    // `movups %xmm6, (%rax)` pair in `decompress_into_history`). So the wider
    // tier costs double the narrow one and the majority of calls never needed
    // it.
    //
    // Testing 16 first is byte-identical -- the same `len` bytes are published
    // either way, and `offset >= 32` implies `offset >= 16` while the capacity
    // requirement only relaxes -- so every one of those calls simply moves to
    // the cheaper path.
    if wide && len <= 16 && offset >= 16 && out.capacity() - out.len() >= 16 {
        let dst_at = out.len();
        // SAFETY: `offset >= 16` means `src + 16 <= out.len() == dst_at`, so the
        // source is initialised and disjoint from the destination.
        // `capacity - len >= 16` gives 16 writable bytes. Only `len <= 16`
        // bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            #[cfg(feature = "profile")]
            DEC_MATCH16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            note_band(2, len);
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 16);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    // TIERS 2/3 AND THE FALLBACK ARE OUTLINED AND COLD -- the same split the
    // ENCODER uses (`push_literals_tiers`) and that `copy_literals` just
    // received. `copy_from_decoded` is fully inlined into all three
    // `decode_compressed_block` twins, so every rung below the first was
    // stamped into each copy.
    //
    // The census in this function already says tier 1 dominates: "a band
    // census over 12 corpora at L3 puts **86.6% of ALL match copies** there
    // with `len <= 16`, at a mean of 7.4 bytes." The other 13% was paying for
    // the 87%'s code size.
    copy_from_decoded_cold(out, src, len, wide, offset)
}

/// Match-copy tiers 2 and 3 plus the `extend_from_within` fallback: the ~13%
/// of match copies that tier 1 declines. Outlined and cold so tier 1 keeps
/// its inlining.
#[allow(unsafe_code)]
#[inline(never)]
#[cold]
fn copy_from_decoded_cold(
    out: &mut Vec<u8>,
    src: usize,
    len: usize,
    wide: bool,
    offset: usize,
) -> Result<(), Error> {
    if wide && len <= 32 && offset >= 32 && out.capacity() - out.len() >= 32 {
        let dst_at = out.len();
        // SAFETY: `offset >= 32` means `src + 32 <= out.len() == dst_at`, so
        // the source is fully initialised and disjoint from the destination.
        // `capacity - len >= 32` gives 32 writable bytes inside the
        // allocation. Only `len <= 32` bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            #[cfg(feature = "profile")]
            DEC_MATCH32.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            // AVX2 AUDIT (4.56): routing this through a runtime-dispatched
            // `#[target_feature(enable = "avx2")]` copy makes it SLOWER. A
            // target_feature function cannot be inlined into a baseline caller,
            // so the 32-byte copy became `2 movs + callq` (3) plus a 4-
            // instruction callee = 7, against 4 for the two inline SSE `movups`
            // pairs. Measured, not assumed. Left as the baseline copy.
            // Sub-census: how much of the 32-byte tier would a 16-byte copy
            // have served? The tier is tested BEFORE the 16-byte one, so any
            // short match with a large offset lands here regardless.
            note_band(if len <= 16 { 5 } else { 1 }, len);
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
    // 64-BYTE TIER. The census that motivated it: `extend_from_within` was the
    // only UN-TIERED band and it carried ~34% of all match bytes. Its length
    // distribution is not diffuse -- **65.9% of its calls and 46.9% of its bytes
    // are 33-64 bytes** (`dsuntier.rs`), i.e. exactly one 2x ymm move pair. The
    // mean of that band is ~67 and would have chosen the wrong width; the
    // histogram chose it.
    //
    // `offset >= 64` does the same double duty as the narrower tiers: it
    // guarantees 64 readable source bytes (`src + 64 <= out.len()`) AND that the
    // source range ends at or before the destination start, so the two 64-byte
    // regions cannot overlap.
    if wide && len <= 64 && offset >= 64 && out.capacity() - out.len() >= 64 {
        let dst_at = out.len();
        // SAFETY: `offset >= 64` means `src + 64 <= out.len() == dst_at`, so the
        // source is fully initialised and disjoint from the destination.
        // `capacity - len >= 64` gives 64 writable bytes inside the allocation.
        // Only `len <= 64` bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            note_band(6, len);
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 64);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    if offset >= len {
        note_band(3, len);
        note_untiered(len);
        out.extend_from_within(src..src + len);
        return Ok(());
    }
    note_band(4, len);
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
        #[cfg(feature = "profile")]
        D3_ITERS.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// WIN 3 gate: the packed tables must equal the RFC tables they replace.
    #[test]
    fn packed_tables_match_rfc() {
        for i in 0..LL_BASE.len() {
            assert_eq!(pk_base(LL_PACK[i]), LL_BASE[i], "LL_BASE[{i}]");
            assert_eq!(pk_bits(LL_PACK[i]), LL_BITS[i], "LL_BITS[{i}]");
        }
        for i in 0..ML_BASE.len() {
            assert_eq!(pk_base(ML_PACK[i]), ML_BASE[i], "ML_BASE[{i}]");
            assert_eq!(pk_bits(ML_PACK[i]), ML_BITS[i], "ML_BITS[{i}]");
        }
    }

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
                    copy_from_decoded(&mut fast, src, len, true).unwrap();
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
                copy_from_decoded(&mut v, before - off, len, true).unwrap();
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
                ll_code(v, true),
                code_from_base(v, &LL_BASE, &LL_BITS),
                "ll_code({v})"
            );
            assert_eq!(
                ml_code(v, true),
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
            assert_eq!(ll_code(v, true), code_from_base(v, &LL_BASE, &LL_BITS));
            assert_eq!(ml_code(v, true), code_from_base(v, &ML_BASE, &ML_BITS));
        }
    }
}
