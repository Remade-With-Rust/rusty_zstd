//! GATE OUTCOME BOARD — full encode AND decode, all 18 corpora, one level,
//! inside a fixed time budget.
//!
//! Run after every gate that lands a CONSTANT or a DISPATCH, so the ledger
//! carries a real before/after instead of a placeholder.
//!
//! Budgeted, not fixed-N: each phase runs until it has either `MAX_ITERS`
//! samples or has spent `BUDGET_MS`, minimum one. That is what keeps L19 and
//! L22 (seconds per pass) in the same wall-clock envelope as L1 (milliseconds).
//! Best-of-N is reported, which is the campaign's standing estimator.
//!
//! The C arm is the same oracle the speed boards use: `zstd -b<lvl> -i1 -T1`,
//! which does its own internal best-of inside one second and reports both
//! phases. No `--check`, matching libzstd's `ZSTD_c_checksumFlag = 0` default.
use std::io::Write;
use std::process::Command;
use std::time::Instant;

const MAX_ITERS: usize = 9;
const BUDGET_MS: f64 = 350.0;

fn best_of<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let t0 = Instant::now();
    let mut best = f64::MAX;
    let mut out = 0usize;
    for _ in 0..MAX_ITERS {
        let t = Instant::now();
        out = f();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
        if t0.elapsed().as_secs_f64() * 1000.0 > BUDGET_MS {
            break;
        }
    }
    (best, out)
}

/// `zstd -b` prints e.g. " 1#file : 8388608 -> 2345678 (3.577), 123.4 MB/s, 456.7 MB/s"
fn c_arm(zstd: &str, path: &str, lvl: i32) -> Option<(f64, f64, usize)> {
    let out = Command::new(zstd)
        .args(["--ultra", &format!("-b{lvl}"), "-i1", "-T1", path])
        .output()
        .ok()?;
    let raw = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout);
    // `zstd -b` rewrites ONE line with CR progress updates, so `.lines()` sees
    // a single concatenated blob. Split on any control char and keep the last
    // the only one carrying both MB/s figures.
    let line = raw
        .split(|c: char| c.is_control())
        .filter(|l| l.matches("MB/s").count() >= 2 && l.contains("->"))
        .next_back()?
        .to_string();
    let after = line.split("->").nth(1)?;
    let csize: usize = after.split_whitespace().next()?.parse().ok()?;
    // the two "<num> MB/s" fields, in order: compress then decompress
    let mut speeds = Vec::new();
    for seg in line.split("MB/s") {
        if let Some(tok) = seg.split_whitespace().next_back() {
            if let Ok(v) = tok.parse::<f64>() {
                speeds.push(v);
            }
        }
    }
    if speeds.len() < 2 {
        return None;
    }
    Some((speeds[0], speeds[1], csize))
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let cap: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 * 1024 * 1024);
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let ids = [
        "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m",
        "mr", "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci",
        "samba", "xml", "x-ray",
    ];
    let t_all = Instant::now();
    println!("GATE OUTCOME BOARD — L{lvl}, full encode+decode, prefix {} MiB", cap / 1048576);
    println!("| corpus       |  C comp | us comp | C/us c | C decomp | us decomp | C/us d | us/c size |");
    println!("| ------------ | ------: | ------: | -----: | -------: | --------: | -----: | --------: |");
    let mut rows: Vec<(String, f64, f64, f64, f64, f64, f64, f64)> = Vec::new();
    for id in ids {
        let full = match std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        let src = &full[..full.len().min(cap)];
        // the C oracle must see the SAME bytes -> write the prefix out
        let tmp = format!("target/_gb_{id}.bin");
        if std::fs::write(&tmp, src).is_err() {
            continue;
        }
        let mb = src.len() as f64 / 1_048_576.0;

        let (cms, csz) = best_of(|| rusty_zstd::compress(src, lvl).unwrap().len());
        let z = rusty_zstd::compress(src, lvl).unwrap();
        let mut dst = Vec::with_capacity(src.len());
        let (dms, _) = best_of(|| {
            dst.clear();
            rusty_zstd::decompress_into(&mut dst, &z).unwrap();
            dst.len()
        });
        assert_eq!(dst, src, "{id}: round-trip FAILED at L{lvl}");

        let us_c = mb / (cms / 1000.0);
        let us_d = mb / (dms / 1000.0);
        let Some((cc, cd, ccsz)) = c_arm(zstd, &tmp, lvl) else {
            let _ = std::fs::remove_file(&tmp);
            continue;
        };
        let _ = std::fs::remove_file(&tmp);
        let ratio = csz as f64 / ccsz as f64;
        println!(
            "| {id:<12} | {cc:>7.1} | {us_c:>7.1} | {:>6.2} | {cd:>8.1} | {us_d:>9.1} | {:>6.2} | {ratio:>9.3} |",
            cc / us_c,
            cd / us_d
        );
        let _ = std::io::stdout().flush();
        rows.push((id.into(), cc, us_c, cc / us_c, cd, us_d, cd / us_d, ratio));
    }
    let n = rows.len() as f64;
    println!(
        "\n{} corpora in {:.1}s | mean C/us comp {:.2} decomp {:.2} | ratio {:.3} | worst ratio {:.3}",
        rows.len(),
        t_all.elapsed().as_secs_f64(),
        rows.iter().map(|r| r.3).sum::<f64>() / n,
        rows.iter().map(|r| r.6).sum::<f64>() / n,
        rows.iter().map(|r| r.7).sum::<f64>() / n,
        rows.iter().map(|r| r.7).fold(0.0, f64::max)
    );
}
