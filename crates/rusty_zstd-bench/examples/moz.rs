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
    for id in ["mozilla", "dickens"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(8*1024*1024)];
        let p = rusty_zstd::compression_params(1, Some(src.len() as u64)).unwrap();
        print!("{id:<9} hlog={} step-arm  ", p.hash_log);
        let mut ds = vec![];
        for _ in 0..5 {
            let g1=best(src,false,15); let s1=best(src,true,15);
            let s2=best(src,true,15);  let g2=best(src,false,15);
            let (g,s)=(g1.min(g2), s1.min(s2));
            ds.push(100.0*(s-g)/g);
        }
        let mean: f64 = ds.iter().sum::<f64>()/ds.len() as f64;
        let pos = ds.iter().filter(|&&x| x>0.0).count();
        println!("deltas {:?}  mean {:+.2}%  positive {}/5",
            ds.iter().map(|x| format!("{x:+.1}")).collect::<Vec<_>>(), mean, pos);
    }
    rusty_zstd::set_fast_spec_arm(true);
}
