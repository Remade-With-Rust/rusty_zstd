//! Byte-identity gate for the window-bounded prefix copy, across the whole
//! level ladder — not just the level it was found on.
//!
//! The bound is `window + BLOCKSIZE_MAX`. A byte-identity pass alone proves
//! nothing -- it may just mean the payload never reaches deep history at these
//! sizes -- so every run also compresses against a HALF-WINDOW prefix and
//! asserts that arm DOES differ. If the control stops firing, the test has gone
//! blind and the identity result is worthless.
fn main() {
    const LEVELS: &[i32] = &[1, 3, 5, 7, 9, 13, 16, 19, 22];
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
    let (mut cells, mut diff, mut tight_diff) = (0, 0, 0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < (5 << 20) { continue }
        let pre = &full[..4 << 20];
        let tail = &full[4 << 20..(4 << 20) + (1 << 20)];
        for &lvl in LEVELS {
            let p = rusty_zstd::compression_params(lvl, Some(tail.len() as u64)).unwrap();
            let window = 1usize << p.window_log.min(31);
            // reference: the FULL prefix, hand-truncated by the caller so the
            // library sees the same bytes it would have copied itself
            let z_ref = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            let keep = window + rusty_zstd::BLOCKSIZE_MAX as usize;
            let z_bound = rusty_zstd::compress_using_prefix(tail, &pre[pre.len().saturating_sub(keep)..], lvl).unwrap();
            // CONTROL: a deliberately too-tight bound. `window` alone is NOT
            // tight enough to fire (measured: identical on 162/162), so the
            // control must be half the window -- `prefixsens` shows that changes
            // 13 of 16 corpora at both L3 and L19. Without a control that fires,
            // a byte-identity pass proves only that the test is blind.
            let z_tight = rusty_zstd::compress_using_prefix(tail, &pre[pre.len().saturating_sub(window / 2)..], lvl).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&z_bound, &pre[pre.len().saturating_sub(keep)..]).unwrap() == tail,
                "{id} L{lvl}: round-trip FAILED");
            cells += 1;
            if z_ref != z_bound { diff += 1; println!("  DIFF {id} L{lvl}: {} vs {}", z_ref.len(), z_bound.len()); }
            if z_ref != z_tight { tight_diff += 1; }
        }
    }
    println!("{cells} cells | window+BLOCKSIZE_MAX differs on {diff} (must be 0)");
    println!("{cells} cells | HALF-window control differs on {tight_diff} (must be > 0, else the test is blind)");
    assert!(tight_diff > 0, "control did not fire -- this test cannot detect a bad bound");
    assert_eq!(diff, 0, "the shipped bound is NOT byte-identical");
}
