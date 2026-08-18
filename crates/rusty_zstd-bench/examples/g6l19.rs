//! GATE 6 @ L19. Dead by construction -- but is the CAPABILITY missing, or
//! already subsumed? The pair search means "when ip misses, also try ip+1".
//! find_opt's DP visits positions itself, so measure how densely it searches.
fn main() {
    println!("{:<10}{:>12}{:>14}{:>14}{:>10}", "corpus", "bytes", "positions", "bt searches", "per pos");
    for id in ["xml","osdb","nci","webster","mozilla"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(1024*1024)];
        let _ = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, 19).unwrap();
        let (sp, rt) = rusty_zstd::take_bt_calls();
        let calls = sp + rt;
        println!("{id:<10}{:>12}{:>14}{:>14}{:>10.3}", src.len(), src.len(), calls,
                 calls as f64 / src.len() as f64);
    }
    println!("\nAt L1 the pair search adds ONE extra probe (ip+1) only when ip missed.");
    println!("If find_opt already searches ~1 position per byte, ip+1 is searched");
    println!("unconditionally and the pair capability is SUBSUMED, not missing.");
}
