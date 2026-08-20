//! GATE 19 @ L1 STEP 1 + STEP 3 coverage, in one pass.
//! L1 is the Fast ladder: match-reach is an OFF switch (rep >= 2.00), so only
//! raw-escape (r_prev >= 0.70) and drift (>= 2.00) can fire.
const IDS:&[&str]=&["mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","jsonlog-16m","smallmsg-8m","versions-16m","incomp-32m","zeros-32m"];
const KB:&[usize]=&[16,32,48,64,96,128];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn sz(src:&[u8],kb:usize)->usize{
    unsafe{ std::env::set_var("RZSTD_BLOCK_KB", kb.to_string()); }
    rusty_zstd::compress(src,1).unwrap().len()
}
fn main(){
    println!("GATE 19 @ L1 -- frame block_max cap (Fast ladder underneath)\n");
    print!("{:<14}","corpus"); for k in KB{print!("{:>9}",format!("{k}KB"));}
    println!("{:>8}{:>8}{:>8}{:>9}","best","blocks","reduced","gap?");
    let mut tot=vec![0usize;KB.len()];
    let (mut moved,mut n,mut gaps)=(0,0,vec![]);
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let base=sz(src,128);
        let mut sizes=vec![];
        for (i,k) in KB.iter().enumerate(){ let s=sz(src,*k); sizes.push(s); tot[i]+=s; }
        // shipped default (no cap) + Gate 5 coverage
        unsafe{ std::env::remove_var("RZSTD_BLOCK_KB"); }
        let _=rusty_zstd::take_g5();
        let _=rusty_zstd::compress(src,1).unwrap();
        let (c,r,d)=rusty_zstd::take_g5();
        let cov=if c>0 {100.0*(r+d) as f64/c as f64} else {0.0};
        let bi=(0..sizes.len()).min_by_key(|&i|sizes[i]).unwrap();
        let win=100.0*(sizes[bi] as f64-base as f64)/base as f64;
        print!("{id:<14}");
        for s in &sizes{ print!("{:>8.3}%",100.0*(*s as f64-base as f64)/base as f64); }
        let gap = win < -0.05 && cov < 25.0;
        println!("{:>8}{c:>8}{cov:>7.1}%{:>9}",format!("{}KB",KB[bi]), if gap {"GAP"} else {""});
        if gap {gaps.push((id.to_string(),win,cov));}
        if sizes[bi]<base {moved+=1;} n+=1;
    }
    let b=*tot.last().unwrap();
    print!("{:<14}","TOTAL"); for t in &tot{ print!("{:>8.3}%",100.0*(*t as f64-b as f64)/b as f64); }
    println!("\n\n{moved}/{n} corpora have an optimum SMALLER than 128 KiB");
    if !gaps.is_empty(){
        println!("COVERAGE GAPS (win available, Gate 5 reaching <25% of blocks):");
        for (id,w,c) in &gaps{ println!("  {id:<14}{w:>+8.3}% available, {c:.1}% reduced"); }
    }
}
