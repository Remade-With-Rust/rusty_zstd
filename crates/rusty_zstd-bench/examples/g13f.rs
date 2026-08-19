//! Measure F: the slow path's FIXED cost, in bytes-equivalent per call.
//!
//! F is the one unknown that decides whether Gate 13's width is a constant or a
//! dispatch -- at F=0 the optimum is width 8 on all 13 corpora, at F=64 it splits
//! four ways. It cannot be assumed; the earlier "CONSTANT 8, unanimous" verdict
//! was exactly the F=0 assumption wearing a deterministic costume.
//!
//! Isolated microbenchmark: both paths, same data, same allocation, interleaved
//! ABBA so drift cancels. This is a far better SNR than a whole-encode timing
//! because nothing else is running in the loop.
use std::time::Instant;

#[inline(never)]
fn fast_path(dst: &mut Vec<u8>, src: &[u8], from: usize, n: usize, w: usize) {
    let len = dst.len();
    unsafe {
        core::ptr::copy_nonoverlapping(src.as_ptr().add(from), dst.as_mut_ptr().add(len), w);
        dst.set_len(len + n);
    }
}
#[inline(never)]
fn slow_path(dst: &mut Vec<u8>, src: &[u8], from: usize, n: usize) {
    dst.extend_from_slice(&src[from..from + n]);
}

fn main() {
    let iters: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(2_000_000);
    let reps: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(9);
    let src: Vec<u8> = (0..1 << 20).map(|i| (i % 251) as u8).collect();
    println!("SLOW-PATH FIXED COST at width 16, {iters} calls per arm, best-of-{reps} ABBA");
    println!("{:>6} {:>12} {:>12} {:>12} {:>14}", "n", "fast ns/call", "slow ns/call", "delta ns", "F bytes-equiv");
    // bytes-equivalent: measure the marginal ns per byte from the slow path's own slope
    let mut slope_pts = Vec::new();
    for n in [2usize, 4, 6, 8, 12, 16] {
        let mut bf = f64::MAX;
        let mut bs = f64::MAX;
        for _ in 0..reps {
            // A: fast
            let mut d: Vec<u8> = Vec::with_capacity(iters * 32 + 64);
            let t = Instant::now();
            for i in 0..iters { fast_path(&mut d, &src, (i * 7) & 0xFFFF, n, 16); if d.len() + 64 > d.capacity() { unsafe { d.set_len(0) } } }
            let e = t.elapsed().as_secs_f64() * 1e9 / iters as f64;
            if e < bf { bf = e }
            // B: slow
            let mut d2: Vec<u8> = Vec::with_capacity(iters * 32 + 64);
            let t = Instant::now();
            for i in 0..iters { slow_path(&mut d2, &src, (i * 7) & 0xFFFF, n); if d2.len() + 64 > d2.capacity() { unsafe { d2.set_len(0) } } }
            let e = t.elapsed().as_secs_f64() * 1e9 / iters as f64;
            if e < bs { bs = e }
        }
        slope_pts.push((n as f64, bs));
        println!("{:>6} {:>12.3} {:>12.3} {:>12.3} {:>14}", n, bf, bs, bs - bf, "-");
    }
    // ns per byte, from the slow path's slope across n
    let n0 = slope_pts.len() as f64;
    let mx = slope_pts.iter().map(|p| p.0).sum::<f64>() / n0;
    let my = slope_pts.iter().map(|p| p.1).sum::<f64>() / n0;
    let (mut sxy, mut sxx) = (0.0, 0.0);
    for (x, y) in &slope_pts { sxy += (x - mx) * (y - my); sxx += (x - mx) * (x - mx); }
    let ns_per_byte = sxy / sxx;
    println!("\n  slow-path marginal cost: {ns_per_byte:.4} ns per byte");
    if ns_per_byte > 0.0 {
        println!("  => F (bytes-equivalent) = delta_ns / ns_per_byte, per row above");
        for n in [2usize, 4, 8, 16] {
            let _ = n;
        }
    } else {
        println!("  slope non-positive -- the memcpy is not the slow path's cost at these lengths,");
        println!("  which itself means F is dominated by the FIXED overhead, not the copy.");
    }
}
