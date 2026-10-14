//! SEC-03: every entropy bitstream must be consumed EXACTLY.
//!
//! Bitstreams are read backwards, from the padding mark toward the first byte,
//! so a byte inserted at a stream's START (with the section size bumped to
//! match) leaves every decoded symbol unchanged and that byte unread. libzstd
//! rejects such a frame (`BIT_endOfDStream`); rusty_zstd used to accept it with
//! perfectly plausible output, because nothing checked the stream's end.
//!
//! The frames here are built from our own encoder's output by hand-parsing the
//! block, so no internals are needed. When the pinned C oracle is on disk, each
//! one is also run through it to prove the reference rejects it -- except the
//! 4-stream Huffman case, see `four_stream_huffman_trailing_byte_is_rejected`.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/rusty_zstd -> repo root")
        .to_path_buf()
}

fn find_oracle() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTY_ZSTD_ORACLE") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let extracted = repo_root().join("third_party/zstd/extracted");
    walk(&extracted).ok()?.into_iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("zstd.exe") || n.eq_ignore_ascii_case("zstd"))
    })
}

fn walk(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for ent in fs::read_dir(dir)? {
        let p = ent?.path();
        if p.is_dir() {
            out.extend(walk(&p)?);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

/// `true` when the C oracle decodes `frame` without error.
fn c_accepts(oracle: &Path, frame: &[u8], tag: &str) -> bool {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!("rzstd-strict-{tag}-{nonce}.zst"));
    fs::write(&path, frame).unwrap();
    let ok = Command::new(oracle)
        .args(["-t", "-q"])
        .arg(&path)
        .output()
        .expect("spawn zstd")
        .status
        .success();
    let _ = fs::remove_file(&path);
    ok
}

/// Deterministic word salad: compressible enough for Huffman literals and
/// matches, irregular enough not to collapse to a few sequences.
fn text(n: usize) -> Vec<u8> {
    let words = [
        "alpha ", "beta ", "gamma ", "delta ", "epsilon ", "zeta ", "eta ", "theta ", "iota ",
        "kappa ",
    ];
    let mut s = Vec::new();
    let mut x = 12345u32;
    while s.len() < n {
        x = x.wrapping_mul(1_103_515_245).wrapping_add(12345);
        s.extend_from_slice(words[(x >> 16) as usize % words.len()].as_bytes());
        s.push(b'a' + (x >> 8) as u8 % 26);
    }
    s.truncate(n);
    s
}

fn encode(src: &[u8], level: i32) -> Vec<u8> {
    let p = rusty_zstd::compression_params(level, Some(src.len() as u64)).unwrap();
    let f = rusty_zstd::compress_with_params(src, p, false).unwrap();
    assert_eq!(
        rusty_zstd::decompress(&f).unwrap(),
        src,
        "baseline round-trip"
    );
    f
}

/// Offsets of the single, last, Compressed block of a checksum-less frame:
/// (block header, payload start, payload length).
fn single_block(f: &[u8]) -> (usize, usize, usize) {
    assert_eq!(&f[..4], &[0x28, 0xB5, 0x2F, 0xFD]);
    let fhd = f[4];
    let single = (fhd >> 5) & 1 == 1;
    assert_eq!((fhd >> 2) & 1, 0, "checksum-less frame expected");
    let did = [0usize, 1, 2, 4][usize::from(fhd & 3)];
    let fcs = match fhd >> 6 {
        0 => usize::from(single),
        1 => 2,
        2 => 4,
        _ => 8,
    };
    let at = 5 + usize::from(!single) + did + fcs;
    let v = u32::from(f[at]) | (u32::from(f[at + 1]) << 8) | (u32::from(f[at + 2]) << 16);
    assert_eq!(v & 1, 1, "single last block expected");
    assert_eq!((v >> 1) & 3, 2, "compressed block expected");
    (at, at + 3, (v >> 3) as usize)
}

/// The frame with its block payload replaced (block size rewritten to match).
fn with_payload(f: &[u8], payload: &[u8]) -> Vec<u8> {
    let (hdr, at, len) = single_block(f);
    let mut out = f[..hdr].to_vec();
    let v = 1u32 | (2 << 1) | ((payload.len() as u32) << 3);
    out.extend_from_slice(&v.to_le_bytes()[..3]);
    out.extend_from_slice(payload);
    out.extend_from_slice(&f[at + len..]);
    out
}

/// Compressed-literals header: (size_format, regen, csize, header_len).
fn huff_lit_header(p: &[u8]) -> (u8, u64, u64, usize) {
    assert_eq!(p[0] & 3, 2, "Huffman-compressed literals expected");
    let sf = (p[0] >> 2) & 3;
    let (n, size_bits, csize_shift) = match sf {
        0 | 1 => (3, 10, 14),
        2 => (4, 14, 18),
        _ => (5, 18, 22),
    };
    let mut b = [0u8; 8];
    b[..n].copy_from_slice(&p[..n]);
    let v = u64::from_le_bytes(b);
    let mask = (1u64 << size_bits) - 1;
    (sf, (v >> 4) & mask, (v >> csize_shift) & mask, n)
}

fn encode_huff_lit_header(sf: u8, regen: u64, csize: u64) -> Vec<u8> {
    let (v, n) = match sf {
        0 | 1 => (2 | (u64::from(sf) << 2) | (regen << 4) | (csize << 14), 3),
        2 => (2 | (2 << 2) | (regen << 4) | (csize << 18), 4),
        _ => (2 | (3 << 2) | (regen << 4) | (csize << 22), 5),
    };
    v.to_le_bytes()[..n].to_vec()
}

/// One unread byte at the start of the first Huffman stream.
fn huffman_trailing_byte(f: &[u8]) -> (Vec<u8>, bool) {
    let (_, at, len) = single_block(f);
    let p = &f[at..at + len];
    let (sf, regen, csize, hl) = huff_lit_header(p);
    // Tree description: FSE-compressed weights of `hb` bytes, or direct 4-bit
    // weights for `hb - 127` symbols.
    let hb = usize::from(p[hl]);
    let tree = 1 + if hb < 128 { hb } else { (hb - 127).div_ceil(2) };
    let streams = hl + tree;
    let mut np = encode_huff_lit_header(sf, regen, csize + 1);
    np.extend_from_slice(&p[hl..streams]);
    let four = sf != 0;
    if four {
        let s1 = u16::from_le_bytes([p[streams], p[streams + 1]]) + 1;
        np.extend_from_slice(&s1.to_le_bytes());
        np.extend_from_slice(&p[streams + 2..streams + 6]);
        np.push(0x5A);
        np.extend_from_slice(&p[streams + 6..]);
    } else {
        np.push(0x5A);
        np.extend_from_slice(&p[streams..]);
    }
    (with_payload(f, &np), four)
}

/// One unread byte at the start of the sequence bitstream; `None` when a
/// table is FSE-compressed (its description is not parsed here).
fn sequence_trailing_byte(f: &[u8]) -> Option<Vec<u8>> {
    let (_, at, len) = single_block(f);
    let p = &f[at..at + len];
    let lt = p[0] & 3;
    let sf = (p[0] >> 2) & 3;
    let lits = if lt >= 2 {
        let (_, _, csize, hl) = huff_lit_header(p);
        hl + csize as usize
    } else {
        let (hl, regen) = match sf {
            0 | 2 => (1, usize::from(p[0] >> 3)),
            1 => (2, usize::from(p[0] >> 4) + (usize::from(p[1]) << 4)),
            _ => (
                3,
                usize::from(p[0] >> 4) + (usize::from(p[1]) << 4) + (usize::from(p[2]) << 12),
            ),
        };
        hl + if lt == 0 { regen } else { 1 }
    };
    let mut pos = lits
        + match p[lits] {
            0 => return None,
            1..=127 => 1,
            128..=254 => 2,
            255 => 3,
        };
    let modes = p[pos];
    pos += 1;
    for mode in [modes >> 6, (modes >> 4) & 3, (modes >> 2) & 3] {
        match mode {
            0 => {}
            1 => pos += 1,
            _ => return None,
        }
    }
    let mut np = p[..pos].to_vec();
    np.push(0x5A);
    np.extend_from_slice(&p[pos..]);
    Some(with_payload(f, &np))
}

#[test]
fn single_stream_huffman_trailing_byte_is_rejected() {
    let oracle = find_oracle();
    let mut seen = 0;
    for n in [300usize, 600, 1000] {
        let (bad, four) = huffman_trailing_byte(&encode(&text(n), 3));
        if four {
            continue;
        }
        seen += 1;
        assert_eq!(
            rusty_zstd::decompress(&bad).unwrap_err(),
            rusty_zstd::Error::Corruption,
            "n={n}"
        );
        if let Some(o) = &oracle {
            assert!(!c_accepts(o, &bad, "huf1"), "n={n}: libzstd accepted it");
        }
    }
    assert!(seen > 0, "no single-stream literal section was produced");
}

/// Stricter than libzstd, deliberately. Its FAST 4-stream Huffman loop checks
/// only that each segment produced its byte count, so it accepts this frame;
/// its portable 4-stream path (`HUF_decompress4X*_usingDTable_internal_body`)
/// checks `BIT_endOfDStream` on all four and rejects it. A frame whose
/// acceptance depends on which libzstd build decodes it is not one to accept.
#[test]
fn four_stream_huffman_trailing_byte_is_rejected() {
    let mut seen = 0;
    for n in [2000usize, 5000, 20_000] {
        let (bad, four) = huffman_trailing_byte(&encode(&text(n), 3));
        if !four {
            continue;
        }
        seen += 1;
        assert_eq!(
            rusty_zstd::decompress(&bad).unwrap_err(),
            rusty_zstd::Error::Corruption,
            "n={n}"
        );
    }
    assert!(seen > 0, "no 4-stream literal section was produced");
}

#[test]
fn sequence_trailing_byte_is_rejected() {
    let oracle = find_oracle();
    let mut seen = 0;
    for n in [40usize, 60, 80, 120, 160] {
        for level in [1, 3, 9] {
            let Some(bad) = sequence_trailing_byte(&encode(&text(n), level)) else {
                continue;
            };
            seen += 1;
            assert_eq!(
                rusty_zstd::decompress(&bad).unwrap_err(),
                rusty_zstd::Error::Corruption,
                "n={n} L{level}"
            );
            if let Some(o) = &oracle {
                assert!(
                    !c_accepts(o, &bad, "seq"),
                    "n={n} L{level}: libzstd accepted it"
                );
            }
        }
    }
    assert!(
        seen > 0,
        "no predefined/RLE-table sequence section was produced"
    );
}
