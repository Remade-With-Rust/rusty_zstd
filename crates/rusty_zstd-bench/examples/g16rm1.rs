//! GATE 16 @ L1: RAW_RUN_MIN = 1 saves 1.05% of positions but costs 0.1336%
//! size. Who wins, who loses, and does any signal separate them?
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("L{lvl}: RAW_RUN_MIN 2 -> 1\n");
    println!("{:<14}{:>10}{:>12}{:>10}{:>12}","corpus","size %","positions","pos %","verdict");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_raw_skip_arm(true);
        rusty_zstd::set_raw_run_min_arm(2);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        rusty_zstd::set_raw_run_min_arm(1);
        let _=rusty_zstd::take_mm();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let pb=rusty_zstd::take_mm().0;
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "roundtrip {id}");
        let b=z.len() as i64;
        if a==b && pa==pb {continue;}
        rows.push((*id,100.0*(b-a) as f64/a as f64, pa as i64-pb as i64,
            if pa>0 {100.0*(pb as f64-pa as f64)/pa as f64} else {0.0}));
    }
    rows.sort_by(|x,y|x.1.partial_cmp(&y.1).unwrap());
    for (id,sd,saved,pd) in &rows{
        let v=if *sd>0.05 {"COSTS"} else if *saved>0 {"free win"} else {""};
        println!("{id:<14}{sd:>+9.4}%{saved:>12}{pd:>+9.2}%{v:>12}");
    }
    rusty_zstd::set_raw_run_min_arm(0);
}
