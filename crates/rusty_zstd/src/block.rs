//! RFC 8878 block header.

use crate::error::Error;
use crate::reader::Reader;

/// Block_Type (2 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BlockType {
    /// Uncompressed literals.
    Raw,
    /// One byte repeated Block_Size times.
    Rle,
    /// Literals + sequences (FSE / Huffman).
    Compressed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BlockHeader {
    pub last: bool,
    pub ty: BlockType,
    /// Raw/Compressed: payload bytes. RLE: regenerated size (payload is 1 byte).
    pub size: u32,
}

pub(crate) fn parse_block_header(r: &mut Reader<'_>) -> Result<BlockHeader, Error> {
    let n = r.u24_le()?;
    let last = (n & 1) != 0;
    let ty = match (n >> 1) & 3 {
        0 => BlockType::Raw,
        1 => BlockType::Rle,
        2 => BlockType::Compressed,
        _ => return Err(Error::ReservedBlockType),
    };
    let size = n >> 3;
    Ok(BlockHeader { last, ty, size })
}

impl BlockHeader {
    pub(crate) fn payload_len(self) -> u32 {
        match self.ty {
            BlockType::Rle => 1,
            BlockType::Raw | BlockType::Compressed => self.size,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::reader::Reader;

    #[test]
    fn empty_raw_last() {
        let mut r = Reader::new(&[0x01, 0x00, 0x00]);
        let h = parse_block_header(&mut r).unwrap();
        assert!(h.last);
        assert_eq!(h.ty, BlockType::Raw);
        assert_eq!(h.size, 0);
    }

    #[test]
    fn raw_one_byte() {
        // last=1, type=raw, size=1 -> 1 | (1<<3) = 9
        let mut r = Reader::new(&[0x09, 0x00, 0x00]);
        let h = parse_block_header(&mut r).unwrap();
        assert_eq!(h.size, 1);
        assert_eq!(h.ty, BlockType::Raw);
    }

    #[test]
    fn reserved_type() {
        // last=1, type=3, size=0 -> 1 | (3<<1) = 7
        let mut r = Reader::new(&[0x07, 0x00, 0x00]);
        assert_eq!(
            parse_block_header(&mut r).unwrap_err(),
            Error::ReservedBlockType
        );
    }
}
