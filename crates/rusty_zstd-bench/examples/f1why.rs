//! Why does widening the window help corpora that never match the far region?
//!
//! HYPOTHESIS: it is not reach at all. The cParam clamp shipped earlier ties
//! hash_log and chain_log to window_log (ZSTD_adjustCParams). So widening the
//! window ALSO un-clamps the tables -- bigger hash, bigger tree -- and part of
//! Finding 1's benefit is a TABLE-SIZE effect wearing a reach costume.
//!
//! Test: run Finding 1 with the cParam clamp DISABLED, so table logs stay at
//! their level values and only REACH changes. If the benefit collapses, the
//! benefit was tables.
use rusty_zstd::Dictionary;
const IDS: &[&str] = &["x-ray","ooffice","mozilla","webster","samba","dickens","sao","nci","osdb","mr","xml","reymont","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("FINDING 1: reach or tables? @ L{lvl}");
    println!("{:<13} {:>9} {:>9} | {:>11} {:>11} {:>9} | {:>11} {:>9}",
        "corpus", "h/c narrow", "h/c wide", "narrow B", "wide B", "benefit%", "wide-noclamp", "reach-only%");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let d = Dictionary::raw(f[..PRE].to_vec());
        let tail = &f[PRE..PRE+PAY];
        // params as each arm sees them
        rusty_zstd::set_cparam_clamp_arm(true);
        let pn = rusty_zstd::compression_params(lvl, Some(PAY as u64)).unwrap();
        let pw = rusty_zstd::compression_params(lvl, Some((PAY+PRE) as u64)).unwrap();
        // A: narrow window (Finding 1 off)
        rusty_zstd::set_prefix_window_arm(false);
        let a = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap().len();
        // B: wide window, clamp ON (shipped): tables follow the window
        rusty_zstd::set_prefix_window_arm(true);
        let b = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap().len();
        // C: wide window, clamp OFF: tables stay at level values -> REACH ONLY
        rusty_zstd::set_cparam_clamp_arm(false);
        let c = rusty_zstd::compress_using_dict(tail, &d, lvl).unwrap().len();
        rusty_zstd::set_cparam_clamp_arm(true);
        assert!(rusty_zstd::decompress_using_dict(&rusty_zstd::compress_using_dict(tail,&d,lvl).unwrap(), &d).unwrap() == tail);
        println!("{:<13} {:>6}/{:<2} {:>6}/{:<2} | {:>11} {:>11} {:>8.3}% | {:>11} {:>8.3}%",
            id, pn.hash_log, pn.chain_log, pw.hash_log, pw.chain_log, a, b,
            (b as f64/a as f64-1.0)*100.0, c, (c as f64/a as f64-1.0)*100.0);
    }
    println!("\n  benefit%     = shipped Finding 1 (wide window AND wider tables)");
    println!("  reach-only%  = wide window, tables pinned at level values");
    println!("  if reach-only is ~0 while benefit is negative, the win was TABLES, not reach.");
    rusty_zstd::set_prefix_window_arm(true);
}
