//! FINDING 1 signal, take 2: MARGINAL far-region value.
//!
//! Take 1 asked "does the payload match the far region" and failed (r = +0.295):
//! versions-16m scored 97% and gained nothing, because those grams are ALREADY in
//! the near window. High far-hit-rate does not mean the far region is NEEDED.
//!
//! The quantity that matters is what the far region adds that the near window
//! does not already have: grams present in FAR and ABSENT from NEAR, hit by the
//! payload. That is the marginal reach the wider window buys.
use std::collections::HashSet;
const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","jsonlog-16m","smallmsg-8m","versions-16m"];
const PRE: usize = 4 << 20; const PAY: usize = 1 << 20; const NARROW: usize = 1 << 20;
/// reach-only benefit from f1decomp, L19, percent (negative = widening helped)
const TRUTH: &[(&str, f64)] = &[
    ("mozilla",-0.360),("webster",-2.181),("nci",0.017),("samba",-0.140),("osdb",0.156),
    ("dickens",-0.065),("mr",0.075),("xml",-0.054),("reymont",0.028),("sao",0.130),
    ("ooffice",-0.682),("x-ray",-1.198),("jsonlog-16m",0.429),("smallmsg-8m",0.100),("versions-16m",0.000)];
/// 4-GRAM, not 8. L19 has min_match 3 and hashes 4 bytes (`hash4`), so an
/// 8-gram probe measures a match length the finder never requires. On x-ray the
/// two differ by 370x (0.158% vs 58.3%), which is what refuted the first two
/// signal attempts -- a probe defect, not a codec property.
fn gram(b: &[u8], i: usize) -> u32 { u32::from_le_bytes([b[i],b[i+1],b[i+2],b[i+3]]) }
fn main() {
    let stride: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(4);
    println!("MARGINAL far-region value, 4-GRAM (stride {stride})");
    println!("{:<13} {:>10} {:>10} {:>11} {:>11} {:>10}", "corpus", "far-only", "pay hits", "marginal%", "raw far%", "reach%");
    let mut rows = Vec::new();
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if f.len() < PRE + PAY { continue }
        let pre = &f[..PRE];
        let tail = &f[PRE..PRE+PAY];
        let split = PRE - NARROW;
        let (far, near) = (&pre[..split], &pre[split..]);
        let mut nearset: HashSet<u32> = HashSet::new();
        let mut i = 0; while i + 4 <= near.len() { nearset.insert(gram(near, i)); i += stride; }
        let mut faronly: HashSet<u32> = HashSet::new();
        let mut allfar: HashSet<u32> = HashSet::new();
        i = 0; while i + 4 <= far.len() {
            let g = gram(far, i);
            allfar.insert(g);
            if !nearset.contains(&g) { faronly.insert(g); }
            i += stride;
        }
        let (mut mh, mut rh, mut probes) = (0u64, 0u64, 0u64);
        let mut j = 0;
        while j + 4 <= tail.len() {
            let g = gram(tail, j);
            probes += 1;
            if faronly.contains(&g) { mh += 1; }
            if allfar.contains(&g) { rh += 1; }
            j += stride;
        }
        let marg = mh as f64/probes.max(1) as f64*100.0;
        let raw = rh as f64/probes.max(1) as f64*100.0;
        let t = TRUTH.iter().find(|(k,_)| k==id).map(|(_,v)| *v).unwrap_or(0.0);
        println!("{:<13} {:>10} {:>10} {:>10.4}% {:>10.3}% {:>9.3}%", id, faronly.len(), mh, marg, raw, t);
        rows.push((*id, marg, t));
    }
    let n = rows.len() as f64;
    let (mx,my) = (rows.iter().map(|r| r.1).sum::<f64>()/n, rows.iter().map(|r| r.2).sum::<f64>()/n);
    let (mut sxy,mut sxx,mut syy)=(0.0,0.0,0.0);
    for r in &rows { let a=r.1-mx; let b=r.2-my; sxy+=a*b; sxx+=a*a; syy+=b*b; }
    println!("\n  correlation(marginal, reach benefit) r = {:.3}  (want STRONGLY NEGATIVE)", sxy/(sxx.sqrt()*syy.sqrt()));
    let mut s = rows.clone(); s.sort_by(|a,b| b.1.partial_cmp(&a.1).unwrap());
    println!("  ranked by MARGINAL far value:");
    for (id,m,t) in &s { println!("    {:<13} marginal {:>8.4}%  reach {:>7.3}%  {}", id, m, t, if *t < -0.05 {"WIN"} else if *t > 0.05 {"loses"} else {"neutral"}); }
}
