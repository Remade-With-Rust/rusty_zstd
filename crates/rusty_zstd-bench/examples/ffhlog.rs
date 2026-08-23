//! Which (strategy, hash_log) pairs can the Fast ladder actually reach?
//! Exhaustive over every level that selects Fast and every input size class,
//! plus the unknown-size (streaming) case. Same method as GATE 5's dfast cut.
use std::collections::BTreeSet;
fn main() {
    let mut fast: BTreeSet<u32> = BTreeSet::new();
    let mut dfast: BTreeSet<u32> = BTreeSet::new();
    let mut sizes: Vec<Option<u64>> = vec![None];
    // every power of two up to 2^28, and each +-1 around it
    for b in 0..=28u32 {
        let n = 1u64 << b;
        for d in [0i64, -1, 1] {
            let v = n as i64 + d;
            if v >= 0 { sizes.push(Some(v as u64)); }
        }
    }
    for lvl in -22..=22i32 {
        for &sz in &sizes {
            let Ok(p) = rusty_zstd::compression_params(lvl, sz) else { continue };
            let hl = p.hash_log.clamp(6, 24);
            match p.strategy {
                rusty_zstd::Strategy::Fast => { fast.insert(hl); }
                rusty_zstd::Strategy::DFast => { dfast.insert(hl); }
                _ => {}
            }
        }
    }
    let mut ex: Vec<(i32,Option<u64>,u32)> = Vec::new();
    for lvl in -22..=22i32 {
        for &sz in &sizes {
            let Ok(p) = rusty_zstd::compression_params(lvl, sz) else { continue };
            let hl = p.hash_log.clamp(6, 24);
            if matches!(p.strategy, rusty_zstd::Strategy::DFast) && hl < 14 {
                ex.push((lvl, sz, hl));
            }
        }
    }
    ex.sort(); ex.dedup_by_key(|e| (e.0, e.2));
    println!("DFast with hash_log < 14 (first per level/hl): {:?}", &ex[..ex.len().min(12)]);
    println!("Fast  reachable hash_log: {:?}", fast);
    println!("DFast reachable hash_log: {:?}", dfast);
    println!("Fast  specialised in dispatch: {{12,13,14,15,16}} + generic");
    println!("DFast specialised in dispatch: {{14,15,16,17,18}} + generic");
    let spec: BTreeSet<u32> = [12,13,14,15,16].into_iter().collect();
    println!("Fast  DEAD specialisations: {:?}", spec.difference(&fast).collect::<Vec<_>>());
    println!("Fast  reachable but NOT specialised: {:?}", fast.difference(&spec).collect::<Vec<_>>());
}
