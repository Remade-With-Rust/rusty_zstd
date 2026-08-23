//! GATE 7 @ L1, re-asked after the packed encoding was deleted.
//! The CAPABILITY is "reject a candidate without loading src[m]". Its value is
//! exactly the number of candidates whose 4 bytes do not match -- everything a
//! tag can filter. Deterministic; no clock.
fn main() {
    let ids = ["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m",
               "mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
    println!("{:<14}{:>14}{:>14}{:>10}   tag would save", "corpus", "rejectable", "real matches", "reject %");
    let (mut tf, mut tt) = (0u64, 0u64);
    for id in ids {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let src = &full[..full.len().min(8*1024*1024)];
        let _ = rusty_zstd::take_tag_rejects();
        let _ = rusty_zstd::compress(src, 1).unwrap();
        let (f, t) = rusty_zstd::take_tag_rejects();
        tf += f; tt += t;
        let pct = if f + t > 0 { 100.0 * f as f64 / (f + t) as f64 } else { 0.0 };
        println!("{id:<14}{f:>14}{t:>14}{pct:>9.1}%   {f} loads of src[m]");
    }
    println!("{:<14}{tf:>14}{tt:>14}{:>9.1}%", "TOTAL", 100.0*tf as f64/(tf+tt).max(1) as f64);
}
