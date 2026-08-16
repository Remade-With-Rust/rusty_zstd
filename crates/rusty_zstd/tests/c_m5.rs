//! Dual gate for M5: LDM / rsyncable / target cblock / seekable vs C `zstd -d`.
//!
//! Skips when the pinned oracle is not on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_zstd::{
    compress_seekable, compress_with_advanced, compression_params, decompress, decompress_frame_at,
    parse_seek_table, AdvancedOptions, LdmParams,
};

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

fn nonce() -> u128 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0)
}

fn c_decompress(oracle: &Path, zst: &[u8]) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("rzstd-m5-d-{}", nonce()));
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

fn c_compress(oracle: &Path, src: &[u8], extra: &[&str]) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("rzstd-m5-c-{}", nonce()));
    let inn = tmp.with_extension("in");
    let out = tmp.with_extension("zst");
    fs::write(&inn, src).map_err(|e| e.to_string())?;
    let mut args: Vec<&str> = extra.to_vec();
    args.extend_from_slice(
        [
            "-q",
            "-f",
            "-o",
            out.to_str().unwrap(),
            inn.to_str().unwrap(),
        ]
        .as_slice(),
    );
    let status = Command::new(oracle)
        .args(&args)
        .status()
        .map_err(|e| e.to_string())?;
    if !status.success() {
        let _ = fs::remove_file(&inn);
        return Err(format!("zstd {:?} failed", extra));
    }
    let zst = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&inn);
    let _ = fs::remove_file(&out);
    Ok(zst)
}

fn assert_bytes(got: &[u8], want: &[u8], tag: &str) {
    if got == want {
        return;
    }
    let pos = got
        .iter()
        .zip(want.iter())
        .position(|(a, b)| a != b)
        .unwrap_or(got.len().min(want.len()));
    panic!(
        "{tag}: mismatch at {pos} got={} want={}",
        got.len(),
        want.len()
    );
}

fn assert_dual(oracle: &Path, src: &[u8], zst: &[u8], tag: &str) {
    let us = decompress(zst).unwrap_or_else(|e| panic!("{tag} us decode: {e:?}"));
    assert_bytes(&us, src, &format!("{tag} us"));
    let c = c_decompress(oracle, zst).unwrap_or_else(|e| panic!("{tag} C -d: {e}"));
    assert_bytes(&c, src, &format!("{tag} C"));
}

fn distant_pattern(n: usize) -> Vec<u8> {
    let mut src = vec![0u8; n];
    let mut pat = [0u8; 64];
    for (i, b) in pat.iter_mut().enumerate() {
        *b = (i as u8).wrapping_add(17);
    }
    src[0..64].copy_from_slice(&pat);
    let second = (n / 2) & !127;
    if second + 64 <= n && second > 64 {
        src[second..second + 64].copy_from_slice(&pat);
    }
    for (i, b) in src.iter_mut().enumerate() {
        if *b == 0 {
            *b = (i.wrapping_mul(131) % 251) as u8 + 1;
        }
    }
    src[0..64].copy_from_slice(&pat);
    if second + 64 <= n && second > 64 {
        src[second..second + 64].copy_from_slice(&pat);
    }
    src
}

fn us_long(src: &[u8], window_log: u32) -> Vec<u8> {
    let mut params = compression_params(1, Some(src.len() as u64)).expect("params");
    params.window_log = window_log;
    compress_with_advanced(
        src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            ldm: LdmParams::enabled(),
            ..AdvancedOptions::default()
        },
    )
    .expect("us --long")
}

#[test]
fn us_long_dual_gate() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(256 * 1024);
    let zst = us_long(&src, 18);
    assert_dual(&oracle, &src, &zst, "us --long=18");
}

#[test]
fn c_long_us_decode() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(256 * 1024);
    let zst = match c_compress(&oracle, &src, &["--long=18", "-1"]) {
        Ok(z) => z,
        Err(e) => panic!("C --long=18: {e}"),
    };
    let us = decompress(&zst).expect("us decode C --long");
    assert_bytes(&us, &src, "C --long=18 -> us");
}

#[test]
fn us_rsyncable_dual_gate() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(128 * 1024);
    let mut params = compression_params(1, Some(src.len() as u64)).expect("params");
    params.window_log = 18;
    let zst = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            ldm: LdmParams::enabled(),
            rsyncable: true,
            target_cblock_size: 0,
            ..AdvancedOptions::default()
        },
    )
    .expect("us --rsyncable");
    assert_dual(&oracle, &src, &zst, "us --rsyncable");
}

#[test]
fn c_rsyncable_us_decode() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(64 * 1024);
    let zst = match c_compress(&oracle, &src, &["--rsyncable", "-1"]) {
        Ok(z) => z,
        Err(_) => return,
    };
    let us = decompress(&zst).expect("us decode C --rsyncable");
    assert_bytes(&us, &src, "C --rsyncable -> us");
}

#[test]
fn us_target_cblock_dual_gate() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(64 * 1024);
    let params = compression_params(1, Some(src.len() as u64)).expect("params");
    let zst = compress_with_advanced(
        &src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            ldm: LdmParams::default(),
            rsyncable: false,
            target_cblock_size: 256,
            ..AdvancedOptions::default()
        },
    )
    .expect("us target-cblock");
    assert_dual(&oracle, &src, &zst, "us target-cblock");
}

#[test]
fn us_seekable_dual_gate_and_table() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(8 * 1024);
    let params = compression_params(1, Some(src.len() as u64)).expect("params");
    let zst = compress_seekable(&src, params, true, 512).expect("seekable");
    assert_dual(&oracle, &src, &zst, "us --seekable");
    let table = parse_seek_table(&zst).expect("seek table");
    assert!(table.entries.len() >= 2, "frames={}", table.entries.len());
    assert_eq!(table.uncompressed_size(), src.len() as u64);
    let piece = decompress_frame_at(&zst, &table, 0).expect("frame 0");
    assert_bytes(&piece, &src[..piece.len()], "seek frame 0");
    let mid = table.uncompressed_offset(1);
    let piece1 = decompress_frame_at(&zst, &table, mid).expect("frame 1");
    assert_bytes(
        &piece1,
        &src[mid as usize..mid as usize + piece1.len()],
        "seek frame 1",
    );
}

#[test]
fn c_target_cblock_us_decode() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = distant_pattern(32 * 1024);
    let zst = match c_compress(&oracle, &src, &["--target-compressed-block-size=256", "-1"]) {
        Ok(z) => z,
        Err(_) => return,
    };
    let us = decompress(&zst).expect("us decode C target-cblock");
    assert_bytes(&us, &src, "C target-cblock -> us");
}
