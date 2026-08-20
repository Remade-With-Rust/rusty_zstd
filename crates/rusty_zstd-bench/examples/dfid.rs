//! DFast-pass identity fingerprint: total compressed bytes per level over the
//! 18-corpus board (6 MiB caps), printed as one line per level for exact
//! comparison across a code change that must be byte-identical.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    for lvl in [1u8, 2, 3, 4] {
        let mut tot = 0u64;
        let mut n = 0;
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl as i32, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} L{lvl} round-trip");
            tot += z.len() as u64;
            n += 1;
        }
        println!("L{lvl}: {n} corpora, total {tot}");
    }
}
