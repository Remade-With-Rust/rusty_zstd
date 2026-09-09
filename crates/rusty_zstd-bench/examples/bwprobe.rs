//! What is a copy ACTUALLY worth on this box, at the sizes the codec uses?
//!
//! Every ceiling in this campaign divided bytes-moved by "10-20 GB/s". That is
//! the peak figure for cache-resident data. The decoder's `decoded` window is
//! 2-8 MiB and its compaction memmoves most of it, which is out of L2 and
//! often out of L3 -- so the honest divisor is the measured rate AT THAT SIZE,
//! and using the peak understates every copy by whatever the ratio turns out
//! to be.
fn best<F: FnMut()>(n: u32, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        f();
        b = b.min(t.elapsed().as_secs_f64());
    }
    b
}

fn main() {
    println!(
        "{:>10}{:>14}{:>14}{:>14}",
        "size", "copy GB/s", "memmove GB/s", "fill GB/s"
    );
    for kb in [64usize, 256, 1024, 2048, 4096, 8192, 32768] {
        let n = kb << 10;
        let src = vec![7u8; n];
        let mut dst = vec![0u8; n];
        let mut big = vec![3u8; n + (n / 2)];
        // Enough reps that a single call's overhead is negligible, few enough
        // that a 32 MiB pass does not dominate the run.
        let reps = (64 << 20) / n.max(1);
        let reps = reps.clamp(3, 2000) as u32;

        let c = best(9, || {
            for _ in 0..reps {
                dst.copy_from_slice(&src);
            }
        });
        let m = best(9, || {
            for _ in 0..reps {
                big.copy_within(n / 2.., 0);
            }
        });
        let fl = best(9, || {
            for _ in 0..reps {
                dst.fill(0);
            }
        });
        let gbs = |secs: f64, bytes: usize| (bytes as f64 * reps as f64) / secs / 1e9;
        println!(
            "{:>8} KB{:>14.2}{:>14.2}{:>14.2}",
            kb,
            gbs(c, n),
            gbs(m, n),
            gbs(fl, n)
        );
    }
    println!("\nThe codec's decoded window at L3 is ~2 MiB and its compaction moves most of it.");
}
