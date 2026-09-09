//! SIZE BOARD for the arms that live in the finders no census could see.
//!
//!   cargo run --release -p rusty_zstd-bench --example lazyboard
//!
//! `allgates` covered levels [1, 3, 19, 22] = Fast, DFast, BtUltra2 until this
//! session. Every arm whose call sites are in `find_greedy`, `find_lazy_impl`
//! or `find_bt_lazy` has therefore NEVER been size-boarded -- `lazy_fill` was
//! the first one checked and it had been reported dead for the whole campaign
//! while being worth 266,695 bytes.
//!
//! DISCIPLINE, both learned the hard way earlier today:
//!  * the BASELINE is measured FIRST, before any setter is called, because the
//!    f32/usize arms cache raw bits with a sentinel and their public setters
//!    cannot restore "unset" -- `set_pair_hi_arm(-1.0)` PINS -1.0, and a sweep
//!    that used it as a reset read a 6x inflated win.
//!  * both arms of every bool are measured against that baseline, so which one
//!    is the shipped default is DERIVED, never assumed.
use rusty_zstd as rz;
const IDS: &[&str] = &[
    "jsonlog-16m",
    "smallmsg-8m",
    "mr",
    "ooffice",
    "osdb",
    "reymont",
    "sao",
    "webster",
    "dickens",
    "mozilla",
    "nci",
    "samba",
    "xml",
    "x-ray",
];
const LEVELS: &[(i32, &str)] = &[
    (5, "Greedy"),
    (7, "Lazy"),
    (9, "Lazy2"),
    (13, "BtLazy2"),
    (16, "BtOpt"),
];
fn main() {
    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(1 << 20);
                    f[..n].to_vec()
                })
        })
        .collect();
    let go = |lvl: i32| -> usize {
        srcs.iter()
            .map(|s| {
                rz::compress_with(
                    s,
                    rz::CompressOptions {
                        level: lvl,
                        checksum: false,
                    },
                )
                .unwrap()
                .len()
            })
            .sum()
    };
    type S = fn(bool);
    let arms: &[(&str, S)] = &[
        ("lazy_fill", rz::set_lazy_fill_arm as S),
        ("lazy_gain", rz::set_lazy_gain_arm),
        ("row", rz::set_row_arm),
        ("walk_cont", rz::set_walk_cont_arm),
        ("rep_reprobe", rz::set_rep_reprobe_arm),
        ("chain_tag", rz::set_chain_tag_arm),
        ("wide_chain", rz::set_wide_chain_arm),
        ("prime_bt", rz::set_prime_bt_arm),
        ("prime_bt_tree", rz::set_prime_bt_tree_arm),
        ("step_probe", rz::set_step_probe_arm),
        ("replen_pipe", rz::set_replen_pipe_arm),
        ("raw_skip", rz::set_raw_skip_arm),
    ];
    // BASELINE FIRST -- nothing has been set yet in this process.
    let base: Vec<usize> = LEVELS.iter().map(|(l, _)| go(*l)).collect();
    println!("baseline (untouched defaults):");
    for (i, (l, s)) in LEVELS.iter().enumerate() {
        println!("   L{l:<3}{s:<9}{}", base[i]);
    }
    println!(
        "\n{:<16}{:>4}{:>12}{:>12}   verdict",
        "arm", "L", "d(on)", "d(off)"
    );
    println!("{}", "-".repeat(62));
    for (name, set) in arms {
        for (i, (lvl, strat)) in LEVELS.iter().enumerate() {
            set(true);
            let on = go(*lvl) as i64 - base[i] as i64;
            set(false);
            let off = go(*lvl) as i64 - base[i] as i64;
            if on == 0 && off == 0 {
                continue;
            } // inert here
            let win = on.min(off);
            let tag = if win < 0 {
                format!("WIN {win:+} B ({})", if on < off { "on" } else { "off" })
            } else {
                format!("default already best ({strat})")
            };
            println!("{:<16}{:>4}{:>+12}{:>+12}   {tag}", name, lvl, on, off);
        }
        // restore: whichever arm equals the baseline at the first live level
        set(true);
        let a = go(LEVELS[2].0) as i64 - base[2] as i64;
        if a != 0 {
            set(false);
        }
    }
}
