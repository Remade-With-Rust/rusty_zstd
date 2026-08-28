//! Work-parity check: does OUR arm use the same core count as the C arm?
//!
//! The board pins C with `-T1`. This runs the simserver's exact inner loop so
//! the caller can read cpu/wall and the OS thread count off the process.
//! `cpu/wall < 1` proves single-threadedness (codec-measurement §2) -- which is
//! what makes a comparison against a `-T1` reference like-for-like.
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let n: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(25);
    let f = std::fs::read("corpora/data/silesia/dickens").expect("corpus");
    let s = &f[..f.len().min(8 << 20)];
    let p = rusty_zstd::compression_params(lvl, Some(s.len() as u64)).unwrap();
    // the simserver's exact calls
    let z = rusty_zstd::compress_with_params(s, p, false).unwrap();
    let mut buf = Vec::new();
    for _ in 0..n {
        let _ = rusty_zstd::compress_with_params(s, p, false).unwrap();
    }
    for _ in 0..n {
        rusty_zstd::decompress_into(&mut buf, &z).unwrap();
    }
    println!("done {n} enc + {n} dec, {} -> {}", s.len(), z.len());
}
