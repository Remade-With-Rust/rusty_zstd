//! GATE 3, measurable for the first time. What IS `last_search_per_byte` per
//! corpus, and does it separate the content where the back-fill pays?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(7);
    println!("{:<14}{:>12}{:>12}{:>11}", "corpus","inserts","size if OFF","B/insert");
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_lazy_fill_arm(true);
        let _=rusty_zstd::take_lazy_fill();
        let on=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let (_,_,ins)=rusty_zstd::take_lazy_fill();
        rusty_zstd::set_lazy_fill_arm(false);
        let off=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        rusty_zstd::set_lazy_fill_arm(true);
        // bytes SAVED per insert spent -- the exchange rate that decided Gates 6, 2 and 10
        let rate = if ins>0 {(off-on) as f64/ins as f64} else {0.0};
        rows.push((*id, ins, off-on, rate));
    }
    rows.sort_by(|a,b| a.3.partial_cmp(&b.3).unwrap());
    for (id,ins,d,r) in &rows { println!("{id:<14}{ins:>12}{d:>+12}{r:>11.4}"); }
    let ti: i64 = rows.iter().map(|r| r.1 as i64).sum();
    let td: i64 = rows.iter().map(|r| r.2).sum();
    println!("\nTOTAL inserts {ti}, bytes saved {td} -> {:.4} bytes per insert", td as f64/ti.max(1) as f64);
}
