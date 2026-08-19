//! GATE 16 @ L3, deeper: the PER-CORPUS split (a mean is not a verdict) and a
//! sweep of the two constants the short circuit has always run on.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("=== per-corpus: skip_search ON vs OFF at L{lvl} ===");
    println!("{:<14}{:>11}{:>10}{:>14}{:>10}","corpus","size delta","size %","positions","pos %");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_raw_skip_arm(false);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        rusty_zstd::set_raw_skip_arm(true);
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        if a!=b || pa!=pb {
            println!("{id:<14}{:>11}{:>9.4}%{:>14}{:>9.2}%",b-a,
                100.0*(b-a) as f64/a as f64, pb as i64-pa as i64,
                if pa>0 {100.0*(pb as f64-pa as f64)/pa as f64} else {0.0});
        }
    }
    println!("\n=== sweep RAW_RUN_MIN (blocks of raw before skipping) ===");
    println!("{:>8}{:>13}{:>11}{:>14}{:>10}   worst","run_min","size","size %","positions","pos %");
    let base=|r:u32,p:u32|{
        rusty_zstd::set_raw_skip_arm(true);
        rusty_zstd::set_raw_run_min_arm(r); rusty_zstd::set_raw_probe_arm(p);
        let (mut s,mut q)=(0i64,0u64); let mut per=vec![];
        for id in IDS{
            let Some(f)=load(id) else{continue};
            let src=&f[..f.len().min(8<<20)];
            let _=rusty_zstd::take_mm();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            s+=z; per.push(z); q+=rusty_zstd::take_mm().0;
        }
        (s,q,per)
    };
    let (s0,q0,p0)=base(2,16);
    println!("{:>8}{s0:>13}{:>11}{q0:>14}{:>10}   (shipped)",2,"-","-");
    for r in [1u32,3,4,8]{
        let (s,q,per)=base(r,16);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-p0[k]) as f64/p0[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{r:>8}{s:>13}{:>10.4}%{q:>14}{:>9.2}%   {wid} {w:+.4}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(q as f64-q0 as f64)/q0 as f64);
    }
    println!("\n=== sweep RAW_PROBE_PERIOD (re-probe interval) ===");
    for pp in [4u32,8,32,64,1024]{
        let (s,q,per)=base(2,pp);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            if k<per.len(){let d=100.0*(per[k]-p0[k]) as f64/p0[k].max(1) as f64;
                if d>w {w=d; wid=id;}}
        }
        println!("{pp:>8}{s:>13}{:>10.4}%{q:>14}{:>9.2}%   {wid} {w:+.4}%",
            100.0*(s-s0) as f64/s0 as f64,
            100.0*(q as f64-q0 as f64)/q0 as f64);
    }
    rusty_zstd::set_raw_run_min_arm(0); rusty_zstd::set_raw_probe_arm(0);
}
