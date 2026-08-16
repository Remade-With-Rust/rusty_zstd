//! Dual gate for M6: multi-thread job split vs C `zstd -d`.
//!
//! Skips when the pinned oracle is not on disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use rusty_zstd::{
    compress_with_advanced, compression_params, decompress, inspect_frames, AdvancedOptions,
    FrameKind, JOB_SIZE_MIN,
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
    let tmp = std::env::temp_dir().join(format!("rzstd-m6-d-{}", nonce()));
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
    let _ = fs::remove_file(&inn);
    if !status.success() {
        return Err("zstd -d failed".into());
    }
    let raw = fs::read(&out).map_err(|e| e.to_string())?;
    let _ = fs::remove_file(&out);
    Ok(raw)
}

fn c_compress(oracle: &Path, src: &[u8], extra: &[&str]) -> Result<Vec<u8>, String> {
    let tmp = std::env::temp_dir().join(format!("rzstd-m6-c-{}", nonce()));
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
    let _ = fs::remove_file(&inn);
    if !status.success() {
        return Err(format!("zstd {extra:?} failed"));
    }
    let zst = fs::read(&out).map_err(|e| e.to_string())?;
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

fn noise(n: usize) -> Vec<u8> {
    let mut s = 0x6D36u64;
    let mut v = vec![0u8; n];
    for b in &mut v {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s as u8) | 1;
    }
    v
}

fn us_mt(src: &[u8], workers: u32, overlap_log: u32) -> Vec<u8> {
    let params = compression_params(1, Some(src.len() as u64)).expect("params");
    compress_with_advanced(
        src,
        params,
        true,
        None,
        &[],
        true,
        AdvancedOptions {
            nb_workers: workers,
            job_size: JOB_SIZE_MIN,
            overlap_log,
            ..AdvancedOptions::default()
        },
    )
    .expect("us mt")
}

#[test]
fn us_mt_dual_gate() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = noise(JOB_SIZE_MIN + JOB_SIZE_MIN / 4);
    let zst = us_mt(&src, 2, 1);
    let n = inspect_frames(&zst)
        .unwrap()
        .iter()
        .filter(|f| matches!(f.kind, FrameKind::Zstd(_)))
        .count();
    assert!(n >= 2, "mt frames={n}");
    assert_bytes(&decompress(&zst).unwrap(), &src, "us mt decode");
    assert_bytes(&c_decompress(&oracle, &zst).expect("C -d"), &src, "C mt -d");
}

#[test]
fn c_mt_us_decode() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = noise(JOB_SIZE_MIN + 32 * 1024);
    let zst = match c_compress(&oracle, &src, &["-T2", "-B524288", "-1"]) {
        Ok(z) => z,
        Err(_) => return,
    };
    assert_bytes(
        &decompress(&zst).expect("us decode C -T2"),
        &src,
        "C -T2 -> us",
    );
}

#[test]
fn us_mt_overlap_c_decode() {
    let Some(oracle) = find_oracle() else {
        return;
    };
    let src = noise(JOB_SIZE_MIN + 16 * 1024);
    let zst = us_mt(&src, 2, 9);
    assert_bytes(
        &c_decompress(&oracle, &zst).expect("C ov"),
        &src,
        "C overlap -d",
    );
}
