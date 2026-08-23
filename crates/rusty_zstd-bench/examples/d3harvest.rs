//! D3 harvest: the plan says HARVEST BEFORE BUILDING, because the 64-byte tier
//! was sized from this histogram and its MEAN would have chosen the wrong width.
//! Band 4 = the overlapping chunked path D3 targets (offset < len).
const IDS: &[&str] = &["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice","jsonlog-16m","text-32m","zeros-32m"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap = 8usize<<20;
    let lvl: i32 = std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let _=rusty_zstd::take_dec_bands(); let _=rusty_zstd::take_dec_untiered(); let _=rusty_zstd::take_d3_iters(); let _=rusty_zstd::take_d6_spread(); let _=rusty_zstd::take_n21_predef();
    let mut n=0usize;
    for id in IDS {
        let Some(f)=load(id) else {continue};
        let src=&f[..f.len().min(cap)]; n+=src.len();
        let z=rusty_zstd::compress(src,lvl).unwrap();
        let out=rusty_zstd::decompress(&z).unwrap();
        assert_eq!(out,src,"{id}");
    }
    let d3it=rusty_zstd::take_d3_iters();
    let n21=rusty_zstd::take_n21_predef();
    println!("N21 PROBE: {n21} rebuilds of RFC-constant Predefined FSE tables");
    let d6=rusty_zstd::take_d6_spread();
    println!("D6 PROBE: {} dtable builds, {} with NO low-prob symbols ({:.1}% eligible for the zstd spread fast path)",
        d6[0], d6[1], if d6[0]>0 {100.0*d6[1] as f64/d6[0] as f64} else {0.0});
    println!("          entries the fast path would cover: {} ({:.0} per eligible build)",
        d6[2], if d6[1]>0 {d6[2] as f64/d6[1] as f64} else {0.0});
    let (calls,bytes)=rusty_zstd::take_dec_bands();
    let u=rusty_zstd::take_dec_untiered();
    println!("L{lvl}, {} MiB decoded\n", n>>20);
    let tc: u64 = calls.iter().sum(); let tb: u64 = bytes.iter().sum();
    println!("{:<8}{:>14}{:>9}{:>16}{:>9}{:>10}","band","calls","calls%","bytes","bytes%","mean len");
    for i in 0..8 {
        if calls[i]==0 && bytes[i]==0 { continue }
        println!("{:<8}{:>14}{:>8.2}%{:>16}{:>8.2}%{:>10.1}", i, calls[i],
            100.0*calls[i] as f64/tc as f64, bytes[i],
            100.0*bytes[i] as f64/tb as f64,
            if calls[i]>0 {bytes[i] as f64/calls[i] as f64} else {0.0});
    }
    println!("
D3 PROBE: band-4 made {d3it} extend_from_within calls for {} band-4 calls -> {:.2} per call",
        calls[4], if calls[4]>0 {d3it as f64/calls[4] as f64} else {0.0});
    const LAB:[&str;8]=["<=16","17-32","33-64","65-128","129-256","257-512","513-1024",">1024"];
    println!("\nUN-TIERED length histogram (band3=extend_from_within, band4=overlapping):");
    println!("{:<12}{:>14}{:>16}","bucket","calls","bytes");
    for i in 0..8 {
        if u[i]==0 && u[i+8]==0 { continue }
        println!("{:<12}{:>14}{:>16}", LAB[i], u[i], u[i+8]);
    }
}
