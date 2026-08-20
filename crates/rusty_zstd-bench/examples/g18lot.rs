//! ABBA-paired timing of the 4.72 dispatch, WITH a null arm.
use std::time::Instant;
const IDS:&[&str]=&["mr","ooffice","sao","mozilla","x-ray","dickens","samba","webster","nci","xml","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn t(src:&[u8],lo:f32,n:usize)->f64{
    rusty_zstd::set_pair_lo_arm(lo);
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
    println!("L1 encode, 8 MiB, best-of-7 x ABBA x5. negative = dispatch FASTER\n");
    println!("{:<12}{:>9}{:>11}","corpus","null","dispatch");
    let (mut sn,mut sd,mut c)=(0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let (mut a,mut b,mut nn)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..5{
            a=a.min(t(src,0.0,7));            // OFF
            b=b.min(t(src,0.71,7));           // ON
            nn=nn.min(t(src,0.0,7));          // OFF again = null
            let _=t(src,0.0,7);
        }
        let dn=100.0*(nn-a)/a; let dd=100.0*(b-a)/a;
        sn+=dn.abs(); sd+=dd; c+=1.0;
        println!("{id:<12}{dn:>+8.2}%{dd:>+10.2}%");
    }
    println!("\nmean |null| {:.2}%   mean dispatch {:+.2}%",sn/c,sd/c);
    rusty_zstd::set_pair_lo_arm(f32::NAN);
}
