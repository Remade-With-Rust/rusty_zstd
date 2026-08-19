//! Is the per-frame TABLE ALLOCATION the floor at high levels?
//! At L22 chain_log can reach 24 -> a 64 MiB `vec![0; ..]` zeroed per frame.
//! Compress a tiny payload: whatever time remains is setup, not compression.
use std::time::Instant;
fn main() {
    let tiny = vec![7u8; 4096];
    let med: Vec<u8> = std::fs::read("corpora/data/silesia/xml").unwrap()[..1<<20].to_vec();
    println!("{:>5} {:>6} {:>6} {:>10} {:>12} {:>12} {:>10}", "lvl", "wlog", "clog", "chain MiB", "4 KiB ms", "1 MiB ms", "setup%");
    for lvl in [1i32, 3, 13, 16, 19, 22] {
        let pt = rusty_zstd::compression_params(lvl, Some(4096)).unwrap();
        let pm = rusty_zstd::compression_params(lvl, Some(med.len() as u64)).unwrap();
        let chain_mib = ((1u64 << pm.chain_log.min(24)) * 4) as f64 / 1048576.0;
        let mut bt = f64::MAX;
        for _ in 0..20 { let t = Instant::now(); let _ = rusty_zstd::compress(&tiny, lvl).unwrap(); let e = t.elapsed().as_secs_f64()*1000.0; if e < bt { bt = e; } }
        let mut bm = f64::MAX;
        for _ in 0..3 { let t = Instant::now(); let _ = rusty_zstd::compress(&med, lvl).unwrap(); let e = t.elapsed().as_secs_f64()*1000.0; if e < bm { bm = e; } }
        println!("{:>5} {:>6} {:>6} {:>10.1} {:>12.3} {:>12.1} {:>9.1}%",
            lvl, pt.window_log, pm.chain_log, chain_mib, bt, bm, bt/bm*100.0);
    }
}
