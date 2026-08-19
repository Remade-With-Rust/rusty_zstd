//! FINDINGS 1+2: the EXTENT sweep -- the third cost axis, never swept.
//! Stride moves along the cost/benefit line; depth had a knee at 5. Does extent
//! have a better one? Judged against C, which is what decides shipping.
use rusty_zstd::Dictionary;
use std::process::Command;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let zstd = "third_party/zstd/extracted/zstd-v1.5.7-win64/zstd.exe";
    let mut set = Vec::new();
    let (mut csum, mut cbytes) = (0.0f64, 0i64);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        let (rf, pf, of) = (format!("target/_e_{id}.ref"), format!("target/_e_{id}.pay"), format!("target/_e_{id}.zst"));
        std::fs::write(&rf, &full[..PRE]).unwrap();
        std::fs::write(&pf, &full[PRE..PRE+PAY]).unwrap();
        let mut cb = f64::MAX;
        for _ in 0..3 {
            let t = Instant::now();
            let st = Command::new(zstd).args(["--ultra", &format!("-{lvl}"), "-f", "-q", "--patch-from", &rf, &pf, "-o", &of]).output().unwrap();
            assert!(st.status.success());
            let e = t.elapsed().as_secs_f64()*1000.0; if e < cb { cb = e; }
        }
        cbytes += std::fs::metadata(&of).unwrap().len() as i64;
        csum += cb;
        set.push((*id, full[..PRE].to_vec(), full[PRE..PRE+PAY].to_vec()));
        for f in [&rf,&pf,&of] { let _ = std::fs::remove_file(f); }
    }
    println!("FINDINGS 1+2 EXTENT SWEEP @ L{lvl} — {} corpora, C is the judge", set.len());
    println!("{:>8} {:>12} {:>10} {:>10} {:>10} {:>9}", "extent", "bytes", "us/c size", "us ms", "C/us time", "beat C");
    for &(on, ext) in &[(false,1u32),(true,1),(true,2),(true,4),(true,8),(true,16),(true,32)] {
        rusty_zstd::set_prime_bt_tree_arm(on);
        rusty_zstd::set_prefix_window_arm(on);
        rusty_zstd::set_prime_bt_depth_arm(if on {5} else {0});
        rusty_zstd::set_prime_bt_extent_arm(ext);
        let (mut b, mut t, mut win) = (0i64, 0.0f64, 0);
        for (id, pre, tail) in &set {
            let d = Dictionary::raw(pre.clone());
            let z = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == *tail, "{id}: round-trip");
            let mut best = f64::MAX;
            for _ in 0..3 { let s = Instant::now(); let _ = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap(); let e = s.elapsed().as_secs_f64()*1000.0; if e < best { best = e; } }
            b += z.len() as i64; t += best;
            if best < csum/set.len() as f64 * 3.0 { }
            win += 0;
        }
        let _ = win;
        println!("{:>8} {:>12} {:>10.4} {:>10.0} {:>10.2} {:>9}",
            if !on { "OFF".to_string() } else { format!("1/{ext}") },
            b, b as f64/cbytes as f64, t, csum/t, if csum/t > 1.0 { "yes" } else { "no" });
    }
    rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prefix_window_arm(false); rusty_zstd::set_prime_bt_extent_arm(1);
}
