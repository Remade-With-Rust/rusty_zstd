//! GATE 6 @ L3 -- `payload_reserve_enabled()`: pre-reserve the block payload Vec.
//!
//! This is an ALLOCATION arm, so Step 1 inverts: the output MUST NOT differ. If
//! it does, that is a defect, not a dispatch. What can differ is the work.
//!
//! Deterministic side: with no reserve the payload grows by doubling, so it
//! re-copies ~2x its final size per block. With `Vec::with_capacity(block.len())`
//! it copies 0 -- but reserves 128 KiB to hold `block.len() * ratio` bytes, which
//! on high-ratio content is a 12x over-reservation.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let n = if lvl >= 13 { 3 } else { 9 };
    println!("GATE 6 @ L{lvl} -- payload reserve (cap {} MiB, best-of-{n})", cap>>20);
    println!("{:<13} {:>9} {:>9} {:>8} | {:>10} {:>11} {:>9}", "corpus", "on ms", "off ms", "time%", "ratio", "reserved/used", "identical");
    let (mut ton, mut toff) = (0.0f64, 0.0f64);
    let (mut moved, mut cells) = (0, 0);
    let mut worst_over = 0.0f64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::set_payload_arm(true);
        let a = rusty_zstd::compress(s, lvl).unwrap();
        let t_on = best(n, || rusty_zstd::compress(s, lvl).unwrap().len());
        rusty_zstd::set_payload_arm(false);
        let b = rusty_zstd::compress(s, lvl).unwrap();
        let t_off = best(n, || rusty_zstd::compress(s, lvl).unwrap().len());
        rusty_zstd::set_payload_arm(true);
        assert!(rusty_zstd::decompress(&a).unwrap() == s, "{id}: round-trip");
        if a != b { moved += 1 }
        cells += 1;
        let ratio = a.len() as f64 / s.len() as f64;
        // reserve is block.len(); the payload actually written is block.len()*ratio
        let over = 1.0 / ratio.max(1e-9);
        if over > worst_over { worst_over = over }
        ton += t_on; toff += t_off;
        println!("{:<13} {:>9.2} {:>9.2} {:>7.2}% | {:>10.4} {:>10.1}x {:>9}",
            id, t_on, t_off, (t_on/t_off-1.0)*100.0, ratio, over, if a == b { "yes" } else { "NO" });
    }
    println!("\n  output identical on {}/{} corpora (MUST be all)", cells-moved, cells);
    println!("  TIME reserve-on {:.0} ms vs off {:.0} ms ({:+.2}%)", ton, toff, (ton/toff-1.0)*100.0);
    println!("  worst over-reservation {:.1}x", worst_over);
    assert_eq!(moved, 0, "an allocation arm CHANGED THE OUTPUT -- that is a defect");
}
