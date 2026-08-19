//! How DEEP does a bt walk actually go? 4.33's "exhausted" flag was set at the
//! bottom of every iteration, so it measured "did at least one iteration", not
//! "used all attempts". Measure the real thing.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","versions-16m"];
fn main(){
    for lvl in [13i32,19,22] {
        let (mut w,mut it,mut f)=(0u64,0u64,0u64);
        for id in IDS {
            let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_bt_iters();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            let (a,b,c)=rusty_zstd::take_bt_iters();
            w+=a; it+=b; f+=c;
        }
        let p=rusty_zstd::compression_params(lvl, Some(2<<20)).unwrap();
        println!("L{lvl}: attempts={} | {w} walks | mean depth {:.1} | used ALL attempts {f} ({:.2}%)",
            1usize<<p.search_log.min(12), it as f64/w.max(1) as f64, 100.0*f as f64/w.max(1) as f64);
    }
}
