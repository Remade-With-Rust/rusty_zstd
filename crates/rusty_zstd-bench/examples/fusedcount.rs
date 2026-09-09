//! BRICK 11's deterministic verdict: how often does the fused head resolve the
//! match length from the xor it already had (SHORT: first differing byte inside
//! the first word, no second load pair) versus falling through to the counter
//! (LONG: all eight bytes equal)? Per accepted candidate the fused form is about
//! five instructions cheaper on SHORT and three dearer on LONG, so the mix
//! decides -- and the mix is a count, not a clock.
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example fusedcount
const IDS: &[&str] = &[
    "jsonlog-16m",
    "smallmsg-8m",
    "mr",
    "ooffice",
    "osdb",
    "reymont",
    "sao",
    "webster",
    "dickens",
    "mozilla",
    "nci",
    "samba",
    "xml",
    "x-ray",
    "text-32m",
    "incomp-32m",
];
fn main() {
    let cap = 1usize << 20;
    let srcs: Vec<Vec<u8>> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    f[..n].to_vec()
                })
        })
        .collect();
    println!(
        "{:>4}{:>14}{:>14}{:>9}{:>16}",
        "L", "short", "long", "short%", "net instrs"
    );
    for lvl in [3i32, 4, 5, 7, 9, 12] {
        let _ = rusty_zstd::take_fused();
        for s in &srcs {
            let _ = rusty_zstd::compress_with(
                s,
                rusty_zstd::CompressOptions {
                    level: lvl,
                    checksum: false,
                },
            )
            .unwrap();
        }
        let (short, long) = rusty_zstd::take_fused();
        let tot = (short + long).max(1);
        let net = short as i64 * -5 + long as i64 * 3;
        println!(
            "{:>4}{:>14}{:>14}{:>8.1}%{:>+16}",
            lvl,
            short,
            long,
            short as f64 / tot as f64 * 100.0,
            net
        );
    }
    println!("\nnet < 0 at a level = the fused head removes instructions there (modelled -5 per SHORT, +3 per LONG).");
}
