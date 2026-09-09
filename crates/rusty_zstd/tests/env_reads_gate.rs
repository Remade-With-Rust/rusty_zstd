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

#[test]
fn env_knob_reads_do_not_scale_with_input() {
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
