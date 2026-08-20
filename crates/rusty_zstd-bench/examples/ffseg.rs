fn main() {
    let f = std::fs::read("corpora/data/generated/versions-16m").unwrap();
    let s = &f[..f.len().min(8 << 20)];
    let seg = 512 << 10;
    println!("{:<6} {:>9} {:>9} {:>8}", "seg", "legacy", "wide", "delta");
    let (mut tl, mut tw) = (0i64, 0i64);
    for (i, ch) in s.chunks(seg).enumerate() {
        rusty_zstd::set_fast_hash_arm(false);
        let a = rusty_zstd::compress(ch, 1).unwrap().len() as i64;
        rusty_zstd::set_fast_hash_arm(true);
        let b = rusty_zstd::compress(ch, 1).unwrap().len() as i64;
        tl += a; tw += b;
        if (b - a).abs() > 20 { println!("{:<6} {:>9} {:>9} {:>+8}", i, a, b, b - a); }
    }
    println!("TOTAL  {tl:>9} {tw:>9} {:>+8}   (independent 512K segments)", tw - tl);
}
