//! GATE 12 (`lazy_fill_stride` / RZSTD_LAZY_FILL_S) protocol step 1.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for &lvl in &[3i32,1,7,9,13,19,22] {
        let mut moved=0;
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            std::env::remove_var("RZSTD_LAZY_FILL_S");
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            std::env::set_var("RZSTD_LAZY_FILL_S","4");
            let b=rusty_zstd::compress(src,lvl).unwrap().len();
            std::env::remove_var("RZSTD_LAZY_FILL_S");
            if a!=b { moved+=1; }
        }
        println!("L{lvl:<3} stride 1->4 moves {moved:>2}/18{}", if moved==0 {"   -> GATE 12 IS DEAD HERE"} else {""});
    }
}
