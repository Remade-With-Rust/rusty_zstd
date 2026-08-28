//! MATCH-COPY BAND CENSUS -- where do the copies actually land, post-cuts?
//!
//! COPYMATCH CUT 8's decision instrument: an 8-byte rung (`len <= 8 &&
//! offset >= 8`) only pays if a material share of copies falls past tier 1
//! into the cold fn with `offset < 16`. Bands (see `note_band`):
//!   0 = splat (offset 1)         3 = extend_from_within, non-overlap
//!   1 = 32-tier, len > 16        4 = overlap loop
//!   2 = 16-tier (tier 1)         5 = 32-tier, len <= 16 (off in 16..32)
//!   6 = 64-tier
//! Requires --features profile.
const IDS: &[&str] = &["dickens", "mozilla", "samba", "webster", "xml", "nci", "reymont", "osdb"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let mut calls = [0u64; 8];
    let mut bytes = [0u64; 8];
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(16 << 20)];
        let z = rusty_zstd::compress(src, lvl).unwrap();
        let _ = rusty_zstd::take_dec_bands();
        let out = rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out.len(), src.len());
        let (c, b) = rusty_zstd::take_dec_bands();
        for i in 0..8 {
            calls[i] += c[i];
            bytes[i] += b[i];
        }
    }
    let tc: u64 = calls.iter().sum();
    let tb: u64 = bytes.iter().sum();
    const NAMES: [&str; 8] = [
        "0 splat(off=1)", "1 t32 len>16", "2 t16 (hot)", "3 within", "4 overlap",
        "5 t32 len<=16", "6 t64", "7 --",
    ];
    println!("L{lvl}: {tc} match copies, {tb} bytes\n");
    println!("{:<16}{:>14}{:>9}{:>16}{:>9}", "band", "calls", "calls%", "bytes", "bytes%");
    for i in 0..8 {
        if calls[i] == 0 { continue; }
        println!(
            "{:<16}{:>14}{:>8.2}%{:>16}{:>8.2}%",
            NAMES[i], calls[i],
            100.0 * calls[i] as f64 / tc as f64,
            bytes[i],
            100.0 * bytes[i] as f64 / tb as f64
        );
    }
    println!("\ncold share (everything but band 2): {:.2}% of calls",
        100.0 * (tc - calls[2]) as f64 / tc as f64);
}
