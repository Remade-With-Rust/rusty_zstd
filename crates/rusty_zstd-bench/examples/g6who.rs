//! How many blocks reach find_fast at all, per corpus? The per-block gate can
//! only work on blocks that RUN it.
const IDS: &[&str] = &["mozilla","samba","nci","x-ray","sao","webster","dickens","xml","osdb","mr"];
fn main() {
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.0);
    println!("{:<12}{:>10}{:>14}{:>14}", "corpus", "MiB", "blocks(128K)", "find_fast calls");
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let _ = rusty_zstd::take_finder_calls();
        let _ = rusty_zstd::compress(src, 1).unwrap();
        let (fast, _opt) = rusty_zstd::take_finder_calls();
        println!("{id:<12}{:>10.1}{:>14}{:>14}", src.len() as f64/1048576.0,
                 (src.len()+131071)/131072, fast);
    }
}
