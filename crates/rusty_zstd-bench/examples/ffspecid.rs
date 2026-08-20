//! Identity for the live-arm specialisation, A/B'd in-process via
//! set_fast_spec_arm. Arm OFF routes every block to the generic bodies (the old
//! shipped behaviour, since the old specialised arms served ZERO blocks); arm ON
//! routes to the new specialised copies. Must be byte-identical: the consts take
//! the values the runtime variables already held.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let (mut same, mut tot) = (0usize, 0usize);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(6 << 20)];
        for lvl in [1, 2] {
            rusty_zstd::set_fast_spec_arm(false);
            let a = rusty_zstd::compress(s, lvl).unwrap();
            rusty_zstd::set_fast_spec_arm(true);
            let b = rusty_zstd::compress(s, lvl).unwrap();
            tot += 1;
            if a == b { same += 1 } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", a.len(), b.len()); }
            assert!(rusty_zstd::decompress(&b).unwrap() == s, "{id} L{lvl} round-trip");
        }
    }
    println!("BYTE-IDENTICAL across the arm: {same}/{tot} cells");
}
