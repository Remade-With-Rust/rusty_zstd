//! N9 ceiling probe: how often is the RFC-constant default FSE ctable rebuilt?
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap=8usize<<20;
    println!("{:<7}{:>14}{:>14}{:>12}","level","rebuilds","MiB in","per MiB");
    for lvl in [1i32,3,9,19] {
        let _=rusty_zstd::take_n9_basic();
        let mut n=0usize;
        for id in IDS { let Some(f)=load(id) else{continue}; let src=&f[..f.len().min(cap)]; n+=src.len();
            let _=rusty_zstd::compress(src,lvl).unwrap(); }
        let b=rusty_zstd::take_n9_basic();
        println!("{:<7}{:>14}{:>14}{:>12.1}",format!("L{lvl}"),b,n>>20,b as f64/((n>>20) as f64));
    }
}
