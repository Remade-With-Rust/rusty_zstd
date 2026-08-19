//! GATE 2 @ L22 — how DENSELY to prime the prefix.
//!
//! Objective for this cell: SPEED, tolerating a minimal size increase. That
//! reopens the priming stride, which was refused twice under strict size parity.
//!
//! `stride = huge` primes exactly one position, i.e. the prefix is copied and
//! its history is reachable by repcode/back-extension but contributes no hash
//! heads -- the "priming off" end of the dial.
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
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    let n = if lvl >= 13 { 2 } else { 8 };
    let mut srcs = Vec::new();
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        srcs.push((*id, full[..PRE].to_vec(), full[PRE..PRE+PAY].to_vec()));
    }
    println!("GATE 2 @ L{lvl} — priming DENSITY sweep (ref {} MiB, payload {} MiB, best-of-{n})", PRE>>20, PAY>>20);
    println!("{:>10} {:>13} {:>10} {:>11} {:>10} {:>9}", "stride", "bytes", "size%", "ms", "time%", "worst+%");
    let mut base_b = 0i64; let mut base_t = 0.0f64;
    let mut base_each: Vec<usize> = Vec::new();
    for (i, stride) in [1usize, 2, 4, 8, 16, 64, 1<<30].iter().copied().enumerate() {
        rusty_zstd::set_prime_stride_arm(stride);
        let (mut b, mut t) = (0i64, 0.0f64);
        let mut each = Vec::new();
        let mut worst = f64::MIN;
        for (id, pre, tail) in &srcs {
            let z = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&z, pre).unwrap() == *tail, "{id}: round-trip");
            b += z.len() as i64;
            each.push(z.len());
            t += best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        }
        if i == 0 { base_b = b; base_t = t; base_each = each.clone(); }
        for (k, v) in each.iter().enumerate() {
            let d = (*v as f64 / base_each[k] as f64 - 1.0) * 100.0;
            if d > worst { worst = d; }
        }
        println!("{:>10} {:>13} {:>9.3}% {:>11.0} {:>9.1}% {:>8.2}%",
            if stride > 1000 { "off".to_string() } else { stride.to_string() },
            b, (b as f64/base_b as f64-1.0)*100.0, t, (t/base_t-1.0)*100.0, worst);
    }
    rusty_zstd::set_prime_stride_arm(1);
}
