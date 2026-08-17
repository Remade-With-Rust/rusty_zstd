fn main() {
    println!("reachable (hash_log, chain_log) by input size — DFast levels");
    println!("{:>12}{:>8}{:>8}{:>8}{:>8}", "size", "L3 h", "L3 c", "L4 h", "L4 c");
    let mut hs = std::collections::BTreeSet::new();
    for &n in &[1usize<<10, 1<<12, 1<<14, 1<<16, 1<<17, 1<<18, 1<<20, 1<<22, 1<<24, 1<<26, 1<<28] {
        let a = rusty_zstd::compression_params(3, Some(n as u64)).unwrap();
        let b = rusty_zstd::compression_params(4, Some(n as u64)).unwrap();
        hs.insert(a.hash_log); hs.insert(b.hash_log);
        println!("{:>12}{:>8}{:>8}{:>8}{:>8}", n, a.hash_log, a.chain_log.min(24), b.hash_log, b.chain_log.min(24));
    }
    // unknown size (streaming) is the other reachable case
    let u3 = rusty_zstd::compression_params(3, None).unwrap();
    let u4 = rusty_zstd::compression_params(4, None).unwrap();
    hs.insert(u3.hash_log); hs.insert(u4.hash_log);
    println!("{:>12}{:>8}{:>8}{:>8}{:>8}", "None", u3.hash_log, u3.chain_log.min(24), u4.hash_log, u4.chain_log.min(24));
    println!("\nreachable hash_log values for DFast: {:?}", hs);
    println!("currently specialised: 12..=20  ({} monomorphizations)", 9);
}
