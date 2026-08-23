//! GATE 13 literal-push receipt: how many appends the tiered wildcopy serves.
//! `take_lit_push()` -> (tier1, slow) ; `take_lit_tiers()` -> (tier2, tier3)
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = [
            ("silesia","dickens"),("silesia","samba"),("silesia","webster"),
            ("silesia","xml"),("silesia","mozilla"),("silesia","nci"),
            ("generated","jsonlog-16m"),("generated","smallmsg-8m"),
        ];
        for lvl in [1i32, 3, 5, 7, 9, 12, 19] {
            let _ = rusty_zstd::take_lit_push();
            let _ = rusty_zstd::take_lit_tiers();
            for (dir,id) in ids {
                let Ok(f) = std::fs::read(format!("corpora/data/{dir}/{id}")) else { continue };
                let s = &f[..f.len().min(4<<20)];
                let _ = rusty_zstd::compress_with(
                    s, rusty_zstd::CompressOptions{level:lvl,checksum:false}).unwrap();
            }
            let (t1, slow) = rusty_zstd::take_lit_push();
            let (t2, t3) = rusty_zstd::take_lit_tiers();
            let tot = t1 + t2 + t3 + slow;
            if tot == 0 { println!("L{lvl:<2} (none)"); continue; }
            println!("L{lvl:<2} appends {tot:>10}  tier1 {t1:>10}  tier2 {t2:>8}  tier3 {t3:>7}  \
                      memcpy-call {slow:>9} ({:5.2}%)", slow as f64*100.0/tot as f64);
        }
    }
}
