//! RFC 8878 frame header and skippable frames.

use crate::error::Error;
use crate::reader::Reader;

/// Zstandard frame magic (little-endian `0xFD2FB528`).
pub const MAGIC: u32 = 0xFD2F_B528;
/// First skippable magic (`0x184D2A50`).
pub const MAGIC_SKIPPABLE_MIN: u32 = 0x184D_2A50;
/// Last skippable magic (`0x184D2A5F`).
pub const MAGIC_SKIPPABLE_MAX: u32 = 0x184D_2A5F;

/// RFC 8878 / libzstd `ZSTD_BLOCKSIZE_MAX`.
pub const BLOCKSIZE_MAX: u32 = 128 * 1024;
/// Default decoder window cap (CLI `-M` default): 128 MiB.
pub const DEFAULT_WINDOW_MAX: u64 = 128 * 1024 * 1024;

/// Parsed Zstandard frame header (not skippable).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameHeader {
    /// Back-reference window in bytes.
    pub window_size: u64,
    /// Dictionary_ID if present.
    pub dict_id: Option<u32>,
    /// Frame_Content_Size if present.
    pub content_size: Option<u64>,
    /// Content_Checksum_Flag.
    pub checksum: bool,
    /// Single_Segment_Flag.
    pub single_segment: bool,
}

impl FrameHeader {
    /// Maximum block size for this frame: min(window, 128 KiB).
    pub fn block_size_max(self) -> u32 {
        let w = self.window_size.min(u64::from(BLOCKSIZE_MAX));
        w as u32
    }
}

/// First frame at `src`: a Zstd frame or a skippable frame.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameKind {
    /// RFC 8878 Zstandard frame.
    Zstd(FrameHeader),
    /// Skippable frame (`0x184D2A5X`).
    Skippable {
        /// Full 4-byte magic.
        magic: u32,
        /// User_Data length.
        user_data_size: u32,
    },
}

pub(crate) fn is_skippable_magic(m: u32) -> bool {
    (MAGIC_SKIPPABLE_MIN..=MAGIC_SKIPPABLE_MAX).contains(&m)
}

/// Parse the first frame header (skippable or zstd). Does not consume blocks.
pub fn get_frame_header(src: &[u8]) -> Result<FrameKind, Error> {
    let mut r = Reader::new(src);
    parse_kind(&mut r)
}

pub(crate) fn parse_kind(r: &mut Reader<'_>) -> Result<FrameKind, Error> {
    let magic = r.u32_le()?;
    if magic == MAGIC {
        Ok(FrameKind::Zstd(parse_zstd_header(r)?))
    } else if is_skippable_magic(magic) {
        let user_data_size = r.u32_le()?;
        Ok(FrameKind::Skippable {
            magic,
            user_data_size,
        })
    } else {
        Err(Error::BadMagic)
    }
}

pub(crate) fn parse_zstd_header(r: &mut Reader<'_>) -> Result<FrameHeader, Error> {
    let desc = r.u8()?;
    let fcs_flag = desc >> 6;
    let single_segment = (desc & 0x20) != 0;
    let unused = (desc & 0x10) != 0;
    let reserved = (desc & 0x08) != 0;
    let checksum = (desc & 0x04) != 0;
    let dict_flag = desc & 0x03;

    if reserved {
        return Err(Error::ReservedBitSet);
    }
    if unused {
        return Err(Error::UnusedBitSet);
    }

    let window_desc = if single_segment { None } else { Some(r.u8()?) };

    let dict_id = match dict_flag {
        0 => None,
        1 => Some(u32::from(r.u8()?)),
        2 => Some(u32::from(r.u16_le()?)),
        3 => Some(r.u32_le()?),
        _ => return Err(Error::BadMagic),
    };

    let content_size = match fcs_flag {
        0 if single_segment => Some(u64::from(r.u8()?)),
        0 => None,
        1 => Some(u64::from(r.u16_le()?) + 256),
        2 => Some(u64::from(r.u32_le()?)),
        3 => Some(r.u64_le()?),
        _ => return Err(Error::BadMagic),
    };

    let window_size = if single_segment {
        content_size.unwrap_or(0)
    } else {
        window_size_from_desc(window_desc.unwrap_or(0))?
    };

    Ok(FrameHeader {
        window_size,
        dict_id,
        content_size,
        checksum,
        single_segment,
    })
}

fn window_size_from_desc(desc: u8) -> Result<u64, Error> {
    let exponent = u32::from(desc >> 3);
    let mantissa = u32::from(desc & 7);
    let window_log = 10u32.saturating_add(exponent);
    if window_log >= 64 {
        return Err(Error::WindowTooLarge);
    }
    let base = 1u64 << window_log;
    let extra = u64::from(mantissa) << (window_log.saturating_sub(3));
    Ok(base.saturating_add(extra))
}

#[cfg(test)]
mod tests {
    use super::*;

    // facebook/zstd v1.5.7 `echo -n a | zstd -3 --no-check`
    const A_NC: &[u8] = &[0x28, 0xB5, 0x2F, 0xFD, 0x20, 0x01, 0x09, 0x00, 0x00, 0x61];

    #[test]
    fn parses_c_a_no_check() {
        match get_frame_header(A_NC).unwrap() {
            FrameKind::Zstd(h) => {
                assert!(h.single_segment);
                assert!(!h.checksum);
                assert_eq!(h.content_size, Some(1));
                assert_eq!(h.window_size, 1);
                assert_eq!(h.dict_id, None);
                assert_eq!(h.block_size_max(), 1);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn skippable_header() {
        let src = [
            0x50, 0x2A, 0x4D, 0x18, 0x04, 0x00, 0x00, 0x00, 0xDE, 0xAD, 0xBE, 0xEF,
        ];
        match get_frame_header(&src).unwrap() {
            FrameKind::Skippable {
                magic,
                user_data_size,
            } => {
                assert_eq!(magic, 0x184D_2A50);
                assert_eq!(user_data_size, 4);
            }
            other => panic!("unexpected {other:?}"),
        }
    }

    #[test]
    fn bad_magic() {
        assert_eq!(
            get_frame_header(&[0, 1, 2, 3]).unwrap_err(),
            Error::BadMagic
        );
    }
}
