fn main() {
    let n = 262144usize;
    for id in ["zeros-32m", "xml"] {
        let src = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
        let src = &src[..src.len().min(n)];
        for lv in [19, 22] {
            let p = rusty_zstd::compression_params(lv, Some(src.len() as u64)).unwrap();
            let t = std::time::Instant::now();
            let z = rusty_zstd::compress(src, lv).unwrap();
            println!("{id:<10} L{lv} slog={:<3} -> {:>8} B in {:>9.1} ms",
                p.search_log, z.len(), t.elapsed().as_secs_f64()*1000.0);
        }
    }
}
