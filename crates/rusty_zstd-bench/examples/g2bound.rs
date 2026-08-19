//! GATE 2 CONSTANT TEST, done through the ARM so both sides run in one process.
//!
//! The naive version of this compared `compress_using_prefix(tail, full_pre)`
//! against `compress_using_prefix(tail, short_pre)` -- but the shipped encoder
//! now truncates internally, so BOTH arms copied the same bytes and the A/B
//! measured nothing. Same null-arm trap as the L19/L22 MT cells.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const PREFIX: usize = 4 << 20;
const PAYLOAD: usize = 1 << 20;
const N: usize = 21;
fn best<F: FnMut() -> usize>(mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..N { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let p = rusty_zstd::compression_params(lvl, Some(PAYLOAD as u64)).unwrap();
    let window = 1usize << p.window_log.min(31);
    println!("GATE 2 @ L{lvl} CONSTANT TEST via set_prefix_bound_arm — ref {} MiB, payload {} MiB, window {} KiB",
        PREFIX>>20, PAYLOAD>>20, window>>10);
    println!("{:<13} {:>11} {:>11} {:>9}  {:>8}", "corpus", "full ms", "bound ms", "delta%", "identical");
    let (mut faster, mut slower, mut ident, mut n) = (0,0,0,0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PREFIX + PAYLOAD { continue }
        let pre = &full[..PREFIX];
        let tail = &full[PREFIX..PREFIX+PAYLOAD];
        rusty_zstd::set_prefix_bound_arm(false);
        let zf = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let tf = best(|| rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        rusty_zstd::set_prefix_bound_arm(true);
        let zb = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let tb = best(|| rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        assert!(rusty_zstd::decompress_using_prefix(&zb, pre).unwrap() == tail, "{id}: round-trip");
        let d = (tb/tf - 1.0)*100.0;
        if d < -1.0 { faster += 1 } else if d > 1.0 { slower += 1 }
        if zf == zb { ident += 1 }
        n += 1;
        println!("{:<13} {:>11.2} {:>11.2} {:>8.1}%  {:>8}", id, tf, tb, d, if zf==zb {"yes"} else {"NO"});
    }
    println!("\n  byte-identical {ident}/{n} | faster {faster} | slower {slower}");
    assert_eq!(ident, n, "the shipped bound is NOT byte-identical to the full copy");
}
