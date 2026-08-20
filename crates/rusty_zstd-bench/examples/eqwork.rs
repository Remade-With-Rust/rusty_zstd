//! count_eq_len work receipt: (calls, total bytes counted) over the board.
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = ["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao",
            "webster","dickens","mozilla","nci","samba","xml","x-ray","zeros-32m","text-32m","incomp-32m"];
        for lvl in [3i32, 5, 7, 9, 12] {
            let _ = rusty_zstd::take_eqlen_stats();
            for id in ids {
                let f = std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
                let s = &f[..f.len().min(6 << 20)];
                let _ = rusty_zstd::compress_with(s, rusty_zstd::CompressOptions { level: lvl, checksum: false }).unwrap();
            }
            let (calls, _we, h) = rusty_zstd::take_eqlen_stats();
            println!("L{lvl}: calls {calls}, len-hist [<8:{} 8-31:{} 32-63:{} 64-255:{} 256+:{}]", h[0],h[1],h[2],h[3],h[4]);
        }
    }
}
