//! D4 coverage census: does ANY workload reach the dict/frame CROSSING path?
//!
//! A brick on that path is unverified until `dict_CROSS` is non-zero. Per the
//! skills' own law: byte-identity over content that never enters the changed
//! branch proves nothing about that branch.
fn main() {
    let mut total = [0u64; 3];
    let mut show = |label: &str| {
        let p = rusty_zstd::take_d4_paths();
        for i in 0..3 { total[i] += p[i]; }
        println!("  {label:<44} frame_only={:<9} dict_only={:<7} dict_CROSS={}", p[0], p[1], p[2]);
    };
    let _ = rusty_zstd::take_d4_paths();

    // 1. a dictionary whose TAIL is the pattern the frame continues with --
    //    the shape that makes a match start in the dict and run past its end.
    let mut dict = Vec::new();
    for i in 0..(64usize << 10) { dict.push((i.wrapping_mul(2654435761) >> 13) as u8); }
    let tail: Vec<u8> = dict[dict.len() - 64..].to_vec();
    let mut src = Vec::new();
    for _ in 0..2000 { src.extend_from_slice(&tail); }
    let d = rusty_zstd::Dictionary::raw(dict.clone());
    for lvl in [1i32, 3, 9, 19] {
        let z = rusty_zstd::compress_using_dict(&src, &d, lvl).expect("compress");
        let out = rusty_zstd::decompress_using_dict(&z, &d).expect("decompress");
        assert_eq!(out, src, "roundtrip L{lvl}");
        show(&format!("dict-tail repeat, L{lvl}"));
    }

    // 2. frame that begins with the dict tail then diverges
    let mut src2 = tail.clone();
    for i in 0..40_000usize { src2.push((i % 251) as u8); }
    src2.extend_from_slice(&tail);
    for lvl in [1i32, 3, 19] {
        let z = rusty_zstd::compress_using_dict(&src2, &d, lvl).expect("compress");
        let out = rusty_zstd::decompress_using_dict(&z, &d).expect("decompress");
        assert_eq!(out, src2, "roundtrip2 L{lvl}");
        show(&format!("dict-tail then diverge, L{lvl}"));
    }
    println!("\nTOTAL  frame_only={}  dict_only={}  dict_CROSS={}", total[0], total[1], total[2]);
    if total[2] == 0 {
        println!("\n*** dict_CROSS == 0: the D4 branch is NOT covered by this fixture. ***");
    }
}
