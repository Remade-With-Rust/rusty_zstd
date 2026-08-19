//! GATE 5 SIGNAL: what predicts wanting a SMALLER block?
//!
//! Physically, a smaller block pays when the entropy tables need re-adapting --
//! when the content's statistics DRIFT along the file. It costs when they do not,
//! because the extra headers buy nothing.
//!
//! So the candidate is drift itself: split the input into 128 KiB chunks,
//! compress each INDEPENDENTLY, and take the spread of their ratios. A file whose
//! chunks compress unevenly is a file whose tables go stale.
use std::time::Instant;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray"];
const KB: &[usize] = &[16, 32, 64, 84, 96, 128];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 5 SIGNAL @ L{lvl} — chunk-ratio drift vs the gain from a smaller block");
    println!("{:<13} {:>9} {:>9} {:>9} {:>8} {:>10}", "corpus", "drift CV", "ratio", "hdr frac", "best K", "gain%");
    let mut rows: Vec<(&str, f64, f64, f64)> = Vec::new();
    let t0 = Instant::now();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        // DRIFT: independent 128 KiB chunk ratios, coefficient of variation
        let mut rs = Vec::new();
        for c in s.chunks(128 << 10) {
            if c.len() < (64 << 10) { continue }
            let z = rusty_zstd::compress(c, lvl).unwrap();
            rs.push(z.len() as f64 / c.len() as f64);
        }
        if rs.len() < 3 { continue }
        let m = rs.iter().sum::<f64>() / rs.len() as f64;
        let var = rs.iter().map(|r| (r-m)*(r-m)).sum::<f64>() / rs.len() as f64;
        let cv = var.sqrt() / m.max(1e-9);
        // HEADER FRACTION: how much of the frame is per-block overhead, estimated
        // as (sum of independent chunk sizes - whole-file size) / whole-file size
        let whole = rusty_zstd::compress(s, lvl).unwrap().len() as f64;
        let indep: f64 = rs.iter().zip(s.chunks(128<<10)).map(|(r,c)| r * c.len() as f64).sum();
        let hdr = (indep - whole) / whole;
        // TARGET: gain at the best block size vs 128K
        let mut sz = Vec::new();
        for k in KB {
            std::env::set_var("RZSTD_BLOCK_KB", k.to_string());
            sz.push(rusty_zstd::compress(s, lvl).unwrap().len());
        }
        std::env::remove_var("RZSTD_BLOCK_KB");
        let base = *sz.last().unwrap();
        let bi = (0..sz.len()).min_by_key(|i| sz[*i]).unwrap();
        let gain = (sz[bi] as f64 / base as f64 - 1.0) * 100.0;
        println!("{:<13} {:>9.4} {:>9.4} {:>9.4} {:>7}K {:>9.3}%", id, cv, m, hdr, KB[bi], gain);
        rows.push((*id, cv, hdr, gain));
    }
    let n = rows.len() as f64;
    for (name, get) in [("drift CV", 0usize), ("hdr frac", 1)] {
        let g = |r: &(&str,f64,f64,f64)| if get==0 { r.1 } else { r.2 };
        let (mx,my)=(rows.iter().map(&g).sum::<f64>()/n, rows.iter().map(|r| r.3).sum::<f64>()/n);
        let (mut sxy,mut sxx,mut syy)=(0.0,0.0,0.0);
        for r in &rows { let a=g(r)-mx; let b=r.3-my; sxy+=a*b; sxx+=a*a; syy+=b*b; }
        println!("  correlation({name}, gain) r = {:+.3}", sxy/(sxx.sqrt()*syy.sqrt()));
    }
    println!("  ({:.0}s)", t0.elapsed().as_secs_f64());
}
