//! The opt-ladder thresholds must not touch Fast (L1) or the middle ladder
//! (L3/L5/L9/L13). Proven by swinging them to extremes.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let mut n = 0; let mut moved = 0;
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        for cap in [1usize<<20, 4<<20] {
            if f.len() < cap { continue }
            let s = &f[..cap];
            for lvl in [1i32, 3, 5, 9, 13] {
                rusty_zstd::set_g5_opt_arms(2.0, 0.50, 1.50);
                let a = rusty_zstd::compress(s, lvl).unwrap();
                rusty_zstd::set_g5_opt_arms(-1.0, 0.01, 0.01);
                let b = rusty_zstd::compress(s, lvl).unwrap();
                rusty_zstd::set_g5_opt_arms(2.0, 0.50, 1.50);
                n += 1;
                if a != b { moved += 1; println!("  LEAK {id} L{lvl}"); }
            }
        }
    }
    println!("{n} cells at L1-L13: opt arms moved output on {moved} (must be 0)");
    assert_eq!(moved, 0, "opt-ladder thresholds LEAKED below L16");
}
