//! GATE 14 @ L3 step 2: does the outcome differ by CONTENT? Per corpus at the
//! candidate value, with the signals the encoder already measures.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let g:usize=std::env::args().nth(2).and_then(|s|s.parse().ok()).unwrap_or(24);
    println!("good_ml 8 -> {g} at L{lvl}\n");
    println!("{:<14}{:>10}{:>10}{:>10}{:>10}{:>9}{:>9}","corpus","size %","probe %","mean ml","rep yld","seqs","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_good_ml_arm(0);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_match_stats();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        let (mb,sq,_bb,_rb,rh)=rusty_zstd::take_dfast_match_stats();
        rusty_zstd::set_dfast_good_ml_arm(g);
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        if sq==0 {continue;}
        let sd=100.0*(b-a) as f64/a as f64;
        let pd=if pa>0 {100.0*(pb as f64-pa as f64)/pa as f64} else {0.0};
        rows.push((*id,sd,pd,mb as f64/sq as f64, rh as f64/sq as f64, sq));
    }
    rows.sort_by(|x,y| x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,pd,ml,ry,sq) in &rows{
        let v=if *sd>0.05 {"LOSES"} else if *sd< -0.02 {"wins"} else {""};
        println!("{id:<14}{sd:>+9.3}%{pd:>+9.2}%{ml:>10.2}{ry:>10.3}{sq:>9}  {v}");
    }
    rusty_zstd::set_dfast_good_ml_arm(0);
}
