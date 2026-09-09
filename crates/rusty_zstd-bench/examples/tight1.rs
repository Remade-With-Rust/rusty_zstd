//! Per-corpus verdict for the source-sized hash at tight=1 (one bucket per
//! position instead of C's two), with a round-trip on every cell.
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb",
    "reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray",
    "text-32m","incomp-32m","zeros-32m"];
fn main() {
    let t: u32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    for cap in [64usize << 10, 256 << 10, 1 << 20, 4 << 20] {
        let mut tot = [0i64; 5];
        let mut tbl = [0i64; 5];
        let mut base = [0u64; 5];
        let mut tb0 = [0u64; 5];
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            for (k, lvl) in [1i32, 2, 3, 4, 5].iter().enumerate() {
                let p0 = { rz::set_hash_tight_arm(0);
                           rz::compression_params(*lvl, Some(s.len() as u64)).unwrap() };
                rz::prof_reset();
                let a = rz::compress_with_params(s, p0, false).unwrap().len() as i64;
                let ca = rz::prof_encode_counts();
                rz::set_hash_tight_arm(t);
                let p1 = rz::compression_params(*lvl, Some(s.len() as u64)).unwrap();
                rz::prof_reset();
                let z = rz::compress_with_params(s, p1, false).unwrap();
                let cb = rz::prof_encode_counts();
                assert_eq!(rz::decompress(&z).unwrap(), s, "{id} L{lvl} cap{cap}");
                rz::set_hash_tight_arm(0); tot[k] += z.len() as i64 - a;
                base[k] += a as u64;
                let ta = ca.table_hash_bytes + ca.table_hash_long_bytes + ca.table_chain_bytes;
                let tbb = cb.table_hash_bytes + cb.table_hash_long_bytes + cb.table_chain_bytes;
                tb0[k] += ta;
                tbl[k] += tbb as i64 - ta as i64;
            }
        }
        println!("cap {:>5}K   tight={t}", cap >> 10);
        for (k, lvl) in [1i32, 2, 3, 4, 5].iter().enumerate() {
            println!("   L{:<3} size {:>+9} ({:>+7.4}%)   tables {:>+10} KiB ({:>+6.1}%)",
                     lvl, tot[k], tot[k] as f64 / base[k] as f64 * 100.0,
                     tbl[k] / 1024, tbl[k] as f64 / tb0[k] as f64 * 100.0);
        }
        println!();
    }
}
