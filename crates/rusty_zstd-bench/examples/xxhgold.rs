//! GOLDEN ORACLE for the xxh64 optimisation campaign.
//!
//! `xxhdiff` compares the two ARMS against each other -- it cannot catch a
//! refactor that changes BOTH. This folds every reachable code path into one
//! 64-bit number: one-shot x both arms x 6 patterns x every length 0..4096 plus
//! stripe/tile boundaries, AND the incremental `Xxh64` under 9 chunk schedules.
//! Same number after a brick == byte-identical. This is a FORMAT checksum, so
//! the gate is exhaustive, not sampled.
fn mix(acc: &mut u64, v: u64) {
    *acc = (*acc ^ v).wrapping_mul(0x9E37_79B1_85EB_CA87).rotate_left(31);
}
fn pat(p: u32, l: usize) -> Vec<u8> {
    (0..l)
        .map(|i| match p {
            0 => (i as u8).wrapping_mul(31),
            1 => 0u8,
            2 => 0xFFu8,
            3 => ((i * 2654435761) >> 13) as u8,
            4 => (i % 7) as u8,
            _ => ((i * i) ^ (i >> 3)) as u8,
        })
        .collect()
}
fn main() {
    let mut lens: Vec<usize> = (0..4097).collect();
    // tile (256), stripe (32), chunk (128) and buffer boundaries, +-1 each
    for b in [
        8192usize, 16384, 65536, 65535, 65537, 1 << 20, (1 << 20) + 31, (1 << 20) + 32,
        (1 << 20) + 33, (1 << 20) + 255, (1 << 20) + 256, (1 << 20) + 257, 3_145_728,
    ] {
        lens.push(b);
    }
    let mut acc = 0xDEAD_BEEF_CAFE_F00Du64;
    let mut n = 0usize;
    for p in 0..6u32 {
        for &l in &lens {
            let d = pat(p, l);
            for arm in [false, true] {
                rusty_zstd::set_xxh_avx2_arm(arm);
                mix(&mut acc, rusty_zstd::xxh64_pub(&d));
                n += 1;
            }
        }
    }
    // incremental hasher: every chunk schedule that lands on a different
    // buffer/stripe/tile phase
    rusty_zstd::set_xxh_avx2_arm(true);
    let mut m = 0usize;
    for p in 0..3u32 {
        for &l in &[0usize, 1, 7, 31, 32, 33, 63, 127, 128, 129, 255, 256, 257, 1000, 4096, 100_003] {
            let d = pat(p, l);
            for &c in &[1usize, 3, 7, 17, 31, 32, 33, 128, 257] {
                let mut h = rusty_zstd::Xxh64Pub::new();
                for ch in d.chunks(c.max(1)) {
                    h.update(ch);
                }
                mix(&mut acc, h.digest());
                m += 1;
                assert_eq!(h.digest(), rusty_zstd::xxh64_pub(&d), "pat{p} len{l} chunk{c}");
            }
        }
    }
    rusty_zstd::set_xxh_avx2_arm(true);
    assert_eq!(rusty_zstd::xxh64_pub(b""), 0xEF46DB3751D8E999, "spec vector");
    println!("one-shot cells {n}   incremental cells {m}");
    println!("GOLD {acc:016X}");
}
