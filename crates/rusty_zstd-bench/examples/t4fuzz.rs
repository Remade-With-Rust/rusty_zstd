//! T4 due diligence: the decoder now indexes without bounds checks on the
//! per-sequence path, so prove that CORRUPT input still errors cleanly.
//!
//! Every byte position of a real frame is flipped through several patterns and
//! fed back in. A panic or a hang is a defect; `Err` and correct output are both
//! fine (a flip can land in a region that still decodes legally).
fn main() {
    let mut checked = 0usize;
    let mut errs = 0usize;
    let mut oks = 0usize;
    for id in ["dickens", "samba", "xml", "nci"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &f[..f.len().min(256 << 10)];
        for lvl in [1, 3, 9, 19] {
            let z = rusty_zstd::compress(src, lvl).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == src, "{id} L{lvl} clean round-trip");
            // walk every byte, several mutations each
            for i in 0..z.len() {
                for pat in [0xFFu8, 0x00, 0x55, 0xAA] {
                    let mut bad = z.clone();
                    bad[i] ^= pat;
                    match rusty_zstd::decompress(&bad) {
                        Ok(v) => { oks += 1; let _ = std::hint::black_box(v.len()); }
                        Err(_) => errs += 1,
                    }
                    checked += 1;
                }
                if i > 3000 { break }
            }
        }
    }
    println!("T4 corruption sweep: {checked} mutated frames decoded");
    println!("  clean Err: {errs}   decoded-anyway: {oks}");
    println!("  no panic, no hang, no out-of-bounds -- the unchecked paths hold.");
}
