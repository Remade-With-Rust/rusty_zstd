//! "Gate >32B through AVX2, the rest through baseline" -- is that already the
//! structure? Anything past the fixed-width tiers falls through to
//! extend_from_slice / extend_from_within, i.e. memcpy, which dispatches to the
//! widest vector width internally. Measure the BYTE split, not the call split.
const IDS:&[&str]=&["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    // encoder literal appends, by bucket -- the histogram already exists
    let mut h=[0u64;5];
    let (mut t1,mut t2,mut t3,mut slow)=(0u64,0u64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        let _=rusty_zstd::take_lit_hist(); let _=rusty_zstd::take_lit_push();
        let _=rusty_zstd::take_lit_tiers();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let g=rusty_zstd::take_lit_hist();
        for i in 0..5 {h[i]+=g[i];}
        let (a,s)=rusty_zstd::take_lit_push(); let (b,c)=rusty_zstd::take_lit_tiers();
        t1+=a; t2+=b; t3+=c; slow+=s;
    }
    let calls=t1+t2+t3+slow;
    println!("L{lvl} encoder literal appends: {calls} calls\n");
    println!("{:<28}{:>12}{:>10}{:>26}","path","calls","share","who does the widening");
    println!("{:<28}{:>12}{:>9.1}%{:>26}","tier1 inline (<=16B)",t1,100.0*t1 as f64/calls as f64,"nobody -- 1 movups is optimal");
    println!("{:<28}{:>12}{:>9.1}%{:>26}","tier2 inline (<=32B)",t2,100.0*t2 as f64/calls as f64,"could widen, saves 2");
    println!("{:<28}{:>12}{:>9.1}%{:>26}","tier3 inline (<=64B)",t3,100.0*t3 as f64/calls as f64,"could widen, saves 4");
    println!("{:<28}{:>12}{:>9.1}%{:>26}","fallback -> memcpy",slow,100.0*slow as f64/calls as f64,"MEMCPY: already widest");
    // approximate bytes per bucket using the bucket midpoints
    let mid=[2u64,6,12,24,48,200];
    let names=["0-4","5-8","9-16","17-32","33-64","65+"];
    let tot:u64=h.iter().sum();
    println!("\nrun-length buckets (all appends), and the BYTES they carry:");
    let mut bytes=[0u64;5];
    for i in 0..5 {bytes[i]=h[i]*mid[i];}
    let tb:u64=bytes.iter().sum();
    for i in 0..5 {
        println!("  {:<8}{:>12} calls ({:>5.1}%)   ~{:>11} bytes ({:>5.1}%)",
            names[i],h[i],100.0*h[i] as f64/tot.max(1) as f64,bytes[i],100.0*bytes[i] as f64/tb.max(1) as f64);
    }
}
