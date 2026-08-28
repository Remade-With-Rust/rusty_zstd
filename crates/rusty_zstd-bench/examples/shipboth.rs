//! THE SHIP BOARD for the two unflipped defaults, measured TOGETHER.
//!
//! Both change the bitstream, so both are boarded on size before they ship, and
//! both are boarded on the CURRENT tree rather than on the numbers that were
//! measured when each was first found.
//!
//!   * `dfast_bext` -- DFast back-extends its matches, as C's
//!     `ZSTD_compressBlock_doubleFast` does and `emit_fast_seq_body` already
//!     does on the Fast ladder. Pure size win, costs work. Bites at L3/L4.
//!   * `walk_first` -- the C-parity chain walk's first-find bar, tightened by
//!     0.15 from its shipping ladder (0.80/0.70/0.55 by attempts). Trades size
//!     for a large probe reduction. Bites at L5+.
//!
//! They act on DISJOINT ladders, so the "both" arm should equal the sum of the
//! two singles. It is measured anyway -- an interaction is exactly the kind of
//! thing that a per-level board catches and a per-feature one does not.
//!
//! SIZE and probe counts are deterministic and are the ship criteria. There is
//! no clock in this board at all; speed is `firstbar.rs`, which interleaves.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example shipboth

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
    "x-ray", "reymont", "jsonlog",
];

/// The shipping ladder from `walk_first_max`, keyed by the level's actual
/// `search_log` attempts (clevels.h): L5=8, L7/L9=16, L12+=64.
fn ships(lvl: i32) -> f32 {
    match lvl {
        l if l <= 5 => 0.80,
        l if l <= 9 => 0.70,
        _ => 0.55,
    }
}

fn arm(bext: bool, walk: bool, lvl: i32) {
    rusty_zstd::set_dfast_bext_arm(bext);
    if walk {
        rusty_zstd::set_walk_first_max_arm((ships(lvl) - 0.15).max(0.30));
    } else {
        // u32::MAX is the sentinel for "unset" -- take the shipping ladder.
        rusty_zstd::set_walk_first_max_arm(f32::from_bits(u32::MAX));
    }
}

fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);

    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect();
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();

    println!(
        "SHIP BOARD -- {} corpora, {:.1} MiB. Deterministic size + probes.\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64
    );
    println!(
        "{:>5} {:>10} {:>13} {:>11} {:>11} {:>11} {:>11} {:>11}",
        "level", "strategy", "base bytes", "bext%", "walk%", "both%", "probes%", "worst corpus"
    );

    // Per-corpus regression tracking for the combined arm.
    let mut worst_any: Vec<(String, f64)> = Vec::new();

    for lvl in [1i32, 2, 3, 4, 5, 7, 9, 12, 15, 19] {
        let p = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");

        let mut bytes = [0u64; 4];
        let mut probes = [0u64; 4];
        let mut per: [Vec<u64>; 4] = [vec![], vec![], vec![], vec![]];

        for (i, (b, w)) in [(false, false), (true, false), (false, true), (true, true)]
            .iter()
            .enumerate()
        {
            arm(*b, *w, lvl);
            rusty_zstd::prof_reset();
            for (id, s) in &srcs {
                let z = rusty_zstd::compress(s, lvl).expect("compress");
                assert_eq!(&rusty_zstd::decompress(&z).expect("rt")[..], &s[..], "{id} L{lvl}");
                bytes[i] += z.len() as u64;
                per[i].push(z.len() as u64);
            }
            probes[i] = rusty_zstd::prof_encode_counts().hash_probes;
        }

        let pc = |x: u64, b: u64| 100.0 * (x as f64 - b as f64) / b as f64;

        // Worst single corpus under the combined arm.
        let mut worst = ("", 0.0f64);
        for (k, (id, _)) in srcs.iter().enumerate() {
            let d = pc(per[3][k], per[0][k]);
            if d > worst.1 {
                worst = (id, d);
            }
        }
        worst_any.push((format!("L{lvl} {}", worst.0), worst.1));

        println!(
            "{:>5} {:>10} {:>13} {:>10.3}% {:>10.3}% {:>10.3}% {:>10.1}% {:>7} {:+.2}%",
            lvl,
            format!("{:?}", p.strategy),
            bytes[0],
            pc(bytes[1], bytes[0]),
            pc(bytes[2], bytes[0]),
            pc(bytes[3], bytes[0]),
            pc(probes[3], probes[0]),
            worst.0,
            worst.1
        );
    }

    // Restore shipping arms so nothing downstream inherits a swept value.
    rusty_zstd::set_walk_first_max_arm(f32::from_bits(u32::MAX));

    println!(
        "\n`bext%` and `walk%` are each feature ALONE; `both%` is them together.\n\
         If both% != bext% + walk% at a level, the two interact and must be\n\
         adjudicated as one change rather than two.\n\
         `worst corpus` is the largest single-corpus size REGRESSION under the\n\
         combined arm -- the number that decides whether this ships."
    );
}
