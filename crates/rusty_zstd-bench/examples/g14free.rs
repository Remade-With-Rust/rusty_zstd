//! GATE 14 @ L19/L22: how hard can the depth budget be cut before it costs?
//!
//! At L19 the budget is 128 attempts and the mean walk is ~8. Cutting by 2
//! (128 -> 32) is still 4x the mean, and the L13 signal search found the size
//! cost there is 0.000-0.042% on EVERY corpus -- no variance to dispatch on.
//! So the question is not "which content" but "how far".
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    println!("GATE 14 DEPTH BUDGET @ L{lvl} — how far is free? (cap {} KiB)", cap>>10);
    println!("{:<13} {:>8} | {:>8} {:>9} | {:>8} {:>9} | {:>8} {:>9} | {:>8} {:>9}",
        "corpus", "depth", "it -1", "sz -1", "it -2", "sz -2", "it -3", "sz -3", "it -4", "sz -4");
    let (mut ti, mut n) = ([0.0f64; 4], 0.0f64);
    let mut worst = [f64::MIN; 4];
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::set_search_log_delta(0);
        let _ = rusty_zstd::take_bt_iters();
        let z0 = rusty_zstd::compress(s, lvl).unwrap();
        let (w, it0, _) = rusty_zstd::take_bt_iters();
        if w < 1000 { continue }
        let mut row = format!("{:<13} {:>8.2} |", id, it0 as f64 / w as f64);
        for (k, d) in [-1i32, -2, -3, -4].iter().enumerate() {
            rusty_zstd::set_search_log_delta(*d);
            let _ = rusty_zstd::take_bt_iters();
            let z = rusty_zstd::compress(s, lvl).unwrap();
            let (_, it, _) = rusty_zstd::take_bt_iters();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} d={d}: round-trip");
            let ip = (it as f64 / it0.max(1) as f64 - 1.0) * 100.0;
            let sp = (z.len() as f64 / z0.len() as f64 - 1.0) * 100.0;
            if sp > worst[k] { worst[k] = sp }
            ti[k] += ip;
            row += &format!(" {:>7.1}% {:>8.3}% |", ip, sp);
        }
        rusty_zstd::set_search_log_delta(0);
        n += 1.0;
        println!("{row}");
    }
    println!("\n  mean iterations   d-1 {:+.1}%   d-2 {:+.1}%   d-3 {:+.1}%   d-4 {:+.1}%",
        ti[0]/n, ti[1]/n, ti[2]/n, ti[3]/n);
    println!("  WORST corpus size d-1 {:+.3}%  d-2 {:+.3}%  d-3 {:+.3}%  d-4 {:+.3}%",
        worst[0], worst[1], worst[2], worst[3]);
}
