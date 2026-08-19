//! GATE 17 pushed to dispatch: min_match is a LEVEL constant, but its outcome
//! splits by CONTENT. Who wins both axes at mls=5, and what separates them?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","versions-16m","text-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: mls 7 -> 5, per corpus, both axes + candidate signals\n");
    println!("{:<14}{:>9}{:>10}{:>10}{:>10}{:>9}{:>10}","corpus","size %","seqs %","mean ml","lit/seq","rep yld","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let mut p7=rusty_zstd::compression_params(lvl,Some(src.len() as u64)).unwrap();
        p7.min_match=7;
        let mut p5=p7; p5.min_match=5;
        let _=rusty_zstd::take_rep_rate();
        let a=rusty_zstd::compress_with_params(src,p7,false).unwrap().len() as i64;
        let (_pr,_rb,rh,mb,sq)=rusty_zstd::take_rep_rate();
        let _=rusty_zstd::take_rep_rate();
        let b=rusty_zstd::compress_with_params(src,p5,false).unwrap().len() as i64;
        let (_,_,_,_,sq5)=rusty_zstd::take_rep_rate();
        if sq==0 {continue;}
        let mean_ml = mb as f64/sq as f64;
        let lit_per = (src.len() as f64 - mb as f64)/sq as f64;
        rows.push((*id, 100.0*(b-a) as f64/a as f64,
            100.0*(sq5 as f64-sq as f64)/sq as f64, mean_ml, lit_per,
            rh as f64/sq as f64));
    }
    rows.sort_by(|x,y|x.2.partial_cmp(&y.2).unwrap());
    for (id,sd,qd,ml,lp,ry) in &rows{
        let v=if *qd>100.0 {"seq blowup"} else if *qd<40.0 {"CHEAP"} else {""};
        println!("{id:<14}{sd:>+8.3}%{qd:>+9.1}%{ml:>10.2}{lp:>10.2}{ry:>9.3}{v:>10}");
    }
}
