//! GATE 4 @ L1: does the 13-way specialisation lose to the generic arm anywhere?
//! Byte-identical by construction, so this is a SPEED question; sizes are
//! asserted equal as the correctness gate.
fn best(src: &[u8], spec: bool, n: usize) -> (f64, usize) {
    rusty_zstd::set_fast_spec_arm(spec);
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
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    let (mut tg, mut ts) = (0.0, 0.0);
    let mut lose = 0;
    println!("{:<14}{:>12}{:>12}{:>10}", "file", "generic ms", "spec ms", "delta");
    for f in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{f}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{f}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let (g1, s1) = best(src, false, 7);
        let (p1, s2) = best(src, true, 7);
        let (p2, _)  = best(src, true, 7);
        let (g2, _)  = best(src, false, 7);
        assert_eq!(s1, s2, "{f}: specialisation changed output");
        let (g, p) = (g1.min(g2), p1.min(p2));
        tg += g; ts += p;
        let d = 100.0*(p-g)/g;
        if d > 0.5 { lose += 1; }
        println!("{f:<14}{g:>12.1}{p:>12.1}{d:>9.2}%");
    }
    println!("{:<14}{tg:>12.1}{ts:>12.1}{:>9.2}%  <- TOTAL", "", 100.0*(ts-tg)/tg);
    println!("\ncorpora where the specialisation LOSES by >0.5%: {lose}/18");
}
