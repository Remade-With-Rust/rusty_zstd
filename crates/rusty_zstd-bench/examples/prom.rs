//! PROMETHEUS ADJUDICATION of the fitted gate constants.
//!
//! No `Prometheus/` workspace exists in this repo, so the full refinery
//! (harvest -> symreg -> SMT -> forge) is not available. What IS available, and
//! is the prerequisite for any of it, is the question the refinery asks first:
//! **is this fitted constant LIVE, INERT, or MIS-FITTED?**
//!
//! For each candidate we sweep its value and total the compressed bytes over the
//! board. A constant whose sweep never moves output is INERT -- dead
//! configuration surface, and the campaign's own precedent is to REMOVE it
//! (`OPT_SKIP_FLOOR`, "built, measured inert, and REMOVED"). One whose optimum
//! is not the shipped value is MIS-FITTED and worth the refinery's attention.
const IDS: &[&str] = &["dickens","samba","xml","nci","mozilla","x-ray","sao","webster"];

fn board(lvl: i32, cap: usize) -> u64 {
    let mut t = 0u64;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        t += rusty_zstd::compress(s, lvl).unwrap().len() as u64;
    }
    t
}

fn sweep(name: &str, lvl: i32, shipped: f32, vals: &[f32], set: impl Fn(f32)) {
    let cap = 4 << 20;
    set(shipped);
    let base = board(lvl, cap);
    let mut best = (shipped, base);
    let mut moved = 0usize;
    let mut line = String::new();
    for &v in vals {
        set(v);
        let b = board(lvl, cap);
        if b != base { moved += 1 }
        if b < best.1 { best = (v, b) }
        line.push_str(&format!("{v}={:+.4}%  ", (b as f64 / base as f64 - 1.0) * 100.0));
    }
    set(shipped);
    let verdict = if moved == 0 {
        "INERT -- sweep moves NOTHING".to_string()
    } else if (best.0 - shipped).abs() > f32::EPSILON {
        format!("MIS-FITTED -- {} beats {shipped} by {:.4}%", best.0, (1.0 - best.1 as f64 / base as f64) * 100.0)
    } else {
        format!("LIVE and best at the shipped {shipped}")
    };
    println!("\n{name}  (L{lvl}, shipped {shipped})\n   {line}\n   => {verdict}  [{moved}/{} values moved output]", vals.len());
}

fn main() {
    println!("PROMETHEUS ADJUDICATION -- are the fitted constants live, inert, or mis-fitted?");
    sweep("pair_gain_lo", 1, 0.71, &[0.3, 0.5, 0.6, 0.8, 0.9, 1.2], rusty_zstd::set_pair_lo_arm);
    sweep("pair_rate_hi", 1, 1.0, &[0.5, 0.75, 1.5, 2.0, 4.0], rusty_zstd::set_pair_hi_arm);
    sweep("pair_gain_min", 1, 1.0, &[0.25, 0.5, 2.0, 4.0, 8.0], rusty_zstd::set_pair_gain_arm);
    sweep("dfast_spec_min", 3, 0.5, &[0.0, 0.25, 0.75, 0.9, 1.0], rusty_zstd::set_dfast_spec_min_arm);
    sweep("pair_rep_max", 1, 0.7, &[0.2, 0.4, 0.9, 1.5, 3.0], |v| std::env::set_var("RZSTD_PAIR_T", v.to_string()));
    sweep("rep_yield_min", 1, 0.10, &[0.0, 0.05, 0.2, 0.5, 1.0], |v| std::env::set_var("RZSTD_REPMIN", v.to_string()));
    sweep("tag_min", 1, 0.50, &[0.0, 0.25, 0.75, 0.9, 1.0], |v| std::env::set_var("RZSTD_TAG_T", v.to_string()));
    sweep("G5_REP_MIN (L3 ladder)", 3, 0.30, &[0.0, 0.15, 0.5, 1.0, 2.0],
          |v| rusty_zstd::set_g5_arms(v, 0.70, 1.50));
}
