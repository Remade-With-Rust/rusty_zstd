//! How many times does a compress read the ENVIRONMENT?
//!
//! Every `env_knob` is an OS lookup and a `String` allocation for a value fixed
//! for the life of the process. The right number is "once per knob, ever". A
//! count that scales with blocks means a cache whose sentinel collides with the
//! value it caches -- e.g. storing 0 for "unset" when 0 is also the default.
fn main() {
    let lvl: i32 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(3);
    let f = std::fs::read("corpora/data/silesia/dickens").expect("corpus");
    for mib in [1usize, 2, 4, 8] {
        let src = &f[..f.len().min(mib << 20)];
        let _ = rusty_zstd::take_env_reads();
        let z = rusty_zstd::compress(src, lvl).expect("c");
        let n = rusty_zstd::take_env_reads();
        assert_eq!(rusty_zstd::decompress(&z).expect("d"), src);
        let _ = rusty_zstd::take_env_reads();
        println!(
            "{mib:>3} MiB -> {n:>7} env reads  ({:.1} per MiB)",
            n as f64 / mib as f64
        );
    }
    println!("\nFlat across sizes = cached. Growing with input = a broken cache.");
}
