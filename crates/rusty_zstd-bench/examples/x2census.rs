//! Which of `decode_4x`'s two paths actually runs? `use_x2` picks per literals
//! section, and the answer decides whether the X1 path's share of three ISA
//! twins is code anyone executes.
fn main() {
    #[cfg(feature = "profile")]
    {
        let _ = rusty_zstd::take_x2_stats();
        for (dir, id) in [
            ("silesia", "dickens"),
            ("silesia", "samba"),
            ("silesia", "xml"),
            ("silesia", "webster"),
            ("silesia", "mozilla"),
        ] {
            let Ok(f) = std::fs::read(format!("corpora/data/{dir}/{id}")) else {
                continue;
            };
            let s = &f[..f.len().min(4 << 20)];
            for lvl in [1i32, 3, 9, 19] {
                let c = rusty_zstd::compress_with(
                    s,
                    rusty_zstd::CompressOptions { level: lvl, checksum: false },
                )
                .unwrap();
                let _ = rusty_zstd::decompress(&c).unwrap();
            }
        }
        let x1 = rusty_zstd::take_x4_x1_calls();
        let (_, x2) = rusty_zstd::take_x2_stats();
        let tot = x1 + x2;
        let (bail, ok) = rusty_zstd::take_f4x2_arm();
        let ft = bail + ok;
        println!(
            "fast_4x2: succeeded={ok} ({:.2}%)  bailed={bail} ({:.2}%)",
            if ft == 0 { 0.0 } else { ok as f64 * 100.0 / ft as f64 },
            if ft == 0 { 0.0 } else { bail as f64 * 100.0 / ft as f64 },
        );
        println!(
            "sections: x1={x1} ({:.2}%)  x2={x2} ({:.2}%)  total={tot}",
            if tot == 0 { 0.0 } else { x1 as f64 * 100.0 / tot as f64 },
            if tot == 0 { 0.0 } else { x2 as f64 * 100.0 / tot as f64 },
        );
    }
    #[cfg(not(feature = "profile"))]
    println!("needs --features rusty_zstd/profile");
}
