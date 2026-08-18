const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [16i32,19,22] {
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let z=rusty_zstd::compress(src,lvl).unwrap();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} L{lvl}");
            println!("L{lvl}\t{id}\t{}", z.len());
        }
    }
}
