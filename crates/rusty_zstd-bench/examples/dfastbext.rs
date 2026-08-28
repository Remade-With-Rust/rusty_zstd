//! THE BACK-EXTENSION DFAST IS NOT DOING -- sizing the prize before building it.
//!
//! `emit_fast_seq_body` back-extends every Fast (L1) match: a backward walk from
//! the match start that converts literal bytes into match bytes. DFast -- the
//! finder the DEFAULT level runs -- has no such walk anywhere, and neither of
//! its two commit sites performs one. C's `ZSTD_compressBlock_doubleFast` does.
//!
//! `l13anat.rs` shows the consequence as a flat zero: `back_ext_bytes` is
//! 0.0069 per input byte at L1 and EXACTLY 0.0000 at L3.
//!
//! This measures what that costs, without changing a byte. The probe in
//! `find_dfast_impl_inner` mirrors `emit_fast_seq_body`'s walk exactly -- same
//! guards, same `back_eq` -- so `bytes` below is what applying it would recover.
//!
//! Every literal byte converted to a match byte is a byte that stops being
//! Huffman-coded and starts being covered by a match length, so this is a RATIO
//! lever, not a speed one. Sizes are deterministic, so the follow-up is a size
//! board, not a clock.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example dfastbext

const IDS: &[&str] = &[
    "x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao", "dickens", "samba", "nci",
    "webster", "mozilla", "mr",
];

fn main() {
    let cap: usize = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);
    let lvl: i32 = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(3);

    println!("DFAST BACK-EXTENSION PROBE @ L{lvl} ({} MiB cap)\n", cap >> 20);
    println!(
        "{:<13} {:>11} {:>11} {:>8} {:>11} {:>9} {:>9}",
        "corpus", "matches", "can_extend", "share", "bytes", "per_ext", "vs lits"
    );

    let (mut tb, mut tm, mut ts, mut tl) = (0u64, 0u64, 0u64, 0u64);
    for id in IDS {
        let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
        else {
            continue;
        };
        let s = &f[..f.len().min(cap)];
        rusty_zstd::prof_reset();
        let _ = rusty_zstd::take_dfast_bext();
        let _ = rusty_zstd::compress(s, lvl).expect("compress");
        let (bytes, can, seen) = rusty_zstd::take_dfast_bext();
        let c = rusty_zstd::prof_encode_counts();
        if seen == 0 {
            continue;
        }
        println!(
            "{:<13} {:>11} {:>11} {:>7.1}% {:>11} {:>9.2} {:>8.2}%",
            id,
            seen,
            can,
            100.0 * can as f64 / seen as f64,
            bytes,
            if can > 0 { bytes as f64 / can as f64 } else { 0.0 },
            // Recovered bytes as a share of the literals actually emitted --
            // this is the fraction of the literal stream that would stop being
            // Huffman-coded.
            if c.lit_bytes > 0 { 100.0 * bytes as f64 / c.lit_bytes as f64 } else { 0.0 }
        );
        tb += bytes;
        tm += can;
        ts += seen;
        tl += c.lit_bytes;
    }
    if ts > 0 {
        println!(
            "\nTOTAL  matches {ts}  can_extend {tm} ({:.1}%)  bytes {tb}  \
             per_ext {:.2}  = {:.2}% of all literals emitted",
            100.0 * tm as f64 / ts as f64,
            if tm > 0 { tb as f64 / tm as f64 } else { 0.0 },
            if tl > 0 { 100.0 * tb as f64 / tl as f64 } else { 0.0 }
        );
        println!(
            "\nEach recovered byte moves from the literal section into a match\n\
             length. Whether that PAYS depends on the literal's Huffman cost\n\
             against the matchlen code's -- so the verdict is a size board."
        );
    }
}
