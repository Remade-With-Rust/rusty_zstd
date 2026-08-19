//! GATE 14 SIGNAL, wider sweep. The obvious variable was in the g14bind output
//! all along and never correlated: MEAN WALK DEPTH (iterations / walks).
//!
//! Physically it is the quantity the cap acts on. Capping at 4 truncates almost
//! nothing when the average walk is 2.5 deep, and truncates most of the work when
//! it is 12. `full%`, `nogain%` and `short%` are all one step removed from that.
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m"];
fn r(v: &[(f64, f64)]) -> f64 {
    let n = v.len() as f64;
    let (mx, my) = (v.iter().map(|p| p.0).sum::<f64>()/n, v.iter().map(|p| p.1).sum::<f64>()/n);
    let (mut sxy, mut sxx, mut syy) = (0.0, 0.0, 0.0);
    for (x, y) in v { let a = x-mx; let b = y-my; sxy += a*b; sxx += a*a; syy += b*b; }
    sxy / (sxx.sqrt()*syy.sqrt())
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    println!("GATE 14 SIGNAL SWEEP @ L{lvl}");
    println!("{:<13} {:>9} {:>9} {:>9} {:>9} {:>9} {:>10}",
        "corpus", "meandep", "full%", "nogain%", "short%", "ratio", "size -2%");
    let mut rows: Vec<(&str, f64, f64, f64, f64, f64, f64)> = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::set_search_log_delta(0);
        let _ = rusty_zstd::take_bt_iters(); let _ = rusty_zstd::take_bt_probe_stats();
        let z0 = rusty_zstd::compress(s, lvl).unwrap();
        let (w, it, full) = rusty_zstd::take_bt_iters();
        let (probe, short, nogain) = rusty_zstd::take_bt_probe_stats();
        if w < 1000 || probe < 1000 { continue }
        rusty_zstd::set_search_log_delta(-2);
        let z2 = rusty_zstd::compress(s, lvl).unwrap();
        rusty_zstd::set_search_log_delta(0);
        assert!(rusty_zstd::decompress(&z2).unwrap() == s, "{id}: round-trip");
        let md = it as f64 / w as f64;
        let fp = full as f64 / w as f64 * 100.0;
        let ng = nogain as f64 / probe as f64 * 100.0;
        let sh = short as f64 / probe as f64 * 100.0;
        let ratio = z0.len() as f64 / s.len() as f64;
        let sz = (z2.len() as f64 / z0.len() as f64 - 1.0) * 100.0;
        println!("{:<13} {:>9.2} {:>8.2}% {:>8.2}% {:>8.2}% {:>9.3} {:>9.3}%", id, md, fp, ng, sh, ratio, sz);
        rows.push((*id, md, fp, ng, sh, ratio, sz));
    }
    println!("\n  correlation with the SIZE COST of the cut:");
    for (name, f) in [("mean depth", 0usize), ("full%", 1), ("nogain%", 2), ("short%", 3), ("ratio", 4)] {
        let v: Vec<(f64,f64)> = rows.iter().map(|t| (match f {0=>t.1,1=>t.2,2=>t.3,3=>t.4,_=>t.5}, t.6)).collect();
        println!("    {:<12} r = {:+.3}", name, r(&v));
    }
    // best two-variable products, kept simple and reported honestly
    let v: Vec<(f64,f64)> = rows.iter().map(|t| (t.1 / t.5.max(0.001), t.6)).collect();
    println!("    {:<12} r = {:+.3}", "depth/ratio", r(&v));
    rows.sort_by(|a,b| a.1.partial_cmp(&b.1).unwrap());
    println!("\n  ranked by MEAN DEPTH (shallowest first):");
    for (id, md, _, _, _, _, sz) in &rows {
        println!("    {:<13} depth {:>6.2}   size {:>8.3}%  {}", id, md, sz,
            if *sz < 1.0 { "CHEAP" } else if *sz > 4.0 { "expensive" } else { "mid" });
    }
}
