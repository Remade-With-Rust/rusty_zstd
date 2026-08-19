//! GATE 5 re-fit across LEVEL and SIZE together.
//!
//! The shipped thresholds were fitted at L3 across sizes. At L1 they fail two
//! ways: mozilla +0.208% and samba +0.153% at 8 MiB (a regression I shipped,
//! because the level check used a 4 MiB cap), and versions sits at 0.000% while
//! -3.935% is available, because the rep_yield guard that protects it at L3 is
//! wrong at L1.
//!
//! A threshold fitted on one level is not fitted. Fit on TRAIN over
//! levels x sizes, constrained so NO train cell regresses, then judge once.
const TRAIN: &[&str] = &["dickens","mozilla","nci","samba","xml","x-ray"];
const HOLDOUT: &[&str] = &["mr","ooffice","osdb","reymont","sao","webster"];
const GEN: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
const LVLS: &[i32] = &[1, 3, 9];
const CAPS: &[usize] = &[2<<20, 8<<20];
fn main() {
    let mut cells: Vec<(&str, i32, Vec<u8>, usize)> = Vec::new();
    for id in TRAIN.iter().chain(HOLDOUT).chain(GEN) {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        for &c in CAPS { if f.len() < c { continue }
            for &l in LVLS {
                rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
                let base = rusty_zstd::compress(&f[..c], l).unwrap().len();
                cells.push((*id, l, f[..c].to_vec(), base));
            }
        }
    }
    println!("GATE 5 RE-FIT across level x size — {} cells", cells.len());
    let mut cands: Vec<(f64,f64,f32,f32,f32)> = Vec::new();
    for &r in &[0.10f32, 0.30, 0.60, 0.90, 2.0] {
      for &a in &[0.60f32, 0.70, 0.80, 0.90] {
        for &d in &[1.0f32, 1.5, 2.0, 3.0] {
            rusty_zstd::set_g5_arms(r, a, d);
            let (mut t, mut b, mut worst) = (0i64, 0i64, f64::MIN);
            for (id, l, v, base) in &cells {
                if !TRAIN.contains(id) { continue }
                let n = rusty_zstd::compress(v, *l).unwrap().len();
                let pc = (n as f64 / *base as f64 - 1.0)*100.0;
                if pc > worst { worst = pc }
                t += n as i64; b += *base as i64;
            }
            cands.push((worst, (t as f64/b as f64-1.0)*100.0, r, a, d));
        }
      }
    }
    // safest first, then best total among the safe ones
    cands.sort_by(|x,y| x.0.partial_cmp(&y.0).unwrap().then(x.1.partial_cmp(&y.1).unwrap()));
    println!("  best TRAIN configs (worst, then total):");
    for (w,t,r,a,d) in cands.iter().take(5) {
        println!("    worst {:+.3}%  total {:+.4}%   rep>={r} ratio>={a} drift>={d}", w, t);
    }
    let safe: Vec<_> = cands.iter().filter(|c| c.0 <= 0.02).collect();
    let pick = if let Some(best) = safe.iter().min_by(|x,y| x.1.partial_cmp(&y.1).unwrap()) { **best } else { cands[0] };
    let (_, trt, r, a, d) = pick;
    println!("  PICK: rep>={r} ratio>={a} drift>={d}  train total {trt:+.4}%");
    rusty_zstd::set_g5_arms(r, a, d);
    for (name, set) in [("train", TRAIN), ("HOLDOUT", HOLDOUT), ("generated", GEN)] {
        let (mut t, mut b, mut worst, mut wid, mut wl) = (0i64, 0i64, f64::MIN, "", 0);
        for (id, l, v, base) in &cells {
            if !set.contains(id) { continue }
            let n = rusty_zstd::compress(v, *l).unwrap().len();
            let pc = (n as f64/ *base as f64 - 1.0)*100.0;
            if pc > worst { worst = pc; wid = id; wl = *l }
            t += n as i64; b += *base as i64;
        }
        if b > 0 { println!("  {:<10} total {:+.4}%   worst {:+.3}% ({wid} L{wl})", name, (t as f64/b as f64-1.0)*100.0, worst); }
    }
}
