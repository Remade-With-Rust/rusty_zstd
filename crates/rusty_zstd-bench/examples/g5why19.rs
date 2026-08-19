//! Which branch blocks the dispatch at L19? Disable one term at a time.
const IDS: &[&str] = &["samba","sao","mr","xml","x-ray","mozilla","osdb","webster","nci","versions-16m","zeros-32m","incomp-32m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    // (label, rep, ratio, drift)
    let arms: &[(&str, f32, f32, f32)] = &[
        ("shipped        ", 0.30, 0.70, 1.50),
        ("rep OFF        ", 2.00, 0.70, 1.50),
        ("rep OFF+drift.5", 2.00, 0.70, 0.50),
        ("rep OFF+ratio.5", 2.00, 0.50, 1.50),
        ("all loose      ", 2.00, 0.50, 0.50),
    ];
    println!("GATE 5 @ L{lvl} branch probe (cap {} KiB)", cap>>10);
    println!("{:<16} {:>10} {:>9} {:>9}", "arm", "total", "worst", "improved");
    for (label, r, a, d) in arms {
        let (mut on, mut off, mut worst, mut wid, mut better) = (0i64, 0i64, f64::MIN, "", 0);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
            let b = rusty_zstd::compress(s, lvl).unwrap().len();
            rusty_zstd::set_g5_arms(*r, *a, *d);
            let n = rusty_zstd::compress(s, lvl).unwrap().len();
            let pc = (n as f64/b as f64 - 1.0)*100.0;
            if pc > worst { worst = pc; wid = id }
            if n < b { better += 1 }
            on += n as i64; off += b as i64;
        }
        println!("{:<16} {:>9.4}% {:>8.3}% ({wid}) {:>4}", label, (on as f64/off as f64-1.0)*100.0, worst, better);
    }
}
