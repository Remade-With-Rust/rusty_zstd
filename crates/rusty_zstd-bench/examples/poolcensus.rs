//! Scratch-pool hit/miss/drop census.
//!
//! The allocation census puts the encoder at ~144 allocations per MiB against
//! the decoder's 2.1, and attribution lands on functions that ALREADY pool
//! their buffers. A pool that misses allocates, and at the call site a miss is
//! indistinguishable from never having pooled -- so the hit rate is the whole
//! question, and a DROP (free list full) is what starves the next take.
fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    println!(
        "{:<12}{:>7}{:>10}{:>9}{:>8}{:>9}{:>10}{:>8}",
        "corpus", "MiB", "hits", "misses", "drops", "gives", "give0", "hit%"
    );
    let (mut th, mut tm, mut td, mut tg, mut tge) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for id in ["dickens", "samba", "webster", "mozilla"] {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
            continue;
        };
        let src = &f[..f.len().min(32 << 20)];
        let _ = rusty_zstd::take_pool_census();
        let z = rusty_zstd::compress(src, lvl).expect("c");
        let (h, m, d, g, ge) = rusty_zstd::take_pool_census();
        assert_eq!(rusty_zstd::decompress(&z).expect("d"), src);
        let _ = rusty_zstd::take_pool_census();
        println!(
            "{id:<12}{:>7.1}{h:>10}{m:>9}{d:>8}{g:>9}{ge:>10}{:>7.1}%",
            src.len() as f64 / (1 << 20) as f64,
            if h + m == 0 {
                0.0
            } else {
                100.0 * h as f64 / (h + m) as f64
            }
        );
        th += h;
        tm += m;
        td += d;
        tg += g;
        tge += ge;
    }
    println!(
        "\nTOTAL hits {th} misses {tm} drops {td} -> {:.1}% hit rate",
        if th + tm == 0 {
            0.0
        } else {
            100.0 * th as f64 / (th + tm) as f64
        }
    );
    println!("every MISS is an allocation; every DROP is a buffer thrown away that a\nlater take then had to allocate.");
}
