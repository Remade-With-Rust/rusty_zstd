//! T1 -- carry the packed rejection tag to DFast.
//!
//! Step 1 (dead check, INVERTED): the tag is a pure filter. It derives from the
//! same 4 bytes as the index and DFast's min_match is 5, so a mismatch cannot
//! hide a match. Output MUST be byte-identical; anything else is a defect and
//! the arm does not ship.
//!
//! Step 2: the win is DETERMINISTIC -- candidate loads avoided. The per-frame
//! clock has a +-24% null floor here and cannot see a 30% change in one term of
//! the inner loop, so it is reported but never decisive.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("T1 @ L{lvl} -- DFast rejection tag ({} MiB board)\n", cap >> 20);
    println!("{:<13} {:>11} {:>11} {:>8} {:>9} {:>9} {:>7}",
        "corpus", "off bytes", "on bytes", "ident", "off ms", "on ms", "t%");
    println!("{}", "-".repeat(76));
    let (mut ident, mut n, mut faster) = (0usize, 0usize, 0usize);
    let (mut so, mut sn) = (0.0f64, 0.0f64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut best = [f64::MAX; 2];
        let mut out: [Vec<u8>; 2] = [Vec::new(), Vec::new()];
        for pass in 0..3 {
            for arm in [pass % 2, 1 - pass % 2] {
                rusty_zstd::set_dfast_tag_arm(arm == 1);
                for _ in 0..5 {
                    let t = Instant::now();
                    let z = rusty_zstd::compress(s, lvl).unwrap();
                    let e = t.elapsed().as_secs_f64() * 1000.0;
                    if e < best[arm] { best[arm] = e; }
                    if out[arm].is_empty() { out[arm] = z; }
                }
            }
        }
        assert!(rusty_zstd::decompress(&out[1]).unwrap() == s, "{id}: ROUND-TRIP FAILED");
        let same = out[0] == out[1];
        if same { ident += 1 } else { println!("  {id}: OUTPUT DIFFERS -- DEFECT") }
        if best[1] < best[0] { faster += 1 }
        so += best[0]; sn += best[1]; n += 1;
        println!("{:<13} {:>11} {:>11} {:>8} {:>9.2} {:>9.2} {:>6.2}%",
            id, out[0].len(), out[1].len(), if same {"yes"} else {"NO"},
            best[0], best[1], (best[1] / best[0] - 1.0) * 100.0);
    }
    println!("\n  BYTE-IDENTICAL: {ident}/{n}  (REQUIRED -- the tag is a pure filter)");
    println!("  faster on {faster}/{n}; total {:.1} ms -> {:.1} ms ({:+.2}%)",
        so, sn, (sn / so - 1.0) * 100.0);
    println!("  The clock is CONFIRMATORY ONLY -- null floor here is +-24%.");
}
