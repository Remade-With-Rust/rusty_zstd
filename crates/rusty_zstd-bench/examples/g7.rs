//! GATE 7 (packed tag slots). Step 1: reached at L3? Step 2: the CAPABILITY --
//! tag-based candidate rejection without touching src[m] -- does it pay where
//! it IS live (L1)? Sizes are exact, so the biased timing estimator is irrelevant.
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    for lvl in [3, 1] {
        let (mut diff, mut n, mut tot_a, mut tot_b) = (0, 0, 0usize, 0usize);
        println!("--- L{lvl} ---");
        for id in ids {
            let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let src = &full[..full.len().min(8*1024*1024)];
            rusty_zstd::set_tag_arm(false);
            let a = rusty_zstd::compress(src, lvl).unwrap().len();
            rusty_zstd::set_tag_arm(true);
            let b = rusty_zstd::compress(src, lvl).unwrap().len();
            rusty_zstd::set_tag_arm(false);
            n += 1; tot_a += a; tot_b += b;
            if a != b {
                diff += 1;
                if lvl == 1 { println!("  {id:<14}{a:>12}{b:>12}{:>9.4}%", 100.0*(b as f64-a as f64)/a as f64); }
            }
        }
        println!("  changed {diff}/{n}   total {tot_a} -> {tot_b}  ({:+.4}%)",
                 100.0*(tot_b as f64-tot_a as f64)/tot_a as f64);
    }
}
