fn main() {
    let id = std::env::args().nth(1).unwrap_or("mozilla".into());
    let full = std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))).unwrap();
    let src = &full[..full.len().min(8*1024*1024)];
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.20);
    let _ = rusty_zstd::compress(src, 1).unwrap();
}
