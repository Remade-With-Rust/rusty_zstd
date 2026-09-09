//! Is the row finder's size verdict CAP-DEPENDENT?
//!
//! The chain deepens as input grows; the row is fixed at 16 positions per
//! bucket. So the depth the row gives up is a function of how much data has
//! been seen -- which would make any single-cap verdict a statement about that
//! cap and not about the finder.
use rusty_zstd as rz;
const IDS: &[&str] = &["dickens","mozilla","samba","webster","xml","x-ray","osdb",
    "reymont","nci","sao","mr","ooffice","jsonlog-16m","smallmsg-8m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let caps = [512usize << 10, 1 << 20, 2 << 20, 4 << 20, 8 << 20];
    print!("{:<14}", "corpus");
    for c in caps { print!("{:>10}", format!("{}K", c >> 10)); }
    println!();
    println!("{}", "-".repeat(64));
    let mut agg = vec![(0u64, 0u64); caps.len()];
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
        print!("{:<14}", id);
        for (i, cap) in caps.iter().enumerate() {
            let src = &f[..f.len().min(*cap)];
            rz::set_row_arm(false);
            let a = rz::compress(src, lvl).unwrap().len();
            rz::set_row_arm(true);
            let b = rz::compress(src, lvl).unwrap().len();
            rz::set_row_arm(false);
            agg[i].0 += a as u64; agg[i].1 += b as u64;
            print!("{:>10.4}", b as f64 / a as f64);
        }
        println!();
    }
    print!("{:<14}", "AGGREGATE");
    for (a, b) in &agg { print!("{:>10.4}", *b as f64 / *a as f64); }
    println!();
    print!("{:<14}", "EQUAL-WEIGHT");
    println!("  (per-corpus mean is the row above averaged, not size-weighted)");
}
