//! Gate 2 re-tune after the step fix: per-corpus, candidate vs shipped 0.20.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(8<<20);(*id,f[..n].to_vec())})).collect();
    std::env::remove_var("RZSTD_REPMIN");
    let base: Vec<usize> = srcs.iter().map(|(_,s)| rusty_zstd::compress(s,1).unwrap().len()).collect();
    for t in ["0.0","0.10"] {
        std::env::set_var("RZSTD_REPMIN", t);
        println!("\n=== REPMIN {t} vs shipped 0.20 ===");
        let (mut w,mut l)=(0,0);
        let (mut tb,mut tn)=(0usize,0usize);
        for (i,(id,s)) in srcs.iter().enumerate() {
            let n=rusty_zstd::compress(s,1).unwrap().len();
            let d=100.0*(n as f64-base[i] as f64)/base[i] as f64;
            if d.abs()>=0.005 { println!("  {id:<14}{d:>8.3}%"); }
            if d < -0.005 {w+=1;} if d>0.005 {l+=1;}
            tb+=base[i]; tn+=n;
        }
        println!("  better {w}, WORSE {l}, total {:+.3}%", 100.0*(tn as f64-tb as f64)/tb as f64);
    }
    std::env::remove_var("RZSTD_REPMIN");
}
