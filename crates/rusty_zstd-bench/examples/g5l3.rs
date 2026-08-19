//! GATE 5 @ L3 — `RZSTD_BLOCK_KB`, the block_max cap. Census Z3.
//!
//! Every block pays a fixed header cost (a Huffman write_tree, up to three FSE
//! table descriptions, the block header) and buys a freshly adapted set of
//! entropy tables. `block_max` sets that exchange rate and it has never been
//! swept. The census recorded the symptom: `mr` emits 77 compressed blocks where
//! C emits 138 -- C splits smaller and re-adapts more often.
//!
//! Step 2 wants WIN AND LOSS on content: if some corpora get smaller with a
//! smaller block while others get bigger, that sign flip IS the dispatch.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const KB: &[usize] = &[16, 32, 64, 84, 96, 128];
fn best<F: FnMut() -> usize>(n: usize, mut f: F) -> f64 {
    let mut b = f64::MAX;
    for _ in 0..n { let t = Instant::now(); f(); let e = t.elapsed().as_secs_f64()*1000.0; if e < b { b = e; } }
    b
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    let n = if lvl >= 13 { 3 } else { 7 };
    print!("GATE 5 @ L{lvl} — block_max sweep (cap {} MiB, best-of-{n})\n{:<13}", cap>>20, "corpus");
    for k in KB { print!(" {:>8}K", k); }
    println!("  {:>8} {:>9}", "best", "vs 128");
    let (mut smaller, mut bigger, mut n_c) = (0, 0, 0);
    let mut tot = vec![0i64; KB.len()];
    let mut ttime = vec![0.0f64; KB.len()];
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut sz = Vec::new();
        for (i, k) in KB.iter().enumerate() {
            std::env::set_var("RZSTD_BLOCK_KB", k.to_string());
            let z = rusty_zstd::compress(s, lvl).unwrap();
            assert!(rusty_zstd::decompress(&z).unwrap() == s, "{id} {k}K: round-trip");
            ttime[i] += best(n, || rusty_zstd::compress(s, lvl).unwrap().len());
            tot[i] += z.len() as i64;
            sz.push(z.len());
        }
        std::env::remove_var("RZSTD_BLOCK_KB");
        let base = *sz.last().unwrap();
        let bi = (0..sz.len()).min_by_key(|i| sz[*i]).unwrap();
        let d = (sz[bi] as f64 / base as f64 - 1.0) * 100.0;
        if sz[bi] < base { smaller += 1 } else if sz[bi] > base { bigger += 1 }
        n_c += 1;
        print!("{:<13}", id);
        for v in &sz { print!(" {:>9}", v); }
        println!("  {:>8}K {:>8.3}%", KB[bi], d);
    }
    println!("\n  best block size per corpus: {smaller} prefer SMALLER than 128K, {bigger} prefer 128K, of {n_c}");
    for (i, k) in KB.iter().enumerate() {
        println!("  {:>4}K  total {:>12}  ({:+.3}% vs 128K)   time {:>8.0} ms ({:+.1}%)",
            k, tot[i], (tot[i] as f64/(*tot.last().unwrap()) as f64 - 1.0)*100.0,
            ttime[i], (ttime[i]/ttime[ttime.len()-1] - 1.0)*100.0);
    }
}
