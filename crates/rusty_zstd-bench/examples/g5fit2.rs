//! GATE 5 re-fit: the first fit used ONE input size and `samba` flipped sign with
//! size (+0.459% at 4 MiB, -0.151% at 8 MiB). A threshold that generalises across
//! CONTENT but not across SIZE is not fitted yet.
//!
//! So: fit on TRAIN across four caps at once, constrained so no train corpus
//! regresses at ANY of them, then judge once on HOLDOUT across the same caps.
const TRAIN: &[&str] = &["dickens","mozilla","nci","samba","xml","x-ray"];
const HOLDOUT: &[&str] = &["mr","ooffice","osdb","reymont","sao","webster"];
const GEN: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
const CAPS: &[usize] = &[1<<20, 2<<20, 4<<20, 8<<20];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let mut data: Vec<(&str, usize, Vec<u8>, usize)> = Vec::new();
    for id in TRAIN.iter().chain(HOLDOUT).chain(GEN) {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        for &c in CAPS {
            if f.len() < c { continue }
            rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
            let base = rusty_zstd::compress(&f[..c], lvl).unwrap().len();
            data.push((*id, c, f[..c].to_vec(), base));
        }
    }
    println!("GATE 5 RE-FIT @ L{lvl} — {} (corpus, size) cells", data.len());
    let mut best = (f64::MAX, 0.0f32, 0.0f32, 0.0f32);
    let mut cands: Vec<(f64,f64,f32,f32,f32)> = Vec::new();
    for &r in &[0.10f32, 0.20, 0.30, 0.50] {
      for &a in &[0.60f32, 0.70, 0.80, 0.90] {
        for &d in &[0.10f32, 0.20, 0.30, 0.50] {
            rusty_zstd::set_g5_arms(r, a, d);
            let (mut t, mut b, mut worst) = (0i64, 0i64, f64::MIN);
            for (id, _, v, base) in &data {
                if !TRAIN.contains(id) { continue }
                let n = rusty_zstd::compress(v, lvl).unwrap().len();
                let pc = (n as f64 / *base as f64 - 1.0)*100.0;
                if pc > worst { worst = pc }
                t += n as i64; b += *base as i64;
            }
            let total = (t as f64/b as f64 - 1.0)*100.0;
            cands.push((worst, total, r, a, d));
            if worst <= 0.02 && total < best.0 { best = (total, r, a, d); }
        }
      }
    }
    cands.sort_by(|x,y| x.0.partial_cmp(&y.0).unwrap());
    println!("  no config met worst <= +0.02%. Best achievable worst-case on TRAIN:");
    for (w,t,r,a,d) in cands.iter().take(6) {
        println!("    worst {:+.3}%  total {:+.4}%   rep>={r} ratio>={a} drift>={d}", w, t);
    }
    let (_, _, r, a, d) = cands[0];
    let trt = cands[0].1;
    println!("  taking the safest: rep>={r} ratio>={a} drift>={d}, train total {trt:+.4}%");
    rusty_zstd::set_g5_arms(r, a, d);
    for (name, set) in [("train", TRAIN), ("HOLDOUT", HOLDOUT), ("generated", GEN)] {
        let (mut t, mut b, mut worst, mut wid) = (0i64, 0i64, f64::MIN, "");
        for (id, _, v, base) in &data {
            if !set.contains(id) { continue }
            let n = rusty_zstd::compress(v, lvl).unwrap().len();
            let pc = (n as f64/ *base as f64 - 1.0)*100.0;
            if pc > worst { worst = pc; wid = id }
            t += n as i64; b += *base as i64;
        }
        if b > 0 {
            println!("  {:<10} total {:+.4}%   worst {:+.3}% ({})", name, (t as f64/b as f64-1.0)*100.0, worst, wid);
        }
    }
}
