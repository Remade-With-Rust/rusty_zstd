//! L1/L2 byte-identity board: per-corpus compressed sizes, 8 MiB caps.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    for lvl in [1i32, 2] {
        let mut total = 0usize;
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(8 << 20)];
            let z = rusty_zstd::compress(s, lvl).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), s, "{id} roundtrip");
            println!("L{lvl} {id} {}", z.len());
            total += z.len();
        }
        println!("L{lvl} TOTAL {total}");
    }
}
