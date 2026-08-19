//! GATE 12, measured through an arm that actually changes. Work (chain inserts)
//! against size.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(7);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    rusty_zstd::set_lazy_fill_stride_arm(1);
    let mut base=vec![]; let (mut bs,mut bi)=(0i64,0u64);
    for (_,s) in &srcs { let _=rusty_zstd::take_lazy_fill();
        let n=rusty_zstd::compress(s,lvl).unwrap().len() as i64; let (_,_,i)=rusty_zstd::take_lazy_fill();
        base.push(n); bs+=n; bi+=i; }
    println!("L{lvl} stride 1: {bi} back-fill inserts, {bs} bytes\n");
    for st in [2usize,3,4,8] {
        rusty_zstd::set_lazy_fill_stride_arm(st);
        let (mut sz,mut ins)=(0i64,0u64); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let _=rusty_zstd::take_lazy_fill();
            let z=rusty_zstd::compress(s,lvl).unwrap();
            let (_,_,n)=rusty_zstd::take_lazy_fill();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id} stride {st}");
            sz+=z.len() as i64; ins+=n;
            let d=100.0*(z.len() as i64-base[i]) as f64/base[i] as f64;
            if d>worst {worst=d; wc=id;}
        }
        println!("stride {st}: inserts {ins:>11} ({:>6.1}%)   size {:+.4}%   worst +{worst:.3}% ({wc})",
            100.0*(ins as f64-bi as f64)/bi as f64, 100.0*(sz-bs) as f64/bs as f64);
    }
    rusty_zstd::set_lazy_fill_stride_arm(1);
}
