//! Is there a DETERMINISTIC CONTENT SIGNAL that separates the corpora the row
//! finder helps from the ones it hurts?
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example rowsig
//!
//! `rowboard` reports L9 aggregate 1.0005x and calls the row finder a wash.
//! Per corpus it is not a wash at all -- it wins on 8 and loses on 4, and the
//! aggregate only lands at 1.0 because the four losers happen to be the two
//! largest files. Weight the corpora equally (1 MiB each) and the same arm
//! reads -1.18%.
//!
//! A split that clean asks for a dispatch. The row holds the last 16 positions
//! for its bucket, so it trades DEPTH for RECENCY -- and recent means SMALL
//! OFFSETS, which cost fewer bits. That should pay where matches are dense and
//! cost where they are sparse and the deep candidate was the only one. This
//! prints the signals the encoder already maintains against the measured
//! ratio, and looks for an empty interval.
use rusty_zstd as rz;
const IDS: &[&str] = &["dickens","mozilla","samba","webster","xml","x-ray","osdb",
    "reymont","nci","sao","mr","ooffice","jsonlog-16m","smallmsg-8m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap = 8usize << 20;
    let mut rows = vec![];
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
        let src = &f[..f.len().min(cap)];
        rz::set_row_arm(false);
        rz::prof_reset();
        let a = rz::compress(src, lvl).unwrap();
        let c = rz::prof_encode_counts();
        rz::set_row_arm(true);
        let b = rz::compress(src, lvl).unwrap();
        assert_eq!(rz::decompress(&b).unwrap(), src, "{id} round-trip");
        rz::set_row_arm(false);
        let n = src.len() as f64;
        // Signals the encoder already has, all from the CHAIN (default) run.
        let lit = c.lit_bytes as f64 / n;                      // literal share
        let mml = if c.seqs == 0 { 0.0 } else { c.match_bytes as f64 / c.seqs as f64 };
        let seqd = c.seqs as f64 / n * 1000.0;                 // sequences per KiB
        rows.push((*id, b.len() as f64 / a.len() as f64, lit, mml, seqd));
    }
    rows.sort_by(|x, y| x.1.partial_cmp(&y.1).unwrap());
    println!("{:<14}{:>9}{:>10}{:>10}{:>11}", "corpus", "row/chain", "lit share", "mean ml", "seqs/KiB");
    println!("{}", "-".repeat(54));
    for (id, r, lit, mml, sd) in &rows {
        println!("{:<14}{:>9.4}{:>10.4}{:>10.2}{:>11.2}", id, r, lit, mml, sd);
    }
    for (name, idx) in [("lit share", 2usize), ("mean ml", 3), ("seqs/KiB", 4)] {
        let g = |t: &(&str, f64, f64, f64, f64)| match idx { 2 => t.2, 3 => t.3, _ => t.4 };
        let win: Vec<f64> = rows.iter().filter(|t| t.1 < 1.0).map(&g).collect();
        let los: Vec<f64> = rows.iter().filter(|t| t.1 > 1.0).map(&g).collect();
        if win.is_empty() || los.is_empty() { continue; }
        let wmax = win.iter().cloned().fold(f64::MIN, f64::max);
        let wmin = win.iter().cloned().fold(f64::MAX, f64::min);
        let lmax = los.iter().cloned().fold(f64::MIN, f64::max);
        let lmin = los.iter().cloned().fold(f64::MAX, f64::min);
        let sep = wmax < lmin || lmax < wmin;
        println!("\n{name}: wins [{wmin:.4}, {wmax:.4}]  losses [{lmin:.4}, {lmax:.4}]  {}",
                 if sep { ">>> SEPARABLE <<<" } else { "overlap" });
    }
}
