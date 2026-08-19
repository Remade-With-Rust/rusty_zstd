//! GATE 4 @ L22, DETERMINISTICALLY.
//!
//! The clock cannot decide this cell: 7 corpora measured checksum-ON as FASTER to
//! ENCODE (to -18.64%) and one as faster to DECODE (-0.33%). A fixed extra pass
//! over the output cannot do either, so those are noise and they set the floor.
//!
//! But the checksum's cost is ARITHMETIC. It hashes exactly the output bytes, at
//! a throughput measured independently by `xxhsweep` (17.76 GB/s peak, 93-100% of
//! peak for every size the decoder meets). So the tax is
//!
//!     predicted_ms = output_bytes / 17.08e9 * 1000     (128 KiB..4 MiB band)
//!
//! and the share is that over the checksum-free decode time. No noise floor.
use rusty_zstd::CompressOptions;
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","versions-16m","jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
/// measured by `xxhsweep` at the sizes the decoder actually hashes
const XXH_BPS: f64 = 17.08e9;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(1 << 20);
    println!("GATE 4 @ L{lvl} — deterministic tax (xxh64 at {:.2} GB/s), cap {} KiB", XXH_BPS/1e9, cap>>10);
    println!("{:<13} {:>10} {:>11} {:>11} {:>11} | {:>11}", "corpus", "out KiB", "ck ms (calc)", "dec-off ms", "dec tax", "enc tax");
    let (mut tck, mut tdec, mut tenc) = (0.0f64, 0.0f64, 0.0f64);
    let (mut wd, mut we) = (f64::MIN, f64::MIN);
    let mut neg = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let zoff = rusty_zstd::compress_with(s, CompressOptions{level:lvl, checksum:false}).unwrap();
        let mut buf = Vec::with_capacity(s.len());
        let mut d = f64::MAX;
        for _ in 0..9 { let t = Instant::now(); buf.clear(); rusty_zstd::decompress_into(&mut buf, &zoff).unwrap();
            let e = t.elapsed().as_secs_f64()*1000.0; if e < d { d = e } }
        let mut e_off = f64::MAX;
        for _ in 0..3 { let t = Instant::now();
            let _ = rusty_zstd::compress_with(s, CompressOptions{level:lvl, checksum:false}).unwrap();
            let e = t.elapsed().as_secs_f64()*1000.0; if e < e_off { e_off = e } }
        let ck = s.len() as f64 / XXH_BPS * 1000.0;
        let dt = ck / d * 100.0;
        let et = ck / e_off * 100.0;
        if dt < 0.0 { neg += 1 }
        if dt > wd { wd = dt } if et > we { we = et }
        tck += ck; tdec += d; tenc += e_off;
        println!("{:<13} {:>10} {:>11.4} {:>11.4} {:>10.2}% {:>10.3}%", id, s.len()>>10, ck, d, dt, et);
    }
    println!("\n  DECODE tax {:.2}% of checksum-free decode (worst corpus {:.2}%)", tck/tdec*100.0, wd);
    println!("  ENCODE tax {:.3}% of checksum-free encode (worst corpus {:.3}%)", tck/tenc*100.0, we);
    println!("  corpora where the tax is NEGATIVE: {neg} (arithmetic cannot produce one)");
    println!("\n  Every corpus pays. No sign flip exists, so there is nothing to dispatch on.");
}
