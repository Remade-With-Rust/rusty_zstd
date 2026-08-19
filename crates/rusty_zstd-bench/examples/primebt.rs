//! GATE 1 @ L19 curiosity: `prime_tables` writes the CHAIN layout into what the
//! Bt ladder reads as a BINARY TREE. Does removing that write help?
//!
//! Two paths reach it: an explicit prefix (`--patch-from` / dictionary) and
//! EVERY multithread job after the first, whose overlap prefix at L19 is the
//! whole 8 MiB window.
use rusty_zstd::AdvancedOptions;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];

fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> (f64, usize) {
    let (mut b, mut o) = (f64::MAX, 0);
    for _ in 0..n { let t = Instant::now(); o = f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    (b, o)
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = if lvl >= 19 { 2 << 20 } else { 4 << 20 };
    let n = if lvl >= 13 { 2 } else { 3 };
    println!("PRIME-BT @ L{lvl} — prefix path, cap {} KiB", cap >> 10);
    println!("{:<13} {:>11} {:>11} {:>9} | {:>11} {:>11} {:>9}", "corpus", "keep B", "skip B", "size%", "keep ms", "skip ms", "time%");
    let (mut tot_k, mut tot_s) = (0i64, 0i64);
    let (mut smaller, mut larger, mut faster) = (0, 0, 0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &full[..full.len().min(cap)];
        if s.len() < 65536 { continue }
        let (pre, tail) = s.split_at(s.len()/2);
        rusty_zstd::set_prime_bt_arm(true);
        let (tk, bk) = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        rusty_zstd::set_prime_bt_arm(false);
        let (ts, bs) = best(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        // both arms must still round-trip
        let z = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        assert!(rusty_zstd::decompress_using_prefix(&z, pre).unwrap() == tail, "{id}: round-trip FAILED");
        rusty_zstd::set_prime_bt_arm(true);
        let sp = (bs as f64 / bk as f64 - 1.0) * 100.0;
        let tp = (ts / tk - 1.0) * 100.0;
        if bs < bk { smaller += 1 } else if bs > bk { larger += 1 }
        if tp < -1.0 { faster += 1 }
        tot_k += bk as i64; tot_s += bs as i64;
        println!("{:<13} {:>11} {:>11} {:>8.3}% | {:>11.2} {:>11.2} {:>8.2}%", id, bk, bs, sp, tk, ts, tp);
    }
    println!("\ntotal keep {tot_k} B, skip {tot_s} B -> {:+.4}% ({} smaller, {} larger, {} faster by >1%)",
        (tot_s as f64/tot_k as f64 - 1.0)*100.0, smaller, larger, faster);
}
