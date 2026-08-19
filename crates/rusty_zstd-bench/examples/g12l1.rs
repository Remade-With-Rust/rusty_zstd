//! GATE 12 @ L1. Fast's back-fill is the SAME `fill_hash_after_match` DFast
//! uses, but short-table only -- half the writes -- into a table 64x smaller.
//! Step 1: is the gate dead? Does the default differ from the value set?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let arms=[(2u8,"both (today)"),(1,"drop end-2"),(3,"drop start+2"),(0,"drop both")];
    println!("{:<14}{:>12}{:>13}{:>13}{:>13}{:>11}","arm","size","mainloop pos","endfill wr","total work","size %");
    let mut b=(0i64,0u64,0u64);
    let mut moved=[0usize;4];
    let mut base_per=vec![];
    for (k,(n,label)) in arms.iter().enumerate(){
        let (mut sz,mut mm,mut fw)=(0i64,0u64,0u64);
        let mut per=vec![];
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_fill_n_arm(*n);
            let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_endfill();
            let z=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            sz+=z; per.push(z);
            mm+=rusty_zstd::take_mm().0;
            fw+=rusty_zstd::take_dfast_endfill();
        }
        let ops=mm+fw;
        if k==0 {b=(sz,mm,fw); base_per=per.clone();
            println!("{label:<14}{sz:>12}{mm:>13}{fw:>13}{ops:>13}{:>11}","-");
        } else {
            moved[k]=per.iter().zip(&base_per).filter(|(a,b)| a!=b).count();
            println!("{label:<14}{sz:>12}{mm:>13}{fw:>13}{ops:>13}{:>+10.4}%   work {:>+8.2}%  corpora moved {}/{}",
                100.0*(sz-b.0) as f64/b.0 as f64,
                100.0*(ops as i64-(b.1+b.2) as i64) as f64/(b.1+b.2) as f64,
                moved[k], base_per.len());
        }
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
