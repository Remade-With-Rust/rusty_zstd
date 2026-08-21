fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
        "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    for lvl in [13i32, 16, 19, 22] {
        let mut tot = 0u64;
        for id in ids {
            let f = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
            let s = &f[..f.len().min(2 << 20)];
            let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == s);
            tot += z.len() as u64;
        }
        println!("L{lvl}: total {tot}");
    }
}
