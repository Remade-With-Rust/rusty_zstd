//! Per-corpus response to the acceleration shift at L1, plus a timed A/B.
//! Positions are DEPENDENT work, so unlike 4.40's fills this should move time.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("=== per corpus at L1, shift 8 -> 7 ===");
    println!("{:<14}{:>11}{:>11}{:>10}{:>13}{:>10}","corpus","shift 8","shift 7","size %","positions","pos %");
    let mut rows=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_accel_shift_arm(8);
        let _=rusty_zstd::take_mm();
        let a=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pa=rusty_zstd::take_mm().0;
        rusty_zstd::set_accel_shift_arm(7);
        let _=rusty_zstd::take_mm();
        let b=rusty_zstd::compress(src,1).unwrap().len() as i64;
        let pb=rusty_zstd::take_mm().0;
        let sd=100.0*(b-a) as f64/a as f64;
        let pd=if pa>0 {100.0*(pb as f64-pa as f64)/pa as f64} else {0.0};
        rows.push((*id,a,b,sd,pa,pd));
    }
    rows.sort_by(|x,y|x.3.partial_cmp(&y.3).unwrap());
    for (id,a,b,sd,pa,pd) in &rows{
        let m=if *sd<=0.0010 {"  FREE"} else {""};
        println!("{id:<14}{a:>11}{b:>11}{sd:>+9.3}%{pa:>13}{pd:>+9.1}%{m}");
    }
    rusty_zstd::set_accel_shift_arm(8);
}
