fn main(){
    let n: u64 = 2<<20;
    println!("{:<5}{:>7}{:>7}{:>7}{:>7}{:>7}{:>8}  {}", "lvl","wlog","clog","hlog","slog","mml","tlen","strategy");
    for l in 16..=22 {
        let p=rusty_zstd::compression_params(l, Some(n)).unwrap();
        println!("L{l:<4}{:>7}{:>7}{:>7}{:>7}{:>7}{:>8}  {:?}",
            p.window_log,p.chain_log,p.hash_log,p.search_log,p.min_match,p.target_length,p.strategy);
    }
}
