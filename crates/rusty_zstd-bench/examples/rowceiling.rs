//! Ceiling probe for the ROW MATCH FINDER, before building it.
//!
//! zstd's `ZSTD_row_match_finder` replaces the hash-CHAIN walk on the lazy
//! ladder. The chain walk is a SERIAL DEPENDENT-LOAD chain: every step loads
//! the next candidate index out of the chain table, so each step is a
//! potential cache miss that cannot issue until the previous one retired.
//! The row finder replaces N such steps with ONE load of a 16-tag row plus a
//! SIMD compare.
//!
//! So the ceiling is a COUNT, not a clock: `WALK_EXAM` is exactly the number
//! of dependent chain-table loads the row finder would delete. Deterministic,
//! one run, immune to the 10.88% null arm this box is carrying.
//!
//! Prints exams/KiB and the row-equivalent (exams/16) per level, so the
//! arithmetic that decides whether to build it happens BEFORE building it
//! (codec-measurement 11: prune on arithmetic first).
//!
//! Requires --features profile.
const IDS: &[&str] = &[
    "dickens", "mozilla", "samba", "webster", "xml", "x-ray", "osdb", "reymont", "nci", "sao", "mr", "ooffice",
];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let levels: Vec<i32> = match std::env::args().nth(1) {
        Some(s) => s.split(',').map(|v| v.parse().unwrap()).collect(),
        None => vec![1, 3, 5, 7, 9, 12, 15, 19],
    };
    println!("ROW-FINDER CEILING -- deterministic counts, no clock\n");
    println!("{:<7}{:<10}{:>8}{:>16}{:>12}{:>12}{:>11}",
        "level", "strategy", "MiB", "chain loads", "loads/KiB", "rows(/16)", "saved");
    for lvl in levels {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        let (mut tot, mut bytes) = (0u64, 0u64);
        for id in IDS {
            let Some(f) = load(id) else { continue };
            let src = &f[..f.len().min(16 << 20)];
            let _ = rusty_zstd::take_walk_census();
            let z = rusty_zstd::compress(src, lvl).unwrap();
            std::hint::black_box(&z);
            let (exam, _miss) = rusty_zstd::take_walk_census();
            tot += exam;
            bytes += src.len() as u64;
        }
        let kib = bytes as f64 / 1024.0;
        let rows = (tot as f64 / 16.0).ceil();
        println!("L{:<6}{:<10}{:>8.1}{:>16}{:>12.1}{:>12.0}{:>10.1}x",
            lvl, p.strategy.name(), bytes as f64 / (1 << 20) as f64, tot,
            tot as f64 / kib, rows,
            if rows > 0.0 { tot as f64 / rows } else { 0.0 });
    }
    println!("\nchain loads = WALK_EXAM = serial dependent loads into the chain table.");
    println!("A row finder replaces each RUN of them with one 16-tag row load + SIMD compare.");
    println!("Zero on a level means that level does not walk a chain -- the row finder");
    println!("cannot help there no matter how well it is written.");
}
