//! Byte-identity gate for the window-bounded prefix copy, across the level ladder.
//!
//! Driven through `set_prefix_bound_arm` rather than by handing the library a
//! pre-truncated prefix. Since FINDING 1 the window is sized from
//! payload + prefix, so truncating at the CALLER also shrinks the window and the
//! two arms would no longer be comparable -- the test would be measuring the
//! window change, not the copy bound.
//!
//! A byte-identity pass proves nothing on its own, so a HALF-WINDOW control runs
//! alongside and must DIFFER. If it stops firing the test has gone blind.
fn main() {
    const LEVELS: &[i32] = &[1, 3, 5, 9, 13, 19, 22];
    const IDS: &[&str] = &["mozilla","webster","nci","samba","osdb","dickens","mr","xml","reymont","sao","ooffice","x-ray","zeros-32m","text-32m","incomp-32m","jsonlog-16m","smallmsg-8m","versions-16m"];
    let (mut cells, mut diff, mut ctrl) = (0, 0, 0);
    for id in IDS {
        let Ok(full) = std::fs::read(format!("corpora/data/generated/{id}"))
            .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
        if full.len() < (3 << 20) { continue }
        let pre = &full[..2 << 20];
        let tail = &full[2 << 20..(2 << 20) + (512 << 10)];
        for &lvl in LEVELS {
            rusty_zstd::set_prefix_bound_arm(false);
            let z_full = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            rusty_zstd::set_prefix_bound_arm(true);
            let z_bound = rusty_zstd::compress_using_prefix(tail, pre, lvl).unwrap();
            assert!(rusty_zstd::decompress_using_prefix(&z_bound, pre).unwrap() == tail,
                "{id} L{lvl}: round-trip FAILED");
            cells += 1;
            if z_full != z_bound { diff += 1; println!("  DIFF {id} L{lvl}: {} vs {}", z_full.len(), z_bound.len()); }
            // CONTROL: hand it a half-window prefix. Must differ, or the test is blind.
            let p = rusty_zstd::compression_params(lvl, Some((tail.len() + pre.len()) as u64)).unwrap();
            let half = (1usize << p.window_log.min(31)) / 2;
            let z_half = rusty_zstd::compress_using_prefix(tail, &pre[pre.len().saturating_sub(half)..], lvl).unwrap();
            if z_half != z_bound { ctrl += 1; }
        }
    }
    println!("{cells} cells | bounded vs full-copy differs on {diff} (must be 0)");
    println!("{cells} cells | HALF-window control differs on {ctrl} (must be > 0, else blind)");
    assert_eq!(diff, 0, "the shipped bound is NOT byte-identical");
    assert!(ctrl > 0, "control did not fire -- this test cannot detect a bad bound");
}
