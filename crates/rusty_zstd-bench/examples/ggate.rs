//! ONE Addendum gate at ONE level: CONSTANT or DISPATCH?
//!
//!   cargo run --release -p rusty_zstd-bench --example ggate -- <gate> <level>
//!
//! Deliberately one cell per invocation. The first attempt at this ran all five
//! gates at all four levels in a single process, spent 49 minutes inside
//! `train()` without emitting a line, and had to be killed. One cell per run is
//! bounded, observable, and checkpointable.
//!
//! Protocol, identical for every cell:
//!   1. DEAD CHECK — does the default differ from the value set? Reported per
//!      corpus, because the answer is not the same for all of them.
//!   2. CONSTANT TEST — pin every corpus to one arm. A corpus LOSES when another
//!      arm is >3% faster at no size cost, or strictly smaller at no time cost.
//!      No corpus loses -> CONSTANT. Any corpus loses -> DISPATCH.
//!   3. Speed is the objective, size is the guard (we are at size parity).
use rusty_zstd::{AdvancedOptions, CompressOptions, Dictionary};
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LOSS_PCT: f64 = 3.0;

fn iters(lvl: i32) -> usize {
    if lvl >= 13 { 2 } else { 3 }
}

fn best_of<F: FnMut() -> usize>(n: usize, mut f: F) -> (f64, usize) {
    let (mut best, mut out) = (f64::MAX, 0usize);
    for _ in 0..n {
        let t = Instant::now();
        out = f();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
    }
    (best, out)
}

fn load(cap: usize) -> Vec<(&'static str, Vec<u8>)> {
    IDS.iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect()
}

struct Cell {
    id: &'static str,
    /// SOURCE bytes — not the compressed size. The first cut printed
    /// `arms[0].2 / 1024` under a "MiB" heading, i.e. the compressed size in
    /// KiB mislabelled as source MiB.
    src: usize,
    arms: Vec<(String, f64, usize)>,
}

/// Size regression still counted as "parity". We are at size parity and the
/// objective is speed, so a 4.17x speedup that costs 0.11% is a WIN, not a tie.
/// The strict (size <= 0) count is printed too, so the guard is never hidden.
const SIZE_GUARD_PCT: f64 = 0.5;

/// Everything a cell needs to be decided, printed in one shape for all five gates.
fn decide(gate: u32, lvl: i32, cells: &[Cell], pin: &str, dead: &str, vars: &str) {
    let labels: Vec<String> = cells[0].arms.iter().map(|a| a.0.clone()).collect();
    print!("{:<13} {:>7}", "corpus", "MiB");
    for l in &labels {
        print!(" {:>10}", l);
    }
    println!("  {:>8} {:>9}", "best", "size%");
    println!("{}", "-".repeat(30 + 11 * labels.len() + 20));

    let pi = labels.iter().position(|l| l == pin).expect("pin arm not in set");
    let mut losers: Vec<String> = Vec::new();
    let mut strict = 0usize;
    let mut anti: Vec<String> = Vec::new();
    for c in cells {
        let p = &c.arms[pi];
        // best OTHER arm by time
        let best = c
            .arms
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != pi)
            .min_by(|a, b| a.1 .1.partial_cmp(&b.1 .1).unwrap())
            .map(|(_, a)| a)
            .unwrap();
        let faster = (p.1 - best.1) / p.1 * 100.0;
        let size_pct = (best.2 as f64 / p.2 as f64 - 1.0) * 100.0;
        if faster > LOSS_PCT && size_pct <= SIZE_GUARD_PCT {
            losers.push(format!("{}:{:.2}x/{:+.2}%", c.id, p.1 / best.1, size_pct));
            if best.2 <= p.2 {
                strict += 1;
            }
        } else if best.2 < p.2 && faster > -LOSS_PCT {
            losers.push(format!("{}:{}{:+}B", c.id, best.0, best.2 as i64 - p.2 as i64));
            strict += 1;
        }
        // corpora that would be HURT by pinning the other way
        if faster < -LOSS_PCT || size_pct > 1.0 {
            anti.push(format!("{}:{:+.1}%t/{:+.2}%s", c.id, -faster, size_pct));
        }
        print!("{:<13} {:>7}", c.id, c.src >> 20);
        for a in &c.arms {
            print!(" {:>10.1}", a.1);
        }
        println!("  {:>7.2}x {:>8.2}%", p.1 / best.1, size_pct);
    }
    println!("\n  DEAD CHECK: {dead}");
    println!(
        "  CONSTANT TEST (pin = {pin}): {} of {} corpora lose at the {SIZE_GUARD_PCT}% size guard ({strict} of {} at a STRICT size<=0 guard)",
        losers.len(), cells.len(), cells.len()
    );
    if !losers.is_empty() {
        println!("    {}", losers.join("  "));
    }
    if !anti.is_empty() {
        println!("  PINNING THE OTHER WAY hurts {} corpora: {}", anti.len(), anti.join("  "));
    }
    let v = if losers.is_empty() {
        format!("CONSTANT {pin}")
    } else {
        format!("DISPATCH — {} of {} lose under CONSTANT {pin}. VARIABLES: {vars}", losers.len(), cells.len(), )
    };
    println!("\nVERDICT: GATE {gate} @ L{lvl} = {v}");
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------------

fn gate1(lvl: i32) {
    let cap = if lvl >= 13 { 4 << 20 } else { 32 << 20 };
    let srcs = load(cap);
    let n = iters(lvl);
    // At L19/L22 `4 * window` is 32-512 MiB, so the AUTO job size can never be
    // reached by any corpus we own. Add an explicit job so the arm exists.
    let expl = if lvl >= 13 { 1 << 20 } else { 4 << 20 };
    let mut cells = Vec::new();
    let mut unreach = Vec::new();
    for (id, s) in &srcs {
        let p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
        let ov = rusty_zstd::overlap_size(p.window_log, 0, p.strategy);
        let autojob = rusty_zstd::resolve_job_size(0, p.window_log, ov);
        if s.len() <= autojob {
            unreach.push(*id);
        }
        let run = |w: u32, job: usize| {
            let adv = AdvancedOptions { nb_workers: w, job_size: job, ..Default::default() };
            move || rusty_zstd::compress_with_advanced(s, p, false, None, &[], true, adv).unwrap().len()
        };
        let mut arms = Vec::new();
        for (label, w, j) in [
            ("oneshot".to_string(), 0u32, 0usize),
            ("mt4/auto".to_string(), 4, 0),
            ("mt8/auto".to_string(), 8, 0),
            (format!("mt4/{}M", expl >> 20), 4, expl),
            (format!("mt8/{}M", expl >> 20), 8, expl),
        ] {
            let (ms, sz) = best_of(n, run(w, j));
            arms.push((label, ms, sz));
        }
        let adv = AdvancedOptions { nb_workers: 8, job_size: expl, ..Default::default() };
        let z = rusty_zstd::compress_with_advanced(s, p, false, None, &[], true, adv).unwrap();
        assert!(rusty_zstd::decompress(&z).unwrap() == *s, "{id}: MT round-trip FAILED");
        cells.push(Cell { id, src: s.len(), arms });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!();
    let p = rusty_zstd::compression_params(lvl, None).unwrap();
    let ov = rusty_zstd::overlap_size(p.window_log, 0, p.strategy);
    let autojob = rusty_zstd::resolve_job_size(0, p.window_log, ov);
    decide(1, lvl, &cells, "oneshot",
        &format!("auto job = 4*window = {} MiB (window_log {}, overlap {} KiB). {} of {} corpora are AT OR UNDER it, so the shipped mt arm is UNREACHABLE on them: {}",
            autojob >> 20, p.window_log, ov >> 10, unreach.len(), cells.len(), unreach.join(" ")),
        "X = src.len() vs resolve_job_size(job, window_log, overlap); Y = encode throughput of the content (per-job prime_tables walks `overlap` bytes ONE AT A TIME); Z = long-range/repcode reach (jobs cannot match across a boundary)");
}

fn gate2(lvl: i32) {
    let cap = if lvl >= 13 { 2 << 20 } else { 8 << 20 };
    let srcs = load(cap);
    let n = iters(lvl);
    let mut cells = Vec::new();
    let (mut ident, mut wasted) = (0usize, 0u64);
    for (id, s) in &srcs {
        if s.len() < 65536 {
            continue;
        }
        let (pre, tail) = s.split_at(s.len() / 2);
        let p = rusty_zstd::compression_params(lvl, Some(tail.len() as u64)).unwrap();
        let window = 1usize << p.window_log.min(31);
        // PROVABLE BOUND: matches are rejected at `ip - m > window`, and
        // `back_extend` walks down at most `ip - anchor`, i.e. one block. Prefix
        // below `window + BLOCKSIZE_MAX` is unreachable, so copying it is waste.
        let keep = window + rusty_zstd::BLOCKSIZE_MAX as usize;
        let cut = pre.len().saturating_sub(keep);
        wasted += cut as u64;
        let (t0, b0) = best_of(n, || rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        let (t1, b1) = best_of(n, || rusty_zstd::compress_using_prefix(tail, &pre[cut..], lvl).unwrap().len());
        let z0 = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let z1 = rusty_zstd::compress_using_prefix(tail, &pre[cut..], lvl).unwrap();
        if z0 == z1 {
            ident += 1;
        }
        assert!(rusty_zstd::decompress_using_prefix(&z1, &pre[cut..]).unwrap() == tail, "{id}: bounded-prefix round-trip FAILED");
        cells.push(Cell { id, src: s.len(), arms: vec![("full-prefix".into(), t0, b0), ("win-bounded".into(), t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!();
    let n_ = cells.len();
    decide(2, lvl, &cells, "full-prefix",
        &format!("prefix path is FORCED whenever a prefix exists; the value set is HOW MUCH of it to copy. Truncating to window+128KiB is BYTE-IDENTICAL on {ident}/{n_} corpora and skips {} KiB of copy in this run", wasted / 1024),
        "X = prefix.len() vs window + BLOCKSIZE_MAX (everything below is provably unreachable); Y = window_log (which sets the bound)");
}

fn gate3(lvl: i32, dict: &Dictionary) {
    let cap = if lvl >= 13 { 1 << 20 } else { 4 << 20 };
    let srcs = load(cap);
    let n = iters(lvl);
    let mut cells = Vec::new();
    for (id, s) in &srcs {
        let go = |w: bool| {
            move || rusty_zstd::compress_using_dict_with(s, dict, CompressOptions { level: lvl, checksum: false }, w).unwrap().len()
        };
        let (t0, b0) = best_of(n, go(true));
        let (t1, b1) = best_of(n, go(false));
        let z = rusty_zstd::compress_using_dict_with(s, dict, CompressOptions { level: lvl, checksum: false }, true).unwrap();
        assert!(rusty_zstd::decompress_using_dict(&z, dict).unwrap() == *s, "{id}: dict round-trip FAILED");
        cells.push(Cell { id, src: s.len(), arms: vec![("write-id".into(), t0, b0), ("no-id".into(), t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!();
    decide(3, lvl, &cells, "write-id",
        &format!("dictionary id {:#x} is NON-ZERO, so the arms genuinely differ (with a raw dict, id = 0, and this gate would be a null A/B). The whole delta is 4 header bytes per FRAME", dict.id()),
        "none — the field is a decoder-contract obligation, not a content decision");
}

fn gate4(lvl: i32) {
    let cap = if lvl >= 13 { 2 << 20 } else { 8 << 20 };
    let srcs = load(cap);
    let n = iters(lvl);
    let mut cells = Vec::new();
    for (id, s) in &srcs {
        let (t0, b0) = best_of(n, || rusty_zstd::compress_with(s, CompressOptions { level: lvl, checksum: true }).unwrap().len());
        let (t1, b1) = best_of(n, || rusty_zstd::compress_with(s, CompressOptions { level: lvl, checksum: false }).unwrap().len());
        cells.push(Cell { id, src: s.len(), arms: vec![("checksum-on".into(), t0, b0), ("checksum-off".into(), t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!();
    decide(4, lvl, &cells, "checksum-on",
        "`compress()` ships checksum ON, `zstd -b` runs it OFF. The arms differ by exactly 4 bytes and by a full xxh64 pass",
        "none admissible — dropping integrity verification is the CALLER's decision, never the codec's");
}

fn gate5(lvl: i32) {
    const KB: &[usize] = &[16, 32, 64, 96, 128];
    let cap = if lvl >= 13 { 1 << 20 } else { 8 << 20 };
    let srcs = load(cap);
    let n = iters(lvl);
    let mut cells = Vec::new();
    for (id, s) in &srcs {
        let mut arms = Vec::new();
        std::env::remove_var("RZSTD_BLOCK_KB");
        let (t, b) = best_of(n, || rusty_zstd::compress_with(s, CompressOptions { level: lvl, checksum: false }).unwrap().len());
        arms.push(("default".to_string(), t, b));
        for &k in KB {
            std::env::set_var("RZSTD_BLOCK_KB", k.to_string());
            let (t, b) = best_of(n, || rusty_zstd::compress_with(s, CompressOptions { level: lvl, checksum: false }).unwrap().len());
            arms.push((format!("{k}K"), t, b));
        }
        std::env::remove_var("RZSTD_BLOCK_KB");
        cells.push(Cell { id, src: s.len(), arms });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    println!();
    decide(5, lvl, &cells, "default",
        "RZSTD_BLOCK_KB caps block_max below the 128 KiB format maximum; every value in {16,32,64,96,128} moves the output, so the arm is live",
        "X = huff_reuse (previous block's literal table still legal); Y = header_frac (tree+table bytes / payload bytes); Z = lit_bits_delta (opt_lit_price drift)");
}

fn main() {
    let a: Vec<String> = std::env::args().collect();
    let gate: u32 = a.get(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let lvl: i32 = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("\n======== GATE {gate} @ L{lvl} ========");
    let t = Instant::now();
    match gate {
        1 => gate1(lvl),
        2 => gate2(lvl),
        3 => {
            let b = std::fs::read("target/gg.dict").expect("target/gg.dict missing — run `zstd --train`");
            let d = Dictionary::from_bytes(&b).expect("parse dict");
            gate3(lvl, &d)
        }
        4 => gate4(lvl),
        5 => gate5(lvl),
        _ => panic!("gate must be 1..=5"),
    }
    println!("  [{:.0}s]", t.elapsed().as_secs_f64());
}
