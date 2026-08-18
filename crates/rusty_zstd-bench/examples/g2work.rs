//! What WORK does a DFast rep threshold remove? `try_rep1` is live at every
//! position of every block whose gate is on -- a deterministic count.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let t = std::env::args().nth(1).unwrap_or("0.005".into());
    println!("{:<14}{:>12}{:>12}{:>11}{:>10}", "corpus","rep pos @0.0",format!("@{t}"),"work saved","size");
    let (mut ta,mut tb)=(0u64,0u64); let (mut sa,mut sb)=(0usize,0usize);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        std::env::remove_var("RZSTD_REPMIN");
        let _=rusty_zstd::take_dfast_rep_blocks();
        let a=rusty_zstd::compress(src,3).unwrap().len();
        let (_,_,pa)=rusty_zstd::take_dfast_rep_blocks();
        std::env::set_var("RZSTD_REPMIN",&t);
        let _=rusty_zstd::take_dfast_rep_blocks();
        let b=rusty_zstd::compress(src,3).unwrap().len();
        let (_,_,pb)=rusty_zstd::take_dfast_rep_blocks();
        std::env::remove_var("RZSTD_REPMIN");
        println!("{id:<14}{pa:>12}{pb:>12}{:>10.1}%{:>9.3}%",
            if pa>0 {100.0*(pa-pb) as f64/pa as f64} else {0.0},
            100.0*(b as f64-a as f64)/a as f64);
        ta+=pa; tb+=pb; sa+=a; sb+=b;
    }
    println!("\nTOTAL rep positions {ta} -> {tb}  ({:.1}% of the probe work removed)", 100.0*(ta-tb) as f64/ta.max(1) as f64);
    println!("TOTAL size {:+.4}%", 100.0*(sb as f64-sa as f64)/sa as f64);
}
