//! RFC 8878 dictionaries: raw content and trained (`0xEC30A437`).

use crate::error::Error;
use crate::fse::{self, FseCTable, FseTable};
use crate::huffman::{self, HuffCTable, HuffmanTable};
use crate::xxh64::content_checksum;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// Trained dictionary magic (little-endian `0xEC30A437`).
pub const MAGIC_DICTIONARY: u32 = 0xEC30_A437;

/// Minimum public Dictionary_ID (RFC 8878 reserved below this).
pub const DICT_ID_PUBLIC_MIN: u32 = 32768;
/// Public Dictionary_IDs are below 2^31.
pub const DICT_ID_PUBLIC_MAX: u32 = 0x8000_0000;

/// Entropy tables carried in a trained dictionary.
#[derive(Clone, Debug)]
pub(crate) struct DictEntropy {
    pub huff_d: HuffmanTable,
    pub huff_c: HuffCTable,
    pub ll_d: FseTable,
    pub of_d: FseTable,
    pub ml_d: FseTable,
    pub ll_c: FseCTable,
    pub of_c: FseCTable,
    pub ml_c: FseCTable,
    pub reps: [u32; 3],
}

/// A zstd dictionary (raw bytes or trained with entropy tables).
#[derive(Clone, Debug)]
pub struct Dictionary {
    id: u32,
    content: Vec<u8>,
    entropy: Option<DictEntropy>,
}

impl Dictionary {
    /// Parse `src` as a trained dictionary if it has the magic, otherwise raw content.
    pub fn from_bytes(src: &[u8]) -> Result<Self, Error> {
        if src.len() >= 8 {
            let magic = u32::from_le_bytes([src[0], src[1], src[2], src[3]]);
            if magic == MAGIC_DICTIONARY {
                return parse_trained(src);
            }
        }
        Ok(Self {
            id: 0,
            content: src.to_vec(),
            entropy: None,
        })
    }

    /// Raw-content dictionary (no entropy tables, Dictionary_ID 0).
    pub fn raw(content: impl Into<Vec<u8>>) -> Self {
        Self {
            id: 0,
            content: content.into(),
            entropy: None,
        }
    }

    /// Dictionary_ID (0 for raw content).
    pub fn id(&self) -> u32 {
        self.id
    }

    /// Bytes used as a match prefix.
    pub fn content(&self) -> &[u8] {
        &self.content
    }

    pub(crate) fn entropy(&self) -> Option<&DictEntropy> {
        self.entropy.as_ref()
    }

    pub(crate) fn with_parts(id: u32, content: Vec<u8>, entropy: Option<DictEntropy>) -> Self {
        Self {
            id,
            content,
            entropy,
        }
    }
}

fn parse_trained(src: &[u8]) -> Result<Dictionary, Error> {
    if src.len() < 8 {
        return Err(Error::Corruption);
    }
    let id = u32::from_le_bytes([src[4], src[5], src[6], src[7]]);
    let mut pos = 8usize;
    let (huff_d, hn) = huffman::read_table(&src[pos..])?;
    let (huff_c, _) = huffman::read_ctable(&src[pos..])?;
    pos += hn;
    let (of_d, of_c, n) = fse::read_ncount_ctable(&src[pos..], 31, 8)?;
    pos += n;
    let (ml_d, ml_c, n) = fse::read_ncount_ctable(&src[pos..], 52, 9)?;
    pos += n;
    let (ll_d, ll_c, n) = fse::read_ncount_ctable(&src[pos..], 35, 9)?;
    pos += n;
    if pos + 12 > src.len() {
        return Err(Error::Corruption);
    }
    let r0 = u32::from_le_bytes([src[pos], src[pos + 1], src[pos + 2], src[pos + 3]]);
    let r1 = u32::from_le_bytes([src[pos + 4], src[pos + 5], src[pos + 6], src[pos + 7]]);
    let r2 = u32::from_le_bytes([src[pos + 8], src[pos + 9], src[pos + 10], src[pos + 11]]);
    pos += 12;
    let content = src[pos..].to_vec();
    let clen = content.len() as u32;
    if r0 == 0 || r1 == 0 || r2 == 0 || r0 > clen || r1 > clen || r2 > clen {
        return Err(Error::Corruption);
    }
    Ok(Dictionary::with_parts(
        id,
        content,
        Some(DictEntropy {
            huff_d,
            huff_c,
            ll_d,
            of_d,
            ml_d,
            ll_c,
            of_c,
            ml_c,
            reps: [r0, r1, r2],
        }),
    ))
}

/// Pick a public Dictionary_ID (RFC reserved ranges avoided unless `forced`).
pub fn public_dict_id(content: &[u8], forced: Option<u32>) -> u32 {
    if let Some(id) = forced {
        return id;
    }
    let mut id = content_checksum(content);
    if id < DICT_ID_PUBLIC_MIN {
        id = id.saturating_add(DICT_ID_PUBLIC_MIN);
    }
    if id >= DICT_ID_PUBLIC_MAX {
        id &= DICT_ID_PUBLIC_MAX - 1;
        if id < DICT_ID_PUBLIC_MIN {
            id += DICT_ID_PUBLIC_MIN;
        }
    }
    if id == 0 {
        id = DICT_ID_PUBLIC_MIN;
    }
    id
}

/// Trainer-facing: build trained bytes from NCount headers + Huffman tree + content.
#[cfg(all(feature = "alloc", feature = "std"))]
pub(crate) fn write_trained_parts(
    id: u32,
    huff_tree: &[u8],
    of_ncount: &[u8],
    ml_ncount: &[u8],
    ll_ncount: &[u8],
    reps: [u32; 3],
    content: &[u8],
) -> Vec<u8> {
    let mut out = Vec::new();
    out.extend_from_slice(&MAGIC_DICTIONARY.to_le_bytes());
    out.extend_from_slice(&id.to_le_bytes());
    out.extend_from_slice(huff_tree);
    out.extend_from_slice(of_ncount);
    out.extend_from_slice(ml_ncount);
    out.extend_from_slice(ll_ncount);
    out.extend_from_slice(&reps[0].to_le_bytes());
    out.extend_from_slice(&reps[1].to_le_bytes());
    out.extend_from_slice(&reps[2].to_le_bytes());
    out.extend_from_slice(content);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn raw_has_id_zero() {
        let d = Dictionary::from_bytes(b"hello dict").unwrap();
        assert_eq!(d.id(), 0);
        assert_eq!(d.content(), b"hello dict");
        assert!(d.entropy().is_none());
    }

    #[test]
    fn public_id_avoids_reserved() {
        let id = public_dict_id(b"abc", None);
        assert!(id >= DICT_ID_PUBLIC_MIN);
        assert!(id < DICT_ID_PUBLIC_MAX);
    }
}
