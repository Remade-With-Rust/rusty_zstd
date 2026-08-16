//! Dual gate: rusty_zstd compress -> our decompress AND C zstd -d, bit-exact.
//!
//! Skips when the pinned oracle is not on disk.

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

fn c_decompress(oracle: &Path, zst: &[u8]) -> Result<Vec<u8>, String> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("rzstd-m2-{}-{}", zst.len(), nonce));
    let inn = tmp.with_extension("zst");
    let out = tmp.with_extension("out");
    fs::write(&inn, zst).map_err(|e| e.to_string())?;
    let status = Command::new(oracle)
        .args([
            "-d",
            "-f",
            "-q",
            "-o",
            out.to_str().unwrap(),
            inn.to_str().unwrap(),
        ])
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = fs::remove_file(&inn);
        return Err(format!("zstd -d failed on {} byte frame", zst.len()));
    }
    let raw = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&inn);
    let _ = fs::remove_file(&out);
    Ok(raw)
}

fn assert_dual_frame(oracle: &Path, src: &[u8], zst: &[u8]) {
    let us = rusty_zstd::decompress(zst)
        .unwrap_or_else(|e| panic!("us decompress src={} zst={}: {e:?}", src.len(), zst.len()));
    if us != src {
        let pos = us
            .iter()
            .zip(src.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(us.len());
        panic!("us mismatch at {pos} us={} src={}", us.len(), src.len());
    }
    let c = c_decompress(oracle, zst)
        .unwrap_or_else(|e| panic!("C zstd -d src={} zst={}: {e}", src.len(), zst.len()));
    if c != src {
        let pos = c
            .iter()
            .zip(src.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(c.len());
        panic!("C mismatch at {pos} c={} src={}", c.len(), src.len());
    }
}

fn assert_dual(oracle: &Path, src: &[u8], level: i32) {
    let zst = rusty_zstd::compress(src, level)
        .unwrap_or_else(|e| panic!("compress L{level} {} bytes: {e:?}", src.len()));
    assert_dual_frame(oracle, src, &zst);
}

fn xorshift_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed;
    let mut v = vec![0u8; n];
    for b in &mut v {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xFF) as u8;
        if *b == 0 {
            *b = 1;
        }
    }
    v
}

#[test]
fn dual_gate_small_minus7_to_3() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 4096 {
        text.extend_from_slice(fox);
    }
    for level in -7i32..=3 {
        assert_dual(&oracle, b"", level);
        assert_dual(&oracle, b"a", level);
        assert_dual(&oracle, b"hello", level);
        assert_dual(&oracle, &[0u8; 16], level);
        assert_dual(&oracle, &vec![0u8; 4096], level);
        assert_dual(&oracle, &text, level);
        assert_dual(&oracle, &xorshift_bytes(0xA5A5_5A5A, 1024), level);
        assert_dual(&oracle, &xorshift_bytes(0xA5A5_5A5A, 128 * 1024), level);
    }
}

#[test]
fn dual_gate_greedy_and_higher() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let src = xorshift_bytes(0x1111_2222, 32 * 1024);
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(64);
    for level in [5, 6, 9, 13, 19, 22] {
        assert_dual(&oracle, &src, level);
        assert_dual(&oracle, &fox, level);
    }
}

#[test]
fn dual_gate_huffman_and_multi_block() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut two_blocks = Vec::new();
    while two_blocks.len() < 130 * 1024 {
        two_blocks.extend_from_slice(fox);
    }
    assert_dual(&oracle, &fox.repeat(8), 1);
    assert_dual(&oracle, &vec![0u8; 8192], 3);
    assert_dual(&oracle, &two_blocks, 3);
}

#[test]
fn dual_gate_1m_l1_l3() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let src = xorshift_bytes(0xA5A5_5A5A, 1024 * 1024);
    for level in [1, 3] {
        assert_dual(&oracle, &src, level);
    }
}

#[test]
fn dual_gate_full_level_range() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(64);
    let noise = xorshift_bytes(0xBEEF_F00D, 2048);
    for level in rusty_zstd::MIN_CLEVEL..=rusty_zstd::MAX_CLEVEL {
        assert_dual(&oracle, &fox, level);
        assert_dual(&oracle, &noise, level);
        assert_dual(&oracle, b"hello", level);
    }
}

#[test]
fn dual_gate_all_strategies() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n".repeat(32);
    let noise = xorshift_bytes(0x3333_4444, 4096);
    let zeros = [0u8; 2048];
    for id in 1i32..=9 {
        let mut params = rusty_zstd::compression_params(3, Some(fox.len() as u64)).unwrap();
        params.apply_zstd_kv("strategy", id).unwrap();
        for src in [fox.as_slice(), noise.as_slice(), zeros.as_slice()] {
            let zst = rusty_zstd::compress_with_params(src, params, true)
                .unwrap_or_else(|e| panic!("strategy {id}: {e:?}"));
            assert_dual_frame(&oracle, src, &zst);
        }
    }
}

#[test]
fn dual_gate_literals_and_seq_modes() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut skip = rusty_zstd::compression_params(1, Some(400)).unwrap();
    skip.target_length = 1 << 16;
    skip.min_match = 7;
    skip.strategy = rusty_zstd::Strategy::Fast;

    let mut huff = Vec::new();
    while huff.len() < 400 {
        huff.extend_from_slice(fox);
    }
    huff.truncate(400);
    let mut huff_two = Vec::new();
    while huff_two.len() < 130 * 1024 {
        huff_two.extend_from_slice(fox);
    }
    let mut two_sym = Vec::new();
    while two_sym.len() < 400 {
        two_sym.extend_from_slice(&[0u8, 0, 0, 1]);
    }
    two_sym.truncate(400);

    let mut rle_lits = fox.repeat(20);
    rle_lits.truncate(1024);
    for _ in 0..30 {
        rle_lits.push(0xA5);
        rle_lits.push(0xA5);
        rle_lits.extend_from_slice(&fox[..20]);
    }
    let mut rle_win = rusty_zstd::compression_params(1, Some(rle_lits.len() as u64)).unwrap();
    rle_win.window_log = 10;

    let mut mixed = Vec::new();
    let mut n = 0u32;
    while mixed.len() < 4096 {
        mixed.extend_from_slice(b"block ");
        mixed.push(b'0' + (n % 10) as u8);
        mixed.extend_from_slice(b" extra words for matches ");
        mixed.extend_from_slice(&n.to_le_bytes());
        mixed.push(b'\n');
        n += 1;
    }
    let mixed2 = mixed.repeat(2);
    let mut mix_win = rusty_zstd::compression_params(1, Some(8192)).unwrap();
    mix_win.window_log = 12;
    let fox_multi = fox.repeat(60);
    let mut fox_win = rusty_zstd::compression_params(3, Some(3000)).unwrap();
    fox_win.window_log = 10;
    let repeated = b"TheQuickBrownFox0123456789ABCD".repeat(80);
    let mut small_win = rusty_zstd::compression_params(1, Some(2048)).unwrap();
    small_win.window_log = 10;

    let cases: Vec<(Vec<u8>, rusty_zstd::CompressionParameters)> = vec![
        (rle_lits, rle_win),
        (huff, skip),
        (huff_two, skip),
        (two_sym, skip),
        (
            mixed.clone(),
            rusty_zstd::compression_params(1, Some(mixed.len() as u64)).unwrap(),
        ),
        (mixed2, mix_win),
        (fox_multi, fox_win),
        (repeated, small_win),
    ];
    for (src, p) in cases {
        let zst = rusty_zstd::compress_with_params(&src, p, true)
            .unwrap_or_else(|e| panic!("content-type src={}: {e:?}", src.len()));
        assert_dual_frame(&oracle, &src, &zst);
    }
}

#[test]
fn dual_gate_silesia_l1() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let dir = repo_root().join("corpora/data/silesia");
    if !dir.is_dir() {
        eprintln!("skip: silesia corpus absent");
        return;
    }
    let mut files: Vec<_> = fs::read_dir(&dir)
        .expect("silesia dir")
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| p.is_file())
        .collect();
    files.sort();
    assert!(!files.is_empty(), "silesia dir exists but has no files");
    for path in files {
        let src = fs::read(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        let name = path.file_name().unwrap().to_string_lossy();
        for level in [1, -1, -4] {
            let zst = rusty_zstd::compress(&src, level)
                .unwrap_or_else(|e| panic!("compress {name} L{level}: {e:?}"));
            assert_dual_frame(&oracle, &src, &zst);
        }
    }
}
