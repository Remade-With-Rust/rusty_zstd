//! GATE 14 (chain depth `1 << search_log`). Section 4.33 measured that 82-84% of
//! bt walks END by exhausting `attempts` -- the search is depth-bound, not
//! structure-bound. So the depth knob is the binding constraint. Price it:
//! bt work (deterministic) against size.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    rusty_zstd::set_search_log_delta(0);
    let mut base=vec![]; let (mut bsz,mut bpr)=(0usize,0u64);
    for (_,s) in &srcs {
        let _=rusty_zstd::take_bt_probe_stats();
        let n=rusty_zstd::compress(s,lvl).unwrap().len();
        let (p,_,_)=rusty_zstd::take_bt_probe_stats();
        base.push(n); bsz+=n; bpr+=p;
    }
    println!("L{lvl} baseline: {bpr} bt probes, {bsz} bytes\n");
    for d in [-3i32,-2,-1,1,2] {
        rusty_zstd::set_search_log_delta(d);
        let (mut sz,mut pr)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let _=rusty_zstd::take_bt_probe_stats();
            let z=rusty_zstd::compress(s,lvl).unwrap();
            let (p,_,_)=rusty_zstd::take_bt_probe_stats();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id} d{d}");
            sz+=z.len(); pr+=p;
            let dl=100.0*(z.len() as f64-base[i] as f64)/base[i] as f64;
            if dl>worst {worst=dl; wc=id;}
        }
        println!("searchLog{d:+}  probes {pr:>12} ({:>6.1}%)   size {:+.3}%   worst +{worst:.2}% ({wc})",
            100.0*(pr as f64-bpr as f64)/bpr as f64, 100.0*(sz as f64-bsz as f64)/bsz as f64);
    }
    rusty_zstd::set_search_log_delta(0);
}
