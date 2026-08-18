//! Does hoisting the pair probe's load (so the two misses overlap) recover the
//! serialization cost? PAIRED estimator, both shapes in one binary.
const IDS: &[&str] = &["mr","dickens","webster","ooffice","smallmsg-8m","reymont","osdb","mozilla","samba","xml","nci","jsonlog-16m"];
fn ms(src: &[u8], pre: bool, n: usize) -> f64 {
    rusty_zstd::set_pair_pre_arm(pre);
    let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,1).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b {b=e;} }
    b
}
fn main() {
    rusty_zstd::set_pair_on_arm(true);
    rusty_zstd::set_pair_gain_arm(0.0); // pair ON everywhere: measure the loop itself
    println!("{:<14}{:>12}", "corpus", "hoist vs not");
    let (mut t,mut n)=(0.0,0.0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/silesia/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else {continue};
        let src=&full[..full.len().min(8*1024*1024)];
        let mut d=vec![];
        for _ in 0..3 {
            let a1=ms(src,false,5); let b1=ms(src,true,5);
            let b2=ms(src,true,5);  let a2=ms(src,false,5);
            d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        }
        d.sort_by(|x,y| x.partial_cmp(y).unwrap());
        println!("{id:<14}{:>11.2}%", d[1]);
        t+=d[1]; n+=1.0;
    }
    println!("\nmean {:+.2}%  (negative = hoist is FASTER)", t/n);
}
