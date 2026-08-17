//! Gate 1 @ L1 — proper timing for the two corpora the single-shot table put at
//! 3-4 ms (timer resolution). Best-of-N, ABBA, both arms in one process.
use rusty_zstd::Strategy;

fn best(src: &[u8], st: Option<Strategy>, n: usize) -> (f64, usize) {
    rusty_zstd::set_strategy_arm(st);
    let mut b = f64::MAX;
    let mut sz = 0usize;
    for _ in 0..n {
        let t = std::time::Instant::now();
        let z = rusty_zstd::compress(src, 1).unwrap();
        let e = t.elapsed().as_secs_f64() * 1000.0;
        if e < b {
            b = e;
        }
        sz = z.len();
    }
    rusty_zstd::set_strategy_arm(None);
    (b, sz)
}

fn main() {
    let n: usize = 41;
    for id in ["versions-16m", "text-32m", "nci", "xml", "webster"] {
        let src = match std::fs::read(format!("corpora/data/generated/{id}")) {
            Ok(v) => v,
            Err(_) => match std::fs::read(format!("corpora/data/silesia/{id}")) {
                Ok(v) => v,
                Err(_) => continue,
            },
        };
        let (mf1, sf) = best(&src, Some(Strategy::Fast), n);
        let (ml, sl) = best(&src, Some(Strategy::Lazy), n);
        let (mf2, _) = best(&src, Some(Strategy::Fast), n);
        let fm = mf1.min(mf2);
        let sp = 100.0 * (sl as f64 - sf as f64) / sf as f64;
        let tp = 100.0 * (ml - fm) / fm;
        println!(
            "{:<14} fast {:>10} B {:>8.2} ms | lazy {:>10} B {:>8.2} ms | size {:>7.2} % | time {:>8.1} %",
            id, sf, fm, sl, ml, sp, tp
        );
    }
}
