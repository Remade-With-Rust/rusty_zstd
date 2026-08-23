//! ALLOC-8 probe: how often does the speculative EntropyState save get USED?
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("{:<8}{:>12}{:>12}{:>12}","level","saves","rollbacks","used%");
    for lvl in [1i32,3,9,19] {
        let _=rusty_zstd::take_ent_save();
        for id in IDS { let Some(f)=load(id) else{continue};
            let _=rusty_zstd::compress(&f[..f.len().min(8<<20)],lvl).unwrap(); }
        let e=rusty_zstd::take_ent_save();
        println!("{:<8}{:>12}{:>12}{:>11.2}%",format!("L{lvl}"),e[0],e[1],
            if e[0]>0 {100.0*e[1] as f64/e[0] as f64} else {0.0});
    }
}
