//! Streaming compress / decompress (`ZSTD_compressStream2` / `ZSTD_decompressStream` jobs).

use crate::block::{parse_block_header, BlockType};
use crate::compressed::{decode_compressed_block, BlockState};
use crate::encode::{
    checksum_u32, encode_block_from_scratch, write_frame_header, CompressOptions, EntropyState,
    MatchTables,
};
use crate::error::Error;
use crate::frame::{is_skippable_magic, parse_kind, FrameHeader, FrameKind, BLOCKSIZE_MAX, MAGIC};
use crate::params::{compression_params, CompressionParameters};
use crate::reader::Reader;
use crate::xxh64::Xxh64;
use crate::DecompressOptions;

#[cfg(feature = "alloc")]
use alloc::vec::Vec;

/// `ZSTD_EndDirective`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flush {
    /// Consume input; emit complete blocks only.
    Continue,
    /// Emit a complete (non-last) block for pending input.
    Flush,
    /// Last block + checksum; end the frame.
    End,
}

/// Bytes moved by one [`Compressor::stream`] / [`Decompressor::stream`] call.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamStatus {
    /// Input bytes this call consumed.
    pub input_consumed: usize,
    /// Output bytes this call produced.
    pub output_produced: usize,
    /// Frame fully finished (`Flush::End` completed, or a decompress frame ended and is drained).
    pub done: bool,
}

/// Recommended input chunk (`ZSTD_CStreamInSize`).
pub fn compress_stream_in_size() -> usize {
    BLOCKSIZE_MAX as usize
}

/// Recommended output chunk (`ZSTD_CStreamOutSize`).
pub fn compress_stream_out_size() -> usize {
    crate::compress_bound(BLOCKSIZE_MAX as usize)
}

/// Recommended decompress input chunk (`ZSTD_DStreamInSize`).
pub fn decompress_stream_in_size() -> usize {
    BLOCKSIZE_MAX as usize + 3
}

/// Recommended decompress output chunk (`ZSTD_DStreamOutSize`).
pub fn decompress_stream_out_size() -> usize {
    BLOCKSIZE_MAX as usize
}

/// Reusable compressor (one frame at a time).
pub struct Compressor {
    params: CompressionParameters,
    checksum: bool,
    pledged: Option<u64>,
    started: bool,
    ended: bool,
    xxh: Xxh64,
    reps: [u32; 3],
    tables: MatchTables,
    entropy: EntropyState,
    hist: Vec<u8>,
    in_acc: Vec<u8>,
    /// Consumed prefix of `in_acc`; compacted once per `stream` call.
    in_off: usize,
    out_acc: Vec<u8>,
    produced_in: u64,
    last_block_written: bool,
    dict_id: Option<u32>,
    write_dict_id: bool,
}

impl Compressor {
    /// New compressor at `level`.
    pub fn new(level: i32) -> Result<Self, Error> {
        Self::with_options(
            CompressOptions {
                level,
                checksum: true,
            },
            None,
        )
    }

    /// New compressor with options and an optional size hint (picks window/hash).
    pub fn with_options(opts: CompressOptions, src_hint: Option<u64>) -> Result<Self, Error> {
        let params = compression_params(opts.level, src_hint)?;
        Ok(Self {
            tables: MatchTables::new(params),
            params,
            checksum: opts.checksum,
            pledged: src_hint,
            started: false,
            ended: false,
            xxh: Xxh64::new(),
            reps: [1, 4, 8],
            entropy: EntropyState::default(),
            hist: Vec::new(),
            in_acc: Vec::new(),
            in_off: 0,
            out_acc: Vec::new(),
            produced_in: 0,
            last_block_written: false,
            dict_id: None,
            write_dict_id: true,
        })
    }

    /// Pledge uncompressed size before the first [`Self::stream`] call (writes FCS).
    pub fn set_pledged_src_size(&mut self, size: u64) {
        if !self.started {
            self.pledged = Some(size);
        }
    }

    /// Load a dictionary before the first [`Self::stream`] call.
    pub fn set_dictionary(&mut self, dict: &crate::dict::Dictionary) -> Result<(), Error> {
        if self.started {
            return Err(Error::Corruption);
        }
        self.dict_id = if dict.id() != 0 {
            Some(dict.id())
        } else {
            None
        };
        self.write_dict_id = self.dict_id.is_some();
        self.hist.clear();
        self.hist.extend_from_slice(dict.content());
        let window = 1usize << self.params.window_log.min(31);
        crate::encode::prime_tables(
            &mut self.tables,
            &self.hist,
            self.hist.len(),
            window,
            self.params,
        );
        if let Some(e) = dict.entropy() {
            self.entropy.seed_from_dict(e);
            self.reps = e.reps;
        }
        Ok(())
    }

    /// Use `prefix` as match history (`--patch-from`). No Dictionary_ID is written.
    pub fn set_prefix(&mut self, prefix: &[u8]) -> Result<(), Error> {
        if self.started {
            return Err(Error::Corruption);
        }
        self.dict_id = None;
        self.write_dict_id = false;
        self.hist.clear();
        self.hist.extend_from_slice(prefix);
        let window = 1usize << self.params.window_log.min(31);
        crate::encode::prime_tables(
            &mut self.tables,
            &self.hist,
            self.hist.len(),
            window,
            self.params,
        );
        Ok(())
    }

    /// Omit Dictionary_ID from the frame header (`--no-dictID`).
    pub fn set_write_dict_id(&mut self, write: bool) {
        if !self.started {
            self.write_dict_id = write;
        }
    }

    /// Streaming compress. Copies produced bytes into `output`.
    pub fn stream(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        flush: Flush,
    ) -> Result<StreamStatus, Error> {
        if self.ended {
            let n = self.out_acc.len().min(output.len());
            output[..n].copy_from_slice(&self.out_acc[..n]);
            self.out_acc.drain(..n);
            return Ok(StreamStatus {
                input_consumed: 0,
                output_produced: n,
                done: self.out_acc.is_empty(),
            });
        }
        self.in_acc.extend_from_slice(input);
        if !self.started {
            write_frame_header(
                &mut self.out_acc,
                0,
                self.params.window_log,
                self.checksum,
                self.pledged,
                if self.write_dict_id {
                    self.dict_id
                } else {
                    None
                },
                !self.hist.is_empty(),
            );
            self.started = true;
        }

        let block_max = (1usize << self.params.window_log.min(31)).min(BLOCKSIZE_MAX as usize);
        let emit_all = matches!(flush, Flush::Flush | Flush::End);
        while self.pending() >= block_max || (emit_all && self.pending() > 0) {
            let take = self.pending().min(block_max);
            let last = flush == Flush::End && take == self.pending();
            self.emit_block(take, last)?;
            if !last && flush == Flush::Flush && self.pending() == 0 {
                break;
            }
            if last {
                break;
            }
        }
        self.compact_in();

        if flush == Flush::End && !self.ended {
            if self.pending() == 0 {
                self.emit_empty_last_if_needed()?;
            }
            if self.checksum {
                self.out_acc
                    .extend_from_slice(&checksum_u32(&self.xxh).to_le_bytes());
            }
            self.ended = true;
        }

        let n = self.out_acc.len().min(output.len());
        output[..n].copy_from_slice(&self.out_acc[..n]);
        self.out_acc.drain(..n);
        Ok(StreamStatus {
            input_consumed: input.len(),
            output_produced: n,
            done: self.ended && self.out_acc.is_empty(),
        })
    }

    fn pending(&self) -> usize {
        self.in_acc.len().saturating_sub(self.in_off)
    }

    fn compact_in(&mut self) {
        if self.in_off == 0 {
            return;
        }
        self.in_acc.drain(..self.in_off);
        self.in_off = 0;
    }

    fn emit_empty_last_if_needed(&mut self) -> Result<(), Error> {
        if self.ended {
            return Ok(());
        }
        if self.last_block_written {
            return Ok(());
        }
        self.emit_block(0, true)
    }

    fn emit_block(&mut self, take: usize, last: bool) -> Result<(), Error> {
        let block_start = self.hist.len();
        if take > 0 {
            let end = self.in_off + take;
            self.hist.extend_from_slice(&self.in_acc[self.in_off..end]);
            self.xxh.update(&self.hist[block_start..]);
            self.in_off = end;
        }
        let window = 1usize << self.params.window_log.min(31);
        encode_block_from_scratch(
            &mut self.out_acc,
            &self.hist,
            block_start,
            self.params,
            &mut self.tables,
            &mut self.reps,
            &mut self.entropy,
            last,
        )?;
        if self.hist.len() > window {
            let drop = self.hist.len() - window;
            self.hist.drain(..drop);
            self.tables.reset();
            crate::encode::prime_tables(
                &mut self.tables,
                &self.hist,
                self.hist.len(),
                window,
                self.params,
            );
        }
        self.produced_in += take as u64;
        if last {
            self.last_block_written = true;
        }
        if let (true, Some(n)) = (last, self.pledged) {
            if self.produced_in != n {
                return Err(Error::ContentSizeMismatch);
            }
        }
        Ok(())
    }
}

/// Reusable decompressor. Multi-frame. Window-bounded history.
pub struct Decompressor {
    opts: DecompressOptions,
    input: Vec<u8>,
    decoded: Vec<u8>,
    decoded_off: usize,
    header: Option<FrameHeader>,
    block_state: BlockState,
    xxh: Xxh64,
    frame_out: u64,
    in_checksum: bool,
    saw_zstd: bool,
    dict: Option<crate::dict::Dictionary>,
    prefix: Vec<u8>,
    frame_start: usize,
    frame_skipped: usize,
}

impl Decompressor {
    /// Default window cap (128 MiB).
    pub fn new() -> Self {
        Self::with_options(DecompressOptions::default())
    }

    /// Explicit window cap.
    pub fn with_options(opts: DecompressOptions) -> Self {
        Self {
            opts,
            input: Vec::new(),
            decoded: Vec::new(),
            decoded_off: 0,
            header: None,
            block_state: BlockState::new(),
            xxh: Xxh64::new(),
            frame_out: 0,
            in_checksum: false,
            saw_zstd: false,
            dict: None,
            prefix: Vec::new(),
            frame_start: 0,
            frame_skipped: 0,
        }
    }

    /// Load a dictionary before decompressing.
    pub fn set_dictionary(&mut self, dict: crate::dict::Dictionary) {
        self.prefix = dict.content().to_vec();
        self.dict = Some(dict);
    }

    /// Use `prefix` as match history (`--patch-from`).
    pub fn set_prefix(&mut self, prefix: &[u8]) {
        self.dict = None;
        self.prefix = prefix.to_vec();
    }

    /// Streaming decompress. `end` is true when the caller has no more input.
    pub fn stream(
        &mut self,
        input: &[u8],
        output: &mut [u8],
        end: bool,
    ) -> Result<StreamStatus, Error> {
        self.input.extend_from_slice(input);
        loop {
            if !self.progress()? {
                break;
            }
            if self.decoded.len() > self.decoded_off && output.is_empty() {
                break;
            }
        }
        let avail = self.decoded.len() - self.decoded_off;
        let n = avail.min(output.len());
        output[..n].copy_from_slice(&self.decoded[self.decoded_off..self.decoded_off + n]);
        self.decoded_off += n;
        self.compact();
        if end && !self.input.is_empty() && self.header.is_none() && n == 0 && avail == 0 {
            return Err(Error::UnexpectedEof);
        }
        let done = end
            && self.input.is_empty()
            && self.header.is_none()
            && self.decoded_off == self.decoded.len();
        if done && !self.saw_zstd {
            return Err(Error::UnexpectedEof);
        }
        Ok(StreamStatus {
            input_consumed: input.len(),
            output_produced: n,
            done,
        })
    }

    fn progress(&mut self) -> Result<bool, Error> {
        if self.in_checksum {
            if self.input.len() < 4 {
                return Ok(false);
            }
            let got =
                u32::from_le_bytes([self.input[0], self.input[1], self.input[2], self.input[3]]);
            self.input.drain(..4);
            if checksum_u32(&self.xxh) != got {
                return Err(Error::ChecksumMismatch);
            }
            if let Some(n) = self.header.and_then(|h| h.content_size) {
                if self.frame_out != n {
                    return Err(Error::ContentSizeMismatch);
                }
            }
            self.finish_frame();
            return Ok(true);
        }
        if self.header.is_none() {
            return self.try_header();
        }
        self.try_block()
    }

    fn try_header(&mut self) -> Result<bool, Error> {
        if self.input.len() < 4 {
            return Ok(false);
        }
        let magic =
            u32::from_le_bytes([self.input[0], self.input[1], self.input[2], self.input[3]]);
        if is_skippable_magic(magic) {
            if self.input.len() < 8 {
                return Ok(false);
            }
            let n = u32::from_le_bytes([self.input[4], self.input[5], self.input[6], self.input[7]])
                as usize;
            if self.input.len() < 8 + n {
                return Ok(false);
            }
            self.input.drain(..8 + n);
            return Ok(true);
        }
        if magic != MAGIC {
            return Err(if self.saw_zstd {
                Error::TrailingBytes
            } else {
                Error::BadMagic
            });
        }
        let mut r = Reader::new(&self.input);
        match parse_kind(&mut r) {
            Err(Error::UnexpectedEof) => Ok(false),
            Err(e) => Err(e),
            Ok(FrameKind::Skippable { user_data_size, .. }) => {
                let n = user_data_size as usize;
                if r.remaining() < n {
                    return Ok(false);
                }
                self.input.drain(..r.pos() + n);
                Ok(true)
            }
            Ok(FrameKind::Zstd(h)) => {
                if h.window_size > self.opts.window_max {
                    return Err(Error::WindowTooLarge);
                }
                if let Some(id) = h.dict_id {
                    match self.dict.as_ref() {
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
                let pos = r.pos();
                self.input.drain(..pos);
                self.header = Some(h);
                self.block_state = BlockState::from_dict(self.dict.as_ref());
                self.xxh = Xxh64::new();
                self.frame_out = 0;
                self.frame_start = self.decoded.len();
                self.frame_skipped = 0;
                self.saw_zstd = true;
                Ok(true)
            }
        }
    }

    fn try_block(&mut self) -> Result<bool, Error> {
        let header = self.header.ok_or(Error::Corruption)?;
        if self.input.len() < 3 {
            return Ok(false);
        }
        let mut r = Reader::new(&self.input);
        let bh = match parse_block_header(&mut r) {
            Err(Error::UnexpectedEof) => return Ok(false),
            Err(e) => return Err(e),
            Ok(h) => h,
        };
        let payload_n = bh.payload_len() as usize;
        if r.remaining() < payload_n {
            return Ok(false);
        }
        let hdr_len = r.pos();
        let payload = self.input[hdr_len..hdr_len + payload_n].to_vec();
        self.input.drain(..hdr_len + payload_n);
        let block_max = header.block_size_max();
        let start = self.decoded.len();
        match bh.ty {
            BlockType::Raw => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                self.decoded.extend_from_slice(&payload);
            }
            BlockType::Rle => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                let b = *payload.first().ok_or(Error::Corruption)?;
                let n = bh.size as usize;
                self.decoded.resize(self.decoded.len() + n, b);
            }
            BlockType::Compressed => {
                if bh.size > block_max {
                    return Err(Error::BlockTooLarge);
                }
                decode_compressed_block(
                    &payload,
                    &mut self.decoded,
                    header.window_size,
                    block_max,
                    &mut self.block_state,
                    &self.prefix,
                    self.frame_start,
                    self.frame_skipped,
                )?;
            }
        }
        let produced = &self.decoded[start..];
        self.xxh.update(produced);
        self.frame_out += produced.len() as u64;
        if bh.last {
            if header.checksum {
                self.in_checksum = true;
            } else {
                if let Some(n) = header.content_size {
                    if self.frame_out != n {
                        return Err(Error::ContentSizeMismatch);
                    }
                }
                self.finish_frame();
            }
        }
        Ok(true)
    }

    fn finish_frame(&mut self) {
        self.header = None;
        self.in_checksum = false;
        self.block_state = BlockState::new();
    }

    fn compact(&mut self) {
        let window = self
            .header
            .map(|h| h.window_size as usize)
            .unwrap_or(BLOCKSIZE_MAX as usize);
        let keep_from = self.decoded.len().saturating_sub(window);
        let drop = self.decoded_off.min(keep_from);
        if drop > 0 {
            self.decoded.drain(..drop);
            self.decoded_off -= drop;
            if drop <= self.frame_start {
                self.frame_start -= drop;
            } else {
                self.frame_skipped += drop - self.frame_start;
                self.frame_start = 0;
            }
        }
    }
}

impl Default for Decompressor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::compress;

    #[test]
    fn stream_compress_chunked_roundtrip() {
        let src = b"The quick brown fox jumps over the lazy dog. ".repeat(200);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(src.len() as u64);
        let mut zst = Vec::new();
        let mut tmp = [0u8; 64];
        let mut off = 0usize;
        while off < src.len() {
            let n = (src.len() - off).min(7);
            let st = c
                .stream(&src[off..off + n], &mut tmp, Flush::Continue)
                .unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            off += n;
        }
        loop {
            let st = c.stream(&[], &mut tmp, Flush::End).unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            if st.done {
                break;
            }
        }
        let mut d = Decompressor::new();
        let mut out = vec![0u8; src.len() + 16];
        let st = d.stream(&zst, &mut out, true).unwrap();
        assert!(st.done);
        assert_eq!(&out[..st.output_produced], src.as_slice());
    }

    #[test]
    fn silesia_mr_prefix_stream_roundtrip() {
        let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../..")
            .join("corpora/data/silesia/mr");
        if !path.is_file() {
            return;
        }
        let mut src = std::fs::read(&path).expect("read mr");
        src.truncate(277_521);
        let mut c = Compressor::new(1).unwrap();
        c.set_pledged_src_size(src.len() as u64);
        let mut zst = Vec::new();
        let mut tmp = vec![0u8; 64 * 1024];
        let mut off = 0usize;
        while off < src.len() {
            let n = (src.len() - off).min(4096);
            let st = c
                .stream(&src[off..off + n], &mut tmp, Flush::Continue)
                .unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            off += n;
        }
        loop {
            let st = c.stream(&[], &mut tmp, Flush::End).unwrap();
            zst.extend_from_slice(&tmp[..st.output_produced]);
            if st.done {
                break;
            }
        }
        let got = crate::decompress(&zst).expect("decompress stream frame");
        if got.as_slice() != src.as_slice() {
            let pos = got
                .iter()
                .zip(src.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(got.len().min(src.len()));
            panic!("stream mismatch at {pos}/{}", src.len());
        }
    }

    #[test]
    fn oneshot_and_stream_decompress() {
        let src = b"hello hello hello hello";
        let zst = compress(src, 3).unwrap();
        let mut d = Decompressor::new();
        let mut out = vec![0u8; 64];
        let mut got = Vec::new();
        let mut off = 0usize;
        while off < zst.len() {
            let n = (zst.len() - off).min(3);
            let end = off + n == zst.len();
            let st = d.stream(&zst[off..off + n], &mut out, end).unwrap();
            got.extend_from_slice(&out[..st.output_produced]);
            off += n;
            if st.done {
                break;
            }
        }
        assert_eq!(got, src);
    }
}
