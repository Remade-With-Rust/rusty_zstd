//! GATE 12 @ L3: the FULL work ledger for reducing inserts. 4.39 counted only
//! the main-loop positions the sparse arm costs and missed the larger term --
//! the table writes it removes.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let arms=[(2u8,"both (today)"),(1,"drop end-2"),(3,"drop start+2"),(0,"drop both")];
    println!("{:<14}{:>12}{:>13}{:>13}{:>13}{:>11}","arm","size","mainloop pos","endfill wr","NET work ops","size %");
    let mut b=(0i64,0u64,0u64);
    for (n,label) in arms{
        let (mut sz,mut mm,mut fw)=(0i64,0u64,0u64);
        for id in IDS{
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_dfast_fill_n_arm(n);
            let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_endfill();
            sz+=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
            mm+=rusty_zstd::take_mm().0;
            fw+=rusty_zstd::take_dfast_endfill();
        }
        // a main-loop position issues 2 hashes + table reads; a fill write is
        // 1 hash + 1 store. Count both as one "op" -- the ranking is what matters.
        let ops=mm+fw;
        if n==2 {b=(sz,mm,fw);
            println!("{label:<14}{sz:>12}{mm:>13}{fw:>13}{ops:>13}{:>11}","-");
        } else {
            println!("{label:<14}{sz:>12}{mm:>13}{fw:>13}{ops:>13}{:>+10.4}%   net ops {:>+10} ({:+.2}%)",
                100.0*(sz-b.0) as f64/b.0 as f64, ops as i64-(b.1+b.2) as i64,
                100.0*(ops as i64-(b.1+b.2) as i64) as f64/(b.1+b.2) as f64);
        }
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
