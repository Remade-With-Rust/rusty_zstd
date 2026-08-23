//! D8a measurement: what wiring `stripes_hybrid` into `Xxh64::update` buys on
//! real decode.
//!
//! METHOD: the arm is read ONCE from the environment at startup and never
//! again -- codec-measurement 15's corollary, the A/B switch must not live
//! inside the loop it is timing. The process is pinned and its CPU TIME is
//! read by the caller (this box runs at ~70% load from a neighbouring job, so
//! wall time is not admissible). Work count is printed for parity.
use std::time::Instant;
fn main() {
    let arm = std::env::var("D8A_ARM").unwrap_or_default() == "1";
    let id = std::env::var("D8A_CORPUS").unwrap_or_else(|_| "zeros-32m".into());
    let reps: usize = std::env::var("D8A_REPS").ok().and_then(|s| s.parse().ok()).unwrap_or(12);
    rusty_zstd::set_xxh_avx2_arm(arm);

    let f = std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .expect("corpus");
    let src = &f[..f.len().min(32 << 20)];
    let z = rusty_zstd::compress(src, 3).expect("compress");
    let mut dst = Vec::with_capacity(src.len() + 4096);

    // deterministic pass, outside every timed region
    let n = rusty_zstd::decompress_into(&mut dst, &z).expect("decode");
    assert_eq!(&dst[..], src, "roundtrip");
    let receipt = rusty_zstd::xxh64_pub(src);

    let t = Instant::now();
    let mut bytes = 0usize;
    for _ in 0..reps {
        dst.clear();
        bytes += std::hint::black_box(rusty_zstd::decompress_into(&mut dst, std::hint::black_box(&z)).unwrap());
    }
    let w = t.elapsed().as_secs_f64();
    println!("arm={} corpus={id} reps={reps} bytes={bytes} n={n} receipt={receipt:016X} wall={w:.4}",
        u8::from(arm));
}
