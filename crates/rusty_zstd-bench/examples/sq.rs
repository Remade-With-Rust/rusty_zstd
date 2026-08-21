fn main() {
    for bar in [0.60f32, 0.65, 0.70] {
        rusty_zstd::set_wide_first_max_arm(bar);
        for lvl in [9i32, 12] {
            let ids = ["smallmsg-8m","jsonlog-16m","mr","dickens","reymont","webster","sao","xml"];
            print!("bar {bar} L{lvl}: ");
            let (mut ta, mut tb) = (0u64, 0u64);
            let mut worst=(0.0f64,"-");
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                let s = &f[..f.len().min(6 << 20)];
                rusty_zstd::set_wide_chain_arm(false);
                let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                rusty_zstd::set_wide_chain_arm(true);
                let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
                if d > worst.0 { worst=(d,id); }
                ta+=za.len() as u64; tb+=zb.len() as u64;
            }
            println!("total {:+.4}% worst {} {:+.3}%", 100.0*(tb as f64-ta as f64)/ta as f64, worst.1, worst.0);
        }
    }
}
