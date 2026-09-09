//! The one head-to-head signal that survives a 16% null: at L7/L9 we read
//! 11.5-12.2x slower than C on INCOMPRESSIBLE data. An 11x gap is not noise,
//! so it should be visible in a deterministic counter -- and if it is, the
//! clock is not needed at all.
//!
//! `incomp_skip` is a LIVE arm at L1/L3 (`allgates`), but the levels above
//! run the chain/lazy finders. If the skip is not reaching them, we walk full
//! hash chains over random bytes that can never match.
use rusty_zstd as rz;
fn main() {
    let f = std::fs::read("corpora/data/generated/incomp-32m")
        .expect("incomp-32m");
    let src = &f[..f.len().min(1 << 20)];
    println!("incompressible input: {} KiB\n", src.len() >> 10);
    println!("{:>4}{:>10}{:>14}{:>14}{:>14}{:>12}",
             "L", "strategy", "positions", "candidates", "chain loads", "out bytes");
    println!("{}", "-".repeat(70));
    for lvl in [1i32, 3, 5, 7, 9, 12] {
        let p = rz::compression_params(lvl, Some(src.len() as u64)).unwrap();
        rz::prof_reset();
        let _ = rz::take_mm();
        let _ = rz::take_walk_census();
        let out = rz::compress_with(src, rz::CompressOptions { level: lvl, checksum: false }).unwrap();
        let c = rz::prof_encode_counts();
        let pos = rz::take_mm().0;
        let walk = rz::take_walk_census().0;
        println!("{:>4}{:>10}{:>14}{:>14}{:>14}{:>12}",
                 lvl, format!("{:?}", p.strategy), pos, c.hash_probes, walk, out.len());
    }
    println!("\npositions/candidates are Fast/DFast counters; `chain loads` (WALK_EXAM)");
    println!("is the Greedy/Lazy ladder's. A large chain-load count on data that");
    println!("cannot match is work spent proving there is no match.");
}
