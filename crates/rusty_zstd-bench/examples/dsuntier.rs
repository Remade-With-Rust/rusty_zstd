//! Length distribution of the ONE un-tiered match-copy band.
//!
//! `extend_from_within` carries ~34% of all match bytes at a mean of ~125, but a
//! mean cannot choose a tier width. This prints the distribution so the width is
//! read off data instead of guessed.
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
const B: [&str; 8] = ["<=16","17-32","33-64","65-128","129-256","257-512","513-1024",">1024"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let cap = 8usize<<20;
    let mut tot = [0u64;16];
    for id in IDS {
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        let z=rusty_zstd::compress(s,lvl).unwrap();
        let _=rusty_zstd::take_dec_untiered();
        let out=rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out.len(),s.len());
        let h=rusty_zstd::take_dec_untiered();
        for i in 0..16 { tot[i]+=h[i]; }
    }
    let calls: u64 = tot[..8].iter().sum();
    let bytes: u64 = tot[8..].iter().sum();
    println!("`extend_from_within` band @ L{lvl}: {calls} calls, {bytes} bytes, mean {:.1}\n",
        if calls>0 {bytes as f64/calls as f64} else {0.0});
    println!("| length | calls | % calls | bytes | % bytes | cum % bytes |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    let mut cum=0f64;
    for i in 0..8 {
        let pb = if bytes>0 {100.0*tot[8+i] as f64/bytes as f64} else {0.0};
        cum += pb;
        println!("| {} | {} | {:.1} | {} | {:.1} | {:.1} |", B[i], tot[i],
            if calls>0 {100.0*tot[i] as f64/calls as f64} else {0.0}, tot[8+i], pb, cum);
    }
}
