//! Size the hash from the SOURCE rather than the window: what it saves in
//! zeroed table, what it costs in ratio, what it buys in time.
//!
//! Ratio is exact. Time is measured only as an arm-vs-arm ratio in ONE process
//! with a null, so a loaded box moves both arms together.
use rusty_zstd as rz;
use std::time::Instant;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao",
    "webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","incomp-32m"];
fn main() {
    for cap in [256usize << 10, 1 << 20] {
        let srcs: Vec<Vec<u8>> = IDS.iter().filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok().map(|f| { let n = f.len().min(cap); f[..n].to_vec() })
        }).collect();
        let n_in: u64 = srcs.iter().map(|s| s.len() as u64).sum();
        for lvl in [5i32, 7, 9, 12] {
            let go = || -> (u64, u64) {
                let (mut b, mut t) = (0u64, 0u64);
                for s in &srcs {
                    rz::prof_reset();
                    let p = rz::compression_params(lvl, Some(s.len() as u64)).unwrap();
                    b += rz::compress_with_params(s, p, false).unwrap().len() as u64;
                    let c = rz::prof_encode_counts();
                    t += c.table_hash_bytes + c.table_hash_long_bytes + c.table_chain_bytes;
                }
                (b, t)
            };
            let time = || -> f64 {
                let mut best = f64::MAX;
                for _ in 0..5 {
                    let t0 = Instant::now();
                    for s in &srcs {
                        let p = rz::compression_params(lvl, Some(s.len() as u64)).unwrap();
                        std::hint::black_box(rz::compress_with_params(s, p, false).unwrap().len());
                    }
                    let e = t0.elapsed().as_secs_f64();
                    if e < best { best = e }
                }
                best * 1000.0
            };
            rz::set_hash_tight_arm(0);
            let (b0, t0) = go();
            let m0 = time();
            let m0b = time();
            println!("\n=== L{lvl}  cap {} KiB  in {} KiB ===  C-sizing: {} B, tables {} KiB, {:.1} ms (null {:+.1}%)",
                     cap >> 10, n_in >> 10, b0, t0 >> 10, m0, (m0b / m0 - 1.0) * 100.0);
            println!("  {:>6}{:>12}{:>10}{:>12}{:>10}{:>10}", "tight", "bytes", "d size", "tables KiB", "d tbl", "speedup");
            for t in [1u32, 2, 3] {
                rz::set_hash_tight_arm(t);
                let (b, tb) = go();
                let m = time();
                println!("  {:>6}{:>12}{:>+10}{:>12}{:>9.0}%{:>9.2}x",
                         t, b, b as i64 - b0 as i64, tb >> 10,
                         (tb as f64 / t0 as f64 - 1.0) * 100.0, m0 / m);
            }
            rz::set_hash_tight_arm(0);
        }
    }
}
