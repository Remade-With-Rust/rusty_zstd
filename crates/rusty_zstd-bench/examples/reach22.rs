//! GATE 4 @ L22: reachability, WHY it is dead, and whether the bt-path
//! specialisation actually covers this level.
fn main() {
    println!("{:<8}{:>10}{:>10}{:>12}{:>12}{:>10}", "level", "fast", "opt", "bt-spec", "bt-runtime", "params");
    for &(lvl, cap) in &[(13usize, 2), (19, 2), (22, 2)] {
        let src = std::fs::read("corpora/data/silesia/xml").unwrap();
        let src = &src[..src.len().min(cap * 1024 * 1024)];
        let p = rusty_zstd::compression_params(lvl as i32, Some(src.len() as u64)).unwrap();
        let _ = rusty_zstd::take_finder_calls();
        let _ = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, lvl as i32).unwrap();
        let (f, o) = rusty_zstd::take_finder_calls();
        let (bs, br) = rusty_zstd::take_bt_calls();
        println!(
            "L{lvl:<7}{f:>10}{o:>10}{bs:>12}{br:>12}   {:?} h={} c={}",
            p.strategy, p.hash_log, p.chain_log.min(24)
        );
    }
    println!("\nWHY Gate 4 is dead at L19/L22: find_sequences_strategy routes");
    println!("BtUltra2 -> find_opt. find_fast is entered only when strategy == Fast.");
    println!("bt-runtime > 0 would mean the new specialisation MISSES this level.");
}
