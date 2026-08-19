//! Which corpus regresses, and does it depend on the input SIZE?
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("{:<13} {:>10} {:>10} {:>10} {:>10}", "corpus", "1 MiB", "2 MiB", "4 MiB", "8 MiB");
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let mut row = format!("{:<13}", id);
        let mut any = false;
        for cap in [1usize<<20, 2<<20, 4<<20, 8<<20] {
            if f.len() < cap { row += &format!(" {:>10}", "-"); continue }
            let s = &f[..cap];
            rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
            let off = rusty_zstd::compress(s, lvl).unwrap().len();
            rusty_zstd::set_g5_arms(0.30, 0.70, 1.50);
            let on = rusty_zstd::compress(s, lvl).unwrap().len();
            let pc = (on as f64/off as f64 - 1.0)*100.0;
            if pc > 0.05 { any = true }
            row += &format!(" {:>9.3}%", pc);
        }
        if any { row += "   <== regresses" }
        println!("{row}");
    }
}
