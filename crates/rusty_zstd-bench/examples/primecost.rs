//! Where does the prefix path's time actually GO? Priming vs payload encode.
//! No optimisation is worth designing before this number exists.
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
    let n = if lvl >= 13 { 3 } else { 6 };
    println!("PRIMING SHARE @ L{lvl} — ref {} MiB, payload {} MiB", PRE>>20, PAY>>20);
    println!("{:<13} {:>10} {:>10} {:>10} {:>9} {:>12}", "corpus", "no-pref ms", "pref ms", "delta ms", "share%", "primed pos");
    let (mut tn, mut tp, mut tot_it) = (0.0f64, 0.0f64, 0u64);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        let pre = &full[..PRE];
        let tail = &full[PRE..PRE+PAY];
        let a = best(n, || rusty_zstd::compress(tail, lvl).unwrap().len());
        let _ = rusty_zstd::take_prime_iters();
        let _ = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let it = rusty_zstd::take_prime_iters();
        let b = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        tn += a; tp += b; tot_it += it;
        println!("{:<13} {:>10.1} {:>10.1} {:>10.1} {:>8.1}% {:>12}", id, a, b, b-a, (b-a)/b*100.0, it);
    }
    println!("\n  total no-pref {tn:.0} ms, pref {tp:.0} ms, delta {:.0} ms = {:.1}% of the prefix path",
        tp-tn, (tp-tn)/tp*100.0);
    println!("  primed positions {tot_it} -> {:.1} ns per primed position", (tp-tn)*1e6/tot_it as f64);
}
