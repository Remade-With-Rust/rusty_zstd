//! Byte-identity gate for changes to `simd.rs`.
//!
//! `count_eq_len_ge8` feeds every match finder, so a defect in it moves match
//! lengths, which moves the sequence stream, which moves the compressed bytes.
//! This compresses every available corpus at one level per strategy family and
//! prints `level file bytes sha256`. A byte-identical change leaves this output
//! character-for-character equal.
//!
//! It also round-trips every stream, so a change that alters the bitstream in a
//! SELF-CONSISTENT way (still decodable, just different) is caught by the hash
//! rather than slipping through a round-trip-only check.
//!
//! Usage: capture before, apply the change, capture after, `diff` them.
//!   cargo run -p rusty_zstd-bench --release --example simdparity > after.txt

use sha2::{Digest, Sha256};

fn main() {
    let ids = [
        ("generated", "jsonlog-16m"),
        ("generated", "smallmsg-8m"),
        ("generated", "versions-16m"),
        ("generated", "text-32m"),
        ("generated", "incomp-32m"),
        ("generated", "zeros-1m"),
        ("silesia", "dickens"),
        ("silesia", "mozilla"),
        ("silesia", "mr"),
        ("silesia", "nci"),
        ("silesia", "ooffice"),
        ("silesia", "osdb"),
        ("silesia", "reymont"),
        ("silesia", "samba"),
        ("silesia", "sao"),
        ("silesia", "webster"),
        ("silesia", "xml"),
        ("silesia", "x-ray"),
    ];
    // One level per strategy family, so every finder that calls
    // `count_eq_len_ge8` is exercised: fast, dfast, greedy, lazy, lazy2,
    // btlazy2, btopt, btultra.
    let mut files = 0usize;
    for lvl in [1i32, 3, 5, 7, 9, 12, 16, 19] {
        for (dir, id) in ids {
            let path = format!("corpora/data/{dir}/{id}");
            let Ok(f) = std::fs::read(&path) else {
                continue;
            };
            // 4 MiB is enough to cross block boundaries and fill the window at
            // every level while keeping the whole sweep to a couple of minutes.
            let s = &f[..f.len().min(4 << 20)];
            let c = rusty_zstd::compress_with(
                s,
                rusty_zstd::CompressOptions {
                    level: lvl,
                    checksum: false,
                },
            )
            .expect("compress");
            let d = rusty_zstd::decompress(&c).expect("decompress");
            assert_eq!(d.as_slice(), s, "ROUND-TRIP FAILED at L{lvl} {id}");
            let mut h = Sha256::new();
            h.update(&c);
            println!("L{lvl:<2} {id:<14} {:>10} {:x}", c.len(), h.finalize());
            files += 1;
        }
    }
    eprintln!("simdparity: {files} (level, file) pairs, all round-tripped");
}
