//! GATE 19 @ L3 STEP 1: does the frame block_max cap move output?
//! Sweep RZSTD_BLOCK_KB. Gate 5 (adaptive_block_max) is ACTIVE underneath and can
//! already halve to 64 KiB per block, so this measures what the FRAME cap adds
//! on top of the shipped per-block dispatch.
use std::time::Instant;
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
const KB:&[usize]=&[16,32,48,64,96,128];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn run(src:&[u8],kb:usize)->(usize,f64){
    unsafe{ std::env::set_var("RZSTD_BLOCK_KB", kb.to_string()); }
    let mut best=f64::MAX; let mut n=0;
    for _ in 0..5{
        let t=Instant::now();
        let z=std::hint::black_box(rusty_zstd::compress(std::hint::black_box(src),3).unwrap());
        let e=t.elapsed().as_secs_f64();
        n=z.len(); if e<best{best=e;}
    }
    (n,best)
}
fn main(){
    println!("GATE 19 @ L3 -- frame block_max cap, on top of Gate 5's per-block dispatch\n");
    print!("{:<14}","corpus"); for k in KB{print!("{:>9}",format!("{k}KB"));} println!("{:>9}{:>10}","best","time@best");
    let mut tot=vec![0usize;KB.len()];
    let (mut moved,mut n)=(0,0);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let base=run(src,128);
        let mut sizes=vec![]; let mut times=vec![];
        for (i,k) in KB.iter().enumerate(){
            let (s,t)=run(src,*k); sizes.push(s); times.push(t); tot[i]+=s;
        }
        let bi=(0..sizes.len()).min_by_key(|&i|sizes[i]).unwrap();
        print!("{id:<14}");
        for s in &sizes{ print!("{:>8.3}%",100.0*(*s as f64-base.0 as f64)/base.0 as f64); }
        let dt=100.0*(times[bi]-times[KB.len()-1])/times[KB.len()-1];
        println!("{:>9}{:>9.1}%",format!("{}KB",KB[bi]),dt);
        if sizes[bi]<base.0 {moved+=1;} n+=1;
    }
    let b=*tot.last().unwrap();
    print!("{:<14}","TOTAL"); for t in &tot{ print!("{:>8.3}%",100.0*(*t as f64-b as f64)/b as f64); }
    println!();
    println!("\n{moved}/{n} corpora have an optimum SMALLER than the 128 KiB default");
    unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
}
