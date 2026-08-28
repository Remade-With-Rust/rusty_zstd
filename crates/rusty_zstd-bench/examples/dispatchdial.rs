//! DOES THE WALK-CONTINUE DISPATCH DISCRIMINATE AT ALL?
//!
//! The dispatch has four conditions:
//!
//! ```text
//!   walk_cont = walk_cont_enabled()
//!       && strategy != Fast                                    // structural
//!       && rep_yield <= walk_rep_max()                         // 0.10 at L9
//!       && (walk_first_share <= walk_first_max(attempts)       // 0.70 at L9
//!           || walk_probe == 0)                                // every 16 blocks
//! ```
//!
//! Sweeping `walk_rep_max` showed it is nearly INERT at L9: 0.10 -> 0.05 -> 0.02
//! moves the probe count by ~2%, because `rep_yield` almost never exceeds any of
//! those bars. Only 0.0 -- which disables the feature outright -- does anything.
//!
//! This sweeps the OTHER bar, `walk_first_max`, the same way. A bar that
//! discriminates will move `probes/B` and `bytes` smoothly as it tightens. A bar
//! that is inert will sit flat until it hits the value that switches the feature
//! off entirely, which is not a dispatch -- it is a constant with extra steps.
//!
//! Both swept columns are deterministic.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example dispatchdial [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn run(srcs: &[(&str, Vec<u8>)], lvl: i32, total: u64) -> (f64, u64) {
    rusty_zstd::prof_reset();
    let mut bytes = 0u64;
    for (id, s) in srcs {
        let z = rusty_zstd::compress(s, lvl).expect("compress");
        assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
        bytes += z.len() as u64;
    }
    let c = rusty_zstd::prof_encode_counts();
    (c.hash_probes as f64 / total as f64, bytes)
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);

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
        "WALK-CONTINUE DISPATCH: do its bars discriminate? @ L{lvl}\n\
         {} corpora, {:.1} MiB. Shipping bars: rep_max 0.10, first_max 0.70\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );

    // baseline with both bars at their shipping values
    rusty_zstd::set_walk_rep_max_arm(0.10);
    rusty_zstd::set_walk_first_max_arm(0.70);
    let (bp, bb) = run(&srcs, lvl, total);
    println!("{:>22} {:>11} {:>13} {:>10} {:>10}", "setting", "probes/B", "bytes", "probes vs", "size vs");
    println!("{:>22} {:>11.3} {:>13} {:>9.1}% {:>9.3}%", "SHIPPING", bp, bb, 0.0, 0.0);

    println!("\n-- sweeping walk_first_max (rep_max held at 0.10) --");
    for v in [0.55f32, 0.40, 0.20, 0.0] {
        rusty_zstd::set_walk_first_max_arm(v);
        let (p, b) = run(&srcs, lvl, total);
        println!(
            "{:>22} {:>11.3} {:>13} {:>9.1}% {:>9.3}%",
            format!("first_max {v}"),
            p,
            b,
            100.0 * (p - bp) / bp,
            100.0 * (b as f64 - bb as f64) / bb as f64
        );
    }
    rusty_zstd::set_walk_first_max_arm(0.70);

    println!("\n-- sweeping walk_rep_max (first_max held at 0.70) --");
    for v in [0.05f32, 0.02, 0.01, 0.0] {
        rusty_zstd::set_walk_rep_max_arm(v);
        let (p, b) = run(&srcs, lvl, total);
        println!(
            "{:>22} {:>11.3} {:>13} {:>9.1}% {:>9.3}%",
            format!("rep_max {v}"),
            p,
            b,
            100.0 * (p - bp) / bp,
            100.0 * (b as f64 - bb as f64) / bb as f64
        );
    }
    rusty_zstd::set_walk_rep_max_arm(0.10);

    println!(
        "\nA bar that DISCRIMINATES moves these columns smoothly as it tightens.\n\
         A bar that sits flat until it switches the feature off is a constant,\n\
         and the per-block signal behind it is not doing the job it was added for."
    );
}
