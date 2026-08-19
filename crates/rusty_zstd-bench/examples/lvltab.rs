//! L19 and L22 produced sizes 45 bytes apart across 18 corpora. What actually
//! differs between the upper levels' parameters?
fn main(){
    let n:u64=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(2<<20);
    println!("params for a {} KiB input\n", n>>10);
    println!("{:>4}{:>10}{:>7}{:>7}{:>8}{:>8}{:>9}{:>9}","lvl","strategy","wlog","hlog","clog","slog","minmatch","tlen");
    for lvl in [13i32,16,17,18,19,20,21,22]{
        let p=rusty_zstd::compression_params(lvl,Some(n)).unwrap();
        println!("{lvl:>4}{:>10}{:>7}{:>7}{:>8}{:>8}{:>9}{:>9}",
            format!("{:?}",p.strategy),p.window_log,p.hash_log,p.chain_log,p.search_log,p.min_match,p.target_length);
    }
}
