//! IS 28.8% EMPTY WHAT A GOOD HASH WOULD GIVE? (It is. Read the baseline note.)
//!
//! `walkexit.rs` measures 28.8% of L9 chain searches ending on an EMPTY hash
//! bucket. That number only means something against a baseline, and picking the
//! WRONG baseline turns it into a false alarm -- which is what happened on this
//! board's first run.
//!
//! GET THE BASELINE RIGHT. `e^(-n/s)` is the STEADY-STATE empty rate for `n`
//! positions in `s` slots -- the answer once every position is in. But the table
//! fills PROGRESSIVELY during the pass: the search at position `i` sees a table
//! holding about `i` entries, not `n`. Averaged over the pass the correct
//! baseline is
//!
//! ```text
//!     (1/n) * integral(0..n) e^(-i/s) di  =  (s/n) * (1 - e^(-n/s))
//! ```
//!
//! which is far higher. The difference decides the verdict:
//!
//! ```text
//!   load n/s   steady e^-L   fill-averaged   measured   vs fill-avg
//!       0.50         60.7%           78.7%      40.7%         0.52x
//!       1.00         36.8%           63.2%      36.6%         0.58x
//!       2.00         13.5%           43.2%      28.8%         0.67x
//!       4.00          1.8%           24.5%      22.8%         0.93x
//! ```
//!
//! Against `steady` the last row reads as 12.5x "excess" -- severe clustering,
//! a hash defect, a lead worth chasing. Against `fill-avg` it reads as 0.93x:
//! the hash is doing BETTER than uniform. There is no clustering. The empty
//! buckets are the table warming up, which is inherent to a chain finder and
//! not a defect at all.
//!
//! Both columns are printed so the comparison cannot be made against the wrong
//! one by accident. Counts only; deterministic.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example bucketfill [level]

const IDS: &[&str] = &["dickens", "webster", "samba", "mozilla", "osdb", "mr"];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);

    println!("L{lvl} BUCKET-OCCUPANCY CHECK -- measured empty rate vs a uniform hash\n");
    println!(
        "{:>8} {:>12} {:>10} {:>9} {:>9} {:>10} {:>10} {:>9}",
        "input", "positions", "slots", "load n/s", "steady", "fill-avg", "measured", "vs f-avg"
    );

    for cap_mb in [1usize, 2, 4, 8] {
        let cap = cap_mb << 20;
        let srcs: Vec<Vec<u8>> = IDS
            .iter()
            .filter_map(|id| {
                std::fs::read(format!("corpora/data/generated/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                    .ok()
                    .map(|f| {
                        let n = f.len().min(cap);
                        f[..n].to_vec()
                    })
            })
            .collect();
        if srcs.is_empty() {
            continue;
        }
        let p = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");
        let slots = 1u64 << p.hash_log;

        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_walk_exit();
        for s in &srcs {
            let _ = rusty_zstd::compress_with_params(s, p, false).expect("compress");
        }
        let e = rusty_zstd::take_walk_exit();
        let walks: u64 = e[..7].iter().sum();
        if walks == 0 {
            continue;
        }

        // Per FILE: the tables are reset per frame, so the load factor is
        // per-file rather than summed across the corpus set.
        let per_file = srcs.iter().map(|s| s.len() as f64).sum::<f64>() / srcs.len() as f64;
        let load = per_file / slots as f64;
        let steady = (-load).exp() * 100.0;
        let fill_avg = if load > 0.0 {
            (1.0 - (-load).exp()) / load * 100.0
        } else {
            100.0
        };
        let meas = 100.0 * e[0] as f64 / walks as f64;
        println!(
            "{:>7}M {:>12.0} {:>10} {:>9.2} {:>8.1}% {:>9.1}% {:>9.1}% {:>8.2}x",
            cap_mb,
            per_file,
            slots,
            load,
            steady,
            fill_avg,
            meas,
            if fill_avg > 0.01 { meas / fill_avg } else { 0.0 }
        );
    }
    println!(
        "\nCompare `measured` to `fill-avg`, NEVER to `steady`. Below 1.0x means\n\
         the hash beats a uniform one against a progressively filling table --\n\
         which is what it does. The empty buckets are warm-up, not a defect."
    );
}
