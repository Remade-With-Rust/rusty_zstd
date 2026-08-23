//! How often does each FSE sequence-table MODE fire? Mode 3 (Repeat) makes
//! `seq_table` CLONE the previous table: a Vec allocation plus a memcpy of up
//! to 512 x 4 bytes, per table, per block.
const IDS:&[&str]=&["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    let cap=8usize<<20;
    let mut tot=[0u64;4];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _=rusty_zstd::compress(s,lvl).unwrap();
        let c=rusty_zstd::prof_encode_counts();
        for i in 0..4 { tot[i]+=c.seq_modes[i]; }
    }
    let n:u64=tot.iter().sum();
    const M:[&str;4]=["Predefined","RLE","Compressed","**Repeat (CLONES)**"];
    println!("FSE sequence-table mode selections @ L{lvl} (LL+OF+ML summed)\n");
    println!("| mode | count | % |");
    println!("| --- | ---: | ---: |");
    for i in 0..4 {
        println!("| {} | {} | {:.1} |", M[i], tot[i], if n>0 {100.0*tot[i] as f64/n as f64} else {0.0});
    }
    println!("\ntotal table selections: {n}");
}
