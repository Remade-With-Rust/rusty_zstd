//! Width decided on INSTRUCTIONS, the quantity the machine actually charges.
//!
//! From the emitted asm: the fast path is 7 instructions at width 8 AND 16 (one
//! movq / one movups), 9 at width 32 (two movups), and 138 at width 64 -- LLVM
//! stops inlining and emits a memcpy CALL with a loop.
//!
//! So bytes were never the cost: w8 and w16 are the same instruction count, and
//! the store-traffic model that preferred w8 by 33% was pricing a quantity that
//! does not exist. Widening 16 -> 32 costs 2 instructions on every fast call and
//! converts every 17-32 byte run from the slow path to the fast one.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao","x-ray"];
/// measured from the emitted asm
const I_FAST_16: f64 = 7.0;
const I_FAST_32: f64 = 9.0;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let i_slow: f64 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(47.0);
    println!("GATE 13 WIDTH by INSTRUCTION COUNT @ L{lvl}  (fast16={I_FAST_16}, fast32={I_FAST_32}, slow={i_slow})");
    println!("{:<13} {:>10} {:>9} {:>9} {:>12} {:>12} {:>10}", "corpus", "calls", "cov16%", "cov32%", "instr w16", "instr w32", "delta%");
    let (mut t16, mut t32) = (0.0f64, 0.0f64);
    let mut wins = 0; let mut n = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let (h, _, _) = rusty_zstd::take_lp_stats();
        let calls: u64 = h.iter().sum();
        if calls < 1000 { continue }
        // buckets: 0-4, 5-8, 9-16, 17-32, 33-64, 65+
        let c16 = (h[0] + h[1] + h[2]) as f64;          // <=16 reach the fast path at w16
        let c32 = c16 + h[3] as f64;                    // 17-32 join at w32
        let all = calls as f64;
        let cost16 = c16 * I_FAST_16 + (all - c16) * i_slow;
        let cost32 = c32 * I_FAST_32 + (all - c32) * i_slow;
        t16 += cost16; t32 += cost32;
        let d = (cost32 / cost16 - 1.0) * 100.0;
        if cost32 < cost16 { wins += 1 }
        n += 1;
        println!("{:<13} {:>10} {:>8.1}% {:>8.1}% {:>12.0} {:>12.0} {:>9.2}%",
            id, calls, c16/all*100.0, c32/all*100.0, cost16, cost32, d);
    }
    println!("\n  TOTAL instructions  w16 {:.0}  ->  w32 {:.0}   ({:+.2}%)", t16, t32, (t32/t16-1.0)*100.0);
    println!("  w32 cheaper on {wins}/{n} corpora");
    println!("\n  NOTE: w8 is identical to w16 in instruction count (7), so the 33% byte");
    println!("  saving the earlier model reported buys nothing. Width 64 is excluded:");
    println!("  it lowers to a memcpy CALL (138 instructions), a cliff not a slope.");
}
