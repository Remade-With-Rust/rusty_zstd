//! REACHABILITY PROBE — is a gate's code path actually entered at this level?
//!
//! A size-based "0 of 18 changed" cannot answer this on its own: a gate that is
//! byte-identical by construction (const-generic specialisation) reads as 0/18
//! whether it ran or not. This forces the finder with `set_strategy_arm` so the
//! selectors are exercised at THIS level's parameters, which separates
//! "not reached" from "reached but byte-neutral".
use rusty_zstd::Strategy;

fn size(src: &[u8], lvl: i32, forced: Option<Strategy>) -> usize {
    rusty_zstd::set_strategy_arm(forced);
    let n = rusty_zstd::compress(src, lvl).unwrap().len();
    rusty_zstd::set_strategy_arm(None);
    n
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let ids = ["xml", "osdb", "nci", "webster", "mr", "sao"];
    println!("REACHABILITY @ L{lvl}: do find_fast's selectors move output?");
    println!("  arm A = level default          (find_dfast at L3)");
    println!("  arm B = strategy FORCED Fast   (find_fast at L3's params)\n");
    for &(env_k, env_v) in &[("RZSTD_STEP0", "1"), ("RZSTD_MF_PIPE", "0")] {
        let mut def_moved = 0;
        let mut forced_moved = 0;
        for id in ids {
            let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
                continue;
            };
            let src = &full[..full.len().min(4 * 1024 * 1024)];
            // arm A: level default, selector off then on
            std::env::remove_var(env_k);
            let a0 = size(src, lvl, None);
            std::env::set_var(env_k, env_v);
            rusty_zstd::reset_env_arms();
            let a1 = size(src, lvl, None);
            std::env::remove_var(env_k);
            rusty_zstd::reset_env_arms();
            if a0 != a1 {
                def_moved += 1;
            }
            // arm B: find_fast FORCED, same selector toggle
            let b0 = size(src, lvl, Some(Strategy::Fast));
            std::env::set_var(env_k, env_v);
            rusty_zstd::reset_env_arms();
            let b1 = size(src, lvl, Some(Strategy::Fast));
            std::env::remove_var(env_k);
            rusty_zstd::reset_env_arms();
            if b0 != b1 {
                forced_moved += 1;
            }
        }
        println!(
            "  {env_k}={env_v}\n    default finder : {def_moved}/{} moved\n    forced Fast    : {forced_moved}/{} moved",
            ids.len(),
            ids.len()
        );
    }
    println!(
        "\nIf 'forced Fast' moves and 'default' does not, find_fast is NOT REACHED\n\
         at this level -- the selectors work, they simply have no caller."
    );
}
