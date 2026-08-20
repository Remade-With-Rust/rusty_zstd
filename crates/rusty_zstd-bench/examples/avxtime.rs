//! DECODE time, best-of-N. Decode is where the 32B copy tiers live (52% of all
//! tiered copies). Prints per corpus so a cross-build diff can be paired.
use std::time::Instant;
fn main(){
    let n:usize=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(9);
    for id in ["mr","dickens","samba","mozilla","xml","nci","webster","osdb","ooffice","reymont"]{
        let Ok(f)=std::fs::read(format!("corpora/data/silesia/{id}")) else{continue};
        let s=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(s,1).unwrap();
        let mut best=f64::MAX;
        for _ in 0..n{
            let t=Instant::now();
            let o=std::hint::black_box(rusty_zstd::decompress(std::hint::black_box(&z)).unwrap());
            let e=t.elapsed().as_secs_f64();
            std::hint::black_box(o.len());
            if e<best{best=e;}
        }
        println!("{id} {:.6}",best);
    }
}
