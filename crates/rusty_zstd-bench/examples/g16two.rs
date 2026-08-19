//! GATE 16 @ L3: the two mechanisms, measured SEPARATELY and against the work
//! each actually removes.
//!   skip_search  -> removes the SEARCH on runs of raw blocks (main-loop positions)
//!   raw_limit    -> sends marginal blocks raw, removing ENTROPY coding (time)
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(skip:bool,rawlim:Option<bool>,lvl:i32)->(i64,u64,f64){
    rusty_zstd::set_raw_skip_arm(skip);
    rusty_zstd::set_incomp_skip_arm(rawlim);
    let (mut sz,mut pos)=(0i64,0u64);
    let mut ms=f64::MAX;
    let mut srcs=vec![];
    for id in IDS{
        if let Some(f)=load(id){ srcs.push(f[..f.len().min(8<<20)].to_vec()); }
    }
    for _ in 0..5 {
        let _=rusty_zstd::take_mm();
        let t=std::time::Instant::now();
        let (mut s,mut p)=(0i64,0u64);
        for x in &srcs { s+=rusty_zstd::compress(x,lvl).unwrap().len() as i64; }
        let e=t.elapsed().as_secs_f64()*1000.0;
        p+=rusty_zstd::take_mm().0;
        if e<ms {ms=e;} sz=s; pos=p;
    }
    (sz,pos,ms)
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let (s0,p0,t0)=run(true,None,lvl);          // shipped
    println!("L{lvl} shipped:            size {s0}, positions {p0}, {t0:.0} ms\n");
    println!("{:<30}{:>12}{:>10}{:>14}{:>10}{:>9}","arm","size","size %","positions","pos %","ms");
    for (label,sk,rl) in [
        ("skip_search OFF",           false, None),
        ("raw_limit OFF",             true,  Some(false)),
        ("both OFF (pre-gate)",       false, Some(false)),
    ]{
        let (s,p,t)=run(sk,rl,lvl);
        println!("{label:<30}{s:>12}{:>9.4}%{p:>14}{:>9.2}%{t:>9.0}",
            100.0*(s-s0) as f64/s0 as f64,
            if p0>0 {100.0*(p as f64-p0 as f64)/p0 as f64} else {0.0});
    }
    rusty_zstd::set_raw_skip_arm(true); rusty_zstd::set_incomp_skip_arm(None);
}
