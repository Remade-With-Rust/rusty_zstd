//! Isolate each piece of GATE 14 on the CLOCK. Two half-ledgers have already
//! misled me here, so this measures time directly, one change at a time.
const IDS:&[&str]=&["x-ray","dickens","smallmsg-8m","mr","osdb","reymont","webster","sao","jsonlog-16m","samba","mozilla","nci","ooffice","xml"];
fn set(g2:usize){
    rusty_zstd::set_dfast_good_ml_arm(8);
    rusty_zstd::set_dfast_good_ml2_arm(g2);
    rusty_zstd::set_nl_off_worse_arm(-1.0);
}
fn ms(src:&[u8],g2:usize,r:usize)->f64{
    set(g2);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,3).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:usize,b:usize)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,7); let b1=ms(src,b,7); let b2=ms(src,b,7); let a2=ms(src,a,7);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    println!("L3, 8 MiB, best-of-7 x ABBA x5. All arms pinned; ONLY cand2 varies.\n");
    println!("{:<12}{:>9}{:>11}{:>11}","corpus","null","cand2=16","cand2=24");
    let (mut tn,mut t16,mut t24,mut k)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let n=paired(src,8,8); let a=paired(src,8,16); let b=paired(src,8,24);
        println!("{id:<12}{n:>8.2}%{a:>10.2}%{b:>10.2}%");
        tn+=n.abs(); t16+=a; t24+=b; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   cand2=16 {:+.2}%   cand2=24 {:+.2}%", tn/k, t16/k, t24/k);
    rusty_zstd::set_dfast_good_ml_arm(0); rusty_zstd::set_dfast_good_ml2_arm(0);
    rusty_zstd::set_nl_off_worse_arm(0.60);
}
