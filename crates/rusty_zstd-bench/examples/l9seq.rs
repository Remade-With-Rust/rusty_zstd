//! L9 DECODE: the lever is SEQUENCE COUNT, not the loop.
//!
//! `dsanat` puts the DecSeq loop at ~81% of L9 decode (87.1% of decode is
//! DecSeq, 92.9% of DecSeq is the loop), and that loop already carries ~50
//! recorded cuts (WIN 2/3/9, CUT 4, T4, W5, D9, and sections 17-27 of
//! inline-execution.md). Its per-trip cost is not where a win is left.
//!
//! The TRIP COUNT is. `l9dec.rs` measures L9 emitting 61,108 sequences per MiB
//! against L1's 43,128 -- 42% more trips through the same loop, which is why
//! L9 decodes SLOWER per MiB than L1 (3.12 vs 2.50 ms/MiB) despite compressing
//! 18% better. The decode gap has always tracked sequence count rather than
//! bytes; this is that finding applied.
//!
//! `target_length` is the knob that trades sequence count for ratio: a higher
//! bar makes the finder skip marginal short matches, which become literals and
//! collapse into the neighbouring sequence. Both columns here are
//! DETERMINISTIC -- the sequence count is a census and the size is the
//! bitstream -- so this board reads the same on any machine at any load.
//!
//! usage: cargo run --release -p rusty_zstd-bench --features profile --example l9seq [level]

const IDS: &[&str] = &[
    "dickens", "webster", "samba", "mozilla", "osdb", "mr", "nci", "xml", "ooffice", "sao",
];

fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(9);
    let cap: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8 << 20);

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
    let total: u64 = srcs.iter().map(|(_, s)| s.len() as u64).sum();
    let base = rusty_zstd::compression_params(lvl, Some(cap as u64)).expect("params");

    println!(
        "L{lvl} SEQUENCE-COUNT BOARD -- {} corpora, {:.0} MiB\nshipping target_length = {}\n",
        srcs.len(),
        total as f64 / (1 << 20) as f64,
        base.target_length
    );
    println!(
        "{:>10} {:>13} {:>11} {:>10} {:>13} {:>10}",
        "target_len", "sequences", "seq/MiB", "vs base", "bytes", "size vs"
    );

    let (mut b_seq, mut b_bytes) = (0u64, 0u64);
    for (i, tl) in [base.target_length, 16, 32, 64, 128].iter().enumerate() {
        let mut p = base;
        p.target_length = *tl;
        let (mut nseq, mut bytes) = (0u64, 0u64);
        for (id, s) in &srcs {
            let z = rusty_zstd::compress_with_params(s, p, false).expect("compress");
            bytes += z.len() as u64;
            let _ = rusty_zstd::take_dec_bands();
            let out = rusty_zstd::decompress(&z).expect("decompress");
            assert_eq!(&out[..], &s[..], "{id} round-trip");
            let (bands, _) = rusty_zstd::take_dec_bands();
            nseq += bands.iter().sum::<u64>();
        }
        if i == 0 {
            b_seq = nseq;
            b_bytes = bytes;
        }
        println!(
            "{:>10} {:>13} {:>11.0} {:>9.2}x {:>13} {:>9.3}%",
            if i == 0 { format!("{tl} (base)") } else { tl.to_string() },
            nseq,
            nseq as f64 / (total as f64 / (1 << 20) as f64),
            if b_seq > 0 { nseq as f64 / b_seq as f64 } else { 0.0 },
            bytes,
            if b_bytes > 0 {
                100.0 * (bytes as f64 - b_bytes as f64) / b_bytes as f64
            } else {
                0.0
            }
        );
    }
    println!(
        "\nEvery sequence removed is one fewer trip through the DecSeq loop --\n\
         ~81% of L9 decode. A row that cuts trips hard for little size is a\n\
         DECODE win bought on the encode side, which is where the lever is."
    );
}
