//! Settle it with a COUNTER, not a clock: is the 4.6x ns/position spread across
//! corpora descent DEPTH or memory behaviour?
//!
//! probes/position is deterministic and immune to cache effects. If it tracks
//! ns/position, the walk is deeper on those corpora (depth -- dialable). If it
//! is flat while ns/position varies, the nodes cost more to reach (memory --
//! not dialable by depth).
//!
//! Requires --features rusty_zstd/profile.
use rusty_zstd::Dictionary;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","x-ray","ooffice","sao","versions-16m","jsonlog-16m","samba","osdb","smallmsg-8m","mr","xml","reymont","webster","nci","dickens"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("TREE DEPTH vs MEMORY @ L{lvl} — probes/position is the arbiter");
    println!("{:<13} {:>10} {:>12} {:>12} {:>11} {:>10}", "corpus", "ns/pos", "positions", "bt probes", "probes/pos", "ns/probe");
    let mut rows = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let (pre, tail) = (f[..PRE].to_vec(), f[PRE..PRE+PAY].to_vec());
        let d = Dictionary::raw(pre.clone());
        let run = |tree: bool| -> (f64, u64, u64) {
            rusty_zstd::set_prime_bt_tree_arm(tree);
            rusty_zstd::set_prefix_window_arm(false);
            rusty_zstd::set_prime_bt_extent_arm(1);
            rusty_zstd::set_prime_bt_depth_arm(if tree {5} else {0});
            let mut b = f64::MAX;
            let (mut it, mut pr) = (0u64, 0u64);
            for _ in 0..3 {
                let _ = rusty_zstd::take_prime_iters();
                let _ = rusty_zstd::take_bt_probe_stats();
                let s = Instant::now();
                let _ = rusty_zstd::compress_using_dict(&tail, &d, lvl).unwrap();
                let e = s.elapsed().as_secs_f64()*1000.0;
                it = rusty_zstd::take_prime_iters();
                pr = rusty_zstd::take_bt_probe_stats().0;
                if e < b { b = e; }
            }
            (b, it, pr)
        };
        let (th, ph, prh) = run(false);
        let (tt, pt, prt) = run(true);
        assert_eq!(ph, pt, "{id}: positions differ");
        let build_probes = prt.saturating_sub(prh);
        let nspos = (tt - th) * 1e6 / pt.max(1) as f64;
        let ppos = build_probes as f64 / pt.max(1) as f64;
        println!("{:<13} {:>10.1} {:>12} {:>12} {:>11.2} {:>10.2}",
            id, nspos, pt, build_probes, ppos, if build_probes > 0 { (tt-th)*1e6/build_probes as f64 } else { 0.0 });
        rows.push((*id, nspos, ppos));
    }
    let n = rows.len() as f64;
    let mx = |f: fn(&(&str,f64,f64))->f64| rows.iter().map(f).fold(f64::MIN, f64::max);
    let mn = |f: fn(&(&str,f64,f64))->f64| rows.iter().map(f).fold(f64::MAX, f64::min);
    let (ns_lo, ns_hi) = (mn(|r| r.1), mx(|r| r.1));
    let (pp_lo, pp_hi) = (mn(|r| r.2), mx(|r| r.2));
    // correlation between ns/pos and probes/pos
    let (mns, mpp) = (rows.iter().map(|r| r.1).sum::<f64>()/n, rows.iter().map(|r| r.2).sum::<f64>()/n);
    let (mut sxy, mut sxx, mut syy) = (0.0,0.0,0.0);
    for r in &rows { let a = r.1-mns; let b = r.2-mpp; sxy += a*b; sxx += a*a; syy += b*b; }
    println!("\n  ns/pos spread     {:.1}x   ({:.1} .. {:.1})", ns_hi/ns_lo, ns_lo, ns_hi);
    println!("  probes/pos spread {:.1}x   ({:.2} .. {:.2})", pp_hi/pp_lo.max(0.0001), pp_lo, pp_hi);
    println!("  correlation r = {:.3}", sxy/(sxx.sqrt()*syy.sqrt()));
    println!("\n  probes/pos spread ~= ns/pos spread and r near 1  => DEPTH (dialable)");
    println!("  probes/pos FLAT while ns/pos varies              => MEMORY (not dialable by depth)");
    rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prime_bt_extent_arm(16);
}
