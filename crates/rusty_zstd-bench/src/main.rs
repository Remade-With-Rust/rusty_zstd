//! rusty_zstd-bench -- M0 harness.
//!
//! Shells out to a **pinned facebook/zstd CLI**. Never links libzstd.
//! `--baseline-only` writes C numbers into `bench/ledger.jsonl`.

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

mod corpus;
mod gate1;
mod ledger;
mod measure;
mod oracle;

use std::env;
use std::fs::File;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::corpus::{ensure_generated, list_silesia, GeneratedFile};
use crate::ledger::{
    append_jsonl, BaselineLine, CBench, CZstd, CorpusId, Gates, RatioLine, SessionLine, SpeedLine,
};
use crate::measure::{
    current_peak_rss, oneshot_roundtrip, parse_zstd_bench, pin_command, pin_current_process,
    process_cpu_ms,
};
use crate::oracle::{find_oracle, Oracle};

/// Reset only the counter block (the stage timers keep their dump).
fn prof_reset_counts() {
    rusty_zstd::prof_reset();
}

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/rusty_zstd-bench -> repo root")
        .to_path_buf()
}

fn usage() {
    eprintln!("rzstd-bench -- C libzstd baseline harness (never links C)");
    eprintln!();
    eprintln!("Usage:");
    eprintln!("  rzstd-bench --baseline-only [--smoke] [--levels 1,3]");
    eprintln!("  rzstd-bench --m2-ratio [--smoke] [--levels -7,-3,-1,1,2,3]");
    eprintln!("  rzstd-bench --m7-speed [--smoke] [--levels 1,-1,-4,3]");
    eprintln!("  rzstd-bench --m7-profile [--levels 1,-1,-4,3]");
    eprintln!("  rzstd-bench --m7-harvest [--levels 1,-1,-4]  # per-block CSV; Silesia only");
    eprintln!("  rzstd-bench --ab-tag [--levels 1] [--files a,b]  # in-process ABBA arm A/B");
    eprintln!();
    eprintln!("Oracle: set RUSTY_ZSTD_ORACLE or run scripts/fetch-oracle.ps1");
}

fn git_sha(root: &Path) -> Option<String> {
    let out = std::process::Command::new("git")
        .args(["rev-parse", "HEAD"])
        .current_dir(root)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let s = String::from_utf8(out.stdout).ok()?.trim().to_string();
    if s.is_empty() {
        None
    } else {
        Some(s)
    }
}

fn now_rfc3339() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("{secs}Z")
}

fn parse_levels(raw: Option<&str>, smoke: bool, default: &str) -> Vec<i32> {
    if smoke {
        return vec![1];
    }
    let s = raw.unwrap_or(default);
    s.split(',').filter_map(|p| p.trim().parse().ok()).collect()
}

fn c_level_flag(level: i32) -> String {
    if level < 0 {
        format!("--fast={}", -level)
    } else {
        format!("-{level}")
    }
}

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|a| a == "-h" || a == "--help") {
        usage();
        return ExitCode::SUCCESS;
    }
    let baseline = args.iter().any(|a| a == "--baseline-only");
    let m2_ratio = args.iter().any(|a| a == "--m2-ratio");
    let m7_speed = args.iter().any(|a| a == "--m7-speed");
    let m7_profile = args.iter().any(|a| a == "--m7-profile");
    let m7_harvest = args.iter().any(|a| a == "--m7-harvest");
    let ab_tag = args.iter().any(|a| a == "--ab-tag");
    let gg_matchfind = args.iter().any(|a| a == "--gg-matchfind");
    let gg_gate1 = args.iter().any(|a| a == "--gg-gate1");
    if !baseline && !m2_ratio && !m7_speed && !m7_profile && !m7_harvest && !ab_tag && !gg_matchfind && !gg_gate1
    {
        usage();
        return ExitCode::from(2);
    }
    let smoke = args.iter().any(|a| a == "--smoke");
    // Restrict the board to named corpora. A brick A/B should measure its
    // canary files with adequate N, not heat the box through all 12 Silesia
    // files; the full board is for standing / re-baseline runs.
    let only: Vec<String> = args
        .windows(2)
        .find(|w| w[0] == "--files")
        .map(|w| w[1].split(',').map(|s| s.trim().to_string()).collect())
        .unwrap_or_default();
    let levels_raw = args
        .windows(2)
        .find(|w| w[0] == "--levels")
        .map(|w| w[1].as_str());
    let default_levels = if m7_speed || m7_profile || m7_harvest {
        "1,-1,-4,3"
    } else if m2_ratio {
        "-7,-3,-1,1,2,3"
    } else {
        "1,3"
    };
    let levels = parse_levels(levels_raw, smoke && !m7_speed, default_levels);
    if levels.is_empty() {
        eprintln!("no compression levels");
        return ExitCode::from(2);
    }

    let root = repo_root();
    let oracle = match find_oracle(&root) {
        Ok(o) => o,
        Err(e) => {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    };

    let gen_dir = root.join("corpora").join("data").join("generated");
    let files = match ensure_generated(&gen_dir, smoke) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("corpus: {e}");
            return ExitCode::from(1);
        }
    };

    if m7_profile || m7_harvest {
        let harvest_out = args
            .windows(2)
            .find(|w| w[0] == "--harvest-out")
            .map(|w| PathBuf::from(&w[1]))
            .or_else(|| {
                m7_harvest.then(|| {
                    root.join("_greatgate")
                        .join("harvests")
                        .join("silesia-profile.csv")
                })
            });
        return run_m7_profile(&root, &oracle, &files, &levels, harvest_out.as_deref());
    }
    if gg_gate1 {
        let mut all = files.clone();
        all.extend(corpus::list_silesia(&root));
        let out = args
            .windows(2)
            .find(|w| w[0] == "--harvest-out")
            .map(|w| PathBuf::from(&w[1]))
            .unwrap_or_else(|| root.join("_greatgate").join("harvests").join("gate1.csv"));
        pin_current_process();
        return gate1::run(&all, *levels.first().unwrap_or(&1), &only, &out);
    }
    if gg_matchfind {
        let out = args
            .windows(2)
            .find(|w| w[0] == "--harvest-out")
            .map(|w| PathBuf::from(&w[1]))
            .unwrap_or_else(|| {
                root.join("_greatgate")
                    .join("harvests")
                    .join("gg-matchfind.csv")
            });
        // All 18: the 6 generated classes plus the 12 Silesia files. The gate
        // must clear the WORST corpus, so the harvest must contain them all.
        let mut all = files.clone();
        all.extend(corpus::list_silesia(&root));
        let gate = args
            .windows(2)
            .find(|w| w[0] == "--gate")
            .map(|w| w[1].as_str())
            .unwrap_or("step0");
        return run_gg_matchfind(&all, &levels, &only, &out, gate);
    }
    if ab_tag {
        return run_ab_tag(&root, &oracle, &levels, &only);
    }
    if m7_speed {
        return run_m7_speed(&root, &oracle, &files, &levels, smoke, &only);
    }
    if m2_ratio {
        return run_m2_ratio(&root, &oracle, &files, &levels, smoke);
    }

    let bench_secs: u32 = if smoke { 1 } else { 3 };
    let ledger_path = root.join("bench").join("ledger.jsonl");

    println!("oracle {} ({})", oracle.version_line, oracle.path.display());
    println!(
        "method pinned=yes affinity=4 priority=High cpu+wall zstd_-b_i={bench_secs} oneshot=yes T1"
    );

    let null_arm = match null_arm_floor(&oracle, &files[0], levels[0], bench_secs) {
        Ok(r) => r,
        Err(e) => {
            eprintln!("null-arm: {e}");
            return ExitCode::from(1);
        }
    };
    println!(
        "null-arm C vs C compress_MBps ratio = {:.4} (session floor)",
        null_arm
    );

    let session = SessionLine {
        kind: "session",
        ts: now_rfc3339(),
        git_sha: git_sha(&root),
        host: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        c_zstd: CZstd::from_oracle(&oracle),
        method: format!(
            "pinned=yes affinity=4 priority=High cpu+wall zstd_-b_i={bench_secs} oneshot=yes threads=T1 null_arm={null_arm:.4}"
        ),
        null_arm_compress_mbps_ratio: null_arm,
        notes: if smoke {
            "M0 smoke; not a standing speed number"
        } else {
            "M0 baseline-only; no rusty_zstd codec arm"
        },
    };
    if let Err(e) = append_jsonl(&ledger_path, &session) {
        eprintln!("ledger: {e}");
        return ExitCode::from(1);
    }

    for file in &files {
        for &level in &levels {
            match run_one(&oracle, file, level, bench_secs, &session) {
                Ok(line) => {
                    println!(
                        "{id} L{level}: {src} -> {dst} ratio={ratio:.4}  c_bench {c:.1}/{d:.1} MB/s  roundtrip={ok}",
                        id = file.id,
                        src = line.src_bytes,
                        dst = line.compressed_bytes,
                        ratio = line.ratio,
                        c = line.c_bench.compress_mbps,
                        d = line.c_bench.decompress_mbps,
                        ok = line.roundtrip_ok
                    );
                    if let Err(e) = append_jsonl(&ledger_path, &line) {
                        eprintln!("ledger: {e}");
                        return ExitCode::from(1);
                    }
                    if !line.roundtrip_ok {
                        eprintln!("correctness gate failed on {}", file.id);
                        return ExitCode::from(1);
                    }
                }
                Err(e) => {
                    eprintln!("{} L{}: {e}", file.id, level);
                    return ExitCode::from(1);
                }
            }
        }
    }

    println!("ledger {}", ledger_path.display());
    ExitCode::SUCCESS
}

fn null_arm_floor(
    oracle: &Oracle,
    file: &GeneratedFile,
    level: i32,
    bench_secs: u32,
) -> Result<f64, String> {
    let a = zstd_b(oracle, &file.path, level, bench_secs)?;
    let b = zstd_b(oracle, &file.path, level, bench_secs)?;
    if a.compress_mbps <= 0.0 || b.compress_mbps <= 0.0 {
        return Err("zstd -b returned non-positive compress MB/s".into());
    }
    Ok(a.compress_mbps / b.compress_mbps)
}

/// Returns C's `-b` figures plus the child's peak working set.
fn zstd_b_rss(
    oracle: &Oracle,
    src: &Path,
    level: i32,
    bench_secs: u32,
) -> Result<(CBench, Option<u64>), String> {
    let rss = std::cell::Cell::new(None);
    let b = zstd_b_inner(oracle, src, level, bench_secs, &rss)?;
    Ok((b, rss.get()))
}

fn zstd_b(oracle: &Oracle, src: &Path, level: i32, bench_secs: u32) -> Result<CBench, String> {
    zstd_b_inner(oracle, src, level, bench_secs, &std::cell::Cell::new(None))
}

fn zstd_b_inner(
    oracle: &Oracle,
    src: &Path,
    level: i32,
    bench_secs: u32,
    rss_out: &std::cell::Cell<Option<u64>>,
) -> Result<CBench, String> {
    // Windows zstd.exe rejects a glued `-b-1` token. Negative levels use `--fast=N -b`.
    let mut args: Vec<String> = if level < 0 {
        vec![format!("--fast={}", -level), "-b".into()]
    } else {
        vec![format!("-b{level}")]
    };
    args.push(format!("-i{bench_secs}"));
    args.push("-T1".into());
    args.push("--no-progress".into());
    args.push(src.to_string_lossy().into_owned());
    let sample = pin_command(&oracle.path, &args, None)?;
    if sample.status != 0 {
        return Err(format!(
            "zstd -b exit {}: {}",
            sample.status,
            sample.stderr.trim()
        ));
    }
    rss_out.set(sample.peak_rss_bytes);
    parse_zstd_bench(&sample.stdout)
}

fn run_one(
    oracle: &Oracle,
    file: &GeneratedFile,
    level: i32,
    bench_secs: u32,
    session: &SessionLine,
) -> Result<BaselineLine, String> {
    let c_bench = zstd_b(oracle, &file.path, level, bench_secs)?;
    let rt = oneshot_roundtrip(oracle, &file.path, level)?;
    let ratio = if file.bytes == 0 {
        0.0
    } else {
        rt.compressed_bytes as f64 / file.bytes as f64
    };
    Ok(BaselineLine {
        kind: "baseline",
        ts: now_rfc3339(),
        git_sha: session.git_sha.clone(),
        host: session.host.clone(),
        c_zstd: session.c_zstd.clone(),
        corpus: CorpusId {
            id: file.id.clone(),
            split: file.split,
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        },
        level,
        src_bytes: file.bytes,
        compressed_bytes: rt.compressed_bytes,
        ratio,
        roundtrip_ok: rt.ok,
        c_bench,
        oneshot: rt.oneshot,
        method: session.method.clone(),
        gates: Gates {
            correctness: if rt.ok { "pass" } else { "fail" },
            ratio: "baseline",
            speed: "baseline",
            footprint: "baseline",
        },
        notes: session.notes,
    })
}

fn run_m2_ratio(
    root: &Path,
    oracle: &Oracle,
    files: &[GeneratedFile],
    levels: &[i32],
    smoke: bool,
) -> ExitCode {
    let ledger_path = root.join("bench").join("ledger.jsonl");
    println!("oracle {} ({})", oracle.version_line, oracle.path.display());
    println!(
        "method m2-ratio in-process rusty_zstd compress; C zstd -d of our frames; C oneshot size"
    );

    let session = SessionLine {
        kind: "session",
        ts: now_rfc3339(),
        git_sha: git_sha(root),
        host: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        c_zstd: CZstd::from_oracle(oracle),
        method: "m2-ratio rusty_zstd::compress vs C oneshot; dual-gate C -d".into(),
        null_arm_compress_mbps_ratio: 0.0,
        notes: if smoke {
            "M2 ratio smoke; not a standing speed number"
        } else {
            "M2 compressor ratio vs C at matched level; Huffman literals still M3"
        },
    };
    if let Err(e) = append_jsonl(&ledger_path, &session) {
        eprintln!("ledger: {e}");
        return ExitCode::from(1);
    }

    for file in files {
        let src = match std::fs::read(&file.path) {
            Ok(b) => b,
            Err(e) => {
                eprintln!("read {}: {e}", file.path.display());
                return ExitCode::from(1);
            }
        };
        for &level in levels {
            match ratio_one(oracle, file, &src, level, &session) {
                Ok(line) => {
                    println!(
                        "{id} L{level}: us={us} c={c} us/c={rel:.3}  us_rt={urt} c_dec={cdec}",
                        id = file.id,
                        us = line.us_compressed_bytes,
                        c = line.c_compressed_bytes,
                        rel = line.us_over_c,
                        urt = line.us_roundtrip_ok,
                        cdec = line.c_decode_us_ok
                    );
                    if let Err(e) = append_jsonl(&ledger_path, &line) {
                        eprintln!("ledger: {e}");
                        return ExitCode::from(1);
                    }
                    if !line.us_roundtrip_ok || !line.c_decode_us_ok {
                        eprintln!("M2 dual gate failed on {} L{level}", file.id);
                        return ExitCode::from(1);
                    }
                }
                Err(e) => {
                    eprintln!("{} L{}: {e}", file.id, level);
                    return ExitCode::from(1);
                }
            }
        }
    }
    println!("ledger {}", ledger_path.display());
    ExitCode::SUCCESS
}

fn ratio_one(
    oracle: &Oracle,
    file: &GeneratedFile,
    src: &[u8],
    level: i32,
    session: &SessionLine,
) -> Result<RatioLine, String> {
    let us_zst = rusty_zstd::compress(src, level).map_err(|e| format!("us compress: {e}"))?;
    let us_back = rusty_zstd::decompress(&us_zst).map_err(|e| format!("us decompress: {e}"))?;
    let us_roundtrip_ok = us_back.as_slice() == src;
    let c_decode_us_ok = c_decompress_buf(oracle, &us_zst).is_ok_and(|v| v.as_slice() == src);
    let c_bytes = c_compress_size(oracle, &file.path, level)?;
    let src_n = src.len() as u64;
    let us_n = us_zst.len() as u64;
    let us_ratio = if src_n == 0 {
        0.0
    } else {
        us_n as f64 / src_n as f64
    };
    let c_ratio = if src_n == 0 {
        0.0
    } else {
        c_bytes as f64 / src_n as f64
    };
    let us_over_c = if c_bytes == 0 {
        0.0
    } else {
        us_n as f64 / c_bytes as f64
    };
    Ok(RatioLine {
        kind: "m2_ratio",
        ts: now_rfc3339(),
        git_sha: session.git_sha.clone(),
        host: session.host.clone(),
        c_zstd: session.c_zstd.clone(),
        corpus: CorpusId {
            id: file.id.clone(),
            split: file.split,
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        },
        level,
        src_bytes: src_n,
        us_compressed_bytes: us_n,
        c_compressed_bytes: c_bytes,
        us_ratio,
        c_ratio,
        us_over_c,
        us_roundtrip_ok,
        c_decode_us_ok,
        method: session.method.clone(),
        gates: Gates {
            correctness: if us_roundtrip_ok && c_decode_us_ok {
                "pass"
            } else {
                "fail"
            },
            ratio: "quantified",
            speed: "not_measured",
            footprint: "not_measured",
        },
        notes: session.notes,
    })
}

fn c_compress_size(oracle: &Oracle, src: &Path, level: i32) -> Result<u64, String> {
    let parent = src.parent().ok_or("src has no parent")?;
    let zst = parent.join(format!(
        "{}.m2.L{level}.zst",
        src.file_name().unwrap().to_string_lossy()
    ));
    let args = vec![
        c_level_flag(level),
        "-T1".into(),
        "-f".into(),
        "-q".into(),
        "-o".into(),
        zst.to_string_lossy().into_owned(),
        src.to_string_lossy().into_owned(),
    ];
    let sample = pin_command(&oracle.path, &args, None)?;
    if sample.status != 0 {
        return Err(format!("C compress: {}", sample.stderr.trim()));
    }
    let n = std::fs::metadata(&zst)
        .map_err(|e| format!("stat {}: {e}", zst.display()))?
        .len();
    let _ = std::fs::remove_file(&zst);
    Ok(n)
}

fn c_decompress_buf(oracle: &Oracle, zst: &[u8]) -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let inn = dir.join(format!("rzstd-m2r-{nonce}.zst"));
    let out = dir.join(format!("rzstd-m2r-{nonce}.out"));
    std::fs::write(&inn, zst).map_err(|e| e.to_string())?;
    let args = vec![
        "-d".into(),
        "-f".into(),
        "-q".into(),
        "-o".into(),
        out.to_string_lossy().into_owned(),
        inn.to_string_lossy().into_owned(),
    ];
    let sample = pin_command(&oracle.path, &args, None)?;
    let _ = std::fs::remove_file(&inn);
    if sample.status != 0 {
        let _ = std::fs::remove_file(&out);
        return Err(format!("C -d: {}", sample.stderr.trim()));
    }
    let raw = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    Ok(raw)
}

/// C brag flags: `zstd -1`, `zstd --fast=1`, `zstd --fast=4`.
fn brag_flag(level: i32) -> &'static str {
    match level {
        1 => "zstd -1",
        3 => "zstd -3",
        -1 => "zstd --fast=1",
        -4 => "zstd --fast=4",
        _ => "zstd",
    }
}

/// Per-block Z1 harvest. `gain` = seqs in the block (entropy work if skipped).
/// Legal signals only — never C's raw fraction. HYPOTHESES ONLY.
struct HarvestCsv {
    file: File,
}

impl HarvestCsv {
    fn create(path: &Path) -> Result<Self, String> {
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| format!("mkdir {}: {e}", dir.display()))?;
        }
        let mut file = File::create(path).map_err(|e| format!("create {}: {e}", path.display()))?;
        writeln!(
            file,
            "gain,clip,clip_total,split,work,cpu_ms,shipped,clevel,match_frac,lit_peak,min_gain_frac,nseq,block_len,early_raw"
        )
        .map_err(|e| format!("harvest header: {e}"))?;
        Ok(Self { file })
    }

    fn rows(
        &mut self,
        id: &str,
        split: &str,
        src_len: usize,
        level: i32,
        us_ms: f64,
        taps: &[rusty_zstd::ProfBlockTap],
    ) -> Result<(), String> {
        let n = src_len as f64;
        for t in taps {
            let bl = f64::from(t.block_len);
            let match_frac = if bl > 0.0 {
                f64::from(t.match_bytes) / bl
            } else {
                0.0
            };
            let min_gain_frac = if bl > 0.0 {
                f64::from(t.min_gain) / bl
            } else {
                0.0
            };
            let work = f64::from(t.nseq);
            let cpu = if n > 0.0 { us_ms * bl / n } else { 0.0 };
            writeln!(
                self.file,
                "{work:.4},{id},{src_len},{split},{work:.4},{cpu:.4},{},{level},{match_frac:.6},{},{min_gain_frac:.6},{},{},{}",
                t.early_raw,
                t.lit_peak,
                t.nseq,
                t.block_len,
                t.early_raw
            )
            .map_err(|e| format!("harvest row: {e}"))?;
        }
        Ok(())
    }
}

/// Better of two throughput readings (best-of-N across the ABBA arms).
fn best2(a: f64, b: f64) -> f64 {
    if a >= b {
        a
    } else {
        b
    }
}

fn mean2(a: f64, b: f64) -> f64 {
    (a + b) / 2.0
}

fn cores_busy(cpu_ms: Option<f64>, wall_ms: f64) -> Option<f64> {
    match cpu_ms {
        Some(c) if wall_ms > 0.0 => Some(c / wall_ms),
        _ => None,
    }
}

fn fmt_census(c: rusty_zstd::BlockCensus) -> String {
    format!(
        "rle={} raw={} comp={}  rle_b={} raw_b={} comp_b={}",
        c.rle, c.raw, c.compressed, c.rle_regen, c.raw_bytes, c.compressed_payload
    )
}

fn c_compress_bytes(oracle: &Oracle, src: &[u8], level: i32) -> Result<Vec<u8>, String> {
    let dir = std::env::temp_dir();
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let inn = dir.join(format!("rzstd-m7p-{nonce}.bin"));
    let out = dir.join(format!("rzstd-m7p-{nonce}.zst"));
    std::fs::write(&inn, src).map_err(|e| e.to_string())?;
    let args = vec![
        c_level_flag(level),
        "-T1".into(),
        "-f".into(),
        "-q".into(),
        "-o".into(),
        out.to_string_lossy().into_owned(),
        inn.to_string_lossy().into_owned(),
    ];
    let sample = pin_command(&oracle.path, &args, None)?;
    let _ = std::fs::remove_file(&inn);
    if sample.status != 0 {
        let _ = std::fs::remove_file(&out);
        return Err(format!("C compress: {}", sample.stderr.trim()));
    }
    let zst = std::fs::read(&out).map_err(|e| e.to_string())?;
    let _ = std::fs::remove_file(&out);
    Ok(zst)
}

fn profile_one(
    oracle: &Oracle,
    id: &str,
    split: &str,
    src: &[u8],
    level: i32,
    harvest: Option<&mut HarvestCsv>,
) -> Result<(), String> {
    let flag = brag_flag(level);
    rusty_zstd::prof_reset();
    let t0 = std::time::Instant::now();
    let zst = rusty_zstd::compress(src, level).map_err(|e| format!("us compress: {e}"))?;
    let us_ms = t0.elapsed().as_secs_f64() * 1000.0;
    let counts = rusty_zstd::prof_encode_counts();
    let taps = rusty_zstd::prof_take_block_taps();
    let back = rusty_zstd::decompress(&zst).map_err(|e| format!("{id}: us decompress: {e}"))?;
    if back.as_slice() != src {
        return Err(format!("{id}: roundtrip mismatch"));
    }
    let dump = rusty_zstd::prof_dump();
    let c_ok = c_decompress_buf(oracle, &zst).is_ok_and(|v| v.as_slice() == src);
    if !c_ok {
        return Err(format!("{id}: C -d of us frame failed"));
    }
    let c_zst = c_compress_bytes(oracle, src, level)?;
    let us_c = rusty_zstd::frame_block_census(&zst).map_err(|e| format!("us census: {e}"))?;
    let c_c = rusty_zstd::frame_block_census(&c_zst).map_err(|e| format!("C census: {e}"))?;
    let n = src.len() as f64;
    let probes_per_b = if n > 0.0 {
        counts.hash_probes as f64 / n
    } else {
        0.0
    };
    let match_frac = if n > 0.0 {
        counts.match_bytes as f64 / n
    } else {
        0.0
    };
    let hit_rate = if counts.hash_probes == 0 {
        0.0
    } else {
        counts.probe_hits as f64 / counts.hash_probes as f64
    };
    let rel = if c_zst.is_empty() {
        0.0
    } else {
        zst.len() as f64 / c_zst.len() as f64
    };
    let t_off = std::time::Instant::now();
    let z_off = rusty_zstd::compress_with(
        src,
        rusty_zstd::CompressOptions {
            level,
            checksum: false,
        },
    )
    .map_err(|e| format!("us nocheck: {e}"))?;
    let off_ms = t_off.elapsed().as_secs_f64() * 1000.0;
    println!();
    println!("=== {id} split={split} {flag} src={} ===", src.len());
    println!(
        "COUNTED probes={} hits={} fills={} seqs={} match_b={} lit_b={} scratch={}",
        counts.hash_probes,
        counts.probe_hits,
        counts.hash_fills,
        counts.seqs,
        counts.match_bytes,
        counts.lit_bytes,
        counts.scratch_allocs
    );
    println!(
        "COUNTED probes/byte={probes_per_b:.4} hit_rate={hit_rate:.6} match_frac={match_frac:.4} xxh_b={}",
        counts.checksum_bytes
    );
    println!(
        "COUNTED tables hash={} long={} chain={} unused_long_chain={}",
        counts.table_hash_bytes,
        counts.table_hash_long_bytes,
        counts.table_chain_bytes,
        counts.table_hash_long_bytes + counts.table_chain_bytes
    );
    println!(
        "COUNTED us_blocks {}  C_blocks {}",
        fmt_census(us_c),
        fmt_census(c_c)
    );
    println!(
        "COUNTED us_size={} C_size={} us/c={rel:.3}  rt=true C-d=true early_raw={}",
        zst.len(),
        c_zst.len(),
        counts.early_raw_blocks
    );
    // BIT ACCOUNTANT (codec-analyzer 6): where do our bytes go, and how much
    // of the gap to C is literals coding vs sequence coding?
    //
    // C's split is obtained by decoding C'S OWN frame through our decoder with
    // the same counters -- same units on both sides, so the gap is attributable.
    let emitted = counts.emit_lit_bytes + counts.emit_seq_bytes;
    let (c_lit, c_seq) = {
        crate::prof_reset_counts();
        let _ = rusty_zstd::decompress(&c_zst);
        let cc = rusty_zstd::prof_encode_counts();
        (cc.emit_lit_bytes, cc.emit_seq_bytes)
    };
    println!(
        "COUNTED C_bits lit={c_lit} seq={c_seq}  |  us lit={} seq={}  =>  lit_gap={} seq_gap={}",
        counts.emit_lit_bytes,
        counts.emit_seq_bytes,
        counts.emit_lit_bytes as i64 - c_lit as i64,
        counts.emit_seq_bytes as i64 - c_seq as i64
    );
    let gap = zst.len() as i64 - c_zst.len() as i64;
    println!(
        "COUNTED bits lit={} ({:.1}%) seq={} ({:.1}%) other={} | gap_vs_C={gap} ({:.1}% of us)",
        counts.emit_lit_bytes,
        if emitted > 0 {
            counts.emit_lit_bytes as f64 / emitted as f64 * 100.0
        } else {
            0.0
        },
        counts.emit_seq_bytes,
        if emitted > 0 {
            counts.emit_seq_bytes as f64 / emitted as f64 * 100.0
        } else {
            0.0
        },
        zst.len() as i64 - emitted as i64,
        gap as f64 / zst.len().max(1) as f64 * 100.0
    );
    println!(
        "COUNTED back_ext bytes={} matches={} (of {} hits) mean_when_extended={:.2}",
        counts.back_ext_bytes,
        counts.back_ext_matches,
        counts.probe_hits,
        if counts.back_ext_matches > 0 {
            counts.back_ext_bytes as f64 / counts.back_ext_matches as f64
        } else {
            0.0
        }
    );
    println!(
        "COUNTED seq_modes predef={} rle={} comp={} REPEAT={}",
        counts.seq_modes[0], counts.seq_modes[1], counts.seq_modes[2], counts.seq_modes[3]
    );
    println!(
        "MEASURED us_on={us_ms:.2}ms us_nocheck={off_ms:.2}ms nocheck_size={} (oneshot, not standing)",
        z_off.len()
    );
    print!("{dump}");
    if let Some(h) = harvest {
        h.rows(id, split, src.len(), level, us_ms, &taps)?;
    }
    Ok(())
}

fn run_m7_profile(
    root: &Path,
    oracle: &Oracle,
    files: &[GeneratedFile],
    levels: &[i32],
    harvest_out: Option<&Path>,
) -> ExitCode {
    pin_current_process();
    println!(
        "method m7-profile  us oneshot + C census + decode scopes  pin affinity=4 High  flags=zstd_-1/--fast=1/--fast=4/-3"
    );
    println!("oracle {} ({})", oracle.version_line, oracle.path.display());
    println!("{}", rusty_zstd::prof_dump().trim_end());

    let mut harvest = match harvest_out {
        Some(path) => match HarvestCsv::create(path) {
            Ok(h) => {
                println!("harvest {}", path.display());
                Some(h)
            }
            Err(e) => {
                eprintln!("harvest: {e}");
                return ExitCode::from(1);
            }
        },
        None => None,
    };

    let harvest_only = harvest.is_some();
    if harvest_only {
        println!("harvest is Silesia per-block (generated skipped)");
    }

    if !harvest_only {
        for f in files {
            let src = match std::fs::read(&f.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("read {}: {e}", f.id);
                    return ExitCode::from(1);
                }
            };
            for &level in levels {
                if let Err(e) = profile_one(oracle, &f.id, f.split, &src, level, None) {
                    eprintln!("{e}");
                    return ExitCode::from(1);
                }
            }
        }

        println!("\n=== size sweep (same density, prefixes) ===");
        let sweep_ids = ["text-32m", "incomp-32m"];
        let sweep_ns = [
            256 * 1024usize,
            1024 * 1024,
            4 * 1024 * 1024,
            16 * 1024 * 1024,
            32 * 1024 * 1024,
        ];
        for id in sweep_ids {
            let Some(f) = files.iter().find(|x| x.id == id) else {
                continue;
            };
            let src = match std::fs::read(&f.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("read {}: {e}", f.id);
                    return ExitCode::from(1);
                }
            };
            for &n in &sweep_ns {
                if n > src.len() {
                    continue;
                }
                let slice = &src[..n];
                rusty_zstd::prof_reset();
                let t0 = std::time::Instant::now();
                match rusty_zstd::compress(slice, 1) {
                    Ok(_) => {
                        let ms = t0.elapsed().as_secs_f64() * 1000.0;
                        let mbps = if ms > 0.0 {
                            (n as f64) / (ms / 1000.0) / 1_000_000.0
                        } else {
                            0.0
                        };
                        let c = rusty_zstd::prof_encode_counts();
                        println!(
                        "SWEEP {id} n={n} L1 oneshot_ms={ms:.2} MB/s={mbps:.0} probes={} match_b={} rle/raw/comp={}/{}/{}",
                        c.hash_probes,
                        c.match_bytes,
                        c.rle_blocks,
                        c.raw_blocks,
                        c.comp_blocks
                    );
                    }
                    Err(e) => {
                        eprintln!("sweep {id} n={n}: {e}");
                        return ExitCode::from(1);
                    }
                }
            }
        }
    } // generated + size sweep (not on --m7-harvest)

    let silesia = list_silesia(root);
    if silesia.is_empty() {
        println!("\nSilesia absent (corpora/data/silesia); generated set is the board.");
    } else {
        println!("\n=== Silesia (real content, per file, never averaged) ===");
        for f in &silesia {
            let src = match std::fs::read(&f.path) {
                Ok(b) => b,
                Err(e) => {
                    eprintln!("read silesia {}: {e}", f.id);
                    return ExitCode::from(1);
                }
            };
            for &level in levels {
                if let Err(e) = profile_one(oracle, &f.id, f.split, &src, level, harvest.as_mut()) {
                    println!("FAIL {e}");
                }
            }
        }
    }
    ExitCode::SUCCESS
}

/// Interleave the two brick arms INSIDE one process, alternating per file.
///
/// Each arm is measured seconds apart on the same file instead of minutes
/// apart in separate processes. On this box the separate-process form gave
/// two pairs of the SAME brick reading -36.7% and +2.6%, purely from drift.
/// Order is A,B,B,A per file so a monotone drift cancels to first order.
fn run_ab_tag(root: &Path, _oracle: &Oracle, levels: &[i32], only: &[String]) -> ExitCode {
    let name = std::env::var("RZSTD_AB_ARM").unwrap_or_else(|_| "tag".into());
    let arm: fn(bool) = match name.as_str() {
        "pipe" => rusty_zstd::set_pipe_arm,
        "lut" => rusty_zstd::set_lut_arm,
        "litcopy" => rusty_zstd::set_litcopy_arm,
        "litpush" => rusty_zstd::set_litpush_arm,
        "litpushhoist" => rusty_zstd::set_litpush_hoist_arm,
        "payload" => rusty_zstd::set_payload_arm,
        "matchcopy" => rusty_zstd::set_matchcopy_arm,
        "seqcheck" => rusty_zstd::set_seqcheck_arm,
        "huff" => rusty_zstd::set_huff_fast_arm,
        "lazyfill" => rusty_zstd::set_lazy_fill_arm,
        "rep1" => rusty_zstd::set_rep1_arm,
        _ => rusty_zstd::set_tag_arm,
    };
    pin_current_process();
    let min = std::time::Duration::from_secs(3);
    // The A/B harness used to list Silesia ONLY, which made the generated
    // corpora unreachable -- so any brick whose mechanism lives in RLE blocks
    // or constant-run content (zeros-32m, text-32m) could not be measured here
    // at all, and silently returned a board with those rows simply absent.
    let mut files = list_silesia(root);
    let gen_dir = root.join("corpora").join("data").join("generated");
    if let Ok(g) = ensure_generated(&gen_dir, false) {
        files.extend(g);
    }
    if !only.is_empty() {
        files.retain(|f| only.contains(&f.id));
        // A name that matches nothing must FAIL, not silently shrink the board.
        for want in only {
            if !files.iter().any(|f| &f.id == want) {
                eprintln!("--files: no corpus named {want:?}");
                return ExitCode::from(2);
            }
        }
    }
    let level = levels[0];
    println!("A/B arm={name} in-process ABBA per file, level {level}  (A=on, B=off)");
    println!(
        "{:<10} {:>9} {:>9} {:>8} | {:>9} {:>9} {:>8} | spread",
        "file", "c A", "c B", "c delta", "d A", "d B", "d delta"
    );
    let (mut cw, mut dw, mut n) = (0i32, 0i32, 0i32);
    for f in &files {
        let Ok(src) = std::fs::read(&f.path) else {
            continue;
        };
        let (mut ca, mut cb, mut da, mut db) = (Vec::new(), Vec::new(), Vec::new(), Vec::new());
        for &on in &[true, false, false, true] {
            arm(on);
            match us_arm(&src, level, min) {
                Ok((r, _)) => {
                    let c = rusty_zstd::mbps_best(src.len(), r.compress_best_ms);
                    let d = rusty_zstd::mbps_best(src.len(), r.decompress_best_ms);
                    if on {
                        ca.push(c);
                        da.push(d);
                    } else {
                        cb.push(c);
                        db.push(d);
                    }
                }
                Err(e) => {
                    eprintln!("{}: {e}", f.id);
                    return ExitCode::from(1);
                }
            }
        }
        let m = |v: &Vec<f64>| (v[0] + v[1]) / 2.0;
        let sp = |v: &Vec<f64>| (v[0] - v[1]).abs() / v[0].min(v[1]) * 100.0;
        let (cam, cbm, dam, dbm) = (m(&ca), m(&cb), m(&da), m(&db));
        let spread = sp(&ca).max(sp(&cb)).max(sp(&da)).max(sp(&db));
        let cd = (cam / cbm - 1.0) * 100.0;
        let dd = (dam / dbm - 1.0) * 100.0;
        n += 1;
        if cd > 0.0 {
            cw += 1;
        }
        if dd > 0.0 {
            dw += 1;
        }
        println!(
            "{:<10} {:9.1} {:9.1} {:+7.1}% | {:9.1} {:9.1} {:+7.1}% | {:5.1}%{}",
            f.id,
            cam,
            cbm,
            cd,
            dam,
            dbm,
            dd,
            spread,
            if spread > 5.0 { "  NOISY" } else { "" }
        );
    }
    if n > 0 {
        let z = |w: i32| (f64::from(w) - f64::from(n) / 2.0) / (0.5 * f64::from(n).sqrt());
        println!(
            "A wins compress {cw}/{n} (z={:+.2})   decompress {dw}/{n} (z={:+.2})   |z|>2 = verdict",
            z(cw),
            z(dw)
        );
    }
    ExitCode::SUCCESS
}

fn run_m7_speed(
    root: &Path,
    oracle: &Oracle,
    files: &[GeneratedFile],
    levels: &[i32],
    smoke: bool,
    only: &[String],
) -> ExitCode {
    pin_current_process();
    let keep = |v: &[GeneratedFile]| -> Vec<GeneratedFile> {
        if only.is_empty() {
            v.to_vec()
        } else {
            v.iter().filter(|f| only.contains(&f.id)).cloned().collect()
        }
    };
    let bench_secs: u32 = if smoke { 0 } else { 3 };
    let min = std::time::Duration::from_secs(u64::from(bench_secs));
    let ledger_path = root.join("bench").join("ledger.jsonl");

    println!("oracle {} ({})", oracle.version_line, oracle.path.display());
    println!(
        "method m7-speed ABBA C,us,us,C  flags=zstd_-1/--fast=1/--fast=4/-3  us=in-process rusty_zstd::bench_roundtrip  C=zstd_-b_-T1  pin affinity=4 High  i={bench_secs}"
    );

    let src0 = match std::fs::read(&files[0].path) {
        Ok(b) => b,
        Err(e) => {
            eprintln!("read: {e}");
            return ExitCode::from(1);
        }
    };
    let null_arm = match rusty_zstd::bench_roundtrip(&src0, levels[0], min) {
        Ok(a) => match rusty_zstd::bench_roundtrip(&src0, levels[0], min) {
            Ok(b) => {
                let ma = rusty_zstd::mbps_best(src0.len(), a.compress_best_ms);
                let mb = rusty_zstd::mbps_best(src0.len(), b.compress_best_ms);
                if mb <= 0.0 {
                    0.0
                } else {
                    ma / mb
                }
            }
            Err(e) => {
                eprintln!("null-arm us: {e}");
                return ExitCode::from(1);
            }
        },
        Err(e) => {
            eprintln!("null-arm us: {e}");
            return ExitCode::from(1);
        }
    };
    println!("null-arm us vs us compress_MBps ratio = {null_arm:.4} (session floor)");

    let session = SessionLine {
        kind: "session",
        ts: now_rfc3339(),
        git_sha: git_sha(root),
        host: format!("{}-{}", env::consts::OS, env::consts::ARCH),
        c_zstd: CZstd::from_oracle(oracle),
        method: format!(
            "m7-speed ABBA pinned=yes affinity=4 us=in-process C=zstd_-b_T1 i={bench_secs} estimator=best_of_n(both_arms) timer=wall null_arm={null_arm:.4} flags=-1,--fast=1,--fast=4,-3"
        ),
        null_arm_compress_mbps_ratio: null_arm,
        notes: if smoke {
            "M7 speed smoke; not a standing number"
        } else {
            "M7 speed vs C 1.5.7 brag flags; not an exit claim"
        },
    };
    if let Err(e) = append_jsonl(&ledger_path, &session) {
        eprintln!("ledger: {e}");
        return ExitCode::from(1);
    }

    // WARMUP, discarded. The first file measured in a session absorbs cold
    // caches and the CPU frequency ramp: sao (measured first) swung 5.102 ->
    // 8.322 cyc/byte for byte-identical code while later files in the same
    // runs held to 3-8%. Burn that on a throwaway pass.
    {
        let warm = std::time::Duration::from_millis(1500);
        let _ = rusty_zstd::bench_roundtrip(&src0[..src0.len().min(4 << 20)], levels[0], warm);
        println!("warmup discarded (first-file cold-start / frequency ramp)");
    }

    let gen_set = keep(files);
    if !gen_set.is_empty() {
        if let Err(e) = m7_speed_files(
            oracle,
            &gen_set,
            levels,
            bench_secs,
            min,
            &session,
            &ledger_path,
        ) {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    }

    let silesia = keep(&list_silesia(root));
    if silesia.is_empty() {
        println!("Silesia absent (corpora/data/silesia); generated set is the board.");
    } else {
        println!("\n=== Silesia (real content, per file, never averaged) ===");
        if let Err(e) = m7_speed_files(
            oracle,
            &silesia,
            levels,
            bench_secs,
            min,
            &session,
            &ledger_path,
        ) {
            eprintln!("{e}");
            return ExitCode::from(1);
        }
    }
    println!("ledger {}", ledger_path.display());
    ExitCode::SUCCESS
}

fn m7_speed_files(
    oracle: &Oracle,
    files: &[GeneratedFile],
    levels: &[i32],
    bench_secs: u32,
    min: std::time::Duration,
    session: &SessionLine,
    ledger_path: &Path,
) -> Result<(), String> {
    for file in files {
        let src =
            std::fs::read(&file.path).map_err(|e| format!("read {}: {e}", file.path.display()))?;
        for &level in levels {
            let line = m7_one(oracle, file, &src, level, bench_secs, min, session)
                .map_err(|e| format!("{} L{}: {e}", file.id, level))?;
            println!(
                "{id} {flag}: C {cc:.1}/{cd:.1}  us {uc:.1}/{ud:.1} MB/s  C/us {cx:.2}/{dx:.2}  size us/c={rel:.3}  rt={ok} C-d={cdok}",
                id = file.id,
                flag = line.c_flag,
                cc = line.c_compress_mbps,
                cd = line.c_decompress_mbps,
                uc = line.us_compress_mbps,
                ud = line.us_decompress_mbps,
                cx = line.compress_c_over_us,
                dx = line.decompress_c_over_us,
                rel = line.us_over_c,
                ok = line.us_roundtrip_ok,
                cdok = line.c_decode_us_ok
            );
            if let Some(cb) = line.us_cores_busy {
                print!("  cores-busy us={cb:.2}");
            }
            if let (Some(cc), Some(dc)) = (
                line.us_compress_cycles_per_byte,
                line.us_decompress_cycles_per_byte,
            ) {
                print!("  cyc/byte c={cc:.3} d={dc:.3}");
            }
            if let (Some(sc), Some(sd)) = (
                line.us_compress_same_arm_spread,
                line.us_decompress_same_arm_spread,
            ) {
                print!("  same-arm c={:.1}% d={:.1}%", sc * 100.0, sd * 100.0);
            }
            println!();
            append_jsonl(ledger_path, &line).map_err(|e| format!("ledger: {e}"))?;
            if !line.us_roundtrip_ok || !line.c_decode_us_ok {
                return Err(format!(
                    "M7 dual gate failed on {} {}",
                    file.id, line.c_flag
                ));
            }
        }
    }
    Ok(())
}

fn m7_one(
    oracle: &Oracle,
    file: &GeneratedFile,
    src: &[u8],
    level: i32,
    bench_secs: u32,
    min: std::time::Duration,
    session: &SessionLine,
) -> Result<SpeedLine, String> {
    let flag = brag_flag(level);
    // ABBA: C, us, us, C
    let (c1, c_rss) = zstd_b_rss(oracle, &file.path, level, bench_secs.max(1))?;
    let (u1, cpu1) = us_arm(src, level, min)?;
    let (u2, cpu2) = us_arm(src, level, min)?;
    let c2 = zstd_b(oracle, &file.path, level, bench_secs.max(1))?;

    // ESTIMATOR PARITY (codec-measurement 4/8). C `-b` reports its FASTEST
    // round, so `us` must also report its fastest, and across the two ABBA
    // arms we take the better of the two on BOTH sides. Quoting our mean
    // against C's best understates us by the width of our own loop spread.
    let us_c_mbps = best2(
        rusty_zstd::mbps_best(src.len(), u1.compress_best_ms),
        rusty_zstd::mbps_best(src.len(), u2.compress_best_ms),
    );
    let us_d_mbps = best2(
        rusty_zstd::mbps_best(src.len(), u1.decompress_best_ms),
        rusty_zstd::mbps_best(src.len(), u2.decompress_best_ms),
    );
    let c_c_mbps = best2(c1.compress_mbps, c2.compress_mbps);
    let c_d_mbps = best2(c1.decompress_mbps, c2.decompress_mbps);

    // Mean-rate twins, kept for audit: mean/best is this arm's loop spread.
    let us_c_mbps_mean = mean2(
        rusty_zstd::mbps(src.len(), u1.loops, u1.compress_ms),
        rusty_zstd::mbps(src.len(), u2.loops, u2.compress_ms),
    );
    let us_d_mbps_mean = mean2(
        rusty_zstd::mbps(src.len(), u1.loops, u1.decompress_ms),
        rusty_zstd::mbps(src.len(), u2.loops, u2.decompress_ms),
    );

    let zst = rusty_zstd::compress(src, level).map_err(|e| format!("us compress: {e}"))?;
    let back = rusty_zstd::decompress(&zst).map_err(|e| format!("us decompress: {e}"))?;
    let us_roundtrip_ok = back.as_slice() == src;
    let c_decode_us_ok = c_decompress_buf(oracle, &zst).is_ok_and(|v| v.as_slice() == src);
    let c_bytes = c_compress_size(oracle, &file.path, level)?;
    let us_n = zst.len() as u64;
    let us_over_c = if c_bytes == 0 {
        0.0
    } else {
        us_n as f64 / c_bytes as f64
    };

    let us_cpu = match (cpu1, cpu2) {
        (Some(a), Some(b)) => Some(mean2(a, b)),
        _ => None,
    };
    let us_wall = mean2(u1.wall_ms, u2.wall_ms);

    Ok(SpeedLine {
        kind: "m7_speed",
        ts: now_rfc3339(),
        git_sha: session.git_sha.clone(),
        host: session.host.clone(),
        c_zstd: session.c_zstd.clone(),
        corpus: CorpusId {
            id: file.id.clone(),
            split: file.split,
            bytes: file.bytes,
            sha256: file.sha256.clone(),
        },
        level,
        c_flag: flag,
        src_bytes: src.len() as u64,
        us_compressed_bytes: us_n,
        c_compressed_bytes: c_bytes,
        us_over_c,
        us_compress_mbps: us_c_mbps,
        us_decompress_mbps: us_d_mbps,
        c_compress_mbps: c_c_mbps,
        c_decompress_mbps: c_d_mbps,
        compress_c_over_us: if us_c_mbps <= 0.0 {
            0.0
        } else {
            c_c_mbps / us_c_mbps
        },
        decompress_c_over_us: if us_d_mbps <= 0.0 {
            0.0
        } else {
            c_d_mbps / us_d_mbps
        },
        us_loops: u1.loops.saturating_add(u2.loops) / 2,
        us_cores_busy: cores_busy(us_cpu, us_wall),
        c_cores_busy: None,
        us_roundtrip_ok,
        c_decode_us_ok,
        method: session.method.clone(),
        gates: Gates {
            correctness: if us_roundtrip_ok && c_decode_us_ok {
                "pass"
            } else {
                "fail"
            },
            ratio: "quantified",
            speed: "measured_not_exit",
            footprint: "not_measured",
        },
        notes: session.notes,
        estimator: "best_of_n",
        us_compress_mbps_mean: Some(us_c_mbps_mean),
        us_decompress_mbps_mean: Some(us_d_mbps_mean),
        us_compress_same_arm_spread: spread(
            rusty_zstd::mbps_best(src.len(), u1.compress_best_ms),
            rusty_zstd::mbps_best(src.len(), u2.compress_best_ms),
        ),
        us_decompress_same_arm_spread: spread(
            rusty_zstd::mbps_best(src.len(), u1.decompress_best_ms),
            rusty_zstd::mbps_best(src.len(), u2.decompress_best_ms),
        ),
        us_peak_rss_bytes: current_peak_rss(),
        c_peak_rss_bytes: c_rss,
        us_compress_cycles_per_byte: cpb(u1.compress_best_ticks, u2.compress_best_ticks, src.len()),
        us_decompress_cycles_per_byte: cpb(
            u1.decompress_best_ticks,
            u2.decompress_best_ticks,
            src.len(),
        ),
    })
}

/// Fractional spread between two readings of IDENTICAL code -- the honest
/// per-file noise floor for this session.
fn spread(a: f64, b: f64) -> Option<f64> {
    let lo = a.min(b);
    if lo <= 0.0 {
        return None;
    }
    Some((a - b).abs() / lo)
}

/// Cycles per input byte from the better (fewer-cycle) of the two ABBA arms.
fn cpb(a: u64, b: u64, src_len: usize) -> Option<f64> {
    let ticks = match (a, b) {
        (0, 0) => return None,
        (0, x) | (x, 0) => x,
        (x, y) => x.min(y),
    };
    if src_len == 0 {
        return None;
    }
    Some(ticks as f64 / src_len as f64)
}

fn us_arm(
    src: &[u8],
    level: i32,
    min: std::time::Duration,
) -> Result<(rusty_zstd::InProcessBench, Option<f64>), String> {
    let cpu0 = process_cpu_ms();
    let b = rusty_zstd::bench_roundtrip_clocked(src, level, min, || {
        crate::measure::thread_cycles().unwrap_or(0)
    })
    .map_err(|e| format!("us bench: {e}"))?;
    let cpu1 = process_cpu_ms();
    let cpu_delta = match (cpu0, cpu1) {
        (Some(a), Some(b)) => Some((b - a).max(0.0)),
        _ => None,
    };
    Ok((b, cpu_delta))
}


/// P0/gg-matchfind: the per-BLOCK, two-arm harvest that Gate 9 (probe density)
/// is fitted from.
///
/// Why per block, not per file: Gates 3/9/14 decide per block, and
/// `great-gate.md` law 2 says decide on the unit the metric counts. The existing
/// `--m7-harvest` CSV is file-grain -- a truth table, not a gate fit.
///
/// Why both arms in ONE process: `gain` is a DIFFERENCE, so both arms must be
/// measured, and this box drifts enough that two process runs minutes apart
/// produced -36.7% and +2.6% for the same brick. `set_step0_arm` puts the arms
/// milliseconds apart, ABBA-ordered.
///
/// **Known confound, stated rather than hidden.** The match tables carry across
/// blocks, so block N's gain under the routed arm includes that arm's effect on
/// blocks 0..N-1. This is not removable -- a shipped per-block gate would do the
/// same -- but it means one block's row is not an independent experiment. The
/// verdict rests on the `clip`/`clip_total` macro aggregation, which is exactly
/// why the calculator reports micro AND macro and flags sign disagreements.
/// The gates this harness can A/B. Each names the SHIPPED arm and the ROUTED
/// arm; `gain`/`work`/`cpu_ms` are always ROUTED measured against SHIPPED, so a
/// positive number always means "routing this block wins".
///
/// Adding a gate here is the whole cost of testing it across all 18 corpora --
/// the harvest, the per-block deltas, the split, and the calculator contract are
/// all shared.
fn apply_gate_arm(gate: &str, routed: bool) -> Result<(), String> {
    match gate {
        // Gate 9: probe density. shipped step0 = 2, routed = 1 (C's density).
        "step0" => rusty_zstd::set_step0_arm(if routed { 1 } else { 2 }),
        // Gate 3: the lazy chain back-fill. shipped ON; routed OFF.
        // NOTE the direction -- the shipped arm is the RICH one here, so a
        // positive gain means turning the back-fill OFF wins.
        "lazyfill" => rusty_zstd::set_lazy_fill_arm(!routed),
        // Gate 2: repcode-1 search forced on vs left to the measured yield.
        "rep1" => rusty_zstd::set_rep1_arm(routed),
        // Gate 14: chain-walk depth, +1 exponent = twice the candidates.
        "chaindepth" => rusty_zstd::set_search_log_delta(if routed { 1 } else { 0 }),
        // Gate 14, the other direction: half the candidates.
        "chaindepth-down" => rusty_zstd::set_search_log_delta(if routed { -1 } else { 0 }),
        other => {
            return Err(format!(
                "unknown --gate {other}; known: step0, lazyfill, rep1, chaindepth, chaindepth-down"
            ))
        }
    }
    Ok(())
}

/// Restore every arm this harness can touch to its SHIPPED value.
fn reset_gate_arms() {
    rusty_zstd::set_step0_arm(2);
    rusty_zstd::set_lazy_fill_arm(true);
    rusty_zstd::set_rep1_arm(false);
    rusty_zstd::set_search_log_delta(0);
}

fn run_gg_matchfind(
    files: &[corpus::GeneratedFile],
    levels: &[i32],
    only: &[String],
    out_path: &Path,
    gate: &str,
) -> ExitCode {
    pin_current_process();
    if let Err(e) = apply_gate_arm(gate, false) {
        eprintln!("{e}");
        return ExitCode::from(2);
    }
    reset_gate_arms();
    println!("gg-matchfind gate={gate}  shipped-arm vs routed-arm, ABBA per file");
    if !cfg!(feature = "profile") {
        eprintln!("--gg-matchfind requires --features rusty_zstd/profile (per-block taps are off)");
        return ExitCode::from(2);
    }
    if let Some(dir) = out_path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let mut w = match std::fs::File::create(out_path) {
        Ok(f) => std::io::BufWriter::new(f),
        Err(e) => {
            eprintln!("create {}: {e}", out_path.display());
            return ExitCode::from(1);
        }
    };
    // gain / work / cpu_ms are all signed SAVINGS of the routed arm (step0 = 1)
    // against the shipped arm (step0 = 2). Positive = routing this block wins.
    if writeln!(
        w,
        "gain,clip,clip_total,split,work,cpu_ms,shipped,clevel,block_idx,match_frac,lit_share,nseq_per_kb,hit_rate,probes_per_byte,rep_yield,off_collision,off_buckets,lit_peak,early_raw,csize_shipped,csize_routed"
    )
    .is_err()
    {
        eprintln!("write header failed");
        return ExitCode::from(1);
    }
    let mut rows = 0usize;
    let mut matched_any = false;
    for f in files {
        if !only.is_empty() && !only.iter().any(|o| o == &f.id) {
            continue;
        }
        matched_any = true;
        let src = match std::fs::read(&f.path) {
            Ok(v) => v,
            Err(e) => {
                eprintln!("read {}: {e}", f.path.display());
                return ExitCode::from(1);
            }
        };
        for &lvl in levels {
            // ABBA inside the file; keep the SECOND pass of each arm so both are
            // equally warm.
            let mut taps_ship: Vec<rusty_zstd::ProfBlockTap> = Vec::new();
            let mut taps_rout: Vec<rusty_zstd::ProfBlockTap> = Vec::new();
            for pass in 0..4 {
                // ABBA: shipped, routed, routed, shipped.
                let routed = pass == 1 || pass == 2;
                if apply_gate_arm(gate, routed).is_err() {
                    return ExitCode::from(2);
                }
                rusty_zstd::prof_reset();
                if rusty_zstd::compress(&src, lvl).is_err() {
                    eprintln!("{} L{lvl}: compress failed", f.id);
                    return ExitCode::from(1);
                }
                let t = rusty_zstd::prof_take_block_taps();
                if pass == 3 {
                    taps_ship = t;
                } else if pass == 2 {
                    taps_rout = t;
                }
            }
            reset_gate_arms();
            if taps_ship.len() != taps_rout.len() {
                eprintln!(
                    "{} L{lvl}: block count differs between arms ({} vs {}) -- rows would not align, refusing",
                    f.id,
                    taps_ship.len(),
                    taps_rout.len()
                );
                return ExitCode::from(3);
            }
            let clip_total: u64 = taps_ship.iter().map(|t| u64::from(t.csize)).sum();
            let routed_total: u64 = taps_rout.iter().map(|t| u64::from(t.csize)).sum();
            let (mut pp, mut hp, mut np) = (0u64, 0u64, 0u64);
            let (mut pr, mut nr) = (0u64, 0u64);
            for (i, (a, b)) in taps_ship.iter().zip(taps_rout.iter()).enumerate() {
                let probes_a = a.probes.saturating_sub(pp);
                let hits_a = a.hits.saturating_sub(hp);
                let ns_a = a.mf_ns.saturating_sub(np);
                pp = a.probes;
                hp = a.hits;
                np = a.mf_ns;
                let probes_b = b.probes.saturating_sub(pr);
                let ns_b = b.mf_ns.saturating_sub(nr);
                pr = b.probes;
                nr = b.mf_ns;

                let bl = f64::from(a.block_len).max(1.0);
                let gain = f64::from(a.csize) - f64::from(b.csize);
                let work = probes_a as f64 - probes_b as f64;
                let cpu_ms = (ns_a as f64 - ns_b as f64) / 1.0e6;
                let hit_rate = if probes_a > 0 {
                    hits_a as f64 / probes_a as f64
                } else {
                    0.0
                };
                if writeln!(
                    w,
                    "{:.4},{},{},{},{:.1},{:.6},0,{},{},{:.6},{:.6},{:.4},{:.6},{:.6},{:.4},{:.4},{},{},{},{},{}",
                    gain,
                    f.id,
                    clip_total,
                    f.split,
                    work,
                    cpu_ms,
                    lvl,
                    i,
                    f64::from(a.match_bytes) / bl,
                    f64::from(a.lit_bytes) / bl,
                    f64::from(a.nseq) * 1024.0 / bl,
                    hit_rate,
                    probes_a as f64 / bl,
                    f64::from(a.rep_yield_x1000) / 1000.0,
                    f64::from(a.off_collision_x1000) / 1000.0,
                    a.off_buckets,
                    a.lit_peak,
                    a.early_raw,
                    a.csize,
                    b.csize,
                )
                .is_err()
                {
                    eprintln!("write row failed");
                    return ExitCode::from(1);
                }
                rows += 1;
            }
            println!(
                "gg-matchfind[{}] {} L{}: {} blocks  shipped={} routed={} ({:+.3}%)",
                gate,
                f.id,
                lvl,
                taps_ship.len(),
                clip_total,
                routed_total,
                100.0 * (routed_total as f64 - clip_total as f64) / (clip_total.max(1) as f64)
            );
        }
    }
    if !matched_any {
        eprintln!("--files matched no corpus");
        return ExitCode::from(2);
    }
    if w.flush().is_err() {
        eprintln!("flush failed");
        return ExitCode::from(1);
    }
    println!("harvest {} ({} block rows)", out_path.display(), rows);
    ExitCode::SUCCESS
}
