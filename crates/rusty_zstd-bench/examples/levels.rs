fn main() {
    let mut prev = String::new();
    for lv in rusty_zstd::MIN_CLEVEL..=rusty_zstd::MAX_CLEVEL {
        let p = rusty_zstd::compression_params(lv, None).unwrap();
        let s = format!("{:?}", p.strategy);
        let mark = if s != prev { " <-- strategy change" } else { "" };
        println!("L{:<4} {:<10} wlog={:<3} clog={:<3} hlog={:<3} slog={:<2} mml={} tlen={}{}",
            lv, s, p.window_log, p.chain_log, p.hash_log, p.search_log, p.min_match, p.target_length, mark);
        prev = s;
    }
}
