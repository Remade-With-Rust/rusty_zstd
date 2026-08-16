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
}

impl Default for DecompressOptions {
    fn default() -> Self {
        Self {
            window_max: DEFAULT_WINDOW_MAX,
        }
    }
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

#[cfg(feature = "alloc")]
fn decompress_with_history(
    src: &[u8],
    opts: DecompressOptions,
    dict: Option<&Dictionary>,
    prefix: &[u8],
) -> Result<Vec<u8>, Error> {
    if src.is_empty() {
        return Err(Error::UnexpectedEof);
    }
    let hist = dict.map(Dictionary::content).unwrap_or(prefix);
    let mut r = Reader::new(src);
    let mut out = Vec::new();
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
                decode_zstd_frame(&mut r, header, opts, dict, hist, &mut out)?;
                saw_zstd = true;
            }
        }
    }
    if !saw_zstd {
        return Err(Error::UnexpectedEof);
    }
    Ok(out)
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
            if bh.last {
                break;
            }
        }
    }

    if header.checksum {
        let _c = crate::prof::scope(crate::prof::Stage::DecodeChecksum);
        let got = r.u32_le()?;
        let produced = &out[start_len..];
        if content_checksum(produced) != got {
            return Err(Error::ChecksumMismatch);
        }
    }

    if let Some(n) = header.content_size {
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
