//! THE WIDE-CHAIN LATCH, PRICED ON BOTH SIDES.
//!
//! `wide_chain_enabled()` ships ON, and its own comment records the adjudication:
//! "L5 -0.07% / L7 -0.46% / L9 -0.52% / L12 -0.32% totals with ZERO losing
//! corpora at any level." That is a RATIO board. The latch's cost was never
//! measured, because until `WIDE_LATCH` there was no counter for it -- and the
//! cost is not small: firing does a full O(window) rebuild of the chain, a
//! per-position `lz_insert` across the whole lookback.
//!
//! `l9route.rs` measures that rebuild at **10.9M positions at L9, 9.0% of the
//! entire probe count**, and 20.3% at L5. It fires at L5/L7/L9/L12 and nowhere
//! else -- precisely the levels where this codec is furthest behind C on encode
//! speed.
//!
//! So this board runs the arm both ways and reports what each side buys. SIZE
//! and the work counters are deterministic. The time column is best-of-N with a
//! null arm beside it and should only be read as "did it move by much more than
//! the spread".
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example latchcost [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(5);

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
        "WIDE-CHAIN LATCH @ L{lvl} -- {} corpora, {:.1} MiB, best-of-{n}\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>12} {:>10} {:>14} {:>12} {:>13} {:>10} {:>8}",
        "arm", "latches", "resc. inserts", "probes/B", "bytes", "MB/s", "spread"
    );

    let (mut b_bytes, mut b_mbps) = (0u64, 0f64);
    for (i, on) in [true, false].iter().enumerate() {
        rusty_zstd::set_wide_chain_arm(*on);
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_wide_latch();

        // Counters from one clean pass, then timing separately so the census
        // reset does not land inside a timed region.
        let mut bytes = 0u64;
        for (id, s) in &srcs {
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
            bytes += z.len() as u64;
        }
        let (events, resc) = rusty_zstd::take_wide_latch();
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
            b_mbps = mbps;
        }
        println!(
            "{:>12} {:>10} {:>14} {:>12.3} {:>13} {:>10.1} {:>7.1}%",
            if *on { "ON (ships)" } else { "OFF" },
            events,
            resc,
            c.hash_probes as f64 / total as f64,
            bytes,
            mbps,
            spread
        );
        if i == 1 {
            println!(
                "\n  turning the latch OFF: size {:+.3}%, throughput {:+.1}%",
                100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64,
                100.0 * (mbps - b_mbps) / b_mbps
            );
        }
    }
    rusty_zstd::set_wide_chain_arm(true);
    println!(
        "\nThe latch was adjudicated on RATIO alone (-0.52% at L9). This is the\n\
         other half of that trade."
    );
}
