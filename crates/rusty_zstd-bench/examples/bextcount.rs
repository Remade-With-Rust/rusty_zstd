//! Backward-extension census: is a word-at-a-time backward extension worth
//! building? The byte loop costs ~8 instructions per extended byte; a word
//! form costs ~10 fixed per match. So it pays iff extended bytes per match is
//! comfortably above one -- a count, not a clock.
//!
//!   cargo run --release --features profile -p rusty_zstd-bench --example bextcount
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
    for lvl in [1i32, 3, 7, 9] {
        let _ = rusty_zstd::take_bext();
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
        println!(
            "L{lvl}: take_bext() = {:?}   (matches, extended>0, bytes, >=8 -- see note_bext)",
            rusty_zstd::take_bext()
        );
    }
}
