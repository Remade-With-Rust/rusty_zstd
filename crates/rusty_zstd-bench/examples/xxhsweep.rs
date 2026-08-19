//! Does the checksum have a size regime where it collapses?
//!
//! `xxh64_seed` dispatches on an ABSOLUTE constant (`len >= 32`) and then again
//! on a 128-byte chunk loop with a 32-byte remainder loop. Law 1.1 flags absolute
//! thresholds, so the question is whether throughput has a cliff the block size
//! could land on -- if it does, Gate 4's CONSTANT is hiding a size dispatch.
//!
//! Deterministic in structure (same bytes, same code), so only the clock varies;
//! best-of-many on a fixed buffer keeps it honest.
use std::time::Instant;
fn main() {
    let buf: Vec<u8> = (0..(4usize << 20)).map(|i| (i * 2654435761) as u8).collect();
    println!("{:>10} {:>12} {:>10}", "bytes", "GB/s", "vs peak");
    let sizes: Vec<usize> = vec![
        16, 31, 32, 33, 63, 64, 96, 127, 128, 129, 160, 255, 256, 257,
        1024, 4096, 16384, 65536, 131072, 262144, 1 << 20, 4 << 20,
    ];
    let mut peak = 0.0f64;
    let mut rows = Vec::new();
    for n in sizes {
        let s = &buf[..n];
        // iterations scaled so every size gets similar wall time
        let iters = (1 << 26) / n.max(1) + 16;
        let mut best = f64::MAX;
        for _ in 0..7 {
            let t = Instant::now();
            let mut acc = 0u64;
            for _ in 0..iters { acc ^= rusty_zstd::xxh64(s); }
            std::hint::black_box(acc);
            let e = t.elapsed().as_secs_f64();
            if e < best { best = e }
        }
        let gbs = (n as f64 * iters as f64) / best / 1e9;
        if gbs > peak { peak = gbs }
        rows.push((n, gbs));
    }
    for (n, gbs) in &rows {
        println!("{:>10} {:>12.2} {:>9.0}%", n, gbs, gbs / peak * 100.0);
    }
    println!("\n  peak {peak:.2} GB/s");
    println!("  XXH64 reference is ~13 GB/s on modern x86 for large inputs.");
}
