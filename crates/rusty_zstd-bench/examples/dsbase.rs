//! Baseline: DecodeSeq stage ns BEFORE any inner guards are added.
//! This exists to price the instrument I am about to build.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["webster","nci","xml","samba","dickens","mr","osdb","jsonlog-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = 8 << 20;
    println!("{:<14}{:>14}{:>14}{:>10}", "corpus", "DecSeq ns", "DecTotal ns", "% dec");
    let (mut ds, mut dt) = (0u64, 0u64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        rusty_zstd::prof_reset();
        let out = rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out.len(), s.len());
        let a = rusty_zstd::prof_stage_ns(S::DecodeSeq);
        let b = rusty_zstd::prof_stage_ns(S::DecodeTotal);
        ds += a; dt += b;
        println!("{id:<14}{a:>14}{b:>14}{:>9.1}%", 100.0 * a as f64 / b as f64);
    }
    println!("\nTOTAL DecSeq {ds} / DecTotal {dt} = {:.2}%", 100.0 * ds as f64 / dt as f64);
}
