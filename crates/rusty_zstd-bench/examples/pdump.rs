fn main() {
    for hint in [None, Some(262144u64), Some(8u64*1024*1024)] {
        println!("--- src_hint = {:?} ---", hint);
        for lv in [19, 22] {
            let p = rusty_zstd::compression_params(lv, hint).unwrap();
            println!("  L{lv} {:?} wlog={} clog={} hlog={} slog={} mml={} tlen={}",
                p.strategy, p.window_log, p.chain_log, p.hash_log, p.search_log, p.min_match, p.target_length);
        }
    }
}
