//! GATE 18 @ L3: x-ray is 4.7x faster at mls=8 while most corpora get slower.
//! What separates it? mean match length is already tracked (dfast_mean_ml, used
//! by GATE 9's step dispatch) -- test it as the axis.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("L3: mls 5 -> 8, per corpus, against mean match length\n");
    println!("{:<13}{:>9}{:>9}{:>9}{:>10}{:>11}","corpus","mean ml","size %","pos %","seqs %","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let mut p5=rusty_zstd::compression_params(3,Some(src.len() as u64)).unwrap();
        p5.min_match=5;
        let mut p8=p5; p8.min_match=8;
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
        let a=rusty_zstd::compress_with_params(src,p5,false).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        let (mb,sq,_bb,_rb,_rh)=rusty_zstd::take_dfast_match_stats();
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
        let b=rusty_zstd::compress_with_params(src,p8,false).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        let (_,sq8,_,_,_)=rusty_zstd::take_dfast_match_stats();
        if sq==0 {continue;}
        rows.push((*id, mb as f64/sq as f64,
            100.0*(b-a) as f64/a as f64,
            100.0*(pb as f64-pa as f64)/pa as f64,
            100.0*(sq8 as f64-sq as f64)/sq as f64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,ml,sd,pd,qd) in &rows{
        let v=if *ml<7.0 {"LOW ml"} else {""};
        println!("{id:<13}{ml:>9.2}{sd:>+8.3}%{pd:>+8.2}%{qd:>+9.2}%{v:>11}");
    }
}
