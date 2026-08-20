//! Adjudicate the mls-wide Fast hash. OUTPUT-CHANGING: decided like a gate
//! cell -- worst corpus on the size axis, train/holdout, plus the waste
//! receipt that motivated it. The clock is reported, never decisive.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn holdout(id: &str) -> bool { matches!(id, "mr"|"ooffice"|"osdb"|"reymont"|"sao"|"webster") }
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("mls-wide hash @ L{lvl}, {} MiB cap", cap >> 20);
    println!("  {:<12} {:>10} {:>10} {:>9} {:>8}", "corpus", "4B bytes", "wide B", "size%", "time%");
    let (mut t4, mut tw) = (0u64, 0u64);
    let (mut tr4, mut trw, mut ho4, mut how) = (0u64, 0u64, 0u64, 0u64);
    let (mut worst, mut worst_id) = (f64::MIN, String::new());
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let (mut b4, mut bw) = (f64::MAX, f64::MAX);
        let (mut z4, mut zw) = (0usize, 0usize);
        for pass in 0..3 {
            for wide in [pass % 2 == 0, pass % 2 != 0] {
                rusty_zstd::set_fast_hash_arm(wide);
                for _ in 0..5 {
                    let t = Instant::now();
                    let z = rusty_zstd::compress(s, lvl).unwrap();
                    let e = t.elapsed().as_secs_f64() * 1000.0;
                    if wide { if e < bw { bw = e } zw = z.len(); }
                    else { if e < b4 { b4 = e } z4 = z.len(); }
                }
            }
        }
        rusty_zstd::set_fast_hash_arm(true);
        let z = rusty_zstd::compress(s, lvl).unwrap();
        assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id}: WIDE ROUND-TRIP FAILED");
        rusty_zstd::set_fast_hash_arm(false);
        let sp = (zw as f64 / z4 as f64 - 1.0) * 100.0;
        if sp > worst { worst = sp; worst_id = (*id).to_string(); }
        t4 += z4 as u64; tw += zw as u64;
        if holdout(id) { ho4 += z4 as u64; how += zw as u64 } else { tr4 += z4 as u64; trw += zw as u64 }
        println!("  {:<12} {:>10} {:>10} {:>8.3}% {:>7.1}%", id, z4, zw, sp, (bw / b4 - 1.0) * 100.0);
    }
    println!("  TOTAL {:+.4}%  TRAIN {:+.4}%  HOLDOUT {:+.4}%  WORST {worst_id} {:+.4}%",
        (tw as f64 / t4 as f64 - 1.0) * 100.0,
        (trw as f64 / tr4 as f64 - 1.0) * 100.0,
        (how as f64 / ho4 as f64 - 1.0) * 100.0, worst);
}
