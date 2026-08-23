//! 4.81 A/B: one-shot AND incremental xxh64 throughput.
//! METHOD: in-process, buffer allocated once outside the timed region,
//! best-of-N (min), N=25, 5 ABBA rounds, null arm = the same arm run twice.
//! Deterministic work count printed per arm so the two builds are comparable.
use std::time::Instant;
fn one(d: &[u8], n: usize) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = Instant::now();
        let h = std::hint::black_box(rusty_zstd::xxh64_pub(std::hint::black_box(d)));
        let e = t.elapsed().as_secs_f64();
        std::hint::black_box(h);
        if e < b { b = e }
    }
    b
}
fn incr(d: &[u8], chunk: usize, n: usize) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = Instant::now();
        let mut h = rusty_zstd::Xxh64Pub::new();
        for c in std::hint::black_box(d).chunks(chunk) { h.update(c); }
        let g = std::hint::black_box(h.digest());
        let e = t.elapsed().as_secs_f64();
        std::hint::black_box(g);
        if e < b { b = e }
    }
    b
}
fn main() {
    rusty_zstd::set_xxh_avx2_arm(true);
    println!("METHOD: in-process, best-of-25, 5 rounds, buffer preallocated, null arm shown");
    println!("{:<12}{:>13}{:>13}{:>13}{:>9}", "size", "oneshot GB/s", "incr64k GB/s", "incr4k GB/s", "null%");
    for kb in [64usize, 1024, 8192, 32768] {
        let n = kb << 10;
        let data: Vec<u8> = (0..n).map(|i| (i as u8).wrapping_mul(31)).collect();
        let (mut o, mut a, mut b4, mut nl) = (f64::MAX, f64::MAX, f64::MAX, f64::MAX);
        for _ in 0..5 {
            o = o.min(one(&data, 25));
            a = a.min(incr(&data, 65536, 25));
            b4 = b4.min(incr(&data, 4096, 25));
            nl = nl.min(one(&data, 25));
        }
        let g = |t: f64| n as f64 / t / 1e9;
        println!("{:<12}{:>13.2}{:>13.2}{:>13.2}{:>8.1}%",
            if kb >= 1024 { format!("{} MiB", kb / 1024) } else { format!("{kb} KiB") },
            g(o), g(a), g(b4), 100.0 * (nl - o) / o);
    }
    // deterministic work-parity receipt: identical across both builds or the A/B is void
    let d: Vec<u8> = (0..(1usize << 20)).map(|i| (i * 2654435761) as u8).collect();
    println!("\nwork receipt: xxh64(1MiB)={:016X}", rusty_zstd::xxh64_pub(&d));
}
