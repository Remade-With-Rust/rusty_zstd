//! The rep_yield decay schedule IS the warm-up cost now that the DFast gate
//! fires. Sweep it: work removed vs size, both deterministic.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(8<<20);(*id,f[..n].to_vec())})).collect();
    std::env::remove_var("RZSTD_REP_DECAY");
    let mut base=vec![]; let mut bpos=0u64;
    for (_,s) in &srcs {
        let _=rusty_zstd::take_dfast_rep_blocks();
        base.push(rusty_zstd::compress(s,3).unwrap().len());
        let (_,_,p)=rusty_zstd::take_dfast_rep_blocks(); bpos+=p;
    }
    let btot: usize = base.iter().sum();
    println!("baseline (decay 0.5): {bpos} rep positions\n");
    for d in ["0.35","0.25","0.10","0.0"] {
        std::env::set_var("RZSTD_REP_DECAY", d);
        let (mut tot,mut pos)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let _=rusty_zstd::take_dfast_rep_blocks();
            let n=rusty_zstd::compress(s,3).unwrap().len();
            let (_,_,p)=rusty_zstd::take_dfast_rep_blocks();
            tot+=n; pos+=p;
            let dl=100.0*(n as f64-base[i] as f64)/base[i] as f64;
            if dl>worst {worst=dl; wc=id;}
        }
        println!("decay {d:<5} rep pos {pos:>11}  ({:>5.1}% less than baseline)   size {:+.4}%   worst +{worst:.3}% ({wc})",
            100.0*(bpos-pos) as f64/bpos as f64, 100.0*(tot as f64-btot as f64)/btot as f64);
    }
    std::env::remove_var("RZSTD_REP_DECAY");
}
