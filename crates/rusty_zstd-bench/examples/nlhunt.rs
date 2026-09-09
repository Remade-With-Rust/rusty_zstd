//! Is there a DETERMINISTIC SIGNAL that identifies the corpora where the
//! next-long probe COSTS ratio instead of buying it?
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example nlhunt
//!
//! `sizehunt` found that turning `next_long` off at L3 makes `sao` 15,001 bytes
//! SMALLER while making all 15 other corpora larger. The probe commits at
//! `ip + 1`, so it can change the parse for the worse -- it is not a pure
//! filter. The encoder already maintains `next_long_yield = nl_hits/nl_probes`
//! per block, and `NL_GAIN_G` accumulates the match bytes the probe won.
//!
//! If sao separates from the other 15 on a signal the encoder ALREADY has,
//! the 15,001 bytes are a dispatch away and cost no new instrumentation.
//! Size is the currency here: exact, no clock, no null band.
use rusty_zstd as rz;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m",
    "versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla",
    "nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = 4 << 20;
    println!("{:<14}{:>12}{:>12}{:>10}{:>12}{:>11}",
             "corpus", "nl_probes", "nl_hits", "yield", "gain B", "d bytes off");
    println!("{}", "-".repeat(72));
    let mut rows = vec![];
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let o = rz::CompressOptions { level: 3, checksum: false };
        rz::set_next_long_arm(true);
        let _ = rz::take_next_long();
        let on = rz::compress_with(s, o).unwrap().len();
        let (p, h, g) = rz::take_next_long();
        rz::set_next_long_arm(false);
        let off = rz::compress_with(s, o).unwrap().len();
        rz::set_next_long_arm(true);
        let y = if p == 0 { f64::NAN } else { h as f64 / p as f64 };
        rows.push((*id, p, h, y, g, off as i64 - on as i64));
    }
    rows.sort_by(|a, b| a.5.cmp(&b.5));
    for (id, p, h, y, g, d) in &rows {
        println!("{:<14}{:>12}{:>12}{:>10.4}{:>12}{:>+11}", id, p, h, y, g, d);
    }
    println!("\nsorted by `d bytes off`: NEGATIVE = the probe COSTS ratio there.");
    let bad: Vec<_> = rows.iter().filter(|r| r.5 < 0).collect();
    let good: Vec<_> = rows.iter().filter(|r| r.5 > 0).collect();
    if !bad.is_empty() && !good.is_empty() {
        let by = bad.iter().map(|r| r.3).fold(f64::MIN, f64::max);
        let gy = good.iter().map(|r| r.3).fold(f64::MAX, f64::min);
        println!("highest yield among COSTS-ratio corpora: {by:.4}");
        println!("lowest  yield among BUYS-ratio  corpora: {gy:.4}");
        println!("{}", if by < gy { ">>> SEPARABLE: an empty interval exists <<<" }
                       else { "NOT separable on yield alone -- the classes overlap" });
    }
}
