//! Function-level MatchFind anatomy: per level, the finder that serves it,
//! MatchFind's share of encode, and its absolute time per input MiB.
//! Shares come from the profile scopes and are comparable only within a run.
use rusty_zstd::ProfStage as S;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
    "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = 8 << 20;
    println!("| level | strategy | finder | MF ns/MiB | MF % of encode | encode ns/MiB |");
    println!("| ----: | -------- | ------ | --------: | -------------: | ------------: |");
    for lvl in [1i32, 3, 5, 7, 9, 12, 13, 15, 16, 19, 22] {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        let strat = format!("{:?}", p.strategy);
        let (mut mf, mut et, mut mb) = (0f64, 0f64, 0f64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::prof_reset();
            let _ = rusty_zstd::compress(s, lvl).unwrap();
            mf += rusty_zstd::prof_stage_ns(S::EncodeMatchFind) as f64;
            et += rusty_zstd::prof_stage_ns(S::EncodeTotal) as f64;
            mb += s.len() as f64 / 1_048_576.0;
        }
        let finder = match strat.as_str() {
            "Fast" => "find_fast_impl (48 base + 8 twin)",
            "DFast" => "find_dfast_impl (5 base + 1 twin)",
            "Greedy" => "find_greedy + chain_find_best",
            "Lazy" | "Lazy2" => "find_lazy + chain_find_best",
            "BtLazy2" => "find_bt_lazy + bt ladder",
            _ => "find_opt + bt ladder",
        };
        println!("| L{lvl} | {strat} | {finder} | {:.0} | {:.1} | {:.0} |",
            mf / mb, mf / et * 100.0, et / mb);
    }
}
