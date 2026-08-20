//! PROMETHEUS PREREQ: how often is each fitted constant READ per frame?
//! Every one of these accessors calls `std::env::var` with no cache.
const NAMES: [&str; 14] = ["fast_lazy_threshold","rep_len_min","rep_decay","rep_yield_min_for",
    "rep_yield_min","dfast_ml_min","opt_rep_min","opt_fill_enabled","opt_fill_rep_max",
    "opt_fill_max","opt_fill_stride","next_long_min","pair_rep_max","tag_min"];
fn main() {
    let cap: usize = 8 << 20;
    for lvl in [1, 3, 5, 9, 13, 19, 22] {
        let mut tot = [0u64; 14];
        let mut bytes = 0usize;
        for id in ["dickens", "samba", "mozilla", "x-ray"] {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            let _ = rusty_zstd::take_envhits();
            let _ = rusty_zstd::compress(s, lvl).unwrap();
            let h = rusty_zstd::take_envhits();
            for i in 0..14 { tot[i] += h[i]; }
            bytes += s.len();
        }
        let sum: u64 = tot.iter().sum();
        let top: Vec<String> = {
            let mut v: Vec<(usize, u64)> = tot.iter().copied().enumerate().collect();
            v.sort_by_key(|x| std::cmp::Reverse(x.1));
            v.into_iter().filter(|x| x.1 > 0).take(4)
                .map(|(i, c)| format!("{}={}", NAMES[i], c)).collect()
        };
        println!("L{lvl:<3} {sum:>9} env::var reads over {} MiB   top: {}", bytes >> 20, top.join("  "));
    }
}
