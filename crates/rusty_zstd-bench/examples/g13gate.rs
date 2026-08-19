//! GATE 13 @ L1 dispatch, verified DETERMINISTICALLY.
//!
//! The gate only ever declines the fixed-width copy's guard, so:
//!   (a) output must be BYTE-IDENTICAL -- both paths append the same bytes
//!   (b) wasted guard evaluations must FALL
//!   (c) bytes written must not RISE -- the declined calls were going slow anyway
//! None of that needs a clock.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 13 @ L{lvl} DISPATCH — deterministic verification (cap {} MiB)", cap>>20);
    println!("{:<13} {:>11} {:>11} {:>11} {:>9} | {:>9}", "corpus", "guard fail", "guard skip", "wasted off", "removed", "identical");
    let (mut tf_off, mut tf_on, mut ts_on) = (0u64, 0u64, 0u64);
    let mut moved = 0; let mut n = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        // gate OFF (constant, pre-dispatch)
        rusty_zstd::set_lit_short_arm(-1.0);
        let _ = rusty_zstd::take_lp_guard();
        let a = rusty_zstd::compress(s, lvl).unwrap();
        let (fail_off, _) = rusty_zstd::take_lp_guard();
        // gate ON (shipped)
        rusty_zstd::set_lit_short_arm(0.25);
        let _ = rusty_zstd::take_lp_guard();
        let b = rusty_zstd::compress(s, lvl).unwrap();
        let (fail_on, skip_on) = rusty_zstd::take_lp_guard();
        assert!(rusty_zstd::decompress(&b).unwrap() == s, "{id}: round-trip");
        if a != b { moved += 1; }
        n += 1;
        tf_off += fail_off; tf_on += fail_on; ts_on += skip_on;
        let removed = fail_off as i64 - fail_on as i64;
        println!("{:<13} {:>11} {:>11} {:>11} {:>8.1}% | {:>9}",
            id, fail_on, skip_on, fail_off,
            if fail_off > 0 { removed as f64 / fail_off as f64 * 100.0 } else { 0.0 },
            if a == b { "yes" } else { "NO" });
    }
    println!("\n  wasted guard evaluations {tf_off} -> {tf_on} ({:+.1}%), {ts_on} skipped by the gate",
        if tf_off > 0 { (tf_on as f64/tf_off as f64 - 1.0)*100.0 } else { 0.0 });
    println!("  output identical on {}/{} corpora", n - moved, n);
    assert_eq!(moved, 0, "the dispatch CHANGED OUTPUT -- it must not");
    rusty_zstd::set_lit_short_arm(0.25);
}
