//! Is `bt_find_best_runtime` reachable? It is a SECOND hand-written copy of the
//! bt walk -- the same drift hazard that let `find_dfast_runtime` fall out of
//! sync until Gate 6 silently broke Gate 4's byte-identity.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for lvl in [13i32,14,15,16,17,18,19,20,21,22] {
        let (mut sp,mut rt)=(0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(1<<20)];
            let _=rusty_zstd::take_bt_calls();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (a,b)=rusty_zstd::take_bt_calls();
            sp+=a; rt+=b;
        }
        println!("L{lvl:<3} specialised {sp:>9}   runtime {rt:>9}{}", if rt>0 {"  <- REACHABLE"} else {""});
    }
}
