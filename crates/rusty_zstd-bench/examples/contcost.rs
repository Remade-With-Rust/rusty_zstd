//! WALK-CONTINUE, PRICED ON BOTH SIDES.
//!
//! `walk_cont_enabled()` ships ON, and its comment records the adjudication:
//! "dickens -8.99%, reymont -7.62%, webster -6.03%" -- a RATIO board. What it
//! costs was never measured.
//!
//! It is the reason the walk runs deep. `walkexit.rs` shows the `walk_cont off`
//! exit firing **0.0%** of the time and 58.9% of walks spending their entire
//! `attempts` budget: with continue ON, a tag miss or a byte miss steps to the
//! next link instead of stopping, so nothing terminates a walk early except the
//! budget itself. Every one of those steps is a DEPENDENT load.
//!
//! This runs the arm both ways. `probes/B` and `size` are deterministic. The
//! time column is best-of-N with a null arm beside it and is only readable when
//! the spread is small -- on a loaded host it is not.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example contcost [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(10);

    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| { let k = f.len().min(cap); (*id, f[..k].to_vec()) })
        })
        .collect();
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();

    println!(
        "WALK-CONTINUE @ L{lvl} -- {} corpora, {:.1} MiB, best-of-{n}\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>12} {:>11} {:>13} {:>13} {:>10} {:>8}",
        "arm", "probes/B", "depth spent", "bytes", "MB/s", "spread"
    );

    let (mut b_bytes, mut b_probes, mut b_mbps) = (0u64, 0f64, 0f64);
    for (i, on) in [true, false].iter().enumerate() {
        rusty_zstd::set_walk_cont_arm(*on);
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_walk_exit();
        let mut bytes = 0u64;
        for (id, s) in &srcs {
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
            bytes += z.len() as u64;
        }
        let e = rusty_zstd::take_walk_exit();
        let c = rusty_zstd::prof_encode_counts();
        let walks: u64 = e[..7].iter().sum();
        let ppb = c.hash_probes as f64 / total as f64;

        let mut arm = [f64::MAX; 2];
        for a in 0..2 {
            for _ in 0..n {
                let t = std::time::Instant::now();
                for (_, s) in &srcs {
                    let _ = rusty_zstd::compress(s, lvl).expect("compress");
                }
                let el = t.elapsed().as_secs_f64();
                if el < arm[a] { arm[a] = el; }
            }
        }
        let mbps = total as f64 / (1 << 20) as f64 / arm[0];
        let spread = (arm[0].max(arm[1]) / arm[0].min(arm[1]) - 1.0) * 100.0;
        if i == 0 { b_bytes = bytes; b_probes = ppb; b_mbps = mbps; }
        println!(
            "{:>12} {:>11.3} {:>12.1}% {:>13} {:>10.1} {:>7.1}%",
            if *on { "ON (ships)" } else { "OFF" },
            ppb,
            if walks > 0 { 100.0 * e[5] as f64 / walks as f64 } else { 0.0 },
            bytes,
            mbps,
            spread
        );
        if i == 1 {
            println!(
                "\n  turning continue OFF: probes {:+.1}%, size {:+.3}%, throughput {:+.1}%",
                100.0 * (ppb - b_probes) / b_probes,
                100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64,
                100.0 * (mbps - b_mbps) / b_mbps
            );
        }
    }
    rusty_zstd::set_walk_cont_arm(true);
}
