//! L9 ENCODE THROUGHPUT, with its own noise floor.
//!
//! Used to A/B a code change across two BUILDS (the change is compile-time, so
//! it cannot be armed in-process). Prints two independent best-of-N arms over
//! identical input: `arm A` is the number, `spread` is how far the null arm
//! disagreed with it. A difference between builds is only real if it is much
//! larger than the spread on both sides.
//!
//! Compressed SIZE is printed too and is deterministic -- if it moves, the
//! change was not byte-identical and the comparison is invalid.
//!
//! usage: cargo run --release -p rusty_zstd-bench --example l9time [level] [cap] [n]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(15);

    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| { let k = f.len().min(cap); f[..k].to_vec() })
        })
        .collect();
    let total: u64 = srcs.iter().map(|s| s.len() as u64).sum();

    // Warm the allocator and any lazily-built tables before timing.
    let mut bytes = 0u64;
    for s in &srcs {
        bytes += rusty_zstd::compress(s, lvl).expect("compress").len() as u64;
    }

    let mut arm = [f64::MAX; 2];
    for a in 0..2 {
        for _ in 0..n {
            let t = std::time::Instant::now();
            for s in &srcs {
                let _ = rusty_zstd::compress(s, lvl).expect("compress");
            }
            let el = t.elapsed().as_secs_f64();
            if el < arm[a] {
                arm[a] = el;
            }
        }
    }
    let mbps = total as f64 / (1 << 20) as f64 / arm[0];
    let spread = (arm[0].max(arm[1]) / arm[0].min(arm[1]) - 1.0) * 100.0;
    println!(
        "L{lvl}  {:.1} MiB  best-of-{n}   {:.2} MB/s   spread {:.1}%   bytes {}",
        total as f64 / (1 << 20) as f64,
        mbps,
        spread,
        bytes
    );
}
