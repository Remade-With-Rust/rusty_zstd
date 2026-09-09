//! Per-corpus size impact of the incompressible-section acceleration, and a
//! round-trip on every cell. An aggregate near zero can hide a big regression
//! cancelled by a big gain; this checks.
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb",
    "reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray",
    "text-32m","incomp-32m","zeros-32m"];
fn main() {
    let sh: usize = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(10);
    for cap in [1usize << 20, 4 << 20] {
        println!("\n===== shift {sh}, cap {} MiB =====", cap >> 20);
        println!("{:<14}{:>10}{:>10}{:>10}{:>10}", "corpus", "L5 d", "L7 d", "L9 d", "L12 d");
        println!("{}", "-".repeat(54));
        let mut tot = [0i64; 4];
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            print!("{:<14}", id);
            for (k, lvl) in [5i32, 7, 9, 12].iter().enumerate() {
                let o = rz::CompressOptions { level: *lvl, checksum: false };
                rz::set_lazy_accel_arm(0);
                let a = rz::compress_with(s, o).unwrap().len() as i64;
                rz::set_lazy_accel_arm(sh);
                let z = rz::compress_with(s, o).unwrap();
                assert_eq!(rz::decompress(&z).unwrap(), s, "{id} L{lvl} round-trip");
                let d = z.len() as i64 - a;
                tot[k] += d;
                print!("{:>10}", d);
            }
            println!();
        }
        rz::set_lazy_accel_arm(0);
        println!("{:<14}{:>10}{:>10}{:>10}{:>10}   <== TOTAL",
                 "", tot[0], tot[1], tot[2], tot[3]);
    }
}
