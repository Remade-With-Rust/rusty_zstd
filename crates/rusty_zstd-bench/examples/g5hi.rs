//! GATE 5 at the HIGH levels: what do the L3-fitted thresholds do at L19/L22,
//! and is a separate fit warranted the way it was for the Fast ladder?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    println!("GATE 5 @ L{lvl} — shipped L3+ thresholds (cap {} KiB)", cap>>10);
    println!("{:<13} {:>11} {:>11} {:>9}", "corpus", "off", "on", "delta");
    let (mut a, mut b, mut worst, mut wid, mut better) = (0i64, 0i64, f64::MIN, "", 0);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::set_g5_opt_arms(-1.0, 2.0, 2.0);
        let off = rusty_zstd::compress(s, lvl).unwrap();
        rusty_zstd::set_g5_opt_arms(2.0, 0.50, 1.50);
        let on = rusty_zstd::compress(s, lvl).unwrap();
        assert!(rusty_zstd::decompress(&on).unwrap() == s, "{id}: round-trip");
        let pc = (on.len() as f64/off.len() as f64 - 1.0)*100.0;
        if pc > worst { worst = pc; wid = id }
        if on.len() < off.len() { better += 1 }
        a += off.len() as i64; b += on.len() as i64;
        println!("{:<13} {:>11} {:>11} {:>8.3}%", id, off.len(), on.len(), pc);
    }
    println!("\n  total {:+.4}%   worst {:+.3}% ({wid})   improved {better}/18",
        (b as f64/a as f64-1.0)*100.0, worst);
}
