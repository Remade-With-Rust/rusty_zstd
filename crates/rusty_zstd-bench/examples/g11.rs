//! GATE 11 (`lazy_fill_enabled` / RZSTD_LAZY_FILL) protocol step 1, across the
//! levels the back-fill can actually reach. It is gated `strategy != Fast`, and
//! there is a SECOND site in find_bt_lazy (L13-L15) that regate never covers.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    for &lvl in &[3i32,1,19,22,5,7,9,12,13,14,15] {
        let (mut moved,mut bt_on,mut bt_off)=(0,0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            rusty_zstd::set_lazy_fill_arm(true);
            let _=rusty_zstd::take_bt_calls();
            let a=rusty_zstd::compress(src,lvl).unwrap().len();
            let (s1,r1)=rusty_zstd::take_bt_calls();
            rusty_zstd::set_lazy_fill_arm(false);
            let _=rusty_zstd::take_bt_calls();
            let z=rusty_zstd::compress(src,lvl).unwrap();
            let (s2,r2)=rusty_zstd::take_bt_calls();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id} L{lvl}");
            if a!=z.len() { moved+=1; }
            bt_on+=s1+r1; bt_off+=s2+r2;
        }
        rusty_zstd::set_lazy_fill_arm(true);
        println!("L{lvl:<3} sizes move {moved:>2}/18   bt calls ON {bt_on:>11} OFF {bt_off:>11}{}",
            if bt_on==0 && bt_off==0 {"   (no bt at this level)"} else if moved==0 {"   <- byte-identical, work differs?"} else {""});
    }
}
