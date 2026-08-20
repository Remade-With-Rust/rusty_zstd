//! T4: which route does each MATCH COPY actually take in the decoder?
//!
//! `copy_match` has an offset-1 splat, a 32-byte tier and a 16-byte tier, then
//! falls through to `extend_from_within` (a runtime-length memcpy CALL) or, when
//! the match overlaps itself, to a loop. Before widening anything, measure where
//! the traffic is -- by CALLS and by BYTES, so a rare-but-long band cannot hide.
const IDS: &[&str] = &["dickens","samba","xml","nci","webster","mozilla","x-ray","sao","mr","osdb","reymont","ooffice"];
const NAMES: [&str; 6] = ["offset-1 splat", "32B tier", "16B tier", "extend_from_within", "overlap loop", "32B tier, len<=16"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = 8 << 20;
    let mut tc = [0u64; 6];
    let mut tb = [0u64; 6];
    println!("T4 MATCH-COPY BAND CENSUS @ L{lvl}\n");
    println!("{:<13} {:>10} {:>10} {:>10} {:>12} {:>12}", "corpus", NAMES[0], NAMES[1], NAMES[2], NAMES[3], NAMES[4]);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let z = rusty_zstd::compress(s, lvl).unwrap();
        let _ = rusty_zstd::take_dec_bands();
        let d = rusty_zstd::decompress(&z).unwrap();
        assert!(d == s);
        let (c, b) = rusty_zstd::take_dec_bands();
        for i in 0..6 { tc[i] += c[i]; tb[i] += b[i]; }
        println!("{:<13} {:>10} {:>10} {:>10} {:>12} {:>12}", id, c[0], c[1], c[2], c[3], c[4]);
    }
    let sc: u64 = tc.iter().sum();
    let sb: u64 = tb.iter().sum();
    println!("\n  {:<20} {:>12} {:>8}   {:>14} {:>8}", "route", "calls", "share", "bytes moved", "share");
    for i in 0..6 {
        println!("  {:<20} {:>12} {:>7.1}%   {:>14} {:>7.1}%",
            NAMES[i], tc[i], tc[i] as f64 / sc.max(1) as f64 * 100.0,
            tb[i], tb[i] as f64 / sb.max(1) as f64 * 100.0);
    }
    let means: Vec<String> = (0..6)
        .map(|i| format!("{}={:.1}", NAMES[i], tb[i] as f64 / tc[i].max(1) as f64))
        .collect();
    println!("  mean bytes per call: {}", means.join("  "));
}
