//! GATE 13 @ L1 decided DETERMINISTICALLY.
//!
//! The width sweep's clock is not admissible: `text-32m` makes 5 push_literals
//! calls and `x-ray` makes 42, so width CANNOT affect them -- yet they read
//! -9.09% and -4.46%. Those are null arms and they put the noise floor above
//! every width difference measured.
//!
//! Store traffic is deterministic. For width w:
//!   fast calls  = those with n <= w            -> write w bytes each
//!   slow calls  = the rest (extend_from_slice) -> write n bytes each
//! Total bytes written is then a counter, not a duration.
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","nci","xml","mozilla","samba","webster","reymont","dickens","osdb","ooffice","mr","sao","versions-16m"];
/// bucket upper bounds and a representative mean length per bucket
const HI: [usize; 6] = [4, 8, 16, 32, 64, 128];
const MEAN: [f64; 6] = [2.5, 6.5, 12.5, 24.5, 48.0, 160.0];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    println!("GATE 13 @ L{lvl} — store traffic per width (deterministic)");
    println!("{:<13} {:>10} | {:>11} {:>11} {:>11} | {:>6} {:>9}", "corpus", "calls", "w8 KB", "w16 KB", "w32 KB", "best", "vs w16");
    let (mut w8, mut w16, mut w32) = (0, 0, 0);
    let (mut t8, mut t16, mut t32) = (0.0f64, 0.0f64, 0.0f64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let _ = rusty_zstd::take_lp_stats();
        let _ = rusty_zstd::compress(s, lvl).unwrap();
        let (h, _, _) = rusty_zstd::take_lp_stats();
        let calls: u64 = h.iter().sum();
        if calls < 1000 { continue }
        let bytes = |w: usize| -> f64 {
            let mut b = 0.0;
            for i in 0..6 {
                if HI[i] <= w { b += h[i] as f64 * w as f64 }        // fast path: writes w
                else { b += h[i] as f64 * MEAN[i] }                   // slow path: writes n
            }
            b
        };
        let (b8, b16, b32) = (bytes(8), bytes(16), bytes(32));
        t8 += b8; t16 += b16; t32 += b32;
        let v = [b8, b16, b32];
        let bi = (0..3).min_by(|a,b| v[*a].partial_cmp(&v[*b]).unwrap()).unwrap();
        match bi { 0 => w8 += 1, 1 => w16 += 1, _ => w32 += 1 }
        println!("{:<13} {:>10} | {:>11.0} {:>11.0} {:>11.0} | {:>6} {:>8.1}%",
            id, calls, b8/1e3, b16/1e3, b32/1e3, [8,16,32][bi], (v[bi]/b16-1.0)*100.0);
    }
    println!("\n  best width by store traffic: w8 {w8}, w16 {w16}, w32 {w32}");
    println!("  totals  w8 {:.1} MB   w16 {:.1} MB   w32 {:.1} MB", t8/1e6, t16/1e6, t32/1e6);
    println!("  w8 vs w16: {:+.1}%   w32 vs w16: {:+.1}%", (t8/t16-1.0)*100.0, (t32/t16-1.0)*100.0);
}
