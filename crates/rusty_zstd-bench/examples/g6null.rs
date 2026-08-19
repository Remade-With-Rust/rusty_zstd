//! NULL ARM for the Gate 6 harness: measure payload_arm(true) against
//! payload_arm(true). Any reading other than ~0 is the harness, not the codec.
use std::time::Instant;
const IDS: &[&str] = &["dickens","samba","mozilla","webster","xml","incomp-32m","jsonlog-16m","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 6 NULL ARM @ L{lvl} -- same setting on both sides");
    println!("{:<13} {:>10} {:>10} {:>10} {:>10}", "corpus", "run 1", "run 2", "run 3", "|max|");
    let mut worst = 0.0f64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut pcs = Vec::new();
        for _ in 0..3 {
            let (mut a, mut b) = (f64::MAX, f64::MAX);
            for phase in 0..2 {
                for first in [phase == 0, phase != 0] {
                    rusty_zstd::set_payload_arm(true);          // IDENTICAL both sides
                    for _ in 0..7 {
                        let t = Instant::now();
                        let _ = rusty_zstd::compress(s, lvl).unwrap();
                        let e = t.elapsed().as_secs_f64()*1000.0;
                        if first { if e < a { a = e } } else if e < b { b = e }
                    }
                }
            }
            pcs.push((a/b - 1.0)*100.0);
        }
        let m = pcs.iter().fold(0.0f64, |x,y| x.max(y.abs()));
        if m > worst { worst = m }
        println!("{:<13} {:>9.2}% {:>9.2}% {:>9.2}% {:>9.2}%", id, pcs[0], pcs[1], pcs[2], m);
    }
    println!("\n  NOISE FLOOR of this harness: +-{worst:.2}%");
    println!("  Any Gate 6 reading inside that band measures nothing.");
}
