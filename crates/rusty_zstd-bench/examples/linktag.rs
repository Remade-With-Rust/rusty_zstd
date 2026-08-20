//! Chain-link tag receipt: bytes must be IDENTICAL across the arm (the tag
//! provably cannot hide an mls_eq match), FALSE skips must be 0, and the
//! skip count is the src loads avoided. Requires --features profile.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        for lvl in [5i32, 7, 9, 12] {
            let (mut cells, mut same, mut tskip) = (0usize, 0usize, 0u64);
            for id in IDS {
                let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
                let s = &f[..f.len().min(6 << 20)];
                rusty_zstd::set_chain_tag_arm(false);
                let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                rusty_zstd::set_chain_tag_arm(true);
                let _ = rusty_zstd::take_link_tag();
                let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&zb).unwrap() == s, "{id} L{lvl} round-trip");
                let (skips, fal) = rusty_zstd::take_link_tag();
                assert_eq!(fal, 0, "{id} L{lvl}: {fal} FALSE link-tag skips");
                cells += 1;
                if za == zb { same += 1; } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", za.len(), zb.len()); }
                tskip += skips;
            }
            println!("L{lvl}: {same}/{cells} byte-identical, {tskip} src loads skipped by the link tag, FALSE 0");
        }
    }
}
