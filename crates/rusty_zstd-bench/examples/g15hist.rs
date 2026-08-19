//! GATE 15: what prefix lengths does count_match actually get, and how often is
//! the 64-byte wide path even eligible? Deterministic, since the clock cannot
//! resolve this.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}: prefix length returned by count_eq_len\n");
    println!("{:<14}{:>12}{:>9}{:>9}{:>9}{:>9}{:>9}{:>10}","corpus","calls","<8","8-31","32-63","64-255","256+","wide elig");
    let (mut tc,mut tw)=(0u64,0u64); let mut th=[0u64;5];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_eqlen_stats();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (c,w,h)=rusty_zstd::take_eqlen_stats();
        if c==0 {continue;}
        let s:u64=h.iter().sum();
        tc+=c; tw+=w; for i in 0..5 {th[i]+=h[i];}
        println!("{id:<14}{c:>12}{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>9.1}%",
            100.0*h[0] as f64/s.max(1) as f64,100.0*h[1] as f64/s.max(1) as f64,
            100.0*h[2] as f64/s.max(1) as f64,100.0*h[3] as f64/s.max(1) as f64,
            100.0*h[4] as f64/s.max(1) as f64,100.0*w as f64/c as f64);
    }
    let s:u64=th.iter().sum();
    println!("\n{:<14}{tc:>12}{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>8.1}%{:>9.1}%","TOTAL",
        100.0*th[0] as f64/s as f64,100.0*th[1] as f64/s as f64,
        100.0*th[2] as f64/s as f64,100.0*th[3] as f64/s as f64,
        100.0*th[4] as f64/s as f64,100.0*tw as f64/tc as f64);
}
