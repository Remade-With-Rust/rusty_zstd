fn main() {
    let f = std::fs::read("corpora/data/generated/versions-16m").unwrap();
    let s = &f[..f.len().min(8 << 20)];
    for wide in [false, true] {
        rusty_zstd::set_fast_hash_arm(wide);
        rusty_zstd::FF_LATCH.store(0, std::sync::atomic::Ordering::Relaxed);
        let z = rusty_zstd::compress(s, 1).unwrap();
        let latch = rusty_zstd::FF_LATCH.load(std::sync::atomic::Ordering::Relaxed);
        let (c4, acc) = rusty_zstd::take_ff_waste();
        println!("wide={wide}: {} bytes, latches {latch}, cand4 {c4}, accepted {acc}", z.len());
    }
}
