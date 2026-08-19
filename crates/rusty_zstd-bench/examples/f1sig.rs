//! Is FINDING 1 a CONSTANT or a DISPATCH?
//!
//! Shipped as a constant: always widen the window to cover the dictionary. But
//! the per-corpus truth table says the value is not uniform -- webster gains
//! -2.26% for +0.6% time while sao gets 0.17% LARGER for +5.8%, and nci/mr gain
//! nothing. That is a dispatch shape.
//!
//! CANDIDATE SIGNAL, computable before any encoding and O(sample): how much of
//! the payload can be found in the FAR region of the dictionary -- the part only
//! the widened window can reach. If the payload never matches out there, widening
//! buys nothing and only costs.
//!
//! This is a TRUTH TABLE, not a gate. Fit nothing here; just ask whether the
//! signal separates the winners from the losers.
use std::collections::HashSet;

const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;
/// The narrow window Finding 1 replaces (payload-sized, L19 gives wlog 20).
const NARROW: usize = 1 << 20;

/// Measured size delta of widening, from `f1solo` (tree OFF), L19, in percent.
/// NEGATIVE = widening helped.
const TRUTH: &[(&str, f64)] = &[
    ("mozilla", -0.405), ("webster", -2.258), ("nci", 0.014), ("samba", -0.142),
    ("osdb", 0.035), ("dickens", -0.108), ("mr", 0.003), ("xml", -0.054),
    ("reymont", -0.102), ("sao", 0.166), ("ooffice", -0.805), ("x-ray", -1.372),
    ("jsonlog-16m", 0.121), ("smallmsg-8m", 0.065), ("versions-16m", 0.000),
];

fn gram(b: &[u8], i: usize) -> u64 {
    u64::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3],b[i+4],b[i+5],b[i+6],b[i+7]])
}

fn main() {
    // sampling stride: O(sample), not O(dictionary)
    let dstride: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    let pstride: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(16);
    println!("FINDING 1 SIGNAL TRUTH TABLE — far-region hit rate vs measured benefit");
    println!("  dictionary stride {dstride}, payload stride {pstride} (both O(sample))");
    println!("{:<13} {:>12} {:>12} {:>11} {:>10}", "corpus", "far anchors", "payload hits", "hit rate", "benefit%");
    let mut rows = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let pre = &f[..PRE];
        let tail = &f[PRE..PRE+PAY];
        // FAR region = the part of the dictionary only the WIDE window reaches
        let far = &pre[..PRE.saturating_sub(NARROW)];
        let mut set: HashSet<u64> = HashSet::new();
        let mut i = 0;
        while i + 8 <= far.len() { set.insert(gram(far, i)); i += dstride; }
        let (mut hits, mut probes) = (0u64, 0u64);
        let mut j = 0;
        while j + 8 <= tail.len() {
            probes += 1;
            if set.contains(&gram(tail, j)) { hits += 1; }
            j += pstride;
        }
        let rate = hits as f64 / probes.max(1) as f64;
        let truth = TRUTH.iter().find(|(k,_)| k == id).map(|(_,v)| *v).unwrap_or(0.0);
        println!("{:<13} {:>12} {:>12} {:>10.4}% {:>9.3}%", id, set.len(), hits, rate*100.0, truth);
        rows.push((*id, rate, truth));
    }
    // correlation between hit rate and benefit (benefit is NEGATIVE when good)
    let n = rows.len() as f64;
    let (mx, my) = (rows.iter().map(|r| r.1).sum::<f64>()/n, rows.iter().map(|r| r.2).sum::<f64>()/n);
    let (mut sxy, mut sxx, mut syy) = (0.0,0.0,0.0);
    for r in &rows { let a = r.1-mx; let b = r.2-my; sxy += a*b; sxx += a*a; syy += b*b; }
    println!("\n  correlation(hit rate, benefit) r = {:.3}   (want STRONGLY NEGATIVE)", sxy/(sxx.sqrt()*syy.sqrt()));
    // does a threshold separate winners from non-winners?
    let mut sorted = rows.clone();
    sorted.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap());
    println!("\n  ranked by hit rate (highest first):");
    for (id, rate, truth) in &sorted {
        println!("    {:<13} hit {:>8.4}%   benefit {:>7.3}%  {}", id, rate*100.0, truth,
            if *truth < -0.1 { "WIN" } else if *truth > 0.05 { "loses size" } else { "neutral" });
    }
}
