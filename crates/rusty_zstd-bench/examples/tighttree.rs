//! Is the source-sized hash FREE on the tree strategies?
//!
//! `tight1.rs` showed L16 (BtOpt) costing exactly zero bytes at every cap while
//! cutting the table 25%. The tree finders address candidates through the
//! binary tree, not through a hash chain, so extra hash buckets buy them very
//! little. This checks that across the whole tree ladder and more sizes.
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao",
    "webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","incomp-32m"];
fn main() {
    println!("{:>5}{:>9}{:>12}{:>11}{:>10}{:>13}{:>9}",
             "L", "cap KiB", "base bytes", "d size", "d size %", "tables KiB", "d tbl %");
    println!("{}", "-".repeat(70));
    for lvl in [13i32, 16, 19, 22] {
        for cap in [64usize << 10, 256 << 10, 1 << 20, 4 << 20] {
            let (mut b0, mut b1, mut t0v, mut t1v) = (0u64, 0u64, 0u64, 0u64);
            for id in IDS {
                let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
                let s = &f[..f.len().min(cap)];
                for (t, bs, ts) in [(0u32, &mut b0, &mut t0v), (1, &mut b1, &mut t1v)] {
                    rz::set_hash_tight_arm(t);
                    let p = rz::compression_params(lvl, Some(s.len() as u64)).unwrap();
                    rz::prof_reset();
                    let z = rz::compress_with_params(s, p, false).unwrap();
                    if t == 1 { assert_eq!(rz::decompress(&z).unwrap(), s, "{id} L{lvl}"); }
                    *bs += z.len() as u64;
                    let c = rz::prof_encode_counts();
                    *ts += c.table_hash_bytes + c.table_hash_long_bytes + c.table_chain_bytes;
                }
            }
            rz::set_hash_tight_arm(0);
            println!("{:>5}{:>9}{:>12}{:>+11}{:>9.4}%{:>13}{:>8.1}%",
                     lvl, cap >> 10, b0, b1 as i64 - b0 as i64,
                     (b1 as i64 - b0 as i64) as f64 / b0 as f64 * 100.0,
                     t0v >> 10, (t1v as f64 / t0v as f64 - 1.0) * 100.0);
        }
    }
}
