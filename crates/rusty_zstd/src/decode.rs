//! One-shot decompress: skippable, raw, RLE, and compressed frames.

use crate::block::{parse_block_header, BlockType};
use crate::compressed::{decode_compressed_block, BlockState};
#[cfg(feature = "alloc")]
use crate::dict::Dictionary;
use crate::error::Error;
use crate::frame::{parse_kind, FrameHeader, FrameKind, DEFAULT_WINDOW_MAX, MAGIC};
use crate::reader::Reader;
use crate::xxh64::content_checksum;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Decoder knobs for [`decompress_with`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DecompressOptions {
    /// Reject frames whose window exceeds this (bytes). Default 128 MiB.
    pub window_max: u64,
    /// Skip XXH64 content-checksum VERIFICATION (`ZSTD_d_forceIgnoreChecksum`).
    ///
    /// The 4 checksum bytes are still consumed, so concatenated frames keep
    /// parsing; only the comparison is skipped. Default `false`.
    ///
    /// This is a real cost, not a micro-optimisation: xxh64 over the output is
    /// **the majority of decode time on high-ratio content** (measured 61% on a
    /// 32 MiB all-zeros frame -- 7792 MB/s with verification, 20150 without).
    /// Set this only where the transport already guarantees integrity; it
    /// disables the frame's own corruption detection.
    pub force_ignore_checksum: bool,
}

impl Default for DecompressOptions {
    fn default() -> Self {
        Self {
            window_max: DEFAULT_WINDOW_MAX,
            force_ignore_checksum: false,
        }
    }
}

/// GATE 4 @ L19: hash decoded blocks as they land instead of re-reading the whole
/// output at the end.
///
/// **DEFAULT OFF -- it measured FLAT.** The hypothesis was that the final pass
/// re-reads from DRAM what the decoder just wrote, so hashing each block while it
/// is cache-hot should be cheaper. Measured both ways:
///
///   1 MiB outputs   +1.99%  (incomp -18.1%, samba +17.4%)
///   16 MiB outputs  -1.70%  (zeros -8.0%, text -8.6%, mozilla +5.0%)
///
/// The direction depends on output size and content and never exceeds the
/// scatter. It wins where the decode is trivially fast and the checksum
/// dominates, and loses on dense binary -- plausibly because xxh64's 4-lane
/// stripe loop wants one long contiguous buffer, and per-block updates break the
/// stripes and add per-call setup that cancels the locality gain.
///
/// Kept as an arm rather than deleted because the mechanism is real and would
/// matter for a streaming decoder, where the final pass is not even available.
/// `set_ck_stream_arm(true)` enables it; both arms compute the same XXH64 over
/// the same bytes in the same order, verified good-frame and corrupt-frame on
/// 18/18 corpora.
/// DEFAULT OFF (`usize::MAX` never fires). The size dispatch was built on a
/// MEASUREMENT ARTIFACT.
///
/// Five runs of `decompress()` said fusing won, monotonically in output size:
/// -0.35% at 1 MiB, -2.62% at 2, -4.58% at 8, -5.27% at 32, with a clean
/// cache-residency story (small output = cache-hot re-read, large output = cold
/// memory traversal). A sixth run flipped the 8 MiB cell to +0.63%.
///
/// `decompress()` ALLOCATES its output buffer, so every timed call paid fresh
/// page faults -- and page-fault cost scales with buffer size, which is the same
/// curve the cache story predicts. Re-measured with `decompress_into` on a warmed,
/// reused buffer:
///
/// ```text
///   output    fused vs separate     null
///    2 MiB          +0.35%          2.0%
///    8 MiB          -0.20%          0.5%   <- null is 0.5% and the effect is gone
///   32 MiB          -1.14%          1.6%
/// ```
///
/// There is no effect. The arm's original rejection was correct, and the whole
/// apparent dispatch was the allocator.
const CK_FUSE_MIN: usize = usize::MAX;

static CK_FUSE_ARM: core::sync::atomic::AtomicUsize = core::sync::atomic::AtomicUsize::new(0);

/// Bench arm. `usize::MAX` disables the size dispatch (pre-4.79 behaviour).
pub fn set_ck_fuse_arm(v: usize) {
    CK_FUSE_ARM.store(v, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn ck_fuse_min() -> usize {
    let v = CK_FUSE_ARM.load(core::sync::atomic::Ordering::Relaxed);
    if v == 0 {
        CK_FUSE_MIN
    } else {
        v
    }
}

static CK_STREAM_ARM: core::sync::atomic::AtomicU8 = core::sync::atomic::AtomicU8::new(0);

/// Bench hook for the Gate 4 decode-checksum A/B.
pub fn set_ck_stream_arm(on: bool) {
    CK_STREAM_ARM.store(u8::from(on) + 1, core::sync::atomic::Ordering::Relaxed);
}

#[inline]
fn ck_stream_enabled() -> bool {
    matches!(CK_STREAM_ARM.load(core::sync::atomic::Ordering::Relaxed), 2)
}

/// One-shot decompress of one or more concatenated frames (skippable frames ignored).
#[cfg(feature = "alloc")]
pub fn decompress(src: &[u8]) -> Result<Vec<u8>, Error> {
    decompress_with(src, DecompressOptions::default())
}

/// One-shot decompress using a dictionary (`ZSTD_decompress_usingDict`).
#[cfg(feature = "alloc")]
pub fn decompress_using_dict(src: &[u8], dict: &Dictionary) -> Result<Vec<u8>, Error> {
    decompress_using_dict_with(src, dict, DecompressOptions::default())
}

/// [`decompress_using_dict`] with an explicit window cap (`-d --long`).
#[cfg(feature = "alloc")]
pub fn decompress_using_dict_with(
    src: &[u8],
    dict: &Dictionary,
    opts: DecompressOptions,
) -> Result<Vec<u8>, Error> {
    decompress_with_history(src, opts, Some(dict), &[])
}

/// One-shot decompress using a prefix (`ZSTD_decompress_usingDDict` / `--patch-from`).
#[cfg(feature = "alloc")]
pub fn decompress_using_prefix(src: &[u8], prefix: &[u8]) -> Result<Vec<u8>, Error> {
    decompress_using_prefix_with(src, prefix, DecompressOptions::default())
}

/// [`decompress_using_prefix`] with an explicit window cap.
#[cfg(feature = "alloc")]
pub fn decompress_using_prefix_with(
    src: &[u8],
    prefix: &[u8],
    opts: DecompressOptions,
) -> Result<Vec<u8>, Error> {
    decompress_with_history(src, opts, None, prefix)
}

/// One-shot decompress with an explicit window cap.
#[cfg(feature = "alloc")]
pub fn decompress_with(src: &[u8], opts: DecompressOptions) -> Result<Vec<u8>, Error> {
    decompress_with_history(src, opts, None, &[])
}

/// Decompress into a caller-owned buffer, **appending**. Returns bytes written.
///
/// This is the `ZSTD_decompress(dst, dstCapacity, ..)` shape: the caller owns
/// the allocation and can reuse it across many frames. [`decompress`] must hand
/// back a fresh `Vec` every call, which means fresh pages the kernel has to
/// fault in and zero before the decoder may write them.
///
/// **That allocation, not the decoding, is the dominant cost on
/// high-ratio content.** Measured on a 32 MiB all-zeros frame (256 RLE blocks):
/// the full `decompress` call took 12.6 ms, of which a bare
/// allocate-and-touch-every-page with **no decoder involved at all** was
/// 10.3 ms -- 82%. The same memset into an already-faulted buffer is 1.3 ms.
/// Reuse the buffer and the decode is several times faster; every real consumer
/// that decodes more than one message wants this entry point.
///
/// ```
/// let frame = rusty_zstd::compress(b"hello hello hello", 3).unwrap();
/// let mut buf = Vec::new();
/// let n = rusty_zstd::decompress_into(&mut buf, &frame).unwrap();
/// assert_eq!(&buf[..n], b"hello hello hello");
/// // Reuse the allocation for the next frame.
/// buf.clear();
/// rusty_zstd::decompress_into(&mut buf, &frame).unwrap();
/// ```
#[cfg(feature = "alloc")]
pub fn decompress_into(dst: &mut Vec<u8>, src: &[u8]) -> Result<usize, Error> {
    decompress_into_with(dst, src, DecompressOptions::default())
}

/// [`decompress_into`] with an explicit window cap.
#[cfg(feature = "alloc")]
pub fn decompress_into_with(
    dst: &mut Vec<u8>,
    src: &[u8],
    opts: DecompressOptions,
) -> Result<usize, Error> {
    let start = dst.len();
    decompress_into_history(src, opts, None, &[], dst)?;
    Ok(dst.len() - start)
}

#[cfg(feature = "alloc")]
fn decompress_with_history(
    src: &[u8],
    opts: DecompressOptions,
    dict: Option<&Dictionary>,
    prefix: &[u8],
) -> Result<Vec<u8>, Error> {
    let mut out = Vec::new();
    decompress_into_history(src, opts, dict, prefix, &mut out)?;
    Ok(out)
}

/// Shared body. `out` may already hold data; everything is appended, and each
/// frame's back-references are bounded by its own start offset.
#[cfg(feature = "alloc")]
fn decompress_into_history(
    src: &[u8],
    opts: DecompressOptions,
    dict: Option<&Dictionary>,
    prefix: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    if src.is_empty() {
        return Err(Error::UnexpectedEof);
    }
    let hist = dict.map(Dictionary::content).unwrap_or(prefix);
    let mut r = Reader::new(src);
    let mut saw_zstd = false;
    while !r.is_empty() {
        match r.peek_u32_le() {
            Ok(m) if m == MAGIC || crate::frame::is_skippable_magic(m) => {}
            Ok(_) if saw_zstd => return Err(Error::TrailingBytes),
            Ok(_) => return Err(Error::BadMagic),
            Err(e) => {
                if saw_zstd && r.remaining() > 0 {
                    return Err(Error::TrailingBytes);
                }
                return Err(e);
            }
        }
        match parse_kind(&mut r)? {
            FrameKind::Skippable { user_data_size, .. } => {
                let n = user_data_size as usize;
                let _ = r.take(n)?;
            }
            FrameKind::Zstd(header) => {
                decode_zstd_frame(&mut r, header, opts, dict, hist, out)?;
                saw_zstd = true;
            }
        }
    }
    if !saw_zstd {
        return Err(Error::UnexpectedEof);
    }
    Ok(())
}

/// Frame_Content_Size of the first Zstd frame, skipping leading skippable frames.
/// `None` means the size was not present in the header.
pub fn content_size(src: &[u8]) -> Result<Option<u64>, Error> {
    let mut r = Reader::new(src);
    loop {
        match parse_kind(&mut r)? {
            FrameKind::Skippable { user_data_size, .. } => {
                let _ = r.take(user_data_size as usize)?;
            }
            FrameKind::Zstd(h) => return Ok(h.content_size),
        }
    }
}

/// Byte length of the first frame (Zstd or skippable), including checksum.
pub fn find_frame_compressed_size(src: &[u8]) -> Result<usize, Error> {
    let mut r = Reader::new(src);
    match parse_kind(&mut r)? {
        FrameKind::Skippable { user_data_size, .. } => {
            let _ = r.take(user_data_size as usize)?;
            Ok(r.pos())
        }
        FrameKind::Zstd(header) => {
            skip_blocks(&mut r, header)?;
            Ok(r.pos())
        }
    }
}

/// One frame in a concatenated stream (`-l` / [`inspect_frames`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ListedFrame {
    /// Header of this frame (Zstd or skippable).
    pub kind: FrameKind,
    /// Compressed byte length including header, blocks, and checksum.
    pub compressed_size: usize,
}

/// Walk every concatenated frame (Zstd and skippable).
#[cfg(feature = "alloc")]
pub fn inspect_frames(src: &[u8]) -> Result<Vec<ListedFrame>, Error> {
    let mut out = Vec::new();
    let mut off = 0usize;
    if src.is_empty() {
        return Err(Error::UnexpectedEof);
    }
    while off < src.len() {
        let n = find_frame_compressed_size(&src[off..])?;
        if n == 0 {
            return Err(Error::Corruption);
        }
        // D23 REFUTED, recorded: `get(off..off+n).ok_or(Corruption)` here
        // removed the pad but grew this function 166 -> 182 (**+16**). The
        // `saturating_add` plus the `Option` cost more than the check.
        //
        // Third data point for the pad rule, and it is consistent: the win is
        // real where the index is genuinely opaque and the site is hot
        // (D17 -44, D21 -31, D22), and negative where the bound is already
        // near-provable or the site is cold (D19 +3, D23 +16).
        let kind = crate::get_frame_header(&src[off..off + n])?;
        out.push(ListedFrame {
            kind,
            compressed_size: n,
        });
        off += n;
    }
    Ok(out)
}

/// Block-type census of the first Zstd frame (C vs us work-parity at bitstream level).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct BlockCensus {
    pub raw: u32,
    pub rle: u32,
    pub compressed: u32,
    pub raw_bytes: u64,
    pub rle_regen: u64,
    pub compressed_payload: u64,
}

/// Count Raw / RLE / Compressed blocks without reconstructing the payload.
#[cfg(feature = "alloc")]
pub fn frame_block_census(src: &[u8]) -> Result<BlockCensus, Error> {
    let mut r = Reader::new(src);
    match parse_kind(&mut r)? {
        FrameKind::Skippable { .. } => Err(Error::BadMagic),
        FrameKind::Zstd(_) => {
            let mut c = BlockCensus::default();
            loop {
                let bh = parse_block_header(&mut r)?;
                match bh.ty {
                    BlockType::Raw => {
                        c.raw += 1;
                        c.raw_bytes += u64::from(bh.size);
                    }
                    BlockType::Rle => {
                        c.rle += 1;
                        c.rle_regen += u64::from(bh.size);
                    }
                    BlockType::Compressed => {
                        c.compressed += 1;
                        c.compressed_payload += u64::from(bh.size);
                    }
                }
                let _ = r.take(bh.payload_len() as usize)?;
                if bh.last {
                    break;
                }
            }
            Ok(c)
        }
    }
}

#[cfg(feature = "alloc")]
// D16 REFUTED, and the mistake that produced it is worth more than the attempt.
//
// I outlined this believing it had TWO call sites. It has ONE. The count came
// from an ad-hoc `grep -c 'decode_zstd_frame('` -- which counts the DEFINITION
// as a site. (The census script in `tools/premise_audit.py` subtracts one for
// exactly this reason; the shell one-liner I typed instead did not.)
//
// With one caller, outlining cannot remove a copy -- it can only add a frame,
// and it did: `decompress_into_history` 570 -> 143, `decode_zstd_frame` 464,
// total 607 against 570. **+37.**
//
// CHECK THE SITE COUNT EXCLUDES THE DEFINITION before outlining anything.
fn decode_zstd_frame(
    r: &mut Reader<'_>,
    header: FrameHeader,
    opts: DecompressOptions,
    dict: Option<&Dictionary>,
    hist: &[u8],
    out: &mut Vec<u8>,
) -> Result<(), Error> {
    let _dec = crate::prof::scope(crate::prof::Stage::DecodeTotal);
    if header.window_size > opts.window_max {
        return Err(Error::WindowTooLarge);
    }
    if let Some(id) = header.dict_id {
        match dict {
            None => return Err(Error::DictionaryNeeded { id }),
            Some(d) if d.id() != id => {
                return Err(Error::DictionaryMismatch {
                    frame: id,
                    loaded: d.id(),
                });
            }
            Some(_) => {}
        }
    }
    if let Some(n) = header.content_size {
        let extra = usize::try_from(n).map_err(|_| Error::ContentSizeTooLarge)?;
        out.try_reserve(extra)
            .map_err(|_| Error::ContentSizeTooLarge)?;
    }

    let block_max = header.block_size_max();
    let start_len = out.len();
    let mut block_state = BlockState::from_dict(dict);

    // GATE 4 @ L19 -- hash each block as it lands, while it is still hot.
    //
    // The ENCODER has always done this (`h.update(&workspace[off..end])` per
    // block). The DECODER did one pass over the WHOLE output after every block
    // was written, re-reading from memory what it had just produced. On content
    // the decoder finishes quickly that second traversal dominates: an all-zeros
    // frame decodes by `resize` (a memset) and then pays a full re-read to hash
    // it, which measured the checksum at 5x the entire decode.
    //
    // Brick 85 fused the checksum into the ENCODE block loop and measured 12%
    // WORSE, but the two cases are not the same: there the source is already
    // being streamed by the match finder, so the hash competes for the same
    // loads. Here the block was just WRITTEN, so it is in L1/L2 and the fused
    // read is nearly free.
    // 4.79 -- GATE 20 DISPATCH ON OUTPUT SIZE.
    //
    // The two arms compute the SAME XXH64 over the same bytes, so this is pure
    // speed with no correctness trade. Which one wins is decided by CACHE
    // RESIDENCY of the decoded buffer, and it is monotonic in output size:
    //
    // ```text
    //   output    fused vs separate     null
    //    1 MiB          -0.35%          3.0%   <- no effect
    //    2 MiB          -2.62%          1.7%
    //    8 MiB          -4.58%          2.6%
    //   32 MiB          -5.27%          1.5%   <- 3.5x the null
    // ```
    //
    // Small output: the post-decode pass re-reads a cache-hot buffer, and fusing
    // only breaks the 128-byte stripes -- which is what the original rejection
    // measured, at ONE size. Large output: the separate pass is a COLD memory
    // traversal of the whole buffer, and fusing pays for itself several times
    // over (`zeros-32m` -1.13% -> -10.25% across the sweep).
    //
    // `content_size` is optional in the frame header. When it is absent we keep
    // today's behaviour rather than guess -- a wrong guess costs speed on every
    // small frame, and the arm is only worth ~5%.
    let fuse_by_size = match header.content_size {
        Some(n) => n >= ck_fuse_min() as u64,
        None => false,
    };
    let mut running = if header.checksum
        && !opts.force_ignore_checksum
        && (ck_stream_enabled() || fuse_by_size)
    {
        Some(crate::xxh64::Xxh64::new())
    } else {
        None
    };
    let mut hashed_to = start_len;

    {
        let _b = crate::prof::scope(crate::prof::Stage::DecodeBlocks);
        loop {
            let bh = parse_block_header(r)?;
            match bh.ty {
                BlockType::Raw => {
                    if bh.size > block_max {
                        return Err(Error::BlockTooLarge);
                    }
                    let payload = r.take(bh.size as usize)?;
                    out.extend_from_slice(payload);
                }
                BlockType::Rle => {
                    if bh.size > block_max {
                        return Err(Error::BlockTooLarge);
                    }
                    let b = r.u8()?;
                    let n = bh.size as usize;
                    out.resize(out.len() + n, b);
                }
                BlockType::Compressed => {
                    if bh.size > block_max {
                        return Err(Error::BlockTooLarge);
                    }
                    let payload = r.take(bh.size as usize)?;
                    decode_compressed_block(
                        payload,
                        out,
                        header.window_size,
                        block_max,
                        &mut block_state,
                        hist,
                        start_len,
                        0,
                    )?;
                }
            }
            if let Some(h) = running.as_mut() {
                // hash exactly the bytes this block produced, still cache-hot
                // D26 REFUTED, recorded: `get(..).ok_or` on this slice and
                // the two below drove this function's pads 2 -> 0 and measured
                // **+4** (575 -> 579). Fourth data point, same rule: the pad
                // transform pays on HOT sites with genuinely opaque indices
                // (D17 -44, D21 -31, D25 -14) and loses on cold or
                // near-provable ones (D19 +3, D23 +16, here +4).
                h.update(&out[hashed_to..]);
                hashed_to = out.len();
            }
            if bh.last {
                break;
            }
        }
    }

    if header.checksum {
        let _c = crate::prof::scope(crate::prof::Stage::DecodeChecksum);
        // The 4 bytes are consumed either way -- skipping the READ as well
        // would desync every following frame in a concatenated stream.
        let got = r.u32_le()?;
        if !opts.force_ignore_checksum {
            let computed = match running {
                // streamed per block above -- no second traversal
                Some(h) => {
                    debug_assert_eq!(hashed_to, out.len());
                    h.digest() as u32
                }
                None => content_checksum(&out[start_len..]),
            };
            if computed != got {
                return Err(Error::ChecksumMismatch);
            }
        }
    }

    if let Some(n) = header.content_size {
        // D30 REFUTED: D26's subtraction, isolated, still measured **+2**. So
        // D26's +4 was not caused by its two `get` calls -- this whole frame-
        // level site is simply one where the check is already cheap. Sixth and
        // final refutation in the pad class; the rule holds without exception.
        let produced = (out.len() - start_len) as u64;
        if produced != n {
            return Err(Error::ContentSizeMismatch);
        }
    }
    Ok(())
}

fn skip_blocks(r: &mut Reader<'_>, header: FrameHeader) -> Result<(), Error> {
    loop {
        let bh = parse_block_header(r)?;
        if bh.ty == BlockType::Compressed {
            // Size-find still walks the payload; decode is the unsupported part.
        }
        if matches!(bh.ty, BlockType::Raw | BlockType::Compressed)
            && bh.size > header.block_size_max()
        {
            return Err(Error::BlockTooLarge);
        }
        let _ = r.take(bh.payload_len() as usize)?;
        if bh.last {
            break;
        }
    }
    if header.checksum {
        let _ = r.u32_le()?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {

    /// `decompress_into` must equal `decompress`, byte for byte, on content
    /// that exercises RLE blocks, literals and matches alike.
    #[test]
    fn into_matches_decompress() {
        for src in [
            alloc::vec![0u8; 300_000],
            alloc::vec![7u8; 1],
            b"hello hello hello world world".to_vec(),
            (0..70_000u32).map(|i| (i % 251) as u8).collect(),
            alloc::vec![],
        ] {
            for lvl in [1, 3, 9, 19] {
                let f = crate::compress(&src, lvl).unwrap();
                let want = decompress(&f).unwrap();
                let mut got = Vec::new();
                let n = decompress_into(&mut got, &f).unwrap();
                assert_eq!(n, want.len(), "returned count, level {lvl}");
                assert_eq!(got, want, "level {lvl}, {} bytes", src.len());
                assert_eq!(want, src);
            }
        }
    }

    /// It APPENDS: pre-existing bytes survive, and the return value counts only
    /// what this call wrote. Back-references must not reach into the prefix.
    #[test]
    fn into_appends_without_disturbing_prefix() {
        let src: Vec<u8> = (0..40_000u32).map(|i| (i % 97) as u8).collect();
        let f = crate::compress(&src, 5).unwrap();
        let mut buf = b"PREFIX".to_vec();
        let n = decompress_into(&mut buf, &f).unwrap();
        assert_eq!(n, src.len());
        assert_eq!(&buf[..6], b"PREFIX");
        assert_eq!(&buf[6..], &src[..]);
    }

    /// SIBLING-PATH PARITY for `decompress_into`.
    ///
    /// `compressed.rs` decides `nodict = dict.is_empty() && frame_start == 0
    /// && frame_skipped == 0` once per block and runs a different sequence
    /// path on each side of it. Every other test here decodes into an EMPTY
    /// buffer, so they all take the `frame_start == 0` arm; the arm that the
    /// append contract actually selects was covered by exactly one case (a
    /// 6-byte prefix, one level, one content).
    ///
    /// This drives the other arm across word, block and window boundaries and
    /// compares byte-for-byte against the one-shot oracle. The failure this
    /// API's shape invites is a back-reference reaching past `frame_start`
    /// into the caller's own bytes, which changes the appended region -- so
    /// the oracle comparison catches it. The prefix is filled with a pattern
    /// the CONTENT also contains, so a leaked match cannot coincidentally
    /// land on the right bytes and hide.
    #[test]
    fn into_appends_correctly_at_every_prefix_length() {
        let contents: [Vec<u8>; 3] = [
            (0..90_000u32).map(|i| (i % 97) as u8).collect(),
            alloc::vec![0u8; 200_000],
            b"hello hello hello world world world".to_vec(),
        ];
        // Straddle the u64 word, the 64 KiB and 128 KiB block sizes, and the
        // window sizes small levels pick.
        const PREFILLS: &[usize] = &[
            0, 1, 7, 8, 9, 63, 64, 65, 4095, 4096, 65_535, 65_536, 131_072, 131_073,
        ];
        // A DETERMINISTIC RECEIPT that the body ran. A test that silently
        // stops entering its own loop still reports `ok`, and this one runs
        // fast enough (the fixtures are highly compressible) to look skipped.
        let mut cases = 0usize;
        let mut appended = 0usize;
        for (ci, src) in contents.iter().enumerate() {
            for lvl in [1, 5, 19] {
                let f = crate::compress(src, lvl).unwrap();
                let want = decompress(&f).unwrap();
                assert_eq!(&want, src, "oracle c{ci} L{lvl}");
                for &p in PREFILLS {
                    let prefix: Vec<u8> = (0..p).map(|i| (i % 97) as u8).collect();
                    let mut buf = prefix.clone();
                    let n = decompress_into(&mut buf, &f).unwrap();
                    assert_eq!(n, want.len(), "count c{ci} L{lvl} prefill {p}");
                    assert_eq!(buf.len(), p + want.len(), "len c{ci} L{lvl} prefill {p}");
                    assert_eq!(&buf[..p], &prefix[..], "prefix c{ci} L{lvl} prefill {p}");
                    assert_eq!(&buf[p..], &want[..], "appended c{ci} L{lvl} prefill {p}");
                    cases += 1;
                    appended += n;
                }
            }
        }
        assert_eq!(cases, 3 * 3 * PREFILLS.len(), "cases actually exercised");
        // 3 levels x 14 prefills x (90_000 + 200_000 + 35) bytes.
        assert_eq!(appended, 3 * PREFILLS.len() * (90_000 + 200_000 + 35));
    }

    /// Reusing one allocation across many frames -- the whole point of the API.
    #[test]
    fn into_reuse_across_frames_is_stable() {
        let a = crate::compress(&alloc::vec![0u8; 200_000], 1).unwrap();
        let b = crate::compress(b"second frame contents", 3).unwrap();
        let mut buf = Vec::new();
        for _ in 0..3 {
            buf.clear();
            assert_eq!(decompress_into(&mut buf, &a).unwrap(), 200_000);
            assert!(buf.iter().all(|&x| x == 0));
            buf.clear();
            assert_eq!(decompress_into(&mut buf, &b).unwrap(), 21);
            assert_eq!(&buf[..], b"second frame contents");
        }
    }

    /// `force_ignore_checksum` skips VERIFICATION but must still consume the
    /// 4 checksum bytes -- otherwise a following frame desyncs.
    #[test]
    fn force_ignore_checksum_skips_verification_not_parsing() {
        let src: Vec<u8> = (0..50_000u32).map(|i| (i % 131) as u8).collect();
        let f = crate::compress(&src, 3).unwrap();
        let skip = DecompressOptions {
            force_ignore_checksum: true,
            ..Default::default()
        };

        // Good frame decodes identically with and without verification.
        assert_eq!(decompress_with(&f, skip).unwrap(), src);
        assert_eq!(
            decompress_with(&f, DecompressOptions::default()).unwrap(),
            src
        );

        // Corrupt the stored checksum ONLY (last 4 bytes of the frame).
        let mut bad = f.clone();
        let n = bad.len();
        bad[n - 1] ^= 0xFF;
        // Default: rejected.
        assert!(matches!(
            decompress_with(&bad, DecompressOptions::default()),
            Err(Error::ChecksumMismatch)
        ));
        // Skipping: accepted, and the CONTENT is still correct.
        assert_eq!(decompress_with(&bad, skip).unwrap(), src);

        // Concatenated frames still parse -- proves the bytes were consumed.
        let mut two = f.clone();
        two.extend_from_slice(&f);
        let got = decompress_with(&two, skip).unwrap();
        assert_eq!(got.len(), src.len() * 2);
        assert_eq!(&got[..src.len()], &src[..]);
        assert_eq!(&got[src.len()..], &src[..]);
    }

    /// A failing decode must not be reported as success. The buffer is left
    /// unspecified-but-safe; only the error contract is guaranteed.
    #[test]
    fn into_propagates_errors() {
        let mut buf = Vec::new();
        assert!(decompress_into(&mut buf, b"").is_err());
        assert!(decompress_into(&mut buf, b"not a zstd frame").is_err());
        let f = crate::compress(b"payload here", 3).unwrap();
        assert!(decompress_into(&mut buf, &f[..f.len() / 2]).is_err());
    }

    use super::*;
    use crate::frame::{get_frame_header, FrameKind};

    // facebook/zstd v1.5.7 CLI, captured 2026-08-13.
    const EMPTY: &[u8] = &[
        0x28, 0xB5, 0x2F, 0xFD, 0x24, 0x00, 0x01, 0x00, 0x00, 0x99, 0xE9, 0xD8, 0x51,
    ];
    const EMPTY_NC: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x00, 0x01, 0x00, 0x00];
    const A: &[u8] = &[
        0x28, 0xB5, 0x2F, 0xFD, 0x24, 0x01, 0x09, 0x00, 0x00, 0x61, 0x5B, 0x6E, 0x8C, 0xA9,
    ];
    const A_NC: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x01, 0x09, 0x00, 0x00, 0x61];
    const A_NC_NCS: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD, 0x00, 0x00, 0x09, 0x00, 0x00, 0x61];
    const HI_NC: &[u8] = &[
        0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x02, 0x11, 0x00, 0x00, 0x68, 0x69,
    ];
    const HELLO: &[u8] = &[
        0x28, 0xB5, 0x2F, 0xFD, 0x24, 0x05, 0x29, 0x00, 0x00, 0x68, 0x65, 0x6C, 0x6C, 0x6F, 0xA3,
        0x6D, 0x9F, 0x88,
    ];
    // `zstd -3` on 16 zero bytes: compressed block -- decode-and-stop.
    const ZEROS16: &[u8] = &[
        0x28, 0xB5, 0x2F, 0xFD, 0x24, 0x10, 0x45, 0x00, 0x00, 0x10, 0x00, 0x00, 0x01, 0x00, 0x32,
        0xC0, 0x02, 0x32, 0x7C, 0x24, 0x16,
    ];

    #[test]
    fn c_empty() {
        assert_eq!(decompress(EMPTY).unwrap(), b"");
        assert_eq!(decompress(EMPTY_NC).unwrap(), b"");
        assert_eq!(content_size(EMPTY).unwrap(), Some(0));
        assert_eq!(find_frame_compressed_size(EMPTY).unwrap(), EMPTY.len());
    }

    #[test]
    fn c_raw_small() {
        assert_eq!(decompress(A).unwrap(), b"a");
        assert_eq!(decompress(A_NC).unwrap(), b"a");
        assert_eq!(decompress(A_NC_NCS).unwrap(), b"a");
        assert_eq!(decompress(HI_NC).unwrap(), b"hi");
        assert_eq!(decompress(HELLO).unwrap(), b"hello");
        assert_eq!(content_size(A_NC_NCS).unwrap(), None);
    }

    #[test]
    fn c_zeros16_compressed() {
        assert_eq!(decompress(ZEROS16).unwrap(), [0u8; 16]);
        match get_frame_header(ZEROS16).unwrap() {
            FrameKind::Zstd(h) => {
                assert_eq!(h.content_size, Some(16));
                assert!(h.checksum);
            }
            other => panic!("{other:?}"),
        }
        assert_eq!(find_frame_compressed_size(ZEROS16).unwrap(), ZEROS16.len());
    }

    #[test]
    fn rle_handcrafted() {
        // SS, no checksum, FCS=5, last RLE size=5, byte 'A'
        // block: last=1 type=1 size=5 -> 1 | 2 | 40 = 43 = 0x2B
        let src = [0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x05, 0x2B, 0x00, 0x00, b'A'];
        assert_eq!(decompress(&src).unwrap(), b"AAAAA");
    }

    #[test]
    fn skippable_then_zstd() {
        let mut src = vec![
            0x50, 0x2A, 0x4D, 0x18, 0x04, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF,
        ];
        src.extend_from_slice(A_NC);
        assert_eq!(decompress(&src).unwrap(), b"a");
        assert_eq!(content_size(&src).unwrap(), Some(1));
        assert_eq!(find_frame_compressed_size(&src).unwrap(), 12);
    }

    #[test]
    fn multi_frame() {
        let mut src = EMPTY_NC.to_vec();
        src.extend_from_slice(HELLO);
        assert_eq!(decompress(&src).unwrap(), b"hello");

        let mut ab = A_NC.to_vec();
        ab.extend_from_slice(HI_NC);
        assert_eq!(decompress(&ab).unwrap(), b"ahi");
    }

    #[test]
    fn checksum_mismatch() {
        let mut bad = A.to_vec();
        let n = bad.len();
        bad[n - 1] ^= 0xFF;
        assert_eq!(decompress(&bad).unwrap_err(), Error::ChecksumMismatch);
    }

    #[test]
    fn truncated() {
        assert_eq!(decompress(&A[..4]).unwrap_err(), Error::UnexpectedEof);
        assert_eq!(decompress(&[]).unwrap_err(), Error::UnexpectedEof);
    }

    #[test]
    fn dict_id_needed() {
        // SS, no checksum, dict_flag=1, dict id=7, FCS=0, empty raw last
        let src = [0x28, 0xB5, 0x2F, 0xFD, 0x21, 0x07, 0x00, 0x01, 0x00, 0x00];
        assert_eq!(
            decompress(&src).unwrap_err(),
            Error::DictionaryNeeded { id: 7 }
        );
    }

    #[test]
    fn reserved_bit() {
        let src = [0x28, 0xB5, 0x2F, 0xFD, 0x08];
        assert_eq!(get_frame_header(&src).unwrap_err(), Error::ReservedBitSet);
    }

    #[test]
    fn window_too_large() {
        // not SS, window desc exponent high enough for > 128 MiB
        // exponent=18 -> window_log=28 -> 256 MiB
        let desc = 18u8 << 3;
        let src = [0x28, 0xB5, 0x2F, 0xFD, 0x00, desc, 0x01, 0x00, 0x00];
        assert_eq!(decompress(&src).unwrap_err(), Error::WindowTooLarge);
    }

    #[test]
    fn trailing_garbage() {
        let mut src = A_NC.to_vec();
        src.push(0xFF);
        assert_eq!(decompress(&src).unwrap_err(), Error::TrailingBytes);
    }
}
