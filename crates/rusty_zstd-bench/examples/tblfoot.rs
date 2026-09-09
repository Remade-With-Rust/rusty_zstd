//! Table footprint of the two finders, against this box's cache levels.
//! Decides whether a slot access is an L2 hit or a DRAM round trip -- i.e.
//! whether the slot primitive is instruction-bound or latency-bound.
const IDS: &[&str] = &["dickens", "webster", "mozilla", "samba", "nci", "x-ray"];
fn main() {
    let cap: usize = 8 << 20;
    println!("{:<6} {:<8} {:>9} {:>10} {:>10} {:>10} {:>11}",
             "level", "strategy", "hash_log", "short KiB", "long KiB", "tags KiB", "total KiB");
    for lvl in [1i32, 3] {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        let mut agg = (0u64, 0u64, 0u64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::prof_reset();
            let _ = rusty_zstd::compress(s, lvl).unwrap();
            let c = rusty_zstd::prof_encode_counts();
            agg.0 = agg.0.max(c.table_hash_bytes);
            agg.1 = agg.1.max(c.table_hash_long_bytes);
            agg.2 = agg.2.max(c.table_chain_bytes);
        }
        let tot = agg.0 + agg.1 + agg.2;
        println!("{:<6} {:<8} {:>9} {:>10} {:>10} {:>10} {:>11}",
                 format!("L{lvl}"), format!("{:?}", p.strategy), p.hash_log,
                 agg.0 / 1024, agg.1 / 1024, agg.2 / 1024, tot / 1024);
    }
    println!("\n(table_chain_bytes doubles as the tag-array column for fast/dfast.)");
}
