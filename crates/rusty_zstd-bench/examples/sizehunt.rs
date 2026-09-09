//! SIZE HUNT -- deterministic wins in the one currency that has no noise floor.
//!
//!   cargo run --release -p rusty_zstd-bench --example sizehunt
//!
//! Every speed verdict on this box fights a +-1.5% null band (`eqlever.rs`).
//! COMPRESSED BYTES have no null band at all: same input, same arm, same
//! number, on any machine at any load. So an arm value that is strictly
//! SMALLER on some corpus is a deterministic win the moment a signal separates
//! it from the corpora it hurts -- which is the content-adaptive dispatch this
//! encoder already uses for `pair_gain`, `rep1_mode`, `incomp_skip`.
//!
//! `allgates` prints only the first four moved cells per arm, so the negative
//! ones are mostly invisible there. This prints EVERY cell, sorted, and flags
//! the arms where a non-default value never loses.
use rusty_zstd as rz;

const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m",
    "versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla",
    "nci","samba","xml","x-ray"];

fn sizes(lvl: i32, srcs: &[(&str, Vec<u8>)]) -> Vec<usize> {
    srcs.iter().map(|(_, s)| rz::compress_with(s,
        rz::CompressOptions { level: lvl, checksum: false }).unwrap().len()).collect()
}

fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    println!("SIZE HUNT  cap={} KiB  corpora={}\n", cap >> 10, srcs.len());

    type S = fn(bool);
    // (name, setter, default, level) -- arms reaching the two finders under study.
    let bools: &[(&str, S, bool, i32)] = &[
        ("pair_on",       rz::set_pair_on_arm as S,  true,  1),
        ("tag_alloc",     rz::set_tag_alloc_arm,     true,  1),
        ("pipe_rep1",     rz::set_pipe_rep1_arm,     true,  1),
        ("next_long",     rz::set_next_long_arm,     true,  3),
        ("dfast_bext",    rz::set_dfast_bext_arm,    true,  3),
        ("fast_pack",     rz::set_fast_pack_arm,     true,  1),
        ("fast_hash",     rz::set_fast_hash_arm,     true,  1),
        ("long_tag",      rz::set_long_tag_arm,      true,  3),
        ("dfast_tag",     rz::set_dfast_tag_arm,     true,  3),
    ];
    for (name, set, deflt, lvl) in bools {
        set(*deflt);
        let a = sizes(*lvl, &srcs);
        set(!*deflt);
        let b = sizes(*lvl, &srcs);
        set(*deflt);
        let mut cells: Vec<(i64, &str)> = a.iter().zip(b.iter()).zip(srcs.iter())
            .map(|((x, y), (id, _))| (*y as i64 - *x as i64, *id)).collect();
        cells.sort();
        let wins: Vec<&(i64, &str)> = cells.iter().filter(|(d, _)| *d < 0).collect();
        let loss: Vec<&(i64, &str)> = cells.iter().filter(|(d, _)| *d > 0).collect();
        let net: i64 = cells.iter().map(|(d, _)| d).sum();
        if wins.is_empty() && loss.is_empty() { continue; }
        println!("{name} @ L{lvl}  (flipping to {})   net {net:+}", !*deflt);
        print!("   SMALLER on {}: ", wins.len());
        for (d, id) in wins.iter().take(6) { print!("{id}{d} "); }
        println!();
        print!("   larger  on {}: ", loss.len());
        for (d, id) in loss.iter().rev().take(6) { print!("{id}+{d} "); }
        println!();
        if !wins.is_empty() && loss.is_empty() {
            println!("   >>> STRICT WIN: never larger on any corpus <<<");
        }
        println!();
    }
}
