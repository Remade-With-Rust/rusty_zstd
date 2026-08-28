//! L9 ROUTING PROBE -- what does the encoder actually DO at each level?
//!
//! Every board so far has measured the chain walk's internals. This asks a
//! different question: does L9 take the route it is supposed to, and what do
//! the runtime latches cost?
//!
//! The suspect is `maybe_latch_wide_chain`. It is a mid-frame LATCH: frames
//! start on the narrow chain key and switch to the wide one once
//! `walk_first_share` has held above its bar for three measured blocks. When it
//! fires it does a **full O(window) rebuild of the chain** -- a per-position
//! `lz_insert` from `block_start - window` to `block_start - 8`. At L9 the
//! window is 2^22, so a single latch can rescan up to 4M positions.
//!
//! Nothing counted that. `WIDE_LATCH` now does, so the rebuild appears beside
//! the per-position work it is meant to improve.
//!
//! Counts only. Deterministic.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example l9route

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let cap: usize = std::env::args()
        .nth(1)
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
        "WIDE-CHAIN LATCH COST -- {} corpora, {:.0} MiB total\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>5} {:>10} {:>9} {:>15} {:>14} {:>13} {:>11}",
        "level", "strategy", "latches", "positions resc.", "resc./input B", "probes/B", "resc/probe"
    );

    for lvl in [1i32, 3, 5, 7, 9, 12, 15, 19] {
        let p = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_wide_latch();
        for (_, s) in &srcs {
            let _ = rusty_zstd::compress(s, lvl).expect("compress");
        }
        let (events, resc) = rusty_zstd::take_wide_latch();
        let c = rusty_zstd::prof_encode_counts();
        println!(
            "{:>5} {:>10} {:>9} {:>15} {:>13.3} {:>13.3} {:>10.2}%",
            lvl,
            format!("{:?}", p.strategy),
            events,
            resc,
            resc as f64 / total as f64,
            c.hash_probes as f64 / total as f64,
            if c.hash_probes > 0 { 100.0 * resc as f64 / c.hash_probes as f64 } else { 0.0 }
        );
    }
    println!(
        "\n`resc./input B` is extra table inserts per input byte that exist ONLY\n\
         because the latch fired. `resc/probe` compares that rebuild against the\n\
         entire per-position search it is meant to make cheaper -- if the rebuild\n\
         is a large fraction of the search, the latch is paying for itself out of\n\
         the same budget it is trying to reduce."
    );
}
