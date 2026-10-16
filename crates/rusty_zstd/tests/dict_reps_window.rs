//! Issue #23: a dictionary's repeat offsets may exceed the frame window. They
//! are legal while the frame is within its first Window_Size bytes and not
//! after, and the encoder kept offering them as match candidates past that
//! point -- emitting frames rusty_zstd's own decoder rejects as corrupt.
#![cfg(feature = "std")]

use rusty_zstd::{
    compress_with_history, compression_params, decompress_using_dict, train, Dictionary,
    TrainOptions,
};

fn xorshift(seed: u64, n: usize) -> Vec<u8> {
    let mut x = seed;
    (0..n)
        .map(|_| {
            x ^= x << 13;
            x ^= x >> 7;
            x ^= x << 17;
            (x >> 32) as u8
        })
        .collect()
}

/// A trained dictionary with `extra` appended to its content and its repeat
/// offsets replaced by `reps` (the 12 bytes just before the content).
pub fn dict_with_reps(reps: [u32; 3], extra: &[u8]) -> Dictionary {
    let samples: Vec<Vec<u8>> = (0..24)
        .map(|i| format!("{{\"id\":{i},\"kind\":\"row\",\"name\":\"s-{}\"}}", i * 7).into_bytes())
        .collect();
    let refs: Vec<&[u8]> = samples.iter().map(Vec::as_slice).collect();
    let mut raw = train(&refs, TrainOptions::default()).expect("train");
    let content_len = Dictionary::from_bytes(&raw).expect("parse").content().len();
    let at = raw.len() - content_len - 12;
    for (i, r) in reps.iter().enumerate() {
        raw[at + 4 * i..at + 4 * i + 4].copy_from_slice(&r.to_le_bytes());
    }
    raw.extend_from_slice(extra);
    let d = Dictionary::from_bytes(&raw).expect("parse patched");
    assert!(reps.iter().all(|&r| r as usize <= d.content().len()));
    d
}

/// Repeats with period `r`, so a rep of `r` matches everywhere and nothing
/// shorter does.
fn periodic(r: usize, len: usize) -> Vec<u8> {
    xorshift(0x9E37_79B9_7F4A_7C15, r).iter().cycle().take(len).copied().collect()
}

#[test]
fn dictionary_reps_beyond_the_window_decode() {
    let r = 1500u32;
    let dict = dict_with_reps([r, 4, 8], &xorshift(7, 4096));
    let src = periodic(r as usize, 64 * 1024);
    for level in [1, 2, 3, 4, 5, 6, 7, 9, 12, 13, 15, 16, 17, 19, 22] {
        let mut p = compression_params(level, Some(src.len() as u64)).unwrap();
        p.window_log = 10;
        p.hash_log = p.hash_log.min(11);
        p.chain_log = p.chain_log.min(11);
        let zst = compress_with_history(&src, p, true, Some(&dict), &[], true).unwrap();
        let back = decompress_using_dict(&zst, &dict)
            .unwrap_or_else(|e| panic!("L{level}: our decoder rejects our frame: {e:?}"));
        assert_eq!(back, src, "L{level}");
    }
}
