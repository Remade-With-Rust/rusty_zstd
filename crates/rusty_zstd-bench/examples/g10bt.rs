//! What does find_opt's per-position bt search actually return at L22?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    println!("{:<14}{:>12}{:>9}{:>12}{:>10}", "corpus","bt calls","dry%","seqs kept","calls/seq");
    let (mut tc,mut td,mut ts)=(0u64,0u64,0u64);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_opt_bt();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (c,d,_l,sq)=rusty_zstd::take_opt_bt();
        println!("{id:<14}{c:>12}{:>8.1}%{sq:>12}{:>10.1}",
            100.0*d as f64/c.max(1) as f64, c as f64/sq.max(1) as f64);
        tc+=c; td+=d; ts+=sq;
    }
    println!("\nTOTAL {tc} bt calls, {td} returned nothing ({:.1}%), {ts} sequences kept -> {:.1} calls per emitted sequence",
        100.0*td as f64/tc.max(1) as f64, tc as f64/ts.max(1) as f64);
}
