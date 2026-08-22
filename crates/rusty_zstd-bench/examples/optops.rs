//! W1 receipt: how many `ops` tuples did the parse push before vs after?
//! Before = one per PARSE STEP; after = one per MATCHED step. The step count
//! is positions-visited minus jumped spans; the matched count is `seqs`.
fn main() {
    #[cfg(feature = "profile")]
    {
        let ids = ["dickens","webster","mozilla","nci","samba","xml","sao","mr"];
        for lvl in [16i32, 19, 22] {
            let _ = rusty_zstd::take_opt_skips();
            let _ = rusty_zstd::take_opt_bt();
            for id in ids {
                let f = std::fs::read(format!("corpora/data/silesia/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))).unwrap();
                let s = &f[..f.len().min(2 << 20)];
                let _ = rusty_zstd::compress(s, lvl).unwrap();
            }
            let (pos, _, _, _) = rusty_zstd::take_opt_skips();
            let (_, _, _, seqs) = rusty_zstd::take_opt_bt();
            let bytes_before = pos * 16;
            let bytes_after = seqs * 16;
            println!("L{lvl}: parse steps ~{pos}, matched {seqs} -> ops tuples {:.1}x fewer; \
                      bytes {} -> {} ({:.1} MB saved)",
                pos as f64 / seqs.max(1) as f64, bytes_before, bytes_after,
                (bytes_before - bytes_after) as f64 / 1.048576e6);
        }
    }
}
