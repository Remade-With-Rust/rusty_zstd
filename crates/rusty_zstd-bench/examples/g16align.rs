//! The previous test had an ALIGNMENT ARTIFACT: re-probes land on blocks
//! 0, P+1, 2(P+1)..., so a transition at block 40 is caught exactly when
//! 40 % (P+1) == 0 -- true for P = 1, 3, 4 and false otherwise. That is a
//! property of the test, not the gate. Average over transition points.
fn build(noise_blocks: usize, text_blocks: usize) -> Vec<u8> {
    let blk = 128*1024usize;
    let mut src = Vec::new();
    let mut x = 0x243F6A88u32;
    for _ in 0..(noise_blocks*blk) {
        x ^= x << 13; x ^= x >> 17; x ^= x << 5;
        src.push(x as u8);
    }
    let pat = b"the quick brown fox jumps over the lazy dog. ";
    while src.len() < (noise_blocks+text_blocks)*blk { src.extend_from_slice(pat); }
    src
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    // transitions at 8 different block offsets, so no period can be lucky on all
    let offsets = [17usize,19,23,29,31,37,41,43];
    println!("mean cost of a raw->compressible transition, over {} offsets, L{lvl}\n", offsets.len());
    println!("{:>8}{:>14}{:>12}{:>10}","period","mean size %","worst %","corpus pos %");
    let corpus_pos = [(1u32,0.16f64),(2,0.10),(3,0.07),(4,0.05),(6,0.03),(8,0.02),(16,0.0)];
    for pp in [1u32,2,3,4,6,8,16]{
        let (mut sum,mut worst)=(0.0,0.0f64);
        for &off in &offsets {
            let src = build(off, 40);
            rusty_zstd::set_raw_skip_arm(false);
            let base = rusty_zstd::compress(&src,lvl).unwrap().len() as i64;
            rusty_zstd::set_raw_skip_arm(true);
            rusty_zstd::set_raw_probe_arm(pp);
            let z = rusty_zstd::compress(&src,lvl).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(), src);
            let d = 100.0*(z.len() as i64 - base) as f64 / base as f64;
            sum += d; if d>worst {worst=d;}
        }
        let cp = corpus_pos.iter().find(|x|x.0==pp).map(|x|x.1).unwrap_or(0.0);
        println!("{pp:>8}{:>13.3}%{:>11.3}%{cp:>9.2}%", sum/offsets.len() as f64, worst);
    }
    rusty_zstd::set_raw_probe_arm(0);
}
