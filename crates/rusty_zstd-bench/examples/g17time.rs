//! min_match at L1, ABBA-paired with a null arm. The unpaired sweep put BOTH
//! mls=5 and mls=8 faster than the shipped 7, which would make 7 a local maximum
//! -- implausible enough to demand a proper instrument.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],m:u32,lvl:i32,r:usize)->f64{
    let mut p=rusty_zstd::compression_params(lvl,Some(src.len() as u64)).unwrap();
    p.min_match=m;
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now();
        let _=rusty_zstd::compress_with_params(src,p,false).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u32,b:u32,lvl:i32)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,lvl,5); let b1=ms(src,b,lvl,5);
        let b2=ms(src,b,lvl,5); let a2=ms(src,a,lvl,5);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}, 8 MiB, best-of-5 x ABBA x5, median. negative = faster than mls=7\n");
    println!("{:<12}{:>9}{:>10}{:>10}","corpus","null","mls=5","mls=8");
    let (mut tn,mut t5,mut t8,mut k)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,7,7,lvl); let a=paired(src,7,5,lvl); let b=paired(src,7,8,lvl);
        println!("{id:<12}{n:>8.2}%{a:>9.2}%{b:>9.2}%");
        tn+=n.abs(); t5+=a; t8+=b; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   mls=5 {:+.2}%   mls=8 {:+.2}%", tn/k, t5/k, t8/k);
}
