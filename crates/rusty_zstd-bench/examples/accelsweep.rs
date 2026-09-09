//! Sweep C's incompressible-section acceleration on our chain ladder.
//!
//! Two questions, two currencies:
//!  * SIZE across the board -- exact, load-immune.
//!  * TIME on incompressible data -- measured ONLY as an arm-vs-arm ratio in
//!    ONE process with a null, so a loaded box moves both arms together.
use rusty_zstd as rz;
use std::time::Instant;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao",
    "webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m","incomp-32m"];
fn main() {
    let cap = 1usize << 20;
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    let inc: Vec<u8> = srcs.iter().find(|(i, _)| *i == "incomp-32m").unwrap().1.clone();
    let go = |lvl: i32| -> u64 { srcs.iter().map(|(_, s)|
        rz::compress_with(s, rz::CompressOptions { level: lvl, checksum: false })
            .unwrap().len() as u64).sum() };
    let t_inc = |lvl: i32| -> f64 {
        let p = rz::compression_params(lvl, Some(inc.len() as u64)).unwrap();
        let mut b = f64::MAX;
        for _ in 0..9 {
            let t = Instant::now();
            let z = rz::compress_with_params(&inc, p, false).unwrap();
            let e = t.elapsed().as_secs_f64();
            std::hint::black_box(z.len());
            if e < b { b = e }
        }
        b * 1000.0
    };
    for lvl in [5i32, 7, 9, 12] {
        rz::set_lazy_accel_arm(0);
        let base = go(lvl);
        let tb = t_inc(lvl);
        let tb2 = t_inc(lvl);                       // null arm: same setting twice
        println!("\n=== L{lvl} ===  base {base} B | incomp {tb:.2} ms (null {:+.1}%)",
                 (tb2 / tb - 1.0) * 100.0);
        println!("  {:>6}{:>12}{:>10}{:>12}{:>10}", "shift", "bytes", "d size", "incomp ms", "speedup");
        for sh in [4usize, 6, 7, 8, 9, 10, 12] {
            rz::set_lazy_accel_arm(sh);
            let n = go(lvl);
            let t = t_inc(lvl);
            println!("  {:>6}{:>12}{:>+10}{:>12.2}{:>9.2}x", sh, n, n as i64 - base as i64, t, tb / t);
        }
        rz::set_lazy_accel_arm(0);
    }
}
