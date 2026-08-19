//! min_match at L1, with the FULL ledger. Lower mls finds more matches, so:
//!   positions DOWN (ip advances further)  <- the term the last sweep showed
//!   sequences UP   (more, shorter matches) <- entropy work, the missing term
//! and a timed check, because three half-ledgers in this campaign had the sign
//! wrong until the clock was consulted.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(m:u32,lvl:i32)->(i64,u64,u64,f64){
    let mut srcs=vec![];
    for id in IDS{ if let Some(f)=load(id){ srcs.push(f[..f.len().min(8<<20)].to_vec()); } }
    let (mut sz,mut pos,mut seqs)=(0i64,0u64,0u64);
    let mut best=f64::MAX;
    for r in 0..5 {
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_rep_rate();
        let t=std::time::Instant::now();
        let mut s=0i64;
        for src in &srcs{
            let mut p=rusty_zstd::compression_params(lvl,Some(src.len() as u64)).unwrap();
            p.min_match=m;
            s+=rusty_zstd::compress_with_params(src,p,false).unwrap().len() as i64;
        }
        let e=t.elapsed().as_secs_f64()*1000.0;
        if e<best {best=e;}
        if r==0 { sz=s; pos=rusty_zstd::take_mm().0; seqs=rusty_zstd::take_rep_rate().4; }
    }
    (sz,pos,seqs,best)
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let (s0,p0,q0,t0)=run(7,lvl);
    println!("L{lvl} min_match, FULL ledger. shipped mls=7: {s0} bytes, {p0} positions, {q0} seqs, {t0:.0} ms\n");
    println!("{:>5}{:>11}{:>11}{:>11}{:>11}","mls","size %","pos %","seqs %","time %");
    for m in [4u32,5,6,8]{
        let (s,p,q,t)=run(m,lvl);
        println!("{m:>5}{:>10.4}%{:>10.2}%{:>10.2}%{:>10.2}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(p as f64-p0 as f64)/p0 as f64,
            100.0*(q as f64-q0 as f64)/q0.max(1) as f64,
            100.0*(t-t0)/t0);
    }
}
