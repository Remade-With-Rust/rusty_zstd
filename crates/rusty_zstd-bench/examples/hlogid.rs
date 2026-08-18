//! Byte-identity for the hash_log fix across the levels that use each finder.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [1i32,3,5,7,9,12,13,19] {
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(4<<20)];
            let z=rusty_zstd::compress(src,lvl).unwrap();
            println!("L{lvl}\t{id}\t{}", z.len());
        }
    }
}
