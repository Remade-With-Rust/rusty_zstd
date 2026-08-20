//! Tighter: 8 ABBA rounds, best-of-15, on the corpora that read outside the null.
use std::time::Instant;
const IDS:&[&str]=&["xml","zeros-32m","text-32m","incomp-32m","jsonlog-16m","nci","mozilla","dickens"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn dt(z:&[u8],fused:bool,n:usize)->f64{
    rusty_zstd::set_ck_stream_arm(fused);
    let mut b=f64::MAX;
    for _ in 0..n{
        let s=Instant::now();
        let v=std::hint::black_box(rusty_zstd::decompress(std::hint::black_box(z)).unwrap());
        let e=s.elapsed().as_secs_f64();
        std::hint::black_box(v.len());
        if e<b{b=e;}
    }
    b
}
fn main(){
    println!("FUSED vs separate, best-of-15 x ABBA x8. negative = fused FASTER\n");
    println!("{:<14}{:>9}{:>10}{:>12}","corpus","null","fused","ratio |f/n|");
    let (mut sn,mut sf,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let z=rusty_zstd::compress(src,3).unwrap();
        let (mut a,mut fu,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..8{
            a=a.min(dt(&z,false,15));
            fu=fu.min(dt(&z,true,15));
            nn=nn.min(dt(&z,false,15));
        }
        let dn=100.0*(nn-a)/a; let df=100.0*(fu-a)/a;
        sn+=dn.abs(); sf+=df; c+=1.0;
        println!("{id:<14}{dn:>+8.2}%{df:>+9.2}%{:>12.1}",
            if dn.abs()>1e-9 {df.abs()/dn.abs()} else {f64::INFINITY});
    }
    println!("\nmean |null| {:.2}%   mean fused {:+.2}%",sn/c,sf/c);
    rusty_zstd::set_ck_stream_arm(false);
}
