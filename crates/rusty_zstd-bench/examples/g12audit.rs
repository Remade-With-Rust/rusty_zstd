//! AUDIT: GATE 12 @ L1 was ruled CONSTANT on an AGGREGATE (+0.1538% to drop
//! end-2). No per-corpus table was ever taken. Does the size cost SIGN-FLIP, and
//! does pair_gain or the route mix predict it?
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(src:&[u8],n:u8)->(i64,u64,f64,f64){
    rusty_zstd::set_dfast_fill_n_arm(n);
    let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_route_hist();
    let z=rusty_zstd::compress(src,1).unwrap();
    let pos=rusty_zstd::take_mm().0;
    let (r0,r1,r2,g,_)=rusty_zstd::take_route_hist();
    let t=(r0+r1+r2).max(1);
    (z.len() as i64,pos,g,100.0*r2 as f64/t as f64)
}
fn main(){
    println!("GATE 12 @ L1 per-corpus TRUTH TABLE (never taken in 4.41)\n");
    println!("{:<14}{:>10}{:>11}{:>12}{:>11}{:>9}","corpus","pair_gain","route2 %","drop end-2","drop both","d pos");
    let (mut ta,mut tb,mut tc)=(0i64,0i64,0i64);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let (a,pa,g,r2)=run(src,2);        // default: both fills
        let (b,_,_,_)=run(src,1);          // drop end-2
        let (c,pc,_,_)=run(src,0);         // drop both
        ta+=a; tb+=b; tc+=c;
        println!("{id:<14}{g:>10.4}{r2:>10.1}%{:>+11.4}%{:>+10.4}%{:>+9.2}%",
            100.0*(b-a) as f64/a as f64, 100.0*(c-a) as f64/a as f64,
            100.0*(pc as f64-pa as f64)/pa.max(1) as f64);
    }
    println!("\nTOTAL drop end-2 {:+.4}%   drop both {:+.4}%",
        100.0*(tb-ta) as f64/ta as f64, 100.0*(tc-ta) as f64/ta as f64);
    rusty_zstd::set_dfast_fill_n_arm(2);
}
