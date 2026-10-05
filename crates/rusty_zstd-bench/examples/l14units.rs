//! THE MATCH-EVENT UNITS for L1-L4 (the Fast and DFast finders).
//!
//!   cargo run --release -p rusty_zstd-bench --features profile --example l14units -- [levels] [ids]
//!
//! Section 7 / 8.6 of docs/plans/m7-anatomy.md localised the L1/L3 encode cost
//! to the match EVENT and left its decomposition open. This prints the UNIT
//! half of that decomposition: how many of each unit of work run per input
//! byte -- positions scanned, events (split hash / repcode), count-match
//! calls, back-extended bytes, table fills, literal pushes by tier -- so each
//! can be multiplied by a price read off the emitted assembly.
//!
//! Counts, not clocks: every column is identical on any machine at any load.
//! The three stage columns at the end ARE clocks (thread-CPU, best of 3, from a
//! profile build whose per-position counters tax the match finder) and are
//! printed only as shares.
#[cfg(feature = "profile")]
const DEFAULT_IDS: &[&str] = &["dickens", "mozilla", "samba", "nci", "xml", "x-ray"];

fn main() {
    #[cfg(not(feature = "profile"))]
    {
        println!("needs --features profile");
    }
    #[cfg(feature = "profile")]
    {
        use rusty_zstd::ProfStage as S;
        let args: Vec<String> = std::env::args().skip(1).collect();
        let levels: Vec<i32> = args.iter().filter_map(|a| a.parse().ok()).collect();
        let levels = if levels.is_empty() { vec![1, 2, 3, 4] } else { levels };
        let ids: Vec<String> = args.iter().filter(|a| a.parse::<i32>().is_err()).cloned().collect();
        let ids: Vec<String> =
            if ids.is_empty() { DEFAULT_IDS.iter().map(|s| (*s).to_string()).collect() } else { ids };
        for lvl in levels {
            let strat = rusty_zstd::compression_params(lvl, None).map(|p| format!("{:?}", p.strategy)).unwrap_or_default();
            println!("\n=== L{lvl} ({strat}) -- units per KiB of input ===");
            println!(
                "{:<9} {:>7} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6} {:>7} {:>6} {:>6} {:>6} {:>6} {:>6} {:>6}  {:>5} {:>5}",
                "corpus", "pos", "cand", "seqs", "rep", "hash", "cnt", "bextM", "bextB", "fills", "lpF", "lpS", "litB",
                "l<=16", "l<=32", "l>32", "MF%", "ENT%"
            );
            for id in &ids {
                let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
                else {
                    continue;
                };
                let s = &f[..];
                let (mut mf_b, mut en_b, mut tot_b) = (f64::MAX, f64::MAX, f64::MAX);
                for _ in 0..3 {
                    rusty_zstd::prof_reset();
                    let _ = rusty_zstd::compress(s, lvl).expect("compress");
                    let t = rusty_zstd::prof_stage_ns(S::EncodeTotal) as f64;
                    if t < tot_b {
                        tot_b = t;
                        mf_b = rusty_zstd::prof_stage_ns(S::EncodeMatchFind) as f64;
                        en_b = rusty_zstd::prof_stage_ns(S::EncodeEntropy) as f64;
                    }
                }
                // drain everything, then ONE counted pass
                let _ = rusty_zstd::take_mm();
                let _ = rusty_zstd::take_rep_rate();
                let _ = rusty_zstd::take_dfast_match_stats();
                let _ = rusty_zstd::take_ff_waste();
                let _ = rusty_zstd::take_bext();
                let _ = rusty_zstd::take_dfast_bext();
                let _ = rusty_zstd::take_lp_stats();
                let _ = rusty_zstd::take_eqlen_stats();
                let _ = rusty_zstd::take_route_hist();
                let _ = rusty_zstd::take_ff_pipe();
                let _ = rusty_zstd::take_pair_stats();
                let _ = rusty_zstd::take_next_long();
                let _ = rusty_zstd::take_dfast_spec();
                let _ = rusty_zstd::take_lit_tiers();
                rusty_zstd::prof_reset();
                let z = rusty_zstd::compress(s, lvl).expect("compress");
                let c = rusty_zstd::prof_encode_counts();
                let (pos, _miss) = rusty_zstd::take_mm();
                let (_rp, _rb, rep_fast, _amb, _aseq) = rusty_zstd::take_rep_rate();
                let (_dmb, _dseq, _dbb, _drb, rep_dfast) = rusty_zstd::take_dfast_match_stats();
                let (cand4, _acc) = rusty_zstd::take_ff_waste();
                let (_bm, bext_n, bext_b, _ge8) = rusty_zstd::take_bext();
                let (dbext_b, dbext_m, _dbext_s) = rusty_zstd::take_dfast_bext();
                let (hist, lp_fast, lp_slow) = rusty_zstd::take_lp_stats();
                let (t2, t3) = rusty_zstd::take_lit_tiers();
                let (cnt_calls, _wide, _eh) = rusty_zstd::take_eqlen_stats();
                let (r0, r1, r2, _g, _y) = rusty_zstd::take_route_hist();
                let (pipe_blocks, ff_made, ff_used) = rusty_zstd::take_ff_pipe();
                let (pair_probes, pair_hits, _pb, _mb) = rusty_zstd::take_pair_stats();
                let (nl_probes, nl_hits, _nlg) = rusty_zstd::take_next_long();
                let (sp_made, sp_used) = rusty_zstd::take_dfast_spec();
                let k = s.len() as f64 / 1024.0;
                let dfast = strat == "DFast";
                let rep = if dfast { rep_dfast } else { rep_fast };
                let cand = if dfast { c.hash_probes } else { cand4 };
                let (bm, bb) = if dfast { (dbext_m, dbext_b) } else { (bext_n, bext_b) };
                let le16 = hist[0] + hist[1] + hist[2];
                let le32 = hist[3];
                let gt32 = hist[4] + hist[5];
                println!(
                    "{:<9} {:>7.1} {:>7.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>7.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1} {:>6.1}  {:>5.1} {:>5.1}",
                    id,
                    pos as f64 / k,
                    cand as f64 / k,
                    c.seqs as f64 / k,
                    rep as f64 / k,
                    (c.seqs - rep.min(c.seqs)) as f64 / k,
                    cnt_calls as f64 / k,
                    bm as f64 / k,
                    bb as f64 / k,
                    c.hash_fills as f64 / k,
                    lp_fast as f64 / k,
                    (lp_slow + t2 + t3) as f64 / k,
                    c.lit_bytes as f64 / k,
                    le16 as f64 / k,
                    le32 as f64 / k,
                    gt32 as f64 / k,
                    100.0 * mf_b / tot_b,
                    100.0 * en_b / tot_b,
                );
                if dfast {
                    println!(
                        "          nl_probes {:.1} nl_hits {:.1} spec_made {:.1} spec_used {:.1} (per KiB)   bytes {} -> {}   matchB/seq {:.1}",
                        nl_probes as f64 / k,
                        nl_hits as f64 / k,
                        sp_made as f64 / k,
                        sp_used as f64 / k,
                        s.len(),
                        z.len(),
                        c.match_bytes as f64 / c.seqs.max(1) as f64,
                    );
                } else {
                    println!(
                        "          routes {r0}/{r1}/{r2} pipe_blocks {pipe_blocks} spec {:.1}/{:.1} pair_probes {:.1} pair_hits {:.1} (per KiB)   bytes {} -> {}   matchB/seq {:.1}",
                        ff_made as f64 / k,
                        ff_used as f64 / k,
                        pair_probes as f64 / k,
                        pair_hits as f64 / k,
                        s.len(),
                        z.len(),
                        c.match_bytes as f64 / c.seqs.max(1) as f64,
                    );
                }
            }
        }
        println!(
            "\npos   positions scanned (loop-top count, both finders)\n\
             cand  Fast: candidates passing the 4-byte gate; DFast: tag-filter survivors examined\n\
             seqs  = rep + hash events;  cnt = out-of-line count_match calls\n\
             bextM/bextB  events that back-extend / bytes walked;  fills = table stores after a match\n\
             lpF/lpS  literal pushes on the fixed-width path / the outlined tiers+fallback\n\
             l<=16 / l<=32 / l>32  literal-run length histogram, runs per KiB"
        );
    }
}
