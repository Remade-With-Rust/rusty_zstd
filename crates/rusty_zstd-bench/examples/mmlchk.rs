//! Both signal probes used 8-grams. What min_match does the level actually use?
//! If the finder matches at 3, an 8-gram probe measures the wrong thing and
//! x-ray -- whose matches may all be short -- would look like it has no far-region
//! content when it has plenty.
use std::collections::HashSet;
fn gram4(b: &[u8], i: usize) -> u32 { u32::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3]]) }
fn gram8(b: &[u8], i: usize) -> u64 { u64::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3],b[i+4],b[i+5],b[i+6],b[i+7]]) }
fn main() {
    for lvl in [1i32, 3, 19, 22] {
        let p = rusty_zstd::compression_params(lvl, Some(1 << 20)).unwrap();
        println!("L{lvl}: min_match {} strategy {:?}", p.min_match, p.strategy);
    }
    const PRE: usize = 4 << 20; const PAY: usize = 1 << 20; const NARROW: usize = 1 << 20;
    println!("\n{:<13} {:>12} {:>12}   (far-region hit rate on the payload)", "corpus", "8-gram", "4-gram");
    for id in ["x-ray","versions-16m","webster","ooffice","mozilla","nci"] {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let far = &f[..PRE - NARROW];
        let tail = &f[PRE..PRE+PAY];
        let (mut s8, mut s4) = (HashSet::new(), HashSet::new());
        let mut i = 0; while i + 8 <= far.len() { s8.insert(gram8(far,i)); s4.insert(gram4(far,i)); i += 4; }
        let (mut h8, mut h4, mut n) = (0u64,0u64,0u64);
        let mut j = 0; while j + 8 <= tail.len() { n += 1;
            if s8.contains(&gram8(tail,j)) { h8 += 1; }
            if s4.contains(&gram4(tail,j)) { h4 += 1; } j += 4; }
        println!("{:<13} {:>11.4}% {:>11.4}%", id, h8 as f64/n as f64*100.0, h4 as f64/n as f64*100.0);
    }
}
