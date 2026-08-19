//! GATE 1 @ L22 — `adv.nb_workers > 0`: CONSTANT or DISPATCH?
//!
//!   cargo run --release -p rusty_zstd-bench --example g1l22
//!
//! # Why this cell needed its own harness
//!
//! `resolve_job_size` ends with `raw.max(JOB_SIZE_MIN).max(overlap)`, and at
//! L19/L22 `overlap_log` defaults to 9 for the Bt strategies, so
//! `overlap = window >> (9 - 9) = window`. An explicit `job_size` BELOW the
//! window is therefore silently raised to the window, and `compress_mt` then
//! short-circuits to `encode_oneshot` whenever `src.len() <= job`.
//!
//! My first pass at Gates 1 @ L19/L22 asked for a 1 MiB job and got a job the
//! size of the whole source. Both "MT" arms ran `encode_oneshot` — the same code
//! as the pin. The losses it reported were noise between two runs of identical
//! work: a NULL A/B, the exact failure this campaign keeps catching in others.
//!
//! So every MT arm here PROVES it ran, deterministically, by counting frames
//! with `inspect_frames`. MT emits one frame per job; `frames == 1` means the
//! arm degenerated to oneshot and the cell is refused rather than scored.
use rusty_zstd::AdvancedOptions;
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

const CAP: usize = 2 << 20;
const ITERS: usize = 2;
const LOSS_PCT: f64 = 3.0;

fn best_of<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let (mut best, mut out) = (f64::MAX, 0usize);
    for _ in 0..ITERS {
        let t = Instant::now();
        out = f();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
    }
    (best, out)
}

fn frames(z: &[u8]) -> usize {
    rusty_zstd::inspect_frames(z).map(|v| v.len()).unwrap_or(0)
}

struct ArmSpec {
    label: &'static str,
    workers: u32,
    ovlog: u32,
    job: usize,
}
const ARMS: &[ArmSpec] = &[
    ArmSpec { label: "oneshot", workers: 0, ovlog: 0, job: 0 },
    ArmSpec { label: "mt8/ov-def", workers: 8, ovlog: 0, job: 1 << 20 },
    ArmSpec { label: "mt4/ov1", workers: 4, ovlog: 1, job: 1 << 20 },
    ArmSpec { label: "mt8/ov1", workers: 8, ovlog: 1, job: 1 << 20 },
    ArmSpec { label: "mt8/ov6", workers: 8, ovlog: 6, job: 1 << 20 },
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    #[allow(non_snake_case)] let LVL = lvl;
    println!("GATE 1 @ L{LVL} — nb_workers, 18 corpora, {} MiB cap, {} cores", CAP >> 20,
        std::thread::available_parallelism().map(|p| p.get()).unwrap_or(0));
    println!("  every MT arm PROVES it ran by counting frames (one frame per job)\n");

    // ---- PART 1: the structural claim, deterministic, no clock ----
    println!("PART 1 — can MT engage at L{LVL} AT ALL with the default overlap?");
    println!("{:<13} {:>6} {:>6} {:>9} {:>9} {:>9} {:>8}", "corpus", "MiB", "wlog", "window", "overlap", "job", "frames");
    let mut degenerate = 0usize;
    let mut srcs: Vec<(&str, Vec<u8>)> = Vec::new();
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        let src = full[..full.len().min(CAP)].to_vec();
        let p = rusty_zstd::compression_params(LVL, Some(src.len() as u64)).unwrap();
        let window = 1usize << p.window_log.min(31);
        let ov = rusty_zstd::overlap_size(p.window_log, 0, p.strategy);
        let job = rusty_zstd::resolve_job_size(1 << 20, p.window_log, ov);
        // ask for 8 workers and a 1 MiB job, then COUNT what came out
        let adv = AdvancedOptions { nb_workers: 8, job_size: 1 << 20, ..Default::default() };
        let z = rusty_zstd::compress_with_advanced(&src, p, false, None, &[], true, adv).unwrap();
        let nf = frames(&z);
        if nf <= 1 {
            degenerate += 1;
        }
        println!("{:<13} {:>6} {:>6} {:>8}K {:>8}K {:>8}K {:>8}", id, src.len() >> 20, p.window_log,
            window >> 10, ov >> 10, job >> 10, nf);
        srcs.push((id, src));
        let _ = std::io::stdout().flush();
    }
    println!("\n  {degenerate} of {} corpora produced ONE frame — i.e. `nb_workers=8` ran ONESHOT.", srcs.len());
    println!("  Mechanism: overlap_log defaults to 9 for BtUltra2, so overlap == window;");
    println!("  resolve_job_size does `.max(overlap)`, so the job is raised to the window;");
    println!("  and window_log is clamped to the source size, so job >= src and MT never splits.\n");

    // ---- PART 2: measure with the overlap lowered so MT genuinely runs ----
    println!("PART 2 — with overlap_log lowered, does the gate lose under CONSTANT oneshot?");
    print!("{:<13}", "corpus");
    for a in ARMS {
        print!(" {:>11}", a.label);
    }
    println!("  {:>7} {:>9} {:>8}", "best", "size%", "frames");

    let mut losers: Vec<String> = Vec::new();
    let mut anti: Vec<String> = Vec::new();
    let mut refused = 0usize;
    for (id, src) in &srcs {
        let p = rusty_zstd::compression_params(LVL, Some(src.len() as u64)).unwrap();
        let mut res: Vec<(f64, usize, usize)> = Vec::new();
        for a in ARMS {
            let adv = AdvancedOptions {
                nb_workers: a.workers,
                job_size: a.job,
                overlap_log: a.ovlog,
                ..Default::default()
            };
            let (ms, sz) = best_of(|| {
                rusty_zstd::compress_with_advanced(src, p, false, None, &[], true, adv).unwrap().len()
            });
            let z = rusty_zstd::compress_with_advanced(src, p, false, None, &[], true, adv).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == *src, "{id}: {} ROUND-TRIP FAILED", a.label);
            res.push((ms, sz, frames(&z)));
        }
        let base = res[0];
        // Only arms that PROVABLY split (frames > 1) may be compared.
        let cand: Vec<(usize, &(f64, usize, usize))> = res
            .iter()
            .enumerate()
            .skip(1)
            .filter(|(_, r)| r.2 > 1)
            .collect();
        print!("{:<13}", id);
        for r in &res {
            print!(" {:>11.1}", r.0);
        }
        if cand.is_empty() {
            refused += 1;
            println!("  {:>7} {:>9} {:>8}", "-", "-", "1 REFUSED");
            continue;
        }
        let best = cand.iter().min_by(|a, b| a.1 .0.partial_cmp(&b.1 .0).unwrap()).unwrap();
        let faster = (base.0 - best.1 .0) / base.0 * 100.0;
        let size_pct = (best.1 .1 as f64 / base.1 as f64 - 1.0) * 100.0;
        if faster > LOSS_PCT && size_pct <= 0.5 {
            losers.push(format!("{id}:{:.2}x/{:+.2}%", base.0 / best.1 .0, size_pct));
        }
        if faster < -LOSS_PCT || size_pct > 1.0 {
            anti.push(format!("{id}:{:+.1}%t/{:+.2}%s", -faster, size_pct));
        }
        println!("  {:>6.2}x {:>8.2}% {:>8}", base.0 / best.1 .0, size_pct, best.1 .2);
        let _ = std::io::stdout().flush();
    }

    let n = srcs.len();
    println!("\n  REFUSED (no arm produced >1 frame): {refused} of {n}");
    println!("  CONSTANT TEST (pin = oneshot): {} of {n} corpora lose", losers.len());
    if !losers.is_empty() {
        println!("    {}", losers.join("  "));
    }
    if !anti.is_empty() {
        println!("  PINNING MT-ON hurts {} corpora: {}", anti.len(), anti.join("  "));
    }
    let verdict = if losers.is_empty() { "CONSTANT oneshot" } else { "DISPATCH" };
    println!("\nVERDICT: GATE 1 @ L{LVL} = {verdict}");
}
