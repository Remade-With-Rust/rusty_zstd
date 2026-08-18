//! GATE 6 @ L22: same three steps, measured not inherited from L19.
fn main() {
    // STEP 1: is the gate reached at all, and is the arm live elsewhere?
    let probe = std::fs::read("corpora/data/silesia/xml").unwrap();
    let probe = &probe[..probe.len().min(1024*1024)];
    for lvl in [1, 22] {
        rusty_zstd::set_pair_on_arm(false);
        let _ = rusty_zstd::take_finder_calls();
        let a = rusty_zstd::compress(probe, lvl).unwrap().len();
        let (f0, o0) = rusty_zstd::take_finder_calls();
        rusty_zstd::set_pair_on_arm(true);
        let b = rusty_zstd::compress(probe, lvl).unwrap().len();
        let (f1, o1) = rusty_zstd::take_finder_calls();
        println!("L{lvl:<3} pair OFF {a:>9} ({f0} fast, {o0} opt)   pair ON {b:>9} ({f1} fast, {o1} opt)   {}",
                 if a == b { "NO EFFECT" } else { "CHANGES OUTPUT" });
    }
    rusty_zstd::set_pair_on_arm(true);
    println!();
    // STEP 2: is the capability subsumed? measure the DP's search density.
    println!("{:<10}{:>14}{:>14}{:>10}", "corpus", "positions", "bt searches", "per pos");
    for id in ["xml","osdb","nci","webster","mozilla"] {
        let full = std::fs::read(format!("corpora/data/silesia/{id}")).unwrap();
        let src = &full[..full.len().min(1024*1024)];
        let _ = rusty_zstd::take_bt_calls();
        let _ = rusty_zstd::compress(src, 22).unwrap();
        let (sp, rt) = rusty_zstd::take_bt_calls();
        println!("{id:<10}{:>14}{:>14}{:>10.3}", src.len(), sp+rt, (sp+rt) as f64/src.len() as f64);
    }
}
