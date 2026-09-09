//! Broad numeric-arm sweep for SIZE. Exact bytes, no clock, no null band.
use rusty_zstd as rz;
const IDS: &[&str] = &["jsonlog-16m","smallmsg-8m","versions-16m","mr","ooffice","osdb",
    "reymont","sao","webster","dickens","mozilla","nci","samba","xml","x-ray","text-32m"];
fn main() {
    let cap: usize = 4 << 20;
    let srcs: Vec<(&str, Vec<u8>)> = IDS.iter().filter_map(|id| {
        std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
            .ok().map(|f| { let n = f.len().min(cap); (*id, f[..n].to_vec()) })
    }).collect();
    let go = |lvl: i32| -> usize { srcs.iter().map(|(_, s)|
        rz::compress_with(s, rz::CompressOptions { level: lvl, checksum: false })
            .unwrap().len()).sum() };
    println!("{:<20}{:>4}{:>10}{:>12}{:>10}", "arm = value", "L", "bytes", "delta", "pct");
    println!("{}", "-".repeat(58));
    macro_rules! sweep {
        ($name:expr, $lvl:expr, $set:expr, $reset:expr, $vals:expr) => {{
            $reset;
            let base = go($lvl);
            println!("{:<20}{:>4}{:>10}{:>12}{:>10}", format!("{} (default)", $name), $lvl, base, 0, "");
            for v in $vals {
                $set(v);
                let s = go($lvl);
                let d = s as i64 - base as i64;
                println!("{:<20}{:>4}{:>10}{:>+12}{:>9.3}%",
                         format!("  {} = {:?}", $name, v), $lvl, s, d,
                         d as f64 / base as f64 * 100.0);
            }
            $reset;
        }};
    }
    sweep!("accel_shift", 3, rz::set_accel_shift_arm, rz::set_accel_shift_arm(0),
           [2u32, 3, 4, 5, 6, 8]);
    sweep!("accel_shift", 1, rz::set_accel_shift_arm, rz::set_accel_shift_arm(0),
           [2u32, 3, 4, 5, 6, 8]);
    sweep!("dfast_step", 3, rz::set_dfast_step_arm, rz::set_dfast_step_arm(0), [1usize, 2, 3]);
    sweep!("search_log_d", 9, rz::set_search_log_delta, rz::set_search_log_delta(0),
           [-1i32, 1]);
    sweep!("pair_hi", 1, rz::set_pair_hi_arm, rz::set_pair_hi_arm(-1.0),
           [0.0f32, 0.5, 2.0, 4.0, 9.0]);
    sweep!("pair_gain", 1, rz::set_pair_gain_arm, rz::set_pair_gain_arm(-1.0),
           [0.0f32, 0.05, 0.1, 0.4, 1.0]);
}
