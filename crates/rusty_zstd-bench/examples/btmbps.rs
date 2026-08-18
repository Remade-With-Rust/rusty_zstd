//! Throughput on the Bt ladder for the descent-load hoist. Cross-binary is
//! unavoidable for a code-shape change, so the verdict rests on the DIRECTION
//! being consistent across corpora, not on any single number.
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","osdb","webster","reymont","nci","xml","smallmsg-8m"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(2<<20)];
        let mut best=f64::MAX;
        for _ in 0..9 {
            let t=std::time::Instant::now();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let e=t.elapsed().as_secs_f64()*1000.0; if e<best {best=e;}
        }
        println!("{id} {:.3}", (src.len() as f64/1048576.0)/(best/1000.0));
    }
}
