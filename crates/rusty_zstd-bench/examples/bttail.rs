//! Does terminating the bt node on every exit (as C does) change size / work?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [13i32,16,19,22] {
        let (mut sz,mut bt)=(0usize,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_bt_calls();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            let (a,b)=rusty_zstd::take_bt_calls();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} L{lvl}");
            sz+=z.len(); bt+=a+b;
        }
        println!("L{lvl}\t{sz}\t{bt}");
    }
}
