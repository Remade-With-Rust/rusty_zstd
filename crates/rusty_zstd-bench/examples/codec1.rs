//! codec1 -- the m7-speed "us" arm, standalone, for inputs the house bench
//! cannot take (the README codec table uses copies capped at 8 MiB; C's own
//! `zstd -b` asserts on whole mozilla at L3). Runs
//! `rusty_zstd::bench_roundtrip_clocked` -- the routine `rzstd-bench --m7-speed`
//! calls -- under the same rusty_alloc seam, best of two 3 s runs, and prints
//! `encode_MBps decode_MBps`. Driven by `bench/codec-board.ps1`.
#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let src = std::fs::read(&a[1]).unwrap();
    let lvl: i32 = a[2].parse().unwrap();
    let min = std::time::Duration::from_secs(3);
    let (mut be, mut bd) = (f64::MAX, f64::MAX);
    for _ in 0..2 {
        let b = rusty_zstd::bench_roundtrip_clocked(&src, lvl, min, || 0).unwrap();
        be = be.min(b.compress_best_ms);
        bd = bd.min(b.decompress_best_ms);
    }
    println!(
        "{:.1} {:.1}",
        rusty_zstd::mbps_best(src.len(), be),
        rusty_zstd::mbps_best(src.len(), bd)
    );
}
