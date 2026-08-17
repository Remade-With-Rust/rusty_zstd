//! Is the TAG arm LIVE at L1? A 0/18 byte result is ambiguous between
//! "byte-identical by design" and "the toggle does nothing". Timing separates
//! them: a different monomorphization must have a different cost.
fn best(src: &[u8], tag: bool, n: usize) -> (f64, usize) {
    rusty_zstd::set_tag_arm(tag);
    let mut b = f64::MAX; let mut sz = 0;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let z = rusty_zstd::compress(src, 1).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < b { b = e; }
        sz = z.len();
    }
    (b, sz)
}
fn main() {
    println!("{:<10}{:>11}{:>11}{:>9}   size on/off", "file", "tag ON", "tag OFF", "delta");
    for f in ["webster","mozilla","nci","xml","osdb","dickens"] {
        let Ok(src) = std::fs::read(format!("corpora/data/silesia/{f}")) else { continue };
        let (a1,s1) = best(&src, true, 7);
        let (b1,s2) = best(&src, false, 7);
        let (b2,_)  = best(&src, false, 7);
        let (a2,_)  = best(&src, true, 7);
        let (a,b)=(a1.min(a2), b1.min(b2));
        println!("{f:<10}{a:>11.1}{b:>11.1}{:>8.2}%   {} / {}{}",
            100.0*(b-a)/a, s1, s2, if s1==s2 {" (identical)"} else {" *** DIFFER ***"});
    }
}
