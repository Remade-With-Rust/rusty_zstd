//! FINDING 2: does building the Bt tree over the prefix pay?
//! Size AND time, because ZSTD_updateTree is not free.
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let n = if lvl >= 13 { 2 } else { 3 };
    println!("FINDING 2 @ L{lvl} — heads-only vs tree-built prefix priming (ref {} MiB, payload {} MiB)", PRE>>20, PAY>>20);
    println!("{:<13} {:>11} {:>11} {:>9} | {:>10} {:>10} {:>9}", "corpus", "heads B", "tree B", "size%", "heads ms", "tree ms", "time%");
    let (mut smaller, mut larger, mut th, mut tt) = (0, 0, 0i64, 0i64);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        let pre = &full[..PRE];
        let tail = &full[PRE..PRE+PAY];
        rusty_zstd::set_prime_bt_tree_arm(false);
        let a = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let ta = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        rusty_zstd::set_prime_bt_tree_arm(true);
        let b = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let tb = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        assert!(rusty_zstd::decompress_using_prefix(&b, pre).unwrap() == tail, "{id}: round-trip FAILED");
        if b.len() < a.len() { smaller += 1 } else if b.len() > a.len() { larger += 1 }
        th += a.len() as i64; tt += b.len() as i64;
        println!("{:<13} {:>11} {:>11} {:>8.2}% | {:>10.1} {:>10.1} {:>8.1}%", id, a.len(), b.len(),
            (b.len() as f64/a.len() as f64 - 1.0)*100.0, ta, tb, (tb/ta - 1.0)*100.0);
    }
    println!("\n  total {th} -> {tt} ({:+.4}%) | smaller {smaller}, larger {larger}",
        (tt as f64/th as f64 - 1.0)*100.0);
}
