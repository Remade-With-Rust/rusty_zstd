//! L3 RATIO GAP: is the TAG FILTER dropping matches C would find?
//!
//! Section 21 measured, on identical input at L3, that C emits **4.5% MORE
//! sequences and 4.6% SMALLER output** than we do. More matches AND smaller
//! means we are MISSING matches, not mis-coding them.
//!
//! One structural difference is a prime suspect: our DFast runs a TAG FILTER
//! on both hash tables that C's `ZSTD_compressBlock_doubleFast` does not have
//! at all. A tag is a lossy hash of the candidate's bytes; when it mismatches
//! we skip the candidate WITHOUT loading it. That is a pure win when the
//! candidate would have failed anyway -- and a lost match when it would not.
//!
//! The counters for exactly this already exist (`TAG_FALSE_REJECT` is a
//! rejection the byte compare would have ACCEPTED). Nobody had read them at
//! L3. Requires --features profile.
const IDS: &[&str] = &["dickens", "webster", "samba", "xml", "nci", "reymont", "osdb", "mozilla"];
fn load(id: &str) -> Option<Vec<u8>> {
    std::fs::read(format!("corpora/data/silesia/{id}"))
        .or_else(|_| std::fs::read(format!("corpora/data/generated/{id}")))
        .ok()
}
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(3);
    println!("DFAST TAG-FILTER AUDIT @ L{lvl}\n");
    println!(
        "{:<10}{:>14}{:>14}{:>10}{:>16}{:>12}",
        "corpus", "short reject", "FALSE reject", "false%", "long reject", "LONG FALSE"
    );
    let (mut tr, mut tf, mut lr, mut lf, mut ln) = (0u64, 0u64, 0u64, 0u64, 0u64);
    for id in IDS {
        let Some(f) = load(id) else { continue };
        let src = &f[..f.len().min(8 << 20)];
        let _ = rusty_zstd::take_tag_rejects();
        let _ = rusty_zstd::take_ltag_audit();
        let z = rusty_zstd::compress(src, lvl).unwrap();
        assert_eq!(rusty_zstd::decompress(&z).unwrap(), src, "{id}: round-trip");
        let (fr, rt) = rusty_zstd::take_tag_rejects();
        let [lne, lrj, lfa] = rusty_zstd::take_ltag_audit();
        println!(
            "{:<10}{:>14}{:>14}{:>9.3}%{:>16}{:>12}",
            id,
            rt,
            fr,
            100.0 * fr as f64 / rt.max(1) as f64,
            lrj,
            lfa
        );
        tr += rt;
        tf += fr;
        lr += lrj;
        lf += lfa;
        ln += lne;
    }
    println!(
        "\n**SHORT table: {} rejections, {} FALSE ({:.3}%)**",
        tr,
        tf,
        100.0 * tf as f64 / tr.max(1) as f64
    );
    println!(
        "**LONG  table: {} nonempty, {} rejections, {} FALSE ({:.3}% of rejections)**",
        ln,
        lr,
        lf,
        100.0 * lf as f64 / lr.max(1) as f64
    );
    println!("\nA FALSE reject is a candidate the byte compare would have ACCEPTED --");
    println!("a match C's untagged doubleFast finds and we do not.");
}
