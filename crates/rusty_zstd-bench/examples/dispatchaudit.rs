//! WHICH DISPATCH BARS STILL DISCRIMINATE?
//!
//! A runtime bar is a DECISION: "apply this arm only where the measured signal
//! is under `X`". Like the inline/twin decisions `tools/premise_audit.py`
//! audits, a bar's justification can expire -- but for a different reason. A
//! twin expires when its BODY moves. A bar expires when its SIGNAL moves, or
//! when the axis it was adjudicated on stops being the only one that matters.
//!
//! `walk_rep_max` is the worked example. It gates the deep chain walk on
//! `rep_yield <= 0.10`, and at L9 tightening it to 0.05 / 0.02 / 0.01 moves the
//! probe count by 0.9% / 2.1% / 2.4%: the signal never approaches the bar, so
//! the condition is a CONSTANT with extra steps. Its sibling `walk_first_max`
//! is the opposite -- moving it 0.70 -> 0.55 cuts probes 19.1%.
//!
//! Both were adjudicated on RATIO alone. One turned out to be the largest speed
//! lever in the encoder; the other does nothing. There was no way to tell them
//! apart without this test.
//!
//! THE TEST. Set each bar to both extremes and compress the same input. If the
//! compressed bytes AND the probe count are IDENTICAL at 0.0 and at 1.0, the
//! bar never fires at this level -- the branch it guards always goes one way.
//! That is not necessarily wrong (it may be load-bearing at another level, or
//! on content not in this corpus set) but it is a decision no longer doing the
//! job it was added for, and it should be re-adjudicated rather than trusted.
//!
//! Fully deterministic -- bytes and counters only, no clock.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example dispatchaudit [level]

const IDS: &[&str] = &["dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml"];

type Setter = (&'static str, fn(f32));

fn bars() -> Vec<Setter> {
    vec![
        ("lazy_fill_threshold", rusty_zstd::set_lazy_fill_threshold_arm as fn(f32)),
        ("nl_off_worse", rusty_zstd::set_nl_off_worse_arm),
        ("step_forfeit", rusty_zstd::set_step_forfeit_arm),
        // `step_seq` was a row here until its knob was removed -- the setter stored
        // into a static nothing read, so sweeping it to both extremes moved nothing.
        // That is exactly the finding this board exists to make, so it is recorded
        // rather than silently dropped.
        ("lit_short", rusty_zstd::set_lit_short_arm),
        ("dfast_spec_min", rusty_zstd::set_dfast_spec_min_arm),
        ("bt_deep_min", rusty_zstd::set_bt_deep_min_arm),
        ("walk_rep_max", rusty_zstd::set_walk_rep_max_arm),
        ("walk_first_max", rusty_zstd::set_walk_first_max_arm),
        ("wide_first_max", rusty_zstd::set_wide_first_max_arm),
        ("wide_spb_min", rusty_zstd::set_wide_spb_min_arm),
        ("pair_lo", rusty_zstd::set_pair_lo_arm),
        ("pair_hi", rusty_zstd::set_pair_hi_arm),
        ("pair_gain", rusty_zstd::set_pair_gain_arm),
    ]
}

fn measure(srcs: &[Vec<u8>], lvl: i32) -> (u64, u64) {
    rusty_zstd::prof_reset();
    let mut bytes = 0u64;
    for s in srcs {
        bytes += rusty_zstd::compress(s, lvl).expect("compress").len() as u64;
    }
    (bytes, rusty_zstd::prof_encode_counts().hash_probes)
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(2 << 20);

    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let k = f.len().min(cap);
                    f[..k].to_vec()
                })
        })
        .collect();

    println!(
        "DISPATCH BAR AUDIT @ L{lvl} -- {} corpora. Does each bar still fire?\n",
        srcs.len()
    );
    println!(
        "{:>22} {:>13} {:>13} {:>11} {:>10}  verdict",
        "bar", "bytes @ 0.0", "bytes @ 1.0", "probes swing", "size swing"
    );

    let mut inert = Vec::new();
    let mut live = Vec::new();
    for (name, set) in bars() {
        set(0.0);
        let (b_lo, p_lo) = measure(&srcs, lvl);
        set(1.0);
        let (b_hi, p_hi) = measure(&srcs, lvl);
        // Restore: u32::MAX is the "unset" sentinel every one of these reads.
        set(f32::from_bits(u32::MAX));

        let dp = if p_lo > 0 {
            100.0 * (p_hi as f64 - p_lo as f64) / p_lo as f64
        } else {
            0.0
        };
        let db = if b_lo > 0 {
            100.0 * (b_hi as f64 - b_lo as f64) / b_lo as f64
        } else {
            0.0
        };
        let dead = b_lo == b_hi && p_lo == p_hi;
        if dead {
            inert.push(name);
        } else {
            live.push(name);
        }
        println!(
            "{:>22} {:>13} {:>13} {:>10.2}% {:>9.3}%  {}",
            name,
            b_lo,
            b_hi,
            dp,
            db,
            if dead { "INERT at this level" } else { "live" }
        );
    }

    println!("\n  live at L{lvl}:  {}", live.join(", "));
    println!("  INERT at L{lvl}: {}", if inert.is_empty() { "none".into() } else { inert.join(", ") });
    println!(
        "\nINERT means both extremes produced byte-identical output AND an\n\
         identical probe count -- the guarded branch always goes the same way\n\
         here. Check the other levels before concluding a bar is dead: several\n\
         of these are strategy-scoped by construction."
    );
}
