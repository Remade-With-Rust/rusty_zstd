//! What did fixing the pipelined loop's stale `rep1` buy, at DEFAULT settings?
//! Compares the current binary against the sizes recorded at HEAD (pre-fix).
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let z=rusty_zstd::compress(src,lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} round-trip");
        println!("{id}\t{}", z.len());
    }
}
