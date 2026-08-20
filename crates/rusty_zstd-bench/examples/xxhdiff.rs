//! BIT-EXACTNESS GATE. The AVX2 stripe loop must agree with scalar on every
//! length and every content pattern. This is a FORMAT checksum -- a single
//! differing bit is a corrupt frame, so the gate is exhaustive, not sampled.
fn main(){
    let mut bad=0usize; let mut n=0usize;
    // every length 0..2048 covers all stripe/tail boundaries, plus large sizes
    let mut lens:Vec<usize>=(0..2048).collect();
    for e in [4096usize, 65536, 1<<20, (1<<20)+31, (1<<20)+32, (1<<20)+33, 3_145_728] { lens.push(e); }
    for pat in 0..4u32 {
        for &l in &lens{
            let data:Vec<u8>=(0..l).map(|i| match pat{
                0 => (i as u8).wrapping_mul(31),
                1 => 0u8,
                2 => 0xFFu8,
                _ => ((i*2654435761) >> 13) as u8,
            }).collect();
            rusty_zstd::set_xxh_avx2_arm(false);
            let a=rusty_zstd::xxh64_pub(&data);
            rusty_zstd::set_xxh_avx2_arm(true);
            let b=rusty_zstd::xxh64_pub(&data);
            n+=1;
            if a!=b { if bad<5 {println!("MISMATCH pat{pat} len{l}: scalar {a:016x} avx2 {b:016x}");} bad+=1; }
        }
    }
    println!("{n} (pattern,length) cells, {bad} mismatches");
    // known-good vectors from the spec / C zstd
    rusty_zstd::set_xxh_avx2_arm(true);
    assert_eq!(rusty_zstd::xxh64_pub(b""), 0xEF46DB3751D8E999, "empty vector");
    println!("spec vector XXH64(\"\",0) = EF46DB3751D8E999 OK");
    if bad==0 { println!("\nBIT-EXACT."); } else { println!("\nFAILED"); std::process::exit(1); }
}
