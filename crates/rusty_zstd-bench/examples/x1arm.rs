//! X1-vs-X2 ARM CENSUS -- is `decode_4x_x1` code anyone runs?
//!
//! `decode_4x_inner` splits on `use_x2()`: the X1 arm calls `decode_4x_x1`
//! (532 instructions, 20 variable shifts, compiled at BASELINE ISA beneath the
//! BMI2 twin that calls it), the X2 arm goes to `fast_4x2` + `decode_into_x2`.
//!
//! Twinning `decode_4x_x1` costs 532 instructions of duplicated body to convert
//! 20 shift ops -- 26.6 per op, which passes the ratio screen. Ratio is not
//! sufficient: `parse_ncount_into` has the BEST ratio in the crate (13.3) and is
//! not worth twinning because it runs three times per block. So this counts how
//! often each arm is actually taken before anything is duplicated.
//!
//! A count, not a clock -- same number on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example x1arm

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LEVELS: &[i32] = &[1, 3, 9, 19];

fn main() {
    let cap: usize = std::env::var("BG_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1 << 20);
    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect();

    // SPLIT BY POPULATION, deliberately. The census already recorded against
    // `decode_4x_x1` -- "509 of 517 sections (98.45%) to the X2 arm, 8 (1.55%)
    // here" -- was taken over SILESIA only. Measuring the mixed set and
    // comparing the result to that number would be overturning a documented
    // decision using a different population, which is not a measurement.
    let synthetic = |id: &str| {
        matches!(
            id,
            "zeros-32m"
                | "text-32m"
                | "incomp-32m"
                | "jsonlog-16m"
                | "smallmsg-8m"
                | "versions-16m"
        )
    };

    for (label, keep) in [
        ("SILESIA (the population the 1.55% census used)", false),
        ("SYNTHETIC stress corpora", true),
    ] {
        let set: Vec<&(&str, Vec<u8>)> =
            srcs.iter().filter(|(id, _)| synthetic(id) == keep).collect();
        if set.is_empty() {
            continue;
        }
        println!("\n=== {label} -- {} corpora ===", set.len());
        println!("{:>3}  {:>12} {:>12} {:>10}", "L", "x1_sections", "x2_sections", "x1_share");
        let (mut tx1, mut tx2) = (0u64, 0u64);
        for &l in LEVELS {
            let frames: Vec<Vec<u8>> = set
                .iter()
                .map(|(_, s)| rusty_zstd::compress(s, l).expect("compress"))
                .collect();
            let _ = rusty_zstd::take_x4_arms();
            for z in &frames {
                let _ = rusty_zstd::decompress(z).expect("decompress");
            }
            // Both counters increment at exactly ONE site each, so this ratio
            // is readable. `X2_STATS[1]` is shared with the 1-stream path and
            // would silently inflate the denominator.
            let (x1, x2) = rusty_zstd::take_x4_arms();
            tx1 += x1;
            tx2 += x2;
            let tot = x1 + x2;
            println!(
                "{:>3}  {:>12} {:>12} {:>9.2}%",
                l,
                x1,
                x2,
                if tot > 0 { 100.0 * x1 as f64 / tot as f64 } else { 0.0 }
            );
        }
        let tot = tx1 + tx2;
        println!(
            "  TOTAL x1={tx1} x2={tx2}  x1_share={:.2}%",
            if tot > 0 { 100.0 * tx1 as f64 / tot as f64 } else { 0.0 }
        );
    }
    println!(
        "A twin for `decode_4x_x1` costs 532 instructions to convert 20 shift ops.\n\
         If x1_share is near zero it is the `decode_4x_x2_slow` case: correct,\n\
         reachable, and not worth three ISA copies."
    );
}
