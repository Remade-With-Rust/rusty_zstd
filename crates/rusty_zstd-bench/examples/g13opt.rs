//! GATE 13 @ L1, take 2: the variables were wrong.
//!
//! Take 1 dispatched on WIDTH and judged it with a clock the box cannot resolve.
//! But the corpora fail the guard for TWO DIFFERENT REASONS and I conflated them:
//!
//!   sao / x-ray   94% of runs are LONGER than the width -> the guard fails, and
//!                 a WIDER width would capture them (or none should be tried)
//!   smallmsg      95.5% of runs are 5-8 -> a NARROWER width serves everything
//!                 at half the store traffic
//!
//! So the dispatched quantity is not a width picked by a stopwatch; it is a
//! PERCENTILE of the run-length distribution -- the population-relative form
//! great-gate law 1.1 demands. Width = smallest power of two covering the P-th
//! percentile of the previous block's runs.
//!
//! Decided deterministically: total cost = bytes written + F x slow calls, swept
//! over F so the answer's dependence on the one unknown is visible rather than
//! assumed.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao"];
const HI: [usize; 6] = [4, 8, 16, 32, 64, 128];
const MEAN: [f64; 6] = [2.5, 6.5, 12.5, 24.5, 48.0, 160.0];
const WIDTHS: [usize; 5] = [8, 16, 32, 64, 128];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    // harvest the histogram once per corpus
    let mut hs = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let (h, _, _) = rusty_zstd::take_lp_stats();
        if h.iter().sum::<u64>() < 1000 { continue }
        hs.push((*id, h));
    }
    println!("GATE 13 @ L{lvl}: optimal width at the MEASURED slow-path cost F ~ 9-23 bytes-equiv");
    println!("  (F from microbenchmark: 0.1231 ns/byte marginal, 1.07-2.85 ns/call fixed)");
    print!("{:<13} {:>7}", "corpus", "P90");
    for f in [10usize, 12, 16, 20] { print!("  F={f:<3}"); }
    println!("   verdict");
    let mut flips = 0;
    for (id, h) in &hs {
        let calls: f64 = h.iter().map(|x| *x as f64).sum();
        // P90 of the run length, from the buckets
        let mut acc = 0.0; let mut p90 = 128;
        for i in 0..6 { acc += h[i] as f64; if acc/calls >= 0.90 { p90 = HI[i]; break } }
        print!("{:<13} {:>7}", id, p90);
        let mut chosen = Vec::new();
        for f in [10.0f64, 12.0, 16.0, 20.0] {
            let cost = |w: usize| -> f64 {
                let mut c = 0.0;
                for i in 0..6 {
                    if HI[i] <= w { c += h[i] as f64 * w as f64 }
                    else { c += h[i] as f64 * (MEAN[i] + f) }
                }
                c
            };
            let best = WIDTHS.iter().copied().min_by(|a,b| cost(*a).partial_cmp(&cost(*b)).unwrap()).unwrap();
            chosen.push(best);
            print!("  {best:<5}");
        }
        let uniq: std::collections::BTreeSet<usize> = chosen.iter().copied().collect();
        if uniq.len() > 1 { flips += 1; }
        println!("   {}", if uniq.len() > 1 { "F-sensitive" } else { "stable" });
    }
    // does the optimum DIFFER ACROSS CORPORA at a fixed F? that is the dispatch test
    println!("\n  DISPATCH TEST -- optimal width across corpora at each fixed F:");
    for f in [10.0f64, 12.0, 16.0, 20.0] {
        let mut set = std::collections::BTreeMap::new();
        for (id, h) in &hs {
            let cost = |w: usize| -> f64 {
                let mut c = 0.0;
                for i in 0..6 {
                    if HI[i] <= w { c += h[i] as f64 * w as f64 } else { c += h[i] as f64 * (MEAN[i] + f) }
                }
                c
            };
            let best = WIDTHS.iter().copied().min_by(|a,b| cost(*a).partial_cmp(&cost(*b)).unwrap()).unwrap();
            set.entry(best).or_insert_with(Vec::new).push(*id);
        }
        let widths: Vec<String> = set.iter().map(|(w,v)| format!("{w}:{}", v.len())).collect();
        println!("    F={:<3} -> {} distinct width(s)  [{}]  {}", f as usize, set.len(), widths.join(" "),
            if set.len() > 1 { "DISPATCH" } else { "constant" });
    }
    println!("\n  {flips} corpora whose optimum moves with F (i.e. depend on the unknown)");
}
