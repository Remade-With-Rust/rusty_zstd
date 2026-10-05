//! A knob must be read from the environment ONCE PER PROCESS, not per block.
//!
//! Every read is an OS lookup and a `String` allocation for a value fixed for
//! the life of the process. This crate has been bitten by the per-call shape
//! repeatedly -- one instance is recorded as having cost 60% of L19 encode --
//! and the newest one hid behind a cache whose sentinel COLLIDED with the value
//! it cached: `dfast_step_forced` stored 0 for "cached" while 0 was also what an
//! unset knob resolves to, so the cache never took.
//!
//! Grepping cannot catch that. Counting can: a correctly cached knob costs a
//! fixed number of reads no matter how much data is compressed, so if the count
//! SCALES WITH INPUT SIZE something is re-reading per block.
#![cfg(feature = "profile")]

/// The read counter is ONE process-wide atomic and the harness runs tests on
/// parallel threads, so two tests that both read-and-clear it would count each
/// other's reads. Every test here holds this for its whole body.
static SERIAL: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[test]
fn env_knob_reads_do_not_scale_with_input() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let base: Vec<u8> = (0..(1u32 << 20))
        .map(|i| (i as u8) ^ (i >> 5) as u8)
        .collect();

    // Warm every cache first: the first compress legitimately reads each knob.
    let _ = rusty_zstd::compress(&base[..1 << 16], 3).expect("warm");
    let _ = rusty_zstd::take_env_reads();

    let small = rusty_zstd::compress(&base[..1 << 17], 3).expect("small");
    let n_small = rusty_zstd::take_env_reads();
    let big = rusty_zstd::compress(&base, 3).expect("big");
    let n_big = rusty_zstd::take_env_reads();
    assert!(!small.is_empty() && !big.is_empty());

    // 8x the input must not cost meaningfully more reads. A per-block read
    // would scale with the block count, i.e. ~8x here.
    assert!(
        n_big <= n_small + 2,
        "env reads scale with input: {n_small} for 128 KiB, {n_big} for 1 MiB -- \
         a knob is being re-read per block. Check for a cache whose sentinel \
         collides with the knob's default value."
    );
}

/// ...and not per CALL either.
///
/// The test above holds the call count fixed and scales the input, so a knob
/// read once per `compress` call (not per block) sails through it: 2 reads
/// for 128 KiB, 2 for 1 MiB. That is how `RZSTD_BLOCK_KB` sat uncached on the
/// one-shot driver -- an OS lookup and a `String` per call, ~190 ns of a
/// 250 ns driver on a tiny message -- and how `RZSTD_OPT_FILL_S` /
/// `RZSTD_OPT_FILL_MAX` sat uncached per BLOCK on the optimal-parse ladder,
/// which a test pinned to L3 never enters.
///
/// So: make a set of small calls once (every knob those paths resolve is now
/// cached, however lazily it is first reached), then make the SAME calls
/// again and require ZERO reads. The encoder is deterministic, so the second
/// pass walks exactly the first pass's paths and any read is a re-read.
/// L1 is deliberately absent -- `RZSTD_FFBAR_ALL` is a `profile`-only
/// experiment knob read per Fast block, and this test only exists under
/// `profile`; a shipping build does not carry that read.
#[test]
fn env_knob_reads_do_not_scale_with_calls() {
    let _serial = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
    let base: Vec<u8> = (0..(1u32 << 16))
        .map(|i| (i as u8) ^ (i >> 5) as u8)
        .collect();
    let dict = rusty_zstd::Dictionary::raw(base[..8192].to_vec());
    for level in [3, 5, 9, 13, 16, 19, 22] {
        let pass = || {
            for m in base[8192..].chunks(1024).take(40) {
                let _ = rusty_zstd::compress(m, level).expect("compress");
                let _ = rusty_zstd::compress_using_dict(m, &dict, level).expect("dict compress");
            }
        };
        pass();
        let _ = rusty_zstd::take_env_reads();
        pass();
        let n = rusty_zstd::take_env_reads();
        assert_eq!(
            n, 0,
            "L{level}: {n} env reads across 80 repeated calls -- a knob is read per call or per block"
        );
    }
}
