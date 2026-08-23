//! Does the ENCODER build X2 Huffman tables it never decodes with?
//!
//! `HuffCTable.table` is `#[allow(dead_code)]` and documented as "Decode twin
//! kept as the test oracle". Release code reads only its X1 half, at build
//! time, to derive `entry[]`. The X2 half is a 2048-entry data-dependent gather
//! plus an 8 KiB allocation. Count the builds that happen during ENCODE.
const IDS:&[&str]=&["dickens","mozilla","samba","webster","x-ray","nci","xml","osdb","reymont","sao","mr","ooffice"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let cap=8usize<<20;
    println!("{:<8}{:>14}{:>14}{:>16}","level","X2 builds","X2 uses","wasted 8KiB passes");
    for lvl in [1i32,3,9,19] {
        let _=rusty_zstd::take_x2_stats();
        let mut n=0usize;
        for id in IDS { let Some(f)=load(id) else{continue}; let src=&f[..f.len().min(cap)]; n+=src.len();
            // ENCODE ONLY -- no decompress call in this loop
            let _=rusty_zstd::compress(src,lvl).unwrap(); }
        let (b,u)=rusty_zstd::take_x2_stats();
        println!("{:<8}{:>14}{:>14}{:>16}  ({} MiB encoded)",format!("L{lvl}"),b,u,b-u,n>>20);
    }
}
