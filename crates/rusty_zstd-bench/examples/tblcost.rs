//! How much table does a SMALL input have to zero before it can be compressed?
//!
//! `incomp-32m` at 1 MiB does almost no search at any level (0 chain loads at
//! L7/L9) and emits raw blocks -- yet the head-to-head board reads 149 MB/s
//! against C's 1719. Near-zero search plus raw output means the time is not in
//! the finder; it is in the SETUP. `MatchTables::new` allocates and zeroes the
//! hash/chain/long tables, and `vec![0; n]` on a fresh allocation is a memset.
//!
//! This prints the zeroed footprint per level against the input it serves.
use rusty_zstd as rz;
fn main() {
    let f = std::fs::read("corpora/data/generated/incomp-32m").expect("incomp-32m");
    println!("{:>7}{:>4}{:>10}{:>11}{:>11}{:>11}{:>12}{:>9}",
             "input", "L", "strategy", "hash KiB", "long KiB", "chain KiB", "total KiB", "x input");
    println!("{}", "-".repeat(76));
    for cap in [64usize << 10, 256 << 10, 1 << 20, 4 << 20] {
        let src = &f[..f.len().min(cap)];
        for lvl in [1i32, 3, 7, 9, 12] {
            let p = rz::compression_params(lvl, Some(src.len() as u64)).unwrap();
            rz::prof_reset();
            let _ = rz::compress_with(src, rz::CompressOptions { level: lvl, checksum: false }).unwrap();
            let c = rz::prof_encode_counts();
            let tot = c.table_hash_bytes + c.table_hash_long_bytes + c.table_chain_bytes;
            println!("{:>6}K{:>4}{:>10}{:>11}{:>11}{:>11}{:>12}{:>8.1}x",
                     cap >> 10, lvl, format!("{:?}", p.strategy),
                     c.table_hash_bytes / 1024, c.table_hash_long_bytes / 1024,
                     c.table_chain_bytes / 1024, tot / 1024,
                     tot as f64 / src.len() as f64);
        }
        println!();
    }
}
