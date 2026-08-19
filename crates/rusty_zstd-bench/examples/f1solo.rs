//! FINDING 1 ALONE, with the tree OFF.
//!
//! Every measurement of Finding 1 so far carried Finding 2's cost: a wider
//! window gives the tree more positions to insert, so the two compounded. With
//! heads-only priming the window widening costs only the extra heads -- a few ns
//! per position -- and may PAY for itself by letting the search reach further.
//!
//! Finding 1 is also a CONFORMANCE issue on its own terms: libzstd's
//! ZSTD_adjustCParams(cPar, srcSize, dictSize) clamps windowLog against
//! srcSize + dictSize. We clamp against srcSize alone, so a caller who supplies
//! a dictionary larger than the payload cannot reference most of it.
use rusty_zstd::Dictionary;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(5);
    for &(dk, payk) in &[(4096usize, 1024usize)] {
        let (dn, pn) = (dk << 10, payk << 10);
        println!("\n=== FINDING 1 ALONE (tree OFF) — dict {dk} KiB / payload {payk} KiB @ L{lvl}, best-of-{n} ===");
        println!("{:<13} {:>5} {:>5} {:>10} {:>10} {:>8} | {:>9} {:>9} {:>8}", "corpus", "wlogA", "wlogB", "off B", "on B", "size%", "off ms", "on ms", "time%");
        let (mut sb, mut sn, mut tb, mut tn) = (0i64,0i64,0.0f64,0.0f64);
        let (mut bigger, mut slower, mut ws, mut wt) = (0, 0, f64::MIN, f64::MIN);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if f.len() < dn + pn { continue }
            let d = Dictionary::raw(f[..dn].to_vec());
            let tail = &f[dn..dn+pn];
            let run = |on: bool| -> (f64, usize, u32) {
                rusty_zstd::set_prime_bt_tree_arm(false);          // TREE OFF -- isolate Finding 1
                rusty_zstd::set_prefix_window_arm(on);
                let p = rusty_zstd::compression_params(lvl, Some(if on { (tail.len()+dn) as u64 } else { tail.len() as u64 })).unwrap();
                let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
                assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == tail, "{id}: round-trip");
                let mut b = f64::MAX;
                for _ in 0..n { let s = Instant::now(); let _ = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap(); let e = s.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
                (b, z.len(), p.window_log)
            };
            let (t0,b0,w0) = run(false);
            let (t1,b1,w1) = run(true);
            let sd = (b1 as f64/b0 as f64-1.0)*100.0;
            let td = (t1/t0-1.0)*100.0;
            if b1 > b0 { bigger += 1 } if td > 2.0 { slower += 1 }
            if sd > ws { ws = sd } if td > wt { wt = td }
            sb += b0 as i64; sn += b1 as i64; tb += t0; tn += t1;
            println!("{:<13} {:>5} {:>5} {:>10} {:>10} {:>7.3}% | {:>9.1} {:>9.1} {:>7.1}%", id, w0, w1, b0, b1, sd, t0, t1, td);
        }
        println!("  TOTAL size {:+.3}% | time {:+.2}% | bigger {bigger}, slower(>2%) {slower} | worst size {ws:+.3}%, worst time {wt:+.1}%",
            (sn as f64/sb as f64-1.0)*100.0, (tn/tb-1.0)*100.0);
    }
    rusty_zstd::set_prefix_window_arm(false);
}
