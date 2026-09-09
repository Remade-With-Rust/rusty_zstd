//! The candidate shipping config vs the shipped default: size, work, round-trip.
//!   nl_dispatch ON + dfast_good_ml 48 (good_ml2 follows)
//! Chosen off the `mlgrid` plateau rather than its argmax: 40..64 all land
//! within 0.03% of each other, so the extreme cell (64/24, -82,975) is 322 B
//! better than 48/follow (-82,653) and far more likely to be this corpus set.
use rusty_zstd as rz;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m",
    "versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla",
    "nci","samba","xml","x-ray"];
fn set(on: bool) {
    rz::set_nl_dispatch_arm(on);
    rz::set_dfast_good_ml_arm(if on { 48 } else { 0 });
}
fn main() {
    for cap in [4usize << 20, 8 << 20] {
        let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
        }).collect();
        for lvl in [3i32, 4] {
            let o = rz::CompressOptions { level: lvl, checksum: true };
            let mut r = [(0u64, 0u64, 0u64, 0u64); 2];
            let mut rt = 0;
            for (i, on) in [false, true].iter().enumerate() {
                set(*on);
                let _ = rz::take_mm();
                for (_, s) in &srcs {
                    rz::prof_reset();
                    let z = rz::compress_with(s, o).unwrap();
                    r[i].0 += z.len() as u64;
                    let c = rz::prof_encode_counts();
                    r[i].1 += c.hash_probes; r[i].2 += c.hash_fills;
                    if *on {
                        assert_eq!(&rz::decompress(&z).expect("dec"), s, "ROUND-TRIP L{lvl}");
                        rt += 1;
                    }
                }
                r[i].3 = rz::take_mm().0;
            }
            set(false);
            let pc = |a: u64, b: u64| (b as f64 - a as f64) / a as f64 * 100.0;
            println!("cap {:>2} MiB L{lvl}  size {:+.3}% ({:+} B)  cand {:+.2}%  fills {:+.2}%  \
pos {:+.2}%  rt {rt}/{}",
                     cap >> 20, pc(r[0].0, r[1].0), r[1].0 as i64 - r[0].0 as i64,
                     pc(r[0].1, r[1].1), pc(r[0].2, r[1].2), pc(r[0].3, r[1].3), srcs.len());
        }
    }
}
