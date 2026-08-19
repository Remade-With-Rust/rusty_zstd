//! GATE 5 shipped default: verify at every campaign level, and that the arm-off
//! fallback restores the pre-dispatch bytes.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    println!("{:>5} {:>12} {:>12} {:>10} {:>10} {:>8}", "lvl", "off", "on", "total", "worst", "better");
    for lvl in [1i32, 3, 5, 9, 13, 19, 22] {
        let (mut a, mut b, mut worst, mut better, mut n) = (0i64, 0i64, f64::MIN, 0, 0);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(if lvl >= 13 { cap/4 } else { cap })];
            rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
            let off = rusty_zstd::compress(s, lvl).unwrap();
            rusty_zstd::set_g5_arms(0.30, 0.70, 1.50);
            let on = rusty_zstd::compress(s, lvl).unwrap();
            assert!(rusty_zstd::decompress(&on).unwrap() == s, "{id} L{lvl}: round-trip");
            let pc = (on.len() as f64/off.len() as f64 - 1.0)*100.0;
            if pc > worst { worst = pc } if on.len() < off.len() { better += 1 }
            a += off.len() as i64; b += on.len() as i64; n += 1;
        }
        println!("{:>5} {:>12} {:>12} {:>9.4}% {:>9.3}% {:>6}/{}", lvl, a, b, (b as f64/a as f64-1.0)*100.0, worst, better, n);
    }
}
