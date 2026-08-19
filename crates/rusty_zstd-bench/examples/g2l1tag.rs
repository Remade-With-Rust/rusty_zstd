//! GATE 2 @ L1 curiosity: `prime_tables` calls `put_h`, which writes `hash` but
//! NOT `tags`. `store_fast` writes both. At L1 the tag filter runs on block 0
//! (`tag_yield` seeds to 1.0, `tag_min` is 0.50), so every primed prefix slot is
//! compared against a STALE tag of 0.
//!
//! If that is hurting, turning the tag filter OFF should make the prefix-primed
//! output SMALLER. Deterministic sizes — no clock needed.
fn main() {
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m","text-32m","zeros-32m","incomp-32m"];
    for &lvl in &[1i32, 3] {
        println!("\n=== L{lvl}: prefix-primed, tag filter ON (shipped) vs OFF ===");
        println!("{:<13} {:>11} {:>11} {:>9}   {:>11} {:>11} {:>9}", "corpus", "pre tagON", "pre tagOFF", "delta%", "noPre ON", "noPre OFF", "delta%");
        let (mut better, mut worse, mut tot_on, mut tot_off) = (0, 0, 0i64, 0i64);
        for id in IDS {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if full.len() < (5 << 20) { continue }
            let pre = &full[..4 << 20];
            let tail = &full[4 << 20..(4 << 20) + (1 << 20)];
            rusty_zstd::set_tag_arm(true);
            let a = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len();
            let na = rusty_zstd::compress(tail, lvl).unwrap().len();
            rusty_zstd::set_tag_arm(false);
            let b = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len();
            let nb = rusty_zstd::compress(tail, lvl).unwrap().len();
            rusty_zstd::set_tag_arm(true);
            let d = (b as f64 / a as f64 - 1.0) * 100.0;
            let nd = (nb as f64 / na as f64 - 1.0) * 100.0;
            if b < a { better += 1 } else if b > a { worse += 1 }
            tot_on += a as i64; tot_off += b as i64;
            println!("{:<13} {:>11} {:>11} {:>8.3}%   {:>11} {:>11} {:>8.3}%", id, a, b, d, na, nb, nd);
        }
        println!("  tag OFF is smaller on {better}, larger on {worse}; total {tot_on} -> {tot_off} ({:+.4}%)",
            (tot_off as f64/tot_on as f64 - 1.0)*100.0);
    }
}
