//! GATE 13 capability at L22 (find_opt): deterministic verification.
//! Byte-identity is structural; the win is coverage and reservations.
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const I16: f64 = 7.0; const I32: f64 = 9.0; const ISLOW: f64 = 47.0;
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(22);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(256 << 10);
    println!("GATE 13 CAPABILITY @ L{lvl} (find_opt) — cap {} KiB", cap>>10);
    println!("{:<13} {:>10} {:>8} {:>8} {:>8} {:>8} | {:>12} {:>12} {:>9}",
        "corpus", "calls", "0-4", "5-8", "9-16", "17-32", "instr before", "instr after", "delta%");
    let (mut tb, mut ta) = (0.0f64, 0.0f64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let z = rusty_zstd::compress(s, lvl).unwrap();
        let (h, _, _) = rusty_zstd::take_lp_stats();
        assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id}: round-trip");
        let calls: u64 = h.iter().sum();
        if calls < 500 { continue }
        let all = calls as f64;
        let pc = |x: u64| x as f64/all*100.0;
        let c16 = (h[0]+h[1]+h[2]) as f64;
        let c32 = c16 + h[3] as f64;
        // before: every append was extend_from_slice
        let before = all * ISLOW;
        let (sh, md) = (c16/all, h[3] as f64/all);
        let after = if sh < 0.25 { all*ISLOW }
                    else if md > sh*0.0526 { c32*I32 + (all-c32)*ISLOW }
                    else { c16*I16 + (all-c16)*ISLOW };
        tb += before; ta += after;
        println!("{:<13} {:>10} {:>7.1}% {:>7.1}% {:>7.1}% {:>7.1}% | {:>12.0} {:>12.0} {:>8.2}%",
            id, calls, pc(h[0]), pc(h[1]), pc(h[2]), pc(h[3]), before, after, (after/before-1.0)*100.0);
    }
    println!("\n  TOTAL modelled instructions {:.0} -> {:.0}  ({:+.2}%)", tb, ta, (ta/tb-1.0)*100.0);
}
