//! Verify the row AUTO gate: fires in the measured band, silent outside,
//! round-trips everywhere.
use rusty_zstd as rz;
const IDS: &[&str] = &["dickens","mozilla","samba","webster","xml","x-ray","osdb",
    "reymont","nci","sao","mr","ooffice","jsonlog-16m","smallmsg-8m"];
fn main() {
    println!("{:>9}{:>5}{:>12}{:>12}{:>10}   {}",
             "cap", "L", "old(chain)", "new(auto)", "ratio", "gate");
    println!("{}", "-".repeat(64));
    for lvl in [5i32, 7, 9, 12, 13] {
        for cap in [256usize << 10, 512 << 10, 1 << 20, 2 << 20, 3 << 20, 8 << 20] {
            let (mut a, mut b) = (0u64, 0u64);
            for id in IDS {
                let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
                    .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
                let src = &f[..f.len().min(cap)];
                rz::set_row_arm(false);                       // old shipped default
                a += rz::compress(src, lvl).unwrap().len() as u64;
                rz::set_row_arm_auto();                       // new default
                let z = rz::compress(src, lvl).unwrap();
                assert_eq!(rz::decompress(&z).unwrap(), src, "round-trip L{lvl} cap{cap}");
                b += z.len() as u64;
            }
            let r = b as f64 / a as f64;
            println!("{:>8}K{:>5}{:>12}{:>12}{:>10.4}   {}", cap >> 10, lvl, a, b, r,
                     if (r - 1.0).abs() < 1e-9 { "off" } else { "ON" });
        }
    }
    rz::set_row_arm_auto();
}
