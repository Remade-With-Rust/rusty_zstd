//! Byte-identity fingerprint for the GATE 6 payload-buffer reuse.
//! Prints one line per (corpus, level): compressed size and a content hash.
//! Run on both builds and diff. Reuse must change NOTHING.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn h(b: &[u8]) -> u64 {
    let mut x = 0xcbf29ce484222325u64;
    for &c in b { x ^= c as u64; x = x.wrapping_mul(0x100000001b3); }
    x
}
fn main() {
    let cap: usize = 4 << 20;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        for lvl in [1, 3, 19, 22] {
            let z = rusty_zstd::compress(s, lvl).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} L{lvl} ROUND-TRIP FAILED");
            println!("{id:<13} L{lvl:<2} {:>10} {:016x}", z.len(), h(&z));
        }
    }
}
