//! Is compression DETERMINISTIC with respect to call history? Output must depend
//! only on (input, params) -- never on what was compressed before in the process.
const IDS: &[&str] = &["jsonlog-16m","mr","ooffice","reymont","sao","webster","mozilla","nci","x-ray","dickens","samba","xml"];
fn main(){
    for &lvl in &[19i32, 3, 1] {
        let mut bad=0;
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let a=rusty_zstd::compress(src,lvl).unwrap();
            let b=rusty_zstd::compress(src,lvl).unwrap();
            let c=rusty_zstd::compress(src,lvl).unwrap();
            if a!=b || b!=c {
                bad+=1;
                println!("  L{lvl} {id:<14} {} / {} / {}", a.len(), b.len(), c.len());
            }
        }
        println!("L{lvl}: {bad}/12 corpora NON-DETERMINISTIC across repeated calls");
    }
}
