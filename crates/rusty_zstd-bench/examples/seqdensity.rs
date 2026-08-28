//! Is the decode gap PER-SEQUENCE or PER-BYTE? A deterministic count.
//!
//! The board says C decodes 1.71x faster at L3 and 1.49x faster at L19.
//! Two hypotheses explain a gap that SHRINKS as the level rises:
//!
//!   (a) per-BYTE cache-miss latency -- would GROW with the window, not shrink,
//!       and zstd only switches to its prefetching `_Long` decoder above a
//!       16 MiB window, which no level up to L19 reaches. Predicts no change.
//!   (b) per-SEQUENCE fixed overhead -- higher levels emit FEWER, LONGER
//!       matches, so a fixed per-sequence cost is amortised over more output
//!       bytes and the gap shrinks. Predicts the gap tracks sequences/MiB.
//!
//! This prices (b): sequences per MiB of OUTPUT, and mean bytes per sequence.
//! If sequences/MiB falls between L3 and L19 by roughly the same factor the
//! decode gap falls, the fixed per-sequence cost is the target and no amount
//! of prefetching or row-scanning touches it.
//!
//! Requires --features profile.
const IDS: &[&str] = &["dickens", "mozilla", "samba", "webster", "xml", "nci", "reymont", "osdb"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let levels: Vec<i32> = match std::env::args().nth(1) {
        Some(s) => s.split(',').map(|v| v.parse().unwrap()).collect(),
        None => vec![3, 5, 9, 19],
    };
    println!("SEQUENCE DENSITY vs LEVEL -- deterministic, no clock\n");
    println!("{:<7}{:>10}{:>16}{:>14}{:>14}{:>12}",
        "level", "out MiB", "match copies", "copies/MiB", "bytes/copy", "vs L3");
    let mut base = 0.0f64;
    for (i, lvl) in levels.iter().enumerate() {
        let (mut copies, mut cbytes, mut outb) = (0u64, 0u64, 0u64);
        for id in IDS {
            let Some(f) = load(id) else { continue };
            let src = &f[..f.len().min(16 << 20)];
            let z = rusty_zstd::compress(src, *lvl).unwrap();
            let (bc, bb) = rusty_zstd::take_dec_bands();
            let _ = rusty_zstd::take_dec_untiered();
            let out = rusty_zstd::decompress(&z).unwrap();
            assert_eq!(out.len(), src.len());
            let (bc2, bb2) = rusty_zstd::take_dec_bands();
            let u = rusty_zstd::take_dec_untiered();
            let _ = (bc, bb);
            let c: u64 = bc2.iter().sum::<u64>() + u[..8].iter().sum::<u64>();
            let b: u64 = bb2.iter().sum::<u64>() + u[8..].iter().sum::<u64>();
            copies += c;
            cbytes += b;
            outb += src.len() as u64;
        }
        let mib = outb as f64 / (1 << 20) as f64;
        let per = copies as f64 / mib;
        if i == 0 { base = per; }
        println!("L{:<6}{:>10.1}{:>16}{:>14.0}{:>14.1}{:>11.2}x",
            lvl, mib, copies, per,
            if copies > 0 { cbytes as f64 / copies as f64 } else { 0.0 },
            if base > 0.0 { per / base } else { 0.0 });
    }
    println!("\n'match copies' counts the tiered + untiered match-copy calls: one per");
    println!("sequence executed. copies/MiB IS the per-sequence overhead multiplier.");
}
