fn main() {
    let ids = ["jsonlog-16m","smallmsg-8m","mr","dickens","reymont","webster","sao","xml"];
    for thr in [0.60f32, 0.70, 0.80] {
        rusty_zstd::set_walk_first_max_arm(thr);
        for lvl in [5i32, 9] {
            print!("first<={thr} L{lvl}: ");
            let mut tot_a = 0u64; let mut tot_b = 0u64; let mut worst = (0.0f64, "-");
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                let s = &f[..f.len().min(6 << 20)];
                rusty_zstd::set_walk_cont_arm(false);
                let za = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                rusty_zstd::set_walk_cont_arm(true);
                let zb = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
                let d = 100.0*(zb.len() as f64 - za.len() as f64)/za.len() as f64;
                if d > worst.0 { worst = (d, id); }
                tot_a += za.len() as u64; tot_b += zb.len() as u64;
            }
            println!("total {:+.4}%  worst {} {:+.3}%", 100.0*(tot_b as f64 - tot_a as f64)/tot_a as f64, worst.1, worst.0);
        }
    }
}
