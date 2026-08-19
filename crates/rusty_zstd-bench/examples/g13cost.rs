//! Does the Gate 13 dispatch COST fast-path hits? A block whose share is below
//! the threshold still has SOME runs the copy would have caught; gating off
//! sends those to the slow path too. That is the trade, and it must be counted
//! rather than asserted.
const IDS: &[&str] = &["sao","x-ray","mr","ooffice","osdb","mozilla","dickens","webster","nci","samba","xml","jsonlog-16m","smallmsg-8m","reymont"];
fn main() {
    let lvl: i32 = std::env::args().nth(1).and_then(|s| s.parse().ok()).unwrap_or(1);
    let cap: usize = std::env::args().nth(2).and_then(|s| s.parse().ok()).unwrap_or(8 << 20);
    for t in [0.25f32, 0.40, 0.60] {
        println!("\n=== threshold {t} ===");
        println!("{:<13} {:>10} {:>10} {:>9} | {:>10} {:>10} {:>9}", "corpus", "fast off", "fast on", "lost", "fail off", "fail on", "saved");
        let (mut lo, mut ln, mut fo, mut fn_) = (0i64,0i64,0i64,0i64);
        for id in IDS {
            let Ok(f) = std::fs::read(format!("corpora/data/generated/{id}"))
                .or_else(|_| std::fs::read(format!("corpora/data/silesia/{id}"))) else { continue };
            let s = &f[..f.len().min(cap)];
            rusty_zstd::set_lit_short_arm(-1.0);
            let _ = rusty_zstd::take_lp_stats(); let _ = rusty_zstd::take_lp_guard();
            let a = rusty_zstd::compress(s, lvl).unwrap();
            let (_, fast_off, _) = rusty_zstd::take_lp_stats();
            let (fail_off, _) = rusty_zstd::take_lp_guard();
            rusty_zstd::set_lit_short_arm(t);
            let _ = rusty_zstd::take_lp_stats(); let _ = rusty_zstd::take_lp_guard();
            let b = rusty_zstd::compress(s, lvl).unwrap();
            let (_, fast_on, _) = rusty_zstd::take_lp_stats();
            let (fail_on, _) = rusty_zstd::take_lp_guard();
            assert_eq!(a, b, "{id}: output moved");
            let lost = fast_off as i64 - fast_on as i64;
            let saved = fail_off as i64 - fail_on as i64;
            if lost != 0 || saved != 0 {
                println!("{:<13} {:>10} {:>10} {:>9} | {:>10} {:>10} {:>9}", id, fast_off, fast_on, lost, fail_off, fail_on, saved);
            }
            lo += fast_off as i64; ln += fast_on as i64; fo += fail_off as i64; fn_ += fail_on as i64;
        }
        println!("  TOTAL fast-path hits LOST {:>8}   wasted guards SAVED {:>8}   net {:+}",
            lo-ln, fo-fn_, (fo-fn_)-(lo-ln));
    }
    rusty_zstd::set_lit_short_arm(0.25);
}
