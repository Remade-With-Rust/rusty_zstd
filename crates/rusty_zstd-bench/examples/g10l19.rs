//! GATE 10 @ L19: the opt DP calls `try_rep1` at EVERY position, unconditionally.
//! What does it earn? Probes, hit rate, and bytes per probe -- the same exchange
//! rate that decided Gate 6 and Gate 2 @ L3.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    println!("{:<14}{:>12}{:>9}{:>11}", "corpus","rep probes","hit%","B/probe");
    let (mut tp,mut th,mut tb)=(0u64,0u64,0u64);
    let mut rows=vec![];
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_opt_rep();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (p,h,b)=rusty_zstd::take_opt_rep();
        rows.push((*id,p,h,b)); tp+=p; th+=h; tb+=b;
    }
    rows.sort_by(|a,b| (a.3 as f64/a.1.max(1) as f64).partial_cmp(&(b.3 as f64/b.1.max(1) as f64)).unwrap());
    for (id,p,h,b) in &rows {
        println!("{id:<14}{p:>12}{:>8.2}%{:>11.4}", 100.0*(*h) as f64/(*p).max(1) as f64, (*b) as f64/(*p).max(1) as f64);
    }
    println!("\nTOTAL {tp} probes, {th} hits ({:.2}%), {:.4} bytes/probe",
        100.0*th as f64/tp.max(1) as f64, tb as f64/tp.max(1) as f64);
}
