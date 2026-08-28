//! SIZE BOARD for DFast back-extension. Bitstream-changing, so identity cannot
//! be the gate -- SIZE is.
//!
//! `dfastbext.rs` sized the prize: 9.6% of DFast matches could extend backward,
//! recovering 2.65% of all emitted literals (21.0% on dickens, 16.1% on
//! webster). Every recovered byte moves out of the Huffman-coded literal
//! section and into a match length, so whether it PAYS is a size question, not
//! a count question.
//!
//! Both arms run in-process over identical input, and every frame is
//! round-tripped, so this is a correctness gate as well. Sizes are
//! deterministic: this board reads the same on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --example bextboard [level]

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    let cap: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);

    println!("DFAST BACK-EXTENSION SIZE BOARD @ L{lvl} ({} MiB cap)\n", cap >> 20);
    println!(
        "{:<14} {:>12} {:>12} {:>10} {:>9}",
        "corpus", "off", "on", "delta", "pct"
    );

    let (mut toff, mut ton) = (0u64, 0u64);
    let (mut wins, mut losses, mut ties) = (0u32, 0u32, 0u32);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        let s = &f[..f.len().min(cap)];

        rusty_zstd::set_dfast_bext_arm(false);
        let a = rusty_zstd::compress(s, lvl).expect("compress off");
        assert_eq!(&rusty_zstd::decompress(&a).expect("rt off")[..], &s[..], "{id} off");

        rusty_zstd::set_dfast_bext_arm(true);
        let b = rusty_zstd::compress(s, lvl).expect("compress on");
        assert_eq!(&rusty_zstd::decompress(&b).expect("rt on")[..], &s[..], "{id} on");

        let d = b.len() as i64 - a.len() as i64;
        match d.cmp(&0) {
            std::cmp::Ordering::Less => wins += 1,
            std::cmp::Ordering::Greater => losses += 1,
            std::cmp::Ordering::Equal => ties += 1,
        }
        println!(
            "{:<14} {:>12} {:>12} {:>+10} {:>8.3}%",
            id,
            a.len(),
            b.len(),
            d,
            100.0 * d as f64 / a.len() as f64
        );
        toff += a.len() as u64;
        ton += b.len() as u64;
    }
    rusty_zstd::set_dfast_bext_arm(false);

    let d = ton as i64 - toff as i64;
    println!(
        "\nTOTAL {:>12} {:>12} {:>+10} {:>8.4}%",
        toff, ton, d, 100.0 * d as f64 / toff as f64
    );
    println!("  smaller on {wins}, larger on {losses}, unchanged on {ties}");
    println!(
        "\nNegative delta = we emit FEWER bytes with back-extension on.\n\
         Every frame above was round-tripped, so this is a correctness gate too."
    );
}
