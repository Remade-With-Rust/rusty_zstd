//! GATE 14 SIGNAL SEARCH. Truth table = the SIZE cost of cutting depth by 2,
//! which spans -0.083% (x-ray) to +24.655% (xml).
//!
//! `full%` is refuted as a signal: mozilla 9.11% costs +0.115% while webster
//! 11.28% costs +4.648%, and nci at 52.02% costs LESS than xml at 19.83%.
//!
//! Candidate: the NO-GAIN probe share. BT_NOGAIN counts probes whose match was
//! no better than the best already found. If most probes are unproductive, depth
//! is buying nothing and the cut should be cheap -- that is the mechanism, not a
//! correlate.
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(13);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(512 << 10);
    println!("GATE 14 SIGNAL @ L{lvl} — no-gain share vs the cost of cutting depth");
    println!("{:<13} {:>10} {:>10} {:>10} | {:>10} {:>10}", "corpus", "nogain%", "short%", "mean ml", "iters -2%", "size -2%");
    let mut rows = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::set_search_log_delta(0);
        let _ = rusty_zstd::take_bt_probe_stats(); let _ = rusty_zstd::take_bt_iters();
        let z0 = rusty_zstd::compress(s, lvl).unwrap();
        let (probe, short, nogain) = rusty_zstd::take_bt_probe_stats();
        let (_, it0, _) = rusty_zstd::take_bt_iters();
        if probe < 1000 { continue }
        rusty_zstd::set_search_log_delta(-2);
        let _ = rusty_zstd::take_bt_iters();
        let z2 = rusty_zstd::compress(s, lvl).unwrap();
        let (_, it2, _) = rusty_zstd::take_bt_iters();
        rusty_zstd::set_search_log_delta(0);
        assert!(rusty_zstd::decompress(&z2).unwrap() == s, "{id}: round-trip");
        let ng = nogain as f64/probe as f64*100.0;
        let sh = short as f64/probe as f64*100.0;
        let sz = (z2.len() as f64/z0.len() as f64 - 1.0)*100.0;
        let itd = (it2 as f64/it0.max(1) as f64 - 1.0)*100.0;
        println!("{:<13} {:>9.2}% {:>9.2}% {:>10.2} {:>9.1}% {:>9.3}%", id, ng, sh, 0.0, itd, sz);
        rows.push((*id, ng, sh, sz, itd));
    }
    // correlate each candidate with the size cost
    let n = rows.len() as f64;
    for (name, idx) in [("nogain%", 1usize), ("short%", 2)] {
        let get = |r: &(&str,f64,f64,f64,f64)| if idx==1 { r.1 } else { r.2 };
        let (mx,my)=(rows.iter().map(&get).sum::<f64>()/n, rows.iter().map(|r| r.3).sum::<f64>()/n);
        let (mut sxy,mut sxx,mut syy)=(0.0,0.0,0.0);
        for r in &rows { let a=get(r)-mx; let b=r.3-my; sxy+=a*b; sxx+=a*a; syy+=b*b; }
        println!("  correlation({name}, size cost) r = {:.3}", sxy/(sxx.sqrt()*syy.sqrt()));
    }
    rows.sort_by(|a,b| a.3.partial_cmp(&b.3).unwrap());
    println!("\n  ranked by SIZE COST of the cut (cheapest first):");
    for (id,ng,sh,sz,itd) in &rows {
        println!("    {:<13} size {:>8.3}%  iters {:>7.1}%  nogain {:>6.2}%  short {:>6.2}%", id, sz, itd, ng, sh);
    }
}
