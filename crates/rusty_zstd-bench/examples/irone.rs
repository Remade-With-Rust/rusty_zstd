//! ONE compress call, for an instruction counter.
//!
//!   valgrind --tool=callgrind target/release/examples/irone <level> <id> [cap_bytes] [reps]
//!
//! The clock on a shared box resolves about +-3% on encode; an instruction
//! count resolves one instruction. This is the smallest program that puts one
//! `compress` under a counter: read the corpus file, compress it `reps` times
//! (default 1), print the work-parity anchors. Everything outside `compress`
//! is a constant few thousand instructions, so the process total moves only
//! when the encoder does.
//!
//! Anchors printed (they cannot move if the work is unchanged):
//!   id  level  src_bytes  compressed_bytes  fnv64(compressed)
fn main() {
    let a: Vec<String> = std::env::args().skip(1).collect();
    if a.len() < 2 {
        eprintln!("usage: irone <level> <corpus-id> [cap_bytes] [reps]");
        std::process::exit(2);
    }
    let level: i32 = a[0].parse().expect("level");
    let id = &a[1];
    let cap: usize = a.get(2).and_then(|s| s.parse().ok()).unwrap_or(usize::MAX);
    let reps: usize = a.get(3).and_then(|s| s.parse().ok()).unwrap_or(1);
    let mut p = format!("corpora/data/silesia/{id}");
    if !std::path::Path::new(&p).exists() {
        p = format!("corpora/data/generated/{id}");
    }
    let full = std::fs::read(&p).expect("corpus file");
    let src = &full[..full.len().min(cap)];
    let mut z = Vec::new();
    for _ in 0..reps.max(1) {
        z = rusty_zstd::compress(src, level).expect("compress");
    }
    let mut h = 0xCBF2_9CE4_8422_2325u64;
    for &b in &z {
        h = (h ^ u64::from(b)).wrapping_mul(0x100_0000_01B3);
    }
    println!("{id}\t{level}\t{}\t{}\t{h:016X}", src.len(), z.len());
}
