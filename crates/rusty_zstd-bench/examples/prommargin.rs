//! PROMETHEUS STAGE 1b -- the literals gate's MARGIN ceiling.
//!
//! Stage 1 pruned the encode-speed half: the gate never false-accepts
//! (raw won 0 of 794 accepted blocks), so no better gate can save a ctable
//! build. This measures the OTHER side -- the decode-cost term the gate is
//! missing (m7-anatomy S4.4).
//!
//! For every accepted literal section: how much smaller than raw did it come
//! out? A block that wins 0.5% bought almost nothing and made the decoder run
//! a full Huffman decode where a memcpy would have done.
const IDS: &[&str] = &["reymont","dickens","webster","mr","smallmsg-8m","jsonlog-16m",
    "nci","samba","osdb","xml","mozilla","ooffice","sao","x-ray"];
const B: [&str; 8] = ["<=0.5%","0.5-1%","1-2%","2-5%","5-10%","10-20%","20-40%",">40%"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    let cap=8usize<<20;
    let mut tot=[0u64;16];
    for id in IDS{
        let Some(f)=load(id) else{continue};
        let s=&f[..f.len().min(cap)];
        let _=rusty_zstd::prof_take_lit_margin();
        let _=rusty_zstd::compress(s,lvl).unwrap();
        let h=rusty_zstd::prof_take_lit_margin();
        for i in 0..16 { tot[i]+=h[i]; }
    }
    let n:u64=tot[..8].iter().sum();
    let by:u64=tot[8..].iter().sum();
    println!("PROMETHEUS -- literals gate MARGIN over raw, accepted blocks @ L{lvl}");
    println!("({n} accepted sections, {by} raw bytes)\n");
    println!("| size win vs raw | blocks | % blocks | raw bytes | % bytes | cum % bytes |");
    println!("| --- | ---: | ---: | ---: | ---: | ---: |");
    let mut cum=0f64;
    for i in 0..8 {
        let pb= if by>0 {100.0*tot[8+i] as f64/by as f64} else {0.0};
        cum+=pb;
        println!("| {} | {} | {:.1} | {} | {:.1} | {:.1} |", B[i], tot[i],
            if n>0 {100.0*tot[i] as f64/n as f64} else {0.0}, tot[8+i], pb, cum);
    }
    let marginal_blocks:u64=tot[0]+tot[1];
    let marginal_bytes:u64=tot[8]+tot[9];
    println!("\nMARGINAL (win <=1%): {} blocks ({:.1}%), {} raw bytes ({:.1}%).",
        marginal_blocks, if n>0 {100.0*marginal_blocks as f64/n as f64} else {0.0},
        marginal_bytes, if by>0 {100.0*marginal_bytes as f64/by as f64} else {0.0});
    println!("Those are the sections a decode-cost term would hand back to memcpy,\nand the size they would cost.");
}
