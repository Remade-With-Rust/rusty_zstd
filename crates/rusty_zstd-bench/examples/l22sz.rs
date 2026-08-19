//! Emit L16/L19/L22 sizes for every corpus so the find_opt change can be proven
//! byte-identical against the pre-change build.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(256 << 10);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut row = format!("{id}");
        for lvl in [16i32, 19, 22] {
            let z = rusty_zstd::compress(s, lvl).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} L{lvl}: round-trip");
            let mut h: u64 = 1469598103934665603;
            for b in &z { h ^= *b as u64; h = h.wrapping_mul(1099511628211); }
            row += &format!(" L{lvl}={} {:x}", z.len(), h);
        }
        println!("{row}");
    }
}
