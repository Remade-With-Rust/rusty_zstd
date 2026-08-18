//! Re-verify the L19 finding with the STRONGER method (5 meta-runs x 15 iters)
//! on the corpora the 3-run test called "stable generic wins".
fn best(src: &[u8], spec: bool, n: usize) -> f64 {
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
    // NOTE: the dispatch now forces generic at L19, so temporarily we must
    // measure the ARM's effect where it can still act -- report both.
    for id in ["nci", "x-ray", "xml", "samba"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(1024*1024)];
        let mut ds = vec![];
        for _ in 0..5 {
            let g1=best(src,false,9); let s1=best(src,true,9);
            let s2=best(src,true,9);  let g2=best(src,false,9);
            let (g,s)=(g1.min(g2), s1.min(s2));
            ds.push(100.0*(s-g)/g);
        }
        let mean: f64 = ds.iter().sum::<f64>()/ds.len() as f64;
        let pos = ds.iter().filter(|&&x| x>0.0).count();
        println!("{id:<8} deltas {:?}  mean {:+.2}%  positive {}/5",
            ds.iter().map(|x| format!("{x:+.1}")).collect::<Vec<_>>(), mean, pos);
    }
    rusty_zstd::set_bt_spec_arm(true);
}
