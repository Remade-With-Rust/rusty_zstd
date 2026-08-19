//! GATE 15, decided DETERMINISTICALLY. The box is CPU-bound (null spreads of 24
//! and 33 points), so no timing is admissible. Count the compare OPERATIONS each
//! path executes instead -- that number cannot drift.
const IDS:&[&str]=&["x-ray","sao","osdb","smallmsg-8m","mr","dickens","ooffice","webster","reymont","samba","xml","jsonlog-16m","mozilla","nci"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let lvl:i32=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(3);
    println!("L{lvl}: compare operations executed per path (deterministic)\n");
    println!("{:<13}{:>8}{:>11}{:>11}{:>11}{:>10}","corpus","<8B %","avx2 ops","word ops","ratio","better");
    let (mut ta,mut tw)=(0u64,0u64);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_eqlen_arm(0);
        let _=rusty_zstd::take_eq_ops(); let _=rusty_zstd::take_eqlen_stats();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (w0,d0,b0)=rusty_zstd::take_eq_ops();
        let (_c,_e,h)=rusty_zstd::take_eqlen_stats();
        rusty_zstd::set_eqlen_arm(1);
        let _=rusty_zstd::take_eq_ops();
        let _=rusty_zstd::compress(src,lvl).unwrap();
        let (w1,d1,b1)=rusty_zstd::take_eq_ops();
        rusty_zstd::set_eqlen_arm(0);
        let tot:u64=h.iter().sum();
        if tot==0 {continue;}
        let avx=w0+d0+b0; let wrd=w1+d1+b1;
        ta+=avx; tw+=wrd;
        let short=100.0*h[0] as f64/tot as f64;
        let r=wrd as f64/avx.max(1) as f64;
        println!("{id:<13}{short:>7.1}%{avx:>11}{wrd:>11}{r:>11.2}{:>10}",
            if r>1.05 {"avx2"} else if r<0.95 {"WORDS"} else {"tie"});
    }
    println!("\nTOTAL avx2 {ta} ops, words {tw} ops -> words/avx2 = {:.2}", tw as f64/ta as f64);
    println!("(a 32B cmpeq and an 8B compare are both ~4 instructions, so op count IS the work)");
}
