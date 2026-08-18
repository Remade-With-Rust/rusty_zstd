//! GATE 8 @ L3 -- the DFast 2-way software pipeline, built because the gate was
//! DEAD here. Byte-identity is a HARD gate (issue order only); the verdict is
//! on TIME, via the paired estimator.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn ms(src:&[u8],lvl:i32,on:bool,n:usize)->f64{
    rusty_zstd::set_dfast_pipe_arm(on);
    let mut b=f64::MAX;
    for _ in 0..n { let t=std::time::Instant::now(); let _=rusty_zstd::compress(src,lvl).unwrap();
        let e=t.elapsed().as_secs_f64()*1000.0; if e<b{b=e;} } b }
fn main(){
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("GATE 8 @ L{lvl} -- DFast 2-way pipeline (negative = pipeline FASTER)\n");
    println!("{:<14}{:>12}{:>10}", "corpus", "bytes", "time %");
    println!("{}","-".repeat(36));
    let (mut tsum,mut n,mut bad)=(0.0,0.0,0);
    for id in IDS {
        let Ok(full)=std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else {continue};
        let src=&full[..full.len().min(8<<20)];
        rusty_zstd::set_dfast_pipe_arm(false);
        let a=rusty_zstd::compress(src,lvl).unwrap();
        rusty_zstd::set_dfast_pipe_arm(true);
        let b=rusty_zstd::compress(src,lvl).unwrap();
        if a.len()!=b.len() || a!=b { bad+=1; println!("{id:<14} BYTE DIVERGENCE {} vs {}", a.len(), b.len()); }
        assert_eq!(rusty_zstd::decompress(&b).unwrap(), src, "{id} round-trip");
        let mut d=vec![];
        for _ in 0..3 {
            let a1=ms(src,lvl,false,5); let b1=ms(src,lvl,true,5);
            let b2=ms(src,lvl,true,5);  let a2=ms(src,lvl,false,5);
            d.push(0.5*(100.0*(b1-a1)/a1+100.0*(b2-a2)/a2));
        }
        d.sort_by(|x,y| x.partial_cmp(y).unwrap());
        println!("{id:<14}{:>12}{:>9.2}%", b.len(), d[1]);
        tsum+=d[1]; n+=1.0;
    }
    println!("\nbyte divergences: {bad}/18   mean time {:+.2}%", tsum/n);
    rusty_zstd::set_dfast_pipe_arm(true);
}
