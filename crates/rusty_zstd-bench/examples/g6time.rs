//! Does GATE 6 @ L1's allocation win convert to WALL TIME?
//!
//! The per-frame clock has a +-24% noise floor (see g6null), so this uses many
//! paired rounds and reports the MEDIAN of per-round deltas plus sign stability
//! -- and runs a NULL arm (on vs on) at the identical repetition count so the
//! floor is measured, not assumed. A result inside the null band means nothing.
use std::time::Instant;

const IDS: &[&str] = &["dickens", "samba", "mr", "smallmsg-8m", "jsonlog-16m", "webster"];

fn best(s: &[u8], lvl: i32, arm: bool, k: usize) -> f64 {
    rusty_zstd::set_finder_scratch_arm(arm);
    let mut b = f64::MAX;
    for _ in 0..k {
        let t = Instant::now();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < b { b = e }
    }
    b
}

fn med(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    v[v.len() / 2]
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = 8 << 20;
    let rounds: usize = 9;
    let k: usize = 5;
    println!("GATE 6 @ L{lvl} -- does the allocation win show up on the clock?");
    println!("  {rounds} rounds, best-of-{k} each side, ABBA; NULL arm at the same cost\n");
    println!("{:<13} {:>10} {:>10} {:>9} {:>9} {:>8}", "corpus", "off ms", "on ms", "real%", "null%", "verdict");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = best(s, lvl, true, k);
        let (mut real, mut null) = (Vec::new(), Vec::new());
        let (mut soff, mut son) = (0.0f64, 0.0f64);
        for r in 0..rounds {
            // real: off vs on, order alternating
            let (a, b) = if r % 2 == 0 {
                let a = best(s, lvl, false, k); let b = best(s, lvl, true, k); (a, b)
            } else {
                let b = best(s, lvl, true, k); let a = best(s, lvl, false, k); (a, b)
            };
            soff += a; son += b;
            real.push((b / a - 1.0) * 100.0);
            // null: on vs on, same shape and cost
            let c = best(s, lvl, true, k);
            let d = best(s, lvl, true, k);
            null.push((d / c - 1.0) * 100.0);
        }
        let rm = med(&mut real);
        let nb = null.iter().fold(0.0f64, |x, y| x.max(y.abs()));
        let neg = real.iter().filter(|x| **x < 0.0).count();
        let verdict = if rm.abs() > nb && (neg == rounds || neg == 0) { "REAL" } else { "in noise" };
        println!("{:<13} {:>10.2} {:>10.2} {:>8.2}% {:>8.2}% {:>8}   ({neg}/{rounds} negative)",
            id, soff / rounds as f64, son / rounds as f64, rm, nb, verdict);
    }
    println!("\n  real%  = median of per-round (on/off - 1). Negative = keeping the buffers is FASTER.");
    println!("  null%  = worst |on vs on| at the same repetition count. The floor.");
}
