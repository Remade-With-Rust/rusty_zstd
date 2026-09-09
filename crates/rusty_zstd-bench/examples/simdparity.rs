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

/// One level per strategy family. Asserted against the resolved strategies
/// in `main`, so the claim cannot drift away from the list again.
const LEVELS: &[i32] = &[1, 3, 5, 7, 9, 13, 16, 18, 19];

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
    //
    // FIXED: the list said `12` where it meant BtLazy2, but L12 resolves to
    // **Lazy2** -- BtLazy2 starts at L13. So this gate's own doc claimed a
    // strategy it did not exercise, and `find_bt_lazy` -- which calls
    // `count_match` like every other finder -- was never parity-checked. L9
    // already covers Lazy2, so 12 is replaced rather than added.
    //
    // The assertion below makes the doc comment enforceable: if a future
    // level-table edit moves a boundary, this fails loudly instead of
    // silently dropping a finder out of the gate.
    let mut files = 0usize;
    {
        let mut seen: Vec<String> = LEVELS
            .iter()
            .filter_map(|&l| rusty_zstd::compression_params(l, None).ok())
            .map(|p| format!("{:?}", p.strategy))
            .collect();
        seen.sort();
        seen.dedup();
        const WANT: &[&str] = &["Fast", "DFast", "Greedy", "Lazy", "Lazy2",
                                "BtLazy2", "BtOpt", "BtUltra", "BtUltra2"];
        let missing: Vec<&str> = WANT
            .iter()
            .copied()
            .filter(|w| !seen.iter().any(|x| x == w))
            .collect();
        assert!(
            missing.is_empty(),
            "simdparity LEVELS no longer cover every finder: missing {missing:?} \
             (covered: {seen:?}). A simd defect in a missing finder would not \
             move this gate's output."
        );
        eprintln!("simdparity strategies covered: {}", seen.join(", "));
    }
    for &lvl in LEVELS {
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
