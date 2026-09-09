//! Sweep DFast's two "good enough, stop searching" cuts for SIZE.
//!
//! `nl_dispatch` buys -0.25% by raising `good_ml` 8 -> 24 when the offset trade
//! is paying. Both cuts were hardcoded `8` and are now knobs; nothing has swept
//! them for the value that is actually best. Size is exact -- no clock.
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao",
    "webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn main() {
    let cap: usize = 4 << 20;
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    let o = rz::CompressOptions { level: 3, checksum: false };
    let go = || -> usize { srcs.iter().map(|(_, s)|
        rz::compress_with(s, o).unwrap().len()).sum() };
    for disp in [false, true] {
        rz::set_nl_dispatch_arm(disp);
        rz::set_dfast_good_ml_arm(0);
        rz::set_dfast_good_ml2_arm(0);
        let base = go();
        println!("\n=== nl_dispatch {} ===  base {base}", if disp {"ON"} else {"off"});
        println!("  {:>10} {:>12} {:>10}   {:>12} {:>10}",
                 "value", "good_ml", "delta", "good_ml2", "delta");
        for v in [4usize, 6, 8, 12, 16, 20, 24, 32, 48, 64] {
            rz::set_dfast_good_ml_arm(v);
            rz::set_dfast_good_ml2_arm(0);
            let a = go();
            rz::set_dfast_good_ml_arm(0);
            rz::set_dfast_good_ml2_arm(v);
            let b = go();
            println!("  {:>10} {:>12} {:>+10}   {:>12} {:>+10}",
                     v, a, a as i64 - base as i64, b, b as i64 - base as i64);
        }
        rz::set_dfast_good_ml_arm(0);
        rz::set_dfast_good_ml2_arm(0);
    }
    rz::set_nl_dispatch_arm(false);
}
