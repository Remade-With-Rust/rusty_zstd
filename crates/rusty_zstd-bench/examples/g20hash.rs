//! Is our XXH64 leaving throughput on the table? Reference XXH64 on modern x86
//! runs ~13-15 GB/s. If we are near that, the +11.89% ceiling is not recoverable
//! by tuning the scalar hash, and AVX2 cannot help (see the note below).
use std::time::Instant;
fn main(){
    for mb in [1usize,8,32]{
        let n=mb<<20;
        let data:Vec<u8>=(0..n).map(|i|(i as u8).wrapping_mul(31)).collect();
        let mut best=f64::MAX;
        for _ in 0..15{
            let t=Instant::now();
            let h=std::hint::black_box(rusty_zstd::xxh64_pub(std::hint::black_box(&data)));
            let e=t.elapsed().as_secs_f64();
            std::hint::black_box(h);
            if e<best{best=e;}
        }
        println!("{:>3} MiB  {:>8.3} ms  {:>7.2} GB/s", mb, best*1e3, n as f64/best/1e9);
    }
    println!("\nreference XXH64 on modern x86: ~13-15 GB/s");
    println!("AVX2 cannot vectorise XXH64's round(): it needs a 64x64->64 multiply,");
    println!("and AVX2 only has _mm256_mul_epu32 (32x32->64). Emulating 64-bit");
    println!("multiply takes 3 mults + 2 shifts + 2 adds per lane -- slower than scalar imul.");
}
