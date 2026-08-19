//! GATE 14 @ L3: sweep the "good enough, stop searching" match length.
//! Deterministic -- probes and size, per corpus, so Step 2 (does the outcome
//! differ by CONTENT?) is answered from the split, not from a mean.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:>6}{:>13}{:>15}{:>11}{:>11}   worst corpus","good_ml","size","probes","size %","probe %");
    let (mut bs,mut bp)=(0i64,0u64);
    let mut base=vec![];
    for g in [8usize,4,5,6,12,16,24,32]{
        let (mut sz,mut pr)=(0i64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_good_ml_arm(g);
            let _=rusty_zstd::take_mm();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            pr+=rusty_zstd::take_mm().0;
        }
        if g==8 {bs=sz; bp=pr; base=per.clone();
            println!("{g:>6}{sz:>13}{pr:>15}{:>11}{:>11}   (shipped)","-","-");
        } else {
            let (mut w,mut wid)=(0f64,"");
            for (k,id) in IDS.iter().enumerate(){
                if k<per.len(){let d=100.0*(per[k]-base[k]) as f64/base[k].max(1) as f64;
                    if d>w {w=d; wid=id;}}
            }
            println!("{g:>6}{sz:>13}{pr:>15}{:>+10.4}%{:>+10.2}%   {wid} {w:+.3}%",
                100.0*(sz-bs) as f64/bs as f64,
                100.0*(pr as f64-bp as f64)/bp.max(1) as f64);
        }
    }
    rusty_zstd::set_dfast_good_ml_arm(0);
}
