//! The DP prices a literal at a flat 6 bits. Real literals cost ~8 raw. Does
//! correcting it fix the optimal parser LOSING to lazy?
const IDS: &[&str] = &["x-ray","sao","incomp-32m","mr","osdb","webster","dickens","xml","nci","mozilla","samba","jsonlog-16m"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(16);
    print!("{:<14}{:>10}", "corpus", "L15 ref");
    for c in [4u32,5,6,7,8,9,10] { print!("{:>9}", format!("lit{c}")); }
    println!();
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_opt_lit_arm(6);
        let l15=rusty_zstd::compress(src,15).unwrap().len();
        print!("{id:<14}{l15:>10}");
        for c in [4u32,5,6,7,8,9,10] {
            rusty_zstd::set_opt_lit_arm(c);
            let n=rusty_zstd::compress(src,lvl).unwrap().len();
            print!("{:>8.2}%", 100.0*(n as f64-l15 as f64)/l15 as f64);
        }
        println!();
    }
    rusty_zstd::set_opt_lit_arm(6);
    println!("\npercentages are vs L15 (BtLazy2). POSITIVE = the optimal parse LOSES to lazy.");
}
