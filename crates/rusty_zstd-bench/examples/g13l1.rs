//! GATE 13 @ L1 — the fixed 16-byte literal copy: CONSTANT or DISPATCH?
//!
//! The L3 second pass found "a four-condition guard runs to copy FOUR bytes,
//! ~2M times: the guard is the cost, not the copy" -- but left the WIDTH at a
//! constant 16. At L1 `push_literals` is called 15.7M times, so if the run-length
//! distribution differs from L3's the width may be mis-chosen, and if it differs
//! BETWEEN CORPORA the width is a dispatch rather than a constant.
//!
//! Deterministic: run-length buckets and fast/slow path counts. No clock needed
//! to establish the distribution.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 13 @ L{lvl} — literal-run distribution per corpus (cap {} MiB)", cap >> 20);
    println!("{:<13} {:>11} {:>7} {:>7} {:>7} {:>7} {:>7} {:>7} | {:>8} {:>8}",
        "corpus", "calls", "0-4", "5-8", "9-16", "17-32", "33-64", "65+", "<=8 cum", "fast%");
    let (mut t0, mut t8, mut tall) = (0u64, 0u64, 0u64);
    let mut best_w8 = 0; let mut best_w16 = 0; let mut n = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let (h, fast, slow) = rusty_zstd::take_lp_stats();
        let calls: u64 = h.iter().sum();
        if calls == 0 { println!("{:<13} {:>11} (push_literals never called)", id, 0); continue }
        let pc = |x: u64| x as f64 / calls as f64 * 100.0;
        let cum8 = pc(h[0] + h[1]);
        println!("{:<13} {:>11} {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% {:>6.1}% | {:>7.1}% {:>7.1}%",
            id, calls, pc(h[0]), pc(h[1]), pc(h[2]), pc(h[3]), pc(h[4]), pc(h[5]),
            cum8, fast as f64/(fast+slow).max(1) as f64*100.0);
        t0 += h[0]; t8 += h[0]+h[1]; tall += calls;
        // which width would serve more calls at lower store traffic?
        if cum8 >= 85.0 { best_w8 += 1 } else { best_w16 += 1 }
        n += 1;
    }
    if tall > 0 {
        println!("\n  ALL CORPORA: 0-4 = {:.1}%, <=8 = {:.1}% of {} calls",
            t0 as f64/tall as f64*100.0, t8 as f64/tall as f64*100.0, tall);
        println!("  corpora where an 8-byte width would serve >=85% of calls: {best_w8}/{n}");
        println!("  STORE TRAFFIC: width 16 writes {:.1} MB to deliver {:.1} MB of literals in the fast path",
            tall as f64 * 16.0 / 1e6, tall as f64 * 4.0 / 1e6);
    }
}
