//! FINDINGS 1+2 cost control. Striding the tree insert was REFUSED when the
//! window was 1 MiB -- it moved along the same line. FINDING 1 widens the window
//! to 8 MiB, so there are now ~4x more positions than the tree needs. Re-ask.
use std::time::Instant;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20;
const PAY: usize = 1 << 20;
fn best<F: FnMut() -> usize>(mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..2 { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let mut srcs = Vec::new();
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < PRE + PAY { continue }
        srcs.push((full[..PRE].to_vec(), full[PRE..PRE+PAY].to_vec()));
    }
    println!("FINDINGS 1+2 @ L{lvl} cost curve — baseline = BOTH OFF (old behaviour)");
    println!("{:>14} {:>13} {:>10} {:>11} {:>10}", "arm", "bytes", "size%", "ms", "time%");
    rusty_zstd::set_prime_bt_tree_arm(false);
    rusty_zstd::set_prefix_window_arm(false);
    let (mut b0, mut t0) = (0i64, 0.0f64);
    for (pre, tail) in &srcs {
        b0 += rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len() as i64;
        t0 += best(|| rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
    }
    println!("{:>14} {:>13} {:>9.3}% {:>11.0} {:>9.1}%", "both OFF", b0, 0.0, t0, 0.0);
    rusty_zstd::set_prime_bt_tree_arm(true);
    rusty_zstd::set_prefix_window_arm(true);
    for (stride, depth) in [(1usize,5u32),(2,5),(4,5),(8,5),(4,3),(8,3),(16,5)] {
        rusty_zstd::set_prime_stride_arm(stride);
        rusty_zstd::set_prime_bt_depth_arm(depth);
        let (mut b, mut t) = (0i64, 0.0f64);
        for (pre, tail) in &srcs {
            let z = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&z, pre).unwrap() == *tail, "round-trip");
            b += z.len() as i64;
            t += best(|| rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap().len());
        }
        println!("{:>14} {:>13} {:>9.3}% {:>11.0} {:>9.1}%", format!("s{stride}/d{depth}"), b,
            (b as f64/b0 as f64-1.0)*100.0, t, (t/t0-1.0)*100.0);
    }
    rusty_zstd::set_prime_stride_arm(1);
    rusty_zstd::set_prime_bt_depth_arm(5);
}
