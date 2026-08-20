//! What does one uncached `std::env::var` actually cost?
//! On Windows it is GetEnvironmentVariableW plus a String allocation, for a
//! value that is fixed for the life of the process.
use std::time::Instant;
fn main() {
    let n = 200_000;
    // present and absent, since a miss still pays the syscall
    std::env::set_var("RZSTD_BENCH_PRESENT", "0.71");
    let mut best_hit = f64::MAX;
    let mut best_miss = f64::MAX;
    let mut best_cached = f64::MAX;
    let cached = std::sync::atomic::AtomicU32::new(f32::to_bits(0.71));
    for _ in 0..5 {
        let t = Instant::now();
        for _ in 0..n {
            let v: f32 = std::env::var("RZSTD_BENCH_PRESENT").ok()
                .and_then(|v| v.trim().parse().ok()).unwrap_or(0.71);
            std::hint::black_box(v);
        }
        let e = t.elapsed().as_secs_f64() * 1e9 / n as f64;
        if e < best_hit { best_hit = e }

        let t = Instant::now();
        for _ in 0..n {
            let v: f32 = std::env::var("RZSTD_BENCH_ABSENT_XYZ").ok()
                .and_then(|v| v.trim().parse().ok()).unwrap_or(0.71);
            std::hint::black_box(v);
        }
        let e = t.elapsed().as_secs_f64() * 1e9 / n as f64;
        if e < best_miss { best_miss = e }

        let t = Instant::now();
        for _ in 0..n {
            let v = f32::from_bits(cached.load(std::sync::atomic::Ordering::Relaxed));
            std::hint::black_box(v);
        }
        let e = t.elapsed().as_secs_f64() * 1e9 / n as f64;
        if e < best_cached { best_cached = e }
    }
    println!("  env::var HIT     {best_hit:>9.1} ns/call");
    println!("  env::var MISS    {best_miss:>9.1} ns/call   <- the shipping case (vars unset)");
    println!("  cached atomic    {best_cached:>9.1} ns/call");
    println!("\n  cost of one uncached read: {:.1} ns", best_miss - best_cached);
    for (lvl, reads) in [("L1", 1875u64), ("L3", 1320), ("L19", 1805)] {
        let us = (best_miss - best_cached) * reads as f64 / 1000.0;
        println!("  {lvl}: {reads} reads per 32 MiB  ->  {us:.0} us wasted per pass");
    }
}
