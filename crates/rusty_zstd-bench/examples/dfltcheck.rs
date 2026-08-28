//! Is the demo's call byte-identical to our DEFAULT entry point?
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","zeros-32m","incomp-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let mut bad=0; let mut n=0;
    for lvl in [1i32,3,9,19]{
        for id in IDS{
            let Some(f)=load(id) else{continue};
            let s=&f[..f.len().min(8<<20)];
            let a=rusty_zstd::compress(s,lvl).unwrap();              // our DEFAULT
            let p=rusty_zstd::compression_params(lvl,Some(s.len() as u64)).unwrap();
            let b=rusty_zstd::compress_with_params(s,p,true).unwrap(); // the demo's ship arm
            n+=1;
            if a!=b { println!("  DIFFER L{lvl} {id}: default {} B vs demo {} B",a.len(),b.len()); bad+=1; }
        }
    }
    println!("  {n} (level,corpus) cells, {bad} differences");
}
