//! WHY IS A WELL-OPTIMISED ENCODER 2-3x BEHIND? The residency test.
//!
//! Every measurement this campaign runs is an INSTRUCTION COUNT. That is the
//! right instrument for code size and work-per-call, and it is load-independent
//! -- which is why it was chosen on a noisy host. But it cannot see a STALL.
//! A 1,568-instruction loop can spend most of its cycles waiting on memory, and
//! removing instructions from the shadow of a cache miss changes nothing.
//!
//! L9's own parameters say this is the situation: `hash_log 21` is an 8 MiB
//! table and `chain_log 20` a 4 MiB one, both HASH-indexed, i.e. randomly. And
//! the chain walk is a DEPENDENT load chain -- `m = chain[m & mask]`, where
//! each load's address is the previous load's result, so nothing can overlap.
//! At 1.57 probes per input byte with a 3-10% hit rate, that is ~1.5 serialised
//! misses per byte with no other work in flight to hide them.
//!
//! THE TEST NEEDS NO PERFORMANCE COUNTERS. Shrink the tables until they fit
//! cache and see whether throughput moves:
//!
//!   * a LARGE speedup as the tables shrink => the walk is stalling on memory,
//!     and the lever is the access pattern (C's row finder), not instructions
//!   * a FLAT column => the walk really is execution-bound and the instruction
//!     grind was aimed correctly
//!
//! The two outcomes are far apart, which is what makes this readable on a busy
//! host. SIZE is deterministic and exact. The time column is best-of-N with a
//! same-input null arm beside it; read it only as "did this move by MUCH more
//! than the spread".
//!
//! READ THE CONTROL OR THE HEADLINE WILL LIE TO YOU. Shrinking the tables does
//! not only change RESIDENCY -- it also raises the hash collision rate and
//! shrinks `chain_mask`, so the walk's `next >= m` guard breaks sooner and the
//! probe COUNT falls. Measured at L9: 1.845 -> 0.553 probes per byte across
//! these four rows, a 3.3x work reduction against a 2.9x speedup. So the
//! headline is mostly LESS WORK, not faster work.
//!
//! The residency signal is PROBES PER SECOND, which divides the work out:
//!
//! ```text
//!   8M+4M      1.845 probes/B    24.6 MB/s    47.6M probes/s
//!   1M+512K    1.370 probes/B    43.4 MB/s    62.3M probes/s   <- +31%
//!   256K+128K  0.832 probes/B    61.8 MB/s    53.9M probes/s
//!   64K+32K    0.553 probes/B    71.9 MB/s    41.7M probes/s
//! ```
//!
//! Going from 12 MiB of tables to 1.5 MiB buys **~1.3x per probe** -- a real
//! cache effect, and far smaller than the raw throughput column suggests. The
//! column is NOT monotone below that, so the smallest rows are measuring
//! something other than residency (chain shape, branch behaviour) and should
//! not be read as residency at all.
//!
//! usage: cargo run --release -p rusty_zstd-bench --example l9cache [level]

const IDS: &[&str] = &["dickens", "webster", "samba", "mozilla", "osdb", "mr"];

fn tbl(log: u32) -> String {
    let b = (1u64 << log) * 4;
    if b >= 1 << 20 { format!("{}M", b >> 20) } else { format!("{}K", b >> 10) }
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(4 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(5);

    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| { let k = f.len().min(cap); (*id, f[..k].to_vec()) })
        })
        .collect();
    let total: usize = srcs.iter().map(|(_, s)| s.len()).sum();
    let base = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");

    println!(
        "L{lvl} TABLE-RESIDENCY BOARD -- {} corpora, {:.1} MiB, best-of-{n}\n\
         shipping: hash_log {} ({}), chain_log {} ({}) = {} of hash-indexed tables\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64,
        base.hash_log, tbl(base.hash_log),
        base.chain_log, tbl(base.chain_log),
        tbl(base.hash_log + 1),
    );
    println!("{:>6} {:>6} {:>9} {:>12} {:>9} {:>9} {:>8}",
             "hlog", "clog", "tables", "bytes", "size vs", "MB/s", "spread");

    let (mut b_bytes, mut b_mbps) = (0u64, 0f64);
    // Walk the tables down toward L2 residency, holding everything else fixed.
    for (hl, cl) in [
        (base.hash_log, base.chain_log),
        (base.hash_log - 3, base.chain_log - 3),
        (base.hash_log - 5, base.chain_log - 5),
        (base.hash_log - 7, base.chain_log - 7),
    ] {
        let mut p = base;
        p.hash_log = hl;
        p.chain_log = cl;
        let mut arm = [f64::MAX; 2];
        let mut bytes = 0u64;
        for a in 0..2 {
            for _ in 0..n {
                let t = std::time::Instant::now();
                let mut b = 0u64;
                for (_, s) in &srcs {
                    b += rusty_zstd::compress_with_params(s, p, false).expect("c").len() as u64;
                }
                let el = t.elapsed().as_secs_f64();
                if el < arm[a] { arm[a] = el; bytes = b; }
            }
        }
        let mbps = total as f64 / (1 << 20) as f64 / arm[0];
        let spread = (arm[0].max(arm[1]) / arm[0].min(arm[1]) - 1.0) * 100.0;
        if hl == base.hash_log { b_bytes = bytes; b_mbps = mbps; }
        println!(
            "{:>6} {:>6} {:>9} {:>12} {:>8.2}% {:>9.1} {:>7.1}%",
            hl, cl,
            format!("{}+{}", tbl(hl), tbl(cl)),
            bytes,
            if b_bytes > 0 { 100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64 } else { 0.0 },
            mbps, spread
        );
    }
    println!(
        "\nbase = {b_mbps:.1} MB/s at the shipping table sizes.\n\
         If the bottom rows are MUCH faster, the encoder is memory-bound and the\n\
         instruction campaign has been optimising the wrong axis at this level."
    );
}
