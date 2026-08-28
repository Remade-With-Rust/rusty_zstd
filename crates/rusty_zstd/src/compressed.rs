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

/// One outlined copy of `FseTable`'s derived `Clone`. See `BlockState::from_dict`.
#[inline(never)]
fn clone_dtable(t: &crate::fse::FseTable) -> crate::fse::FseTable {
    t.clone()
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
        // C8: `FseTable`'s derived `Clone` has no symbol of its own -- it was
        // inlined at each of these three sites, so the dictionary load carried
        // three copies of a Vec-cloning body. Routing them through one
        // `#[inline(never)]` helper leaves the derive inlined ONCE, inside the
        // helper, and makes these three calls. Runs once per dictionary.
        Self {
            lit_buf: Vec::new(),
            huff: Some(e.huff_d.clone()),
            ll: Some(clone_dtable(&e.ll_d)),
            of: Some(clone_dtable(&e.of_d)),
            ml: Some(clone_dtable(&e.ml_d)),
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
    // BOTH ISA ARMS RETIRED. The avx2 arm's premise was explicit: "the
    // literal-section work inlined into it was emitted as 57 LEGACY SSE
    // instructions ... this arm converts all 57 to VEX and emits 71 ymm ops."
    // That work is no longer inlined here -- `decode_literals` became a shared
    // `#[inline(never)]` symbol -- and the emitted asm now reads:
    //
    //   decode_compressed_block_bmi2   147 instrs   0 BMI2 ops   0 ymm  0 VEX
    //   decode_compressed_block_avx2   147 instrs   0 BMI2 ops   0 ymm  2 VEX
    //
    // Neither converts anything. The bmi2 arm's own justification -- "the
    // block driver carried 100 variable shifts of its own" -- is equally gone:
    // there are zero `shrx` in either body. And the avx2 arm carried an HONEST
    // LEDGER admitting it measured +0.3% DecLits / +0.5% decode, i.e. no win,
    // inside noise. It was kept for "ISA CONTINUITY"; with nothing left to be
    // continuous about, that is 294 instructions of duplicate block driver.
    //
    // The finer-grained twins that do the real work are untouched:
    // `decode_sequences` dispatches its own avx2 arm (23 BMI2 ops), and
    // `decode_4x_bmi2` keeps 36 in 702.

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

/// The literals-section header: type, sizes, stream count. Pure byte
/// arithmetic, factored out of `decode_literals` so the three ISA twins of
/// `decode_compressed_block` share one copy. Runs once per block.
#[inline(never)]
fn parse_lit_header(r: &mut Reader<'_>) -> Result<(u8, u32, u32, u32), Error> {
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

    let _ = header_rest;
    Ok((lit_type, regen, csize, n_streams))
}

// HISTORY: this was `#[inline(always)]` with the note "outlined, it ran
// baseline (transitive trap trace)" -- from the era when the huffman kernels
// inherited their ISA from the CALLER's `#[target_feature]` context, so
// outlining the chain silently dropped every stream to baseline. The kernels
// now carry their OWN per-section CPUID dispatch (`decode_stream` and
// `decode_4x` both guard internally), so the trap cannot recur: outlining
// changes where the parse code sits, not which kernel runs. Un-inlining
// removes two stamps of this body (and of `decode_huff_streams` below) from
// the three `decode_compressed_block` twins.
#[inline(never)]
pub(crate) fn decode_literals(
    recycle: Vec<u8>,
    r: &mut Reader<'_>,
    state: &mut BlockState,
) -> Result<Vec<u8>, Error> {
    // THE HEADER PARSE IS ONE NON-ISA CALL. `decode_literals` stays
    // `#[inline(always)]` into the three `decode_compressed_block` twins BY
    // DESIGN (the huffman chain must inherit the twin's ISA -- see the comment
    // above this function), but the sizes/streams parse is pure byte
    // arithmetic and was being stamped into each twin along with it.
    let (lit_type, regen, csize, n_streams) = parse_lit_header(r)?;
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
            // D22: `tree_size` is whatever `read_table` consumed -- opaque to
            // LLVM, so this slice carried a bounds test and a panic pad.
            // `get` states it once and turns a truncated tree description into
            // `Corruption` instead of an unwind. D17's case, not D19's.
            let body = section.get(tree_size..).ok_or(Error::Corruption)?;
            decode_huff_streams(recycle, table, body, regen, n_streams)
        }
        3 => {
            let table = state.huff.as_ref().ok_or(Error::Corruption)?;
            let section = r.take(csize as usize)?;
            decode_huff_streams(recycle, table, section, regen, n_streams)
        }
        _ => Err(Error::Corruption),
    }
}

// Outlined for the same reason as `decode_literals` above: the 4x kernel
// dispatches its own ISA per section, so the stream-split arithmetic gains
// nothing from living inside the twins.
#[inline(never)]
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
    // D13 REFUTED, recorded: replacing these four running-offset slices with a
    // `split_at` chain measured **+45** and grew the function 308 -> 310. It
    // did drive the pads to zero -- and that was not worth it. `split_at`
    // returns a tuple of two borrows per cut, and the extra pointer/length
    // pairs cost more than the four bounds tests they replaced.
    //
    // Pairs with C1/C2, which won: a FIXED-SIZE array turns a dynamic bound
    // static and the checks vanish. `split_at` keeps both halves dynamic and
    // just adds bookkeeping. Removing pads is not the goal; removing
    // instructions is.
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
/// WIN 9 (megafuse round): `1 << of_code` was a variable `shll %cl` -- 3 uops
/// on baseline x86-64 plus two setup moves, per sequence. The offset bases are
/// a 32-entry constant; a load is 1 uop.
const OF_PACK: [u32; 32] = {
    let mut o = [0u32; 32];
    let mut i = 0;
    while i < 32 {
        o[i] = 1u32 << i;
        i += 1;
    }
    o
};
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
    // D15: gate matches payload -- see the twin. It was `has_avx2() &&
    // has_bmi2()` for a body with zero ymm.
    if seqloop_avx2_on() && crate::simd::has_bmi2() {
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
// D15: THE `avx2` FEATURE IS DROPPED FROM THIS TWIN, and the runtime gate with
// it. Measured on the emitted asm: this body contains **0 `%ymm`** and 23 BMI2
// ops. Its entire payload is BMI2 -- `avx2` bought 8 VEX encodings and cost the
// fast path every CPU that ships BMI2 with AVX2 fused off (the Skylake
// Pentium/Celeron parts `decode_4x`'s own comment names). Those machines were
// falling back to a baseline with no `shrx` at all.
//
// The name stays for API/knob compatibility (`set_seqloop_avx2_arm` is
// `pub use`d); what changes is that the gate now matches the payload.
#[target_feature(enable = "bmi2,lzcnt")]
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

/// The per-block sequence-section header: nseq varint, modes byte, and the
/// three FSE table resolutions. Factored out of `decode_sequences_inner` so the
/// three ISA twins of `decode_compressed_block` share ONE copy -- nothing here
/// touches the bit reader the twins are compiled for. Runs once per block.
///
/// `Ok(None)` = a literals-only block (nseq == 0); the literals have already
/// been flushed to `out`.
#[inline(never)]
#[allow(clippy::type_complexity)]
// D14 REFUTED, recorded: moving the three `FseTable`s from this return type
// into `&mut Option<FseTable>` out-params measured **+383**
// (`decode_compressed_block` 955 -> 1,227, `decode_sequences_avx2` 875 ->
// 1,042). The premise looked sound -- that function is 60% MOVE instructions
// (571 of 955) and returns three `Vec`-owning structs by value.
//
// It was wrong twice over. LLVM already passes this tuple through the return
// slot without copying it; and `Option<FseTable>` adds a discriminant to each
// table plus an unwrap match at the call site, which costs more than the
// moves it was meant to remove.
//
// This BOUNDS the `select_seq_tables` lesson (+348) rather than extending it.
// There the interface was NINE values including three `Vec<u8>` headers built
// inside the callee. Here it is three structs the callee already owns and the
// caller immediately consumes. **"Large returns are expensive" is not a rule;
// measure the specific interface.**
fn decode_seq_header(
    src: &[u8],
    literals: &[u8],
    out: &mut Vec<u8>,
    state: &mut BlockState,
) -> Result<Option<(usize, u32, FseTable, FseTable, FseTable)>, Error> {
    let g_hdr = crate::prof::scope(crate::prof::Stage::DecSeqHeader);
    let _ = &g_hdr;
    let mut pos = 0usize;
    let byte0 = src[0];
    pos += 1;
    let nseq = if byte0 == 0 {
        // Literals-only block: this is tail work, not header work.
        drop(g_hdr);
        let _g = crate::prof::scope(crate::prof::Stage::DecSeqTail);
        out.extend_from_slice(literals);
        return Ok(None);
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
    let _g_tab = crate::prof::scope(crate::prof::Stage::DecSeqTables);
    // D19 REFUTED, recorded: converting the three `&src[pos..]` slices to
    // `get(..).ok_or(Corruption)` drove this function's pads 2 -> 0 and
    // measured **+3** (324 -> 327). Reverted.
    //
    // It pairs with D17/D18, which used the SAME transform and won (-44, -10).
    // The difference is what the pad guards. There, `st.opN`/`iN` reach the
    // slice through a struct or a loop counter and LLVM emits a real test on a
    // hot path. Here `pos` is already fenced by an explicit
    // `pos >= src.len()` check a few lines up, so the pad was already
    // near-free and the `Option` plumbing cost more than it removed.
    //
    // **Removing a pad pays when the bound is genuinely opaque, not when it is
    // merely spelled with brackets.**
    let (ll, n) = seq_table(
        &src[pos..],
        // D33: the SECOND and THIRD `&src[pos..]` only -- located by decoding the
        // landing pads' `Location` operands (`anon.*.90` -> 817:13,
        // `anon.*.89` -> 826:13), not by reading source.
        //
        // The FIRST one is deliberately left alone: at that point `pos` is still
        // fenced by the explicit `pos >= src.len()` test above, so it carries no
        // pad and `get` would only add cost. After `pos += n` it becomes opaque,
        // and the next two do.
        //
        // This is why D19 measured +3: it converted all THREE sites, paying for
        // the one that was already free. **Find the pad before removing it.**
        ll_mode,
        35,
        9,
        state.ll.take(),
        fse::default_ll,
    )?;
    pos += n;
    let (of, n) = seq_table(
        src.get(pos..).ok_or(Error::Corruption)?,
        of_mode,
        31,
        8,
        state.of.take(),
        fse::default_of,
    )?;
    pos += n;
    let (ml, n) = seq_table(
        src.get(pos..).ok_or(Error::Corruption)?,
        ml_mode,
        52,
        9,
        state.ml.take(),
        fse::default_ml,
    )?;
    pos += n;

    Ok(Some((pos, nseq, ll, of, ml)))
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
    // THE SEQUENCE-HEADER PARSE IS ONE NON-ISA CALL. `decode_sequences_inner`
    // is compiled into all three `decode_compressed_block` twins, and this
    // region -- the nseq varint, the modes byte, and the three FSE table
    // resolutions -- was stamped into each. None of it touches the bitstream
    // ISA the twins exist for. `None` = literals-only block, already flushed.
    let (pos, nseq, ll, of, ml) = match decode_seq_header(src, literals, out, state)? {
        Some(t) => t,
        None => return Ok(()),
    };
    // D32: the pad six earlier attempts could not find. Located by decoding
    // the landing pad's `Location` operand out of the asm -- `anon.*.95`
    // holds {file_ptr, len 0x23, line 0x360, col 0x19} = compressed.rs:864:25
    // -- rather than by reading source, which had failed repeatedly.
    //
    // `pos` is whatever `decode_seq_header` consumed: a parser return value,
    // opaque to LLVM, so this carried a bounds test and a pad, inlined into
    // BOTH `decode_sequences` twins. D17/D22's case exactly. (D19 failed on
    // the different `&src[pos..]` sites INSIDE the header parser, where `pos`
    // is already fenced a few lines up.)
    let bitstream = src.get(pos..).ok_or(Error::Corruption)?;
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
    let wide_arm = matchcopy_on();
    // COPYMATCH CUT 1 -- one reserve makes capacity a BLOCK INVARIANT. Nothing
    // reserved `out` before this; both copy tiers paid a capacity test per
    // SEQUENCE as their only guard. One `reserve` per block plus CUT 2's
    // budget bound (`len - block_start <= block_max`, below) gives
    // `capacity - len >= 64` at every sequence, and the hot tiers assert it
    // instead of testing it.
    const COPY_PAD: usize = 64;
    out.reserve(block_max as usize + COPY_PAD);
    // COPYMATCH CUT 2 -- the RFC block bound, enforced as a RUNNING BUDGET.
    // One subtract-and-branch per sequence replaces the old per-sequence
    // `len > block_max` (a weak proxy: no single length exceeded it while the
    // SUM could), covers BOTH copies' destination space under CUT 1's
    // reserve, and rejects a hostile over-long block at the first sequence
    // that overruns instead of at frame end.
    let mut budget = block_max as usize;
    // COPYMATCH CUT 5's right-hand side, hoisted: the window bound as a
    // usize, clamped, once per block.
    let win_lim: usize = window_size.min(usize::MAX as u64) as usize;
    // WIN 3: the per-sequence `nodict` test DELETED from the hot path. Dict
    // blocks get `win_eff == 0`, so CUT 5's guard (`off-1 >= min(dst, 0)`)
    // routes EVERY dictionary match into its reject arm -- where the real
    // `nodict` test lives, off the 99%-path. Plain frames pay nothing.
    let win_eff = if nodict { win_lim } else { 0 };
    // WIN 11: the fast predicate's arm conjunction, once per block. Shipping
    // folds both to `true`; profile builds save an AND per sequence.
    let fast_arms = litcopy_arm & wide_arm;
    // COPYMATCH-III WIN 7: the literal buffer's length, hoisted -- the tier
    // tests compare against a register instead of re-deriving per sequence.
    let lits_len = literals.len();
    // WIN 6: the literal SOURCE as two register recurrences -- a read pointer
    // and a remaining count -- instead of a spilled `lit_pos` re-loaded and
    // re-subtracted per sequence. `lit_pos` is derived (`lits_len - lit_rem`)
    // only at cold boundaries and the tail.
    // WIN 12: `lit_rem` is DELETED as a loop variable -- its per-sequence
    // update (subtract + spill store) is gone. The tier test `lit_rem >= 16`
    // becomes a guard compare against `lit_guard`, and the slow arm derives
    // the remaining count from the pointers when it needs it.
    // SAFETY: `lit_pos <= lits_len`, so the offset is in bounds.
    #[allow(unsafe_code)]
    let mut lit_p = unsafe { literals.as_ptr().add(lit_pos) };
    let lit_start = literals.as_ptr();
    // Wrapping on tiny buffers; the fused wrapping compare handles it.
    let lit_guard = lits_len.wrapping_sub(16);
    // Hoisted for the reason `litcopy_arm` is: an atomic load will not leave a loop.
    let prefetch_arm = prefetch_on();
    let pipeline_arm = pipeline_on();
    let pipe1_arm = pipe1_on();
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
    // DECSEQ CUT 7: `state.reps` lives behind the `&mut BlockState` pointer, so
    // every `resolve_offset` call read and wrote the history through memory the
    // optimiser must keep current across the copy calls. A local array is
    // promotable; it is written back once when the loop is done. (Error paths
    // return without the write-back -- a failed block poisons the whole frame,
    // so no caller reads the state after an Err.)
    let mut reps = state.reps;
    // ---------------------------------------------------------------------
    // D10: the decode-ahead PIPELINE -- our `ZSTD_decompressSequencesLong`.
    //
    // D9 prefetched the match source ~9.4 ns before `copy_match` wanted it and
    // measured nothing (+0.2%, z = -0.91): the out-of-order window already
    // covers that distance unaided. The fix is DISTANCE, and the reason zstd
    // needs a whole second decoder to get it is that the match ADDRESS looks
    // like it depends on execution. It does not:
    //
    //   * `resolve_offset(offset_value, litlen, reps)` reads `reps` and
    //     `litlen` only -- it never touches `out`. So offsets can be resolved
    //     for many sequences ahead, in order, before any byte is copied.
    //   * the output position each sequence will execute AT is just a running
    //     sum: `pred += litlen + matchlen`.
    //
    // So the source address `pred + litlen - offset` is fully known PIPE
    // sequences early. At PIPE = 8 that is ~8 x 30 ns = ~240 ns of distance
    // against a 13-40 ns miss, versus D9's 9.4 ns.
    //
    // BYTE-IDENTICAL: the same sequences are executed in the same order with
    // the same values; only DECODE moves earlier relative to EXECUTE, and the
    // bit reader never reads `out`. `examples/bytegate.rs` (GOLD
    // BE0071FB0CB0CED9, the GOLD of the day; see bytegate.rs for the
    // history) plus the round-trip suite are the gate.
    //
    // The original loop below stays in the tree as the oracle and the arm-off
    // path, per codec-optimize's "the slow version stays forever" rule.
    if pipeline_arm && nseq > 0 {
        const PIPE: usize = 8;
        let (mut q_ll, mut q_ml, mut q_off) = ([0u32; PIPE], [0u32; PIPE], [0u32; PIPE]);
        let (mut n_dec, mut n_exe) = (0u32, 0u32);
        // Predicted `out.len()` at the moment the next DECODED sequence runs.
        let mut pred = out.len();
        macro_rules! decode_one {
            () => {{
                let _ = br.reload();
                let ll_w = llv.entry_u32(ll_s);
                let of_w = ofv.entry_u32(of_s);
                let ml_w = mlv.entry_u32(ml_s);
                let ll_code = crate::fse::fse_symbol(ll_w) as usize;
                let of_code = u32::from(crate::fse::fse_symbol(of_w));
                let ml_code = crate::fse::fse_symbol(ml_w) as usize;
                debug_assert!(ll_code <= 35 && ml_code <= 52 && of_code <= 31);
                if !seqcheck && (ll_code > 35 || ml_code > 52 || of_code > 31) {
                    return Err(Error::Corruption);
                }
                let offset_add = br.read_bits(of_code);
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
                // CUT 4 applies here too: `read_bits(0) == 0`, so no 0-branch.
                let offset_value = {
                    // WIN 9: LUT load, not a %cl shift. `of_code <= 31` is the same
                    // build-time bound the seqcheck fold rests on (T4).
                    debug_assert!((of_code as usize) < OF_PACK.len());
                    #[allow(unsafe_code)]
                    let b = *unsafe { OF_PACK.get_unchecked(of_code as usize) };
                    b + offset_add
                };
                // Safe to run early: reads `reps` and `litlen`, never `out`.
                let offset = resolve_offset(offset_value, litlen, &mut reps)?;
                n_dec += 1;
                if n_dec != nseq {
                    let _ = br.reload();
                    ll_s = FseTable::advance_w(ll_w, &mut br);
                    ml_s = FseTable::advance_w(ml_w, &mut br);
                    of_s = FseTable::advance_w(of_w, &mut br);
                }
                // Issue the history load now, ~PIPE sequences before use.
                // Only into the ALREADY-DECODED region: a nearer match is in
                // cache anyway, and this keeps the pointer provably in-bounds.
                let at = pred + litlen as usize;
                let off = offset as usize;
                if off <= at && at - off < out.len() {
                    note_pf(0);
                    prefetch_addr(out, at - off);
                } else {
                    note_pf(1);
                }
                pred = at + matchlen as usize;
                let slot = (n_dec as usize - 1) % PIPE;
                q_ll[slot] = litlen;
                q_ml[slot] = matchlen;
                q_off[slot] = offset;
            }};
        }
        while n_dec < nseq && n_dec - n_exe < PIPE as u32 {
            decode_one!();
        }
        while n_exe < nseq {
            let slot = (n_exe as usize) % PIPE;
            let (litlen, matchlen, offset) = (q_ll[slot], q_ml[slot], q_off[slot]);
            let need = litlen as usize + matchlen as usize;
            if need > budget {
                return Err(Error::Corruption);
            }
            budget -= need;
            let dst0 = out.len();
            copy_literals_hot(literals, &mut lit_pos, litlen, out, litcopy_arm, dst0)?;
            if nodict {
                copy_match_nodict(
                    out,
                    dst0 + litlen as usize,
                    offset,
                    matchlen,
                    win_lim,
                    wide_arm,
                )?;
            } else {
                copy_match_dict_cold(out, &mctx, offset, matchlen)?;
            }
            n_exe += 1;
            if n_dec < nseq {
                decode_one!();
            }
        }
        rem = 0;
    }
    // ---------------------------------------------------------------------
    // D11: the DEPTH-1 interleave. One pending sequence held in registers;
    // per iteration: advance states past it, decode the NEXT sequence's
    // symbols, issue the NEXT match source's prefetch, THEN execute the
    // pending copies -- so the ~13-40 ns history miss overlaps a full
    // sequence (~30 ns) of real work instead of D9's 9.4 ns, with none of
    // D10's queue (three arrays and a modulo per step, which cost more than
    // the latency they hid, monotonically in depth).
    //
    // BYTE-IDENTICAL by construction: the bitstream is consumed in exactly
    // the classic order (decode N, advance N, decode N+1, ...), the same
    // copies run with the same values in the same output order, and a
    // prefetch has no architectural effect. `bytegate` GOLD is the gate.
    // (On a CORRUPT stream the classic loop had executed sequence N before
    // rejecting N+1 where this rejects first; failed blocks are discarded by
    // every caller, so partial content on error is outside the contract.)
    if pipe1_arm && rem != 0 {
        macro_rules! d11_decode {
            () => {{
                let _ = br.reload();
                let ll_w = llv.entry_u32(ll_s);
                let of_w = ofv.entry_u32(of_s);
                let ml_w = mlv.entry_u32(ml_s);
                let ll_code = crate::fse::fse_symbol(ll_w) as usize;
                let of_code = u32::from(crate::fse::fse_symbol(of_w));
                let ml_code = crate::fse::fse_symbol(ml_w) as usize;
                debug_assert!(ll_code <= 35 && ml_code <= 52 && of_code <= 31);
                if !seqcheck && (ll_code > 35 || ml_code > 52 || of_code > 31) {
                    return Err(Error::Corruption);
                }
                let offset_add = br.read_bits(of_code);
                #[allow(unsafe_code)]
                let (ll_w2, ml_w2) = unsafe {
                    debug_assert!(ll_code < LL_PACK.len() && ml_code < ML_PACK.len());
                    (
                        *LL_PACK.get_unchecked(ll_code),
                        *ML_PACK.get_unchecked(ml_code),
                    )
                };
                let ml_add = br.read_bits(u32::from(pk_bits(ml_w2)));
                let ll_add = br.read_bits(u32::from(pk_bits(ll_w2)));
                let litlen = pk_base(ll_w2) + ll_add;
                let matchlen = pk_base(ml_w2) + ml_add;
                // CUT 4's identity, as in the classic loop.
                let offset_value = {
                    // WIN 9: LUT load, not a %cl shift. `of_code <= 31` is the same
                    // build-time bound the seqcheck fold rests on (T4).
                    debug_assert!((of_code as usize) < OF_PACK.len());
                    #[allow(unsafe_code)]
                    let b = *unsafe { OF_PACK.get_unchecked(of_code as usize) };
                    b + offset_add
                };
                (ll_w, ml_w, of_w, litlen, matchlen, offset_value)
            }};
        }
        macro_rules! d11_exec {
            ($lit:expr, $off:expr, $mat:expr) => {{
                let need = $lit as usize + $mat as usize;
                if need > budget {
                    return Err(Error::Corruption);
                }
                budget -= need;
                let dst0 = out.len();
                copy_literals_hot(literals, &mut lit_pos, $lit, out, litcopy_arm, dst0)?;
                if nodict {
                    copy_match_nodict(out, dst0 + $lit as usize, $off, $mat, win_lim, wide_arm)?;
                } else {
                    copy_match_dict_cold(out, &mctx, $off, $mat)?;
                }
            }};
        }
        // Prologue: decode the first sequence; it becomes the pending one.
        let (mut cw_ll, mut cw_ml, mut cw_of, mut c_lit, mut c_mat, ov0) = d11_decode!();
        let mut c_off = resolve_offset(ov0, c_lit, &mut reps)?;
        rem -= 1;
        while rem != 0 {
            rem -= 1;
            // Advance past the pending sequence, then decode the next --
            // identical bit order to the classic loop.
            let _ = br.reload();
            ll_s = FseTable::advance_w(cw_ll, &mut br);
            ml_s = FseTable::advance_w(cw_ml, &mut br);
            of_s = FseTable::advance_w(cw_of, &mut br);
            let (nw_ll, nw_ml, nw_of, n_lit, n_mat, n_ov) = d11_decode!();
            // Runs a sequence early on purpose: reads `reps` and `litlen`
            // only, never `out` (the D10 block proves the same property).
            let n_off = resolve_offset(n_ov, n_lit, &mut reps)?;
            // The whole point: the NEXT match source's address is fully known
            // before the pending copies run. Prefetch only into the
            // already-decoded region -- a nearer source is cache-warm anyway.
            let pred = out.len() + c_lit as usize + c_mat as usize + n_lit as usize;
            let n_off_us = n_off as usize;
            if n_off_us <= pred {
                let at = pred - n_off_us;
                if at < out.len() {
                    prefetch_addr(out, at);
                }
            }
            // Execute the pending sequence; the next one's miss is in flight.
            d11_exec!(c_lit, c_off, c_mat);
            (cw_ll, cw_ml, cw_of) = (nw_ll, nw_ml, nw_of);
            (c_lit, c_mat, c_off) = (n_lit, n_mat, n_off);
        }
        // Epilogue: the last pending sequence (no advance after the last
        // decode, exactly as the classic loop skips it).
        d11_exec!(c_lit, c_off, c_mat);
    }
    // COPYMATCH-III WIN 1 -- the loop runs on RAW CURSORS, C's `op`/`oend`
    // discipline. The emitted asm showed `out` living behind TWO levels of
    // indirection (`344(%rbp)` -> Vec -> `8`/`16(%rdx)`), its `ptr` and `len`
    // fields reloaded TWICE per sequence and `set_len` stored twice more.
    // `base`/`dst` live in registers; `set_len` publishes only at a COLD
    // boundary (whose callee may reallocate, so `base` is refetched after),
    // at the loop exit, and ahead of the profile-only taps. On an `Err`
    // return `out.len()` may lag bytes physically written past it -- those
    // are unpublished spare capacity, and every caller discards the block.
    // WIN 10: the cursor is `op` -- C's own form -- so every tier store
    // addresses `op` DIRECTLY instead of computing `base + dst` per copy; the
    // integer `dst` is derived only where the guard needs it.
    let mut base = out.as_mut_ptr();
    #[allow(unsafe_code)]
    let mut op = unsafe { base.add(out.len()) };
    macro_rules! publish {
        () => {{
            // SAFETY: `dst` counts only initialised bytes (every tier writes
            // before advancing it) and the block invariant keeps it within
            // capacity.
            #[allow(unsafe_code)]
            unsafe {
                out.set_len(op.offset_from(base) as usize)
            };
        }};
    }
    macro_rules! refetch {
        () => {{
            base = out.as_mut_ptr();
            #[allow(unsafe_code)]
            unsafe {
                op = base.add(out.len());
            }
        }};
    }
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
        // DECSEQ CUT 4: the old `if of_code == 0 { 1 } else { ... }` restated
        // what the expression already computes -- `read_bits(0)` returns 0 by
        // definition (bit.rs) and `1u32 << 0` is 1, so the branch and the
        // else-arm were byte-for-byte the same value. One branch per sequence,
        // deleted by algebra rather than prediction.
        let offset_value = {
            // WIN 9: LUT load, not a %cl shift. `of_code <= 31` is the same
            // build-time bound the seqcheck fold rests on (T4).
            debug_assert!((of_code as usize) < OF_PACK.len());
            #[allow(unsafe_code)]
            let b = *unsafe { OF_PACK.get_unchecked(of_code as usize) };
            b + offset_add
        };

        // D9: start the history load NOW, so its miss overlaps the ~9.4 ns of
        // literal copy and offset resolution that run before `copy_match`.
        // (Profile-armed tap: it reads `out.len()`, so publish first.)
        if prefetch_arm {
            publish!();
            prefetch_hist(out, litlen, offset_value);
        }

        // ---- DecSeq loop anatomy: duplicate ONE op, then undo it ----
        // (Anatomy taps drive the Vec API, so the cursors publish around them.)
        #[cfg(feature = "dupladder")]
        {
            publish!();
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
                        let _ = black_box(br.reload());
                        let _ = black_box(br.reload());
                        br.dup_restore(sv);
                    }
                    5 => {
                        let mut lit_pos = (lit_p as usize).wrapping_sub(lit_start as usize);
                        let (lp, len) = (lit_pos, out.len());
                        let _ = copy_literals(literals, &mut lit_pos, litlen, out, litcopy_arm);
                        lit_pos = lp;
                        out.truncate(len);
                    }
                    _ => {}
                }
            }
            refetch!();
        }
        // COPYMATCH CUT 2 -> MEGAFUSE: the budget test now lives inside the
        // fused fast-path predicate below; the slow arm re-derives it.
        let need = litlen as usize + matchlen as usize;
        let n = litlen as usize;
        #[cfg(feature = "dupladder")]
        if dup == 6 {
            for _ in 0..dup_k {
                let sv = reps;
                let _ = core::hint::black_box(resolve_offset(offset_value, litlen, &mut reps));
                reps = sv;
            }
        }
        let offset = resolve_offset(offset_value, litlen, &mut reps)?;
        #[cfg(feature = "dupladder")]
        if dup == 7 {
            publish!();
            for _ in 0..dup_k {
                let len = out.len();
                let _ = copy_match(out, &mctx, offset, matchlen);
                out.truncate(len);
            }
            refetch!();
        }
        let off = offset as usize;
        let mlen = matchlen as usize;
        let b_ok = budget.wrapping_sub(need);
        let dst = (op as usize).wrapping_sub(base as usize);
        let lit_off = (lit_p as usize).wrapping_sub(lit_start as usize);
        // ---- THE JOINT-SEQUENCE MEGAFUSE ----------------------------------
        // SEVEN borrow-free differences, ONE sign test, for the [79.6%, 80.4%]
        // of sequences (`jointrate.rs`) where both copies take their 16-byte
        // tiers. What each term retires on the fast path:
        //   b_ok            = budget - (lit+match)   CUT 2's reject branch
        //   16 - n                                    lit tier, half
        //   lit_rem - 16                              lit tier, half -- and it
        //                                             IMPLIES the input bound
        //                                             (n <= 16 <= lit_rem), so
        //                                             that reject's term is
        //                                             DROPPED as redundant
        //   16 - mlen, off - 16                       match tier
        //   dst - off, win_eff - off                  CUT 5's guard with the
        //                                             `min`/cmov DECOMPOSED
        //                                             into two terms; `off>=16`
        //                                             subsumes the `off >= 1`
        //                                             wrap-guard
        // Every magnitude is < 2^32, so the OR's sign bit is exactly "some
        // condition failed". The slow arm below re-derives everything -- it is
        // the pre-fuse code, verbatim, and byte-identity holds because the
        // fused predicate is precisely the conjunction of the tests it
        // replaces.
        let fused = b_ok
            | 16usize.wrapping_sub(n)
            | lit_guard.wrapping_sub(lit_off)
            | 16usize.wrapping_sub(mlen)
            | off.wrapping_sub(16)
            | dst.wrapping_sub(off)
            | win_eff.wrapping_sub(off);
        if fast_arms & ((fused as isize) >= 0) {
            budget = b_ok;
            // SAFETY: `lit_rem >= 16` gives 16 readable literal bytes;
            // `off >= 16` with `off <= dst` gives 16 readable, disjoint match
            // bytes; the block invariant gives `need + 64` writable bytes past
            // `dst`; only `n + mlen` are published. `dst - off` is reused from
            // the predicate as the match source.
            #[allow(unsafe_code)]
            unsafe {
                #[cfg(feature = "profile")]
                {
                    DEC_LIT16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    DEC_MATCH16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                }
                note_band(2, mlen);
                core::ptr::copy_nonoverlapping(lit_p, op, 16);
                lit_p = lit_p.add(n);
                let d = op.add(n);
                core::ptr::copy_nonoverlapping(d.sub(off), d, 16);
                // ONE cursor advance for both copies (WIN 12 killed the
                // `lit_rem` update that used to sit beside it).
                op = op.add(need);
            }
        } else {
            // ---- SLOW ARM: the pre-fuse path, verbatim ------------------------
            // D31 REFUTED: `saturating_sub` here measured **+4** and did not move
            // the pad count -- so this subtraction was never the remaining pad in
            // `decode_compressed_block`, and its check was already folded into the
            // `(b_ok | l_ok) < 0` test below. Seventh and last refutation in the
            // pad class.
            let lit_rem = lits_len - lit_off;
            let l_ok = lit_rem.wrapping_sub(n);
            if ((b_ok | l_ok) as isize) < 0 {
                return Err(Error::Corruption);
            }
            budget = b_ok;
            // WIN 4: single branch, FORCED arithmetically like WIN 5 -- the setcc
            // form proved allocator-unstable (LLVM re-split it on a later build).
            // `n <= 16` and `lit_rem >= 16` are both borrow-free subtractions.
            if litcopy_arm
                & ((((16usize.wrapping_sub(n)) | (lit_rem.wrapping_sub(16))) as isize) >= 0)
            {
                // SAFETY: as the fused path's literal half.
                #[allow(unsafe_code)]
                unsafe {
                    #[cfg(feature = "profile")]
                    DEC_LIT16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                    core::ptr::copy_nonoverlapping(lit_p, op, 16);
                    lit_p = lit_p.add(n);
                    op = op.add(n);
                }
            } else {
                // WIN 10: the fallback goes STRAIGHT to the cold rungs -- routing
                // it back through the inline tier re-asked (and re-emitted) the
                // exact test that just failed. The input bound above already
                // holds, so the cold precondition (`end <= lits_len`) does too.
                // SECTION 27: literal raw protocol first. SAFETY: `lit_rem`
                // literals are readable (the input bound just passed); the block
                // invariant gives `n + 64` writable bytes.
                #[allow(unsafe_code)]
                let raw = if litcopy_arm {
                    unsafe { lit_cold_raw(lit_p, lit_rem, op, n) }
                } else {
                    None
                };
                match raw {
                    Some((np, nop)) => {
                        lit_p = np;
                        op = nop;
                    }
                    None => {
                        publish!();
                        let mut lit_pos = lit_off;
                        let end = lit_pos + n;
                        if litcopy_arm {
                            copy_literals_cold::<true>(literals, &mut lit_pos, end, n, out, dst)?;
                        } else {
                            copy_literals_cold::<false>(literals, &mut lit_pos, end, n, out, dst)?;
                        }
                        #[allow(unsafe_code)]
                        unsafe {
                            lit_p = literals.as_ptr().add(lit_pos);
                        }
                        refetch!();
                    }
                }
            }
            {
                // WIN 1's match side: CUT 5's fused guard and the 16-byte tier on
                // the raw cursors; everything else publishes and takes the cold
                // monomorph, refetching `base` because the cold rungs may grow.
                // WIN 3 folded the dict dispatch into this guard via `win_eff`.
                let dst = (op as usize).wrapping_sub(base as usize);
                if off.wrapping_sub(1) >= dst.min(win_eff) {
                    if nodict {
                        return Err(Error::Corruption);
                    }
                    publish!();
                    copy_match_dict_cold(out, &mctx, offset, matchlen)?;
                    refetch!();
                } else {
                    // WIN 5: ONE branch, forced arithmetically -- the `&` form still
                    // emitted two compare-and-branch pairs here. `off >= 16` leaves
                    // `off - 16` borrow-free; `mlen <= 16` leaves `16 - mlen`
                    // borrow-free; both magnitudes are far below 2^63, so the OR of
                    // the two wrapping differences is sign-negative exactly when
                    // either condition fails. One `or`, one sign test.
                    if wide_arm
                        & ((((off.wrapping_sub(16)) | (16usize.wrapping_sub(mlen))) as isize) >= 0)
                    {
                        // SAFETY: `off >= 16` = readable + disjoint (CUT 5 proved
                        // `off <= dst`); 16 writable bytes by the block invariant;
                        // only `mlen <= 16` published via `dst`.
                        #[allow(unsafe_code)]
                        unsafe {
                            #[cfg(feature = "profile")]
                            DEC_MATCH16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
                            note_band(2, mlen);
                            core::ptr::copy_nonoverlapping(op.sub(off), op, 16);
                            op = op.add(mlen);
                        }
                    } else {
                        // SECTION 27: the raw protocol first -- ~99% of cold calls
                        // skip the publish/refetch boundary entirely. SAFETY: the
                        // guard above proved `1 <= off <= dst`; the block invariant
                        // gives `mlen + 64` writable bytes.
                        #[allow(unsafe_code)]
                        let raw = if wide_arm {
                            unsafe { match_cold_raw(op, off, mlen) }
                        } else {
                            None
                        };
                        match raw {
                            Some(nop) => op = nop,
                            None => {
                                publish!();
                                if wide_arm {
                                    copy_from_decoded_cold(true, true, out, dst, off, mlen)?;
                                } else {
                                    copy_from_decoded_cold(true, false, out, dst, off, mlen)?;
                                }
                                refetch!();
                            }
                        }
                    }
                }
            }
        }

        if rem != 0 {
            let _ = br.reload();
            ll_s = FseTable::advance_w(ll_w, &mut br);
            ml_s = FseTable::advance_w(ml_w, &mut br);
            of_s = FseTable::advance_w(of_w, &mut br);
        }
    }
    drop(g_loop);
    // WIN 1's exit publish: the cursors return to the Vec; WIN 6's literal
    // cursor returns to `lit_pos` for the tail.
    publish!();
    lit_pos = (lit_p as usize).wrapping_sub(lit_start as usize);
    // CUT 7's write-back: the loop is done, the history returns to the state.
    state.reps = reps;
    // COPYMATCH CUT 7: the trailing literals complete the block bound -- a
    // conformant block's total regenerated size (sequences + tail) is within
    // `block_max`, so a tail that overruns the remaining budget is corrupt.
    // One compare per BLOCK, and the invariant CUTS 3/4 rely on holds through
    // the tail as well.
    // D20: TWO checks become one. `lit_pos` is computed by `wrapping_sub` on
    // raw pointers a few lines up, so it is genuinely opaque -- LLVM cannot
    // relate it to `literals.len()`. That cost a subtraction-underflow check on
    // `literals.len() - lit_pos` AND a bounds test on `&literals[lit_pos..]`,
    // each with its own panic pad, inlined into both `decode_sequences` twins.
    //
    // One `get` proves the bound once and yields the remainder, whose `len()`
    // is exactly what the budget test wanted. Identical behaviour: an out-of-
    // range `lit_pos` reached both old checks and now returns `Corruption`.
    // This is D17's case, not D19's -- the bound here is opaque, not merely
    // spelled with brackets.
    let rest = literals.get(lit_pos..).ok_or(Error::Corruption)?;
    if rest.len() > budget {
        return Err(Error::Corruption);
    }
    {
        let _g = crate::prof::scope(crate::prof::Stage::DecSeqTail);
        out.extend_from_slice(rest);
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
//
// PROMOTED, and here is why the pending A/B is no longer the gate it was.
// W18's own caveat draws the line at FREQUENCY: "outlining anything reachable
// PER SEQUENCE measured 2.5% SLOWER ... per-block is the safe side of that
// line." `seq_table` runs three times per BLOCK, on the safe side by its own
// test. And its stated mechanism -- "inlined into BOTH `decode_sequences`
// twins beside the hot loop, costing the loop its register budget" -- was
// already fixed from the other end: `decode_seq_header` is `#[inline(never)]`
// since Trans VII, so this build no longer sits beside the sequence loop at
// all. What is left is pure code size: SIX shipping call sites, each inlining
// 51 lines, with no symbol of its own.
#[inline(never)]
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

/// DECSEQ CUT 3 -- pattern A (see section 10.1 of the plan): this arm guards a
/// PER-SEQUENCE test, so in the shipping build it folds to the constant `true`
/// (the range test stays hoisted to table build, which has been the default
/// since brick 45) and the whole `!seqcheck && (...)` body vanishes from both
/// sequence-loop twins. The tri-state read and the `RZSTD_SEQCHECK_HOIST`
/// escape survive under `--features profile`, where the A/B harness lives.
#[cfg(feature = "profile")]
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
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn seqcheck_hoisted() -> bool {
    true
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
#[cfg_attr(
    not(any(test, feature = "dupladder")),
    allow(dead_code) // the checked oracle of `copy_literals_hot`; tests + the dupladder anatomy drive it.
)]
fn copy_literals(
    literals: &[u8],
    lit_pos: &mut usize,
    litlen: u32,
    out: &mut Vec<u8>,
    arm: bool,
) -> Result<(), Error> {
    let n = litlen as usize;
    // DECSEQ CUT 8: the old `checked_add(n)?` paid an add-plus-overflow-branch
    // per sequence to guard a sum the compare below already bounds. Phrased as
    // a REMAINING-LENGTH test, no addition can overflow on any pointer width:
    // `lit_pos <= literals.len()` is this function's own postcondition (it
    // only ever advances `lit_pos` to a validated `end`), so the subtraction
    // is in range, and `end` is computed only after `n` is proven to fit.
    debug_assert!(*lit_pos <= literals.len());
    if n > literals.len() - *lit_pos {
        return Err(Error::Corruption);
    }
    let end = *lit_pos + n;
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
    if arm {
        copy_literals_cold::<true>(literals, lit_pos, end, n, out, len)
    } else {
        copy_literals_cold::<false>(literals, lit_pos, end, n, out, len)
    }
}

/// COPYMATCH CUTS 4 and 6 -- the sequence loop's literal copy. Two things the
/// general `copy_literals` above pays per call are BLOCK INVARIANTS inside the
/// budgeted loop and drop out here:
///
/// * the capacity test: cut 1 reserves `block_max + 64` at loop entry and
///   cut 2's budget bounds `len - block_start` by `block_max`, so
///   `capacity - len >= 64` holds at EVERY sequence -- asserted, not tested;
/// * the `out.len()` re-read: the caller passes `dst_at` in a register.
///
/// The general fn stays as the oracle for the tests and the unbudgeted paths.
#[inline(always)]
#[allow(unsafe_code)]
fn copy_literals_hot(
    literals: &[u8],
    lit_pos: &mut usize,
    litlen: u32,
    out: &mut Vec<u8>,
    arm: bool,
    dst_at: usize,
) -> Result<(), Error> {
    let n = litlen as usize;
    debug_assert!(*lit_pos <= literals.len());
    debug_assert_eq!(dst_at, out.len());
    debug_assert!(out.capacity() - dst_at >= 64, "block reserve invariant");
    if n > literals.len() - *lit_pos {
        return Err(Error::Corruption);
    }
    let end = *lit_pos + n;
    if arm && n <= 16 && *lit_pos + 16 <= literals.len() {
        // SAFETY: the test above gives 16 readable source bytes; the block
        // invariant (reserve + budget, see the doc comment) gives 16 writable
        // destination bytes inside the allocation; `literals` and `out` are
        // distinct buffers; exactly `n <= 16` bytes are published.
        unsafe {
            #[cfg(feature = "profile")]
            DEC_LIT16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            core::ptr::copy_nonoverlapping(
                literals.as_ptr().add(*lit_pos),
                out.as_mut_ptr().add(dst_at),
                16,
            );
            out.set_len(dst_at + n);
        }
        *lit_pos = end;
        return Ok(());
    }
    if arm {
        copy_literals_cold::<true>(literals, lit_pos, end, n, out, dst_at)
    } else {
        copy_literals_cold::<false>(literals, lit_pos, end, n, out, dst_at)
    }
}

/// Tiers 2 and 3 plus the `extend_from_slice` fallback: under 0.4% of literal
/// copies. Outlined and cold so tier 1 keeps its inlining.
#[allow(unsafe_code)]
#[inline(never)]
#[cold]
fn copy_literals_cold<const A: bool>(
    literals: &[u8],
    lit_pos: &mut usize,
    end: usize,
    n: usize,
    out: &mut alloc::vec::Vec<u8>,
    len: usize,
) -> Result<(), Error> {
    // COPYMATCH-III WIN 9: `arm` was a runtime ARGUMENT tested by every rung
    // of this OUTLINED symbol -- across a call boundary the caller's folded
    // constant cannot propagate. As a const parameter the shipping monomorph
    // tests nothing and drops the dead rungs outright.
    let arm = A;
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
    // D22, second site: both ends are runtime and the fast tiers above have
    // already returned, so nothing here proves the range to LLVM.
    let chunk = literals.get(*lit_pos..end).ok_or(Error::Corruption)?;
    out.extend_from_slice(chunk);
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

// SIMD-2 ARM REMOVED, knob and all. The AVX2 block driver it selected was
// retired; `block_avx2_on` was left with no readers and `set_block_avx2_arm`
// with no callers, so the public setter stored a value nothing consulted.
// Silencing the dead reader would have kept a write-only knob in the API,
// which is worse than removing it -- a caller could set it and reasonably
// believe something changed.

/// Runtime arms for the pre-2026-08-15 bricks, so the in-process ABBA
/// harness can re-adjudicate them. Each defaults ON (shipping behaviour).
static LUT_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);
static LITCOPY_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);
static MATCHCOPY_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(2);
/// D9 COVERAGE census: prefetches issued, vs skipped because the offset is a
/// REP code (needs `state.reps`, resolved after the prefetch point), vs skipped
/// as out-of-range. Deterministic -- this is what SIZES D9, because a prefetch
/// only helps the sequences it is actually issued for.
#[cfg(feature = "pfcensus")]
pub static PF_CENSUS: [core::sync::atomic::AtomicU64; 3] =
    [const { core::sync::atomic::AtomicU64::new(0) }; 3];
/// Read and clear `(issued, skipped_rep, skipped_oob)`.
#[cfg(feature = "pfcensus")]
pub fn take_pf_census() -> (u64, u64, u64) {
    use core::sync::atomic::Ordering::Relaxed;
    (
        PF_CENSUS[0].swap(0, Relaxed),
        PF_CENSUS[1].swap(0, Relaxed),
        PF_CENSUS[2].swap(0, Relaxed),
    )
}
#[cfg(feature = "pfcensus")]
#[inline(always)]
fn note_pf(i: usize) {
    PF_CENSUS[i].fetch_add(1, core::sync::atomic::Ordering::Relaxed);
}
#[cfg(not(feature = "pfcensus"))]
#[inline(always)]
fn note_pf(_i: usize) {}

/// D9 arm: issue the match source's history load one step early.
///
/// **Defaults OFF, and that is a NOISE verdict, not a quality one** -- see
/// codec-measurement 12, which requires saying which kind of revert this was.
/// `examples/d9cover.rs` sizes the brick deterministically at **94.1% coverage,
/// 21.9% of DecSeqLoop** if every issued prefetch fully hid its miss -- a large
/// effect, and one `examples/d9probe.rs` could not see: **+0.2%, z = -0.91,
/// against a NULL ARM of 27.7%** on a box carrying `faucet.exe` at 86,147 CPU-s
/// and eight `Code.exe` at 40-90k each.
///
/// The point estimate leans mildly negative (129/273 pairs), so it does not ship
/// on by default. The likely mechanism is that 9.4 ns is simply not far enough:
/// the out-of-order window already reaches past `copy_literals` to the
/// `copy_match` load without help, so the software prefetch buys an instruction
/// and no latency. Closing that needs a FULL SEQUENCE of distance (~40 ns),
/// i.e. the depth-1 decode-ahead pipeline in plan section 13.6.
///
/// Kept behind the arm so re-testing on a quiet box is one call, per
/// codec-measurement 12.
static PREFETCH_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(1);
/// D10 arm: the decode-ahead PIPELINE -- our `ZSTD_decompressSequencesLong`.
///
/// **MEASURED WORSE. Defaults OFF. This is a revert-because-WORSE, not a
/// revert-because-noise** (codec-measurement 12 requires saying which).
///
/// Correct: `examples/d10gate.rs` compares both arms byte-for-byte over 90
/// release cells (5 levels x 18 corpora) and 36 debug cells -- ALL MATCH. The
/// pipeline is a faithful implementation, not a broken one.
///
/// Slower, at every distance tried (`examples/d10probe.rs`, L3, DecSeqLoop
/// stage isolated, ABBA):
///
/// | brick | prefetch distance | delta | z | pairs won |
/// |---|---|---:|---:|---:|
/// | D9 (plain prefetch) | ~9.4 ns | +0.2% | -0.91 | 129/273 |
/// | D10, PIPE = 4 | ~120 ns | **+15.8%** | -13.25 | 5/195 |
/// | D10, PIPE = 8 | ~240 ns | **+18.4%** | -14.59 | 16/273 |
///
/// Three probes, three distances, monotone the WRONG way -- so this is a
/// three-probe refutation of the DIRECTION (codec-measurement 11), not one bad
/// number. The z-scores are far too large to be the 26-33% null arm.
///
/// Why: the queue (three arrays, a head/tail pair, the macro expanded at two
/// sites) costs more than the latency it hides, and its cost GROWS with depth
/// while the latency saved does not. Same shape as brick 64's finding a few
/// lines below -- a structural change that removed real work and still lost,
/// because the structure was the expensive part.
///
/// zstd needs this decoder because its short path differs from ours; ours does
/// not benefit. Kept behind the arm per codec-measurement 12 so a future
/// restructure can re-test it in one call.
/// Set on to enable; see the table above before doing so.
static PIPELINE_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(1);

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
/// D10 arm: the decode-ahead pipeline.
pub fn set_pipeline_arm(on: bool) {
    PIPELINE_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}

/// D11 arm: the DEPTH-1 interleave -- decode sequence N+1's symbols and issue
/// its history prefetch BEFORE executing N's copies. Section 13.6's original
/// shape, held in registers with no queue (the queue is what refuted D10), at
/// the distance (~one sequence, ~30 ns) that D9's 9.4 ns lacked. Section 21's
/// density probe is the motive: C handles MORE sequences 1.71x faster, so the
/// per-sequence stall is ours to hide, and C's own interleaved short decoder
/// is the existence proof.
static PIPE1_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(1);
/// D11 arm: depth-1 decode-ahead. See `PIPE1_ARM`.
pub fn set_pipe1_arm(on: bool) {
    PIPE1_ARM.store(
        if on { 2 } else { 1 },
        core::sync::atomic::Ordering::Relaxed,
    );
}
/// **MEASURED WORSE -- the THIRD distance, closing the direction.** d11probe,
/// L3, 21 ABBA rounds, DecSeqLoop isolated, null median 2.16%:
/// **OFF 30.27 -> ON 30.56 ns/seq = +0.9%, 110/273 pairs, z = -3.21** (xml
/// z = -3.71, mozilla z = -2.84; best corpus ooffice only +1.96). With D9
/// (9.4 ns, null) and D10 (~240 ns + queue, -18%), software decode-ahead is
/// now refuted at THREE distances on this core (codec-measurement 11): the
/// OoO window already overlaps adjacent sequences' misses, and any software
/// shape only adds instructions to a loop that is instruction-THROUGHPUT
/// bound. The lever for the C gap is FEWER instructions per sequence, not
/// hidden latency. Kept behind the arm (codec-measurement 12), folded to
/// const in shipping builds like its siblings.
#[cfg(feature = "profile")]
#[inline(always)]
fn pipe1_on() -> bool {
    PIPE1_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn pipe1_on() -> bool {
    false
}
/// D9 arm: history prefetch in the sequence loop.
pub fn set_prefetch_arm(on: bool) {
    PREFETCH_ARM.store(
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
/// DECSEQ-II CUT 6 -- pattern A, like the four arms above: the brick-35 LUT
/// has been the shipping default since it landed, so the non-`profile` build
/// folds the arm to `true` and the linear-scan alternative drops out of the
/// `write_sequences` twins that hoist this per block.
#[cfg(feature = "profile")]
#[inline(always)]
pub(crate) fn lut_on() -> bool {
    LUT_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
pub(crate) fn lut_on() -> bool {
    true
}
/// DECSEQ CUTS 1, 2 and 6 -- pattern A (plan section 10.1), applied to the four
/// arms `decode_sequences_inner` hoists per block. Their READERS fold to the
/// shipping constants in a non-`profile` build; the setters and statics stay,
/// API-unchanged, so the A/B probes (`d9probe`, `d10probe`, the copy-tier
/// harnesses -- all of which build with `--features profile`) keep their knobs.
///
/// What the constants delete from BOTH sequence-loop twins:
///   - `pipeline_on() == false`: the ENTIRE parked D10 decode-ahead block,
///     including two macro expansions of the full symbol-decode body.
///   - `prefetch_on() == false`: the parked D9 `prefetch_hist` call and its
///     per-sequence branch.
///   - `litcopy_on() / matchcopy_on() == true`: the per-sequence `arm` tests in
///     the literal and match copy tiers become tautologies and vanish.
///
/// This is the same fold `eqlen_arm()` has used since it was measured at 247M
/// per-call loads (10.1: "any site inside a hot loop"), applied late to four
/// arms that were added at block frequency and then reached into per-sequence
/// bodies.
#[cfg(feature = "profile")]
#[inline(always)]
fn litcopy_on() -> bool {
    LITCOPY_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn litcopy_on() -> bool {
    true
}
#[cfg(feature = "profile")]
#[inline(always)]
fn matchcopy_on() -> bool {
    MATCHCOPY_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn matchcopy_on() -> bool {
    true
}

#[cfg(feature = "profile")]
#[inline(always)]
fn prefetch_on() -> bool {
    PREFETCH_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn prefetch_on() -> bool {
    false
}

#[cfg(feature = "profile")]
#[inline(always)]
fn pipeline_on() -> bool {
    PIPELINE_ARM.load(core::sync::atomic::Ordering::Relaxed) == 2
}
#[cfg(not(feature = "profile"))]
#[inline(always)]
fn pipeline_on() -> bool {
    false
}

/// Pull `out[at]` toward L1. `at` MUST be inside the initialised region.
#[inline(always)]
#[allow(unsafe_code)]
fn prefetch_addr(out: &[u8], at: usize) {
    debug_assert!(at < out.len());
    #[cfg(target_arch = "x86_64")]
    // SAFETY: caller guarantees `at < out.len()`, so the pointer is in-bounds.
    // `_mm_prefetch` reads nothing and cannot fault.
    unsafe {
        core::arch::x86_64::_mm_prefetch(
            out.as_ptr().add(at) as *const i8,
            core::arch::x86_64::_MM_HINT_T0,
        );
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: as above; `prfm pldl1keep` is architecturally a no-op.
    unsafe {
        core::arch::asm!(
            "prfm pldl1keep, [{0}]",
            in(reg) out.as_ptr().add(at),
            options(nostack, readonly, preserves_flags)
        );
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = at;
}

/// D9: pull the match source into L1 one step before `copy_match` needs it.
///
/// The `dsloop` ladder prices `copy_match` at **23.29 ns/sequence, 57.6% of
/// DecSeqLoop**, for a mean match length of 7.4 bytes (86.6% of copies are
/// `len <= 16`). A 16-byte `movups` pair is ~1 ns, so that 23 ns is not copy
/// work -- it is the load of `out[len - offset]`, a random address up to the
/// whole window back. At L3 the window is 2 MiB, which overflows L2.
///
/// The address is knowable BEFORE the literal copy runs, so the miss can
/// overlap `copy_literals` + `resolve_offset` (7.11 + 2.28 = 9.4 ns of the
/// ~13 ns L2 miss) instead of stalling in front of `copy_match`.
///
/// Only the LITERAL-offset case is prefetched (`offset_value > 3`), where
/// `offset == offset_value - 3` exactly and needs no `reps`. Rep codes would
/// need the resolution this deliberately runs ahead of; they are skipped rather
/// than guessed, because a wrong guess evicts a line the real load then wants.
///
/// **This cannot change an output byte** -- a prefetch has no architectural
/// effect and the address is bounds-checked into the already-decoded region
/// purely to keep the pointer arithmetic in-bounds. `examples/bytegate.rs` is
/// the gate (GOLD BE0071FB0CB0CED9, the value current when this brick landed --
/// see bytegate.rs for the history), and per section 13.2 of the plan the brick
/// is also WORK-identical, so it has no deterministic counter of its own: the
/// `dsloop` ladder's `copy_match` line is the instrument, not a whole-decode
/// clock against a 10.88-16.74% null arm.
#[inline(always)]
#[allow(unsafe_code)]
fn prefetch_hist(out: &[u8], litlen: u32, offset_value: u32) {
    // Rep codes (1..=3) need `state.reps`, which is resolved after this point.
    if offset_value <= 3 {
        note_pf(1);
        return;
    }
    let off = (offset_value - 3) as usize;
    // Where `copy_match` will read from, once `copy_literals` has published
    // `litlen` more bytes: `(out.len() + litlen) - off`.
    let end = out.len() + litlen as usize;
    if off > end {
        note_pf(2);
        return; // corrupt stream; `copy_match` rejects it a few ns from now
    }
    let at = end - off;
    if at >= out.len() {
        note_pf(2);
        return; // source not decoded yet -- also corrupt, same reasoning
    }
    note_pf(0);
    #[cfg(target_arch = "x86_64")]
    // SAFETY: `at < out.len()`, so the pointer is inside the initialised
    // region. `_mm_prefetch` reads nothing and faults on nothing.
    unsafe {
        core::arch::x86_64::_mm_prefetch(
            out.as_ptr().add(at) as *const i8,
            core::arch::x86_64::_MM_HINT_T0,
        );
    }
    #[cfg(target_arch = "aarch64")]
    // SAFETY: as above. `prfm pldl1keep` is architecturally a no-op.
    unsafe {
        core::arch::asm!(
            "prfm pldl1keep, [{0}]",
            in(reg) out.as_ptr().add(at),
            options(nostack, readonly, preserves_flags)
        );
    }
    #[cfg(not(any(target_arch = "x86_64", target_arch = "aarch64")))]
    let _ = at;
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
    // DECSEQ CUT 5: single dispatch. The old body selected the offset in one
    // `if`/`match`, then re-derived which case it had been in a SECOND time --
    // `is_new` re-tested `offset_value > 3` and `== 3 && litlen == 0`, and a
    // second `match` on `which` re-decoded the rep index -- to apply the
    // history update the first match had already determined. Every arm below
    // is one case of the RFC 8878 repcode table with its update fused, so each
    // condition is tested exactly once. Case-for-case against the old body:
    if offset_value > 3 {
        // New offset (old: `> 3` arm, then `is_new` shift).
        let o = offset_value - 3;
        reps[2] = reps[1];
        reps[1] = reps[0];
        reps[0] = o;
        return Ok(o);
    }
    let o = if litlen != 0 {
        match offset_value {
            // rep1 (old: offset reps[0], `which == 1`, no reorder).
            1 => reps[0],
            // rep2 (old: offset reps[1], `which == 2`, swap(0, 1)).
            2 => {
                reps.swap(0, 1);
                reps[0]
            }
            // rep3 (old: offset reps[2], `which == 3`, rotate_right(1)).
            3 => {
                let o = reps[2];
                reps[2] = reps[1];
                reps[1] = reps[0];
                reps[0] = o;
                o
            }
            _ => return Err(Error::Corruption),
        }
    } else {
        // litlen == 0 shifts the meaning of each value by one.
        match offset_value {
            // value 1 is rep2 (old: offset reps[1], `which == 2`, swap).
            1 => {
                reps.swap(0, 1);
                reps[0]
            }
            // value 2 is rep3 (old: offset reps[2], `which == 3`, rotate).
            2 => {
                let o = reps[2];
                reps[2] = reps[1];
                reps[1] = reps[0];
                reps[0] = o;
                o
            }
            // value 3 is rep1 - 1 -- a NEW offset (old: checked_sub + filter,
            // then the `is_new` shift). `reps[0] <= 1` is exactly the set the
            // old `checked_sub(1).filter(|&o| o > 0)` rejected.
            3 => {
                if reps[0] <= 1 {
                    return Err(Error::Corruption);
                }
                let o = reps[0] - 1;
                reps[2] = reps[1];
                reps[1] = reps[0];
                reps[0] = o;
                o
            }
            _ => return Err(Error::Corruption),
        }
    };
    Ok(o)
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
    dst_at: usize,
    offset: u32,
    matchlen: u32,
    win_lim: usize,
    wide: bool,
) -> Result<(), Error> {
    debug_assert_eq!(dst_at, out.len());
    let off = offset as usize;
    // COPYMATCH CUT 5: THREE rejects, ONE compare. Cut 10 already fused
    // `off == 0` and `off > produced` via the wrapping subtract; folding the
    // window bound into the right-hand side with `min` takes the third:
    // `off - 1 >= min(produced, win_lim)` is true exactly when `off == 0`
    // (wraps to usize::MAX) OR `off > produced` OR `off > win_lim`, and all
    // three were `Err(Corruption)`. The old per-sequence `len > block_max`
    // test is gone entirely -- CUT 2's running budget in the sequence loop
    // subsumes it with a strictly TIGHTER bound (the RFC block bound over the
    // whole block, not one length at a time).
    if off.wrapping_sub(1) >= dst_at.min(win_lim) {
        return Err(Error::Corruption);
    }
    copy_from_decoded_hot(out, dst_at, off, matchlen as usize, wide)
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

/// DECSEQ CUT 9: the dictionary-path arm of the sequence loop, outlined.
///
/// `copy_match` is `inline(always)`, so as the `else` arm of `if nodict` its
/// whole body -- dict-crossing split included -- was stamped into BOTH
/// sequence-loop twins, though dictionary decode is the rare shape (`nodict`
/// covers every plain frame). Round five's rule: outlining pays in proportion
/// to how many times the HOST is reproduced. Here the host is duplicated per
/// twin and the arm is block-rare, so the body moves behind one cold call and
/// the hot loop keeps only `copy_match_nodict`.
#[cold]
#[inline(never)]
fn copy_match_dict_cold(
    out: &mut Vec<u8>,
    mctx: &MatchCtx<'_>,
    offset: u32,
    matchlen: u32,
) -> Result<(), Error> {
    copy_match(out, mctx, offset, matchlen)
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
    // D28: THREE checks become one. `src_pos0` is derived from the match
    // offset against the dictionary boundary -- opaque -- so
    // `dict.len() - src_pos0` carried an underflow test and
    // `&dict[src_pos0..src_pos0 + from_dict]` a bounds test, each with a pad.
    // Taking the tail ONCE proves the offset, `tail.len()` is exactly the
    // remaining-bytes figure the `min` wanted, and `&tail[..from_dict]` is then
    // provably in range because `from_dict <= tail.len()` by that same `min`.
    let tail = dict.get(src_pos0..).ok_or(Error::Corruption)?;
    let from_dict = core::cmp::min(len, tail.len());
    out.extend_from_slice(&tail[..from_dict]);
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
    copy_from_decoded_body(out, src, len, wide)
}

/// DECSEQ-II CUT 9 + COPYMATCH CUTS 3 and 6 -- the hot-path tier, every check
/// either discharged by the caller or a block invariant:
///
/// * `1 <= off <= dst_at` -- `copy_match_nodict`'s fused compare;
/// * `len > 0` -- RFC matchlen >= 3 (`ML_PACK[0]` is 3);
/// * capacity -- CUT 1's `reserve(block_max + 64)` + CUT 2's budget bound
///   `len - block_start <= block_max`, so `capacity - dst_at >= 64` at every
///   sequence (CUT 3: the per-sequence capacity test is DELETED);
/// * `dst_at` and `off` arrive in registers -- no `out.len()` re-read after
///   the literal copy's `set_len`, no `offset`/`src` re-derivation (CUT 6).
///
/// The checked `copy_from_decoded` above remains the general entry for the
/// dictionary path and the tests.
#[inline(always)]
#[allow(unsafe_code)]
fn copy_from_decoded_hot(
    out: &mut Vec<u8>,
    dst_at: usize,
    off: usize,
    len: usize,
    wide: bool,
) -> Result<(), Error> {
    debug_assert_eq!(dst_at, out.len());
    debug_assert!(off >= 1 && off <= dst_at && len > 0);
    debug_assert!(out.capacity() - dst_at >= 64, "block reserve invariant");
    if wide && len <= 16 && off >= 16 {
        // SAFETY: `off >= 16` puts 16 readable initialised bytes at the
        // source AND makes source/destination disjoint; the block invariant
        // (see doc comment) gives 16 writable bytes past `dst_at`; exactly
        // `len <= 16` bytes are published by `set_len`.
        unsafe {
            let p = out.as_mut_ptr();
            #[cfg(feature = "profile")]
            DEC_MATCH16.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            note_band(2, len);
            core::ptr::copy_nonoverlapping(p.add(dst_at - off), p.add(dst_at), 16);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    if wide {
        copy_from_decoded_cold(true, true, out, dst_at, off, len)
    } else {
        copy_from_decoded_cold(true, false, out, dst_at, off, len)
    }
}

#[inline(always)]
#[allow(unsafe_code)]
fn copy_from_decoded_body(
    out: &mut Vec<u8>,
    src: usize,
    len: usize,
    wide: bool,
) -> Result<(), Error> {
    // `src < out.len()` on both entries, so `offset >= 1` -- the old
    // `offset == 0` reject was already unreachable and is simply gone.
    let offset = out.len() - src;
    // DECSEQ-II CUT 10 -- the `offset == 1` byte-splat test ran per SEQUENCE
    // ahead of tier 1 for a band the census puts at 0.01% of calls (band 0).
    // Offset 1 can never satisfy tier 1's `offset >= 16`, so moving the splat
    // into the cold fn changes NOTHING about which path any call takes -- it
    // only takes its test off the 86.6% path. It now lives at the top of
    // `copy_from_decoded_cold`.
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
    if wide {
        copy_from_decoded_cold(false, true, out, out.len(), offset, len)
    } else {
        copy_from_decoded_cold(false, false, out, out.len(), offset, len)
    }
}

/// Match-copy tiers 2 and 3 plus the fallbacks: the ~19.6% of match copies
/// that tier 1 declines. Outlined and cold so tier 1 keeps its inlining.
///
/// COPYMATCH-II: monomorphised over `G` -- `G == true` is the budgeted
/// sequence-loop path, where section 23's reserve + budget give
/// `capacity - (dst_at + len) >= 64` and the capacity tests fold away;
/// `G == false` keeps every runtime test for the dictionary path and the
/// oracle tests. Same source, two symbols, no drift.
///
/// Rung ORDER is call-weighted from `bandcensus` (L3, 8 corpora): the 32-tier
/// takes 13.0% of all copies, the 64-tier 4.6%, `within` 1.8%, overlap 0.24%,
/// splat 0.01% -- so the splat test moved from FIRST (where 19.6% of copies
/// paid it) to after the fixed tiers (where ~2% do). The brick-82 sub-census
/// branch (`band 5`) read ZERO after the 16-first reorder -- tier 1 provably
/// takes every `len <= 16, off >= 32` copy -- and is deleted.
/// SECTION 27 -- the RAW cold protocol for the byte-weighted middle rungs.
///
/// The cold bands carry HALF the match bytes, and every call paid the full
/// Vec boundary: publish (`set_len`), a `&mut Vec` argument, and a refetch.
/// Under the block invariant the fixed-width rungs CANNOT grow `out`, so none
/// of that is needed: `op` in, new `op` out, no Vec anywhere. `None` marks
/// the genuinely Vec-needing leftovers (the overlap band at 0.24% of copies
/// and the `off < 32` within-cases); ~99% of cold calls skip the boundary.
///
/// Rungs are call-ordered from `bandcensus` (t32 12.99%, t64 4.57%, within
/// 1.79%, splat 0.01%) and each test is ONE or-sign branch. The within rungs
/// need no length test at all: reaching them past t32/t64 IMPLIES `len > 32`
/// (resp. `> 64`). `within` takes 64-byte strides when `off >= 64` -- the
/// old path strode 32 for every offset, so iterations on the 12.9%-of-bytes
/// band halve. Stride overshoot is at most 63 bytes into the invariant's
/// 64-byte pad; every stride's source read ends at or before the write
/// cursor, so it reads only initialised bytes.
///
/// SAFETY (caller): the sequence-loop invariants -- `1 <= off <= op - base`,
/// `len + 64` writable bytes past `op`, `wide` arm on.
#[cold]
#[inline(never)]
#[allow(unsafe_code)]
unsafe fn match_cold_raw(op: *mut u8, off: usize, len: usize) -> Option<*mut u8> {
    unsafe {
        if (((32usize.wrapping_sub(len)) | (off.wrapping_sub(32))) as isize) >= 0 {
            #[cfg(feature = "profile")]
            DEC_MATCH32.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            note_band(1, len);
            core::ptr::copy_nonoverlapping(op.sub(off), op, 32);
            return Some(op.add(len));
        }
        if (((64usize.wrapping_sub(len)) | (off.wrapping_sub(64))) as isize) >= 0 {
            note_band(6, len);
            core::ptr::copy_nonoverlapping(op.sub(off), op, 64);
            return Some(op.add(len));
        }
        if off >= 64 {
            // len > 64 is implied (t64 would have taken it).
            note_band(3, len);
            note_untiered(len);
            let mut done = 0usize;
            while done < len {
                core::ptr::copy_nonoverlapping(op.sub(off).add(done), op.add(done), 64);
                done += 64;
            }
            return Some(op.add(len));
        }
        if off >= 32 {
            // len > 32 implied; off in 32..64 keeps the 32-byte stride sound.
            note_band(3, len);
            note_untiered(len);
            let mut done = 0usize;
            while done < len {
                core::ptr::copy_nonoverlapping(op.sub(off).add(done), op.add(done), 32);
                done += 32;
            }
            return Some(op.add(len));
        }
        if off == 1 {
            note_band(0, len);
            core::ptr::write_bytes(op, *op.sub(1), len);
            return Some(op.add(len));
        }
    }
    None
}

/// The literal side of the raw protocol: tiers 2/3 without the Vec boundary.
/// `rem` is the caller's already-derived remaining literal count, so the
/// bound tests are register compares; `None` falls back to the checked cold
/// (the `extend_from_slice` tail and near-end cases).
/// SAFETY (caller): `rem` literals readable at `lit_p`; `n + 64` writable
/// bytes past `op`; literal arm on.
#[cold]
#[inline(never)]
#[allow(unsafe_code)]
unsafe fn lit_cold_raw(
    lit_p: *const u8,
    rem: usize,
    op: *mut u8,
    n: usize,
) -> Option<(*const u8, *mut u8)> {
    unsafe {
        if (((32usize.wrapping_sub(n)) | (rem.wrapping_sub(32))) as isize) >= 0 {
            #[cfg(feature = "profile")]
            DEC_LIT32.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            core::ptr::copy_nonoverlapping(lit_p, op, 32);
            return Some((lit_p.add(n), op.add(n)));
        }
        if (((64usize.wrapping_sub(n)) | (rem.wrapping_sub(64))) as isize) >= 0 {
            #[cfg(feature = "profile")]
            DEC_LIT64.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            core::ptr::copy_nonoverlapping(lit_p, op, 64);
            return Some((lit_p.add(n), op.add(n)));
        }
    }
    None
}

#[allow(unsafe_code)]
#[inline(never)]
#[cold]
// D8: THE `g`/`w` CONST AXES GO RUNTIME -- three copies of this body become
// one. Both consts are load-bearing (`w` gates the 32/64-byte tiers, `g` elides
// a capacity test), but this is the COLD tier: the fast paths live in
// `copy_from_decoded_hot`, and this is its fallback. Paying two predictable
// branches here to stop stamping a 208-instruction copier three times is the
// same trade W7/W9/W10 made for REP/WIDE/PACKED on the fast finder.
fn copy_from_decoded_cold(
    g: bool,
    w: bool,
    out: &mut Vec<u8>,
    dst_at: usize,
    off: usize,
    len: usize,
) -> Result<(), Error> {
    // WIN 8: the match-side twin of WIN 9 -- `wide` as a const, not a tested
    // runtime argument, in the outlined symbol.
    let wide = w;
    debug_assert_eq!(dst_at, out.len());
    debug_assert!(off >= 1 && off <= dst_at && len > 0);
    let src = dst_at - off;
    // D29 REFUTED, recorded: `saturating_sub` on both tier guards'
    // `out.capacity() - dst_at` measured **+4**. The underflow check was
    // already cheap here -- it sits inside a `&&` chain LLVM had folded -- and
    // the saturating form blocked that folding.
    if wide && len <= 32 && off >= 32 && (g || out.capacity() - dst_at >= 32) {
        // SAFETY: `off >= 32` means `src + 32 <= dst_at`, so the source is
        // fully initialised and disjoint from the destination. Writable
        // space: the block invariant under `g`, the runtime test otherwise.
        // Only `len <= 32` bytes are published by `set_len`.
        // (AVX2 AUDIT 4.56: a `target_feature` copy helper here measured
        // WORSE -- call overhead vs 4 inline SSE instructions. Not retried.)
        unsafe {
            let p = out.as_mut_ptr();
            #[cfg(feature = "profile")]
            DEC_MATCH32.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
            note_band(1, len);
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 32);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    // 64-BYTE TIER (history in `dsuntier.rs`: 65.9% of the once-untiered
    // band's calls were 33..=64 bytes). `off >= 64` = readable + disjoint.
    if wide && len <= 64 && off >= 64 && (g || out.capacity() - dst_at >= 64) {
        // SAFETY: as the 32-tier, at width 64.
        unsafe {
            let p = out.as_mut_ptr();
            note_band(6, len);
            core::ptr::copy_nonoverlapping(p.add(src), p.add(dst_at), 64);
            out.set_len(dst_at + len);
        }
        return Ok(());
    }
    // Splat (C ZSTD_overlapCopy8's offset-1 case), band 0 at 0.01% of copies.
    if off == 1 {
        note_band(0, len);
        if g {
            // SAFETY: budget accounting gives `capacity - (dst_at + len) >=
            // 64`, so `len` bytes past `dst_at` are writable; `src < dst_at`
            // is the entry invariant. One memset replaces `resize`'s
            // element-wise extend.
            unsafe {
                let b = *out.as_ptr().add(src);
                core::ptr::write_bytes(out.as_mut_ptr().add(dst_at), b, len);
                out.set_len(dst_at + len);
            }
        } else {
            let b = out[src];
            out.resize(dst_at + len, b);
        }
        return Ok(());
    }
    if off >= len {
        note_band(3, len);
        note_untiered(len);
        // COPYMATCH-II: the untiered band carried 12.9% of ALL match BYTES
        // through a runtime-length `extend_from_within` -- an internal
        // reserve, range checks and a memcpy CALL per copy. Under `g` with
        // `off >= 32`, a 32-byte strided wildcopy does it inline: every
        // chunk's source read ends at `src + 32(i+1) <= dst_at + 32i`, i.e.
        // at or before the write cursor, so it reads only initialised bytes;
        // the write may overshoot `len` by up to 31 bytes, which the block
        // invariant's 64-byte pad absorbs; only `len` bytes are published.
        if g && off >= 32 {
            unsafe {
                let p = out.as_mut_ptr();
                let mut done = 0usize;
                while done < len {
                    core::ptr::copy_nonoverlapping(p.add(src + done), p.add(dst_at + done), 32);
                    done += 32;
                }
                out.set_len(dst_at + len);
            }
            return Ok(());
        }
        // D24: `extend_from_within` has no `get`-shaped API, so the bound is
        // stated explicitly instead. Naming `end` once lets LLVM carry the
        // proof into the call rather than re-deriving `src + len` inside it.
        let end = src.checked_add(len).ok_or(Error::Corruption)?;
        if end > out.len() {
            return Err(Error::Corruption);
        }
        out.extend_from_within(src..end);
        return Ok(());
    }
    note_band(4, len);
    out.reserve(len);
    // COPYMATCH-II: `avail` is a RECURRENCE (`off`, then `+= take`), not a
    // re-read -- the per-iteration `out.len() - src` load-and-subtract is
    // gone, and with it the `avail == 0` reject: `off >= 2` here (the splat
    // rung took `off == 1`, the entry invariant `off >= 1`), so the first
    // iteration always progresses and `avail` only grows. D3's refutation of
    // the pattern-replicate rewrite stands; this only slims the loop it kept.
    let mut avail = off;
    let mut copied = 0usize;
    while copied < len {
        // D25: the loop's `extend_from_within`, given the D24 treatment. `take`
        // is bounded by `avail` and `len - copied`, but neither relates to
        // `out.len()` in a way LLVM can see across the iteration.
        let take = (len - copied).min(avail);
        let end = src.checked_add(take).ok_or(Error::Corruption)?;
        if end > out.len() {
            return Err(Error::Corruption);
        }
        out.extend_from_within(src..end);
        copied += take;
        avail += take;
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
