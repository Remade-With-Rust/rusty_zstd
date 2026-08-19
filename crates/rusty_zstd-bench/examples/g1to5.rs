//! GATES 1-5 of the Addendum campaign — CONSTANT or DISPATCH, at L3, L1, L19, L22.
//!
//!   cargo run --release -p rusty_zstd-bench --example g1to5
//!
//! One protocol, applied five times, at four levels each — twenty cells:
//!
//!   1. CONFIRM THE GATE ISN'T DEAD by validating the default differs from the
//!      value set. A gate whose arms all produce the same bytes AND the same
//!      time cannot be decided; it is reported so, never guessed at.
//!   2. DOES THE GATE LOSE UNDER CONSTANT? Pin every corpus to one arm. If no
//!      corpus loses -> CONSTANT. If any corpus loses -> DISPATCH, and the
//!      variables that separate the losers are named.
//!   3. SPEED IS THE OBJECTIVE, SIZE IS THE GUARD. We are at size parity, so a
//!      cell is decided on time and refused if size regresses.
//!
//! Gate 1 `adv.nb_workers > 0`   -> mt::compress_mt vs encode_oneshot
//! Gate 2 `dict`/`prefix`present -> the prefix||src workspace copy
//! Gate 3 `write_dict_id`        -> the Dictionary_ID frame-header field
//! Gate 4 `opts.checksum`        -> the 4-byte xxh64 trailer
//! Gate 5 `RZSTD_BLOCK_KB`       -> block_max, capped below 128 KiB
use rusty_zstd::{AdvancedOptions, CompressOptions, Dictionary};
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
/// (level, L1/L3 prefix, L19/L22 prefix)
const LEVELS: &[(i32, usize)] = &[
    (3, 4 << 20),
    (1, 4 << 20),
    (19, 512 << 10),
    (22, 512 << 10),
];
/// A corpus is a LOSER under a pinned arm when it is this much slower than the
/// best arm for that corpus. Best-of-N already removes most noise; 3% is the
/// campaign's standing "not a tie" bar for a wall-clock comparison.
const LOSS_PCT: f64 = 3.0;
const BUDGET_MS: f64 = 250.0;
const MAX_ITERS: usize = 7;

fn best_of<F: FnMut() -> usize>(mut f: F) -> (f64, usize) {
    let t0 = Instant::now();
    let (mut best, mut out) = (f64::MAX, 0usize);
    for _ in 0..MAX_ITERS {
        let t = Instant::now();
        out = f();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
        if t0.elapsed().as_secs_f64() * 1000.0 > BUDGET_MS {
            break;
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

/// One measured cell: arm label -> (ms, bytes).
struct Cell {
    id: &'static str,
    arms: Vec<(&'static str, f64, usize)>,
}

/// Decide CONSTANT vs DISPATCH for one gate at one level.
///
/// `pin` is the arm the codec ships. A corpus LOSES under that pin when another
/// arm is `LOSS_PCT` faster at no size cost, or strictly smaller at no time cost.
fn verdict(cells: &[Cell], pin: &str) -> (String, Vec<String>) {
    let mut losers = Vec::new();
    for c in cells {
        let Some(p) = c.arms.iter().find(|a| a.0 == pin) else {
            continue;
        };
        for a in &c.arms {
            if a.0 == pin {
                continue;
            }
            let faster = (p.1 - a.1) / p.1 * 100.0;
            let smaller = p.2 as i64 - a.2 as i64;
            if faster > LOSS_PCT && smaller >= 0 {
                losers.push(format!("{}:{}-{:.1}%/{:+}B", c.id, a.0, faster, -smaller));
                break;
            }
            if smaller > 0 && faster > -LOSS_PCT {
                losers.push(format!("{}:{} {:+}B", c.id, a.0, -smaller));
                break;
            }
        }
    }
    let v = if losers.is_empty() {
        format!("CONSTANT {pin}")
    } else {
        format!("DISPATCH ({} of {} corpora lose under CONSTANT {pin})", losers.len(), cells.len())
    };
    (v, losers)
}

fn report(gate: u32, lvl: i32, cells: &[Cell], pin: &str, note: &str) {
    let (v, losers) = verdict(cells, pin);
    println!("\n  GATE {gate} @ L{lvl}: {v}");
    if !losers.is_empty() {
        println!("    losers: {}", losers.join("  "));
    }
    // per-arm mean time, relative to the pinned arm
    if let Some(first) = cells.first() {
        for (i, (label, _, _)) in first.arms.iter().enumerate() {
            let (mut sum, mut n, mut bytes) = (0.0f64, 0.0f64, 0i64);
            for c in cells {
                if let (Some(p), Some(a)) = (c.arms.iter().find(|a| a.0 == pin), c.arms.get(i)) {
                    sum += (a.1 - p.1) / p.1 * 100.0;
                    bytes += a.2 as i64 - p.2 as i64;
                    n += 1.0;
                }
            }
            println!("    arm {label:<12} mean time {:+7.2}% vs {pin}   total size {bytes:+}", sum / n.max(1.0));
        }
    }
    if !note.is_empty() {
        println!("    {note}");
    }
    let _ = std::io::stdout().flush();
}

// ---------------------------------------------------------------------------

fn enc(src: &[u8], lvl: i32, ck: bool) -> Vec<u8> {
    rusty_zstd::compress_with(src, CompressOptions { level: lvl, checksum: ck }).unwrap()
}

fn gate1(srcs: &[(&'static str, Vec<u8>)], lvl: i32) {
    let params = |n: usize| rusty_zstd::compression_params(lvl, Some(n as u64)).unwrap();
    let mut cells = Vec::new();
    for (id, s) in srcs {
        let p = params(s.len());
        let mt = |w: u32, job: usize| {
            let adv = AdvancedOptions { nb_workers: w, job_size: job, ..Default::default() };
            move || rusty_zstd::compress_with_advanced(s, p, false, None, &[], true, adv).unwrap().len()
        };
        let (t0, b0) = best_of(mt(0, 0));
        let (t1, b1) = best_of(mt(2, 0));
        let (t2, b2) = best_of(mt(2, 1 << 20));
        let (t3, b3) = best_of(mt(4, 1 << 20));
        cells.push(Cell {
            id,
            arms: vec![("oneshot", t0, b0), ("mt2/auto", t1, b1), ("mt2/1M", t2, b2), ("mt4/1M", t3, b3)],
        });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    let same = cells.iter().all(|c| c.arms[0].2 == c.arms[1].2);
    report(1, lvl, &cells, "oneshot", &format!(
        "DEAD CHECK: mt2/auto == oneshot bytes on {}/{} corpora ({}). resolve_job_size(0) = 4*window, \
         and compress_mt short-circuits to oneshot when src.len() <= job.",
        cells.iter().filter(|c| c.arms[0].2 == c.arms[1].2).count(), cells.len(),
        if same { "ALL — the default arm is UNREACHABLE at these sizes" } else { "partial" }));
}

fn gate2(srcs: &[(&'static str, Vec<u8>)], lvl: i32) {
    let mut cells = Vec::new();
    let mut identical = 0usize;
    for (id, s) in srcs {
        if s.len() < 4096 {
            continue;
        }
        let (pre, tail) = s.split_at(s.len() / 2);
        let p = rusty_zstd::compression_params(lvl, Some(tail.len() as u64)).unwrap();
        let window = 1usize << p.window_log.min(31);
        // PROVABLE BOUND: a match is rejected at `ip - m > window`, and
        // `back_extend` can walk down at most `ip - anchor` bytes, which is at
        // most one block. Everything below `window + BLOCKSIZE_MAX` of the
        // prefix is therefore unreachable and copying it is pure waste.
        let keep = window + rusty_zstd::BLOCKSIZE_MAX as usize;
        let cut = pre.len().saturating_sub(keep);
        let (t0, b0) = best_of(|| rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        let (t1, b1) = best_of(|| rusty_zstd::compress_using_prefix(tail, &pre[cut..], lvl).unwrap().len());
        // byte-identity, not just size-identity
        let z0 = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
        let z1 = rusty_zstd::compress_using_prefix(tail, &pre[cut..], lvl).unwrap();
        if z0 == z1 {
            identical += 1;
        }
        assert!(rusty_zstd::decompress_using_prefix(&z1, &pre[cut..]).unwrap() == tail, "{id}: bounded prefix round-trip FAILED");
        cells.push(Cell { id, arms: vec![("full-prefix", t0, b0), ("window-bounded", t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    let n = cells.len();
    report(2, lvl, &cells, "full-prefix", &format!(
        "BYTE-IDENTICAL on {identical}/{n} corpora with the prefix truncated to window+128KiB. \
         The bound is provable: matches are rejected above `window` and back_extend walks at most one block."));
}

fn gate3(srcs: &[(&'static str, Vec<u8>)], lvl: i32, dict: &Dictionary) {
    let mut cells = Vec::new();
    for (id, s) in srcs {
        let s = &s[..s.len().min(1 << 20)];
        let go = |w: bool| {
            move || rusty_zstd::compress_using_dict_with(s, dict, CompressOptions { level: lvl, checksum: false }, w).unwrap().len()
        };
        let (t0, b0) = best_of(go(true));
        let (t1, b1) = best_of(go(false));
        cells.push(Cell { id, arms: vec![("write-id", t0, b0), ("no-id", t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    report(3, lvl, &cells, "write-id", &format!(
        "DEAD CHECK: dictionary id = {} (non-zero, so the two arms DO differ). The delta is a fixed \
         4 header bytes on every frame and zero time — this is a decoder-contract field, not a speed knob.",
        dict.id()));
}

fn gate4(srcs: &[(&'static str, Vec<u8>)], lvl: i32) {
    let mut cells = Vec::new();
    for (id, s) in srcs {
        let (t0, b0) = best_of(|| enc(s, lvl, true).len());
        let (t1, b1) = best_of(|| enc(s, lvl, false).len());
        cells.push(Cell { id, arms: vec![("checksum-on", t0, b0), ("checksum-off", t1, b1)] });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    report(4, lvl, &cells, "checksum-on", "");
}

fn gate5(srcs: &[(&'static str, Vec<u8>)], lvl: i32) {
    const KB: &[usize] = &[16, 32, 64, 84, 96, 128];
    let mut cells = Vec::new();
    for (id, s) in srcs {
        let mut arms = Vec::new();
        // 128 KiB == the shipped default (BLOCKSIZE_MAX), measured with the env unset
        std::env::remove_var("RZSTD_BLOCK_KB");
        let (t, b) = best_of(|| enc(s, lvl, false).len());
        arms.push(("default", t, b));
        for &k in KB {
            std::env::set_var("RZSTD_BLOCK_KB", k.to_string());
            let (t, b) = best_of(|| enc(s, lvl, false).len());
            arms.push((Box::leak(format!("{k}K").into_boxed_str()) as &'static str, t, b));
        }
        std::env::remove_var("RZSTD_BLOCK_KB");
        cells.push(Cell { id, arms });
        print!(".");
        let _ = std::io::stdout().flush();
    }
    report(5, lvl, &cells, "default", "");
}

fn main() {
    println!("GATES 1-5 — CONSTANT or DISPATCH, 18 corpora x L3/L1/L19/L22");
    println!("  speed is the objective, size is the guard (we are at size parity)");
    println!("  a corpus LOSES under a pin when another arm is >{LOSS_PCT}% faster at no size cost,");
    println!("  or strictly smaller at no time cost");
    let t0 = Instant::now();
    // one trained dictionary, shared by every Gate 3 cell
    let seed = load(1 << 20);
    let samples: Vec<&[u8]> = seed.iter().flat_map(|(_, s)| s.chunks(8192).take(24)).collect();
    let dbytes = rusty_zstd::train(&samples, rusty_zstd::TrainOptions::fastcover()).expect("train");
    let dict = Dictionary::from_bytes(&dbytes).expect("parse dict");
    println!("  trained dictionary: {} bytes, id {}", dbytes.len(), dict.id());

    for &(lvl, cap) in LEVELS {
        let cap = if lvl >= 13 { 512 << 10 } else { cap };
        let srcs = load(cap);
        println!("\n================ L{lvl} ({} KiB prefix, {} corpora) ================", cap >> 10, srcs.len());
        print!("  gate 1 ");
        gate1(&srcs, lvl);
        print!("  gate 2 ");
        gate2(&srcs, lvl);
        print!("  gate 3 ");
        gate3(&srcs, lvl, &dict);
        print!("  gate 4 ");
        gate4(&srcs, lvl);
        print!("  gate 5 ");
        gate5(&srcs, lvl);
    }
    println!("\nDONE in {:.0}s", t0.elapsed().as_secs_f64());
}
