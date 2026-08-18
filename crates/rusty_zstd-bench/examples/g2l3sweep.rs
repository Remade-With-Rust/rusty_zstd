//! GATE 2 @ L3: sweep the DFast rep threshold, which is pinned at 0.0 (constant
//! ON). Baseline is today's 0.0. Positive = larger than today.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(8<<20);(*id,f[..n].to_vec())})).collect();
    std::env::remove_var("RZSTD_REPMIN");
    let base: Vec<usize> = srcs.iter().map(|(_,s)| rusty_zstd::compress(s,3).unwrap().len()).collect();
    let btot: usize = base.iter().sum();
    for t in ["0.005","0.01","0.03","0.05","0.10","0.125"] {
        std::env::set_var("RZSTD_REPMIN", t);
        let mut tot=0usize; let (mut w,mut l)=(0,0); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let n=rusty_zstd::compress(s,3).unwrap().len();
            tot+=n;
            let d=100.0*(n as f64-base[i] as f64)/base[i] as f64;
            if d < -0.005 {w+=1;} else if d>0.005 {l+=1; if d>worst {worst=d; wc=id;}}
        }
        println!("REPMIN {t:<7} TOTAL {:+.4}%   better {w:>2}  worse {l:>2}   worst +{worst:.3}% ({wc})",
            100.0*(tot as f64-btot as f64)/btot as f64);
    }
    std::env::remove_var("RZSTD_REPMIN");
}
