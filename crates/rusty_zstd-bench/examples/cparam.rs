//! ZSTD_adjustCParams table clamp: speed AND size, all 18 corpora, all levels.
//! Speed is the objective, a MINIMAL size increase is the budget.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(if lvl >= 13 { 1<<20 } else { 8<<20 });
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(if lvl >= 13 { 3 } else { 9 });
    println!("CPARAM CLAMP @ L{lvl} — cap {} KiB, best-of-{n}", cap>>10);
    println!("{:<13} {:>5} {:>5} {:>10} {:>10} {:>8} | {:>9} {:>9} {:>8}",
        "corpus", "clog", "clog'", "off B", "on B", "size%", "off ms", "on ms", "time%");
    let (mut b0, mut b1, mut t0, mut t1) = (0i64, 0i64, 0.0f64, 0.0f64);
    let (mut faster, mut slower, mut bigger, mut worst) = (0, 0, 0, f64::MIN);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(cap)];
        rusty_zstd::set_cparam_clamp_arm(false);
        let pa = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
        let a = rusty_zstd::compress(src, lvl).unwrap();
        let ta = best(n, || rusty_zstd::compress(src, lvl).unwrap().len());
        rusty_zstd::set_cparam_clamp_arm(true);
        let pb = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
        let b = rusty_zstd::compress(src, lvl).unwrap();
        let tb = best(n, || rusty_zstd::compress(src, lvl).unwrap().len());
        assert!(rusty_zstd::decompress(&b).unwrap() == src, "{id}: round-trip FAILED");
        let sd = (b.len() as f64/a.len() as f64 - 1.0)*100.0;
        let td = (tb/ta - 1.0)*100.0;
        if td < -1.0 { faster += 1 } else if td > 1.0 { slower += 1 }
        if b.len() > a.len() { bigger += 1 }
        if sd > worst { worst = sd }
        b0 += a.len() as i64; b1 += b.len() as i64; t0 += ta; t1 += tb;
        println!("{:<13} {:>5} {:>5} {:>10} {:>10} {:>7.3}% | {:>9.1} {:>9.1} {:>7.1}%",
            id, pa.chain_log, pb.chain_log, a.len(), b.len(), sd, ta, tb, td);
    }
    println!("\n  SIZE {b0} -> {b1} ({:+.4}%) | bigger on {bigger}, worst +{worst:.3}%",
        (b1 as f64/b0 as f64-1.0)*100.0);
    println!("  TIME {t0:.0} -> {t1:.0} ms ({:+.2}%) | faster {faster}, slower {slower}",
        (t1/t0-1.0)*100.0);
}
