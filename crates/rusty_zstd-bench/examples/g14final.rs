const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    std::env::set_var("RZSTD_BT_DEPTH","0");   // dispatch off = old behaviour
    let mut base=vec![]; let (mut bs,mut bp)=(0usize,0u64);
    for (_,s) in &srcs { let _=rusty_zstd::take_bt_probe_stats();
        let n=rusty_zstd::compress(s,lvl).unwrap().len(); let (p,_,_)=rusty_zstd::take_bt_probe_stats();
        base.push(n); bs+=n; bp+=p; }
    std::env::remove_var("RZSTD_BT_DEPTH");
    let (mut sz,mut pr)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
    for (i,(id,s)) in srcs.iter().enumerate() { let _=rusty_zstd::take_bt_probe_stats();
        let z=rusty_zstd::compress(s,lvl).unwrap(); let (p,_,_)=rusty_zstd::take_bt_probe_stats();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id}");
        sz+=z.len(); pr+=p;
        let d=100.0*(z.len() as f64-base[i] as f64)/base[i] as f64;
        if d>worst {worst=d; wc=id;} }
    println!("L{lvl}  bt probes {bp} -> {pr}  ({:.1}% removed)   size {:+.4}%   worst +{worst:.3}% ({wc})",
        100.0*(bp-pr) as f64/bp as f64, 100.0*(sz as f64-bs as f64)/bs as f64);
}
