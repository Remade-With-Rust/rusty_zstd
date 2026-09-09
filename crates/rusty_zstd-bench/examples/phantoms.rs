//! BRICK 46 census: the chain walk's PHANTOM position-0 candidate.
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example phantoms
//!
//! A chain link of 0 is both "no link" and position 0, so a walk whose chain
//! ends inside the first window continues to m = 0 and examines it. Per level:
//! how many candidates were examined at position 0, how many were ACCEPTED
//! (the byte-identity question for an unambiguous null), against all
//! candidates examined.
#[cfg(feature = "profile")]
const IDS: &[&str] = &["dickens", "mozilla", "webster", "xml", "samba"];
fn main() {
    #[cfg(not(feature = "profile"))]
    {
        println!("needs --features rusty_zstd/profile");
    }
    #[cfg(feature = "profile")]
    {
        println!(
            "{:>3} {:>12} {:>12} {:>10} {:>8}",
            "L", "examined", "at pos 0", "accepted", "share"
        );
        for lvl in [5i32, 7, 9, 12] {
            let (mut exam, mut m0, mut acc) = (0u64, 0u64, 0u64);
            for id in IDS {
                let Ok(full) = std::fs::read(format!("corpora/data/silesia/{id}")) else {
                    continue;
                };
                let src = &full[..full.len().min(16 << 20)];
                let _ = rusty_zstd::take_walk_census();
                let _ = rusty_zstd::take_walk_phantom();
                let _ = rusty_zstd::compress(src, lvl).unwrap();
                let (e, _) = rusty_zstd::take_walk_census();
                let (a, b) = rusty_zstd::take_walk_phantom();
                exam += e;
                m0 += a;
                acc += b;
            }
            println!(
                "{:>3} {:>12} {:>12} {:>10} {:>7.2}%",
                lvl,
                exam,
                m0,
                acc,
                100.0 * m0 as f64 / exam.max(1) as f64
            );
        }
    }
}
