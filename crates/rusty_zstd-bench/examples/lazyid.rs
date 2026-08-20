fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
        "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    for lvl in [5i32, 7, 9, 12] {
        let mut tot = 0u64;
        for id in ids {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(6 << 20)];
            let z = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            tot += z.len() as u64;
        }
        println!("L{lvl}: total {tot}");
    }
}
