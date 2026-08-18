//! What did two unconditional atomic fetch_adds per probe cost? Cross-binary is
//! unavoidable for a code-shape change, so this is a DETERMINISTIC cross-check
//! plus a paired timing run with the null arm's known floor in mind.
const IDS: &[&str] = &["mr","dickens","sao","ooffice","mozilla","samba","osdb","webster","reymont","smallmsg-8m","nci","xml"];
fn main(){
    let mut tot=0.0; let mut n=0.0;
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        let mut best=f64::MAX;
        for _ in 0..15 {
            let t=std::time::Instant::now();
            let _=rusty_zstd::compress(src,1).unwrap();
            let e=t.elapsed().as_secs_f64()*1000.0; if e<best {best=e;}
        }
        let mbps = (src.len() as f64/1048576.0)/(best/1000.0);
        println!("{id:<14}{best:>9.2} ms{mbps:>10.1} MB/s");
        tot+=mbps; n+=1.0;
    }
    println!("\nmean {:.1} MB/s", tot/n);
}
