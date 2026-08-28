//! SECTION 14.9: what does the 16-to-1 bucket sharing actually cost?
//!
//! `row_of(h) = (h >> 4) & mask` folds SIXTEEN hash buckets into one 16-slot
//! row, so a tag match does not imply a bucket match. Every cross-bucket
//! candidate that survives the tag filter still costs an `mls_eq` -- a RANDOM
//! load into `src` at the candidate's position -- and for `mls >= 4` it cannot
//! possibly succeed, because a different bucket means different low-4 bytes
//! and those are exactly what `mls_eq` compares first.
//!
//! Also counts `gtag == 0` probes: the wide-hash path (`mls >= 8`) stores tag
//! 0 for every entry, which makes the tag filter match EVERYTHING.
//! Requires --features profile, row arm ON.
const IDS: &[&str] = &["dickens", "webster", "samba", "xml", "nci", "reymont", "osdb", "mozilla"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    rusty_zstd::set_row_arm(true);
    let _ = rusty_zstd::take_row_bucket();
    println!("ROW BUCKET CENSUS @ L{lvl}\n");
    println!(
        "{:<10}{:>11}{:>12}{:>12}{:>9}{:>11}{:>9}",
        "corpus", "probes", "examined", "same-bkt", "same%", "mls_eq ok", "gtag0%"
    );
    let mut t = [0u64; 5];
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(8 << 20)];
        let _ = rusty_zstd::take_row_bucket();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id}: round-trip");
        let c = rusty_zstd::take_row_bucket();
        if c[0] == 0 {
            continue;
        }
        println!(
            "{:<10}{:>11}{:>12}{:>12}{:>8.1}%{:>11}{:>8.1}%",
            id,
            c[4],
            c[0],
            c[1],
            100.0 * c[1] as f64 / c[0] as f64,
            c[2],
            100.0 * c[3] as f64 / c[4].max(1) as f64
        );
        for (i, v) in c.iter().enumerate() {
            t[i] += v;
        }
    }
    rusty_zstd::set_row_arm(false);
    println!(
        "\n**TOTAL: {} probes, {} candidates examined**",
        t[4], t[0]
    );
    println!(
        "  same bucket : {:>12}  ({:.1}%)",
        t[1],
        100.0 * t[1] as f64 / t[0] as f64
    );
    println!(
        "  CROSS bucket: {:>12}  ({:.1}%)  <- each costs a random src load in mls_eq",
        t[0] - t[1],
        100.0 * (t[0] - t[1]) as f64 / t[0] as f64
    );
    println!(
        "  mls_eq pass : {:>12}  ({:.1}% of examined)",
        t[2],
        100.0 * t[2] as f64 / t[0] as f64
    );
    println!("  gtag==0     : {:>12} probes ({:.1}%)", t[3], 100.0 * t[3] as f64 / t[4].max(1) as f64);
    if t[1] >= t[2] {
        println!(
            "\nCross-bucket candidates that PASSED mls_eq: {} (if 0, rejecting them is byte-identical)",
            t[2].saturating_sub(t[1])
        );
    }
}
