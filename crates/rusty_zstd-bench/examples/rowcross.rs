//! Where does the row finder stop paying, and does the load saving survive
//! there? Round-trips every cell.
use rusty_zstd as rz;
const IDS: &[&str] = &["dickens","mozilla","samba","webster","xml","x-ray","osdb",
    "reymont","nci","sao","mr","ooffice","jsonlog-16m","smallmsg-8m"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    println!("L{lvl}  (round-trip asserted on every row cell)\n");
    println!("{:>9}{:>12}{:>12}{:>10}{:>14}{:>14}{:>9}",
             "cap", "chain B", "row B", "ratio", "chain loads", "row loads", "saved");
    println!("{}", "-".repeat(80));
    for cap in [256usize << 10, 512 << 10, 1 << 20, 1536 << 10, 2 << 20, 3 << 20, 4 << 20, 6 << 20] {
        let (mut a, mut b, mut cl, mut rl) = (0u64, 0u64, 0u64, 0u64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/silesia/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}"))) else { continue };
            let src = &f[..f.len().min(cap)];
            rz::set_row_arm(false);
            let _ = rz::take_walk_census();
            let x = rz::compress(src, lvl).unwrap();
            cl += rz::take_walk_census().0;
            rz::set_row_arm(true);
            let _ = rz::take_row_census();
            let y = rz::compress(src, lvl).unwrap();
            // .0 is ROW_EXAM (candidates), .1 is ROW_LOADS. Using .0 here
            // compares chain LOADS against row CANDIDATES and reads ~1.00x,
            // which is not a result -- it is two different units.
            let (_cands, loads) = rz::take_row_census();
            rl += loads;
            assert_eq!(rz::decompress(&y).unwrap(), src, "{id} @ {cap} round-trip");
            rz::set_row_arm(false);
            a += x.len() as u64; b += y.len() as u64;
        }
        println!("{:>8}K{:>12}{:>12}{:>10.4}{:>14}{:>14}{:>8.2}x",
                 cap >> 10, a, b, b as f64 / a as f64, cl, rl,
                 if rl == 0 { 0.0 } else { cl as f64 / rl as f64 });
    }
}
