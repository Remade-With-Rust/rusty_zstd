//! GATE 1 at the HIGH levels (19, 22) — the minimal test.
//!
//! The question is exactly: **does any corpus LOSE under the constant?**
//! At L19/L22 the level default is `BtUltra2`, the most expensive finder there
//! is, so no corpus can lose on SIZE — it already gets the best. A corpus loses
//! by paying a large TIME cost for a size gain it does not receive. So the test
//! is: is there a CHEAPER arm whose size is within noise of the default?
//!
//! Three deliberate economies over `gate1.rs`, each stated because they change
//! what the numbers mean:
//!   1. Three cheap arms (`fast`, `greedy`, `lazy2`), not all seven — the
//!      question needs "is anything cheaper free", not a full ranking.
//!   2. Each corpus is capped to a PREFIX (default 8 MiB). Ratios and relative
//!      times are preserved; ABSOLUTE sizes are NOT comparable to the L1/L3
//!      tables, which used whole files.
//!   3. Flush after every row, so a long run is observable instead of trapped
//!      in an 8 KiB stdout buffer.
use rusty_zstd::Strategy;
use std::io::Write;

const ARMS: &[(&str, Option<Strategy>)] = &[
    ("DEFAULT", None),
    ("fast", Some(Strategy::Fast)),
    ("greedy", Some(Strategy::Greedy)),
    ("lazy2", Some(Strategy::Lazy2)),
];

fn run_arm(src: &[u8], st: Option<Strategy>, lvl: i32) -> (usize, f64) {
    rusty_zstd::set_strategy_arm(st);
    let t = std::time::Instant::now();
    let z = rusty_zstd::compress(src, lvl).unwrap();
    let ms = t.elapsed().as_secs_f64() * 1000.0;
    rusty_zstd::set_strategy_arm(None);
    // correctness gate: an arm override must never change legality
    assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "round-trip failed");
    (z.len(), ms)
}

fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(19);
    let cap: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 * 1024 * 1024);
    let ids = [
        "zeros-32m",
        "text-32m",
        "incomp-32m",
        "jsonlog-16m",
        "smallmsg-8m",
        "versions-16m",
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
    println!("GATE 1 @ L{lvl} — does any corpus LOSE under the constant? (prefix {} MiB)", cap / 1048576);
    println!(
        "{:<14}{:>12}{:>10}   {:>10}{:>9}{:>10}{:>9}{:>10}{:>9}",
        "clip", "DEFAULT B", "ms", "fast %", "fast ms", "greedy %", "grdy ms", "lazy2 %", "lz2 ms"
    );
    println!("{}", "-".repeat(100));
    let mut verdicts: Vec<(String, String)> = Vec::new();
    for id in ids {
        let src = match std::fs::read(format!("corpora/data/generated/{id}")) {
            Ok(v) => v,
            Err(_) => match std::fs::read(format!("corpora/data/silesia/{id}")) {
                Ok(v) => v,
                Err(_) => continue,
            },
        };
        let src = &src[..src.len().min(cap)];
        let mut out = Vec::new();
        for (name, st) in ARMS {
            let (sz, ms) = run_arm(src, *st, lvl);
            out.push((*name, sz, ms));
        }
        let (_, bsz, bms) = out[0];
        let mut row = format!("{id:<14}{bsz:>12}{bms:>9.0}ms   ");
        // A corpus LOSES under the constant when a cheaper arm is either
        //   (a) FREE   -- size within +0.10% AND strictly faster, or
        //   (b) CHEAP  -- >=10x faster for <=2.00% size.
        // Requiring the TIME half is what the first version got wrong: it
        // flagged arms that were smaller but SLOWER as "free".
        let mut free: Option<(&str, f64, f64)> = None;
        for &(n, sz, ms) in &out[1..] {
            let d = 100.0 * (sz as f64 - bsz as f64) / bsz as f64;
            row.push_str(&format!("{d:>9.2}%{ms:>8.0}ms"));
            let faster = ms < bms;
            let speedup = bms / ms.max(1e-9);
            let qualifies = (d <= 0.10 && faster) || (speedup >= 10.0 && d <= 2.00);
            if qualifies && free.is_none() {
                free = Some((n, d, ms));
            }
        }
        println!("{row}");
        let _ = std::io::stdout().flush();
        verdicts.push(match free {
            Some((n, d, ms)) => (
                id.to_string(),
                format!("LOSES: `{n}` is {d:+.2}% size at {:.0}ms vs {:.0}ms ({:.1}x faster)", ms, bms, bms / ms.max(1e-9)),
            ),
            None => (id.to_string(), "ok — needs the default".into()),
        });
    }
    println!("\n=== DOES ANY CORPUS LOSE UNDER THE CONSTANT? ===");
    let mut n = 0;
    for (id, v) in &verdicts {
        if v.starts_with("LOSES") {
            n += 1;
            println!("  {id:<14}{v}");
        }
    }
    println!(
        "\n{} of {} corpora lose  =>  {}",
        n,
        verdicts.len(),
        if n == 0 { "CONSTANT" } else { "DISPATCH" }
    );
}
