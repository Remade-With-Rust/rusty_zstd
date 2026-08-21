//! DETERMINISTIC dead-copy census: which const-generic combinations can the
//! level tables actually produce? Enumerates every clevel x every input-size
//! decade (plus the unknown-size/streaming case) and reports the reachable
//! (strategy, hash_log, chain_log, min_match, target_length) set.
use std::collections::{BTreeMap, BTreeSet};
fn main() {
    let mut sizes: Vec<Option<u64>> = vec![None];
    let mut n: u64 = 1;
    while n <= (1u64 << 31) {
        for m in [1u64, 3, 7] {
            let v = n.saturating_mul(m);
            if v <= (1u64 << 31) { sizes.push(Some(v)); }
        }
        n <<= 1;
    }
    // per strategy: the reachable hash_log set, chain_log set, and pairs
    let mut hl: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut pairs: BTreeMap<String, BTreeSet<(u32, u32)>> = BTreeMap::new();
    let mut mm: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    let mut tl: BTreeMap<String, BTreeSet<u32>> = BTreeMap::new();
    for lv in rusty_zstd::MIN_CLEVEL..=rusty_zstd::MAX_CLEVEL {
        for s in &sizes {
            let Ok(p) = rusty_zstd::compression_params(lv, *s) else { continue };
            let k = format!("{:?}", p.strategy);
            // the finders clamp exactly this way
            let h = p.hash_log.clamp(6, 24);
            let c = p.chain_log.min(24);
            hl.entry(k.clone()).or_default().insert(h);
            pairs.entry(k.clone()).or_default().insert((h, c));
            mm.entry(k.clone()).or_default().insert(p.min_match.max(3) as u32);
            tl.entry(k.clone()).or_default().insert(u32::from(p.target_length != 0));
        }
    }
    for (k, v) in &hl {
        println!("{k:9} hash_log {:?}", v.iter().collect::<Vec<_>>());
        println!("{:9} min_match {:?}  target_len_nonzero {:?}", "", mm[k].iter().collect::<Vec<_>>(), tl[k].iter().collect::<Vec<_>>());
        if matches!(k.as_str(), "BtLazy2" | "BtOpt" | "BtUltra" | "BtUltra2") {
            println!("{:9} (hash_log,chain_log) pairs {} -> {:?}", "", pairs[k].len(), pairs[k].iter().collect::<Vec<_>>());
        }
    }
}
