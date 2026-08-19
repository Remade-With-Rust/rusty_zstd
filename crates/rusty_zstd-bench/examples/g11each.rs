//! Per-corpus effect of the L19 back-fill, with the jumped-position count that
//! predicts it. Absolute bytes too -- percentages on a 1.7 KB file mislead.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let st = std::env::args().nth(2).unwrap_or("1".into());
    println!("{:<14}{:>11}{:>11}{:>10}{:>11}{:>12}", "corpus","off","on","delta%","delta B","jumped pos");
    let (mut ta,mut tb)=(0i64,0i64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        std::env::remove_var("RZSTD_OPT_FILL");
        let _=rusty_zstd::take_opt_skips();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let (_,_,j,_)=rusty_zstd::take_opt_skips();
        std::env::set_var("RZSTD_OPT_FILL","1");
        std::env::set_var("RZSTD_OPT_FILL_S",&st);
        let z=rusty_zstd::compress(src,lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id}");
        std::env::remove_var("RZSTD_OPT_FILL");
        let b=z.len() as i64;
        println!("{id:<14}{a:>11}{b:>11}{:>9.3}%{:>11}{j:>12}", 100.0*(b-a) as f64/a as f64, b-a);
        ta+=a; tb+=b;
    }
    println!("\nTOTAL {ta} -> {tb}   {:+} bytes   {:+.4}%", tb-ta, 100.0*(tb-ta) as f64/ta as f64);
}
