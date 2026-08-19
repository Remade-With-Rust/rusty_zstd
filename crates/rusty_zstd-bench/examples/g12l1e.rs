//! Which 4 corpora does GATE 12 @ L1 not move, and WHY? Per-corpus response to
//! every fill arm, alongside how much fill work each one actually performs.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("{:<14}{:>10}{:>10}{:>10}{:>12}{:>12}{:>9}","corpus","both","drop e2","drop both","fill writes","mainloop","resp");
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_dfast_fill_n_arm(2);
        let _=rusty_zstd::take_mm(); let _=rusty_zstd::take_dfast_endfill();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let mm=rusty_zstd::take_mm().0; let fw=rusty_zstd::take_dfast_endfill();
        rusty_zstd::set_dfast_fill_n_arm(1);
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        rusty_zstd::set_dfast_fill_n_arm(0);
        let c=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let resp=if a==b && a==c {"DEAD"} else {""};
        println!("{id:<14}{a:>10}{:>+10}{:>+10}{fw:>12}{mm:>12}{resp:>9}",b-a,c-a);
    }
    rusty_zstd::set_dfast_fill_n_arm(2);
}
