//! Incompressible content paid the full match search before anything noticed.
//! Measure: bt calls, size, round-trip, across levels.
const IDS: &[&str] = &["incomp-32m","x-ray","sao","mozilla","jsonlog-16m","webster","dickens","mr","nci","versions-16m","text-32m","zeros-32m","samba","xml","osdb","reymont","ooffice","smallmsg-8m"];
fn main(){
    for lvl in [22i32,19,13,3,1] {
        let (mut tc,mut tsz)=(0u64,0usize); let mut worst=0.0f64; let mut wc="";
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_opt_bt();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            let (c,_,_,_)=rusty_zstd::take_opt_bt();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} L{lvl}");
            tc+=c; tsz+=z.len();
            let _=(&mut worst,&mut wc);
        }
        println!("L{lvl:<3} opt bt calls {tc:>12}   total size {tsz}");
    }
}
