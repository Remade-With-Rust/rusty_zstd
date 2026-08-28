//! JOINT FAST-PATH RATE -- prices the copy_match "megafuse" (one branch for
//! the lit16 x match16 case) before anyone builds it. Requires --features profile.
const IDS: &[&str] = &["dickens", "mozilla", "samba", "webster", "xml", "nci", "reymont", "osdb"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let (mut seqs, mut l16, mut m16) = (0u64, 0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(16 << 20)];
        let z = rusty_zstd::compress(src, lvl).unwrap();
        let _ = rusty_zstd::take_dec_copies();
        let (bc, _) = rusty_zstd::take_dec_bands();
        let _ = bc;
        let out = rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out.len(), src.len());
        let (_l32, _m32, lit16, match16) = rusty_zstd::take_dec_copies();
        let (bands, _) = rusty_zstd::take_dec_bands();
        let total: u64 = bands.iter().sum::<u64>();
        seqs += total;
        l16 += lit16;
        m16 += match16;
    }
    let lr = l16 as f64 / seqs as f64;
    let mr = m16 as f64 / seqs as f64;
    println!("L{lvl}: {seqs} sequences");
    println!("lit tier-1 rate:   {:.2}%", 100.0 * lr);
    println!("match tier-1 rate: {:.2}%", 100.0 * mr);
    println!(
        "joint fast-path bounds: [{:.2}%, {:.2}%]  (union bound low, min high)",
        100.0 * (lr + mr - 1.0).max(0.0),
        100.0 * lr.min(mr)
    );
}
