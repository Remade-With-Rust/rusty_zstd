//! What did Gate 6's pair search COST in speed? Shipped on size alone.
//! PAIRED estimator (null-arm error ~0.1%).
fn phase(src: &[u8], pair: bool, n: usize) -> f64 {
    rusty_zstd::set_pair_on_arm(pair);
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
    let ids = ["nci","mozilla","reymont","samba","xml","webster","dickens","ooffice","osdb","mr","sao","x-ray"];
    println!("{:<10}{:>11}{:>11}{:>10}", "corpus", "size %", "time %", "bytes/ms traded");
    let (mut ts, mut tt) = (0.0, 0.0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        rusty_zstd::set_pair_on_arm(false);
        let sa = rusty_zstd::compress(src, 1).unwrap().len() as f64;
        rusty_zstd::set_pair_on_arm(true);
        let sb = rusty_zstd::compress(src, 1).unwrap().len() as f64;
        let mut d = vec![];
        for _ in 0..3 {
            let a1=phase(src,false,7); let b1=phase(src,true,7);
            let b2=phase(src,true,7);  let a2=phase(src,false,7);
            d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        }
        let tm: f64 = d.iter().sum::<f64>()/d.len() as f64;
        let sm = 100.0*(sb-sa)/sa;
        ts += sm; tt += tm;
        println!("{id:<10}{sm:>10.2}%{tm:>10.2}%", );
    }
    let n = ids.len() as f64;
    println!("\nmean size {:+.2}%   mean time {:+.2}%", ts/n, tt/n);
    rusty_zstd::set_pair_on_arm(true);
}
