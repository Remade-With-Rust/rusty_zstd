//! Does -10.15% of POSITIONS buy time, where -25% of table WRITES did not?
//! 4.40 predicts yes: positions are dependent, latency-bound work.
const IDS:&[&str]=&["sao","mozilla","mr","ooffice","dickens","samba","webster","x-ray"];
fn ms(src:&[u8],n:u32,lvl:i32,r:usize)->f64{
    rusty_zstd::set_accel_shift_arm(n);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u32,b:u32,lvl:i32)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,lvl,7); let b1=ms(src,b,lvl,7);
        let b2=ms(src,b,lvl,7); let a2=ms(src,a,lvl,7);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: negative = faster. best-of-7 x ABBA x5, median\n");
    println!("{:<12}{:>9}{:>11}{:>11}","corpus","null","shift 7","shift 6");
    let (mut tn,mut t7,mut t6,mut k)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,8,8,lvl); let a=paired(src,8,7,lvl); let b=paired(src,8,6,lvl);
        println!("{id:<12}{n:>8.2}%{a:>10.2}%{b:>10.2}%");
        tn+=n.abs(); t7+=a; t6+=b; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mean shift7 {:+.2}%   mean shift6 {:+.2}%", tn/k, t7/k, t6/k);
    rusty_zstd::set_accel_shift_arm(8);
}
