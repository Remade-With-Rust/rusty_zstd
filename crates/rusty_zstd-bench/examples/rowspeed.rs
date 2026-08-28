//! E1 SPEED, adjudicated by SIGN rather than magnitude.
//!
//! The objection this answers: this box's worst-pair null arm ran 27-33%, so
//! "the clock cannot judge E1" looked like the honest conclusion. It was too
//! strong. A null arm bounds the MAGNITUDE you can read from one pair. It does
//! not bound the SIGN, because noise scatters pairs symmetrically -- a null arm
//! wins ~50% of its own pairs no matter how wide it is. D10 was decided on this
//! same box at z = -14.59 for exactly that reason.
//!
//! So this harness reports three things and lets them disagree if they will:
//!
//!   * the MEDIAN null arm as well as the WORST, because the worst pair is the
//!     pessimistic statistic and quoting only it overstates the floor;
//!   * the paired win rate and z, which is the statistic that survives noise;
//!   * the WORK COUNTS for both arms -- E1 is bitstream-changing, so the arms
//!     do NOT do identical work (codec-measurement 4). The compressed size and
//!     sequence count are printed so that is visible rather than hidden.
//!
//! Timing is `EncodeMatchFind` stage ns, not process wall: the row finder
//! replaces the chain walk and nothing else, so isolating that stage removes
//! entropy coding, literals and I/O from the comparison.
use rusty_zstd::ProfStage as S;

const IDS: &[&str] = &[
    "dickens", "mozilla", "samba", "webster", "xml", "x-ray", "osdb", "reymont",
    "nci", "sao", "mr", "ooffice",
];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

/// One timed compress on the given arm. Returns (matchfind ns, compressed len).
fn once(src: &[u8], lvl: i32, rows: bool) -> (f64, usize) {
    rusty_zstd::set_row_arm(rows);
    rusty_zstd::prof_reset();
    let z = rusty_zstd::compress(src, lvl).unwrap();
    (rusty_zstd::prof_stage_ns(S::EncodeMatchFind) as f64, z.len())
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

/// One ABBA round, order alternated by `flip`.
fn abba(src: &[u8], lvl: i32, a: bool, b: bool, flip: bool) -> (f64, f64) {
    if flip {
        let b1 = once(src, lvl, b).0;
        let a1 = once(src, lvl, a).0;
        let a2 = once(src, lvl, a).0;
        let b2 = once(src, lvl, b).0;
        (0.5 * (a1 + a2), 0.5 * (b1 + b2))
    } else {
        let a1 = once(src, lvl, a).0;
        let b1 = once(src, lvl, b).0;
        let b2 = once(src, lvl, b).0;
        let a2 = once(src, lvl, a).0;
        (0.5 * (a1 + a2), 0.5 * (b1 + b2))
    }
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let rounds: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(21);
    let cap = 8usize << 20;

    println!("E1 ROW FINDER SPEED @ L{lvl} -- EncodeMatchFind stage, ABBA, {rounds} rounds");
    println!("Adjudicated by SIGN (paired win rate + z), not by magnitude.\n");
    println!("| corpus | chain ns | row ns | delta | wins | z | chain size | row size |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: | ---: | ---: |");

    let (mut tc, mut tr) = (0f64, 0f64);
    let (mut gw, mut gn) = (0usize, 0usize);
    let (mut worst_null, mut all_null) = (0f64, Vec::new());
    let (mut sc, mut sr) = (0u64, 0u64);

    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(cap)];

        // Warm, and capture the work counts both arms actually do.
        let (_, csize) = once(src, lvl, false);
        let (_, rsize) = once(src, lvl, true);
        for _ in 0..2 {
            let _ = once(src, lvl, false);
        }

        let (mut vc, mut vr) = (Vec::new(), Vec::new());
        let (mut wins, mut pairs) = (0usize, 0usize);
        for r in 0..rounds {
            // NULL arm: chain vs chain, identical code, same ABBA shape.
            let (n1, n2) = abba(src, lvl, false, false, r % 2 == 1);
            let nz = 100.0 * (n1 - n2).abs() / n1.max(n2);
            all_null.push(nz);
            worst_null = worst_null.max(nz);
            // Real arm.
            let (c, rr) = abba(src, lvl, false, true, r % 2 == 1);
            vc.push(c);
            vr.push(rr);
            if rr < c {
                wins += 1;
            }
            pairs += 1;
        }
        let (mc, mr) = (med(&mut vc), med(&mut vr));
        let z = (wins as f64 - pairs as f64 / 2.0) / (0.5 * (pairs as f64).sqrt());
        println!(
            "| {id} | {:.2} | {:.2} | {:+.1}% | {wins}/{pairs} | {z:+.2} | {csize} | {rsize} |",
            mc / 1e6,
            mr / 1e6,
            100.0 * (mr - mc) / mc
        );
        tc += mc;
        tr += mr;
        gw += wins;
        gn += pairs;
        sc += csize as u64;
        sr += rsize as u64;
    }

    rusty_zstd::set_row_arm(false);
    let gz = (gw as f64 - gn as f64 / 2.0) / (0.5 * (gn as f64).sqrt());
    println!(
        "\n**BOARD: chain {:.1} ms -> row {:.1} ms = {:+.1}%  |  {gw}/{gn} pairs, z = {gz:+.2}**",
        tc / 1e6,
        tr / 1e6,
        100.0 * (tr - tc) / tc
    );
    println!(
        "**SIZE: chain {sc} -> row {sr} = {:.4}x** (the price; E1 is bitstream-changing)",
        sr as f64 / sc as f64
    );
    println!(
        "\nNULL ARM (chain vs chain): median {:.2}%, worst {:.2}%",
        med(&mut all_null),
        worst_null
    );
    println!("The MEDIAN is the floor for a median delta; the WORST bounds a single pair.");
    println!("Neither bounds the SIGN -- a null arm wins ~50% of its own pairs by");
    println!("construction, so |z| > 2 is a verdict at any floor width.");
}
