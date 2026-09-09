//! In-process compress/decompress speed, for A/B-ing two BUILDS of the library.
//!
//!   cargo run --release -p rusty_zstd-bench --example speedab -- <level> <secs> <id...>
//!
//! Why a separate program rather than `--m7-speed`: that harness shells out to
//! the pinned C zstd and writes the ledger; this one measures ONLY us, in
//! memory (no file I/O in the timed region), so the same source compiled
//! against two library versions is the whole comparison. The estimator is
//! BEST-of-N per run (the floor is what survives a busy box), and the driver
//! alternates whole processes ABBA and takes the median across pairs.
//!
//! The allocator is installed here deliberately: a deliverable declares one,
//! and the rusty_alloc pin is exactly one of the things being measured.

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

use std::time::{Duration, Instant};

fn bench(src: &[u8], level: i32, budget: Duration) -> (f64, f64, usize, u32) {
    // one warm pass, outside the timed region, so tables/scratch are hot
    let warm = rusty_zstd::compress(src, level).expect("compress");
    let csize = warm.len();

    let mut best_c = f64::MAX;
    let mut loops = 0u32;
    let t0 = Instant::now();
    loop {
        let t = Instant::now();
        let out = rusty_zstd::compress(src, level).expect("compress");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        std::hint::black_box(&out);
        if ms < best_c {
            best_c = ms;
        }
        loops += 1;
        if t0.elapsed() >= budget && loops >= 3 {
            break;
        }
    }

    let mut best_d = f64::MAX;
    let t0 = Instant::now();
    let mut dloops = 0u32;
    loop {
        let t = Instant::now();
        let raw = rusty_zstd::decompress(&warm).expect("decompress");
        let ms = t.elapsed().as_secs_f64() * 1000.0;
        assert_eq!(raw.len(), src.len(), "round trip length");
        std::hint::black_box(&raw);
        if ms < best_d {
            best_d = ms;
        }
        dloops += 1;
        if t0.elapsed() >= budget && dloops >= 3 {
            break;
        }
    }
    (best_c, best_d, csize, loops)
}

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: speedab <level> <secs> <corpus-id...>");
        std::process::exit(2);
    }
    let level: i32 = a[0].parse().expect("level");
    let secs: u64 = a[1].parse().expect("secs");
    let budget = Duration::from_secs(secs);
    for id in &a[2..] {
        let mut p = format!("corpora/data/silesia/{id}");
        if !std::path::Path::new(&p).exists() {
            p = format!("corpora/data/generated/{id}");
        }
        let Ok(src) = std::fs::read(&p) else {
            eprintln!("missing {p}");
            continue;
        };
        let (c_ms, d_ms, csize, loops) = bench(&src, level, budget);
        let mb = src.len() as f64 / 1_048_576.0;
        println!(
            "{id}\t{level}\t{}\t{csize}\t{c_ms:.3}\t{d_ms:.3}\t{:.2}\t{:.2}\t{loops}",
            src.len(),
            mb / (c_ms / 1000.0),
            mb / (d_ms / 1000.0),
        );
    }
}
