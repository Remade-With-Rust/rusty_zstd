//! `bt_depth_apply` runs once per `bt_find_best` call and fans out to four
//! `std::env::var` lookups. How many calls, and what is that worth?
fn main(){
    let t=std::time::Instant::now(); let n=200_000; let mut a=0usize;
    for _ in 0..n { a+=std::env::var("RZSTD_BT_DEPTH_TARGET").map(|v|v.len()).unwrap_or(0); }
    let ns=t.elapsed().as_secs_f64()*1e9/n as f64;
    println!("std::env::var miss: {ns:.1} ns  [{a}]");
    let ids=["dickens","samba","nci","xml","mozilla","webster","reymont","mr","ooffice","osdb","sao","x-ray"];
    for lvl in [19i32,22]{
        let (mut calls,mut ms)=(0u64,0.0);
        for id in ids{
            let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}")) else{continue};
            let src=&full[..full.len().min(2<<20)];
            let _=rusty_zstd::take_bt_calls();
            let t=std::time::Instant::now();
            let _=rusty_zstd::compress(src,lvl).unwrap();
            ms+=t.elapsed().as_secs_f64()*1000.0;
            let (s,r)=rusty_zstd::take_bt_calls(); calls+=s+r;
        }
        println!("L{lvl}: {calls} bt calls, encode {ms:.0} ms; \
                  4 env lookups/call = {:.0} ms ({:.0}% of encode)",
            calls as f64*4.0*ns/1e6, 100.0*(calls as f64*4.0*ns/1e6)/ms);
    }
}
