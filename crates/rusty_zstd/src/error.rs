//! Codec errors. Every public path returns one of these -- no panics.

/// Codec error. Stable kinds only.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Error {
    /// This entry point is not implemented yet.
    Unimplemented,
    /// `level` is outside [`crate::MIN_CLEVEL`]..=[`crate::MAX_CLEVEL`].
    InvalidLevel,
    /// Input ended in the middle of a frame or block.
    UnexpectedEof,
    /// First four bytes are not zstd or skippable magic.
    BadMagic,
    /// Frame_Header_Descriptor reserved bit (bit 3) is set.
    ReservedBitSet,
    /// Frame_Header_Descriptor unused bit (bit 4) is set.
    UnusedBitSet,
    /// Block_Type 3 (reserved).
    ReservedBlockType,
    /// Block regenerated or payload size exceeds min(window, 128 KiB).
    BlockTooLarge,
    /// Required window is above the decoder cap (default 128 MiB).
    WindowTooLarge,
    /// Regenerated size does not match Frame_Content_Size.
    ContentSizeMismatch,
    /// Claimed content size cannot be allocated.
    ContentSizeTooLarge,
    /// XXH64 content checksum did not match.
    ChecksumMismatch,
    /// Frame names a dictionary this decoder was not given.
    DictionaryNeeded {
        /// Dictionary_ID from the frame header.
        id: u32,
    },
    /// Frame Dictionary_ID does not match the loaded dictionary.
    DictionaryMismatch {
        /// Dictionary_ID from the frame header.
        frame: u32,
        /// Dictionary_ID of the loaded dictionary.
        loaded: u32,
    },
    /// Bytes remain after the last frame that are not a new frame.
    TrailingBytes,
    /// Malformed entropy tables, sequences, or offsets.
    Corruption,
}

impl Error {
    /// Stable kind name for logs and the C ABI later.
    pub fn kind(self) -> &'static str {
        match self {
            Error::Unimplemented => "unimplemented",
            Error::InvalidLevel => "invalid_level",
            Error::UnexpectedEof => "unexpected_eof",
            Error::BadMagic => "bad_magic",
            Error::ReservedBitSet => "reserved_bit",
            Error::UnusedBitSet => "unused_bit",
            Error::ReservedBlockType => "reserved_block_type",
            Error::BlockTooLarge => "block_too_large",
            Error::WindowTooLarge => "window_too_large",
            Error::ContentSizeMismatch => "content_size_mismatch",
            Error::ContentSizeTooLarge => "content_size_too_large",
            Error::ChecksumMismatch => "checksum_mismatch",
            Error::DictionaryNeeded { .. } => "dictionary_needed",
            Error::DictionaryMismatch { .. } => "dictionary_mismatch",
            Error::TrailingBytes => "trailing_bytes",
            Error::Corruption => "corruption",
        }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Unimplemented => {
                f.write_str("rusty_zstd: not implemented (see docs/plans/rusty-zstd-mission.md)")
            }
            Error::InvalidLevel => {
                f.write_str("rusty_zstd: compression level out of range (-7..=22)")
            }
            Error::UnexpectedEof => f.write_str("rusty_zstd: unexpected end of input"),
            Error::BadMagic => f.write_str("rusty_zstd: not a zstd frame"),
            Error::ReservedBitSet => f.write_str("rusty_zstd: reserved bit set in frame header"),
            Error::UnusedBitSet => f.write_str("rusty_zstd: unused bit set in frame header"),
            Error::ReservedBlockType => f.write_str("rusty_zstd: reserved block type"),
            Error::BlockTooLarge => f.write_str("rusty_zstd: block larger than window"),
            Error::WindowTooLarge => f.write_str("rusty_zstd: window larger than decoder cap"),
            Error::ContentSizeMismatch => {
                f.write_str("rusty_zstd: regenerated size != frame content size")
            }
            Error::ContentSizeTooLarge => f.write_str("rusty_zstd: content size too large"),
            Error::ChecksumMismatch => f.write_str("rusty_zstd: content checksum mismatch"),
            Error::DictionaryNeeded { id } => {
                write!(f, "rusty_zstd: dictionary id {id} required")
            }
            Error::DictionaryMismatch { frame, loaded } => {
                write!(f, "rusty_zstd: dictionary id {frame} != loaded {loaded}")
            }
            Error::TrailingBytes => f.write_str("rusty_zstd: trailing bytes after last frame"),
            Error::Corruption => f.write_str("rusty_zstd: corrupt zstd frame"),
        }
    }
}

#[cfg(feature = "std")]
impl std::error::Error for Error {}
