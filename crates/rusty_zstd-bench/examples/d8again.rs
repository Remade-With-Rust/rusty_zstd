//! What D8a bought, on the path that actually ships (checksum ON).
//!
//! `set_xxh_avx2_arm(false)` forces the scalar stripe loop -- the pre-D8a
//! behaviour, where `Xxh64::update` never reached the AVX2 kernel. Both arms
//! are byte-identical by construction; this is purely how fast the SAME bytes
//! get produced. Same-arm spread reported so no unresolvable gap is read as one.
use std::time::Instant;
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","dickens","mozilla","samba","webster","xml","nci","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dec(z:&[u8], n:usize)->f64{
    let mut buf=Vec::new();
    rusty_zstd::decompress_into(&mut buf,z).unwrap();
    let mut b=f64::MAX;
    for _ in 0..n { buf.clear(); let t=Instant::now();
        rusty_zstd::decompress_into(&mut buf,z).unwrap();
        let e=t.elapsed().as_secs_f64(); if e<b {b=e} }
    b
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let n:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(25);
    let cap=8usize<<20;
    println!("DECODE with checksum ON (the shipped default), L{lvl}, best-of-{n}, MB/s\n");
    println!("{:<14}{:>12}{:>12}{:>10}{:>9}","corpus","scalar xxh","AVX2 xxh","D8a gain","spread");
    let (mut g,mut c)=(0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        let mb=s.len() as f64/1_048_576.0;
        let p=rusty_zstd::compression_params(lvl,Some(s.len() as u64)).unwrap();
        let z=rusty_zstd::compress_with_params(s,p,true).unwrap();  // checksum ON
        rusty_zstd::set_xxh_avx2_arm(false);
        let a=dec(&z,n); let a2=dec(&z,n);
        rusty_zstd::set_xxh_avx2_arm(true);
        let b=dec(&z,n);
        let spread=(a.max(a2)/a.min(a2)-1.0)*100.0;
        let gain=(a/b-1.0)*100.0;
        g+=gain; c+=1.0;
        println!("{id:<14}{:>12.1}{:>12.1}{:>9.1}%{:>8.1}%", mb/a, mb/b, gain, spread);
    }
    rusty_zstd::set_xxh_avx2_arm(true);
    println!("\n  mean D8a decode gain on the shipped path: {:+.1}%", g/c);
}
