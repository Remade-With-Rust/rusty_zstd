//! Bounded little-endian reader over a byte slice. No panics on short input.

use crate::error::Error;

pub(crate) struct Reader<'a> {
    data: &'a [u8],
    pos: usize,
}

impl<'a> Reader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self { data, pos: 0 }
    }

    pub(crate) fn pos(&self) -> usize {
        self.pos
    }

    pub(crate) fn remaining(&self) -> usize {
        self.data.len().saturating_sub(self.pos)
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.remaining() == 0
    }

    pub(crate) fn peek_u32_le(&self) -> Result<u32, Error> {
        // T4: `get(a..b)` yields a slice of statically UNKNOWN length, so
        // indexing it four times cost four bounds checks -- 4 of the codec
        // path's remaining panic sites, all in `decompress_into_history`.
        // Converting to a fixed-size array states the length instead, and needs
        // no unsafe: the conversion itself proves it.
        let s: [u8; 4] = self
            .data
            .get(self.pos..self.pos.saturating_add(4))
            .ok_or(Error::UnexpectedEof)?
            .try_into()
            .map_err(|_| Error::UnexpectedEof)?;
        Ok(u32::from_le_bytes(s))
    }

    pub(crate) fn u8(&mut self) -> Result<u8, Error> {
        let b = *self.data.get(self.pos).ok_or(Error::UnexpectedEof)?;
        self.pos += 1;
        Ok(b)
    }

    pub(crate) fn take(&mut self, n: usize) -> Result<&'a [u8], Error> {
        let end = self.pos.checked_add(n).ok_or(Error::UnexpectedEof)?;
        let s = self.data.get(self.pos..end).ok_or(Error::UnexpectedEof)?;
        self.pos = end;
        Ok(s)
    }

    pub(crate) fn u16_le(&mut self) -> Result<u16, Error> {
        let s = self.take(2)?;
        Ok(u16::from_le_bytes([s[0], s[1]]))
    }

    pub(crate) fn u32_le(&mut self) -> Result<u32, Error> {
        let s = self.take(4)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    pub(crate) fn u64_le(&mut self) -> Result<u64, Error> {
        let s = self.take(8)?;
        Ok(u64::from_le_bytes([
            s[0], s[1], s[2], s[3], s[4], s[5], s[6], s[7],
        ]))
    }

    /// 3-byte little-endian integer (block header).
    pub(crate) fn u24_le(&mut self) -> Result<u32, Error> {
        let s = self.take(3)?;
        Ok(u32::from_le_bytes([s[0], s[1], s[2], 0]))
    }
}
