//! What does the back-fill BUY for 62% of the bt work at L13-L15?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    println!("{:<14}{:>12}{:>12}{:>10}{:>11}", "corpus","bt ON","bt OFF","work cut","size if OFF");
    let (mut ta,mut tb)=(0u64,0u64); let (mut sa,mut sb)=(0usize,0usize);
    let mut rows=vec![];
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
        let b=rusty_zstd::compress(src,lvl).unwrap().len();
        let (s2,r2)=rusty_zstd::take_bt_calls();
        rusty_zstd::set_lazy_fill_arm(true);
        let (on,off)=(s1+r1,s2+r2);
        rows.push((*id,on,off,100.0*(b as f64-a as f64)/a as f64));
        ta+=on; tb+=off; sa+=a; sb+=b;
    }
    rows.sort_by(|x,y| y.3.partial_cmp(&x.3).unwrap());
    for (id,on,off,d) in &rows {
        println!("{id:<14}{on:>12}{off:>12}{:>9.1}%{d:>10.3}%",
            100.0*(*on as f64-*off as f64)/(*on).max(1) as f64);
    }
    println!("\nTOTAL bt {ta} -> {tb} ({:.1}% cut)   size {:+.4}%",
        100.0*(ta-tb) as f64/ta as f64, 100.0*(sb as f64-sa as f64)/sa as f64);
}
