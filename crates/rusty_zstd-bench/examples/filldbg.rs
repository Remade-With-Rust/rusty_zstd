fn main(){
    let id=std::env::args().nth(1).unwrap_or("versions-16m".into());
    let full=std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))).unwrap();
    let src=&full[..full.len().min(2<<20)];
    let _=rusty_zstd::compress(src,19).unwrap();
}
