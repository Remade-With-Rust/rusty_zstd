//! The Fast-ladder change must leave L3+ BYTE-IDENTICAL. Proven, not argued.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let mut n = 0; let mut moved = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        for cap in [2usize<<20, 8<<20] {
            if f.len() < cap { continue }
            let s = &f[..cap];
            for lvl in [3i32, 5, 9, 13] {
                // Fast arms at the shipped default vs deliberately extreme values:
                // neither may touch a non-Fast level.
                rusty_zstd::set_g5_fast_arms(2.0, 0.70, 2.0);
                let a = rusty_zstd::compress(s, lvl).unwrap();
                rusty_zstd::set_g5_fast_arms(-1.0, 0.01, 0.01);
                let b = rusty_zstd::compress(s, lvl).unwrap();
                rusty_zstd::set_g5_fast_arms(2.0, 0.70, 2.0);
                n += 1;
                if a != b { moved += 1; println!("  LEAK {id} L{lvl} cap {}", cap>>20); }
            }
        }
    }
    println!("{n} (corpus,size,level) cells at L3+: Fast arms moved output on {moved} (must be 0)");
    assert_eq!(moved, 0, "the Fast-ladder thresholds LEAKED into a non-Fast level");
}
