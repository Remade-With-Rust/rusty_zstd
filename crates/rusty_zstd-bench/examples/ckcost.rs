//! What does our SHIPPED default (checksum on) cost against the -b-parity arm?
//!
//! Same protocol as the board: best-of-N, warmup discarded, decode into a
//! reused buffer. Two independent arms per configuration give a same-arm spread
//! so no gap smaller than the noise gets read as a result.
use std::time::Instant;
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","zeros-32m","incomp-32m","text-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn enc(s:&[u8], p:rusty_zstd::CompressionParameters, ck:bool, n:usize)->f64{
    let _=rusty_zstd::compress_with_params(s,p,ck);
    let mut b=f64::MAX;
    for _ in 0..n { let t=Instant::now();
        let z=rusty_zstd::compress_with_params(s,p,ck).unwrap();
        let e=t.elapsed().as_secs_f64(); std::hint::black_box(z); if e<b {b=e} }
    b
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
    println!("L{lvl}, best-of-{n}, MB/s. 'ck on' = our SHIPPED default (exercises the D8a AVX2 kernel).\n");
    println!("{:<14}{:>10}{:>10}{:>9}{:>8}   {:>10}{:>10}{:>9}{:>8}",
        "corpus","enc off","enc on","cost","spread","dec off","dec on","cost","spread");
    let (mut se,mut sd,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        let mb=s.len() as f64/1_048_576.0;
        let p=rusty_zstd::compression_params(lvl,Some(s.len() as u64)).unwrap();
        let e_off=enc(s,p,false,n); let e_off2=enc(s,p,false,n);
        let e_on =enc(s,p,true ,n);
        let z_off=rusty_zstd::compress_with_params(s,p,false).unwrap();
        let z_on =rusty_zstd::compress_with_params(s,p,true ).unwrap();
        let d_off=dec(&z_off,n); let d_off2=dec(&z_off,n);
        let d_on =dec(&z_on ,n);
        let es=(e_off.max(e_off2)/e_off.min(e_off2)-1.0)*100.0;
        let ds=(d_off.max(d_off2)/d_off.min(d_off2)-1.0)*100.0;
        let ec=(e_on/e_off-1.0)*100.0; let dc=(d_on/d_off-1.0)*100.0;
        se+=ec; sd+=dc; c+=1.0;
        println!("{id:<14}{:>10.1}{:>10.1}{:>8.1}%{:>7.1}%   {:>10.1}{:>10.1}{:>8.1}%{:>7.1}%",
            mb/e_off, mb/e_on, ec, es, mb/d_off, mb/d_on, dc, ds);
    }
    println!("\n  mean cost of shipping the checksum:  encode {:+.1}%   decode {:+.1}%", se/c, sd/c);
}
