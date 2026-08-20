//! T4: the AVX2 sequence-loop twin -- correctness first, then what it buys.
//!
//! The twin is the WHOLE loop compiled a second time with AVX2 enabled and
//! dispatched once per block, which is the only shape that can work on a
//! portable build (a `target_feature` fn cannot inline into a caller without
//! the feature). Both twins must produce identical bytes.
use std::time::Instant;
const IDS: &[&str] = &["dickens","samba","xml","nci","webster","mozilla","x-ray","sao","mr","osdb","reymont","ooffice"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(15);
    let cap: usize = 8 << 20;
    let mut same = 0usize;
    let mut tot = 0usize;
    let (mut t_off, mut t_on) = (0.0f64, 0.0f64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        // correctness: both paths must reproduce the input exactly
        rusty_zstd::set_seqloop_avx2_arm(false);
        let a = rusty_zstd::decompress(&z).unwrap();
        rusty_zstd::set_seqloop_avx2_arm(true);
        let b = rusty_zstd::decompress(&z).unwrap();
        tot += 1;
        if a == *s && b == *s && a == b { same += 1 } else { println!("  {id}: MISMATCH") }
        // timing, ABBA
        let (mut bo, mut bn) = (f64::MAX, f64::MAX);
        for pass in 0..3 {
            for arm in [pass % 2 == 0, pass % 2 != 0] {
                rusty_zstd::set_seqloop_avx2_arm(arm);
                for _ in 0..n {
                    let t = Instant::now();
                    let d = rusty_zstd::decompress(&z).unwrap();
                    let e = t.elapsed().as_secs_f64() * 1000.0;
                    std::hint::black_box(&d);
                    if arm { if e < bn { bn = e } } else if e < bo { bo = e }
                }
            }
        }
        // NULL ARM at the identical protocol: avx2-on measured against ITSELF.
        let (mut na, mut nb) = (f64::MAX, f64::MAX);
        for pass in 0..3 {
            for first in [pass % 2 == 0, pass % 2 != 0] {
                rusty_zstd::set_seqloop_avx2_arm(true);
                for _ in 0..n {
                    let t = Instant::now();
                    let d = rusty_zstd::decompress(&z).unwrap();
                    let e = t.elapsed().as_secs_f64() * 1000.0;
                    std::hint::black_box(&d);
                    if first { if e < na { na = e } } else if e < nb { nb = e }
                }
            }
        }
        let null = (nb / na - 1.0) * 100.0;
        t_off += bo; t_on += bn;
        println!("{:<13} off {:>8.2}   avx2 {:>8.2}   real {:>+6.2}%   null {:>+6.2}%", id, bo, bn, (bn/bo - 1.0)*100.0, null);
    }
    println!("\n  IDENTICAL OUTPUT: {same}/{tot}  (REQUIRED)");
    println!("  totals: off {t_off:.1} ms -> avx2 {t_on:.1} ms  ({:+.2}%)", (t_on/t_off - 1.0)*100.0);
    println!("  NOTE: decode clock null floor measured at +-25%; treat this as confirmatory only.");
}
