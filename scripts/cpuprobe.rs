//! Which ISA extensions does THIS host actually have? Build with a bare
//! `rustc scripts/cpuprobe.rs` -- it is deliberately not a cargo target, so it
//! cannot inherit the workspace's flags and mislead about the baseline.

fn main() {
    println!("bmi2={} lzcnt={} avx2={}",
        is_x86_feature_detected!("bmi2"),
        is_x86_feature_detected!("lzcnt"),
        is_x86_feature_detected!("avx2"));
}
