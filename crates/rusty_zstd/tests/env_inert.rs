//! Issue #33: the `RZSTD_*` environment variables are campaign arms, read
//! only with the `bench-arms` feature. Without it the process environment
//! must not reach the codec: an arm left set in a shell, a CI job or a
//! container once changed compressed bytes (an L19 frame grew 85%).
//!
//! The knobs are cached per process, so each arm is measured in a fresh one:
//! the test re-runs its own binary as a child, with and without the knobs,
//! and compares a fingerprint of the codec's output. The `bench-arms` twin
//! proves the same knobs do change that fingerprint when they are read, so
//! the default build's equality is not vacuous.
#![cfg(feature = "std")]

use rusty_zstd::{compress, CompressOptions, Compressor, Flush};
use std::process::Command;

const CHILD: &str = "RZSTD_ENV_INERT_CHILD";
const TEST: &str = "environment_fingerprint";

/// Arms measured to change compressed bytes in a default build of 0.4.0.
const KNOBS: &[(&str, &str)] = &[
    ("RZSTD_OPT_LIT", "0"),
    ("RZSTD_ROW", "0"),
    ("RZSTD_LAZY_ACCEL", "1"),
    ("RZSTD_BLOCK_KB", "16"),
    ("RZSTD_ENC_SLIDE_MUL", "2"),
];

fn fnv(h: &mut u64, b: &[u8]) {
    for &x in b {
        *h = (*h ^ u64::from(x)).wrapping_mul(0x100_0000_01b3);
    }
}

fn sample(n: usize) -> Vec<u8> {
    let mut x = 0x5EED_u64;
    let words = [
        "alpha ", "beta ", "gamma ", "delta ", "{\"id\":", "},", "row ", "\n",
    ];
    let mut v = Vec::with_capacity(n + 8);
    while v.len() < n {
        x ^= x << 13;
        x ^= x >> 7;
        x ^= x << 17;
        v.extend_from_slice(words[(x >> 61) as usize].as_bytes());
    }
    v.truncate(n);
    v
}

fn fingerprint() -> u64 {
    let src = sample(600_000);
    let mut h = 0xcbf2_9ce4_8422_2325;
    for level in [1, 3, 6, 9, 19] {
        fnv(&mut h, &compress(&src, level).unwrap());
    }
    let opts = CompressOptions {
        level: 3,
        checksum: true,
    };
    let mut c = Compressor::with_options(opts, None).unwrap();
    let mut out = vec![0u8; 1 << 16];
    for chunk in src.chunks(50_000) {
        let mut input = chunk;
        while !input.is_empty() {
            let st = c.stream(input, &mut out, Flush::Continue).unwrap();
            fnv(&mut h, &out[..st.output_produced]);
            input = &input[st.input_consumed..];
        }
    }
    loop {
        let st = c.stream(&[], &mut out, Flush::End).unwrap();
        fnv(&mut h, &out[..st.output_produced]);
        if st.done {
            break;
        }
    }
    h
}

/// The child's half: print the fingerprint for the parent to read.
#[test]
fn environment_fingerprint() {
    if std::env::var_os(CHILD).is_some() {
        println!("FINGERPRINT {:016x}", fingerprint());
    }
}

fn child(knobs: &[(&str, &str)]) -> String {
    let out = Command::new(std::env::current_exe().unwrap())
        .args([TEST, "--exact", "--nocapture", "--test-threads=1"])
        .env(CHILD, "1")
        .envs(knobs.iter().copied())
        .output()
        .expect("spawn the test binary");
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout)
        .lines()
        // libtest prints `test environment_fingerprint ... ` on the same line.
        .find_map(|l| l.split("FINGERPRINT ").nth(1).map(str::to_owned))
        .expect("child printed no fingerprint")
}

#[test]
#[cfg(not(feature = "bench-arms"))]
fn rzstd_environment_does_not_reach_the_codec() {
    assert_eq!(child(&[]), child(KNOBS));
}

#[test]
#[cfg(feature = "bench-arms")]
fn rzstd_environment_drives_the_arms_with_bench_arms() {
    assert_ne!(child(&[]), child(KNOBS));
}
