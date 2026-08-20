//! The FULL work ledger for the route dispatch. MM_TOTAL counts main-loop
//! positions; route 2 also skips the PAIR search, and the probe adds one double
//! search per 256 blocks. Count every term -- four half-ledgers in this campaign
//! had the sign wrong.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(on:bool)->(i64,u64,u64,u64){
    rusty_zstd::set_step_probe_arm(on);
    let (mut sz,mut pos,mut pp,mut mm)=(0i64,0u64,0u64,0u64);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_pair_stats();
        let z=rusty_zstd::compress(src,1).unwrap();
        sz+=z.len() as i64;
        let m=rusty_zstd::take_mm(); pos+=m.0; mm+=m.1;
        pp+=rusty_zstd::take_pair_stats().0;
    }
    (sz,pos,pp,mm)
}
fn main(){
    let (s0,p0,q0,m0)=run(false);
    let (s1,p1,q1,m1)=run(true);
    println!("GATE 18 @ L1 route dispatch, complete work ledger\n");
    println!("{:<26}{:>14}{:>14}{:>12}","term","probe OFF","probe ON","delta");
    println!("{:<26}{:>14}{:>14}{:>+12}","compressed bytes",s0,s1,s1-s0);
    println!("{:<26}{:>14}{:>14}{:>+12}","main-loop positions",p0,p1,p1 as i64-p0 as i64);
    println!("{:<26}{:>14}{:>14}{:>+12}","pair probes",q0,q1,q1 as i64-q0 as i64);
    let w0=p0+q0; let w1=p1+q1;
    println!("{:<26}{:>14}{:>14}{:>+12}","NET search ops",w0,w1,w1 as i64-w0 as i64);
    println!("\nsize {:+.4}%   positions {:+.2}%   pair probes {:+.2}%   NET {:+.2}%",
        100.0*(s1-s0) as f64/s0 as f64,
        100.0*(p1 as f64-p0 as f64)/p0 as f64,
        if q0>0 {100.0*(q1 as f64-q0 as f64)/q0 as f64} else {0.0},
        100.0*(w1 as f64-w0 as f64)/w0 as f64);
    let _=(m0,m1);
    rusty_zstd::set_step_probe_arm(true);
}
