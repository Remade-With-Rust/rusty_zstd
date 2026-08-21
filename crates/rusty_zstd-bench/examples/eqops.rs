fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = ["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao",
            "webster","dickens","mozilla","nci","samba","xml","x-ray","zeros-32m","text-32m","incomp-32m"];
        for lvl in [3i32, 9, 12, 19] {
            let _ = rusty_zstd::take_eq_ops();
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                let s = &f[..f.len().min(4 << 20)];
                let _ = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            }
            let (v, w, o) = rusty_zstd::take_eq_ops();
            println!("L{lvl}: vector ops {v}, word ops {w}, other {o}");
        }
    }
}
