//! GATE 2 @ L19 — is the reference even REACHABLE?
//!
//! `compress_using_prefix` sizes params from `src.len()` ALONE:
//!     let params = compression_params(level, Some(src.len() as u64))?;
//! libzstd sizes them from BOTH -- `ZSTD_adjustCParams(cPar, srcSize, dictSize)`
//! clamps windowLog against `srcSize + dictSize`. With a 4 MiB reference and a
//! 1 MiB payload we therefore pick windowLog 20 (1 MiB) and CANNOT REACH 3 of the
//! 4 MiB. The prefix-bound constant then correctly discards what the window can
//! never see -- so the bound is right and the window is wrong.
fn main() {
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
    const PRE: usize = 4 << 20;
    const PAY: usize = 1 << 20;
    for &lvl in &[3i32, 19] {
        println!("\n=== L{lvl}: window from payload only (shipped) vs payload+prefix (C's rule) ===");
        println!("{:<13} {:>5} {:>5} {:>11} {:>11} {:>9}", "corpus", "wlogA", "wlogB", "shipped", "c-rule", "delta%");
        let (mut better, mut worse, mut ta, mut tb) = (0, 0, 0i64, 0i64);
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if full.len() < PRE + PAY { continue }
            let pre = &full[..PRE];
            let tail = &full[PRE..PRE + PAY];
            let pa = rusty_zstd::compression_params(lvl, Some(tail.len() as u64)).unwrap();
            let pb = rusty_zstd::compression_params(lvl, Some((tail.len() + pre.len()) as u64)).unwrap();
            let a = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            let b = rusty_zstd::compress_with_history(tail, pb, true, None, pre, false).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&b, pre).unwrap() == tail, "{id}: round-trip");
            let d = (b.len() as f64 / a.len() as f64 - 1.0) * 100.0;
            if b.len() < a.len() { better += 1 } else if b.len() > a.len() { worse += 1 }
            ta += a.len() as i64; tb += b.len() as i64;
            println!("{:<13} {:>5} {:>5} {:>11} {:>11} {:>8.2}%", id, pa.window_log, pb.window_log, a.len(), b.len(), d);
        }
        println!("  C-rule smaller on {better}, larger on {worse} | total {ta} -> {tb} ({:+.3}%)",
            (tb as f64/ta as f64 - 1.0)*100.0);
    }
}
