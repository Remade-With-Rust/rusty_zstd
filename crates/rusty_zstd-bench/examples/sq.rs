fn main() {
    let ids = ["versions-16m","jsonlog-16m","mozilla","webster","nci","samba"];
    for lvl in [7i32, 12] {
        let (mut ta, mut tb) = (0u64, 0u64);
        let mut worst=(0.0f64,"-");
        print!("L{lvl}: ");
        for id in ids {
            let f = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
            rusty_zstd::set_wide_chain_arm(false);
            let za = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            rusty_zstd::set_wide_chain_arm(true);
            let zb = rusty_zstd::compress_with(&f, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&zb).unwrap() == f);
            let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
            if d > worst.0 { worst=(d,id); }
            print!("{id} {d:+.3}%  ");
            ta+=za.len() as u64; tb+=zb.len() as u64;
        }
        println!("| TOTAL {:+.4}% worst {} {:+.3}%", 100.0*(tb as f64-ta as f64)/ta as f64, worst.1, worst.0);
    }
}
