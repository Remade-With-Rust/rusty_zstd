//! NULL-ARM DIAGNOSTIC. At L19 the Gate 5 dispatch forces the generic body
//! regardless of the arm, so both settings execute IDENTICAL code. Any non-zero
//! delta is pure instrument error.
//!
//! Compares two estimators over the same A B B A phases:
//!   POOLED  = (min(B1,B2) - min(A1,A2)) / min(A1,A2)      <- what I was using
//!   PAIRED  = mean( (B1-A1)/A1 , (B2-A2)/A2 )             <- cancels monotone drift
fn phase(src: &[u8], spec: bool, n: usize) -> f64 {
    rusty_zstd::set_bt_spec_arm(spec);
    let mut b = f64::MAX;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let _ = rusty_zstd::compress(src, 19).unwrap();
        let e = t.elapsed().as_secs_f64()*1000.0;
        if e < b { b = e; }
    }
    b
}
fn main() {
    println!("{:<8}{:>12}{:>12}   (both should be ~0: identical code)", "corpus", "POOLED %", "PAIRED %");
    for id in ["nci","x-ray","xml","samba","webster"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(1024*1024)];
        let (mut pool, mut pair) = (vec![], vec![]);
        for _ in 0..5 {
            let a1=phase(src,false,9); let b1=phase(src,true,9);
            let b2=phase(src,true,9);  let a2=phase(src,false,9);
            let (a,b)=(a1.min(a2), b1.min(b2));
            pool.push(100.0*(b-a)/a);
            pair.push(0.5*(100.0*(b1-a1)/a1 + 100.0*(b2-a2)/a2));
        }
        let m=|v:&Vec<f64>| v.iter().sum::<f64>()/v.len() as f64;
        println!("{id:<8}{:>12.2}{:>12.2}", m(&pool), m(&pair));
    }
    rusty_zstd::set_bt_spec_arm(true);
}
