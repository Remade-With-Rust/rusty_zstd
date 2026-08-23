//! Decoder literal-copy tier receipt: 16 / 32 / 64 / memcpy-call.
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = [("silesia","dickens"),("silesia","samba"),("silesia","webster"),
                   ("silesia","xml"),("silesia","mozilla"),("silesia","nci"),
                   ("generated","jsonlog-16m"),("generated","smallmsg-8m")];
        for lvl in [3i32, 9, 19] {
            let mut frames = Vec::new();
            for (dir,id) in ids {
                let Ok(f) = std::fs::read(format!("corpora/data/{dir}/{id}")) else { continue };
                let s = &f[..f.len().min(4<<20)];
                frames.push(rusty_zstd::compress_with(
                    s, rusty_zstd::CompressOptions{level:lvl,checksum:false}).unwrap());
            }
            let _ = rusty_zstd::take_dec_copies(); let _ = rusty_zstd::take_dec_lit64();
            for f in &frames { let _ = rusty_zstd::decompress(f).unwrap(); }
            let (l32,_m32,l16,_m16) = rusty_zstd::take_dec_copies();
            let l64 = rusty_zstd::take_dec_lit64();
            println!("L{lvl:<2} lit16 {l16:>9}  lit32 {l32:>8}  lit64 {l64:>8}  \
                      (64-tier converted {l64} memcpy calls)");
        }
    }
}
