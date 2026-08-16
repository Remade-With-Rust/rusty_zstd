//! Append-only JSONL ledger.

use std::fs::OpenOptions;
use std::io::Write;
use std::path::Path;

use serde::Serialize;

use crate::oracle::Oracle;

#[derive(Debug, Clone, Serialize)]
pub struct CZstd {
    pub tag: String,
    pub version_line: String,
    pub path: String,
    pub sha256: String,
}

impl CZstd {
    pub fn from_oracle(o: &Oracle) -> Self {
        Self {
            tag: crate::oracle::PINNED_TAG.to_string(),
            version_line: o.version_line.clone(),
            path: o.path.display().to_string(),
            sha256: o.sha256.clone(),
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct CorpusId {
    pub id: String,
    pub split: &'static str,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct CBench {
    pub compress_mbps: f64,
    pub decompress_mbps: f64,
    pub compressed_bytes_reported: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct Oneshot {
    pub compress_cpu_ms: Option<f64>,
    pub compress_wall_ms: f64,
    pub compress_peak_rss_bytes: Option<u64>,
    pub decompress_cpu_ms: Option<f64>,
    pub decompress_wall_ms: f64,
    pub decompress_peak_rss_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct Gates {
    pub correctness: &'static str,
    pub ratio: &'static str,
    pub speed: &'static str,
    pub footprint: &'static str,
}

#[derive(Debug, Serialize)]
pub struct SessionLine {
    pub kind: &'static str,
    pub ts: String,
    pub git_sha: Option<String>,
    pub host: String,
    pub c_zstd: CZstd,
    pub method: String,
    pub null_arm_compress_mbps_ratio: f64,
    pub notes: &'static str,
}

#[derive(Debug, Serialize)]
pub struct BaselineLine {
    pub kind: &'static str,
    pub ts: String,
    pub git_sha: Option<String>,
    pub host: String,
    pub c_zstd: CZstd,
    pub corpus: CorpusId,
    pub level: i32,
    pub src_bytes: u64,
    pub compressed_bytes: u64,
    pub ratio: f64,
    pub roundtrip_ok: bool,
    pub c_bench: CBench,
    pub oneshot: Oneshot,
    pub method: String,
    pub gates: Gates,
    pub notes: &'static str,
}

#[derive(Debug, Serialize)]
pub struct RatioLine {
    pub kind: &'static str,
    pub ts: String,
    pub git_sha: Option<String>,
    pub host: String,
    pub c_zstd: CZstd,
    pub corpus: CorpusId,
    pub level: i32,
    pub src_bytes: u64,
    pub us_compressed_bytes: u64,
    pub c_compressed_bytes: u64,
    pub us_ratio: f64,
    pub c_ratio: f64,
    /// `us_compressed / c_compressed` -- 1.0 matches C size; >1 means we spent more bytes.
    pub us_over_c: f64,
    pub us_roundtrip_ok: bool,
    pub c_decode_us_ok: bool,
    pub method: String,
    pub gates: Gates,
    pub notes: &'static str,
}

/// Us vs C at a brag operating point (`-1`, `--fast=1`, `--fast=4`).
#[derive(Debug, Serialize)]
pub struct SpeedLine {
    pub kind: &'static str,
    pub ts: String,
    pub git_sha: Option<String>,
    pub host: String,
    pub c_zstd: CZstd,
    pub corpus: CorpusId,
    pub level: i32,
    pub c_flag: &'static str,
    pub src_bytes: u64,
    pub us_compressed_bytes: u64,
    pub c_compressed_bytes: u64,
    pub us_over_c: f64,
    pub us_compress_mbps: f64,
    pub us_decompress_mbps: f64,
    pub c_compress_mbps: f64,
    pub c_decompress_mbps: f64,
    pub compress_c_over_us: f64,
    pub decompress_c_over_us: f64,
    pub us_loops: u32,
    pub us_cores_busy: Option<f64>,
    pub c_cores_busy: Option<f64>,
    pub us_roundtrip_ok: bool,
    pub c_decode_us_ok: bool,
    pub method: String,
    pub gates: Gates,
    pub notes: &'static str,
    /// Which estimator produced `us_*_mbps` / `c_*_mbps`.
    ///
    /// `best_of_n` is the only value that may be quoted in a `C/us` ratio:
    /// facebook/zstd `-b` reports its fastest round, so a mean on our side is
    /// a systematic bias in C's favour. Rows written before 2026-08-14 carry
    /// no `estimator` field and were `us=mean_of_n` vs `c=best_of_n`.
    pub estimator: &'static str,
    /// Mean-rate figures, retained for audit and for the spread.
    pub us_compress_mbps_mean: Option<f64>,
    pub us_decompress_mbps_mean: Option<f64>,
    /// Same-arm spread: `|u1 - u2| / min(u1, u2)` over the two ABBA `us` arms,
    /// which run IDENTICAL code. This is the per-file, per-session
    /// reproducibility floor and it **replaces the null arm**, which failed to
    /// flag four separate contaminated sessions (it read 1.0162 / 0.9918 /
    /// 1.0344 / 0.9382 while C's own binary moved 20-27%). A brick delta
    /// smaller than this number on the same file is not a result.
    pub us_compress_same_arm_spread: Option<f64>,
    pub us_decompress_same_arm_spread: Option<f64>,
    /// CPU cycles per input byte, from the fastest loop.
    ///
    /// **The cross-session progress metric.** Frequency-invariant, so it is
    /// immune to the mid-session thermal throttling that moves every MB/s
    /// figure on this box by up to 1.87x. Lower is better. `None` off Windows.
    pub us_compress_cycles_per_byte: Option<f64>,
    pub us_decompress_cycles_per_byte: Option<f64>,
    /// Peak working set of the in-process `us` arm, bytes. Monotonic across
    /// the run, so it is a high-water mark, not a per-file figure. Mission 7
    /// wants <= 1.2x C at the same windowLog; `c_peak_rss_bytes` is the C
    /// child's own peak for the same (file, level).
    pub us_peak_rss_bytes: Option<u64>,
    pub c_peak_rss_bytes: Option<u64>,
}

pub fn append_jsonl<T: Serialize>(path: &Path, row: &T) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let mut f = OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)
        .map_err(|e| format!("open {}: {e}", path.display()))?;
    serde_json::to_writer(&mut f, row).map_err(|e| e.to_string())?;
    f.write_all(b"\n").map_err(|e| e.to_string())?;
    Ok(())
}
