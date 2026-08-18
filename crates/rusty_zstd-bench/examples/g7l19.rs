//! GATE 7 @ L19: reachability, with L1 as the live control.
fn main() {
    let full = std::fs::read("corpora/data/silesia/xml").unwrap();
    let src = &full[..full.len().min(2*1024*1024)];
    println!("{:<6}{:>12}{:>12}{:>10}   arm effect", "level", "tag OFF", "tag ON", "delta");
    for lvl in [1, 19, 22] {
        rusty_zstd::set_tag_arm(false);
        let a = rusty_zstd::compress(src, lvl).unwrap().len();
        let t0 = std::time::Instant::now();
        for _ in 0..3 { let _ = rusty_zstd::compress(src, lvl).unwrap(); }
        let ta = t0.elapsed().as_secs_f64();
        rusty_zstd::set_tag_arm(true);
        let b = rusty_zstd::compress(src, lvl).unwrap().len();
        let t1 = std::time::Instant::now();
        for _ in 0..3 { let _ = rusty_zstd::compress(src, lvl).unwrap(); }
        let tb = t1.elapsed().as_secs_f64();
        println!("L{lvl:<5}{a:>12}{b:>12}{:>9.2}%   time {:+.1}%", 
                 100.0*(b as f64-a as f64)/a as f64, 100.0*(tb-ta)/ta);
    }
    rusty_zstd::set_tag_arm(true);
    println!("\nL1 is the CONTROL: the arm must move time there and nowhere else.");
}
