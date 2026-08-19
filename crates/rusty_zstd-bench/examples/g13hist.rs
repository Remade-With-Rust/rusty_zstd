//! Is LIT_PUSH_WIDTH = 16 the right width? Distribution of literal run lengths
//! at the sites GATE 13 serves.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:<14}{:>9}{:>9}{:>9}{:>9}{:>9}{:>9}","corpus","0-4","5-8","9-16","17-32","33-64","65+");
    let mut tot=[0u64;6];
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_lit_hist();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let h=rusty_zstd::take_lit_hist();
        let s:u64=h.iter().sum();
        for i in 0..6 {tot[i]+=h[i];}
        println!("{id:<14}{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%",
            100.0*h[0] as f64/s.max(1) as f64,100.0*h[1] as f64/s.max(1) as f64,
            100.0*h[2] as f64/s.max(1) as f64,100.0*h[3] as f64/s.max(1) as f64,
            100.0*h[4] as f64/s.max(1) as f64,100.0*h[5] as f64/s.max(1) as f64);
    }
    let s:u64=tot.iter().sum();
    println!("\n{:<14}{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%","TOTAL",
        100.0*tot[0] as f64/s as f64,100.0*tot[1] as f64/s as f64,
        100.0*tot[2] as f64/s as f64,100.0*tot[3] as f64/s as f64,
        100.0*tot[4] as f64/s as f64,100.0*tot[5] as f64/s as f64);
    let le8=tot[0]+tot[1]; let le16=le8+tot[2]; let le32=le16+tot[3];
    println!("\ncumulative: <=8 {:.1}%   <=16 {:.1}%   <=32 {:.1}%",
        100.0*le8 as f64/s as f64, 100.0*le16 as f64/s as f64, 100.0*le32 as f64/s as f64);
}
