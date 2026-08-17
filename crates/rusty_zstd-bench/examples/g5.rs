//! GATE 5 @ L3: is the specialised HLOG set COMPLETE for the reachable
//! parameter space? A missing value silently falls to the runtime arm, and the
//! specialisation then does nothing while still being reported as shipped.
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("{:<14}{:>9}{:>7}{:>14}{:>13}   arm", "corpus", "MiB", "hlog", "spec calls", "runtime calls");
    let (mut ts, mut tr) = (0u64, 0u64);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let p = rusty_zstd::compression_params(lvl, Some(full.len() as u64)).unwrap();
        let _ = rusty_zstd::take_dfast_calls();
        let _ = rusty_zstd::compress(&full, lvl).unwrap();
        let (sp, rt) = rusty_zstd::take_dfast_calls();
        ts += sp; tr += rt;
        let arm = if rt > 0 { "*** RUNTIME FALLBACK ***" } else if sp > 0 { "specialised" } else { "finder not used" };
        println!("{id:<14}{:>9.1}{:>7}{sp:>14}{rt:>13}   {arm}", full.len() as f64/1048576.0, p.hash_log);
    }
    println!("\ntotal specialised={ts}  runtime-fallback={tr}");
    println!("{}", if tr == 0 { "GATE 5 SET IS COMPLETE for this level" } else { "GATE 5 SET IS INCOMPLETE -- some inputs miss the fold" });
}
