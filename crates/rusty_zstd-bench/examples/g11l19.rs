//! GATE 11 @ L19: is there anything for a back-fill to fill? Count the positions
//! the DP never inserts.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","versions-16m","text-32m","incomp-32m"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let (mut tp,mut ti,mut tj,mut tn)=(0u64,0u64,0u64,0u64);
    println!("{:<14}{:>12}{:>12}{:>12}{:>9}", "corpus","positions","price=inf","jumped","jumps");
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_opt_skips();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (p,i,j,nj)=rusty_zstd::take_opt_skips();
        println!("{id:<14}{p:>12}{i:>12}{j:>12}{nj:>9}");
        tp+=p; ti+=i; tj+=j; tn+=nj;
    }
    println!("\nTOTAL positions {tp}");
    println!("  never inserted via price=inf : {ti} ({:.3}%)", 100.0*ti as f64/tp.max(1) as f64);
    println!("  never inserted via the jump  : {tj} ({:.3}%) over {tn} jumps", 100.0*tj as f64/tp.max(1) as f64);
}
