//! GATE 2 @ L1 re-validation: the deployed dispatch LOSES 3.72% on xml.
//! Can the Fast threshold be placed so xml is captured without giving back
//! dickens/samba/ooffice? Deterministic: total compressed bytes over all 18.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n=f.len().min(8<<20); (*id, f[..n].to_vec()) })
    }).collect();
    let watch = IDS;
    // baseline = shipped 0.125
    std::env::remove_var("RZSTD_REPMIN");
    let base: Vec<usize> = srcs.iter().map(|(_,s)| rusty_zstd::compress(s,1).unwrap().len()).collect();
    let btot: usize = base.iter().sum();
    print!("{:<8}{:>12}", "REPMIN", "total %");
    for w in watch { print!("{w:>10}"); }
    println!("\n{}", "-".repeat(12+12+10*watch.len()));
    for t in ["0.0","0.05","0.10","0.15","0.20","0.30","0.45","0.60"] {
        std::env::set_var("RZSTD_REPMIN", t);
        let mut tot=0usize; let mut per=std::collections::HashMap::new();
        for (i,(id,s)) in srcs.iter().enumerate() {
            let n=rusty_zstd::compress(s,1).unwrap().len();
            tot+=n;
            per.insert(*id, 100.0*(n as f64-base[i] as f64)/base[i] as f64);
        }
        println!("
REPMIN {t}  TOTAL {:+.3}%", 100.0*(tot as f64-btot as f64)/btot as f64);
        let mut lose = 0;
        for w in watch {
            let v = per.get(w).copied().unwrap_or(0.0);
            if v.abs() >= 0.005 { println!("    {w:<14}{v:>8.3}%"); }
            if v > 0.005 { lose += 1; }
        }
        println!("    corpora WORSE than shipped: {lose}");
    }
    std::env::remove_var("RZSTD_REPMIN");
    println!("\n(negative = SMALLER than shipped 0.125)");
}
