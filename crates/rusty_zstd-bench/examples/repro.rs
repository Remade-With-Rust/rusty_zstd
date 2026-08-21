fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
        "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    for lvl in [5i32, 7, 9, 12] {
        let (mut ta, mut tb) = (0u64, 0u64);
        let mut worst = (0.0f64, "-");
        let mut best = (0.0f64, "-");
        for id in ids {
            let f = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
            let s = &f[..f.len().min(6 << 20)];
            rusty_zstd::set_rep_reprobe_arm(false);
            let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            rusty_zstd::set_rep_reprobe_arm(true);
            let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&zb).unwrap() == s);
            let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
            if d > worst.0 { worst = (d, id); }
            if d < best.0 { best = (d, id); }
            ta += za.len() as u64; tb += zb.len() as u64;
        }
        println!("L{lvl}: total {:+.4}%  best {} {:+.4}%  worst {} {:+.4}%",
            100.0*(tb as f64 - ta as f64)/ta as f64, best.1, best.0, worst.1, worst.0);
    }
}
