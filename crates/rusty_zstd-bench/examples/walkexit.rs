//! WHY DOES THE PROBE COUNT COLLAPSE WHEN THE TABLES SHRINK?
//!
//! `l9cache.rs` shows L9 going from 1.845 to 0.553 probes per input byte as
//! `hash_log`/`chain_log` are walked down -- a 3.3x work reduction that is most
//! of its 2.9x apparent speedup. I explained that as "smaller tables collide
//! more, so the walk's `next >= m` guard breaks sooner" and then did not check
//! it. This checks it.
//!
//! The chain walk can end seven ways, and `WALK_EXIT` counts each. If the
//! explanation is right, shrinking the tables must shift terminations toward
//! index 3 (the LINK GUARD). If instead the shift is toward index 0 (empty
//! bucket) or 2 (window bound), the mechanism is something else entirely and
//! the story was wrong.
//!
//! Counts only -- deterministic, identical on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example walkexit [level]

const IDS: &[&str] = &["dickens", "webster", "samba", "mozilla", "osdb", "mr"];
const NAMES: [&str; 7] = [
    "empty bucket",
    "entry guard",
    "window bound",
    "LINK GUARD",
    "block_end",
    "depth spent",
    "walk_cont off",
];

fn tbl(log: u32) -> String {
    let b = (1u64 << log) * 4;
    if b >= 1 << 20 { format!("{}M", b >> 20) } else { format!("{}K", b >> 10) }
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);

    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| { let n = f.len().min(cap); f[..n].to_vec() })
        })
        .collect();
    let total: u64 = srcs.iter().map(|s| s.len() as u64).sum();
    let base = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");

    println!(
        "L{lvl} CHAIN-WALK EXIT CENSUS -- {} corpora, {:.1} MiB\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    print!("{:>10} {:>11} {:>10}", "tables", "probes/B", "walks");
    for n in NAMES.iter() {
        print!("{:>15}", n);
    }
    println!();

    for (hl, cl) in [
        (base.hash_log, base.chain_log),
        (base.hash_log - 3, base.chain_log - 3),
        (base.hash_log - 5, base.chain_log - 5),
        (base.hash_log - 7, base.chain_log - 7),
    ] {
        let mut p = base;
        p.hash_log = hl;
        p.chain_log = cl;
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_walk_exit();
        for s in &srcs {
            let _ = rusty_zstd::compress_with_params(s, p, false).expect("compress");
        }
        let e = rusty_zstd::take_walk_exit();
        let c = rusty_zstd::prof_encode_counts();
        let walks: u64 = e[..7].iter().sum();
        print!(
            "{:>10} {:>11.3} {:>10}",
            format!("{}+{}", tbl(hl), tbl(cl)),
            c.hash_probes as f64 / total as f64,
            walks
        );
        for i in 0..7 {
            print!("{:>14.1}%", if walks > 0 { 100.0 * e[i] as f64 / walks as f64 } else { 0.0 });
        }
        println!();
    }
    println!(
        "\nEach row is the SAME input at a different table size. Whichever column\n\
         grows as the tables shrink IS the mechanism -- everything else is a\n\
         story. `LINK GUARD` growing confirms the aliasing explanation;\n\
         `empty bucket` or `window bound` growing refutes it."
    );
}
