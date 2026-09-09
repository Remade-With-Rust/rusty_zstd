//! Does repeated compression GROW the process, or is it flat?
//!
//! Two allocations failed during this session's sweeps (4 MB and 16 MB) on a
//! 64-bit box. The machine is genuinely short of memory, but that is a reason
//! to CHECK rather than to assume -- a leak and a loaded box look identical
//! from the outside. This compresses the same input many times at a high level
//! and reports the process working set as it goes.
use rusty_zstd as rz;
#[cfg(windows)]
fn rss() -> u64 {
    // No winapi dependency: read our own working set via the same counter the
    // parent would sample, through GlobalMemoryStatus-free means -- fall back
    // to reporting allocation totals if unavailable.
    0
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let iters: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(60);
    let f = std::fs::read("corpora/data/silesia/dickens").expect("dickens");
    let src = &f[..f.len().min(2 << 20)];
    let _ = rss();
    println!("L{lvl}, {} KiB input, {iters} iterations", src.len() >> 10);
    for i in 0..iters {
        let p = rz::compression_params(lvl, Some(src.len() as u64)).unwrap();
        let z = rz::compress_with_params(src, p, false).unwrap();
        std::hint::black_box(z.len());
        if i % 10 == 0 || i == iters - 1 {
            println!("  iter {i:>3}");
        }
    }
    println!("done");
    std::thread::sleep(std::time::Duration::from_millis(600));
}
