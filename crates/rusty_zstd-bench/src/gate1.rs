//! GATE 1 truth table — `params.strategy`, all 18 corpora, every strategy.
//!
//! The point of this file is a comparison that the standing boards CANNOT make.
//! Comparing L1 to L3 to L5 does not isolate the strategy: it moves
//! `window_log`, `chain_log`, `hash_log`, `search_log`, `min_match` and
//! `target_length` at the same time. `set_strategy_arm` overrides ONLY the
//! strategy at the single point params are derived, so every column below runs
//! the same level parameters with a different match finder.
//!
//! Great Gate law §4: force-on-everywhere must nearly tie the anchor before a
//! dispatch is built on it. This table IS that force-on test, per corpus.

use crate::corpus::GeneratedFile;
use rusty_zstd::Strategy;
use std::process::ExitCode;

const STRATS: &[(&str, Strategy)] = &[
    ("fast", Strategy::Fast),
    ("dfast", Strategy::DFast),
    ("greedy", Strategy::Greedy),
    ("lazy", Strategy::Lazy),
    ("lazy2", Strategy::Lazy2),
    ("btlazy2", Strategy::BtLazy2),
    ("btopt", Strategy::BtOpt),
];

pub fn run(files: &[GeneratedFile], level: i32, only: &[String], out: &std::path::Path) -> ExitCode {
    use std::io::Write;
    let mut w = match std::fs::File::create(out) {
        Ok(f) => std::io::BufWriter::new(f),
        Err(e) => {
            eprintln!("create {}: {e}", out.display());
            return ExitCode::from(1);
        }
    };
    let _ = writeln!(w, "clip,split,level,strategy,size,probes,ms,ratio_vs_level_default");
    println!("GATE 1 truth table — strategy held against every other param at L{level}");
    println!(
        "{:<14}{:>10}{:>10}{:>10}{:>10}{:>10}{:>10}{:>10}",
        "clip", "fast", "dfast", "greedy", "lazy", "lazy2", "btlazy2", "btopt"
    );
    println!("{}", "-".repeat(94));

    for f in files {
        if !only.is_empty() && !only.iter().any(|o| o == &f.id) {
            continue;
        }
        let Ok(src) = std::fs::read(&f.path) else {
            continue;
        };
        // Baseline: the level's own strategy, no override.
        rusty_zstd::set_strategy_arm(None);
        let base = match rusty_zstd::compress(&src, level) {
            Ok(v) => v.len(),
            Err(e) => {
                eprintln!("{} baseline: {e:?}", f.id);
                return ExitCode::from(1);
            }
        };
        let mut row = format!("{:<14}", f.id);
        for (name, st) in STRATS {
            rusty_zstd::set_strategy_arm(Some(*st));
            rusty_zstd::prof_reset();
            let t0 = std::time::Instant::now();
            let z = match rusty_zstd::compress(&src, level) {
                Ok(v) => v,
                Err(e) => {
                    eprintln!("{} {name}: {e:?}", f.id);
                    rusty_zstd::set_strategy_arm(None);
                    return ExitCode::from(1);
                }
            };
            let ms = t0.elapsed().as_secs_f64() * 1000.0;
            let c = rusty_zstd::prof_encode_counts();
            // Round-trip every arm: a strategy override must not change legality.
            match rusty_zstd::decompress(&z) {
                Ok(back) if back == src => {}
                _ => {
                    eprintln!("{} {name}: ROUND-TRIP FAILED", f.id);
                    rusty_zstd::set_strategy_arm(None);
                    return ExitCode::from(3);
                }
            }
            let rel = 100.0 * (z.len() as f64 - base as f64) / base as f64;
            row.push_str(&format!("{rel:>9.2}%"));
            let _ = writeln!(
                w,
                "{},{},{},{},{},{},{:.3},{:.4}",
                f.id,
                f.split,
                level,
                name,
                z.len(),
                c.hash_probes,
                ms,
                rel
            );
        }
        rusty_zstd::set_strategy_arm(None);
        println!("{row}");
    }
    let _ = w.flush();
    println!("\n(each cell = size vs this level's DEFAULT strategy; negative = smaller)");
    println!("gate1 truth table {}", out.display());
    ExitCode::SUCCESS
}
