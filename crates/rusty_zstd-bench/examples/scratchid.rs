//! Identity for the finder-scratch wiring, A/B'd IN-PROCESS via the arm.
//!
//! The other session edits encode.rs live, so a build-to-build fingerprint
//! cannot attribute a difference. Toggling the arm inside one binary compares
//! old behaviour (`Vec::new()` per block) against new (frame scratch) with
//! everything else held identical.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = 4 << 20;
    let mut same = 0usize;
    let mut tot = 0usize;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        for lvl in [1, 3, 5, 7, 9, 11, 13, 15, 17, 19, 22] {
            rusty_zstd::set_finder_scratch_arm(false);
            let a = rusty_zstd::compress(s, lvl).unwrap();
            rusty_zstd::set_finder_scratch_arm(true);
            let b = rusty_zstd::compress(s, lvl).unwrap();
            tot += 1;
            if a == b {
                same += 1;
            } else {
                println!("  MISMATCH {id} L{lvl}: {} vs {} bytes", a.len(), b.len());
            }
            assert!(rusty_zstd::decompress(&b).unwrap() == s, "{id} L{lvl} round-trip");
        }
    }
    println!("BYTE-IDENTICAL across the arm: {same}/{tot} (corpus, level) cells");
}
