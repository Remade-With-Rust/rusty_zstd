//! GATE 1 @ L3 — `adv.nb_workers > 0`: CONSTANT or DISPATCH?
//!
//!   cargo run --release -p rusty_zstd-bench --example g1l3
//!
//! Protocol:
//!   1. DEAD CHECK — does the default (`nb_workers = 0`) differ from the value
//!      set? Reported PER CORPUS, because the answer is not the same for all of
//!      them and an aggregate would hide that.
//!   2. DOES IT LOSE UNDER CONSTANT? Pin every corpus to oneshot and ask whether
//!      any corpus would rather have been routed.
//!   3. Speed is the objective, size is the guard (we are at size parity).
//!
//! Full files capped at 32 MiB — NOT the 8 MiB board prefix. At L3 the window is
//! 2 MiB, so `resolve_job_size(0, 21, ov)` = `4 * window` = 8 MiB, and
//! `compress_mt` short-circuits to `encode_oneshot` when `src.len() <= job`. A
//! board that feeds it 8 MiB therefore measures the gate's OFF arm twice and
//! reports "no change" for a knob it never reached.
use rusty_zstd::AdvancedOptions;
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LVL: i32 = 3;
const CAP: usize = 32 << 20;
const ITERS: usize = 3;

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

struct Arm {
    label: &'static str,
    workers: u32,
    job: usize,
}
const ARMS: &[Arm] = &[
    Arm { label: "oneshot", workers: 0, job: 0 },
    Arm { label: "mt2/auto", workers: 2, job: 0 },
    Arm { label: "mt4/auto", workers: 4, job: 0 },
    Arm { label: "mt8/auto", workers: 8, job: 0 },
    Arm { label: "mt24/auto", workers: 24, job: 0 },
    Arm { label: "mt8/1M", workers: 8, job: 1 << 20 },
];

fn main() {
    println!("GATE 1 @ L{LVL} — `adv.nb_workers > 0`, 18 corpora, full files capped at {} MiB", CAP >> 20);
    println!("  cores {} | speed is the objective, size is the guard", std::thread::available_parallelism().map(|p| p.get()).unwrap_or(0));
    let p = rusty_zstd::compression_params(LVL, None).unwrap();
    let window = 1usize << p.window_log.min(31);
    let ov = rusty_zstd::overlap_size(p.window_log, 0, p.strategy);
    let auto_job = rusty_zstd::resolve_job_size(0, p.window_log, ov);
    println!(
        "  L{LVL} params: window_log {} = {} MiB, overlap {} KiB, resolve_job_size(0) = {} MiB",
        p.window_log, window >> 20, ov >> 10, auto_job >> 20
    );
    println!("  => compress_mt falls back to oneshot whenever src.len() <= {} MiB\n", auto_job >> 20);

    println!(
        "{:<13} {:>6} {:>5} | {:>9} {:>9} {:>9} {:>9} {:>9} | {:>8} {:>8}",
        "corpus", "MiB", "jobs", "oneshot", "mt2", "mt4", "mt8", "mt24", "best", "size%"
    );
    println!("{}", "-".repeat(112));

    let mut reachable = 0usize;
    let mut losers: Vec<String> = Vec::new();
    let mut ident: Vec<String> = Vec::new();
    let mut tot_speedup = 0.0f64;
    let mut n = 0.0f64;

    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        let src = &full[..full.len().min(CAP)];
        let params = rusty_zstd::compression_params(LVL, Some(src.len() as u64)).unwrap();
        let ov = rusty_zstd::overlap_size(params.window_log, 0, params.strategy);
        let job = rusty_zstd::resolve_job_size(0, params.window_log, ov);
        let jobs = if src.len() <= job { 1 } else { src.len().div_ceil(job) };

        let mut res = Vec::new();
        for a in ARMS {
            let adv = AdvancedOptions { nb_workers: a.workers, job_size: a.job, ..Default::default() };
            let (ms, sz) = best_of(|| {
                rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv)
                    .unwrap()
                    .len()
            });
            res.push((a.label, ms, sz));
        }
        // correctness: every arm must round-trip
        for a in ARMS {
            let adv = AdvancedOptions { nb_workers: a.workers, job_size: a.job, ..Default::default() };
            let z = rusty_zstd::compress_with_advanced(src, params, false, None, &[], true, adv).unwrap();
            let d = rusty_zstd::decompress(&z).unwrap();
            assert!(d == src, "{id}: {} ROUND-TRIP FAILED", a.label);
        }

        let base = res[0];
        // DEAD CHECK: does the SHIPPED mt predicate (nb_workers=2, default job) differ?
        if res[1].2 == base.2 {
            ident.push((*id).to_string());
        } else {
            reachable += 1;
        }
        // best MT arm on the auto (shipped) job size
        let best = res[1..5]
            .iter()
            .min_by(|a, b| a.1.partial_cmp(&b.1).unwrap())
            .unwrap();
        let speedup = base.1 / best.1;
        let size_pct = (best.2 as f64 / base.2 as f64 - 1.0) * 100.0;
        if speedup > 1.03 {
            losers.push(format!("{id}:{}x/{:+.2}%", format!("{speedup:.2}"), size_pct));
        }
        tot_speedup += speedup;
        n += 1.0;
        println!(
            "{:<13} {:>6} {:>5} | {:>9.1} {:>9.1} {:>9.1} {:>9.1} {:>9.1} | {:>7.2}x {:>7.2}%",
            id,
            src.len() >> 20,
            jobs,
            res[0].1,
            res[1].1,
            res[2].1,
            res[3].1,
            res[4].1,
            speedup,
            size_pct
        );
        let _ = std::io::stdout().flush();
    }

    println!("\n  DEAD CHECK: nb_workers=2 (shipped job sizing) is BYTE-SIZE-IDENTICAL to oneshot on");
    println!("    {} of 18 corpora: {}", ident.len(), ident.join(" "));
    println!("    -> on those the gate's ON arm is UNREACHABLE. It is live on {reachable} of 18.");
    println!("\n  CONSTANT TEST (pin = oneshot): {} of 18 corpora lose by >3% wall time", losers.len());
    if !losers.is_empty() {
        println!("    {}", losers.join("  "));
    }
    println!("\n  mean best-MT speedup vs oneshot: {:.2}x", tot_speedup / n.max(1.0));
    println!("\n  NOTE ON UNITS: MT buys WALL time by spending CORES. `zstd -b -T1` is a");
    println!("  single-thread board, so these numbers do not belong in the us/c columns.");
    println!("  They answer a different question: is `nb_workers > 0` the right predicate?");
}
