fn best(src: &[u8], spec: bool, n: usize) -> (f64, usize) {
    rusty_zstd::set_dfast_spec_arm(spec);
    let mut b = f64::MAX; let mut sz = 0;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let z = rusty_zstd::compress(src, 3).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < b { b = e; }
        sz = z.len();
    }
    (b, sz)
}
fn main() {
    let files = ["dickens","mozilla","mr","nci","ooffice","osdb","reymont","samba","sao","webster","x-ray","xml"];
    let (mut ta, mut tb) = (0.0, 0.0);
    println!("{:<10}{:>12}{:>12}{:>10}", "file", "runtime ms", "const ms", "delta");
    for f in files {
        let Ok(src) = std::fs::read(format!("corpora/data/silesia/{f}")) else { continue };
        // ABBA in one process
        let (a1, s1) = best(&src, false, 9);
        let (b1, s2) = best(&src, true, 9);
        let (b2, _) = best(&src, true, 9);
        let (a2, _) = best(&src, false, 9);
        assert_eq!(s1, s2, "{f}: specialisation changed output");
        let (a, b) = (a1.min(a2), b1.min(b2));
        ta += a; tb += b;
        println!("{f:<10}{a:>12.1}{b:>12.1}{:>9.2}%", 100.0*(b-a)/a);
    }
    println!("{:<10}{ta:>12.1}{tb:>12.1}{:>9.2}%  <- TOTAL", "", 100.0*(tb-ta)/ta);
}
