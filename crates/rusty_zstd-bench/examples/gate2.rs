//! GATE 2 truth table — the repcode-1 search, three arms, all 18 corpora.
//!
//!   OFF       never probe repcode-1        (constant)
//!   ON        always probe repcode-1       (constant)
//!   MEASURED  rep_yield >= REP_YIELD_MIN   (today's dispatch)
//!
//! The question is the standing one: DOES ANY CORPUS LOSE UNDER A CONSTANT?
//! If one constant is best-or-equal everywhere, ship it and delete the
//! dispatch. If different corpora want different constants, the dispatch is
//! earning its place and the only question left is its threshold.
use std::io::Write;
use std::time::Instant;

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

fn arm(src: &[u8], lvl: i32, mode: Option<bool>) -> (usize, f64) {
    rusty_zstd::set_rep1_mode(mode);
    let mut best = f64::MAX;
    let mut sz = 0;
    let t0 = Instant::now();
    for _ in 0..7 {
        let t = Instant::now();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < best {
            best = e;
        }
        sz = z.len();
        if t0.elapsed().as_secs_f64() * 1000.0 > 300.0 {
            break;
        }
    }
    let z = rusty_zstd::compress(src, lvl).unwrap();
    assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "round-trip failed");
    rusty_zstd::set_rep1_mode(None);
    (sz, best)
}

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 * 1024 * 1024);
    println!("GATE 2 @ L{lvl} — repcode-1 search. Size and ms vs the MEASURED dispatch.");
    println!(
        "{:<14}{:>12}{:>9}   {:>10}{:>9}   {:>10}{:>9}",
        "clip", "MEASURED B", "ms", "OFF %", "OFF ms", "ON %", "ON ms"
    );
    println!("{}", "-".repeat(80));
    let (mut off_w, mut on_w, mut disp_w) = (0, 0, 0);
    for id in IDS {
        let full = match std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        {
            Ok(v) => v,
            Err(_) => continue,
        };
        let src = &full[..full.len().min(cap)];
        let (m_sz, m_ms) = arm(src, lvl, None);
        let (o_sz, o_ms) = arm(src, lvl, Some(false));
        let (n_sz, n_ms) = arm(src, lvl, Some(true));
        let od = 100.0 * (o_sz as f64 - m_sz as f64) / m_sz as f64;
        let nd = 100.0 * (n_sz as f64 - m_sz as f64) / m_sz as f64;
        println!(
            "{id:<14}{m_sz:>12}{m_ms:>8.1}ms   {od:>9.3}%{o_ms:>8.1}ms   {nd:>9.3}%{n_ms:>8.1}ms"
        );
        let _ = std::io::stdout().flush();
        // which arm gives the smallest output for this corpus?
        if o_sz <= m_sz && o_sz <= n_sz {
            off_w += 1;
        }
        if n_sz <= m_sz && n_sz <= o_sz {
            on_w += 1;
        }
        if m_sz <= o_sz && m_sz <= n_sz {
            disp_w += 1;
        }
    }
    println!(
        "\nsmallest-output arm: OFF wins/ties {off_w}/18 | ON wins/ties {on_w}/18 | MEASURED wins/ties {disp_w}/18"
    );
    println!("(a constant is only correct if it wins or ties ALL 18)");
}
