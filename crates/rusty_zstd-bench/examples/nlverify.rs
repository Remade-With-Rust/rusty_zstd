//! Verification for the nl_dispatch size win: round-trip every corpus at every
//! DFast level with the dispatch ON, and confirm the win is stable in the
//! corpus prefix (a win that shrinks as you add data is a warm-up artefact).
use rusty_zstd as rz;
const IDS: &[&str] = &["zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m",
    "versions-16m","mr","ooffice","osdb","reymont","sao","webster","dickens","mozilla",
    "nci","samba","xml","x-ray"];
fn main() {
    for cap in [2usize << 20, 4 << 20, 8 << 20] {
        let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
        }).collect();
        let total: usize = srcs.iter().map(|(_, s)| s.len()).sum();
        for lvl in [3i32, 4] {
            let o = rz::CompressOptions { level: lvl, checksum: true };
            let mut a = 0usize;
            let mut b = 0usize;
            let mut rt = 0usize;
            for (_, s) in &srcs {
                rz::set_nl_dispatch_arm(false);
                a += rz::compress_with(s, o).unwrap().len();
                rz::set_nl_dispatch_arm(true);
                let z = rz::compress_with(s, o).unwrap();
                b += z.len();
                let d = rz::decompress(&z).expect("decompress");
                assert_eq!(&d, s, "ROUND-TRIP FAILED L{lvl}");
                rt += 1;
            }
            rz::set_nl_dispatch_arm(false);
            println!("cap {:>2} MiB  L{lvl}  in {:>9} B   base {:>9}  disp {:>9}  \
{:+.3}%  ({:+} B)  round-trips {rt}/{}",
                     cap >> 20, total, a, b,
                     (b as f64 - a as f64) / a as f64 * 100.0, b as i64 - a as i64, srcs.len());
        }
    }
}
