//! RECEIPT GAP: the walk-continue adjudication ran on 6 MiB slices only, but
//! it ships on >= 16 MiB frames too -- where chain_pack cannot reach (the
//! 24-bit proof fails), so every extra exam pays the full src load. Board it.
fn main() {
    let ids = ["versions-16m","jsonlog-16m","mozilla","webster","nci","samba"];
    for lvl in [7i32, 12] {
        let (mut ta, mut tb) = (0u64, 0u64);
        let mut worst = (0.0f64, "-");
        for id in ids {
            let f = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
            assert!(f.len() >= 0x0100_0000);
            rusty_zstd::set_walk_cont_arm(false);
            let za = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            rusty_zstd::set_walk_cont_arm(true);
            let zb = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
            if d > worst.0 { worst = (d, id); }
            println!("L{lvl} {id:14} {:10} -> {:10}  {d:+.4}%", za.len(), zb.len());
            ta += za.len() as u64; tb += zb.len() as u64;
        }
        println!("L{lvl} TOTAL {:+.4}%  worst {} {:+.4}%", 100.0*(tb as f64 - ta as f64)/ta as f64, worst.1, worst.0);
    }
}
