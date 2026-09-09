//! Price the nl_dispatch size win in WORK, deterministically.
//!
//! The dispatch raises DFast's "good enough, stop searching" cut from 8 to 24,
//! so it must be bought with search. This measures the exact cost in the
//! encoder's own counters -- positions scanned, candidates evaluated, table
//! fills -- alongside the byte win. No clock: this box cannot resolve the
//! likely effect anyway (+-1.5% null, `eqlever.rs`).
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","mr","ooffice","osdb","reymont","sao",
    "webster","dickens","mozilla","nci","samba","xml","x-ray"];
fn run(srcs: &Vec<(&str, Vec<u8>)>, lvl: i32) -> (u64, u64, u64, u64, u64) {
    let (mut b, mut p, mut f, mut s) = (0u64, 0u64, 0u64, 0u64);
    let _ = rz::take_mm();
    for (_, x) in srcs {
        rz::prof_reset();
        b += rz::compress_with(x, rz::CompressOptions { level: lvl, checksum: false })
            .unwrap().len() as u64;
        let c = rz::prof_encode_counts();
        p += c.hash_probes; f += c.hash_fills; s += c.seqs;
    }
    (b, p, f, s, rz::take_mm().0)
}
fn main() {
    let cap: usize = 4 << 20;
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    for lvl in [3i32, 4] {
        rz::set_nl_dispatch_arm(false);
        let a = run(&srcs, lvl);
        rz::set_nl_dispatch_arm(true);
        let b = run(&srcs, lvl);
        rz::set_nl_dispatch_arm(false);
        let pc = |x: u64, y: u64| (y as f64 - x as f64) / x as f64 * 100.0;
        println!("=== L{lvl} ===");
        println!("  bytes      {:>12} -> {:>12}   {:+.3}%  ({:+} B)",
                 a.0, b.0, pc(a.0, b.0), b.0 as i64 - a.0 as i64);
        println!("  positions  {:>12} -> {:>12}   {:+.2}%", a.4, b.4, pc(a.4, b.4));
        println!("  candidates {:>12} -> {:>12}   {:+.2}%", a.1, b.1, pc(a.1, b.1));
        println!("  fills      {:>12} -> {:>12}   {:+.2}%", a.2, b.2, pc(a.2, b.2));
        println!("  sequences  {:>12} -> {:>12}   {:+.2}%", a.3, b.3, pc(a.3, b.3));
    }
}
