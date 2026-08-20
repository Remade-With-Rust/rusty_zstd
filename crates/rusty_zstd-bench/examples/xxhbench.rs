//! XXH64 throughput: scalar vs AVX2. Buffer allocated ONCE outside the timed
//! region (4.79's lesson), best-of-N, ABBA-interleaved, with a null arm.
use std::time::Instant;
fn run(d:&[u8],avx:bool,n:usize)->f64{
    rusty_zstd::set_xxh_avx2_arm(avx);
    let mut b=f64::MAX;
    for _ in 0..n{
        let t=Instant::now();
        let h=std::hint::black_box(rusty_zstd::xxh64_pub(std::hint::black_box(d)));
        let e=t.elapsed().as_secs_f64();
        std::hint::black_box(h);
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("XXH64 scalar vs AVX2, best-of-21 x ABBA x5\n");
    println!("{:<10}{:>12}{:>12}{:>10}{:>10}","size","scalar GB/s","avx2 GB/s","speedup","null");
    for kb in [16usize, 64, 256, 1024, 8192, 32768]{
        let n=kb<<10;
        let data:Vec<u8>=(0..n).map(|i|(i as u8).wrapping_mul(31)).collect();
        let (mut sc,mut av,mut nl)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..5{
            sc=sc.min(run(&data,false,21));
            av=av.min(run(&data,true,21));
            nl=nl.min(run(&data,false,21));
        }
        let g=|t:f64| n as f64/t/1e9;
        println!("{:<10}{:>12.2}{:>12.2}{:>9.2}x{:>9.1}%",
            if kb>=1024 {format!("{} MiB",kb/1024)} else {format!("{kb} KiB")},
            g(sc), g(av), sc/av, 100.0*(nl-sc)/sc);
    }
    rusty_zstd::set_xxh_avx2_arm(true);
}
