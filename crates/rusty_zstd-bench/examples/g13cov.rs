//! Engagement by tier: how many literal appends still reach extend_from_slice?
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao","x-ray"];
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("{:<14}{:>11}{:>10}{:>10}{:>10}{:>11}","corpus","tier1(16)","t2(32)","t3(64)","slow","served %");
    let (mut a,mut b,mut c,mut d)=(0u64,0u64,0u64,0u64);
    for id in IDS{
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_|std::fs::read(format!("corpora/data/generated/{id}"))) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_lit_push(); let _=rusty_zstd::take_lit_tiers();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (f1,slow)=rusty_zstd::take_lit_push();
        let (f2,f3)=rusty_zstd::take_lit_tiers();
        let tot=f1+f2+f3+slow;
        a+=f1; b+=f2; c+=f3; d+=slow;
        println!("{id:<14}{f1:>11}{f2:>10}{f3:>10}{slow:>10}{:>10.1}%",
            100.0*(f1+f2+f3) as f64/tot.max(1) as f64);
    }
    let tot=a+b+c+d;
    println!("\nTOTAL tier1 {a}, tier2 {b}, tier3 {c}, still slow {} -> {:.1}% served",
        d.saturating_sub(b+c), 100.0*(a+b+c) as f64/tot.max(1) as f64);
}
