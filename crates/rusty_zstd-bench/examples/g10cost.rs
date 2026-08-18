//! What does the opt DP's repcode candidate BUY? Size with it on vs off, at L19,
//! against the bytes-per-probe it earns.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("{:<14}{:>11}{:>12}", "corpus","B/probe","size if OFF");
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_opt_rep_arm(true);
        let _=rusty_zstd::take_opt_rep();
        let on=rusty_zstd::compress(src,lvl).unwrap().len() as f64;
        let (p,_h,b)=rusty_zstd::take_opt_rep();
        rusty_zstd::set_opt_rep_arm(false);
        let z=rusty_zstd::compress(src,lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),src,"{id}");
        rusty_zstd::set_opt_rep_arm(true);
        rows.push((*id, b as f64/p.max(1) as f64, 100.0*(z.len() as f64-on)/on, p));
    }
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    let (mut wp,mut tp)=(0u64,0u64);
    for (id,r,d,p) in &rows {
        println!("{id:<14}{r:>11.4}{d:>11.3}%", );
        tp+=p; if *r < 1.0 { wp+=p; }
    }
    println!("\nprobes on corpora below 1.0 B/probe: {wp} of {tp}  ({:.1}%)", 100.0*wp as f64/tp.max(1) as f64);
}
