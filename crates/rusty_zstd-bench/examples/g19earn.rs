//! Is GATE 5's raw-escape EARNING? ABBA-paired with a NULL arm.
//! ON = shipped (rep 2.00, ratio 0.70, drift 2.00). OFF = neither can bind.
use std::time::Instant;
const IDS:&[&str]=&["x-ray","sao","incomp-32m","mozilla","samba","xml","ooffice","mr","nci","dickens"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn on(){ rusty_zstd::set_g5_fast_arms(2.00, 0.70, 2.00); }
fn off(){ rusty_zstd::set_g5_fast_arms(2.00, 2.00, 1.0e9); }
fn t(src:&[u8],n:usize)->f64{
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
    println!("GATE 5 @ L1: ON vs OFF. positive d time = the gate makes it SLOWER");
    println!("best-of-7 x ABBA x5, with a null arm (OFF vs OFF)\n");
    println!("{:<13}{:>9}{:>10}{:>10}{:>11}","corpus","d size","null","d time","verdict");
    let (mut ss,mut st,mut sn,mut c)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        off(); let a=rusty_zstd::compress(src,1).unwrap().len() as f64;
        on();  let b=rusty_zstd::compress(src,1).unwrap().len() as f64;
        let (mut xo,mut xn,mut xf)=(f64::MAX,f64::MAX,f64::MAX);
        for _ in 0..5{
            off(); xf=xf.min(t(src,7));
            on();  xo=xo.min(t(src,7));
            off(); xn=xn.min(t(src,7));
        }
        let ds=100.0*(b-a)/a;
        let dn=100.0*(xn-xf)/xf;
        let dt=100.0*(xo-xf)/xf;
        ss+=ds; st+=dt; sn+=dn.abs(); c+=1.0;
        let v=if ds < -0.02 {"earning"} else if dt>3.0 {"PURE LOSS"} else {"no gain"};
        println!("{id:<13}{ds:>+8.3}%{dn:>+9.2}%{dt:>+9.2}%{v:>11}");
    }
    println!("\nmean |null| {:.2}%   mean d size {:+.3}%   mean d time {:+.2}%",sn/c,ss/c,st/c);
    on();
}
