//! Locate and verify the pinned facebook/zstd CLI. Never link libzstd.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use sha2::{Digest, Sha256};

/// Pinned facebook/zstd release.
pub const PINNED_TAG: &str = "v1.5.7";
/// Substring that must appear in `zstd --version`.
pub const PINNED_VERSION_NEEDLE: &str = "1.5.7";
/// SHA-256 of `zstd-v1.5.7-win64.zip` as fetched 2026-08-13 from GitHub Releases.
#[allow(dead_code)]
pub const WIN64_ZIP_SHA256: &str =
    "acb4e8111511749dc7a3ebedca9b04190e37a17afeb73f55d4425dbf0b90fad9";
/// SHA-256 of the extracted `zstd.exe` from that zip.
pub const WIN64_EXE_SHA256: &str =
    "8076aae03feac7c66b319579e82172eed168deed2a3f25e5e2d3c60f55e84111";
/// GitHub release asset.
#[allow(dead_code)]
pub const WIN64_ZIP_URL: &str =
    "https://github.com/facebook/zstd/releases/download/v1.5.7/zstd-v1.5.7-win64.zip";

#[derive(Debug, Clone)]
pub struct Oracle {
    pub path: PathBuf,
    pub version_line: String,
    pub sha256: String,
}

pub fn find_oracle(root: &Path) -> Result<Oracle, String> {
    let path = resolve_path(root)?;
    let sha256 = file_sha256(&path)?;
    if cfg!(windows) && sha256 != WIN64_EXE_SHA256 {
        return Err(format!(
            "oracle sha256 mismatch: got {sha256}, expected {WIN64_EXE_SHA256} ({PINNED_TAG} win64). Re-run scripts/fetch-oracle.ps1"
        ));
    }
    let version_line = version_of(&path)?;
    if !version_line.contains(PINNED_VERSION_NEEDLE) {
        return Err(format!(
            "oracle version is not {PINNED_TAG}: {version_line}"
        ));
    }
    Ok(Oracle {
        path,
        version_line: version_line.trim().to_string(),
        sha256,
    })
}

fn resolve_path(root: &Path) -> Result<PathBuf, String> {
    if let Ok(p) = std::env::var("RUSTY_ZSTD_ORACLE") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Ok(pb);
        }
        return Err(format!("RUSTY_ZSTD_ORACLE is not a file: {}", pb.display()));
    }
    let extracted = root.join("third_party").join("zstd").join("extracted");
    if extracted.is_dir() {
        if let Some(found) =
            find_named_exe(&extracted, "zstd.exe").or_else(|| find_named_exe(&extracted, "zstd"))
        {
            return Ok(found);
        }
    }
    Err(format!(
        "pinned C zstd ({PINNED_TAG}) not found. Set RUSTY_ZSTD_ORACLE or run scripts/fetch-oracle.ps1\n  expected under {}",
        extracted.display()
    ))
}

fn find_named_exe(dir: &Path, name: &str) -> Option<PathBuf> {
    let walker = walk(dir).ok()?;
    walker.into_iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case(name))
    })
}

fn walk(dir: &Path) -> Result<Vec<PathBuf>, std::io::Error> {
    let mut out = Vec::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let p = ent.path();
        if p.is_dir() {
            out.extend(walk(&p)?);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

pub fn file_sha256(path: &Path) -> Result<String, String> {
    let bytes = fs::read(path).map_err(|e| format!("read {}: {e}", path.display()))?;
    Ok(hex_sha256(&bytes))
}

pub fn hex_sha256(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    let d = h.finalize();
    d.iter().map(|b| format!("{b:02x}")).collect()
}

fn version_of(path: &Path) -> Result<String, String> {
    let out = Command::new(path)
        .arg("--version")
        .output()
        .map_err(|e| format!("spawn {}: {e}", path.display()))?;
    let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if !stdout.is_empty() {
        Ok(stdout)
    } else if !stderr.is_empty() {
        Ok(stderr)
    } else {
        Err("zstd --version produced no output".into())
    }
}
