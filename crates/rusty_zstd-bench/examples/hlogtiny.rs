fn main() {
    let mut hs = std::collections::BTreeSet::new();
    let mut cs = std::collections::BTreeSet::new();
    for n in (0u64..=4096).step_by(1).chain((4096..=1u64<<28).step_by(4096)) {
        for lvl in [3, 4] {
            if let Ok(p) = rusty_zstd::compression_params(lvl, Some(n)) {
                hs.insert(p.hash_log); cs.insert(p.chain_log.min(24));
            }
        }
    }
    for lvl in [3, 4] {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        hs.insert(p.hash_log); cs.insert(p.chain_log.min(24));
    }
    println!("DFast reachable hash_log over EVERY size 0..2^28 plus unknown: {:?}", hs);
    println!("DFast reachable chain_log: {:?}", cs);
    // same for the bt levels
    let mut bt = std::collections::BTreeSet::new();
    for n in (0u64..=1u64<<28).step_by(4096) {
        for lvl in 13..=22 {
            if let Ok(p) = rusty_zstd::compression_params(lvl, Some(n)) {
                bt.insert((p.hash_log, p.chain_log.min(24)));
            }
        }
    }
    for lvl in 13..=22 { let p=rusty_zstd::compression_params(lvl,None).unwrap(); bt.insert((p.hash_log,p.chain_log.min(24))); }
    println!("\nBT levels reachable (hash_log, chain_log) pairs: {} distinct", bt.len());
    let mut v: Vec<_> = bt.into_iter().collect(); v.sort();
    println!("{:?}", v);
}
