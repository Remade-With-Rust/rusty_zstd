//! Array-route chain-link tag on FULL-LENGTH (>= 16 MiB) frames: bytes must
//! be identical across the arm, FALSE 0, skips = src loads avoided.
//! Requires --features profile.
fn main() {
    #[cfg(not(feature = "profile"))]
    panic!("run with --features profile");
    #[cfg(feature = "profile")]
    {
        let ids = ["versions-16m","jsonlog-16m","mozilla","webster","nci","samba"];
        for lvl in [7i32, 12] {
            let (mut cells, mut same, mut tskip) = (0usize, 0usize, 0u64);
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                assert!(f.len() >= 0x0100_0000);
                rusty_zstd::set_chain_tag_arm(false);
                let za = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                rusty_zstd::set_chain_tag_arm(true);
                let _ = rusty_zstd::take_link_tag();
                let zb = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                assert!(rusty_zstd::decompress(&zb).unwrap() == f, "{id} L{lvl} round-trip");
                let (skips, fal) = rusty_zstd::take_link_tag();
                assert_eq!(fal, 0, "{id} L{lvl}: {fal} FALSE skips");
                cells += 1;
                if za == zb { same += 1; } else { println!("  MISMATCH {id} L{lvl}: {} vs {}", za.len(), zb.len()); }
                tskip += skips;
            }
            println!("L{lvl}: {same}/{cells} byte-identical, {tskip} src loads skipped, FALSE 0");
        }
    }
}
