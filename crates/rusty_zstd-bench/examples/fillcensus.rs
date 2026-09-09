//! Fill / walk census for the link-representation and tag levers.
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example fillcensus
//!
//! Per level, over the silesia sample: walks by exit reason (index 0 = the
//! head was EMPTY, no walk), candidates examined, first-word misses, tag
//! skips (and false skips, which must be 0), fill inserts, and the phantom
//! position-0 census. The empty-head count is what "heads hold the decoded
//! link" would turn into a one-iteration walk; the skip/exam pair prices a
//! weaker tag.
#[cfg(feature = "profile")]
const IDS: &[&str] = &["dickens", "mozilla", "webster", "xml", "samba"];
fn main() {
    #[cfg(not(feature = "profile"))]
    {
        println!("needs --features rusty_zstd/profile");
    }
    #[cfg(feature = "profile")]
    {
        let levels: Vec<i32> = std::env::args()
            .skip(1)
            .filter_map(|a| a.parse().ok())
            .collect();
        let levels = if levels.is_empty() {
            vec![5, 7, 9, 12]
        } else {
            levels
        };
        println!(
            "{:>3} {:>11} {:>11} {:>7} {:>11} {:>11} {:>11} {:>9} {:>11} {:>9} {:>6}",
            "L", "walks", "empty", "empty%", "exam", "bytemiss", "skips", "false", "inserts", "m0", "acc"
        );
        for lvl in levels {
            let mut walks = 0u64;
            let mut empty = 0u64;
            let (mut exam, mut miss, mut skips, mut falses, mut ins, mut m0, mut acc) =
                (0u64, 0u64, 0u64, 0u64, 0u64, 0u64, 0u64);
            for id in IDS {
                let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
                    continue;
                };
                let src = &full[..full.len().min(16 << 20)];
                let _ = rusty_zstd::take_walk_exit();
                let _ = rusty_zstd::take_walk_census();
                let _ = rusty_zstd::take_link_tag();
                let _ = rusty_zstd::take_lazy_fill();
                let _ = rusty_zstd::take_walk_phantom();
                let _ = rusty_zstd::compress(src, lvl).unwrap();
                let ex = rusty_zstd::take_walk_exit();
                walks += ex.iter().sum::<u64>();
                empty += ex[0];
                let (e, b) = rusty_zstd::take_walk_census();
                exam += e;
                miss += b;
                let (s, f) = rusty_zstd::take_link_tag();
                skips += s;
                falses += f;
                let (_, _, i) = rusty_zstd::take_lazy_fill();
                ins += i;
                let (a, c) = rusty_zstd::take_walk_phantom();
                m0 += a;
                acc += c;
            }
            println!(
                "{:>3} {:>11} {:>11} {:>6.2}% {:>11} {:>11} {:>11} {:>9} {:>11} {:>9} {:>6}",
                lvl,
                walks,
                empty,
                100.0 * empty as f64 / walks.max(1) as f64,
                exam,
                miss,
                skips,
                falses,
                ins,
                m0,
                acc
            );
        }
    }
}
