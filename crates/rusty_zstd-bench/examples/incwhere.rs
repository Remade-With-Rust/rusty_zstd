//! WHERE does incompressible data spend its time, by level?
//!
//! The head-to-head board reads 11.5-12.2x slower than C at L7/L9 on
//! `incomp-32m`. A 12x effect cannot be a 16% null -- load explains +-16%, not
//! +1100% -- so if the number is wrong it is wrong for a STRUCTURAL reason, not
//! a noise reason. The deterministic counters already ruled out search work
//! (0 chain loads at L7). This asks the stage profiler where it actually goes.
//!
//! Stage timers are distorted by their own instrumentation and are read ONLY as
//! shares, and only to rank -- but a 12x gap does not need precision to locate.
use rusty_zstd::ProfStage as S;
fn main() {
    let f = std::fs::read("corpora/data/generated/incomp-32m").expect("incomp-32m");
    let src = &f[..f.len().min(1 << 20)];
    println!("incompressible, {} KiB\n", src.len() >> 10);
    println!("{:>4}{:>10}{:>12}{:>9}{:>9}{:>9}{:>9}{:>9}",
             "L", "strategy", "total us", "tables", "find%", "entropy%", "huff%", "other%");
    println!("{}", "-".repeat(72));
    for tight in [0u32, 1] {
      println!("--- hash_tight = {tight} ---");
      for lvl in [1i32, 3, 5, 7, 9, 12] {
        rusty_zstd::set_hash_tight_arm(tight);
        let p = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
        // warm, then measure
        for _ in 0..3 { let _ = rusty_zstd::compress_with_params(src, p, false).unwrap(); }
        rusty_zstd::prof_reset();
        for _ in 0..5 { let _ = rusty_zstd::compress_with_params(src, p, false).unwrap(); }
        let tot = rusty_zstd::prof_stage_ns(S::EncodeTotal) as f64;
        let g = |s: S| rusty_zstd::prof_stage_ns(s) as f64 / tot * 100.0;
        let tbl = rusty_zstd::prof_stage_ns(S::EncodeTables) as f64 / tot * 100.0;
        let find = g(S::EncodeMatchFind);
        let ent = g(S::EncodeEntropy);
        let huff = g(S::EncodeHuff);
        println!("{:>4}{:>10}{:>12.0}{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%",
                 lvl, format!("{:?}", p.strategy), tot / 1000.0 / 5.0,
                 tbl, find, ent, huff, 100.0 - tbl - find - ent);
      }
    }
    rusty_zstd::set_hash_tight_arm(0);
    println!("\n(entropy% includes huff%; other% = 100 - tables - find - entropy)");
}
