//! GATE 2 @ L3 — `dict` / `prefix` present: CONSTANT or DISPATCH?
//!
//!   cargo run --release -p rusty_zstd-bench --example g2l3
//!
//! The gate itself is FORCED: if a caller supplies a prefix, the prefix path
//! runs. The decidable question is the one behind it — **how much of the prefix
//! to copy, and how densely to prime it** — because both are pure setup cost
//! paid before a single payload byte is searched.
//!
//! Shaped like `--patch-from`: a 4 MiB reference against a 1 MiB payload, so
//! priming is the dominant term rather than a rounding error. At L3 the window
//! is 2 MiB, so the reference is 2x the window and half of it is provably
//! unreachable.
//!
//! Two arms, and they are different KINDS:
//!   * `win-bounded`  — copy only `window + BLOCKSIZE_MAX`. BYTE-IDENTICAL by
//!                      construction; anything below cannot be referenced.
//!   * `stride N`     — prime every Nth position (libzstd's `ZSTD_dtlm_fast`).
//!                      NOT byte-identical; a size-for-speed trade.
//!
//! Every arm carries `take_prime_iters()`, a deterministic count of positions
//! inserted. That is the primary evidence; the clock is confirmatory. On a setup
//! cost this small the clock alone cannot separate the arms — the first pass at
//! this cell reported the strictly-less-work arm as SLOWER on 8 of 18 corpora,
//! which was pure jitter.
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LVL: i32 = 3;
const PREFIX: usize = 4 << 20;
const PAYLOAD: usize = 1 << 20;
const ITERS: usize = 15;

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

fn main() {
    let p = rusty_zstd::compression_params(LVL, Some(PAYLOAD as u64)).unwrap();
    let window = 1usize << p.window_log.min(31);
    let keep = window + rusty_zstd::BLOCKSIZE_MAX as usize;
    println!("GATE 2 @ L{LVL} — prefix priming. reference {} MiB, payload {} MiB", PREFIX >> 20, PAYLOAD >> 20);
    println!("  window {} KiB, provable keep-bound window+BLOCKSIZE_MAX = {} KiB", window >> 10, keep >> 10);
    println!("  primary evidence = primed positions (deterministic); clock is confirmatory\n");

    println!("{:<13} {:>10} {:>10} {:>8} | {:>9} {:>9} {:>7} | {:>9} {:>9} {:>7}",
        "corpus", "full it", "bound it", "cut%", "full ms", "bound ms", "t%", "s1 bytes", "s3 bytes", "s3 size%");
    println!("{}", "-".repeat(118));

    let (mut ident, mut n) = (0usize, 0usize);
    let (mut tot_full, mut tot_bound) = (0u64, 0u64);
    let (mut s3_smaller, mut s3_larger) = (0usize, 0usize);
    let mut s3_tot = 0i64;
    let mut s1_tot = 0i64;
    let mut faster_bound = 0usize;
    let mut faster_s3 = 0usize;

    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        if full.len() < PREFIX + PAYLOAD {
            continue;
        }
        let pre = &full[..PREFIX];
        let tail = &full[PREFIX..PREFIX + PAYLOAD];
        let cut = pre.len().saturating_sub(keep);
        let short = &pre[cut..];

        rusty_zstd::set_prime_stride_arm(1);
        // ---- arm A: full prefix, stride 1 (SHIPPED) ----
        let _ = rusty_zstd::take_prime_iters();
        let zf = rusty_zstd::compress_using_prefix(tail, pre, LVL).unwrap();
        let it_full = rusty_zstd::take_prime_iters();
        let (tf, _) = best_of(|| rusty_zstd::compress_using_prefix(tail, pre, LVL).unwrap().len());

        // ---- arm B: window-bounded prefix, stride 1 ----
        let _ = rusty_zstd::take_prime_iters();
        let zb = rusty_zstd::compress_using_prefix(tail, short, LVL).unwrap();
        let it_bound = rusty_zstd::take_prime_iters();
        let (tb, _) = best_of(|| rusty_zstd::compress_using_prefix(tail, short, LVL).unwrap().len());
        assert!(rusty_zstd::decompress_using_prefix(&zb, short).unwrap() == tail, "{id}: bounded round-trip");
        if zf == zb {
            ident += 1;
        }

        // ---- arm C: bounded prefix, stride 3 (libzstd dtlm_fast shape) ----
        rusty_zstd::set_prime_stride_arm(3);
        let _ = rusty_zstd::take_prime_iters();
        let z3 = rusty_zstd::compress_using_prefix(tail, short, LVL).unwrap();
        let _it3 = rusty_zstd::take_prime_iters();
        let (t3, _) = best_of(|| rusty_zstd::compress_using_prefix(tail, short, LVL).unwrap().len());
        assert!(rusty_zstd::decompress_using_prefix(&z3, short).unwrap() == tail, "{id}: stride-3 round-trip");
        rusty_zstd::set_prime_stride_arm(1);

        let cutp = (1.0 - it_bound as f64 / it_full.max(1) as f64) * 100.0;
        let tp = (tb / tf - 1.0) * 100.0;
        let s3p = (z3.len() as f64 / zb.len() as f64 - 1.0) * 100.0;
        if tp < -1.0 { faster_bound += 1; }
        if t3 < tb * 0.99 { faster_s3 += 1; }
        if z3.len() < zb.len() { s3_smaller += 1 } else if z3.len() > zb.len() { s3_larger += 1 }
        tot_full += it_full;
        tot_bound += it_bound;
        s1_tot += zb.len() as i64;
        s3_tot += z3.len() as i64;
        n += 1;
        println!("{:<13} {:>10} {:>10} {:>7.1}% | {:>9.2} {:>9.2} {:>6.1}% | {:>9} {:>9} {:>6.2}%",
            id, it_full, it_bound, cutp, tf, tb, tp, zb.len(), z3.len(), s3p);
        let _ = std::io::stdout().flush();
    }

    println!("\n  ARM B (window-bounded copy): BYTE-IDENTICAL on {ident}/{n}");
    println!("    primed positions {tot_full} -> {tot_bound} ({:+.1}%), {faster_bound}/{n} measurably faster",
        (tot_bound as f64 / tot_full as f64 - 1.0) * 100.0);
    println!("  ARM C (stride 3): size {s1_tot} -> {s3_tot} ({:+.4}%), {s3_smaller} smaller / {s3_larger} larger, {faster_s3}/{n} faster",
        (s3_tot as f64 / s1_tot as f64 - 1.0) * 100.0);
}
