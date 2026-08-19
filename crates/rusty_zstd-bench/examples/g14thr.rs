//! Sweep the opt_rep_rate threshold at the larger input size, where nci's rate
//! rises past the 2 MiB value and it starts being caught.
const IDS:&[&str]=&["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn main(){
    let n:usize=std::env::args().nth(1).and_then(|s|s.parse().ok()).unwrap_or(8<<20);
    let (mut ba,mut bp)=(0i64,0u64);
    let mut base=vec![];
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(n)];
        rusty_zstd::set_bt_deep_min_arm(f32::MAX);
        let _=rusty_zstd::take_bt_probe_stats();
        let z=rusty_zstd::compress(src,19).unwrap().len() as i64;
        ba+=z; base.push(z); bp+=rusty_zstd::take_bt_probe_stats().0;
    }
    println!("{} MiB inputs\n{:>8}{:>11}{:>11}   worst corpus", n>>20,"thresh","size %","probes %");
    for t in [2.0f32,3.0,5.0,10.0,20.0]{
        let (mut sz,mut pr)=(0i64,0u64);
        let (mut w,mut wid)=(0f64,"");
        for (k,id) in IDS.iter().enumerate(){
            let Some(full)=load(id) else{continue};
            let src=&full[..full.len().min(n)];
            rusty_zstd::set_bt_deep_min_arm(t);
            let _=rusty_zstd::take_bt_probe_stats();
            let z=rusty_zstd::compress(src,19).unwrap().len() as i64;
            sz+=z; pr+=rusty_zstd::take_bt_probe_stats().0;
            let d=100.0*(z-base[k]) as f64/base[k] as f64;
            if d>w {w=d; wid=id;}
        }
        println!("{t:>8.1}{:>+10.4}%{:>+10.2}%   {} {:+.4}%",
            100.0*(sz-ba) as f64/ba as f64, 100.0*(pr as f64-bp as f64)/bp as f64,
            if wid.is_empty(){"none"}else{wid}, w);
    }
    rusty_zstd::set_bt_deep_min_arm(2.0);
}
