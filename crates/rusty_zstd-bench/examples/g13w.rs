//! GATE 13 @ L1: sweep the copy WIDTH. Byte-identical by construction -- the
//! copy writes `w` bytes but only `n` are published by set_len -- so this is a
//! pure speed question and size is asserted, not measured.
use std::time::Instant;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao","x-ray","versions-16m","text-32m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let n: usize = std::env::args().nth(3).and_then(|s| s.parse().ok()).unwrap_or(9);
    const WS: &[usize] = &[8, 16, 32];
    println!("GATE 13 @ L{lvl} WIDTH SWEEP (cap {} MiB, best-of-{n})", cap>>20);
    println!("{:<13} {:>10} {:>10} {:>10} | {:>7} {:>9}", "corpus", "w8 ms", "w16 ms", "w32 ms", "best", "vs w16");
    let (mut w8win, mut w16win, mut w32win) = (0,0,0);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut base = Vec::new();
        let mut ts = Vec::new();
        for &w in WS {
            rusty_zstd::set_lit_push_width_arm(w);
            let z = rusty_zstd::compress(s, lvl).unwrap();
            if base.is_empty() { base = z.clone(); } else { assert_eq!(z, base, "{id}: width {w} changed OUTPUT"); }
            let mut b = f64::MAX;
            for _ in 0..n { let t = Instant::now(); let _ = rusty_zstd::compress(s, lvl).unwrap(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
            ts.push(b);
        }
        rusty_zstd::set_lit_push_width_arm(0);
        let bi = (0..3).min_by(|a,b| ts[*a].partial_cmp(&ts[*b]).unwrap()).unwrap();
        match bi { 0 => w8win += 1, 1 => w16win += 1, _ => w32win += 1 }
        println!("{:<13} {:>10.2} {:>10.2} {:>10.2} | {:>7} {:>8.2}%",
            id, ts[0], ts[1], ts[2], WS[bi], (ts[bi]/ts[1]-1.0)*100.0);
    }
    println!("\n  best width per corpus: w8 {w8win}, w16 {w16win}, w32 {w32win}");
    println!("  (output asserted identical across all three widths on every corpus)");
}
