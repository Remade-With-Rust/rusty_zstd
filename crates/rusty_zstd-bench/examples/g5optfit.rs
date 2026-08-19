//! GATE 5 @ L19 -- fit the OPT-ladder thresholds. Fast and the middle ladder
//! must not move.
const TRAIN: &[&str] = &["dickens","mozilla","nci","samba","xml","x-ray"];
const HOLDOUT: &[&str] = &["mr","ooffice","osdb","reymont","sao","webster"];
const GEN: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
const CAPS: &[usize] = &[512<<10, 1<<20];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(19);
    let mut cells: Vec<(&str, Vec<u8>, usize)> = Vec::new();
    for id in TRAIN.iter().chain(HOLDOUT).chain(GEN) {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        for &c in CAPS { if f.len() < c { continue }
            rusty_zstd::set_g5_opt_arms(-1.0, 2.0, 2.0);
            let base = rusty_zstd::compress(&f[..c], lvl).unwrap().len();
            cells.push((*id, f[..c].to_vec(), base));
        }
    }
    println!("GATE 5 @ L{lvl} OPT FIT -- {} cells", cells.len());
    let mut cands: Vec<(f64,f64,f32,f32,f32)> = Vec::new();
    for &r in &[0.30f32, 2.0] {
      for &a in &[0.40f32, 0.50, 0.60, 0.70] {
        for &d in &[0.5f32, 1.0, 1.5, 2.0] {
            rusty_zstd::set_g5_opt_arms(r, a, d);
            let (mut t, mut b, mut worst) = (0i64, 0i64, f64::MIN);
            for (id, v, base) in &cells {
                if !TRAIN.contains(id) { continue }
                let n = rusty_zstd::compress(v, lvl).unwrap().len();
                let pc = (n as f64 / *base as f64 - 1.0)*100.0;
                if pc > worst { worst = pc }
                t += n as i64; b += *base as i64;
            }
            cands.push((worst, (t as f64/b as f64-1.0)*100.0, r, a, d));
        }
      }
    }
    let safe: Vec<_> = cands.iter().filter(|c| c.0 <= 0.01).collect();
    let pick = if let Some(b) = safe.iter().min_by(|x,y| x.1.partial_cmp(&y.1).unwrap()) { **b }
               else { cands.iter().cloned().min_by(|x,y| x.0.partial_cmp(&y.0).unwrap()).unwrap() };
    let (pw, trt, r, a, d) = pick;
    println!("  {} safe (worst<=+0.01%); PICK rep>={r} ratio>={a} drift>={d}  train {trt:+.4}% worst {pw:+.3}%", safe.len());
    rusty_zstd::set_g5_opt_arms(r, a, d);
    for (name, set) in [("train", TRAIN), ("HOLDOUT", HOLDOUT), ("generated", GEN)] {
        let (mut t, mut b, mut worst, mut wid, mut best, mut bid, mut imp) = (0i64,0i64,f64::MIN,"",f64::MAX,"",0);
        for (id, v, base) in &cells {
            if !set.contains(id) { continue }
            let n = rusty_zstd::compress(v, lvl).unwrap().len();
            let pc = (n as f64/ *base as f64 - 1.0)*100.0;
            if pc > worst { worst = pc; wid = id }
            if pc < best { best = pc; bid = id }
            if n < *base { imp += 1 }
            t += n as i64; b += *base as i64;
        }
        if b > 0 { println!("  {:<10} total {:+.4}%  worst {:+.3}% ({wid})  best {:+.3}% ({bid})  improved {imp}",
            name, (t as f64/b as f64-1.0)*100.0, worst, best); }
    }
}
