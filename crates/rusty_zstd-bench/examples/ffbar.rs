fn main() {
    for id in ["versions-16m", "dickens", "reymont", "mr"] {
        let f = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
        let s = &f[..f.len().min(8 << 20)];
        rusty_zstd::set_fast_hash_arm(false);
        let a = rusty_zstd::compress(s, 1).unwrap().len();
        rusty_zstd::set_fast_hash_arm(true);
        let b = rusty_zstd::compress(s, 1).unwrap().len();
        println!("{id:<13} legacy {a:>9}  wide+bar {b:>9}  {:+.3}%", (b as f64/a as f64-1.0)*100.0);
    }
}
