//! The wide chain key adjudicated: sizes A/B plus the walk-work census
//! (exams = candidate steps; the wide key should remove the ~48% that are
//! collision link-chases). Requires --features profile for the census.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    for lvl in [5i32, 7, 9, 12] {
        let (mut ta, mut tb, mut texa, mut texb) = (0u64, 0u64, 0u64, 0u64);
        let mut worst = (0.0f64, "-");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            rusty_zstd::set_wide_chain_arm(false);
            #[cfg(feature = "profile")]
            let _ = rusty_zstd::take_walk_census();
            let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            #[cfg(feature = "profile")]
            let (exa, _m) = rusty_zstd::take_walk_census();
            #[cfg(not(feature = "profile"))]
            let exa = 0u64;
            rusty_zstd::set_wide_chain_arm(true);
            let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&zb).unwrap() == s, "{id} L{lvl} round-trip");
            #[cfg(feature = "profile")]
            let (exb, _mb) = rusty_zstd::take_walk_census();
            #[cfg(not(feature = "profile"))]
            let exb = 0u64;
            let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
            if d > worst.0 { worst = (d, id); }
            ta += za.len() as u64; tb += zb.len() as u64;
            texa += exa; texb += exb;
        }
        println!("L{lvl}: size {:+.4}%  worst {} {:+.4}%  exams {} -> {} ({:+.1}%)",
            100.0*(tb as f64-ta as f64)/ta as f64, worst.1, worst.0,
            texa, texb, if texa==0 {0.0} else {100.0*(texb as f64-texa as f64)/texa as f64});
    }
}
