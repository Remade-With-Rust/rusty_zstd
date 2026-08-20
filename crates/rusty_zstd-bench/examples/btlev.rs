//! The Bt ladder specifically -- L13..L18, which the 72-cell board does not cover.
const IDS: &[&str] = &["dickens","samba","xml","nci","webster","mozilla","x-ray","sao","reymont","osdb"];
fn h(b: &[u8]) -> u64 { let mut x=0xcbf29ce484222325u64; for &c in b { x^=c as u64; x=x.wrapping_mul(0x100000001b3);} x }
fn main() {
    let cap = 4 << 20;
    for id in IDS {
        let Ok(f)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s=&f[..f.len().min(cap)];
        for lvl in [5,6,7,8,9,10,11,12] {
            let z=rusty_zstd::compress(s,lvl).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap()==s,"{id} L{lvl} ROUND-TRIP");
            println!("{id:<10} L{lvl:<2} {:>9} {:016x}", z.len(), h(&z));
        }
    }
}
