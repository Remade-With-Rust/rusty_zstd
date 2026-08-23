//! Timing arm for the simd.rs ten-cut batch (codec-measurement §15: the
//! counters are the primary evidence, this is the confirmatory clock).
//!
//! Compress-only — no hashing, no round-trip, no output — so the process CPU
//! time is the encoder. The corpora are read ONCE, up front, and every
//! compression runs from RAM: the first version of this example re-read each
//! file per level and measured `cpu/wall = 0.6`, i.e. 40% of both arms was file
//! I/O diluting the signal (codec-measurement §4).
//!
//! Prints a deterministic WORK COUNT to stderr so both arms can be proven to
//! have done identical work; a differing count voids the comparison.
//!
//! Drive it from a pinned, ABBA-interleaved harness that reads CPU time.

fn main() {
    let files = ["dickens", "samba", "webster"];
    // Read once, outside the repeated work.
    let corpora: Vec<Vec<u8>> = files
        .iter()
        .filter_map(|id| std::fs::read(format!("corpora/data/silesia/{id}")).ok())
        .map(|f| f[..f.len().min(3 << 20)].to_vec())
        .collect();

    let mut total_in = 0usize;
    let mut total_out = 0usize;
    for _rep in 0..2 {
        for lvl in [9i32, 19] {
            for s in &corpora {
                let c = rusty_zstd::compress_with(
                    s,
                    rusty_zstd::CompressOptions {
                        level: lvl,
                        checksum: false,
                    },
                )
                .expect("compress");
                total_in += s.len();
                total_out += c.len();
            }
        }
    }
    // The work count: identical across arms iff the arms did the same work.
    eprintln!("work in={total_in} out={total_out}");
}
