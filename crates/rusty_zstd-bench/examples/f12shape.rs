//! Findings 1+2 in the shape dictionaries are ACTUALLY used.
//!
//! Every measurement so far used a 4 MiB reference against a 1 MiB payload --
//! the --patch-from shape, and the WORST case for a tree: the build is over more
//! positions than the payload it serves. The normal dictionary case is the
//! inverse: a SMALL dictionary amortised over a LARGE payload.
//!
//! Build cost scales with dictionary positions; benefit scales with payload.
//! So the ratio `dict_positions / payload_bytes` should decide, and it has been
//! pinned at ~4.0 for every number taken so far.
use rusty_zstd::Dictionary;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    for &(dk, payk) in &[(112usize, 4096usize), (512, 4096), (112, 1024), (4096, 1024)] {
        let (dn, pn) = (dk << 10, payk << 10);
        println!("\n=== dict {dk} KiB / payload {payk} KiB  (ratio {:.3})  @ L{lvl} ===", dn as f64/pn as f64);
        println!("{:<13} {:>10} {:>10} {:>8} | {:>9} {:>9} {:>8}", "corpus", "off B", "on B", "size%", "off ms", "on ms", "time%");
        let (mut sb, mut sn, mut tb, mut tn) = (0i64,0i64,0.0f64,0.0f64);
        let (mut bigger, mut slower, mut wt) = (0, 0, f64::MIN);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            if f.len() < dn + pn { continue }
            let d = Dictionary::raw(f[..dn].to_vec());
            let tail = &f[dn..dn+pn];
            let run = |on: bool| -> (f64, usize) {
                rusty_zstd::set_prime_bt_tree_arm(on);
                rusty_zstd::set_prefix_window_arm(on);
                rusty_zstd::set_prime_bt_depth_arm(if on {5} else {0});
                rusty_zstd::set_prime_bt_extent_arm(1);
                let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
                assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == tail);
                let mut b = f64::MAX;
                for _ in 0..3 { let s = Instant::now(); let _ = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap(); let e = s.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
                (b, z.len())
            };
            let (t0,b0) = run(false);
            let (t1,b1) = run(true);
            let sd = (b1 as f64/b0 as f64-1.0)*100.0;
            let td = (t1/t0-1.0)*100.0;
            if b1 > b0 { bigger += 1 } if td > 2.0 { slower += 1 }
            if td > wt { wt = td }
            sb += b0 as i64; sn += b1 as i64; tb += t0; tn += t1;
            println!("{:<13} {:>10} {:>10} {:>7.3}% | {:>9.0} {:>9.0} {:>7.1}%", id, b0, b1, sd, t0, t1, td);
        }
        println!("  TOTAL size {:+.3}% | time {:+.2}% | bigger {bigger}, slower(>2%) {slower}, worst time {wt:+.1}%",
            (sn as f64/sb as f64-1.0)*100.0, (tn/tb-1.0)*100.0);
    }
    rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prefix_window_arm(false); rusty_zstd::set_prime_bt_extent_arm(16);
}
