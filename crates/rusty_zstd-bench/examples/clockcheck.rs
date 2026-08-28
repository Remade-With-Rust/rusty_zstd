//! Does the stage profiler's clock tell the truth?
//!
//! `prof::scope` was switched from `Instant` (WALL) to a thread CPU clock, and
//! on Windows that clock counts CYCLES, converted to nanoseconds by a calibrated
//! ratio. A calibration error is invisible in ratios and fatal in absolutes --
//! and every "ns/seq" figure recorded in `docs/plans/` is an absolute.
//!
//! There is one inequality that settles it. **Thread CPU time can never exceed
//! wall time for the same region**, because a thread cannot run for longer than
//! the interval it ran inside. So:
//!
//!   * `stage_ns <= wall_ns` -- consistent; the gap is descheduled time.
//!   * `stage_ns >  wall_ns` -- IMPOSSIBLE; the conversion is miscalibrated,
//!     and by exactly the ratio printed here (codec-measurement 7).
//!
//! The stage measured is the whole decode, so the sum of every scope should sit
//! at or just under the wall time of the call that contains them.
use rusty_zstd::ProfStage as S;

const IDS: &[&str] = &["dickens", "webster", "samba", "nci", "xml", "mozilla"];

fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}

/// Every stage the decoder accumulates, so the comparison is against the WHOLE
/// profiled region rather than one scope inside it.
const DEC_STAGES: &[S] = &[
    S::DecSeqHeader,
    S::DecSeqTables,
    S::DecSeqLoop,
    S::DecSeqTail,
    S::DecodeLiterals,
    S::DecodeChecksum,
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("CLOCK SANITY -- profiled stage ns vs measured WALL ns\n");
    println!("Thread CPU time CANNOT exceed wall for the same region.");
    println!("ratio > 1.00 therefore proves the cycles->ns calibration is wrong.\n");
    println!(
        "{:<12}{:>14}{:>14}{:>10}",
        "corpus", "stage ns", "wall ns", "ratio"
    );
    let (mut ts, mut tw) = (0f64, 0f64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(8 << 20)];
        let z = rusty_zstd::compress(src, lvl).unwrap();
        // Warm, so neither clock pays first-touch or calibration.
        let _ = rusty_zstd::decompress(&z).unwrap();

        rusty_zstd::prof_reset();
        let t0 = std::time::Instant::now();
        let out = rusty_zstd::decompress(&z).unwrap();
        let wall = t0.elapsed().as_nanos() as f64;
        assert_eq!(out.len(), src.len());
        let stage: f64 = DEC_STAGES
            .iter()
            .map(|s| rusty_zstd::prof_stage_ns(*s) as f64)
            .sum();
        println!(
            "{id:<12}{:>14.0}{:>14.0}{:>10.3}",
            stage,
            wall,
            stage / wall
        );
        ts += stage;
        tw += wall;
    }
    let r = ts / tw;
    println!("\nTOTAL stage {ts:.0} ns, wall {tw:.0} ns, ratio {r:.3}");
    if r > 1.0 {
        println!(
            "\n**MISCALIBRATED by {:.2}x.** Stage time exceeds wall, which is\
             \nphysically impossible -- the cycles->ns constant is too small.",
            r
        );
    } else {
        println!(
            "\nConsistent: stage time is {:.1}% of wall. The remainder is time\
             \nspent OUTSIDE the profiled scopes plus any descheduling.",
            100.0 * r
        );
    }
}
