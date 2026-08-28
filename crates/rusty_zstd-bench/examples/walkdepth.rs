//! IS `attempts = 16` DEEPER THAN THE CHAIN JUSTIFIES?
//!
//! `walkexit.rs` measures **58.9% of L9 chain walks running their full depth** --
//! they spend all `1 << search_log` attempts and stop because the budget ran
//! out, not because they found anything or hit a bound. Roughly half of those
//! iterations are tag rejects rather than probes, and each one is a DEPENDENT
//! load (`m = chain[m & mask]`, address from the previous load's result).
//!
//! Section 10 closed the instruction angle on this walk: three of its seven
//! exits never fire, its validity tests are already folded three-into-one, and
//! const-folding `cp`/`ca` would duplicate the whole walk at the ratio D4
//! retired ("152 instructions per op, the worst in the crate"). What is left is
//! the TRIP COUNT, and `search_log` sets it directly.
//!
//! Both columns are deterministic: the walk-exit census and the probe count are
//! counters, and the compressed size is the bitstream. A row that cuts depth
//! hard for little size is the same trade `fillcut.rs` found for the fill.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example walkdepth [level]

const IDS: &[&str] = &["dickens", "webster", "samba", "mozilla", "osdb", "mr"];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);

    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| { let n = f.len().min(cap); f[..n].to_vec() })
        })
        .collect();
    let total: u64 = srcs.iter().map(|s| s.len() as u64).sum();
    let base = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");

    println!(
        "L{lvl} WALK-DEPTH BOARD -- {} corpora, {:.1} MiB\nshipping search_log = {} (attempts = {})\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64,
        base.search_log,
        1u32 << base.search_log
    );
    println!(
        "{:>10} {:>9} {:>12} {:>11} {:>12} {:>11} {:>10}",
        "search_log", "attempts", "probes", "probes/B", "depth spent", "bytes", "size vs"
    );

    let mut b_bytes = 0u64;
    let mut b_probes = 0u64;
    for sl in [base.search_log, base.search_log - 1, base.search_log - 2, base.search_log + 1] {
        let mut p = base;
        p.search_log = sl;
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_walk_exit();
        let mut bytes = 0u64;
        for s in &srcs {
            let z = rusty_zstd::compress_with_params(s, p, false).expect("compress");
            assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..]);
            bytes += z.len() as u64;
        }
        let e = rusty_zstd::take_walk_exit();
        let c = rusty_zstd::prof_encode_counts();
        let walks: u64 = e[..7].iter().sum();
        if sl == base.search_log {
            b_bytes = bytes;
            b_probes = c.hash_probes;
        }
        println!(
            "{:>10} {:>9} {:>12} {:>11.3} {:>11.1}% {:>11} {:>9.3}%",
            format!("{}{}", sl, if sl == base.search_log { "*" } else { "" }),
            1u32 << sl,
            c.hash_probes,
            c.hash_probes as f64 / total as f64,
            if walks > 0 { 100.0 * e[5] as f64 / walks as f64 } else { 0.0 },
            bytes,
            if b_bytes > 0 { 100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64 } else { 0.0 }
        );
    }
    let _ = b_probes;
    println!(
        "\n`depth spent` is the share of walks that exhausted their budget. If it\n\
         stays high as the budget shrinks, the chains are longer than any\n\
         affordable depth and the extra attempts are buying nothing."
    );
}
