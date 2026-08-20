//! Does GATE 5 (the shipped per-block block_max dispatch) REACH the corpora that
//! the GATE 19 sweep says want a smaller block?
const IDS:&[(&str,&str)]=&[("mr","wants 32KB"),("mozilla","wants 32KB"),("sao","wants 16KB"),
 ("xml","wants 64KB"),("ooffice","wants 48KB"),("samba","wants 48KB"),("osdb","wants 96KB"),
 ("x-ray","wants 64KB"),("nci","wants 96KB"),("webster","wants 128KB"),("dickens","wants 128KB"),
 ("versions-16m","wants 128KB"),("text-32m","wants 128KB")];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    println!("GATE 5 coverage vs what GATE 19's sweep says each corpus wants\n");
    println!("{:<14}{:>13}{:>8}{:>9}{:>8}{:>10}","corpus","sweep says","blocks","raw-esc","drift","reduced %");
    for (id,want) in IDS{
        let Some(f)=load(id) else{continue};
        let src=&f[..f.len().min(8<<20)];
        let _=rusty_zstd::take_g5();
        let _=rusty_zstd::compress(src,3).unwrap();
        let (c,r,d)=rusty_zstd::take_g5();
        println!("{id:<14}{want:>13}{c:>8}{r:>9}{d:>8}{:>9.1}%",
            if c>0 {100.0*(r+d) as f64/c as f64} else {0.0});
    }
}
