//! Corrected numeric sweep. Two harness defects fixed from the first attempt:
//!
//!  1. `accel_shift_for` is consulted ONLY under `cfg!(feature = "profile")`
//!     -- release builds take the constant 7/8. Swept without that feature the
//!     arm is inert and every cell reads +0, which looks like a dead knob and
//!     is really a dead harness. Run this with --features profile.
//!  2. The f32 arms cache RAW BITS with `u32::MAX` as "unset", and the public
//!     setter stores `v.to_bits()`. So `set_pair_hi_arm(-1.0)` does not RESET
//!     the arm, it PINS it to -1.0 -- and every delta taken against that
//!     baseline is measured from a changed config. The first run read
//!     `pair_hi=4.0` as -47,965 B; against the true default (1.0) the same
//!     cell is -7,702. Baselines here are the documented defaults, set
//!     explicitly.
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
    println!("profile feature: {}", cfg!(feature = "profile"));
    println!("{:<22}{:>4}{:>11}{:>12}{:>10}", "arm = value", "L", "bytes", "delta", "pct");
    println!("{}", "-".repeat(60));

    // accel_shift: default is 7 at Fast, 8 at DFast. 0 = "not pinned".
    for (lvl, deflt) in [(1i32, 7u32), (3, 8)] {
        rz::set_accel_shift_arm(0);
        let base = go(lvl);
        println!("{:<22}{:>4}{:>11}{:>12}", format!("accel_shift ({deflt})"), lvl, base, 0);
        for v in [3u32, 4, 5, 6, 7, 8, 9, 10, 12] {
            rz::set_accel_shift_arm(v);
            let s = go(lvl);
            println!("{:<22}{:>4}{:>11}{:>+12}{:>9.3}%", format!("  accel_shift = {v}"),
                     lvl, s, s as i64 - base as i64,
                     (s as i64 - base as i64) as f64 / base as f64 * 100.0);
        }
        rz::set_accel_shift_arm(0);
    }
    // pair_hi: documented default 1.0. Baseline set EXPLICITLY, not by a sentinel.
    rz::set_pair_hi_arm(1.0);
    let base = go(1);
    println!("{:<22}{:>4}{:>11}{:>12}", "pair_hi (1.0)", 1, base, 0);
    for v in [0.0f32, 0.5, 1.5, 2.0, 3.0, 4.0, 6.0, 9.0] {
        rz::set_pair_hi_arm(v);
        let s = go(1);
        println!("{:<22}{:>4}{:>11}{:>+12}{:>9.3}%", format!("  pair_hi = {v}"),
                 1, s, s as i64 - base as i64,
                 (s as i64 - base as i64) as f64 / base as f64 * 100.0);
    }
    rz::set_pair_hi_arm(1.0);
}
