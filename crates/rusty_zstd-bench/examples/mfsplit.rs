//! MATCH-FIND WORK SPLIT, per SCANNED POSITION. Deterministic, no clock.
//!
//!   cargo run --release --features rusty_zstd/profile -p rusty_zstd-bench --example mfsplit
//!
//! WHY THE OBVIOUS COMPARISON IS INVALID (checked, recorded so it is not
//! redone): `hash_probes` does NOT mean the same thing in the two finders.
//! In `find_fast_impl_inner` it is bumped at the TOP of the scan loop, before
//! the hash is computed -- so it counts POSITIONS. In `find_dfast_impl_inner`
//! it is bumped inside `if let Some(m8)`, i.e. only once a tag filter has
//! already returned a candidate -- so it counts SURVIVORS. Dividing either by
//! input bytes and putting them in one column compares a flow to a filtered
//! flow, and the "hit%" built on them compares 12% (hits/position) against
//! 94% (hits/candidate). Different denominators, not different quality.
//!
//! `MM_TOTAL` (via `take_mm`) IS bumped at the loop top in BOTH, so it is the
//! common denominator. Everything below is per scanned position.
const IDS: &[&str] = &["x-ray", "osdb", "jsonlog-16m", "smallmsg-8m", "ooffice", "sao",
                       "dickens", "samba", "nci", "webster", "mozilla", "mr"];
fn main() {
    let cap: usize = 8 << 20;
    for lvl in [1i32, 3] {
        let p = rusty_zstd::compression_params(lvl, None).unwrap();
        let (mut tpos, mut tf, mut tc, mut th, mut tb) = (0u64, 0u64, 0u64, 0u64, 0f64);
        println!("\n=== L{lvl} ({:?}) ===", p.strategy);
        println!("{:<13} {:>10} {:>10} {:>10} {:>10} {:>9}",
                 "corpus", "pos/B", "fills/pos", "cand/pos", "fills/B", "adv B/pos");
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::prof_reset();
            let _ = rusty_zstd::take_mm();
            let _ = rusty_zstd::compress(s, lvl).unwrap();
            let c = rusty_zstd::prof_encode_counts();
            let (pos, _miss) = rusty_zstd::take_mm();
            let n = s.len() as f64;
            tpos += pos; tf += c.hash_fills; tc += c.hash_probes; th += c.probe_hits; tb += n;
            let pf = if pos == 0 { 0.0 } else { pos as f64 };
            println!("{:<13} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>9.2}",
                     id, pos as f64 / n, c.hash_fills as f64 / pf,
                     c.hash_probes as f64 / pf, c.hash_fills as f64 / n,
                     if pos == 0 { 0.0 } else { n / pos as f64 });
        }
        let pf = tpos as f64;
        println!("{:<13} {:>10.3} {:>10.3} {:>10.3} {:>10.3} {:>9.2}",
                 "TOTAL", tpos as f64 / tb, tf as f64 / pf, tc as f64 / pf,
                 tf as f64 / tb, tb / pf);
        println!("  positions {tpos}, fills {tf}, candidates {tc}, hits {th}");
    }
}
