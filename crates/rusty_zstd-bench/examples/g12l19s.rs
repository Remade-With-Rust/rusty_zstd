//! GATE 12 @ L19: the opt back-fill's stride. Confirm the gate is not dead, then
//! price it -- bt probes are DEPENDENT work (4.43's lesson), so a real work
//! reduction here should be worth something.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(19);
    println!("{:>7}{:>12}{:>13}{:>14}{:>11}{:>11}   worst","stride","size","fill inserts","bt probes","size %","probe %");
    let (mut bs,mut bi,mut bp)=(0i64,0u64,0u64);
    let mut base=vec![];
    for st in [1usize,2,3,4,8,16]{
        let (mut sz,mut ins,mut pr)=(0i64,0u64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_opt_fill_stride_arm(st);
            let _=rusty_zstd::take_opt_fill_ins(); let _=rusty_zstd::take_bt_probe_stats();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            ins+=rusty_zstd::take_opt_fill_ins();
            pr+=rusty_zstd::take_bt_probe_stats().0;
        }
        if st==1 {bs=sz; bi=ins; bp=pr; base=per.clone();
            println!("{st:>7}{sz:>12}{ins:>13}{pr:>14}{:>11}{:>11}   (today)","-","-");
        } else {
            let (mut w,mut wid)=(0f64,"");
            for (k,id) in IDS.iter().enumerate(){
                if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k] as f64;
                    if d>w {w=d; wid=id;}}
            }
            println!("{st:>7}{sz:>12}{ins:>13}{pr:>14}{:>+10.4}%{:>+10.2}%   {wid} {w:+.3}%",
                100.0*(sz-bs) as f64/bs as f64,
                if bp>0 {100.0*(pr as f64-bp as f64)/bp as f64} else {0.0});
        }
    }
    let _=bi;
    rusty_zstd::set_opt_fill_stride_arm(1);
}
