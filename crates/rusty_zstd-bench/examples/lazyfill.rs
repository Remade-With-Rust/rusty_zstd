//! How much work is the LAZY back-fill, against the finder's own probes?
//!
//! `find_lazy_impl` re-inserts every position a match covered. Section 14.8
//! proved the chain, chain-tags, hash-head and head-tags are all DEAD while
//! the row arm is on -- but it only removed them from the FINDER's insert.
//! The fill still calls the full `lz_insert`. This prices the fill against
//! the probe count so the gap is a number before anything is cut.
//! Requires --features profile.
const IDS: &[&str] = &["dickens", "webster", "samba", "xml", "nci", "reymont", "osdb", "mozilla"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let rows_on = std::env::args().nth(2).map(|v| v != "0").unwrap_or(true);
    rusty_zstd::set_row_arm(rows_on);
    println!("LAZY FILL vs PROBE @ L{lvl} (row arm {})\n", if rows_on { "ON" } else { "off" });
    println!("{:<10}{:>12}{:>12}{:>11}{:>13}{:>11}", "corpus", "probes", "sites", "nonempty", "inserts", "ins/nonempty");
    let (mut tp, mut tf, mut ti, mut tn) = (0u64, 0u64, 0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(8 << 20)];
        let _ = rusty_zstd::take_lazy_fill();
        let _ = rusty_zstd::take_row_walk();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id}: round-trip");
        let (fills, ne, ins) = rusty_zstd::take_lazy_fill();
        let probes = rusty_zstd::take_row_walk()[0];
        println!(
            "{:<10}{:>12}{:>12}{:>11}{:>13}{:>11.2}",
            id, probes, fills, ne, ins, ins as f64 / ne.max(1) as f64
        );
        tn += ne;
        tp += probes;
        tf += fills;
        ti += ins;
    }
    rusty_zstd::set_row_arm(false);
    println!(
        "\n**TOTAL: {} probes, {} fill sites, {} FILL INSERTS = {:.2}x the probe count**",
        tp, tf, ti, ti as f64 / tp.max(1) as f64
    );
    println!(
        "  EMPTY fill sites: {} of {} ({:.1}%) -- setup paid, zero positions inserted",
        tf - tn, tf, 100.0 * (tf - tn) as f64 / tf.max(1) as f64
    );
    println!("  inserts per NON-EMPTY site: {:.2}", ti as f64 / tn.max(1) as f64);
}
