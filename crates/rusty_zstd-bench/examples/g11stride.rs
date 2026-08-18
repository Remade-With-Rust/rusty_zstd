//! Sweep the BtLazy2 back-fill stride: bt work vs size, both deterministic.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let srcs: Vec<(&str,Vec<u8>)> = IDS.iter().filter_map(|id|
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f|{let n=f.len().min(2<<20);(*id,f[..n].to_vec())})).collect();
    std::env::remove_var("RZSTD_BT_FILL_S");
    let (mut b_sz,mut b_bt)=(0usize,0u64); let mut base=vec![];
    for (_,s) in &srcs { let _=rusty_zstd::take_bt_calls();
        let n=rusty_zstd::compress(s,lvl).unwrap().len(); let (x,y)=rusty_zstd::take_bt_calls();
        base.push(n); b_sz+=n; b_bt+=x+y; }
    println!("stride 1 (today): {b_bt} bt calls\n");
    for st in ["2","3","4","8"] {
        std::env::set_var("RZSTD_BT_FILL_S", st);
        let (mut sz,mut bt)=(0usize,0u64); let mut worst=0.0f64; let mut wc="";
        for (i,(id,s)) in srcs.iter().enumerate() {
            let _=rusty_zstd::take_bt_calls();
            let z=rusty_zstd::compress(s,lvl).unwrap();
            let (x,y)=rusty_zstd::take_bt_calls();
            assert_eq!(rusty_zstd::decompress(&z).unwrap(),*s,"{id} stride {st}");
            sz+=z.len(); bt+=x+y;
            let d=100.0*(z.len() as f64-base[i] as f64)/base[i] as f64;
            if d>worst {worst=d; wc=id;}
        }
        println!("stride {st:<3} bt {bt:>11} ({:>5.1}% cut)   size {:+.3}%   worst +{worst:.2}% ({wc})",
            100.0*(b_bt-bt) as f64/b_bt as f64, 100.0*(sz as f64-b_sz as f64)/b_sz as f64);
    }
    std::env::remove_var("RZSTD_BT_FILL_S");
}
