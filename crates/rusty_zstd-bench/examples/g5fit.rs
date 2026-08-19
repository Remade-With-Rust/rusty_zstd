//! GATE 5 @ L3 -- fit the three thresholds on TRAIN, judge ONCE on HOLDOUT.
//! Split unchanged from corpus::list_silesia; both halves are real Silesia.
use std::time::Instant;
const TRAIN: &[&str] = &["dickens","mozilla","nci","samba","xml","x-ray"];
const HOLDOUT: &[&str] = &["mr","ooffice","osdb","reymont","sao","webster"];
const GEN: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
fn load(id: &str, cap: usize) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/generated/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        .ok().map(|f| { let n = f.len().min(cap); f[..n].to_vec() })
}
fn sz(s: &[u8], lvl: i32) -> usize { rusty_zstd::compress(s, lvl).unwrap().len() }
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    // baseline: gate OFF (all three terms disabled -> always base block size)
    let off = |s: &[u8]| { rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0); sz(s, lvl) };
    println!("GATE 5 FIT @ L{lvl} (cap {} MiB)", cap>>20);
    let grid_rep = [0.30f32, 0.50, 0.70];
    let grid_ratio = [0.60f32, 0.70, 0.80];
    let grid_drift = [0.05f32, 0.10, 0.20];
    let mut data = Vec::new();
    for id in TRAIN.iter().chain(HOLDOUT).chain(GEN) {
        if let Some(v) = load(id, cap) { let b = off(&v); data.push((*id, v, b)); }
    }
    // ---- FIT on TRAIN ----
    let mut best = (f64::MAX, 0.0f32, 0.0f32, 0.0f32);
    for &r in &grid_rep { for &a in &grid_ratio { for &d in &grid_drift {
        rusty_zstd::set_g5_arms(r, a, d);
        let (mut tot, mut base, mut worst) = (0i64, 0i64, f64::MIN);
        for (id, v, b) in &data {
            if !TRAIN.contains(id) { continue }
            let n = sz(v, lvl);
            let pc = (n as f64 / *b as f64 - 1.0) * 100.0;
            if pc > worst { worst = pc }
            tot += n as i64; base += *b as i64;
        }
        // objective: total size, REFUSED if any train corpus regresses > 0.05%
        let total = (tot as f64 / base as f64 - 1.0) * 100.0;
        if worst <= 0.05 && total < best.0 { best = (total, r, a, d); }
    }}}
    let (tr_total, r, a, d) = best;
    println!("  FIT on train: rep>={r}, ratio>={a}, drift>={d}  -> train total {tr_total:+.4}%");
    // ---- JUDGE on HOLDOUT, once ----
    rusty_zstd::set_g5_arms(r, a, d);
    println!("\n{:<13} {:>7} {:>11} {:>11} {:>9}", "corpus", "split", "off", "on", "delta");
    let (mut ht, mut hb, mut hworst) = (0i64, 0i64, f64::MIN);
    let (mut gt, mut gb, mut gworst) = (0i64, 0i64, f64::MIN);
    for (id, v, b) in &data {
        let n = sz(v, lvl);
        let pc = (n as f64 / *b as f64 - 1.0) * 100.0;
        let split = if TRAIN.contains(id) { "train" } else if HOLDOUT.contains(id) { "HOLDOUT" } else { "gen" };
        if split == "HOLDOUT" { ht += n as i64; hb += *b as i64; if pc > hworst { hworst = pc } }
        if split == "gen" { gt += n as i64; gb += *b as i64; if pc > gworst { gworst = pc } }
        println!("{:<13} {:>7} {:>11} {:>11} {:>8.3}%", id, split, b, n, pc);
    }
    println!("\n  HOLDOUT total {:+.4}%  worst corpus {:+.3}%", (ht as f64/hb as f64-1.0)*100.0, hworst);
    println!("  GENERATED total {:+.4}%  worst corpus {:+.3}%", (gt as f64/gb as f64-1.0)*100.0, gworst);
    // time, all 18
    let mut t_off = 0.0; let mut t_on = 0.0;
    for (_, v, _) in &data {
        rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
        let mut b1 = f64::MAX; for _ in 0..5 { let t=Instant::now(); sz(v,lvl); let e=t.elapsed().as_secs_f64()*1000.0; if e<b1 {b1=e} }
        rusty_zstd::set_g5_arms(r, a, d);
        let mut b2 = f64::MAX; for _ in 0..5 { let t=Instant::now(); sz(v,lvl); let e=t.elapsed().as_secs_f64()*1000.0; if e<b2 {b2=e} }
        t_off += b1; t_on += b2;
    }
    println!("  TIME all 18: {t_off:.0} -> {t_on:.0} ms ({:+.2}%)", (t_on/t_off-1.0)*100.0);
}
