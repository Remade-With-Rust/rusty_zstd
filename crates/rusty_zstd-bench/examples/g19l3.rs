//! GATE 19 @ L3 -- STEP 1: does the default differ from the value set?
//!
//! POSITIVE CONTROL: the same sweep at L19, where find_opt runs. If L19 also
//! fails to move, the HARNESS is broken, not the gate (4.24's law).
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn sweep(lv:i32,cap:usize)->(usize,usize,f64){
    let vals=[1u32,3,4,6,8,10,12,16];
    let (mut moved,mut n,mut span)=(0usize,0usize,0f64);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(cap)];
        rusty_zstd::set_opt_lit_arm(u32::MAX);            // default path
        let base=rusty_zstd::compress(src,lv).unwrap().len() as f64;
        let (mut lo,mut hi)=(base,base);
        let mut diff=false;
        for v in vals{
            rusty_zstd::set_opt_lit_arm(v);
            let z=rusty_zstd::compress(src,lv).unwrap().len() as f64;
            if z!=base {diff=true;}
            lo=lo.min(z); hi=hi.max(z);
        }
        if diff {moved+=1;}
        n+=1;
        span=span.max(100.0*(hi-lo)/base);
        rusty_zstd::set_opt_lit_arm(u32::MAX);
    }
    (moved,n,span)
}
fn main(){
    println!("GATE 19 STEP 1 -- liveness by sweep, with a positive control\n");
    for (lv,cap,tag) in [(3,8usize<<20,"L3  (DFast   -- gate under test)"),
                         (19,2usize<<20,"L19 (find_opt -- POSITIVE CONTROL)")]{
        let (m,n,s)=sweep(lv,cap);
        println!("{tag}: {m}/{n} corpora move, max span {s:.4}%   -> {}",
            if m>0 {"ALIVE"} else {"DEAD"});
    }
    println!("\n=== finder reach at L3 ===");
    let f=load("dickens").unwrap();
    let _=rusty_zstd::take_finder_calls();
    let _=rusty_zstd::compress(&f[..f.len().min(8<<20)],3).unwrap();
    let fc=rusty_zstd::take_finder_calls();
    println!("find_opt calls at L3: {:?}",fc);
}
