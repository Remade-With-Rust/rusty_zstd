//! GATE 18 @ L1: target_length = 0 hands the step to the GATE 6/9 dispatch.
//! Setting it pins the step and DISABLES that dispatch. Per corpus at tlen=1,
//! and what step the dispatch actually chooses when left alone.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("L1, tlen 0 (dispatched step) -> 1 (pinned step 2)\n");
    println!("{:<14}{:>10}{:>12}{:>10}{:>12}","corpus","size %","positions","pos %","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        let mut p0=rusty_zstd::compression_params(1,Some(src.len() as u64)).unwrap();
        p0.target_length=0;
        let mut p1=p0; p1.target_length=1;
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress_with_params(src,p0,false).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress_with_params(src,p1,false).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        if pa==0 {continue;}
        rows.push((*id,100.0*(b-a) as f64/a as f64,pa as i64-pb as i64,
            100.0*(pb as f64-pa as f64)/pa as f64));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,saved,pd) in &rows{
        let v=if *sd<0.5 {"cheap"} else if *sd>5.0 {"EXPENSIVE"} else {""};
        println!("{id:<14}{sd:>+9.3}%{saved:>12}{pd:>+9.2}%{v:>12}");
    }
}
