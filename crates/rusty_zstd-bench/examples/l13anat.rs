//! WHERE L1 AND L3 SPEND THEIR ENCODE TIME -- the work profile inside MatchFind.
//!
//! `hotspot.rs` establishes that MatchFind is ~73% of encode at L1 and ~76% at
//! L3. It also reports probes/byte, and that number REFUTES the obvious reading:
//! 0.33 probes/byte at L1 and 0.07 at L3 cannot be three quarters of the time.
//! At L3 that is ~590k probes over 8 MiB; for those to carry 76% of encode each
//! probe would have to cost ~250 cycles, which a hash lookup does not.
//!
//! So the stage's cost is per-BYTE work, not per-probe work. This separates the
//! two: every counter below is normalised per INPUT BYTE, so a column that
//! tracks input size is per-byte work and a column that tracks match count is
//! per-probe work. The distinction decides where an optimisation can even help.
//!
//! `probes` counts HASH probes only -- a repcode match needs none, which is why
//! `seqs` can exceed `probes` on repetitive corpora. `rep` is reported beside it
//! so that is visible rather than confusing.
//!
//! Counts, not clocks: identical on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example l13anat

const IDS: &[&str] = &[
    "x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao", "dickens", "samba", "nci",
    "webster", "mozilla", "mr",
];

fn main() {
    let cap: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);

    for lvl in [1i32, 3] {
        println!("\n=== L{lvl} WORK PROFILE -- all columns PER INPUT BYTE ===");
        println!(
            "{:<13} {:>8} {:>8} {:>8} {:>9} {:>8} {:>8}  {:>8} {:>7}",
            "corpus", "probes", "fills", "seqs", "matchB", "litB", "backext", "pos/B", "mlen"
        );
        let (mut tp, mut tf, mut ts, mut tm, mut tl, mut tb, mut tn) =
            (0f64, 0f64, 0f64, 0f64, 0f64, 0f64, 0f64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            else {
                continue;
            };
            let s = &f[..f.len().min(cap)];
            // Best-of-N on the STAGE timers: they are noisy on a loaded host,
            // and the minimum is the cleanest estimator because contention can
            // only ever add time. Counters are re-read from the final run --
            // they are identical in every run by construction.
            let mut mf_best = f64::MAX;
            let mut et_best = f64::MAX;
            for _ in 0..7 {
                rusty_zstd::prof_reset();
                let _ = rusty_zstd::compress(s, lvl).expect("compress");
                let mf = rusty_zstd::prof_stage_ns(rusty_zstd::ProfStage::EncodeMatchFind) as f64;
                let et = rusty_zstd::prof_stage_ns(rusty_zstd::ProfStage::EncodeTotal) as f64;
                if et > 0.0 && et < et_best { et_best = et; mf_best = mf; }
            }
            let c = rusty_zstd::prof_encode_counts();
            let n = s.len() as f64;
            eprintln!("FIT	{}	{}	{:.6}	{:.6}	{:.6}	{:.6}	{:.6}	{:.6}	{:.6}",
                lvl, id,
                mf_best / n, et_best / n,
                c.hash_probes as f64 / n, c.hash_fills as f64 / n,
                c.match_bytes as f64 / n, c.lit_bytes as f64 / n,
                c.seqs as f64 / n);

            // pos/B = table positions TOUCHED per input byte: every hash probe
            // plus every hash fill. This is the per-position work the finder
            // does. If it is far below 1.0 while MatchFind dominates, the cost
            // is NOT per-position and no amount of cheaper probing will help.
            let pos = (c.hash_probes + c.hash_fills) as f64 / n;
            let mlen = if c.seqs > 0 { c.match_bytes as f64 / c.seqs as f64 } else { 0.0 };
            println!(
                "{:<13} {:>8.3} {:>8.3} {:>8.4} {:>9.3} {:>8.3} {:>8.4}  {:>8.3} {:>7.1}",
                id,
                c.hash_probes as f64 / n,
                c.hash_fills as f64 / n,
                c.seqs as f64 / n,
                c.match_bytes as f64 / n,
                c.lit_bytes as f64 / n,
                c.back_ext_bytes as f64 / n,
                pos,
                mlen
            );
            tp += c.hash_probes as f64 / n;
            tf += c.hash_fills as f64 / n;
            ts += c.seqs as f64 / n;
            tm += c.match_bytes as f64 / n;
            tl += c.lit_bytes as f64 / n;
            tb += c.back_ext_bytes as f64 / n;
            tn += 1.0;
        }
        if tn > 0.0 {
            println!(
                "{:<13} {:>8.3} {:>8.3} {:>8.4} {:>9.3} {:>8.3} {:>8.4}  {:>8.3}",
                "MEAN",
                tp / tn,
                tf / tn,
                ts / tn,
                tm / tn,
                tl / tn,
                tb / tn,
                (tp + tf) / tn
            );
        }
    }

    println!(
        "\nmatchB + litB should sum to ~1.0 -- together they are the whole input.\n\
         Whichever of those two is large is where the per-byte work lives:\n\
         matchB is match EXTENSION (count_match_len), litB is literal COPYING.\n\
         `pos/B` is the per-position work. Compare its magnitude to 1.0."
    );
}
