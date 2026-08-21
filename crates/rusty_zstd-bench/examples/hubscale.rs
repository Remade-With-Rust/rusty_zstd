//! Is the strategy HUB's own per-block body worth optimizing? Ratio test:
//! blocks entered vs DP positions visited vs bt probes issued.
fn main() {
    #[cfg(feature = "profile")]
    {
        for (lvl, ids) in [(19i32, ["dickens", "webster"]), (3, ["dickens", "webster"])] {
            let _ = rusty_zstd::take_opt_skips();
            let _ = rusty_zstd::take_bt_iters();
            let mut bytes = 0usize;
            for id in ids {
                let f = std::fs::read(format!("corpora/data/silesia/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))).unwrap();
                let s = &f[..f.len().min(4 << 20)];
                bytes += s.len();
                let _ = rusty_zstd::compress(s, lvl).unwrap();
            }
            let (pos, _, _, _) = rusty_zstd::take_opt_skips();
            let (walks, iters, _) = rusty_zstd::take_bt_iters();
            let blocks = bytes.div_ceil(128 * 1024);
            println!("L{lvl}: {bytes} B -> ~{blocks} blocks | DP positions {pos} | bt walks {walks} iters {iters}");
            if blocks > 0 {
                println!("     per block: {} DP positions, {} bt iters", pos / blocks as u64, iters / blocks as u64);
            }
        }
    }
}
