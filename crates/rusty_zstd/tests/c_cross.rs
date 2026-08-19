//! C libzstd v1.5.7 compress -> rusty_zstd decompress, bit-exact.
//!
//! Skips when the pinned oracle is not on disk (CI without fetch-oracle).
//! The holdout file is `incomp-32m` (split=holdout). Train files are extra coverage.

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .expect("crates/rusty_zstd -> repo root")
        .to_path_buf()
}

fn find_oracle() -> Option<PathBuf> {
    if let Ok(p) = std::env::var("RUSTY_ZSTD_ORACLE") {
        let pb = PathBuf::from(p);
        if pb.is_file() {
            return Some(pb);
        }
    }
    let extracted = repo_root().join("third_party/zstd/extracted");
    walk(&extracted).ok()?.into_iter().find(|p| {
        p.file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.eq_ignore_ascii_case("zstd.exe") || n.eq_ignore_ascii_case("zstd"))
    })
}

fn walk(dir: &Path) -> std::io::Result<Vec<PathBuf>> {
    let mut out = Vec::new();
    if !dir.is_dir() {
        return Ok(out);
    }
    for ent in fs::read_dir(dir)? {
        let p = ent?.path();
        if p.is_dir() {
            out.extend(walk(&p)?);
        } else {
            out.push(p);
        }
    }
    Ok(out)
}

fn c_compress(oracle: &Path, src: &[u8], level: i32) -> Vec<u8> {
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let tmp = std::env::temp_dir().join(format!("rzstd-m1-{}-{}-{}", level, src.len(), nonce));
    let inn = tmp.with_extension("in");
    let out = tmp.with_extension("zst");
    fs::write(&inn, src).unwrap();
    let status = Command::new(oracle)
        .args([
            "-f",
            "-q",
            &format!("-{level}"),
            "-o",
            out.to_str().unwrap(),
            inn.to_str().unwrap(),
        ])
        .status()
        .expect("spawn zstd");
    assert!(status.success(), "zstd -{level} failed");
    let zst = fs::read(&out).unwrap();
    let _ = fs::remove_file(&inn);
    let _ = fs::remove_file(&out);
    zst
}

fn xorshift_bytes(seed: u64, n: usize) -> Vec<u8> {
    let mut s = seed;
    let mut v = vec![0u8; n];
    for b in &mut v {
        s ^= s << 13;
        s ^= s >> 7;
        s ^= s << 17;
        *b = (s & 0xFF) as u8;
        if *b == 0 {
            *b = 1;
        }
    }
    v
}

fn assert_roundtrip(oracle: &Path, src: &[u8], level: i32) {
    let zst = c_compress(oracle, src, level);
    let got = rusty_zstd::decompress(&zst).unwrap_or_else(|e| {
        panic!(
            "decompress L{level} {} bytes: {e:?} zst={}",
            src.len(),
            zst.len()
        )
    });
    if got.len() != src.len() {
        panic!(
            "len mismatch L{level}: got {} want {}",
            got.len(),
            src.len()
        );
    }
    if got != src {
        let pos = got
            .iter()
            .zip(src.iter())
            .position(|(a, b)| a != b)
            .unwrap_or(got.len());
        panic!(
            "mismatch L{level} at byte {pos}/{} got={:02x} want={:02x}",
            src.len(),
            got.get(pos).copied().unwrap_or(0),
            src.get(pos).copied().unwrap_or(0)
        );
    }
}

#[test]
fn c_small_corpus() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::new();
    while text.len() < 4096 {
        text.extend_from_slice(fox);
    }
    for level in [1, 3, 19] {
        assert_roundtrip(&oracle, b"", level);
        assert_roundtrip(&oracle, b"a", level);
        assert_roundtrip(&oracle, b"hello", level);
        assert_roundtrip(&oracle, &[0u8; 16], level);
        assert_roundtrip(&oracle, &vec![0u8; 256], level);
        assert_roundtrip(&oracle, &vec![0u8; 4096], level);
        assert_roundtrip(&oracle, &text, level);
        assert_roundtrip(&oracle, &xorshift_bytes(0xA5A5_5A5A, 1024), level);
        assert_roundtrip(&oracle, &xorshift_bytes(0xA5A5_5A5A, 128 * 1024), level);
        assert_roundtrip(&oracle, &xorshift_bytes(0xA5A5_5A5A, 256 * 1024), level);
        assert_roundtrip(&oracle, &xorshift_bytes(0xA5A5_5A5A, 1024 * 1024), level);
    }
}

#[test]
fn c_holdout_incomp_32m() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let n = 32 * 1024 * 1024;
    let src = xorshift_bytes(0xA5A5_5A5A, n);
    for level in [1, 3] {
        assert_roundtrip(&oracle, &src, level);
    }
}

#[test]
fn c_train_zeros_and_text_32m() {
    let Some(oracle) = find_oracle() else {
        eprintln!("skip: pinned C zstd not found");
        return;
    };
    let n = 32 * 1024 * 1024;
    let zeros = vec![0u8; n];
    let fox = b"The quick brown fox jumps over the lazy dog. 0123456789.\n";
    let mut text = Vec::with_capacity(n);
    while text.len() < n {
        let take = (n - text.len()).min(fox.len());
        text.extend_from_slice(&fox[..take]);
    }
    for level in [1, 3] {
        assert_roundtrip(&oracle, &zeros, level);
        assert_roundtrip(&oracle, &text, level);
    }
}

/// REGRESSION (2026-08-18): `params.hash_log` is user-settable with no upper
/// bound (`hlog` does only `value.max(6)`), while `MatchTables` allocates at
/// `params.hash_log.clamp(6, 24)`. The chain-walking finders indexed with the
/// RAW value, so `hashLog >= 25` ran off the end of a 2^24 table:
///
///   index out of bounds: the len is 16777216 but the index is 28488790
///
/// Brick 52 fixed `find_fast` and `find_dfast` and left `find_lazy`,
/// `find_greedy`, `chain_find_best` and the prefill on the raw value. Reachable
/// from any caller that forwards user configuration.
#[test]
fn oversized_hash_log_does_not_panic() {
    let src: Vec<u8> = (0..2_000_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for hlog in [24u32, 25, 26, 28, 30, 31] {
        for lvl in [1i32, 3, 5, 7, 9, 12, 13, 19] {
            let mut p = rusty_zstd::compression_params(lvl, Some(src.len() as u64)).unwrap();
            p.hash_log = hlog;
            let z = rusty_zstd::compress_with_params(&src, p, false)
                .unwrap_or_else(|e| panic!("hlog {hlog} L{lvl} failed: {e:?}"));
            assert_eq!(
                rusty_zstd::decompress(&z).unwrap(),
                src,
                "hlog {hlog} L{lvl} round-trip"
            );
        }
    }
}

/// REGRESSION (2026-08-18): compression must depend ONLY on (input, params),
/// never on what was compressed earlier in the process.
///
/// The Gate 19 literal-price feedback first shipped in a process-global static
/// instead of `MatchTables`, so a frame inherited the previous compression's
/// measurement. At L19 the same input gave a different result on the first call
/// than on later ones, on 8 of 12 corpora. Every other feedback signal in the
/// encoder (`rep_yield`, `tag_yield`, `pair_gain`, `dfast_mean_ml`) lives in
/// `MatchTables` and resets per frame; this one did not.
#[test]
fn compression_is_independent_of_call_history() {
    let a: Vec<u8> = (0..600_000u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 15) as u8)
        .collect();
    // High-entropy content first, then a repetitive input: the first frame
    // leaves a HIGH measured literal price, the second a low one.
    let mut b = Vec::with_capacity(600_000);
    while b.len() < 600_000 {
        b.extend_from_slice(b"the quick brown fox jumps over the lazy dog 0123456789 ");
    }
    for lvl in [1i32, 3, 9, 13, 16, 19, 22] {
        let want_a = rusty_zstd::compress(&a, lvl).unwrap();
        let want_b = rusty_zstd::compress(&b, lvl).unwrap();
        // Interleave so each frame is preceded by the OTHER content.
        for _ in 0..3 {
            let _ = rusty_zstd::compress(&b, lvl).unwrap();
            assert_eq!(
                rusty_zstd::compress(&a, lvl).unwrap(),
                want_a,
                "L{lvl}: compressing A depended on what preceded it"
            );
            let _ = rusty_zstd::compress(&a, lvl).unwrap();
            assert_eq!(
                rusty_zstd::compress(&b, lvl).unwrap(),
                want_b,
                "L{lvl}: compressing B depended on what preceded it"
            );
        }
    }
}

/// REGRESSION (2026-08-19): the binary-tree finder dispatches to a const-generic
/// specialisation keyed on `(hash_log, chain_log)`, falling back to a slower
/// hand-written runtime body (279 instructions / 4 variable shifts against the
/// specialisation's 260 / 1). Both parameters are DERIVED FROM THE INPUT SIZE,
/// and the original coverage proof varied only the level -- at one size, the
/// 2 MiB corpus prefix. Measured across the size axis, 24 of 64 (size, level)
/// cells fell through: every input at 64 KiB, 512 KiB and 1 MiB, at every bt
/// level, ran the slow body.
///
/// Asserted against `BT_SPEC_PAIRS`, which is generated from the SAME macro list
/// as the dispatch arms, so this cannot pass while a pair is missing from the
/// dispatch. It deliberately does NOT use the call counters: those are gated
/// behind `--features profile`, so a counter-based version of this test passed
/// vacuously with a pair removed.
#[test]
fn bt_specialisation_covers_every_input_size() {
    let mut uncovered = Vec::new();
    let mut n: u64 = 1024;
    while n <= (64 << 20) {
        for lvl in [13i32, 14, 15, 16, 17, 18, 19, 20, 21, 22] {
            let p = rusty_zstd::compression_params(lvl, Some(n)).unwrap();
            let pair = (p.hash_log.min(24), p.chain_log.min(24));
            if !rusty_zstd::BT_SPEC_PAIRS.contains(&pair) {
                uncovered.push((n >> 10, lvl, pair));
            }
        }
        n += (n / 4).max(1024);
    }
    assert!(
        uncovered.is_empty(),
        "bt specialisation misses (KiB, level, (hash_log, chain_log)): {uncovered:?}"
    );

    // and the bytes must not depend on which body served the call
    let big: Vec<u8> = (0..(3 << 20) as u32)
        .map(|i| (i.wrapping_mul(2_654_435_761) >> 13) as u8)
        .collect();
    for &sz in &[64 << 10, 512 << 10, 1 << 20, 3 << 20] {
        let src = &big[..sz.min(big.len())];
        for lvl in [13i32, 17, 19, 22] {
            rusty_zstd::set_bt_spec_arm(false);
            let a = rusty_zstd::compress(src, lvl).unwrap();
            rusty_zstd::set_bt_spec_arm(true);
            let b = rusty_zstd::compress(src, lvl).unwrap();
            assert_eq!(a, b, "specialised body differs from runtime at {sz} L{lvl}");
            assert_eq!(rusty_zstd::decompress(&b).unwrap(), src);
        }
    }
}
