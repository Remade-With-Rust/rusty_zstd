//! The two extremes of the prefix distribution, sampled hard. If a content
//! dispatch exists, it must show HERE or nowhere: x-ray is 99.7% short prefixes,
//! nci is 8.6%.
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],arm:u8,r:usize)->f64{
    rusty_zstd::set_eqlen_arm(arm);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:u8,b:u8,rounds:usize)->(f64,f64){
    let mut d=vec![];
    for _ in 0..rounds { let a1=ms(src,a,9); let b1=ms(src,b,9);
        let b2=ms(src,b,9); let a2=ms(src,a,9);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap());
    (d[rounds/2], d[rounds-1]-d[0])
}
fn main(){
    println!("best-of-9 x ABBA x9, median and spread. negative = WORD loop faster\n");
    println!("{:<10}{:>9}{:>18}{:>18}","corpus","<8B %","words (spread)","null (spread)");
    for (id,short) in [("x-ray",99.7),("nci",8.6)]{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(4<<20)];
        let (w,ws)=paired(src,0,1,9);
        let (n,ns)=paired(src,0,0,9);
        println!("{id:<10}{short:>8.1}%{:>12.2}% ({:.1}){:>12.2}% ({:.1})",w,ws,n,ns);
    }
    rusty_zstd::set_eqlen_arm(0);
}
