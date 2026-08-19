//! FINDINGS 1+2 candidate: is a SMALL tree both smaller and faster than none?
//! Per corpus, so the worst-corpus rule can be applied rather than a mean.
use rusty_zstd::Dictionary;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    let mut set = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        set.push((*id, f[..PRE].to_vec(), f[PRE..PRE+PAY].to_vec()));
    }
    let run = |on: bool, ext: u32, pre: &Vec<u8>, tail: &Vec<u8>| -> (f64, usize) {
        rusty_zstd::set_prime_bt_tree_arm(on);
        rusty_zstd::set_prefix_window_arm(on);
        rusty_zstd::set_prime_bt_depth_arm(if on {5} else {0});
        rusty_zstd::set_prime_bt_extent_arm(ext);
        let d = Dictionary::raw(pre.clone());
        let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
        assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == *tail);
        let mut b = f64::MAX;
        for _ in 0..n { let s = Instant::now(); let _ = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap(); let e = s.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
        (b, z.len())
    };
    for ext in [16u32, 32, 64] {
        println!("\n=== extent 1/{ext} vs OFF @ L{lvl}, best-of-{n} ===");
        println!("{:<13} {:>10} {:>10} {:>9} | {:>9} {:>9} {:>8}", "corpus", "off B", "on B", "size%", "off ms", "on ms", "time%");
        let (mut sb, mut sn, mut tb, mut tn) = (0i64,0i64,0.0f64,0.0f64);
        let (mut wsz, mut wt, mut bigger, mut slower) = (f64::MIN, f64::MIN, 0, 0);
        for (id, pre, tail) in &set {
            let (t0, b0) = run(false, 1, pre, tail);
            let (t1, b1) = run(true, ext, pre, tail);
            let sd = (b1 as f64/b0 as f64 - 1.0)*100.0;
            let td = (t1/t0 - 1.0)*100.0;
            if sd > wsz { wsz = sd } if td > wt { wt = td }
            if b1 > b0 { bigger += 1 } if td > 2.0 { slower += 1 }
            sb += b0 as i64; sn += b1 as i64; tb += t0; tn += t1;
            println!("{:<13} {:>10} {:>10} {:>8.3}% | {:>9.0} {:>9.0} {:>7.1}%", id, b0, b1, sd, t0, t1, td);
        }
        println!("  TOTAL size {:+.3}% | time {:+.2}% | bigger {bigger}, slower(>2%) {slower} | worst size {wsz:+.3}%, worst time {wt:+.1}%",
            (sn as f64/sb as f64-1.0)*100.0, (tn/tb-1.0)*100.0);
    }
    rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prefix_window_arm(false); rusty_zstd::set_prime_bt_extent_arm(1);
}
