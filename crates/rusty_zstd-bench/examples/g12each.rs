//! GATE 12 @ L3: per-corpus cost of dropping the `match_end-2` fill (the cheaper
//! of the two). If any content is neutral, that is 2 of DFast's 4 per-match
//! table writes removed there -- a dispatch, not a constant.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:<14}{:>11}{:>11}{:>10}{:>12}{:>10}","corpus","both","no end-2","size %","pos delta","ml");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_fill_n_arm(2);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        let (mb,sq,_,_,_)=rusty_zstd::take_dfast_match_stats();
        rusty_zstd::set_dfast_fill_n_arm(1);
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        let ml=if sq>0 {mb as f64/sq as f64} else {0.0};
        rows.push((*id,a,b,100.0*(b-a) as f64/a as f64,pb as i64-pa as i64,ml));
    }
    rows.sort_by(|x,y| x.3.partial_cmp(&y.3).unwrap());
    for (id,a,b,d,dp,ml) in &rows{
        let tag=if *d<=0.001 {"  FREE"} else if *d<0.1 {"  cheap"} else {""};
        println!("{id:<14}{a:>11}{b:>11}{d:>+9.3}%{dp:>+12}{ml:>10.1}{tag}");
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
