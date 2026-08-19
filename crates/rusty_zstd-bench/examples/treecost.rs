//! Is our tree-build 3.8x C's because of BAD FUNCTIONS (constant per-node
//! overhead) or because of TREE SHAPE (degenerate descents on repetitive
//! content)?
//!
//! Constant overhead => ns/position is FLAT across corpora.
//! Tree degeneration => ns/position varies with how repetitive the content is.
use rusty_zstd::Dictionary;
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("TREE-BUILD COST @ L{lvl} — is it overhead or shape?");
    println!("{:<13} {:>10} {:>10} {:>10} {:>12} {:>10}", "corpus", "heads ms", "tree ms", "build ms", "positions", "ns/pos");
    let mut rows = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let (pre, tail) = (f[..PRE].to_vec(), f[PRE..PRE+PAY].to_vec());
        let d = Dictionary::raw(pre.clone());
        let run = |tree: bool| -> (f64, u64) {
            rusty_zstd::set_prime_bt_tree_arm(tree);
            rusty_zstd::set_prefix_window_arm(false);   // isolate the TREE, not the window
            rusty_zstd::set_prime_bt_extent_arm(1);
            rusty_zstd::set_prime_bt_depth_arm(if tree {5} else {0});
            let _ = rusty_zstd::take_prime_iters();
            let mut b = f64::MAX;
            let mut it = 0;
            for _ in 0..3 {
                let _ = rusty_zstd::take_prime_iters();
                let s = Instant::now();
                let _ = rusty_zstd::compress_using_dict(&tail, &d, lvl).unwrap();
                let e = s.elapsed().as_secs_f64()*1000.0;
                it = rusty_zstd::take_prime_iters();
                if e < b { b = e; }
            }
            (b, it)
        };
        let (th, ph) = run(false);
        let (tt, pt) = run(true);
        assert_eq!(ph, pt, "{id}: position count differs between arms");
        let build = tt - th;
        let nspos = build * 1e6 / pt.max(1) as f64;
        println!("{:<13} {:>10.0} {:>10.0} {:>10.0} {:>12} {:>10.1}", id, th, tt, build, pt, nspos);
        rows.push((*id, nspos));
    }
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    let (lo, hi) = (rows.first().unwrap(), rows.last().unwrap());
    println!("\n  ns/position spread: {:.1} ({}) .. {:.1} ({}) = {:.1}x",
        lo.1, lo.0, hi.1, hi.0, hi.1/lo.1);
    println!("  FLAT => constant per-node overhead (bad functions).");
    println!("  WIDE => descent depth varies with content, i.e. TREE SHAPE.");
    rusty_zstd::set_prime_bt_tree_arm(false); rusty_zstd::set_prime_bt_extent_arm(16);
}
