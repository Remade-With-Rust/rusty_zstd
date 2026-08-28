//! L9 ENCODE: does the ROW finder close the 3x gap to C?
//!
//! L9 is **95.8% MatchFind** (`hotspot`), and its parameters are byte-for-byte
//! zstd's L9 -- `{22, 20, 21, 4, 5, 16, lazy2}` -- so we are not doing more
//! nominal work than C. What we are doing is walking a hash CHAIN through
//! tables that do not fit cache: `hash_log 21` is an 8 MiB table and
//! `chain_log 20` a 4 MiB one, both indexed by a hash, i.e. randomly. Each
//! chain step is a DEPENDENT load that cannot issue until the previous one
//! retires.
//!
//! That is exactly what `ZSTD_row_match_finder` exists to fix, and C uses row
//! matching at these levels. Ours is built (`rowfind.rs`) and defaults OFF.
//!
//! This boards it. `ROW_EXAM` counts candidates examined and `ROW_LOADS` counts
//! rows touched -- the row finder's whole claim is reaching the same candidates
//! with FEWER dependent loads, so both are needed. `WALK_EXAM` is the chain's
//! counterpart. Counts are deterministic; SIZE is deterministic; the arm is
//! bitstream-changing, so size is the adjudicator.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example l9row [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);

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
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();

    println!(
        "L{lvl} ROW-FINDER BOARD -- {} corpora, {:.0} MiB\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!("{:>14} {:>14} {:>13} {:>13} {:>10}", "arm", "bytes", "size vs", "candidates", "loads");

    let mut base = 0u64;
    for (i, on) in [false, true].iter().enumerate() {
        rusty_zstd::set_row_arm(*on);
        let (mut bytes, mut exam, mut loads) = (0u64, 0u64, 0u64);
        for (id, s) in &srcs {
            rusty_zstd::prof_reset();
            let _ = rusty_zstd::take_row_walk();
            let z = rusty_zstd::compress(s, lvl).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id}");
            bytes += z.len() as u64;
            let w = rusty_zstd::take_row_walk();
            exam += w[0];
            loads += w[1];
        }
        if i == 0 {
            base = bytes;
        }
        println!(
            "{:>14} {:>14} {:>12.3}% {:>13} {:>10}",
            if *on { "row ON" } else { "chain (default)" },
            bytes,
            if base > 0 { 100.0 * (bytes as f64 - base as f64) / base as f64 } else { 0.0 },
            exam,
            loads
        );
    }
    rusty_zstd::set_row_arm(false);
    println!(
        "\ncandidates/loads are the row walk's own counters (zero on the chain\n\
         arm). The row finder's claim is SAME candidates, FEWER dependent\n\
         loads -- which is a cache-latency win the instruction count cannot see."
    );
}
