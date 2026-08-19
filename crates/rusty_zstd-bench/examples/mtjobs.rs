//! GATE 1 support — does the `run_jobs` scheduler change bytes, and does the
//! wave-barrier removal earn its place?
//!
//! The barrier only bites when jobs > workers, so this forces that shape with a
//! small `job_size` rather than waiting for a 200 MiB corpus.
use rusty_zstd::AdvancedOptions;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
const LVL: i32 = 3;
fn main() {
    let want: Vec<u32> = std::env::args().skip(1).filter_map(|s| s.parse().ok()).collect();
    let workers = if want.is_empty() { vec![4u32, 8] } else { want };
    println!("run_jobs shape check @ L{LVL} — jobs >> workers, {} corpora", IDS.len());
    println!("{:<13} {:>5} {:>6} {:>6} | {:>9} {:>9} | {:>10}", "corpus", "MiB", "jobs", "wrk", "ms", "bytes", "sha-ish");
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(32 << 20)];
        let params = rusty_zstd::compression_params(LVL, Some(src.len() as u64)).unwrap();
        for &w in &workers {
            let job = 512 << 10;
            let adv = AdvancedOptions { nb_workers: w, job_size: job, ..Default::default() };
            let mut best = f64::MAX;
            let mut z = Vec::new();
            for _ in 0..3 {
                let t = Instant::now();
                z = rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv).unwrap();
                let e = t.elapsed().as_secs_f64() * 1000.0;
                if e < best { best = e; }
            }
            // cheap order-sensitive fingerprint of the whole frame
            let mut h: u64 = 1469598103934665603;
            for b in &z { h ^= *b as u64; h = h.wrapping_mul(1099511628211); }
            let d = rusty_zstd::decompress(&z).unwrap();
            assert!(d == src, "{id} w={w}: ROUND-TRIP FAILED");
            let jobs = rusty_zstd::resolve_job_size(job, params.window_log, rusty_zstd::overlap_size(params.window_log, 0, params.strategy));
            println!("{:<13} {:>5} {:>6} {:>6} | {:>9.1} {:>9} | {:>16x}", id, src.len()>>20, src.len().div_ceil(jobs), w, best, z.len(), h);
        }
    }
}
