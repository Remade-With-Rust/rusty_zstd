//! GATE 14 @ L19: do the probe savings convert to TIME? Asked BEFORE building
//! any dispatch -- at L3 a work count said -0.38% while the clock said +2.5%.
const IDS:&[&str]=&["mr","samba","mozilla","nci","webster","xml","reymont","dickens","jsonlog-16m","osdb"];
fn load(id:&str)->Option<Vec<u8>>{
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_|std::fs::read(format!("corpora/data/silesia/{id}"))).ok()
}
fn ms(src:&[u8],d:usize,r:usize)->f64{
    rusty_zstd::set_bt_depth_target_arm(d);
    let mut b=f64::MAX;
    for _ in 0..r { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,19).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} }
    b
}
fn paired(src:&[u8],a:usize,b:usize)->f64{
    let mut d=vec![];
    for _ in 0..5 { let a1=ms(src,a,4); let b1=ms(src,b,4); let b2=ms(src,b,4); let a2=ms(src,a,4);
        d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2)); }
    d.sort_by(|x,y|x.partial_cmp(y).unwrap()); d[2]
}
fn main(){
    println!("L19, 2 MiB, best-of-4 x ABBA x5, median. negative = FASTER\n");
    println!("{:<12}{:>9}{:>10}{:>10}{:>11}","corpus","null","depth 24","depth 16","probe% @24");
    let (mut tn,mut t24,mut t16,mut k)=(0.0,0.0,0.0,0.0);
    for id in IDS{
        let Some(full)=load(id) else{continue};
        let src=&full[..full.len().min(2<<20)];
        rusty_zstd::set_bt_depth_target_arm(0);
        let _=rusty_zstd::take_bt_probe_stats();
        let _=rusty_zstd::compress(src,19).unwrap();
        let pa=rusty_zstd::take_bt_probe_stats().0;
        rusty_zstd::set_bt_depth_target_arm(24);
        let _=rusty_zstd::take_bt_probe_stats();
        let _=rusty_zstd::compress(src,19).unwrap();
        let pb=rusty_zstd::take_bt_probe_stats().0;
        let n=paired(src,0,0); let a=paired(src,0,24); let b=paired(src,0,16);
        println!("{id:<12}{n:>8.2}%{a:>9.2}%{b:>9.2}%{:>10.2}%",
            100.0*(pb as f64-pa as f64)/pa.max(1) as f64);
        tn+=n.abs(); t24+=a; t16+=b; k+=1.0;
    }
    println!("\nmean |null| {:.2}%   depth 24 {:+.2}%   depth 16 {:+.2}%", tn/k, t24/k, t16/k);
    rusty_zstd::set_bt_depth_target_arm(0);
}
