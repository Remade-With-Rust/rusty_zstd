//! RFC 8878 dictionaries: raw content and trained (`0xEC30A437`).

use crate::encode::RetainedTable;
use crate::error::Error;
use crate::fse::{self, FseTable};
use crate::huffman::{self, HuffCTable, HuffmanTable};
use crate::xxh64::content_checksum;
use alloc::sync::Arc;

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
    pub ll_d: FseTable,
    pub of_d: FseTable,
    pub ml_d: FseTable,
    // The ENCODE-side tables, already in the shared form a frame's
    // `EntropyState` retains them in, so seeding a frame from the dictionary
    // is four refcount bumps and no copy (see `seed_from_dict`).
    pub huff_c: alloc::sync::Arc<HuffCTable>,
    pub ll_c: alloc::sync::Arc<crate::encode::RetainedTable>,
    pub of_c: alloc::sync::Arc<crate::encode::RetainedTable>,
    pub ml_c: alloc::sync::Arc<crate::encode::RetainedTable>,
    pub reps: [u32; 3],
}

/// A zstd dictionary (raw bytes or trained with entropy tables).
///
/// Compressing many inputs against the SAME `Dictionary` (or clones of it) is
/// the fast path: from the second call on, the match-table state the
/// dictionary primes is kept with it and reused instead of being rebuilt from
/// the dictionary bytes on every call -- libzstd's `ZSTD_CDict`, without a
/// separate type to manage. Build the `Dictionary` once and keep it; a
/// `Dictionary` rebuilt from the same bytes for every call starts cold every
/// time. The output is byte-identical either way.
#[derive(Clone, Debug)]
pub struct Dictionary {
    id: u32,
    content: Vec<u8>,
    entropy: Option<DictEntropy>,
    /// The digested (primed) table states, shared by every clone. See
    /// `encode::DictDigests` for what is kept and its bound. `std` only: it
    /// needs a lock, and the working tables that mirror it are thread-local.
    #[cfg(feature = "std")]
    digests: alloc::sync::Arc<crate::encode::DictDigests>,
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
        Ok(Self::with_parts(0, src.to_vec(), None))
    }

    /// Raw-content dictionary (no entropy tables, Dictionary_ID 0).
    pub fn raw(content: impl Into<Vec<u8>>) -> Self {
        Self::with_parts(0, content.into(), None)
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
            #[cfg(feature = "std")]
            digests: Default::default(),
        }
    }

    /// The digest cache this dictionary (and every clone of it) owns.
    #[cfg(feature = "std")]
    pub(crate) fn digests(&self) -> &alloc::sync::Arc<crate::encode::DictDigests> {
        &self.digests
    }
}

fn parse_trained(src: &[u8]) -> Result<Dictionary, Error> {
    if src.len() < 8 {
        return Err(Error::Corruption);
    }
    // C2: same treatment as `parse_seek_table`. Every `src[i]` here was its
    // own bounds test and panic pad -- seventeen of them across this function
    // -- because nothing in the source relates the indices to `src.len()`.
    // Fixed-size arrays through `try_into` prove all four (or twelve) elements
    // from ONE check, in safe Rust.
    let idb: [u8; 4] = src[4..8].try_into().map_err(|_| Error::Corruption)?;
    let id = u32::from_le_bytes(idb);
    let mut pos = 8usize;
    let (huff_d, hn) = huffman::read_table(None, &src[pos..])?;
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
    let rb: [u8; 12] = src[pos..pos + 12]
        .try_into()
        .map_err(|_| Error::Corruption)?;
    let r0 = u32::from_le_bytes([rb[0], rb[1], rb[2], rb[3]]);
    let r1 = u32::from_le_bytes([rb[4], rb[5], rb[6], rb[7]]);
    let r2 = u32::from_le_bytes([rb[8], rb[9], rb[10], rb[11]]);
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
            ll_d,
            of_d,
            ml_d,
            huff_c: Arc::new(huff_c),
            ll_c: Arc::new(RetainedTable::Own(ll_c)),
            of_c: Arc::new(RetainedTable::Own(of_c)),
            ml_c: Arc::new(RetainedTable::Own(ml_c)),
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
    // C7: ten `extend_from_slice` sites became seven, the `append_seek_table`
    // treatment (C5). Each site inlines `Vec`'s capacity test and grow path,
    // so the SITE COUNT is the code size -- the five variable-length slices
    // must stay separate, but the magic+id pair and the three reps are fixed
    // width and stage into arrays. One `reserve` up front removes the growth
    // from the rest. Byte-for-byte identical dictionary.
    let mut out = Vec::with_capacity(
        8 + huff_tree.len()
            + of_ncount.len()
            + ml_ncount.len()
            + ll_ncount.len()
            + 12
            + content.len(),
    );
    let mut hdr = [0u8; 8];
    hdr[0..4].copy_from_slice(&MAGIC_DICTIONARY.to_le_bytes());
    hdr[4..8].copy_from_slice(&id.to_le_bytes());
    out.extend_from_slice(&hdr);
    out.extend_from_slice(huff_tree);
    out.extend_from_slice(of_ncount);
    out.extend_from_slice(ml_ncount);
    out.extend_from_slice(ll_ncount);
    let mut rb = [0u8; 12];
    rb[0..4].copy_from_slice(&reps[0].to_le_bytes());
    rb[4..8].copy_from_slice(&reps[1].to_le_bytes());
    rb[8..12].copy_from_slice(&reps[2].to_le_bytes());
    out.extend_from_slice(&rb);
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
