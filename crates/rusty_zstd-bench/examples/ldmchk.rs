//! LDM is `enable_ldm: false` by default, so the 72-cell board never runs it.
//! Exercise it explicitly: round-trip and byte-identity across the change.
fn main() {
    let mut ok = 0;
    let mut n = 0;
    for id in ["dickens", "samba", "webster", "nci", "versions-16m", "text-32m"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(8 << 20)];
        for lvl in [3, 9, 19] {
            let mut p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
            p.enable_ldm = true;
            let z = rusty_zstd::compress_with_params(s, p, true).unwrap();
            let d = rusty_zstd::decompress(&z).unwrap();
            n += 1;
            if d == s { ok += 1 } else { println!("  {id} L{lvl}: ROUND-TRIP FAILED") }
            println!("{id:<13} L{lvl:<3} ldm-on {:>10} bytes", z.len());
        }
    }
    println!("\nLDM round-trip: {ok}/{n}");
}
