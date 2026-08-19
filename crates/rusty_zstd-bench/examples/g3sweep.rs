//! Does `last_search_per_byte` -- the signal Gate 3 already carries -- separate
//! the corpora where the back-fill pays? Sweep the threshold end to end.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(7);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    rusty_zstd::set_lazy_fill_threshold_arm(0.0);
    let mut base=vec![]; let (mut bs,mut bi)=(0i64,0u64);
    for (_,s) in &srcs { let _=rusty_zstd::take_lazy_fill();
        let n=rusty_zstd::compress(s,lvl).unwrap().len() as i64; let (_,_,i)=rusty_zstd::take_lazy_fill();
        base.push(n); bs+=n; bi+=i; }
    println!("L{lvl} threshold 0.0 (today): {bi} inserts, {bs} bytes\n");
    for t in [0.05f32,0.1,0.2,0.3,0.5,0.8] {
        rusty_zstd::set_lazy_fill_threshold_arm(t);
        let (mut sz,mut ins)=(0i64,0u64); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let _=rusty_zstd::take_lazy_fill();
            let z=rusty_zstd::compress(s,lvl).unwrap();
            let (_,_,n)=rusty_zstd::take_lazy_fill();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id} t{t}");
            sz+=z.len() as i64; ins+=n;
            let d=100.0*(z.len() as i64-base[i]) as f64/base[i] as f64;
            if d>worst {worst=d; wc=id;}
        }
        println!("thr {t:<5} inserts {ins:>11} ({:>6.1}%)   size {:+.4}%   worst +{worst:.3}% ({wc})",
            100.0*(ins as f64-bi as f64)/bi as f64, 100.0*(sz-bs) as f64/bs as f64);
    }
    rusty_zstd::set_lazy_fill_threshold_arm(0.0);
}
