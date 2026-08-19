//! The `rep_len_ratio` early-return defect: what does fixing it buy at L1?
//! Gate 2's dispatch can finally shut the repcode search off on the 8 corpora
//! that run 93.8% pipelined. Size AND the repcode probe count.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(1);
    println!("{:<14}{:>10}{:>10}{:>10}{:>13}{:>13}{:>9}","corpus","defect","fixed","size %","rep probes","after","probe %");
    let (mut ta,mut tb,mut pa,mut pb)=(0i64,0i64,0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_replen_pipe_arm(false);
        let _=rusty_zstd::take_rep_rate();
        let a=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let ra=rusty_zstd::take_rep_rate().0;
        rusty_zstd::set_replen_pipe_arm(true);
        let _=rusty_zstd::take_rep_rate();
        let b=rusty_zstd::compress(src,lvl).unwrap().len() as i64;
        let rb=rusty_zstd::take_rep_rate().0;
        ta+=a; tb+=b; pa+=ra; pb+=rb;
        let pd=if ra>0 {100.0*(rb as f64-ra as f64)/ra as f64} else {0.0};
        println!("{id:<14}{a:>10}{b:>10}{:>+9.3}%{ra:>13}{rb:>13}{pd:>+8.1}%",
            100.0*(b-a) as f64/a as f64);
    }
    println!("\nTOTAL size {ta} -> {tb} ({:+.4}%)   rep probes {pa} -> {pb} ({:+.2}%)",
        100.0*(tb-ta) as f64/ta as f64, 100.0*(pb as f64-pa as f64)/pa as f64);
    rusty_zstd::set_replen_pipe_arm(true);
}
