//! DECODE-only timing. Compression is done once, outside the timed region, so
//! the encoder (which the other session is editing) cannot contaminate this.
//!
//! Prints one line per corpus: best-of-N decode milliseconds. Two builds of this
//! binary are run alternately by the driver so build-to-build placement noise is
//! interleaved rather than confounded with the change.
use std::time::Instant;
const IDS: &[&str] = &["dickens","samba","xml","nci","webster","mozilla","x-ray","sao","mr","osdb","reymont","ooffice"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(25);
    let cap: usize = 8 << 20;
    let mut total = 0.0f64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        let mut best = f64::MAX;
        for _ in 0..n {
            let t = Instant::now();
            let d = rusty_zstd::decompress(&z).unwrap();
            let e = t.elapsed().as_secs_f64() * 1000.0;
            std::hint::black_box(&d);
            if e < best { best = e }
        }
        total += best;
        println!("{id} {best:.3}");
    }
    println!("TOTAL {total:.3}");
}
