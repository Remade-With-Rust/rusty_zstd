//! GATE 7 dispatch vs OFF. Byte-identical, so purely speed. PAIRED estimator.
fn phase(src: &[u8], on: bool, n: usize) -> f64 {
    rusty_zstd::set_tag_arm(on);
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
    println!("{:<14}{:>10}{:>9}   verdict", "corpus", "mean %", "neg/3");
    let (mut w, mut l) = (0, 0);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let mut d = vec![];
        for _ in 0..3 {
            let a1=phase(src,false,7); let b1=phase(src,true,7);
            let b2=phase(src,true,7);  let a2=phase(src,false,7);
            d.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        }
        let mean: f64 = d.iter().sum::<f64>()/d.len() as f64;
        let neg = d.iter().filter(|&&x| x < 0.0).count();
        let v = if mean < -1.0 && neg == 3 { w+=1; "WINS" }
                else if mean > 1.0 && neg == 0 { l+=1; "LOSES" } else { "no signal" };
        println!("{id:<14}{mean:>10.2}{neg:>9}   {v}");
    }
    println!("\nwins {w}   losses {l}   (of 18)");
    rusty_zstd::set_tag_arm(true);
}
