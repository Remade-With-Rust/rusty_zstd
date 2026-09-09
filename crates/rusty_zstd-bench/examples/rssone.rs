//! Compress one file once at a given level and tightening, then exit, so the
//! caller can read this process's PEAK WORKING SET. Table footprint is a
//! MEMORY question, not a time one -- `vec![0; n]` for a large n takes zero
//! pages from the OS rather than memsetting, so cutting the allocation shows
//! up in RSS, not on the clock.
fn main() {
    let a: Vec<String> = std::env::args().collect();
    let lvl: i32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let tight: u32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
    let cap: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    let id = a.get(4).cloned().unwrap_or_else(|| "dickens".into());
    let f = std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .expect("corpus");
    let s = &f[..f.len().min(cap)];
    rusty_zstd::set_hash_tight_arm(tight);
    let p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
    let z = rusty_zstd::compress_with_params(s, p, false).unwrap();
    println!("L{lvl} tight={tight} in={} out={} hash_log={} chain_log={} window_log={}",
             s.len(), z.len(), p.hash_log, p.chain_log, p.window_log);
    // Hold the tables alive and the process sampleable: PeakWorkingSet64 reads
    // 0 once the process has exited, so the parent must sample it live.
    std::hint::black_box(&z);
    std::thread::sleep(std::time::Duration::from_millis(700));
}
