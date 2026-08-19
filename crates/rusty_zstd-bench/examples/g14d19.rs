//! GATE 14 @ L19: sweep the depth target. Unlike L3 the ledger is clean -- the
//! DP calls bt_find_best once per position whatever the target, so the target
//! only changes PROBES per call. No hidden added term.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("{:>7}{:>13}{:>15}{:>11}{:>11}   worst corpus","depth","size","bt probes","size %","probe %");
    let (mut bs,mut bp)=(0i64,0u64);
    let mut base=vec![];
    for d in [32usize,64,128,24,16,12,8,4]{
        let (mut sz,mut pr)=(0i64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_bt_depth_target_arm(d);
            let _=rusty_zstd::take_bt_probe_stats();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            pr+=rusty_zstd::take_bt_probe_stats().0;
        }
        if d==32 {bs=sz; bp=pr; base=per.clone();
            println!("{d:>7}{sz:>13}{pr:>15}{:>11}{:>11}   (shipped)","-","-");
        } else {
            let (mut w,mut wid)=(0f64,"");
            for (k,id) in IDS.iter().enumerate(){
                if k<per.len(){let x=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                    if x>w {w=x; wid=id;}}
            }
            println!("{d:>7}{sz:>13}{pr:>15}{:>+10.4}%{:>+10.2}%   {wid} {w:+.3}%",
                100.0*(sz-bs) as f64/bs as f64,
                100.0*(pr as f64-bp as f64)/bp.max(1) as f64);
        }
    }
    rusty_zstd::set_bt_depth_target_arm(0);
}
