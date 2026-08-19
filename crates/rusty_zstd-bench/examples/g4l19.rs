//! GATE 4 @ L19 — `opts.checksum`.
//!
//! Step 2 asks for WIN AND LOSS across content: a sign flip. The checksum's sign
//! cannot flip on the encoder -- omitting an xxh64 pass is never slower -- so the
//! question is whether the MAGNITUDE varies enough to matter, and where the cost
//! actually lands.
//!
//! At L1 encode is fast and the checksum was priced at +23% mean. At L19 encode
//! is ~1000x slower per byte, so the same absolute work is a rounding error --
//! but the trailer the ENCODER writes forces the DECODER to verify, and decode at
//! L19 is fast. The tax may simply have moved sides.
use rusty_zstd::CompressOptions;
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(256 << 10);
    let n = if lvl >= 13 { 3 } else { 9 };
    println!("GATE 4 @ L{lvl} — checksum on vs off (cap {} KiB, best-of-{n})", cap>>10);
    println!("{:<13} {:>9} {:>9} {:>9} | {:>9} {:>9} {:>9} | {:>7}",
        "corpus", "enc on", "enc off", "enc tax", "dec on", "dec off", "dec tax", "bytes");
    let (mut te_on, mut te_off, mut td_on, mut td_off) = (0.0f64,0.0f64,0.0f64,0.0f64);
    let (mut worst_e, mut worst_d) = (f64::MIN, f64::MIN);
    let mut neg = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mk = |ck: bool| rusty_zstd::compress_with(s, CompressOptions{level:lvl, checksum:ck}).unwrap();
        let (zon, zoff) = (mk(true), mk(false));
        let e_on = best(n, || mk(true).len());
        let e_off = best(n, || mk(false).len());
        let mut buf = Vec::with_capacity(s.len());
        let d_on = best(n.max(5), || { buf.clear(); rusty_zstd::decompress_into(&mut buf, &zon).unwrap() });
        let d_off = best(n.max(5), || { buf.clear(); rusty_zstd::decompress_into(&mut buf, &zoff).unwrap() });
        assert!(rusty_zstd::decompress(&zon).unwrap() == s, "{id}: round-trip");
        let et = (e_on/e_off - 1.0)*100.0;
        let dt = (d_on/d_off - 1.0)*100.0;
        if et < -1.0 { neg += 1 }
        if et > worst_e { worst_e = et } if dt > worst_d { worst_d = dt }
        te_on += e_on; te_off += e_off; td_on += d_on; td_off += d_off;
        println!("{:<13} {:>9.2} {:>9.2} {:>8.2}% | {:>9.3} {:>9.3} {:>8.2}% | {:>7}",
            id, e_on, e_off, et, d_on, d_off, dt, zon.len() as i64 - zoff.len() as i64);
    }
    println!("\n  ENCODE tax {:+.2}% (worst corpus {:+.2}%)", (te_on/te_off-1.0)*100.0, worst_e);
    println!("  DECODE tax {:+.2}% (worst corpus {:+.2}%)", (td_on/td_off-1.0)*100.0, worst_d);
    println!("  corpora where checksum-ON was FASTER to encode (sign flip): {neg}");
}
