//! FINDINGS 1 + 2 combined: before/after on the prefix path, size AND time,
//! with the C binary as the external anchor.
use std::process::Command;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn set(on: bool) { rusty_zstd::set_prime_bt_tree_arm(on); rusty_zstd::set_prefix_window_arm(on); }
fn main() {
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let n = if lvl >= 13 { 2 } else { 3 };
    println!("FINDINGS 1+2 @ L{lvl} — ref {} MiB, payload {} MiB", PRE>>20, PAY>>20);
    println!("{:<13} {:>10} {:>10} {:>8} | {:>9} {:>9} {:>8} | {:>10} {:>7}",
        "corpus", "before B", "after B", "size%", "before ms", "after ms", "time%", "C bytes", "us/c");
    let (mut b0, mut b1, mut t0, mut t1, mut cb) = (0i64, 0i64, 0.0, 0.0, 0i64);
    let (mut smaller, mut larger) = (0, 0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        let pre = &full[..PRE];
        let tail = &full[PRE..PRE+PAY];
        set(false);
        let a = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let ta = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        set(true);
        let b = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let tb = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        assert!(rusty_zstd::decompress_using_prefix(&b, pre).unwrap() == tail, "{id}: round-trip FAILED");
        // C anchor
        let rf = format!("target/_f12_{id}.ref"); let pf = format!("target/_f12_{id}.pay"); let of = format!("target/_f12_{id}.zst");
        std::fs::write(&rf, pre).unwrap(); std::fs::write(&pf, tail).unwrap();
        let _ = Command::new(zstd).args(["--ultra", &format!("-{lvl}"), "-f", &format!("--patch-from={rf}"), &pf, "-o", &of]).output();
        let csz = std::fs::metadata(&of).map(|m| m.len() as usize).unwrap_or(0);
        for f in [&rf,&pf,&of] { let _ = std::fs::remove_file(f); }
        if b.len() < a.len() { smaller += 1 } else if b.len() > a.len() { larger += 1 }
        b0 += a.len() as i64; b1 += b.len() as i64; t0 += ta; t1 += tb; cb += csz as i64;
        println!("{:<13} {:>10} {:>10} {:>7.2}% | {:>9.0} {:>9.0} {:>7.1}% | {:>10} {:>7.3}",
            id, a.len(), b.len(), (b.len() as f64/a.len() as f64-1.0)*100.0, ta, tb, (tb/ta-1.0)*100.0,
            csz, if csz>0 { b.len() as f64/csz as f64 } else { 0.0 });
    }
    set(true);
    println!("\n  SIZE  {b0} -> {b1} ({:+.4}%) | smaller {smaller}, larger {larger}", (b1 as f64/b0 as f64-1.0)*100.0);
    println!("  TIME  {t0:.0} -> {t1:.0} ms ({:+.1}%)", (t1/t0-1.0)*100.0);
    println!("  us/c  {:.4} before -> {:.4} after", b0 as f64/cb as f64, b1 as f64/cb as f64);
}
