fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = ["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao",
            "webster","dickens","mozilla","nci","samba","xml","x-ray","zeros-32m","text-32m","incomp-32m"];
        for lvl in [1i32, 2, 5, 7, 9, 12, 13] {
            let _ = rusty_zstd::take_bext();
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                let s = &f[..f.len().min(4 << 20)];
                let _ = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            }
            let (m, n, b, ge8) = rusty_zstd::take_bext();
            let mean = if n==0 {0.0} else {b as f64/n as f64};
            println!("L{lvl:2}: matches {m:9}, extended {n:8} ({:4.1}%), bytes {b:9}, mean-ext {mean:5.2}, >=8 {ge8:7} ({:4.2}% of extended)",
                if m==0 {0.0} else {100.0*n as f64/m as f64}, if n==0 {0.0} else {100.0*ge8 as f64/n as f64});
        }
    }
}
