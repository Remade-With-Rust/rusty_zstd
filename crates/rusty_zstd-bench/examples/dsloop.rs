//! Breaking the DecSeq LOOP open by function -- the part `dsanat.rs` reports as
//! a single 97-99% bar.
//!
//! THE METHOD, AND WHY IT IS NOT A CLOCK. One sequence costs ~34 ns; an
//! `Instant` pair costs 74.8 ns, so nothing inside the loop can be timed
//! directly. Instead each op is executed K extra times per sequence and then
//! UNDONE (bit-reader state restored, `out` truncated, `reps` restored), so:
//!
//!   * every arm produces BYTE-IDENTICAL output -- asserted here on every arm,
//!     every corpus, so a mis-built arm cannot silently become a fast arm;
//!   * the arm's delta over baseline prices K executions of that op, measured
//!     over hundreds of thousands of sequences rather than one.
//!
//! TWO CORRECTIONS THIS INSTRUMENT CARRIES, both found by impossible numbers:
//!
//!  1. NEGATIVE COSTS. The first version measured the baseline ONCE and then all
//!     seven arms. On `dickens` and `mr` the baseline landed 35% high and every
//!     arm came out negative -- drift between base and arm was being charged to
//!     the arm. Fixed by ABBA interleaving: each arm is paired with its own
//!     baseline in an A-B-B-A round, and the reported delta is the MEDIAN over
//!     rounds, so a single bad sample cannot set the number.
//!  2. A ~0 FLOOR ON THE CHEAP OPS. At K=1 the entropy primitives measured
//!     +-0.5 ns, i.e. nothing: a dependency-free duplicate of a few-instruction
//!     op hides in spare superscalar issue slots. Raising K lifts the signal
//!     above the floor. It does NOT make the bias vanish -- see below.
//!
//! THE KNOWN BIAS, STATED UP FRONT: the duplicate runs on data the real call
//! just touched, so it is CACHE-WARM and its loads are cheaper than the
//! original's. Every number here is a LOWER BOUND, and the ladder's coverage
//! (sum of attributed ns vs the measured loop) is printed so the gap is visible
//! instead of assumed.
//!
//! Usage: dsloop [level] [rounds] [K]
use rusty_zstd::ProfStage as S;

const IDS: &[&str] = &[
    "reymont", "dickens", "webster", "mr", "smallmsg-8m", "jsonlog-16m", "nci", "samba", "osdb",
    "xml", "mozilla", "ooffice", "sao",
];

/// (arm, label, executions of the op per K)
const ARMS: &[(u8, &str, f64)] = &[
    (7, "copy_match", 1.0),
    (5, "copy_literals", 1.0),
    (6, "resolve_offset", 1.0),
    (3, "FseTable::advance x3", 3.0),
    (1, "FseTable::entry x3", 3.0),
    (2, "BitRev::read_bits x3", 3.0),
    (4, "BitRev::reload x2", 2.0),
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok()
}

/// One timed decode of `z` on the currently selected arm, round-trip asserted.
fn once(z: &[u8], expect: &[u8], arm: u8) -> f64 {
    rusty_zstd::prof_reset();
    let out = rusty_zstd::decompress(z).unwrap();
    assert!(
        out == expect,
        "arm {arm}: OUTPUT CHANGED -- the duplicate was not undone"
    );
    rusty_zstd::prof_stage_ns(S::DecSeqLoop) as f64
}

fn med(v: &mut [f64]) -> f64 {
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n == 0 {
        return 0.0;
    }
    if n % 2 == 1 {
        v[n / 2]
    } else {
        0.5 * (v[n / 2 - 1] + v[n / 2])
    }
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let rounds: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(7);
    let k: u8 = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(4);
    let cap: usize = 8 << 20;

    println!("DecSeq LOOP anatomy @ L{lvl} -- ABBA duplication ladder, K={k}, median of {rounds} rounds\n");
    println!("Every arm asserted byte-identical. Costs are LOWER BOUNDS (cache-warm duplicate).\n");

    let mut tot_base = 0f64;
    let mut tot_seq = 0u64;
    let mut tot = vec![0f64; ARMS.len()];

    print!("| corpus | seqs | ns/seq |");
    for (_, l, _) in ARMS {
        print!(" {l} |");
    }
    println!();
    println!("| --- | ---: | ---: |{}", " ---: |".repeat(ARMS.len()));

    for id in IDS {
        let Some(f) = load(id) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();

        rusty_zstd::prof_reset();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let nseq = rusty_zstd::prof_encode_counts().seqs;
        if nseq < 1000 {
            continue;
        }
        let n = nseq as f64;

        rusty_zstd::set_dup_k(k);
        // Warm the process on this corpus before anything is believed.
        rusty_zstd::set_dup_arm(0);
        for _ in 0..2 {
            let _ = once(&z, s, 0);
        }

        let mut base_all: Vec<f64> = Vec::new();
        let mut cells = Vec::new();
        for (i, (arm, _, per_k)) in ARMS.iter().enumerate() {
            let mut deltas: Vec<f64> = Vec::new();
            for _ in 0..rounds {
                // ABBA: base, arm, arm, base -- drift cancels in the pairing.
                rusty_zstd::set_dup_arm(0);
                let a1 = once(&z, s, 0);
                rusty_zstd::set_dup_arm(*arm);
                let b1 = once(&z, s, *arm);
                let b2 = once(&z, s, *arm);
                rusty_zstd::set_dup_arm(0);
                let a2 = once(&z, s, 0);
                base_all.push(a1.min(a2));
                deltas.push(0.5 * (b1 + b2) - 0.5 * (a1 + a2));
            }
            let d = med(&mut deltas);
            // ns for ONE execution of the op
            let per = d / n / (k as f64) / per_k;
            tot[i] += d / (k as f64);
            cells.push(format!("{per:.2}"));
        }
        rusty_zstd::set_dup_arm(0);
        rusty_zstd::set_dup_k(1);

        let base = med(&mut base_all);
        tot_base += base;
        tot_seq += nseq;
        println!("| {id} | {nseq} | {:.2} | {} |", base / n, cells.join(" | "));
    }

    let base_per = tot_base / tot_seq as f64;
    println!("\n**Board: DecSeqLoop = {base_per:.2} ns/sequence.** Cost of ONE execution:\n");
    println!("| op | executions/seq | ns per execution | ns/seq attributed | % of loop |");
    println!("| --- | ---: | ---: | ---: | ---: |");
    let mut covered = 0f64;
    let mut rows: Vec<(String, f64, f64, f64)> = Vec::new();
    for (i, (_, label, per_k)) in ARMS.iter().enumerate() {
        let per_seq = tot[i] / tot_seq as f64;
        covered += per_seq;
        rows.push(((*label).to_string(), *per_k, per_seq / per_k, per_seq));
    }
    rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap());
    for (label, ex, per_exec, per_seq) in &rows {
        println!(
            "| `{label}` | {ex:.0} | {per_exec:.2} | {per_seq:.2} | {:.1} |",
            100.0 * per_seq / base_per
        );
    }
    println!(
        "| **ladder total** | | | **{covered:.2}** | **{:.1}** |",
        100.0 * covered / base_per
    );
    println!(
        "\nCoverage {:.1}% of the measured {base_per:.2} ns/seq. The shortfall is the\n\
         cache-warm bias plus per-iteration overhead (branch, counter, bounds) that\n\
         belongs to no single op.",
        100.0 * covered / base_per
    );
}
