//! 4.77 TIME: shipped size-dispatch vs pre-4.77, ABBA-paired with a null arm.
use std::time::Instant;
const IDS:&[&str]=&["x-ray","sao","incomp-32m","mozilla","samba","xml","mr","nci","dickens","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn t(src:&[u8],len:usize,n:usize)->f64{
    rusty_zstd::set_g5_fast_len_arm(len);
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let z=std::hint::black_box(rusty_zstd::compress(std::hint::black_box(src),1).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(z.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("4.77 @ L1, 8 MiB, best-of-7 x ABBA x5. negative = 4.77 is FASTER\n");
    println!("{:<13}{:>10}{:>11}","corpus","null","4.77");
    let (mut sn,mut sd,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let (mut a,mut n,mut d)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..5{
            a=a.min(t(src,usize::MAX,7));   // pre-4.77
            d=d.min(t(src,0,7));            // shipped
            n=n.min(t(src,usize::MAX,7));   // null
        }
        let dn=100.0*(n-a)/a; let dd=100.0*(d-a)/a;
        sn+=dn.abs(); sd+=dd; c+=1.0;
        println!("{id:<13}{dn:>+9.2}%{dd:>+10.2}%",);
    }
    println!("\nmean |null| {:.2}%   mean 4.77 {:+.2}%",sn/c,sd/c);
    rusty_zstd::set_g5_fast_len_arm(0);
}
