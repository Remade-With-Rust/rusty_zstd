//! Three-probe refutation on Gate 6's timing. The first pass read dickens
//! -23.39%, samba +17.01%, incomp +20.00% -- but the realloc model predicts
//! ~0.3 ms of extra copying where dickens showed 39 ms, a 100x mismatch. Either
//! the model is wrong or the measurement is.
//!
//! Run the same A/B three times, interleaved ABBA, and watch the SIGN.
use std::time::Instant;
const IDS: &[&str] = &["dickens","samba","mozilla","webster","xml","incomp-32m","jsonlog-16m","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 6 three-probe @ L{lvl} -- sign stability of the reserve");
    println!("{:<13} {:>10} {:>10} {:>10} {:>10}", "corpus", "run 1", "run 2", "run 3", "verdict");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut pcs = Vec::new();
        for _ in 0..3 {
            // ABBA within each run so monotone drift cancels
            let (mut on, mut off) = (f64::MAX, f64::MAX);
            for phase in 0..2 {
                for arm in [phase == 0, phase != 0] {
                    rusty_zstd::set_payload_arm(arm);
                    for _ in 0..7 {
                        let t = Instant::now();
                        let _ = rusty_zstd::compress(s, lvl).unwrap();
                        let e = t.elapsed().as_secs_f64()*1000.0;
                        if arm { if e < on { on = e } } else if e < off { off = e }
                    }
                }
            }
            pcs.push((on/off - 1.0)*100.0);
        }
        rusty_zstd::set_payload_arm(true);
        let same_sign = pcs.iter().all(|p| *p > 0.0) || pcs.iter().all(|p| *p < 0.0);
        println!("{:<13} {:>9.2}% {:>9.2}% {:>9.2}% {:>10}", id, pcs[0], pcs[1], pcs[2],
            if same_sign { "stable" } else { "SIGN FLIPS" });
    }
}
