//! SHARPENING THE WALK-CONTINUE DISPATCH: the `walk_first_share` bar.
//!
//! `walk_cont` gates the deep (C-parity) chain walk on two measured signals.
//! Sweeping them separately shows they are not equally useful:
//!
//!   * `walk_rep_max` (0.10) is nearly INERT at L9 -- tightening it to 0.05 /
//!     0.02 / 0.01 moves the probe count by 0.9% / 2.1% / 2.4%, because
//!     `rep_yield` almost never approaches the bar. Only 0.0, which switches
//!     the feature off outright, does anything.
//!   * `walk_first_max` DISCRIMINATES. `walk_first_share` lives between ~0.40
//!     and ~0.70, so the shipping bar of 0.70 at L9 barely trims: dropping it
//!     to 0.55 cuts probes 19.1% for +1.297% size, and 0.40 saturates the
//!     signal at -22.2% / +1.594%.
//!
//! The rate is what matters. Tightening the first-share bar buys probe
//! reduction ~35% more cheaply than disabling the feature does:
//!
//! ```text
//!   first_max 0.55     -19.1% probes   +1.297% size    14.7 probe-% per size-%
//!   first_max 0.40     -22.2% probes   +1.594% size    13.9
//!   feature disabled   -24.8% probes   +2.265% size    10.9
//! ```
//!
//! MEASUREMENT NOTE. The arms are INTERLEAVED inside one process, alternating
//! per iteration and keeping a per-arm best. Run as separate passes on a loaded
//! host the two arms drift apart and the delta swung from -3.9% to +24.8% on
//! the same configuration; interleaved it reproduced at +13.6% / +12.8% /
//! +13.2%. Drift hits both arms equally when they alternate.
//!
//! usage: cargo run --release -p rusty_zstd-bench --example firstbar [cap] [n]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

/// The shipping bar, from `walk_first_max(attempts)`: 0.80 for attempts <= 8
/// (L5), 0.70 for <= 16 (L7/L9), 0.55 above (L12+).
fn ships(lvl: i32) -> f32 {
    match lvl {
        5 => 0.80,
        7 | 9 => 0.70,
        _ => 0.55,
    }
}

fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(12);

    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let k = f.len().min(cap);
                    f[..k].to_vec()
                })
        })
        .collect();
    let total: u64 = srcs.iter().map(|s| s.len() as u64).sum();

    println!(
        "WALK-FIRST-SHARE BAR -- {} corpora, {:.1} MiB, interleaved best-of-{n}\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>6} {:>10} {:>9} {:>12} {:>12} {:>10} {:>10}",
        "level", "ships", "tighter", "bytes ships", "bytes tight", "size vs", "speed vs"
    );

    for lvl in [5i32, 7, 9, 12] {
        let base = ships(lvl);
        let tight = (base - 0.15).max(0.30);

        // Sizes first: deterministic, no timing involved.
        let mut bytes = [0u64; 2];
        for (i, v) in [base, tight].iter().enumerate() {
            rusty_zstd::set_walk_first_max_arm(*v);
            for s in &srcs {
                bytes[i] += rusty_zstd::compress(s, lvl).expect("compress").len() as u64;
            }
        }

        // INTERLEAVED timing: alternate the arms so host drift hits both.
        let mut best = [f64::MAX; 2];
        for _ in 0..n {
            for (i, v) in [base, tight].iter().enumerate() {
                rusty_zstd::set_walk_first_max_arm(*v);
                let t = std::time::Instant::now();
                for s in &srcs {
                    let _ = rusty_zstd::compress(s, lvl).expect("compress");
                }
                let e = t.elapsed().as_secs_f64();
                if e < best[i] {
                    best[i] = e;
                }
            }
        }
        let mb = |t: f64| total as f64 / (1 << 20) as f64 / t;
        println!(
            "{:>6} {:>10.2} {:>9.2} {:>12} {:>12} {:>9.3}% {:>9.1}%",
            lvl,
            base,
            tight,
            bytes[0],
            bytes[1],
            100.0 * (bytes[1] as f64 - bytes[0] as f64) / bytes[0] as f64,
            100.0 * (mb(best[1]) - mb(best[0])) / mb(best[0])
        );
    }
    rusty_zstd::set_walk_first_max_arm(f32::from_bits(u32::MAX));
    println!(
        "\n`tighter` is the shipping bar minus 0.15. Size is deterministic; the\n\
         speed column is an interleaved A/B and is readable, unlike the\n\
         separate-pass form which drifts on a loaded host."
    );
}
