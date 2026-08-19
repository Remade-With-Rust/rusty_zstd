//! GATE 12 @ L3, the direction that REMOVES work: does DFast need both of its
//! two per-match fill positions? Four table writes per match, never questioned.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let arms=[(2u8,"both (today)"),(1,"start+2 only"),(3,"end-2 only"),(0,"neither")];
    println!("{:<14}{:>12}{:>13}{:>12}{:>11}","arm","size","mainloop pos","seqs","size %");
    let mut base=0i64; let mut per_base=vec![];
    for (n,label) in arms{
        let (mut sz,mut mm,mut sq)=(0i64,0u64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_fill_n_arm(n);
            let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            mm+=rusty_zstd::take_mm().0;
            sq+=rusty_zstd::take_dfast_match_stats().1;
        }
        if base==0 {base=sz; per_base=per.clone();
            println!("{label:<14}{sz:>12}{mm:>13}{sq:>12}{:>11}","-");
        } else {
            let (mut worst,mut wid)=(0f64,"");
            for (k,id) in IDS.iter().enumerate(){
                if k<per.len(){let d=100.0*(per[k]-per_base[k]) as f64/per_base[k] as f64;
                    if d>worst{worst=d;wid=id;}}
            }
            println!("{label:<14}{sz:>12}{mm:>13}{sq:>12}{:>+10.4}%   worst {:>+6.3}% ({wid})",
                100.0*(sz-base) as f64/base as f64, worst);
        }
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
