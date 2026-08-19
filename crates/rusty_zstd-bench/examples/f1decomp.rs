//! Decompose FINDING 1: is the win REACH (wider window) or TABLES (wider hash /
//! chain that the window clamp drags along with it)?
//!
//! The previous attempt was a null arm -- disabling the cParam clamp put the
//! table logs at the LEVEL's raw values, which are the same 22/24 the wide arm
//! already had, so both arms ran identical code. Isolating it needs explicit
//! CompressionParameters, not a switch.
//!
//!   narrow : window 20, tables 21/21   (Finding 1 OFF)
//!   reach  : window 23, tables 21/21   <- reach ONLY
//!   tables : window 20, tables 22/24   <- tables ONLY
//!   both   : window 23, tables 22/24   (Finding 1 ON, shipped)
use rusty_zstd::{CompressionParameters, Dictionary};
const IDS: &[&str] = &["x-ray","ooffice","mozilla","webster","samba","dickens","sao","nci","osdb","mr","xml","reymont","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let pn = rusty_zstd::compression_params(lvl, Some(PAY as u64)).unwrap();
    let pw = rusty_zstd::compression_params(lvl, Some((PAY+PRE) as u64)).unwrap();
    println!("FINDING 1 DECOMPOSED @ L{lvl}  narrow w{}/h{}/c{}  wide w{}/h{}/c{}",
        pn.window_log, pn.hash_log, pn.chain_log, pw.window_log, pw.hash_log, pw.chain_log);
    println!("{:<13} {:>11} {:>9} {:>9} {:>9}", "corpus", "narrow B", "reach%", "tables%", "both%");
    let (mut sr, mut st, mut sb) = (0.0f64, 0.0f64, 0.0f64);
    let mut n = 0.0f64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let d = Dictionary::raw(f[..PRE].to_vec());
        let tail = &f[PRE..PRE+PAY];
        let go = |p: CompressionParameters| -> usize {
            let z = rusty_zstd::compress_with_history(tail, p, false, Some(&d), &[], false).unwrap();
            assert!(rusty_zstd::decompress_using_dict(&z, &d).unwrap() == tail, "{id}: round-trip");
            z.len()
        };
        let narrow = go(pn);
        let reach  = go(CompressionParameters { window_log: pw.window_log, ..pn });
        let tables = go(CompressionParameters { hash_log: pw.hash_log, chain_log: pw.chain_log, ..pn });
        let both   = go(pw);
        let pc = |x: usize| (x as f64/narrow as f64 - 1.0)*100.0;
        sr += pc(reach); st += pc(tables); sb += pc(both); n += 1.0;
        println!("{:<13} {:>11} {:>8.3}% {:>8.3}% {:>8.3}%", id, narrow, pc(reach), pc(tables), pc(both));
    }
    println!("\n  mean   reach {:+.3}%   tables {:+.3}%   both {:+.3}%", sr/n, st/n, sb/n);
    println!("  if `tables` carries most of `both`, Finding 1's win is table sizing,");
    println!("  and the reachability contract is a SEPARATE (and much cheaper) question.");
}
