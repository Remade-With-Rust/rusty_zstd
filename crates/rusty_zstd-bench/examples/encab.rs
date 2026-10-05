//! Compress-only in-memory speed, for A/B-ing two BUILDS of the library.
//!
//!   cargo run --release -p rusty_zstd-bench --example encab -- <secs> <levels,csv> <id...>
//!
//! `speedab`'s shape (one program, two library versions, best-of-N in memory,
//! the driver alternates whole processes ABBA on a pinned core) with two
//! differences that matter for an ENCODE campaign: several levels in one
//! process, and no decompress loop -- `speedab` spends half of every budget
//! timing the decoder, which an encoder brick cannot move.
//!
//! Prints one tab-separated row per (id, level):
//!   id  level  src_bytes  compressed_bytes  best_ms  MB/s  loops
//! `compressed_bytes` is the work-parity anchor: two arms that claim to be
//! byte-identical must print the same value in every row.

#[global_allocator]
static ALLOC: rzstd_alloc::Alloc = rzstd_alloc::Alloc;

use std::time::{Duration, Instant};

fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 3 {
        eprintln!("usage: encab <secs> <levels,csv> <corpus-id...>");
        std::process::exit(2);
    }
    let secs: f64 = a[0].parse().expect("secs");
    let levels: Vec<i32> = a[1].split(',').map(|s| s.parse().expect("level")).collect();
    let budget = Duration::from_secs_f64(secs);
    for id in &a[2..] {
        let mut p = format!("corpora/data/silesia/{id}");
        if !std::path::Path::new(&p).exists() {
            p = format!("corpora/data/generated/{id}");
        }
        let Ok(src) = std::fs::read(&p) else {
            eprintln!("missing {p}");
            continue;
        };
        for &level in &levels {
            // one warm pass outside the timed region
            let warm = rusty_zstd::compress(&src, level).expect("compress");
            let csize = warm.len();
            let mut best = f64::MAX;
            let mut loops = 0u32;
            let t0 = Instant::now();
            loop {
                let t = Instant::now();
                let out = rusty_zstd::compress(&src, level).expect("compress");
                let ms = t.elapsed().as_secs_f64() * 1000.0;
                std::hint::black_box(&out);
                if ms < best {
                    best = ms;
                }
                loops += 1;
                if t0.elapsed() >= budget && loops >= 5 {
                    break;
                }
            }
            let mb = src.len() as f64 / 1_048_576.0;
            println!(
                "{id}\t{level}\t{}\t{csize}\t{best:.3}\t{:.2}\t{loops}",
                src.len(),
                mb / (best / 1000.0)
            );
        }
    }
}
