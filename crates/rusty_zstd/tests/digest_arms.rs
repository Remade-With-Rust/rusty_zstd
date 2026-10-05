//! A digest primed under one arm value must never serve a call made under
//! another.
//!
//! The digested dictionary (`encode.rs`, THE DIGESTED DICTIONARY) keeps the
//! table state `prime_tables` leaves behind and reuses it across calls. That
//! state depends on more than the dictionary bytes and the parameters: it
//! depends on every process-global ARM the priming and the table
//! representation resolve -- the priming stride, the Fast ladder's wide key,
//! the packed-tag switches, the prefix cut. Those arms are bench hooks that a
//! harness flips between calls, so each one has to be part of the digest's
//! key (or make the frame ineligible), or a stale snapshot would be served.
//!
//! This flips each arm back and forth UNDER A LONG-LIVED `Dictionary` at one
//! level at a time (so that level's digest is still cached when the arm
//! moves), and after every flip compares the long-lived dictionary's output
//! with a dictionary built fresh for the call -- a first sighting, which
//! always takes the per-call priming path. Equal bytes or the key is missing
//! an arm.
//!
//! ONE test, on purpose: the arms are process-global, the harness runs tests
//! on parallel threads, and this file is its own process.
#![cfg(feature = "std")]

use rusty_zstd::{
    compress_using_dict, compress_with_history, compression_params, decompress_using_dict,
    Dictionary,
};

fn wordy(seed: u64, n: usize) -> Vec<u8> {
    const WORDS: &[&str] = &[
        "the",
        "of",
        "and",
        "table",
        "window",
        "offset",
        "literal",
        "match",
        "sequence",
        "dictionary",
        "frame",
        "block",
        "entropy",
        "state",
        "symbol",
        "length",
        "repeat",
        "hash",
        "chain",
        "prime",
        "digest",
        "snapshot",
        "restore",
        "message",
        "payload",
    ];
    let mut x = seed | 1;
    let mut out = Vec::with_capacity(n + 16);
    while out.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        out.extend_from_slice(WORDS[(x % WORDS.len() as u64) as usize].as_bytes());
        out.push(if x & 0x300 == 0 { b'\n' } else { b' ' });
        if x & 0x1C00 == 0 {
            out.extend_from_slice(&(x >> 20).to_le_bytes()[..3]);
        }
    }
    out.truncate(n);
    out
}

type Arm = (&'static str, fn(bool));

#[test]
fn digest_key_follows_every_priming_arm() {
    let arms: [Arm; 13] = [
        ("prime_stride", |on| {
            rusty_zstd::set_prime_stride_arm(if on { 3 } else { 1 })
        }),
        ("fast_hash", rusty_zstd::set_fast_hash_arm),
        ("fast_pack", rusty_zstd::set_fast_pack_arm),
        ("tag_alloc", rusty_zstd::set_tag_alloc_arm),
        ("dfast_tag", rusty_zstd::set_dfast_tag_arm),
        ("long_tag", rusty_zstd::set_long_tag_arm),
        ("chain_tag", rusty_zstd::set_chain_tag_arm),
        ("prime_bt", rusty_zstd::set_prime_bt_arm),
        ("prime_bt_tree", rusty_zstd::set_prime_bt_tree_arm),
        ("prefix_window", rusty_zstd::set_prefix_window_arm),
        ("prefix_bound", rusty_zstd::set_prefix_bound_arm),
        ("row", |on| {
            if on {
                rusty_zstd::set_row_arm(true)
            } else {
                rusty_zstd::set_row_arm_auto()
            }
        }),
        ("hash_tight", |on| {
            rusty_zstd::set_hash_tight_arm(u32::from(!on))
        }),
    ];
    let data = wordy(0xA2A5, 700_000);
    let dict_bytes = &data[..40_000];
    let big_bytes = &data[100_000..500_000];
    let kept = Dictionary::raw(dict_bytes.to_vec());
    let kept_big = Dictionary::raw(big_bytes.to_vec());
    let mut checked = 0u32;
    for level in [1, 2, 3, 4, 5, 9, 13, 19] {
        // Seat this level's digest before any arm moves.
        for i in 0..3 {
            let _ = compress_using_dict(&data[600_000 + i * 2000..][..1500], &kept, level).unwrap();
        }
        for (name, set) in arms {
            for (k, on) in [true, false, true, false].into_iter().enumerate() {
                set(on);
                for i in 0..3usize {
                    let at = 510_000 + ((k * 3 + i) * 7919) % 150_000;
                    let msg = &data[at..at + [1200usize, 300, 5000][i]];
                    let want =
                        compress_using_dict(msg, &Dictionary::raw(dict_bytes.to_vec()), level)
                            .unwrap();
                    let got = compress_using_dict(msg, &kept, level).unwrap();
                    assert_eq!(
                        got, want,
                        "arm {name}={on} L{level} message {i}: the digest served a stale snapshot"
                    );
                    assert_eq!(decompress_using_dict(&got, &kept).unwrap(), msg);
                    checked += 1;
                }
                // The prefix cut only bites when the dictionary is larger
                // than window + one block: parameters sized from the payload.
                if level <= 5 {
                    let msg = &data[520_000 + k * 4000..][..3000];
                    let p = compression_params(level, Some(msg.len() as u64)).unwrap();
                    let fresh = Dictionary::raw(big_bytes.to_vec());
                    let want =
                        compress_with_history(msg, p, false, Some(&fresh), &[], true).unwrap();
                    let got =
                        compress_with_history(msg, p, false, Some(&kept_big), &[], true).unwrap();
                    assert_eq!(got, want, "arm {name}={on} L{level}: large dictionary");
                    checked += 1;
                }
            }
        }
    }
    assert!(checked > 1000);
}
