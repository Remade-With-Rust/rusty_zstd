//! GATE 2 dispatch hunt. `rep_yield` = hits/SEQUENCE prices the benefit against
//! a quantity unrelated to the cost (one try_rep1 per POSITION). Does rep match
//! BYTES PER PROBE separate the corpora that want rep1 always-on from those
//! that do not?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(8<<20);(*id,f[..n].to_vec())})).collect();
    // shipped baseline
    std::env::remove_var("RZSTD_REPMIN");
    let base: Vec<usize> = srcs.iter().map(|(_,s)| rusty_zstd::compress(s,1).unwrap().len()).collect();
    // always-on: measure the rate AND the size delta on the same arm
    std::env::set_var("RZSTD_REPMIN","0.0");
    let mut rows=vec![];
    for (i,(id,s)) in srcs.iter().enumerate() {
        let _=rusty_zstd::take_rep_rate();
        let n=rusty_zstd::compress(s,1).unwrap().len();
        let (probes,bytes,hits,allb,alls)=rusty_zstd::take_rep_rate();
        let rate = bytes as f64 / probes.max(1) as f64;
        let replen = bytes as f64 / hits.max(1) as f64;
        let alllen = allb as f64 / alls.max(1) as f64;
        let d = 100.0*(n as f64-base[i] as f64)/base[i] as f64;
        rows.push((*id, rate, d, probes, replen, alllen));
    }
    std::env::remove_var("RZSTD_REPMIN");
    rows.sort_by(|a,b| (a.4/a.5.max(0.001)).partial_cmp(&(b.4/b.5.max(0.001))).unwrap());
    println!("{:<14}{:>10}{:>12}{:>10}{:>10}{:>9}", "corpus","B/probe","size @0.0","rep len","all len","ratio");
    println!("  (negative size = always-on WINS for this corpus)\n");
    for (id,r,d,_p,rl,al) in &rows {
        println!("{id:<14}{r:>10.4}{d:>11.3}%{rl:>10.2}{al:>10.2}{:>9.2}", rl/al.max(0.001));
    }
    println!("\nIf the winners cluster at one end of B/probe, that is the dispatch axis.");
}
