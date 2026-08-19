//! The re-probe is INSURANCE against content that starts incompressible and then
//! starts compressing. No corpus file tests that, so period 1024 reads "free".
//! Synthesise the case and check the insurance actually pays.
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    // 40 blocks of noise, then 40 blocks of highly compressible text
    let blk = 128*1024usize;
    let mut src = Vec::new();
    let mut x = 0x243F6A88u32;
    for _ in 0..(40*blk) {
        x ^= x << 13; x ^= x >> 17; x ^= x << 5;
        src.push(x as u8);
    }
    let pat = b"the quick brown fox jumps over the lazy dog. ";
    while src.len() < 80*blk { src.extend_from_slice(pat); }
    println!("synthetic: {} blocks noise then {} blocks text, L{lvl}\n", 40, 40);
    println!("{:>10}{:>13}{:>11}{:>12}","probe","size","vs best","verdict");
    let mut best=i64::MAX;
    let mut rows=vec![];
    for pp in [1u32,2,3,4,5,6,8,16]{
        rusty_zstd::set_raw_skip_arm(true);
        rusty_zstd::set_raw_probe_arm(pp);
        let z=rusty_zstd::compress(&src,lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip pp={pp}");
        let n=z.len() as i64;
        if n<best {best=n;}
        rows.push((pp,n));
    }
    for (pp,n) in &rows{
        let d=100.0*(n-best) as f64/best as f64;
        let v=if d>1.0 {"INSURANCE LAPSED"} else if d>0.01 {"worse"} else {"ok"};
        println!("{pp:>10}{n:>13}{d:>10.3}%{v:>12}");
    }
    // and with the gate off entirely, for reference
    rusty_zstd::set_raw_skip_arm(false);
    let z=rusty_zstd::compress(&src,lvl).unwrap();
    println!("\ngate OFF: {} bytes ({:+.3}% vs best)", z.len(),
        100.0*(z.len() as i64-best) as f64/best as f64);
    rusty_zstd::set_raw_skip_arm(true); rusty_zstd::set_raw_probe_arm(0);
}
