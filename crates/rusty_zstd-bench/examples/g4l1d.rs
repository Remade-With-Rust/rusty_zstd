//! GATE 4 @ L1: the dispatch test that was never run.
//! find_fast carries 22 monomorphizations -- nearly double the 12 that measured
//! as a regression at L19 -- so the same I-cache question applies here.
fn best(src: &[u8], spec: bool, n: usize) -> f64 {
    rusty_zstd::set_fast_spec_arm(spec);
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, 1).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0;
        if e < b { b = e; }
    }
    b
}
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("{:<14}{:>9}{:>9}{:>9}   verdict", "corpus", "run1 %", "run2 %", "run3 %");
    let (mut sp, mut gn, mut un) = (0,0,0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let mut d=[0.0f64;3];
        for r in 0..3 {
            let g1=best(src,false,7); let s1=best(src,true,7);
            let s2=best(src,true,7);  let g2=best(src,false,7);
            d[r]=100.0*(s1.min(s2)-g1.min(g2))/g1.min(g2);
        }
        let neg=d.iter().filter(|&&x| x < -0.5).count();
        let pos=d.iter().filter(|&&x| x >  0.5).count();
        let v = if neg==3 {sp+=1;"STABLE: spec wins"} else if pos==3 {gn+=1;"STABLE: generic wins"} else {un+=1;"unstable / noise"};
        println!("{id:<14}{:>9.2}{:>9.2}{:>9.2}   {v}", d[0],d[1],d[2]);
    }
    println!("\nstable-spec {sp}  stable-generic {gn}  unstable {un}");
    println!("{}", if sp>0 && gn>0 {"SIGN FLIP -> DISPATCH"}
                   else if gn>0 {"generic wins outright -> the specialisation is a REGRESSION"}
                   else {"no stable flip -> CONSTANT (tested)"});
    rusty_zstd::set_fast_spec_arm(true);
}
