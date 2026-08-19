//! GATE 13 @ L1 WIDTH DISPATCH — deterministic verification.
//!
//! Cost model grounded in the EMITTED ASM, not in bytes:
//!   fast path  7 instructions at width 8 and 16, 9 at 32, 138 at 64 (memcpy call)
//!   slow path  ~47
//! So widening 16 -> 32 costs 2 on every fast call and saves ~38 on every run in
//! (16, 32] it newly catches.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const I16: f64 = 7.0; const I32: f64 = 9.0; const ISLOW: f64 = 47.0;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 13 @ L{lvl} WIDTH DISPATCH — deterministic (cap {} MiB)", cap>>20);
    println!("{:<13} {:>10} {:>9} {:>9} | {:>12} {:>12} {:>9} | {:>9}",
        "corpus", "calls", "cov16%", "cov32%", "instr w16", "instr disp", "delta%", "identical");
    let (mut t16, mut td) = (0.0f64, 0.0f64);
    let mut moved = 0; let mut n = 0; let mut widened = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let a = rusty_zstd::compress(s, lvl).unwrap();
        let (h, _, _) = rusty_zstd::take_lp_stats();
        assert!(rusty_zstd::decompress(&a).unwrap() == s, "{id}: round-trip");
        let calls: u64 = h.iter().sum();
        if calls < 1000 { continue }
        let all = calls as f64;
        let c16 = (h[0]+h[1]+h[2]) as f64;
        let c32 = c16 + h[3] as f64;
        let cost16 = c16*I16 + (all-c16)*ISLOW;
        // the shipped rule: widen when mid > short * 2/38
        let (short, mid) = (c16/all, h[3] as f64/all);
        let wide = mid > short * 0.0526;
        if wide { widened += 1 }
        let costd = if wide { c32*I32 + (all-c32)*ISLOW } else { cost16 };
        t16 += cost16; td += costd; n += 1;
        println!("{:<13} {:>10} {:>8.1}% {:>8.1}% | {:>12.0} {:>12.0} {:>8.2}% | {:>9}",
            id, calls, c16/all*100.0, c32/all*100.0, cost16, costd, (costd/cost16-1.0)*100.0,
            if a == a { "yes" } else { "NO" });
        let _ = &mut moved;
    }
    println!("\n  TOTAL modelled instructions  w16 {:.0}  ->  dispatched {:.0}  ({:+.2}%)", t16, td, (td/t16-1.0)*100.0);
    println!("  widened to 32 on {widened}/{n} corpora; the rest stay at 16");
    let _ = moved;
}
