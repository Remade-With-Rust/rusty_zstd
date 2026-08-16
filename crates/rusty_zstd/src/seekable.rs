//! Seekable zstd: independent frames + skippable seek table (contrib spec 0.1.0).

use crate::encode::{encode_oneshot, AdvancedOptions};
use crate::error::Error;
use crate::frame::is_skippable_magic;
use crate::params::CompressionParameters;
use crate::xxh64::content_checksum;
use alloc::vec::Vec;

/// Skippable magic used by the seek table frame (`0x184D2A5E`).
pub const SEEKABLE_SKIPPABLE_MAGIC: u32 = 0x184D_2A5E;
/// Seek table footer magic (`0x8F92EAB1`).
pub const SEEKABLE_MAGIC: u32 = 0x8F92_EAB1;
/// Default independent-frame size (`--seekable`).
pub const DEFAULT_FRAME_SIZE: usize = 2 * 1024 * 1024;

/// One seek-table row.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SeekEntry {
    /// Compressed size of this frame (including header/checksum).
    pub compressed_size: u32,
    /// Uncompressed size (`0` for skippable / empty).
    pub decompressed_size: u32,
    /// XXH64 low 32 of uncompressed data, if the table stores checksums.
    pub checksum: Option<u32>,
}

/// Parsed seek table (does not include the skippable header itself).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeekTable {
    /// Frames in file order (not including the table frame).
    pub entries: Vec<SeekEntry>,
}

impl SeekTable {
    /// Uncompressed offset of frame `i`.
    pub fn uncompressed_offset(&self, i: usize) -> u64 {
        self.entries
            .iter()
            .take(i)
            .map(|e| u64::from(e.decompressed_size))
            .sum()
    }

    /// Compressed offset of frame `i` from the start of the file.
    pub fn compressed_offset(&self, i: usize) -> u64 {
        self.entries
            .iter()
            .take(i)
            .map(|e| u64::from(e.compressed_size))
            .sum()
    }

    /// Total uncompressed size.
    pub fn uncompressed_size(&self) -> u64 {
        self.entries
            .iter()
            .map(|e| u64::from(e.decompressed_size))
            .sum()
    }
}

/// Compress `src` as independent frames plus a trailing seek table.
pub fn compress_seekable(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    max_frame_size: usize,
) -> Result<Vec<u8>, Error> {
    compress_seekable_adv(
        src,
        params,
        checksum,
        max_frame_size,
        AdvancedOptions::default(),
    )
}

/// [`compress_seekable`] with LDM / rsyncable / target-cblock knobs.
pub fn compress_seekable_adv(
    src: &[u8],
    params: CompressionParameters,
    checksum: bool,
    max_frame_size: usize,
    adv: AdvancedOptions,
) -> Result<Vec<u8>, Error> {
    let max_frame = max_frame_size.max(1);
    let mut out = Vec::new();
    let mut entries: Vec<SeekEntry> = Vec::new();
    if src.is_empty() {
        let zst = encode_oneshot(src, params, checksum, Some(0), None, &[], true, adv)?;
        entries.push(SeekEntry {
            compressed_size: zst.len() as u32,
            decompressed_size: 0,
            checksum: if checksum {
                Some(content_checksum(src))
            } else {
                None
            },
        });
        out.extend_from_slice(&zst);
        append_seek_table(&mut out, &entries, checksum);
        return Ok(out);
    }
    let mut off = 0usize;
    while off < src.len() {
        let end = (off + max_frame).min(src.len());
        let chunk = &src[off..end];
        let zst = encode_oneshot(
            chunk,
            params,
            checksum,
            Some(chunk.len() as u64),
            None,
            &[],
            true,
            adv,
        )?;
        let csize = u32::try_from(zst.len()).map_err(|_| Error::ContentSizeTooLarge)?;
        let dsize = u32::try_from(chunk.len()).map_err(|_| Error::ContentSizeTooLarge)?;
        entries.push(SeekEntry {
            compressed_size: csize,
            decompressed_size: dsize,
            checksum: if checksum {
                Some(content_checksum(chunk))
            } else {
                None
            },
        });
        out.extend_from_slice(&zst);
        off = end;
    }
    append_seek_table(&mut out, &entries, checksum);
    Ok(out)
}

/// Parse a seek table from the end of a seekable blob.
pub fn parse_seek_table(src: &[u8]) -> Result<SeekTable, Error> {
    if src.len() < 17 {
        return Err(Error::Corruption);
    }
    let n = src.len();
    let magic = u32::from_le_bytes([src[n - 4], src[n - 3], src[n - 2], src[n - 1]]);
    if magic != SEEKABLE_MAGIC {
        return Err(Error::BadMagic);
    }
    let descriptor = src[n - 5];
    if descriptor & 0x7C != 0 {
        return Err(Error::ReservedBitSet);
    }
    let has_sum = descriptor & 0x80 != 0;
    let num = u32::from_le_bytes([src[n - 9], src[n - 8], src[n - 7], src[n - 6]]) as usize;
    let entry_size = if has_sum { 12 } else { 8 };
    let payload = num
        .checked_mul(entry_size)
        .and_then(|e| e.checked_add(9))
        .ok_or(Error::Corruption)?;
    if payload > u32::MAX as usize {
        return Err(Error::Corruption);
    }
    let skippable_len = 8usize.saturating_add(payload);
    if n < skippable_len {
        return Err(Error::Corruption);
    }
    let start = n - skippable_len;
    let sm = u32::from_le_bytes([src[start], src[start + 1], src[start + 2], src[start + 3]]);
    if sm != SEEKABLE_SKIPPABLE_MAGIC && !is_skippable_magic(sm) {
        return Err(Error::BadMagic);
    }
    let stated = u32::from_le_bytes([
        src[start + 4],
        src[start + 5],
        src[start + 6],
        src[start + 7],
    ]) as usize;
    if stated != payload {
        return Err(Error::Corruption);
    }
    let mut entries = Vec::with_capacity(num);
    let mut p = start + 8;
    for _ in 0..num {
        let csize = u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
        let dsize = u32::from_le_bytes([src[p + 4], src[p + 5], src[p + 6], src[p + 7]]);
        p += 8;
        let checksum = if has_sum {
            let c = u32::from_le_bytes([src[p], src[p + 1], src[p + 2], src[p + 3]]);
            p += 4;
            Some(c)
        } else {
            None
        };
        entries.push(SeekEntry {
            compressed_size: csize,
            decompressed_size: dsize,
            checksum,
        });
    }
    Ok(SeekTable { entries })
}

/// Decompress the independent frame covering uncompressed `offset` (one frame).
pub fn decompress_frame_at(src: &[u8], table: &SeekTable, offset: u64) -> Result<Vec<u8>, Error> {
    let mut uoff = 0u64;
    let mut coff = 0usize;
    for e in &table.entries {
        let next = uoff + u64::from(e.decompressed_size);
        if offset < next {
            let end = coff
                .checked_add(e.compressed_size as usize)
                .ok_or(Error::Corruption)?;
            if end > src.len() {
                return Err(Error::UnexpectedEof);
            }
            return crate::decompress(&src[coff..end]);
        }
        coff = coff
            .checked_add(e.compressed_size as usize)
            .ok_or(Error::Corruption)?;
        uoff = next;
    }
    Err(Error::UnexpectedEof)
}

fn append_seek_table(out: &mut Vec<u8>, entries: &[SeekEntry], checksums: bool) {
    let entry_size = if checksums { 12 } else { 8 };
    let payload = entries.len() * entry_size + 9;
    out.extend_from_slice(&SEEKABLE_SKIPPABLE_MAGIC.to_le_bytes());
    out.extend_from_slice(&(payload as u32).to_le_bytes());
    for e in entries {
        out.extend_from_slice(&e.compressed_size.to_le_bytes());
        out.extend_from_slice(&e.decompressed_size.to_le_bytes());
        if checksums {
            out.extend_from_slice(&e.checksum.unwrap_or(0).to_le_bytes());
        }
    }
    out.extend_from_slice(&(entries.len() as u32).to_le_bytes());
    let desc = if checksums { 0x80u8 } else { 0 };
    out.push(desc);
    out.extend_from_slice(&SEEKABLE_MAGIC.to_le_bytes());
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{compression_params, decompress};

    #[test]
    fn seekable_roundtrip_and_table() {
        let src = b"seekable rusty_zstd frame one. seekable rusty_zstd frame two. extra".repeat(20);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_seekable(&src, params, true, 64).expect("seek compress");
        assert_eq!(decompress(&zst).expect("concat decode"), src);
        let table = parse_seek_table(&zst).expect("table");
        assert!(table.entries.len() >= 2);
        assert_eq!(table.uncompressed_size(), src.len() as u64);
        let last = table.entries.last().copied().unwrap();
        assert!(last.checksum.is_some());
        let piece = decompress_frame_at(&zst, &table, 0).expect("frame 0");
        assert_eq!(piece, src[..piece.len()]);
        let mid = table.uncompressed_offset(1);
        let piece1 = decompress_frame_at(&zst, &table, mid).expect("frame 1");
        assert_eq!(piece1.len(), table.entries[1].decompressed_size as usize);
        assert_eq!(piece1, src[mid as usize..mid as usize + piece1.len()]);
        let mut acc = Vec::new();
        for i in 0..table.entries.len() {
            let o = table.uncompressed_offset(i);
            acc.extend_from_slice(&decompress_frame_at(&zst, &table, o).expect("frame"));
        }
        assert_eq!(acc, src.as_slice());
        let mut reserved = zst.clone();
        let n = reserved.len();
        reserved[n - 5] |= 0x04;
        assert_eq!(
            parse_seek_table(&reserved).unwrap_err(),
            Error::ReservedBitSet
        );
    }

    #[test]
    fn seekable_without_checksums() {
        let src = b"no checksum seek frames. ".repeat(40);
        let params = compression_params(1, Some(src.len() as u64)).unwrap();
        let zst = compress_seekable(&src, params, false, 48).expect("seek");
        assert_eq!(decompress(&zst).unwrap(), src);
        let table = parse_seek_table(&zst).unwrap();
        assert!(table.entries.len() >= 2);
        assert!(table.entries.iter().all(|e| e.checksum.is_none()));
    }
}
