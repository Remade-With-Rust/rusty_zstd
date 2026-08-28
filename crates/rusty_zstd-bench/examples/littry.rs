//! LITERAL-SECTION CANDIDATE CENSUS -- how many Huffman encodes per block are
//! thrown away.
//!
//! `write_literals_inner` builds up to two candidate literal sections per block
//! (previous table, type 3; new table, type 2), fully Huffman-ENCODES each, then
//! keeps whichever packs smaller. The encode is the expensive half: a 4-stream
//! encode runs `encode_stream_unrolled_*_into` four times, and that symbol is
//! ~1,830 instructions carrying 118 BMI2 shift ops.
//!
//! The interesting part is that `body_bytes_exact` ALREADY computes each
//! candidate's exact body size without encoding it -- the `futile` predicate
//! calls it for BOTH tables. So when both return `Some`, the winner is known
//! before either encode runs, and the losing encode is provably wasted work.
//!
//! This counts, per level, how many encodes happen and how many are discarded.
//! A count, not a clock -- same number on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example littry

const IDS: &[&str] = &[
    "zeros-32m", "text-32m", "incomp-32m", "jsonlog-16m", "smallmsg-8m", "versions-16m", "mr",
    "ooffice", "osdb", "reymont", "sao", "webster", "dickens", "mozilla", "nci", "samba", "xml",
    "x-ray",
];
const LEVELS: &[i32] = &[1, 3, 5, 9, 19];

/// Pull `lit_try blocks=.. prev_ENCODED=.. ..` out of the profile dump.
fn lit_try() -> [u64; 7] {
    let d = rusty_zstd::prof_dump();
    let mut out = [0u64; 7];
    for line in d.lines() {
        if let Some(rest) = line.strip_prefix("lit_try ") {
            for (i, key) in [
                "blocks=",
                "prev_ENCODED=",
                "prev_won=",
                "new_ENCODED=",
                "new_won=",
                "raw_won=",
                "SKIPPED=",
            ]
            .iter()
            .enumerate()
            {
                if let Some(p) = rest.find(key) {
                    let tail = &rest[p + key.len()..];
                    let end = tail.find(' ').unwrap_or(tail.len());
                    out[i] = tail[..end].trim().parse().unwrap_or(0);
                }
            }
        }
    }
    out
}

fn main() {
    let cap: usize = std::env::var("BG_CAP")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1 << 20);
    let srcs: Vec<(&str, Vec<u8>)> = IDS
        .iter()
        .filter_map(|id| {
            std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}")))
                .ok()
                .map(|f| {
                    let n = f.len().min(cap);
                    (*id, f[..n].to_vec())
                })
        })
        .collect();

    println!("LITERAL-SECTION CANDIDATE CENSUS  cap={cap} corpora={}\n", srcs.len());
    println!(
        "{:>3}  {:>9} {:>10} {:>10} {:>9} {:>9} {:>8}  {:>9}",
        "L", "blocks", "encodes", "prev_ENC", "new_ENC", "skipped", "raw_won", "WASTED"
    );

    for &l in LEVELS {
        rusty_zstd::prof_reset();
        for (_, s) in &srcs {
            let _ = rusty_zstd::compress(s, l).expect("compress");
        }
        let c = lit_try();
        let (blocks, prev_enc, prev_won, new_enc, new_won, raw_won, skipped) =
            (c[0], c[1], c[2], c[3], c[4], c[5], c[6]);
        let encodes = prev_enc + new_enc;
        // An encode is WASTED when its section loses. Exactly one candidate can
        // win per block, so every encode beyond the winner is discarded work --
        // and when raw wins, both encodes were discarded.
        let won = prev_won + new_won;
        let wasted = encodes.saturating_sub(won);
        println!(
            "{:>3}  {:>9} {:>10} {:>10} {:>9} {:>9} {:>8}  {:>9} ({:.1}% of encodes)",
            l,
            blocks,
            encodes,
            prev_enc,
            new_enc,
            skipped,
            raw_won,
            wasted,
            if encodes > 0 { 100.0 * wasted as f64 / encodes as f64 } else { 0.0 }
        );
    }

    println!(
        "\nWASTED = encodes whose section lost. `body_bytes_exact` already knows\n\
         each candidate's exact body size before either encode runs."
    );
}
