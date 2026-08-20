//! The three MIS-FITTED candidates, adjudicated per corpus and on BOTH axes.
//! A mean is not a verdict here: the finish line is worst-corpus <= 0.
use std::time::Instant;
const IDS: &[&str] = &["dickens","samba","xml","nci","mozilla","x-ray","sao","webster","mr","osdb","reymont","ooffice"];
// train/holdout per corpus::list_silesia
fn is_holdout(id: &str) -> bool { matches!(id, "mr"|"ooffice"|"osdb"|"reymont"|"sao"|"webster") }
fn run(name: &str, lvl: i32, shipped: f32, cand: f32, set: impl Fn(f32)) {
    let cap = 4 << 20;
    println!("\n{name}: shipped {shipped} vs candidate {cand}  (L{lvl})");
    println!("   {:<10} {:>12} {:>12} {:>9} {:>9}", "corpus", "shipped B", "cand B", "size%", "time%");
    let (mut ts, mut tc) = (0u64, 0u64);
    let (mut worst, mut worst_id) = (f64::MIN, "");
    let (mut trn, mut hld) = ((0u64,0u64), (0u64,0u64));
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        let s = &f[..f.len().min(cap)];
        let mut bs = f64::MAX; let mut bc = f64::MAX;
        let (mut zs, mut zc) = (0usize, 0usize);
        for pass in 0..3 {
            for which in [pass % 2 == 0, pass % 2 != 0] {
                set(if which { cand } else { shipped });
                for _ in 0..5 {
                    let t = Instant::now();
                    let z = rusty_zstd::compress(s, lvl).unwrap();
                    let e = t.elapsed().as_secs_f64() * 1000.0;
                    if which { if e < bc { bc = e } zc = z.len(); } else { if e < bs { bs = e } zs = z.len(); }
                }
            }
        }
        let sp = (zc as f64 / zs as f64 - 1.0) * 100.0;
        if sp > worst { worst = sp; worst_id = id }
        ts += zs as u64; tc += zc as u64;
        if is_holdout(id) { hld.0 += zs as u64; hld.1 += zc as u64 } else { trn.0 += zs as u64; trn.1 += zc as u64 }
        println!("   {:<10} {:>12} {:>12} {:>8.4}% {:>8.2}%", id, zs, zc, sp, (bc/bs - 1.0)*100.0);
    }
    set(shipped);
    println!("   TOTAL {:+.4}%   TRAIN {:+.4}%   HOLDOUT {:+.4}%   WORST {worst_id} {:+.4}%",
        (tc as f64/ts as f64 - 1.0)*100.0,
        (trn.1 as f64/trn.0 as f64 - 1.0)*100.0,
        (hld.1 as f64/hld.0 as f64 - 1.0)*100.0, worst);
}
fn main() {
    run("pair_gain_min", 1, 1.0, 0.25, rusty_zstd::set_pair_gain_arm);
    run("pair_rep_max", 1, 0.7, 0.2, |v| std::env::set_var("RZSTD_PAIR_T", v.to_string()));
    run("pair_rate_hi", 1, 1.0, 4.0, rusty_zstd::set_pair_hi_arm);
}
