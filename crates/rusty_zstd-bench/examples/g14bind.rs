//! Is DEPTH the binding constraint on the Bt walk?
//!
//! Gate 14's arm moves output on 14/18 at L5 and L13 but only 0-3/18 at L22 --
//! and INCREASING depth at L22 changes nothing at all. Either the delta is not
//! reaching the walk, or the walk is not depth-bound.
//!
//! `take_bt_iters()` answers it deterministically: walks, total iterations, and
//! walks that consumed ALL their attempts. If that last number is small, depth
//! cannot be the constraint and Gate 14 has no lever to pull.
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
fn main() {
    let cap: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    println!("BT WALK DEPTH BINDING — cap {} KiB", cap>>10);
    println!("{:>5} {:<13} {:>12} {:>14} {:>12} {:>10} {:>10}", "lvl", "corpus", "walks", "iterations", "full-depth", "full%", "mean it");
    for lvl in [13i32, 19, 22] {
        let (mut tw, mut ti, mut tf) = (0u64, 0u64, 0u64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            let _ = rusty_zstd::take_bt_iters();
            let _ = rusty_zstd::compress(s, lvl).unwrap();
            let (w, i, full) = rusty_zstd::take_bt_iters();
            if w == 0 { continue }
            tw += w; ti += i; tf += full;
            if matches!(*id, "mozilla" | "sao" | "x-ray" | "nci") {
                println!("{:>5} {:<13} {:>12} {:>14} {:>12} {:>9.2}% {:>10.2}",
                    lvl, id, w, i, full, full as f64/w as f64*100.0, i as f64/w as f64);
            }
        }
        if tw > 0 {
            println!("{:>5} {:<13} {:>12} {:>14} {:>12} {:>9.2}% {:>10.2}  <== ALL",
                lvl, "(total)", tw, ti, tf, tf as f64/tw as f64*100.0, ti as f64/tw as f64);
        }
    }
    println!("\n  full% high  -> the walk is DEPTH-BOUND and Gate 14 has a lever");
    println!("  full% low   -> the walk ends on its own guards; depth is not the constraint");
}
