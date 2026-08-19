//! Does the fixed-width path actually engage at L3?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("{:<14}{:>13}{:>13}{:>10}","corpus","fixed-width","fallback","fast %");
    let (mut tf,mut ts)=(0u64,0u64);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_lit_push();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (f,s)=rusty_zstd::take_lit_push();
        tf+=f; ts+=s;
        println!("{id:<14}{f:>13}{s:>13}{:>9.1}%",100.0*f as f64/(f+s).max(1) as f64);
    }
    println!("\nTOTAL fixed-width {tf}, fallback {ts} -> {:.1}% served by the constant-width move",
        100.0*tf as f64/(tf+ts).max(1) as f64);
}
