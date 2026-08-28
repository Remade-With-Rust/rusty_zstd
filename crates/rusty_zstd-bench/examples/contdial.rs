//! THE WALK-CONTINUE DIAL -- speed against ratio, on the bar that already exists.
//!
//! Forcing `walk_cont` off entirely measures **+16% to +30% encode throughput
//! at L9 for +2.265% size** (`contcost.rs`, three runs, spreads 2-7%). That is
//! the first lever in this campaign that moves the clock outside the noise
//! consistently -- but it is a blunt one.
//!
//! It does not have to be blunt. `walk_cont` is already a per-block DISPATCH:
//!
//! ```text
//!   walk_cont = walk_cont_enabled()
//!       && strategy != Fast
//!       && rep_yield <= walk_rep_max()        // 0.10 by default
//!       && (walk_first_share <= walk_first_max(attempts) || walk_probe == 0)
//! ```
//!
//! `walk_rep_max` is the bar: continue only where repcodes are NOT carrying the
//! block. Lowering it applies the deep walk to less content -- less work, less
//! ratio. This sweeps that bar to find whether a middle setting keeps most of
//! the speed for a fraction of the size.
//!
//! `probes/B` and `size` are deterministic. The time column is best-of-N with a
//! null arm; read it only where the spread is small.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example contdial [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(8);

    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let k = f.len().min(cap);
                    (*id, f[..k].to_vec())
                })
        })
        .collect();
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();

    println!(
        "WALK-CONTINUE DIAL @ L{lvl} -- {} corpora, {:.1} MiB, best-of-{n}\n\
         shipping walk_rep_max = 0.10\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>12} {:>11} {:>13} {:>10} {:>10} {:>8}",
        "rep_max", "probes/B", "bytes", "size vs", "MB/s", "spread"
    );

    let mut b_bytes = 0u64;
    for (i, bar) in [0.10f32, 0.05, 0.02, 0.0, 1.0].iter().enumerate() {
        rusty_zstd::set_walk_rep_max_arm(*bar);
        rusty_zstd::prof_reset();
        let mut bytes = 0u64;
        for (id, s) in &srcs {
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
            bytes += z.len() as u64;
        }
        let c = rusty_zstd::prof_encode_counts();

        let mut arm = [f64::MAX; 2];
        for a in 0..2 {
            for _ in 0..n {
                let t = std::time::Instant::now();
                for (_, s) in &srcs {
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
        if i == 0 {
            b_bytes = bytes;
        }
        println!(
            "{:>12} {:>11.3} {:>13} {:>9.3}% {:>10.1} {:>7.1}%",
            if i == 0 { format!("{bar} (ships)") } else { format!("{bar}") },
            c.hash_probes as f64 / total as f64,
            bytes,
            if b_bytes > 0 {
                100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64
            } else {
                0.0
            },
            mbps,
            spread
        );
    }
    rusty_zstd::set_walk_rep_max_arm(0.10);
    println!(
        "\n`rep_max` 0.0 = never continue (deep walk off everywhere); 1.0 = always\n\
         continue (dispatch disabled). The shipping 0.10 sits between them."
    );
}
