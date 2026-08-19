//! Sweep the drift threshold: does samba's 4 MiB regression clear before the
//! wins (sao via the ratio branch, mozilla/xml via drift) disappear?
const IDS: &[&str] = &["sao","mozilla","xml","x-ray","samba","mr","osdb","webster","dickens","nci","ooffice","reymont"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    for d in [0.50f32, 0.70, 1.00, 1.50] {
        let (mut tot, mut base, mut worst, mut wid, mut wins) = (0i64, 0i64, f64::MIN, "", 0);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            for cap in [1usize<<20, 2<<20, 4<<20, 8<<20] {
                if f.len() < cap { continue }
                let s = &f[..cap];
                rusty_zstd::set_g5_arms(-1.0, 2.0, 2.0);
                let off = rusty_zstd::compress(s, lvl).unwrap().len();
                rusty_zstd::set_g5_arms(0.30, 0.70, d);
                let on = rusty_zstd::compress(s, lvl).unwrap().len();
                let pc = (on as f64/off as f64 - 1.0)*100.0;
                if pc > worst { worst = pc; wid = id }
                if on < off { wins += 1 }
                tot += on as i64; base += off as i64;
            }
        }
        println!("drift>={d:<5} total {:+.4}%   worst {:+.3}% ({wid})   cells improved {wins}",
            (tot as f64/base as f64-1.0)*100.0, worst);
    }
}
