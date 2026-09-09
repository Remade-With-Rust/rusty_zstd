//! 2-D sweep of DFast's two search cuts, with the nl dispatch on.
//! Size only; exact. Prints the grid and the best cell.
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
    rz::set_nl_dispatch_arm(false);
    rz::set_dfast_good_ml_arm(0);
    rz::set_dfast_good_ml2_arm(0);
    let ship = go();
    println!("shipped default (nl_dispatch off, cuts 8/8): {ship}\n");
    let ml1 = [24usize, 32, 40, 48, 56, 64];
    let ml2 = [0usize, 12, 16, 20, 24, 32, 48];
    print!("{:>8}", "ml/ml2");
    for b in ml2 { print!("{:>10}", if b == 0 { "follow".to_string() } else { b.to_string() }); }
    println!();
    let mut best = (isize::MAX, 0usize, 0usize);
    rz::set_nl_dispatch_arm(true);
    for a in ml1 {
        print!("{a:>8}");
        for b in ml2 {
            rz::set_dfast_good_ml_arm(a);
            rz::set_dfast_good_ml2_arm(b);
            let d = go() as isize - ship as isize;
            if d < best.0 { best = (d, a, b); }
            print!("{d:>+10}");
        }
        println!();
    }
    rz::set_nl_dispatch_arm(false);
    rz::set_dfast_good_ml_arm(0);
    rz::set_dfast_good_ml2_arm(0);
    println!("\nBEST: good_ml={} good_ml2={}  {:+} B  ({:+.3}%)",
             best.1, best.2, best.0, best.0 as f64 / ship as f64 * 100.0);
}
